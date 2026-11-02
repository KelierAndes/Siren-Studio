//! Native Bluetooth RFCOMM transport.
//!
//! RFCOMM sockets are platform-specific (BlueZ sockets on Linux, Winsock's
//! Bluetooth provider on Windows).  The tunnel only needs a byte stream, so
//! each native socket is bridged to a loopback `TcpStream` pair here and the
//! rest of the stack keeps talking TCP.
//!
//! One side accepts connections (the elected hub), the other connects to the
//! hub's adapter address.  Channel numbers are fixed by us, so no SDP lookup is
//! needed.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::{CoreError, Result};

/// One end of an RFCOMM connection, before it is bridged to TCP.
trait RawSocket: Send + 'static {
    fn write_raw(&mut self, buffer: &[u8]) -> std::io::Result<usize>;
    fn read_raw(&mut self, buffer: &mut [u8]) -> std::io::Result<usize>;
    fn shutdown_raw(&self);
    fn set_nonblocking(&self, enabled: bool) -> std::io::Result<()>;
    /// A second handle on the same connection so read and write can block in
    /// separate threads. On Windows the same SOCKET value is safe to use from
    /// two threads; POSIX needs a dup().
    fn duplicate(&self) -> std::io::Result<Box<dyn RawSocket>>;
}

struct CloseFlag(Arc<AtomicBool>);

impl CloseFlag {
    fn close(&self, raw: &dyn RawSocket) {
        if !self.0.swap(true, Ordering::SeqCst) {
            raw.shutdown_raw();
        }
    }
}

/// Bridges the native RFCOMM socket to a loopback TCP stream so the rest of
/// the stack only deals with `TcpStream`.
fn bridge_raw(raw: impl RawSocket + 'static) -> Result<TcpStream> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let addr = listener.local_addr()?;
    let mirror = raw
        .duplicate()
        .map_err(|e| CoreError::Backend(format!("duplicate rfcomm socket: {e}")))?;
    let closed = Arc::new(AtomicBool::new(false));
    thread::Builder::new()
        .name("siren-rfcomm-bridge".into())
        .spawn(move || {
            let Ok((tcp, _)) = listener.accept() else {
                return;
            };
            let _ = tcp.set_nodelay(true);
            let mut tcp_reader = match tcp.try_clone() {
                Ok(stream) => stream,
                Err(_) => return,
            };
            let mut tcp_writer = tcp;
            let mut raw = raw;
            let mut mirror = mirror;
            let up_flag = CloseFlag(closed.clone());
            let down_flag = CloseFlag(closed);
            let up = thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match tcp_reader.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if write_all(&mut raw, &buffer[..read]).is_err() {
                                break;
                            }
                        }
                    }
                }
                up_flag.close(&raw);
            });
            let down = thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match mirror.read_raw(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if tcp_writer.write_all(&buffer[..read]).is_err() {
                                break;
                            }
                            let _ = tcp_writer.flush();
                        }
                    }
                }
                down_flag.close(mirror.as_ref());
            });
            let _ = up.join();
            let _ = down.join();
        })
        .map_err(|e| CoreError::Backend(format!("spawn rfcomm bridge: {e}")))?;
    let client = TcpStream::connect(addr)?;
    let _ = client.set_nodelay(true);
    Ok(client)
}

