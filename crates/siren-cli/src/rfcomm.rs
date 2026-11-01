//! RFCOMM ↔ local TCP bridge.
//!
//! The Bluetooth tunnel speaks framed bytes over `TcpStream` links. This
//! command terminates an RFCOMM connection (fixed channel) and exposes it as
//! a local TCP byte stream, so `siren bt --link 127.0.0.1:<port>` can carry
//! the mesh over Bluetooth without platform-specific code in the tunnel.
//!
//! Linux opens the RFCOMM socket through BlueZ; Windows uses the Winsock
//! Bluetooth RFCOMM provider. Channel 15 is the default; both ends must agree.
//! Devices must be paired first (classic Bluetooth authentication is handled
//! by the OS).

use anyhow::{anyhow, Result};

#[derive(clap::Args, Debug)]
pub struct RfcommArgs {
    /// Listen for RFCOMM clients (hub side; Linux only).
    #[arg(long)]
    pub listen: bool,
    /// Connect to this Bluetooth address (leaf side, e.g. 58:FB:84:3A:F1:EB).
    #[arg(long)]
    pub connect: Option<String>,
    /// RFCOMM channel (1..30).
    #[arg(long, default_value_t = 15)]
    pub channel: u8,
    /// Local TCP port for one link (`--connect`).
    #[arg(long)]
    pub bridge: Option<u16>,
    /// First local TCP port for accepted links (`--listen`).
    #[arg(long)]
    pub bridge_base: Option<u16>,
    /// Number of RFCOMM links to accept (`--listen`).
    #[arg(long, default_value_t = 1)]
    pub accept: usize,
}