fn write_all(raw: &mut impl RawSocket, mut buffer: &[u8]) -> std::io::Result<()> {
    while !buffer.is_empty() {
        match raw.write_raw(buffer) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "rfcomm closed",
                ))
            }
            Ok(written) => buffer = &buffer[written..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Accepts RFCOMM connections. `accept` polls so `close` always unblocks it.
pub trait BtListener: Send + Sync {
    fn accept(&self) -> Result<TcpStream>;
    fn close(&self);
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BtDevice {
    pub address: String,
    pub name: String,
}

/// Platform RFCOMM factory: adapter address, dial, listen.
pub trait BtTransport: Send + Sync {
    fn local_address(&self) -> Option<String>;
    /// Whether this platform can take part in the overlay at all. Android has
    /// no readable adapter address (the OS masks it) but can still dial the
    /// hub, so it reports `true` without advertising an address.
    fn available(&self) -> bool {
        self.local_address().is_some()
    }
    fn connect(&self, address: &str, channel: u8) -> Result<TcpStream>;
    fn listen(&self, channel: u8) -> Result<Box<dyn BtListener>>;
    /// Make the local adapter discoverable (classic inquiry scan). Used by a
    /// hub so Wi-Fi-less leaves can find it without mDNS.
    fn set_discoverable(&self, _seconds: u32) -> bool {
        false
    }
    /// Classic inquiry; returns devices currently discoverable nearby.
    fn scan(&self, _seconds: u8) -> Vec<BtDevice> {
        Vec::new()
    }
}

/// Transport for platforms with native RFCOMM support in this crate.
pub struct NativeBt;

impl BtTransport for NativeBt {
    fn local_address(&self) -> Option<String> {
        native::local_address()
    }

    fn connect(&self, address: &str, channel: u8) -> Result<TcpStream> {
        native::connect(address, channel)
    }

    fn listen(&self, channel: u8) -> Result<Box<dyn BtListener>> {
        native::listen(channel)
    }

    fn set_discoverable(&self, seconds: u32) -> bool {
        native::set_discoverable(seconds)
    }

    fn scan(&self, seconds: u8) -> Vec<BtDevice> {
        native::scan(seconds)
    }
}

#[cfg(target_os = "linux")]
mod native {
    use super::*;
    use std::os::unix::io::RawFd;

    const BTPROTO_RFCOMM: i32 = 3;
    const SHUT_RDWR: i32 = 2;

    #[repr(C)]
    struct SockAddrRc {
        rc_family: libc::sa_family_t,
        rc_bdaddr: [u8; 6],
        rc_channel: u8,
    }

    struct LinuxSocket(RawFd);

    impl RawSocket for LinuxSocket {
        fn read_raw(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = unsafe { libc::read(self.0, buffer.as_mut_ptr() as *mut _, buffer.len()) };
            if read < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(read as usize)
            }
        }

        fn duplicate(&self) -> std::io::Result<Box<dyn RawSocket>> {
            let fd = unsafe { libc::dup(self.0) };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Box::new(LinuxSocket(fd)))
        }

        fn write_raw(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            let written = unsafe { libc::write(self.0, buffer.as_ptr() as *const _, buffer.len()) };
            if written < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(written as usize)
            }
        }

        fn shutdown_raw(&self) {
            unsafe {
                libc::shutdown(self.0, SHUT_RDWR);
                libc::close(self.0);
            }
        }

        fn set_nonblocking(&self, enabled: bool) -> std::io::Result<()> {
            let flags = unsafe { libc::fcntl(self.0, libc::F_GETFL) };
            if flags < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let updated = if enabled {
                flags | libc::O_NONBLOCK
            } else {
                flags & !libc::O_NONBLOCK
            };
            if unsafe { libc::fcntl(self.0, libc::F_SETFL, updated) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        }
    }

    pub fn local_address() -> Option<String> {
        // Older kernels expose the address in sysfs; newer BlueZ keeps it in
        // its daemon state, so fall back to the control tool.
        for index in 0..4 {
            let path = format!("/sys/class/bluetooth/hci{index}/address");
            if let Ok(text) = std::fs::read_to_string(&path) {
                let value = text.trim();
                if value.len() == 17 {
                    return Some(value.to_ascii_uppercase());
                }
            }
        }
        if let Ok(output) = std::process::Command::new("bluetoothctl")
            .arg("show")
            .output()
        {
            if let Ok(text) = String::from_utf8(output.stdout) {
                for line in text.lines() {
                    let line = line.trim();
                    if let Some(rest) = line.strip_prefix("Controller ") {
                        if let Some(address) = rest.split_whitespace().next() {
                            if address.len() == 17 {
                                return Some(address.to_ascii_uppercase());
                            }
                        }
                    }
                }
            }
        }
        if let Ok(output) = std::process::Command::new("hcitool").arg("dev").output() {
            if let Ok(text) = String::from_utf8(output.stdout) {
                for line in text.lines() {
                    if let Some(address) = line.split_whitespace().nth(1) {
                        if address.len() == 17 {
                            return Some(address.to_ascii_uppercase());
                        }
                    }
                }
            }
        }
        None
    }

    pub fn set_discoverable(seconds: u32) -> bool {
        let timeout = seconds.clamp(30, 3600);
        let _ = std::process::Command::new("bluetoothctl")
            .args(["discoverable-timeout", &timeout.to_string()])
            .output();
        match std::process::Command::new("bluetoothctl")
            .args(["discoverable", "on"])
            .output()
        {
            Ok(output) => {
                let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
                output.status.success() && !text.contains("failed")
            }
            Err(_) => false,
        }
    }

    pub fn scan(seconds: u8) -> Vec<BtDevice> {
        let secs = seconds.clamp(3, 12);
        let mut devices = Vec::new();
        if let Ok(output) = std::process::Command::new("bluetoothctl")
            .args(["--timeout", &secs.to_string(), "scan", "on"])
            .output()
        {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("Device ") {
                    let mut parts = rest.splitn(2, ' ');
                    if let Some(address) = parts.next() {
                        if address.len() == 17 {
                            devices.push(BtDevice {
                                address: address.to_ascii_uppercase(),
                                name: parts.next().unwrap_or("").trim().to_string(),
                            });
                        }
                    }
                }
            }
        }
        if devices.is_empty() {
            if let Ok(output) = std::process::Command::new("hcitool")
                .args(["scan", "--flush"])
                .output()
            {
                for line in String::from_utf8_lossy(&output.stdout).lines() {
                    let mut parts = line.split_whitespace();
                    if let Some(address) = parts.next() {
                        if address.len() == 17 && address.matches(':').count() == 5 {
                            devices.push(BtDevice {
                                address: address.to_ascii_uppercase(),
                                name: parts.collect::<Vec<_>>().join(" "),
                            });
                        }
                    }
                }
            }
        }
        devices.sort_by(|a, b| a.address.cmp(&b.address));
        devices.dedup_by(|a, b| a.address == b.address);
        devices
    }

    fn parse_address(value: &str) -> Result<[u8; 6]> {
        let parts: Vec<&str> = value.split(':').collect();
        if parts.len() != 6 {
            return Err(CoreError::InvalidArgument(format!(
                "invalid Bluetooth address '{value}'"
            )));
        }
        let mut bytes = [0u8; 6];
        for (index, part) in parts.iter().enumerate() {
            // bdaddr_t is little endian: the printed address is reversed.
            bytes[5 - index] = u8::from_str_radix(part, 16).map_err(|_| {
                CoreError::InvalidArgument(format!("invalid Bluetooth address '{value}'"))
            })?;
        }
        Ok(bytes)
    }

    fn open_socket() -> Result<LinuxSocket> {
        let fd = unsafe { libc::socket(libc::AF_BLUETOOTH, libc::SOCK_STREAM, BTPROTO_RFCOMM) };
        if fd < 0 {
            return Err(CoreError::Backend(format!(
                "bluetooth socket: {}",
                std::io::Error::last_os_error()
            )));
        }
        // A small send buffer bounds the Bluetooth backlog: the stale copy is
        // dropped by the receiver anyway, so buffering seconds of audio only
        // delays a future Wi-Fi -> Bluetooth hand-over.
        let buffer: libc::socklen_t = 4 * 1024;
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &buffer as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::socklen_t>() as libc::socklen_t,
            );
        }
        Ok(LinuxSocket(fd))
    }

    pub fn connect(address: &str, channel: u8) -> Result<TcpStream> {
        let target = parse_address(address)?;
        let socket = open_socket()?;
        let addr = SockAddrRc {
            rc_family: libc::AF_BLUETOOTH as libc::sa_family_t,
            rc_bdaddr: target,
            rc_channel: channel,
        };
        let rc = unsafe {
            libc::connect(
                socket.0,
                &addr as *const SockAddrRc as *const libc::sockaddr,
                std::mem::size_of::<SockAddrRc>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            let error = std::io::Error::last_os_error();
            socket.shutdown_raw();
            return Err(CoreError::Backend(format!(
                "RFCOMM connect {address}/{channel}: {error}"
            )));
        }
        bridge_raw(socket)
    }

    pub struct LinuxListener {
        socket: LinuxSocket,
        closed: Arc<AtomicBool>,
    }

    impl BtListener for LinuxListener {
        fn accept(&self) -> Result<TcpStream> {
            loop {
                if self.closed.load(Ordering::Relaxed) {
                    return Err(CoreError::Backend("listener closed".into()));
                }
                let mut storage = [0u8; 128];
                let mut length = storage.len() as libc::socklen_t;
                let fd = unsafe {
                    libc::accept(
                        self.socket.0,
                        storage.as_mut_ptr() as *mut libc::sockaddr,
                        &mut length,
                    )
                };
                if fd >= 0 {
                    return bridge_raw(LinuxSocket(fd));
                }
                let _ = std::io::Error::last_os_error();
                thread::sleep(Duration::from_millis(100));
            }
        }

        fn close(&self) {
            self.closed.store(true, Ordering::Relaxed);
            self.socket.shutdown_raw();
        }
    }

    pub fn listen(channel: u8) -> Result<Box<dyn BtListener>> {
        let socket = open_socket()?;
        let addr = SockAddrRc {
            rc_family: libc::AF_BLUETOOTH as libc::sa_family_t,
            rc_bdaddr: [0u8; 6],
            rc_channel: channel,
        };
        let rc = unsafe {
            libc::bind(
                socket.0,
                &addr as *const SockAddrRc as *const libc::sockaddr,
                std::mem::size_of::<SockAddrRc>() as libc::socklen_t,
            )
        };
        if rc != 0 {
            let error = std::io::Error::last_os_error();
            socket.shutdown_raw();
            return Err(CoreError::Backend(format!(
                "RFCOMM bind channel {channel}: {error}"
            )));
        }
        if unsafe { libc::listen(socket.0, 4) } != 0 {
            let error = std::io::Error::last_os_error();
            socket.shutdown_raw();
            return Err(CoreError::Backend(format!("RFCOMM listen: {error}")));
        }
        socket.set_nonblocking(true).ok();
        Ok(Box::new(LinuxListener {
            socket,
            closed: Arc::new(AtomicBool::new(false)),
        }))
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::core::GUID;
    use windows::Win32::Devices::Bluetooth::{
        BluetoothEnableDiscovery, BluetoothFindDeviceClose, BluetoothFindFirstDevice,
        BluetoothFindNextDevice, BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS,
    };
    use windows::Win32::Devices::Bluetooth::{
        BluetoothFindFirstRadio, BluetoothFindRadioClose, BluetoothGetRadioInfo, AF_BTH,
        BLUETOOTH_FIND_RADIO_PARAMS, BLUETOOTH_RADIO_INFO, BTHPROTO_RFCOMM, SOCKADDR_BTH,
    };
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Networking::WinSock::{
        accept, bind, closesocket, connect as bth_connect, ioctlsocket, listen as winsock_listen,
        recv, send, socket, WSAGetLastError, WSAStartup, SEND_RECV_FLAGS, SOCKET, SOCK_STREAM,
        WSADATA,
    };

    struct WinSocket(SOCKET);

    impl RawSocket for WinSocket {
        fn read_raw(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = unsafe { recv(self.0, buffer, SEND_RECV_FLAGS(0)) };
            if read < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(read as usize)
            }
        }

        fn duplicate(&self) -> std::io::Result<Box<dyn RawSocket>> {
            // The same SOCKET handle may be used for a blocking send in one
            // thread and a blocking recv in another; closesocket happens once
            // behind the shared close flag.
            Ok(Box::new(WinSocket(self.0)))
        }

        fn write_raw(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            let sent = unsafe { send(self.0, buffer, SEND_RECV_FLAGS(0)) };
            if sent < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(sent as usize)
            }
        }

        fn shutdown_raw(&self) {
            unsafe {
                let _ = closesocket(self.0);
            }
        }

        fn set_nonblocking(&self, enabled: bool) -> std::io::Result<()> {
            let mode: u32 = u32::from(enabled);
            let rc = unsafe {
                ioctlsocket(
                    self.0,
                    0x8004_666E_u32 as i32,
                    &mode as *const u32 as *mut u32,
                )
            };
            if rc != 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }

    fn wsa_start() -> Result<()> {
        unsafe {
            let mut data = WSADATA::default();
            if WSAStartup(0x0202, &mut data) != 0 {
                return Err(CoreError::Backend("WSAStartup failed".into()));
            }
        }
        Ok(())
    }

    pub fn local_address() -> Option<String> {
        unsafe {
            let params = BLUETOOTH_FIND_RADIO_PARAMS {
                dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
            };
            let mut radio = HANDLE::default();
            let find = BluetoothFindFirstRadio(&params, &mut radio).ok()?;
            let mut info = BLUETOOTH_RADIO_INFO {
                dwSize: std::mem::size_of::<BLUETOOTH_RADIO_INFO>() as u32,
                ..Default::default()
            };
            let result = BluetoothGetRadioInfo(radio, &mut info);
            let _ = BluetoothFindRadioClose(find);
            if result != 0 {
                return None;
            }
            let value = info.address.Anonymous.ullLong;
            let bytes = value.to_le_bytes();
            Some(format!(
                "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
                bytes[5], bytes[4], bytes[3], bytes[2], bytes[1], bytes[0]
            ))
        }
    }

    fn parse_address(value: &str) -> Result<u64> {
        let bytes: Vec<u8> = value
            .split(':')
            .map(|part| u8::from_str_radix(part, 16))
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| {
                CoreError::InvalidArgument(format!("invalid Bluetooth address '{value}'"))
            })?;
        if bytes.len() != 6 {
            return Err(CoreError::InvalidArgument(format!(
                "invalid Bluetooth address '{value}'"
            )));
        }
        Ok(bytes
            .iter()
            .fold(0u64, |value, byte| (value << 8) | *byte as u64))
    }

    fn open_socket() -> Result<WinSocket> {
        wsa_start()?;
        let handle = unsafe { socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32) }
            .map_err(|error| CoreError::Backend(format!("Bluetooth socket: {error}")))?;
        // Small send buffer: see the Linux comment in open_socket.
        let buffer = 4 * 1024u32;
        unsafe {
            let _ = windows::Win32::Networking::WinSock::setsockopt(
                handle,
                0xFFFF, // SOL_SOCKET
                0x1001, // SO_SNDBUF
                Some(&buffer.to_ne_bytes()),
            );
        }
        Ok(WinSocket(handle))
    }

    fn address_hex(value: u64) -> String {
        let bytes = value.to_le_bytes();
        format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            bytes[5], bytes[4], bytes[3], bytes[2], bytes[1], bytes[0]
        )
    }

    pub fn set_discoverable(_seconds: u32) -> bool {
        unsafe { BluetoothEnableDiscovery(None, true).as_bool() }
    }

    pub fn scan(seconds: u8) -> Vec<BtDevice> {
        unsafe {
            let params = BLUETOOTH_FIND_RADIO_PARAMS {
                dwSize: std::mem::size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
            };
            let mut radio = HANDLE::default();
            let Ok(find_radio) = BluetoothFindFirstRadio(&params, &mut radio) else {
                return Vec::new();
            };
            let search = BLUETOOTH_DEVICE_SEARCH_PARAMS {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
                fReturnAuthenticated: true.into(),
                fReturnRemembered: true.into(),
                fReturnUnknown: true.into(),
                fReturnConnected: true.into(),
                fIssueInquiry: true.into(),
                cTimeoutMultiplier: seconds.clamp(2, 8),
                hRadio: radio,
            };
            let mut info = BLUETOOTH_DEVICE_INFO {
                dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
                ..Default::default()
            };
            let mut devices = Vec::new();
            if let Ok(find) = BluetoothFindFirstDevice(&search, &mut info) {
                loop {
                    let name = String::from_utf16_lossy(
                        &info.szName[..info
                            .szName
                            .iter()
                            .position(|unit| *unit == 0)
                            .unwrap_or(info.szName.len())],
                    );
                    devices.push(BtDevice {
                        address: address_hex(info.Address.Anonymous.ullLong),
                        name,
                    });
                    info = BLUETOOTH_DEVICE_INFO {
                        dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
                        ..Default::default()
                    };
                    if BluetoothFindNextDevice(find, &mut info).is_err() {
                        break;
                    }
                }
                let _ = BluetoothFindDeviceClose(find);
            }
            let _ = BluetoothFindRadioClose(find_radio);
            devices
        }
    }

    pub fn connect(address: &str, channel: u8) -> Result<TcpStream> {
        let target = parse_address(address)?;
        let socket = open_socket()?;
        let addr = SOCKADDR_BTH {
            addressFamily: AF_BTH,
            btAddr: target,
            serviceClassId: GUID::zeroed(),
            port: channel as u32,
        };
        let rc = unsafe {
            bth_connect(
                socket.0,
                &addr as *const SOCKADDR_BTH as *const _,
                std::mem::size_of::<SOCKADDR_BTH>() as i32,
            )
        };
        if rc != 0 {
            let code = unsafe { WSAGetLastError() };
            socket.shutdown_raw();
            return Err(CoreError::Backend(format!(
                "RFCOMM connect {address}/{channel} failed (WSA {})",
                code.0
            )));
        }
        bridge_raw(socket)
    }

    pub struct WinListener {
        socket: WinSocket,
        closed: Arc<AtomicBool>,
    }

    impl BtListener for WinListener {
        fn accept(&self) -> Result<TcpStream> {
            loop {
                if self.closed.load(Ordering::Relaxed) {
                    return Err(CoreError::Backend("listener closed".into()));
                }
                let accepted = unsafe { accept(self.socket.0, None, None) };
                if let Ok(accepted) = accepted {
                    if accepted.0 != usize::MAX && accepted.0 != 0 {
                        return bridge_raw(WinSocket(accepted));
                    }
                }
                let _ = unsafe { WSAGetLastError() };
                thread::sleep(Duration::from_millis(100));
            }
        }

        fn close(&self) {
            self.closed.store(true, Ordering::Relaxed);
            self.socket.shutdown_raw();
        }
    }

    pub fn listen(channel: u8) -> Result<Box<dyn BtListener>> {
        let socket = open_socket()?;
        let addr = SOCKADDR_BTH {
            addressFamily: AF_BTH,
            btAddr: 0,
            serviceClassId: GUID::zeroed(),
            port: channel as u32,
        };
        let rc = unsafe {
            bind(
                socket.0,
                &addr as *const SOCKADDR_BTH as *const _,
                std::mem::size_of::<SOCKADDR_BTH>() as i32,
            )
        };
        if rc != 0 {
            let code = unsafe { WSAGetLastError() };
            socket.shutdown_raw();
            return Err(CoreError::Backend(format!(
                "RFCOMM bind channel {channel} failed (WSA {})",
                code.0
            )));
        }
        if unsafe { winsock_listen(socket.0, 4) } != 0 {
            let code = unsafe { WSAGetLastError() };
            socket.shutdown_raw();
            return Err(CoreError::Backend(format!(
                "RFCOMM listen failed (WSA {})",
                code.0
            )));
        }
        socket.set_nonblocking(true).ok();
        Ok(Box::new(WinListener {
            socket,
            closed: Arc::new(AtomicBool::new(false)),
        }))
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod native {
    use super::*;

    pub fn local_address() -> Option<String> {
        None
    }

    pub fn connect(_address: &str, _channel: u8) -> Result<TcpStream> {
        Err(CoreError::Backend(
            "RFCOMM is not available on this platform".into(),
        ))
    }

    pub fn listen(_channel: u8) -> Result<Box<dyn BtListener>> {
        Err(CoreError::Backend(
            "RFCOMM is not available on this platform".into(),
        ))
    }

    pub fn set_discoverable(_seconds: u32) -> bool {
        false
    }

    pub fn scan(_seconds: u8) -> Vec<BtDevice> {
        Vec::new()
    }
}