pub fn run(args: RfcommArgs) -> Result<()> {
    if !(1..=30).contains(&args.channel) {
        return Err(anyhow!("--channel must be between 1 and 30"));
    }
    if args.listen {
        let base = args
            .bridge_base
            .ok_or_else(|| anyhow!("--listen requires --bridge-base"))?;
        imp::serve(args.channel, base, args.accept.max(1))
    } else if let Some(address) = &args.connect {
        let bridge = args
            .bridge
            .ok_or_else(|| anyhow!("--connect requires --bridge"))?;
        imp::connect(address, args.channel, bridge)
    } else {
        Err(anyhow!("pass --listen or --connect <mac>"))
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use anyhow::{Context, Result};
    use bluer::rfcomm::{Listener, SocketAddr, Stream};
    use bluer::Address;
    use tokio::io::copy_bidirectional;
    use tokio::net::TcpListener;

    fn runtime() -> Result<tokio::runtime::Runtime> {
        Ok(tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?)
    }

    pub fn serve(channel: u8, bridge_base: u16, accept: usize) -> Result<()> {
        runtime()?.block_on(async move {
            let listener = Listener::bind(SocketAddr::new(Address::any(), channel))
                .await
                .with_context(|| format!("listen RFCOMM channel {channel}"))?;
            println!("rfcomm: listening on channel {channel} for {accept} link(s)");
            for index in 0..accept {
                let (stream, peer) = listener.accept().await?;
                let port = bridge_base + index as u16;
                println!("rfcomm: {peer:?} -> tcp 127.0.0.1:{port}");
                tokio::spawn(async move {
                    if let Err(error) = bridge(stream, port).await {
                        eprintln!("rfcomm bridge {port}: {error}");
                    }
                });
            }
            let _ = tokio::signal::ctrl_c().await;
            Ok::<(), anyhow::Error>(())
        })
    }

    pub fn connect(address: &str, channel: u8, bridge_port: u16) -> Result<()> {
        let address: Address = address
            .parse()
            .with_context(|| format!("invalid Bluetooth address '{address}'"))?;
        runtime()?.block_on(async move {
            println!("rfcomm: connecting {address} channel {channel}");
            let stream = Stream::connect(SocketAddr::new(address, channel))
                .await
                .with_context(|| format!("connect RFCOMM {address}/{channel}"))?;
            println!("rfcomm: connected, tcp 127.0.0.1:{bridge_port}");
            bridge(stream, bridge_port).await
        })
    }

    async fn bridge(mut stream: Stream, port: u16) -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await?;
        let (mut tcp, _) = listener.accept().await?;
        tcp.set_nodelay(true)?;
        copy_bidirectional(&mut stream, &mut tcp).await?;
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener};
    use std::thread;

    use anyhow::{anyhow, bail, Result};
    use windows::core::GUID;
    use windows::Win32::Devices::Bluetooth::{AF_BTH, BTHPROTO_RFCOMM, SOCKADDR_BTH};
    use windows::Win32::Networking::WinSock::{
        accept, bind, closesocket, connect as bth_connect, listen, recv, send, socket,
        WSAGetLastError, WSAStartup, SEND_RECV_FLAGS, SOCKET, SOCK_STREAM, WSADATA,
    };

    pub fn serve(channel: u8, bridge_base: u16, accept_count: usize) -> Result<()> {
        unsafe {
            let mut data = WSADATA::default();
            if WSAStartup(0x0202, &mut data) != 0 {
                bail!("WSAStartup failed");
            }
            let handle = socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32)
                .map_err(|error| anyhow!("Bluetooth socket: {error}"))?;
            let address = SOCKADDR_BTH {
                addressFamily: AF_BTH,
                btAddr: 0,
                serviceClassId: GUID::zeroed(),
                port: channel as u32,
            };
            if bind(
                handle,
                &address as *const _ as *const _,
                std::mem::size_of::<SOCKADDR_BTH>() as i32,
            ) != 0
            {
                let code = WSAGetLastError();
                let _ = closesocket(handle);
                bail!("RFCOMM bind channel {channel} failed (WSA {})", code.0);
            }
            if listen(handle, 4) != 0 {
                let code = WSAGetLastError();
                let _ = closesocket(handle);
                bail!("RFCOMM listen failed (WSA {})", code.0);
            }
            println!("rfcomm: listening on channel {channel} (Windows) for {accept_count} link(s)");

            let mut bridges = Vec::new();
            for index in 0..accept_count {
                let accepted = accept(handle, None, None)
                    .map_err(|error| anyhow!("RFCOMM accept: {error}"))?;
                if accepted.0 == usize::MAX || accepted.0 == 0 {
                    let code = WSAGetLastError();
                    let _ = closesocket(handle);
                    bail!("RFCOMM accept failed (WSA {})", code.0);
                }
                let port = bridge_base + index as u16;
                println!("rfcomm: link accepted -> tcp 127.0.0.1:{port}");
                bridges.push(thread::spawn(move || {
                    let _ = bridge(accepted, port);
                }));
            }
            for bridge_thread in bridges {
                let _ = bridge_thread.join();
            }
            let _ = closesocket(handle);
        }
        Ok(())
    }

    pub fn connect(address: &str, channel: u8, bridge_port: u16) -> Result<()> {
        let target = parse_address(address)?;
        unsafe {
            let mut data = WSADATA::default();
            if WSAStartup(0x0202, &mut data) != 0 {
                bail!("WSAStartup failed");
            }
            let handle = socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32)
                .map_err(|error| anyhow!("Bluetooth socket: {error}"))?;
            let address = SOCKADDR_BTH {
                addressFamily: AF_BTH,
                btAddr: target,
                serviceClassId: GUID::zeroed(),
                port: channel as u32,
            };
            let rc = bth_connect(
                handle,
                &address as *const _ as *const _,
                std::mem::size_of::<SOCKADDR_BTH>() as i32,
            );
            if rc != 0 {
                let code = WSAGetLastError();
                let _ = closesocket(handle);
                bail!("RFCOMM connect {target:012X}/{channel} failed (WSA {})", code.0);
            }
            println!("rfcomm: connected {target:012X}/{channel}, tcp 127.0.0.1:{bridge_port}");
            bridge(handle, bridge_port)?;
        }
        Ok(())
    }

    fn bridge(handle: SOCKET, bridge_port: u16) -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", bridge_port))?;
        let (tcp, _) = listener.accept()?;
        tcp.set_nodelay(true)?;
        let mut tcp_reader = tcp.try_clone()?;
        let mut tcp_writer = tcp;
        let raw = handle.0;
        let raw_writer = raw;

        let uplink = thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                let read = unsafe { recv(SOCKET(raw), &mut buffer, SEND_RECV_FLAGS(0)) };
                if read <= 0 {
                    break;
                }
                if tcp_writer.write_all(&buffer[..read as usize]).is_err() {
                    break;
                }
            }
            let _ = tcp_writer.shutdown(Shutdown::Write);
        });
        let downlink = thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match tcp_reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let mut offset = 0;
                        while offset < read {
                            let sent = unsafe {
                                send(SOCKET(raw_writer), &buffer[offset..read], SEND_RECV_FLAGS(0))
                            };
                            if sent <= 0 {
                                return;
                            }
                            offset += sent as usize;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let _ = uplink.join();
        let _ = downlink.join();
        unsafe {
            let _ = closesocket(handle);
        }
        Ok(())
    }

    fn parse_address(value: &str) -> Result<u64> {
        let bytes: Vec<u8> = value
            .split(':')
            .map(|part| u8::from_str_radix(part, 16))
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| anyhow!("invalid Bluetooth address '{value}'"))?;
        if bytes.len() != 6 {
            return Err(anyhow!("invalid Bluetooth address '{value}'"));
        }
        Ok(bytes
            .iter()
            .fold(0u64, |value, byte| (value << 8) | *byte as u64))
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod imp {
    use anyhow::{bail, Result};

    pub fn serve(_channel: u8, _bridge_base: u16, _accept: usize) -> Result<()> {
        bail!("RFCOMM is not implemented on this platform")
    }

    pub fn connect(_address: &str, _channel: u8, _bridge_port: u16) -> Result<()> {
        bail!("RFCOMM is not implemented on this platform")
    }
}
