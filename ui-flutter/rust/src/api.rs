//! flutter_rust_bridge surface.
//!
//! Deliberately primitive: every function takes and returns `String`, `bool`,
//! `u64` or `f32`. Long-running work (center/sender startup, mDNS discovery) is
//! launched on a Rust thread by a synchronous `begin_*` call and observed with
//! a synchronous `poll_*` call that returns a small JSON status string.
//!
//! Engines live in process-global registries addressed by a `u64` id; snapshots
//! cross the bridge as JSON, the same contract a Linux WebSocket daemon uses.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use audio_core::hybrid::{HybridConfig, HybridNode, HybridOptions};
use audio_core::mesh::MeshConfig;


#[cfg(target_os = "android")]
mod android_java {
    use std::os::raw::{c_int, c_void};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;

    use jni::objects::{GlobalRef, JObject, JValue};
    use jni::JavaVM;

    static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();
    static ACTIVITY: OnceLock<GlobalRef> = OnceLock::new();
    static INITIALIZED: AtomicBool = AtomicBool::new(false);

    /// Called by the JVM when Kotlin loads the library.
    #[no_mangle]
    pub unsafe extern "C" fn JNI_OnLoad(vm: *mut c_void, _reserved: *mut c_void) -> c_int {
        match JavaVM::from_raw(vm as *mut _) {
            Ok(jvm) => {
                let _ = JAVA_VM.set(jvm);
                jni::sys::JNI_VERSION_1_6
            }
            Err(_) => jni::sys::JNI_ERR,
        }
    }

    /// Called from MainActivity.onCreate with the application context and the
    /// activity itself (needed for MediaProjection consent).
    #[no_mangle]
    pub unsafe extern "C" fn Java_com_sirenstudio_siren_1ui_MainActivity_nativeSetContext(
        env: *mut jni::sys::JNIEnv,
        _class: jni::sys::jclass,
        app_context: jni::sys::jobject,
        activity: jni::sys::jobject,
    ) {
        if INITIALIZED.swap(true, Ordering::SeqCst) {
            return;
        }
        let Ok(env) = jni::JNIEnv::from_raw(env) else {
            return;
        };
        let app_global = match env.new_global_ref(JObject::from_raw(app_context)) {
            Ok(global) => global,
            Err(_) => return,
        };
        if let Ok(activity_global) = env.new_global_ref(JObject::from_raw(activity)) {
            let _ = ACTIVITY.set(activity_global);
        }
        let Some(vm) = JAVA_VM.get() else {
            return;
        };
        ndk_context::initialize_android_context(
            vm.get_java_vm_pointer() as *mut c_void,
            app_global.as_obj().as_raw() as *mut c_void,
        );
        // The global ref must outlive the process.
        std::mem::forget(app_global);

        audio_core::capture::android_capture::install_java_bridge(
            audio_core::capture::android_capture::JavaBridge {
                ensure_capture_permission: ensure_capture_permission,
                start_system_capture: start_system_capture,
                stop_system_capture: stop_system_capture,
            },
        );
        log::info!("android: ndk_context + capture bridge initialized");
    }

    /// Kotlin's AudioRecord thread hands captured 48 kHz stereo PCM16 blocks.
    #[no_mangle]
    pub unsafe extern "C" fn Java_com_sirenstudio_siren_1ui_CaptureService_nativePushCapture(
        env: *mut jni::sys::JNIEnv,
        _class: jni::sys::jclass,
        samples: jni::sys::jshortArray,
    ) {
        let Ok(env) = jni::JNIEnv::from_raw(env) else {
            return;
        };
        let Some(array) = jni::objects::JShortArray::from_raw(samples).into() else {
            return;
        };
        let Ok(len) = env.get_array_length(&array) else {
            return;
        };
        if len <= 0 {
            return;
        }
        let mut raw = vec![0i16; len as usize];
        if env.get_short_array_region(&array, 0, &mut raw).is_err() {
            return;
        }
        let floats: Vec<f32> = raw.iter().map(|s| *s as f32 / 32768.0).collect();
        audio_core::capture::android_capture::push_system_audio(&floats);
    }

    fn call_activity(name: &str, sig: &str, args: &[JValue]) -> Option<bool> {
        let vm = JAVA_VM.get()?;
        let activity = ACTIVITY.get()?;
        let mut env = vm.attach_current_thread().ok()?;
        let result = env
            .call_method(activity.as_obj(), name, sig, args)
            .ok()?;
        result.z().ok()
    }

    fn call_activity_unit(name: &str, sig: &str) {
        let Some(vm) = JAVA_VM.get() else { return };
        let Some(activity) = ACTIVITY.get() else { return };
        let Ok(mut env) = vm.attach_current_thread() else {
            return;
        };
        let _ = env.call_method(activity.as_obj(), name, sig, &[]);
    }

    pub(super) fn state_dir() -> Option<String> {
        let vm = JAVA_VM.get()?;
        let activity = ACTIVITY.get()?;
        let mut env = vm.attach_current_thread().ok()?;
        let value = env
            .call_method(activity.as_obj(), "stateDir", "()Ljava/lang/String;", &[])
            .ok()?;
        let object = value.l().ok()?;
        if object.is_null() {
            return None;
        }
        let text: String = env
            .get_string(&jni::objects::JString::from(object))
            .ok()?
            .into();
        Some(text)
    }

    /// Hub side: make the adapter discoverable.
    pub(super) fn set_discoverable(seconds: i32) -> bool {
        let Some(vm) = JAVA_VM.get() else { return false };
        let Some(activity) = ACTIVITY.get() else { return false };
        let Ok(mut env) = vm.attach_current_thread() else {
            return false;
        };
        match env.call_method(
            activity.as_obj(),
            "setBtDiscoverable",
            "(I)Z",
            &[JValue::Int(seconds)],
        ) {
            Ok(value) => value.z().unwrap_or(false),
            Err(error) => {
                log::warn!("android: setBtDiscoverable: {error}");
                false
            }
        }
    }

    /// Leaf side: classic inquiry, returns "address<TAB>name" lines.
    pub(super) fn scan_devices(timeout_ms: i32) -> String {
        let Some(vm) = JAVA_VM.get() else { return String::new() };
        let Some(activity) = ACTIVITY.get() else { return String::new() };
        let Ok(mut env) = vm.attach_current_thread() else {
            return String::new();
        };
        let value = match env.call_method(
            activity.as_obj(),
            "scanBtDevices",
            "(I)Ljava/lang/String;",
            &[JValue::Int(timeout_ms)],
        ) {
            Ok(value) => value,
            Err(error) => {
                log::warn!("android: scanBtDevices: {error}");
                return String::new();
            }
        };
        let Ok(object) = value.l() else { return String::new() };
        if object.is_null() {
            return String::new();
        }
        env.get_string(&jni::objects::JString::from(object))
            .map(|text| text.into())
            .unwrap_or_default()
    }

    /// Kotlin reports the local adapter address; modern Android often masks
    /// it to 02:00:00:00:00:00, which the caller treats as "not advertised".
    pub(super) fn bt_local_address() -> Option<String> {
        let vm = JAVA_VM.get()?;
        let activity = ACTIVITY.get()?;
        let mut env = vm.attach_current_thread().ok()?;
        let value = env
            .call_method(activity.as_obj(), "btLocalAddress", "()Ljava/lang/String;", &[])
            .ok()?;
        let object = value.l().ok()?;
        if object.is_null() {
            return None;
        }
        let text: String = env
            .get_string(&jni::objects::JString::from(object))
            .ok()?
            .into();
        Some(text)
    }

    /// Ask Kotlin to dial the hub and expose the RFCOMM socket on loopback.
    pub(super) fn start_bt_bridge(address: &str, channel: u8, port: u16) -> bool {
        let Some(vm) = JAVA_VM.get() else { return false };
        let Some(activity) = ACTIVITY.get() else { return false };
        let Ok(mut env) = vm.attach_current_thread() else {
            return false;
        };
        let Ok(address) = env.new_string(address) else {
            return false;
        };
        let jaddress = JObject::from(address);
        let args = [
            JValue::Object(&jaddress),
            JValue::Int(channel as i32),
            JValue::Int(port as i32),
        ];
        match env.call_method(
            activity.as_obj(),
            "startBtBridge",
            "(Ljava/lang/String;II)Z",
            &args,
        ) {
            Ok(value) => value.z().unwrap_or(false),
            Err(error) => {
                log::warn!("android: startBtBridge failed: {error}");
                false
            }
        }
    }

    fn ensure_capture_permission() -> bool {
        call_activity("ensureCapturePermission", "()Z", &[]) == Some(true)
    }

    fn start_system_capture() -> bool {
        let rate = 48_000i32;
        call_activity("startSystemCapture", "(I)Z", &[JValue::Int(rate)]) == Some(true)
    }

    fn stop_system_capture() {
        call_activity_unit("stopSystemCapture", "()V");
    }
}

enum Pending<T> {
    Idle,
    Starting,
    Done(std::result::Result<T, String>),
}

static MESHES: OnceLock<Mutex<HashMap<u64, HybridNode>>> = OnceLock::new();
static MESH_START: OnceLock<Mutex<Pending<u64>>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static LAST_TRACE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static START_TIME: OnceLock<std::time::Instant> = OnceLock::new();



fn meshes() -> &'static Mutex<HashMap<u64, HybridNode>> {
    MESHES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(serde::Deserialize)]
#[serde(default)]
struct DualStackConfig {
    bt_enabled: bool,
    bt_channel: u8,
}

impl Default for DualStackConfig {
    fn default() -> Self {
        Self {
            bt_enabled: true,
            bt_channel: 16,
        }
    }
}




fn mesh_start_slot() -> &'static Mutex<Pending<u64>> {
    MESH_START.get_or_init(|| Mutex::new(Pending::Idle))
}


fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

fn spawn(name: &str, task: impl FnOnce() + Send + 'static) -> String {
    match std::thread::Builder::new().name(name.into()).spawn(task) {
        Ok(_) => String::new(),
        Err(error) => format!("spawn {name} failed: {error}"),
    }
}

fn status_starting() -> String {
    r#"{"state":"starting"}"#.to_string()
}

fn status_done(id: u64) -> String {
    format!(r#"{{"state":"done","id":{id}}}"#)
}

fn status_error(message: &str) -> String {
    format!(
        r#"{{"state":"error","message":{}}}"#,
        serde_json::to_string(message).unwrap_or_else(|_| "\"unknown\"".to_string())
    )
}

fn status_idle() -> String {
    r#"{"state":"idle"}"#.to_string()
}



















/// Identity each platform persists, so a restart does not re-run the hub
/// election or invalidate the hub addresses peers remembered for us. Android
/// keeps it in its own files dir; elsewhere audio-core picks the location.
fn platform_node_id() -> Option<String> {
    #[cfg(target_os = "android")]
    {
        let dir = android_java::state_dir()?;
        return Some(audio_core::mesh::persisted_node_id_in(Some(
            std::path::PathBuf::from(dir).join("node.id"),
        )));
    }
    #[cfg(not(target_os = "android"))]
    {
        None
    }
}

fn begin_start_mesh_inner(config_json: String) -> String {
    let mut config: MeshConfig = match serde_json::from_str(&config_json) {
        Ok(config) => config,
        Err(error) => return format!("invalid mesh config: {error}"),
    };
    if config.node_id.is_none() {
        config.node_id = platform_node_id();
    }
    let dual: DualStackConfig = serde_json::from_str(&config_json).unwrap_or_default();
    let options = HybridOptions {
        config: HybridConfig {
            bt_enabled: dual.bt_enabled,
            bt_channel: dual.bt_channel.max(1),
        },
        transport: android_transport(),
        state_path: android_state_path(),
    };
    match mesh_start_slot().lock() {
        Ok(mut slot) => *slot = Pending::Starting,
        Err(_) => return "mesh start slot poisoned".into(),
    }
    spawn("siren-ffi-mesh-start", move || {
        let result = HybridNode::start(config, options)
            .map(|node| {
                let id = next_id();
                if let Ok(mut meshes) = meshes().lock() {
                    meshes.insert(id, node);
                    log::info!("ffi: mesh {id} registered");
                }
                id
            })
            .map_err(|error| error.to_string());
        if let Ok(mut slot) = mesh_start_slot().lock() {
            *slot = Pending::Done(result);
        }
    })
}

#[cfg(not(target_os = "android"))]
fn android_transport() -> Option<std::sync::Arc<dyn audio_core::rfcomm::BtTransport>> {
    None
}

#[cfg(not(target_os = "android"))]
fn android_state_path() -> Option<std::path::PathBuf> {
    None
}

#[cfg(target_os = "android")]
fn android_transport() -> Option<std::sync::Arc<dyn audio_core::rfcomm::BtTransport>> {
    Some(std::sync::Arc::new(android_bt::AndroidBt))
}

/// The app's private storage: where the last successful hub is remembered so
/// an offline (Wi-Fi-less) start can still dial in.
#[cfg(target_os = "android")]
fn android_state_path() -> Option<std::path::PathBuf> {
    // api.rs is itself the `api` module, so crate-root paths would point at
    // the wrong level; the sibling module resolves directly.
    android_java::state_dir().map(|dir| std::path::PathBuf::from(dir).join("hub.json"))
}

fn poll_start_mesh_inner() -> String {
    match mesh_start_slot().lock() {
        Ok(slot) => match &*slot {
            Pending::Idle => status_idle(),
            Pending::Starting => status_starting(),
            Pending::Done(Ok(id)) => status_done(*id),
            Pending::Done(Err(error)) => status_error(error),
        },
        Err(_) => status_error("mesh start slot poisoned"),
    }
}

fn mesh_snapshot_json_inner(id: u64) -> String {
    let meshes = match meshes().lock() {
        Ok(meshes) => meshes,
        Err(_) => return r#"{"error":"mesh registry poisoned"}"#.to_string(),
    };
    let node = match meshes.get(&id) {
        Some(node) => node,
        None => return format!(r#"{{"error":"mesh {id} not found"}}"#),
    };
    match serde_json::to_string(&node.snapshot()) {
        Ok(json) => json,
        Err(error) => format!(
            r#"{{"error":{}}}"#,
            serde_json::to_string(&error.to_string()).unwrap_or_default()
        ),
    }
}

fn mesh_set_gain_db_inner(id: u64, stream: u64, gain_db: f32) -> bool {
    meshes()
        .lock()
        .ok()
        .and_then(|meshes| meshes.get(&id).map(|node| node.mesh().center().set_gain_db(stream, gain_db)))
        .unwrap_or(false)
}

fn mesh_set_muted_inner(id: u64, stream: u64, muted: bool) -> bool {
    meshes()
        .lock()
        .ok()
        .and_then(|meshes| meshes.get(&id).map(|node| node.mesh().center().set_muted(stream, muted)))
        .unwrap_or(false)
}

fn mesh_set_target_ms_inner(id: u64, ms: f32) -> bool {
    meshes()
        .lock()
        .ok()
        .and_then(|meshes| meshes.get(&id).map(|node| node.mesh().center().set_target_ms(ms).is_ok()))
        .unwrap_or(false)
}

fn mesh_set_master_gain_db_inner(id: u64, db: f32) -> bool {
    meshes()
        .lock()
        .ok()
        .and_then(|meshes| {
            meshes
                .get(&id)
                .map(|node| node.mesh().center().set_master_gain_db(db).is_ok())
        })
        .unwrap_or(false)
}

fn mesh_stop_inner(id: u64) -> bool {
    log::info!("ffi: mesh_stop({id}) requested");
    match meshes().lock() {
        Ok(meshes) => match meshes.get(&id) {
            Some(node) => {
                node.stop();
                true
            }
            None => false,
        },
        Err(_) => false,
    }
}

fn mesh_release_inner(id: u64) -> bool {
    // Take the node out under the lock, then tear it down outside of it:
    // dropping a node closes sockets and reaps threads, and holding the
    // registry mutex across that blocks the UI's status poll.
    let node = match meshes().lock() {
        Ok(mut meshes) => meshes.remove(&id),
        Err(_) => None,
    };
    log::info!("ffi: mesh_release({id}) requested");
    let released = node.is_some();
    drop(node);
    released
}

fn mesh_is_running_inner(id: u64) -> bool {
    meshes()
        .lock()
        .ok()
        .and_then(|meshes| meshes.get(&id).map(|node| node.mesh().is_running()))
        .unwrap_or(false)
}




/// Minimal logcat bridge so engine logs are visible via adb on device builds.
#[cfg(target_os = "android")]
fn install_android_logger() {
    use std::ffi::CString;
    use std::os::raw::c_int;
    use std::sync::atomic::{AtomicU8, Ordering};

    extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    const ANDROID_LOG_INFO: c_int = 4;
    const ANDROID_LOG_WARN: c_int = 5;
    const ANDROID_LOG_ERROR: c_int = 6;

    struct AndroidLogger;

    impl log::Log for AndroidLogger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::Level::Info
        }
        fn log(&self, record: &log::Record) {
            let prio = match record.level() {
                log::Level::Error => ANDROID_LOG_ERROR,
                log::Level::Warn => ANDROID_LOG_WARN,
                _ => ANDROID_LOG_INFO,
            };
            let tag = CString::new("siren").unwrap_or_default();
            let text = CString::new(format!("{}", record.args())).unwrap_or_default();
            unsafe {
                __android_log_write(prio, tag.as_ptr(), text.as_ptr());
            }
        }
        fn flush(&self) {}
    }

    static INSTALLED: AtomicU8 = AtomicU8::new(0);
    if INSTALLED.swap(1, Ordering::SeqCst) == 0 {
        let _ = log::set_boxed_logger(Box::new(AndroidLogger));
        log::set_max_level(log::LevelFilter::Info);
    }
}



/// Android: the JavaVM and Context are handed over from Kotlin so that
/// `ndk_context` (and through it cpal/Oboe) can initialize, and so the
/// capture layer can drive AudioPlaybackCapture (MediaProjection needs an
/// Activity to show the consent dialog). Without this, every Android audio
/// backend either aborts (cpal) or dead-locks / refuses streams.
/// Android: Kotlin owns the RFCOMM socket (Java API + permissions) and
/// exposes it as a loopback TCP bridge. The Rust supervisor treats that
/// bridge exactly like a native RFCOMM link.
#[cfg(target_os = "android")]
mod android_bt {
    use std::net::TcpStream;
    use std::time::{Duration, Instant};

    use audio_core::rfcomm::{BtListener, BtTransport};
    use audio_core::{CoreError, Result};

    const BRIDGE_PORT: u16 = 47980;

    pub struct AndroidBt;

    impl BtTransport for AndroidBt {
        fn available(&self) -> bool {
            true
        }

        fn local_address(&self) -> Option<String> {
            let address = super::android_java::bt_local_address()?;
            if address.len() == 17 && address != "02:00:00:00:00:00" {
                Some(address)
            } else {
                None
            }
        }

        fn connect(&self, address: &str, channel: u8) -> Result<TcpStream> {
            let started =
                super::android_java::start_bt_bridge(address, channel, BRIDGE_PORT);
            if !started {
                return Err(CoreError::Backend("Kotlin RFCOMM bridge refused".into()));
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match TcpStream::connect(("127.0.0.1", BRIDGE_PORT)) {
                    Ok(stream) => {
                        let _ = stream.set_nodelay(true);
                        return Ok(stream);
                    }
                    Err(error) => {
                        if Instant::now() >= deadline {
                            return Err(CoreError::Backend(format!(
                                "Android RFCOMM bridge {BRIDGE_PORT}: {error}"
                            )));
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        }

        fn listen(&self, _channel: u8) -> Result<Box<dyn BtListener>> {
            // The Android host currently dials the hub; it does not accept.
            Err(CoreError::Backend(
                "Android does not act as a Bluetooth hub".into(),
            ))
        }

        fn set_discoverable(&self, seconds: u32) -> bool {
            super::android_java::set_discoverable(seconds as i32)
        }

        fn scan(&self, seconds: u8) -> Vec<audio_core::rfcomm::BtDevice> {
            let count = (seconds.max(3) as i32) * 1000;
            super::android_java::scan_devices(count)
                .lines()
                .filter_map(|line| {
                    let mut parts = line.splitn(2, '\t');
                    let address = parts.next()?.trim().to_ascii_uppercase();
                    if address.len() != 17 {
                        return None;
                    }
                    Some(audio_core::rfcomm::BtDevice {
                        address,
                        name: parts.next().unwrap_or("").trim().to_string(),
                    })
                })
                .collect()
        }
    }
}



pub fn init_logging() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // Android: log to logcat (a file logger would win the global logger
        // slot and hide every engine log from adb).
        #[cfg(target_os = "android")]
        {
            install_android_logger();
            std::panic::set_hook(Box::new(|info| {
                log::error!("panic: {info}\nthread: {:?}", std::thread::current().name());
            }));
            log::info!("SirenStudio FFI logging initialized (logcat)");
            return;
        }

        #[cfg(not(target_os = "android"))]
        {
            let mut candidates = Vec::new();
            if let Ok(exe) = std::env::current_exe() {
                if let Some(dir) = exe.parent() {
                    candidates.push(dir.join("siren_ui.log"));
                }
            }
            candidates.push(std::path::PathBuf::from("C:/temp/siren_ui.log"));
            candidates.push(std::env::temp_dir().join("siren_ui.log"));
            let mut log_file = None;
            let mut path = candidates[0].clone();
            for candidate in candidates {
                if let Ok(file) = std::fs::File::create(&candidate) {
                    path = candidate;
                    log_file = Some(file);
                    break;
                }
            }
            if let Some(file) = log_file {
                let _ = env_logger::Builder::from_env(
                    env_logger::Env::default().default_filter_or("info"),
                )
                .target(env_logger::Target::Pipe(Box::new(file)))
                .try_init();
            }
            let panic_path = path.clone();
            std::panic::set_hook(Box::new(move |info| {
                use std::io::Write;
                let backtrace = std::backtrace::Backtrace::force_capture();
                let text = format!(
                    "panic: {info}\nthread: {:?}\n{backtrace}\n",
                    std::thread::current().name()
                );
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&panic_path)
                {
                    let _ = file.write_all(text.as_bytes());
                    let _ = file.flush();
                }
            }));
            // Crash reporter: benign first-chance exceptions (thread naming,
            // C++ EH) are filtered inside the handler; see
            // install_crash_reporter.
            #[cfg(windows)]
            install_crash_reporter(&path);
            log::info!("SirenStudio FFI logging initialized");
        }
    });
}

#[cfg(windows)]
fn install_crash_reporter(path: &std::path::Path) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, RtlCaptureStackBackTrace, EXCEPTION_POINTERS,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;

    static CRASH_LOG: OnceLock<std::path::PathBuf> = OnceLock::new();
    static MODULES: OnceLock<Vec<(usize, String)>> = OnceLock::new();
    static IN_CRASH: AtomicBool = AtomicBool::new(false);

    let _ = CRASH_LOG.set(path.to_path_buf());

    // Resolve module names once, outside the handler: the handler itself must not
    // call any loader API (a faulting handler would recurse and hide the crash).
    let mut modules = Vec::new();
    for name in [
        "siren_ui.exe",
        "flutter_windows.dll",
        "rust_lib_siren_ui.dll",
        "ntdll.dll",
        "KERNEL32.DLL",
        "KERNELBASE.dll",
        "d3d11.dll",
        "dxgi.dll",
        "nvwgf2umx.dll",
        "nvoglv64.dll",
        "vulkan-1.dll",
        "AUDIOSES.DLL",
        "MMDevAPI.DLL",
        "wdmaud.drv",
        "ole32.dll",
        "combase.dll",
        "WS2_32.dll",
        "WINMM.dll",
        "dwmapi.dll",
    ] {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        if let Ok(module) = unsafe { GetModuleHandleW(PCWSTR(wide.as_ptr())) } {
            let base = module.0 as usize;
            if base != 0 {
                modules.push((base, name.to_string()));
            }
        }
    }
    modules.sort_by_key(|(base, _)| *base);
    let _ = MODULES.set(modules);

    unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
        // Benign first-chance exceptions (MSVC thread naming, C++/Rust panics)
        // must pass through untouched: doing allocation or file I/O inside
        // their dispatch destabilizes the process.
        if !info.is_null() {
            let pointers = unsafe { &*info };
            if !pointers.ExceptionRecord.is_null() {
                let code = unsafe { (*pointers.ExceptionRecord).ExceptionCode.0 as u32 };
                if code == 0x406D1388 || code == 0xE06D7363 {
                    return 0;
                }
            }
        }
        use std::io::Write;
        if IN_CRASH.swap(true, Ordering::SeqCst) {
            return 0;
        }
        let mut report = String::new();
        report.push_str(&format!(
            "last-trace={} tick={:?}\n",
            LAST_TRACE.load(Ordering::Relaxed),
            START_TIME.get().map(|t| t.elapsed()),
        ));
        if !info.is_null() {
            let pointers = unsafe { &*info };
            if !pointers.ExceptionRecord.is_null() {
                let record = unsafe { &*pointers.ExceptionRecord };
                report.push_str(&format!(
                    "=== crash code=0x{:08X} address=0x{:X}\n",
                    record.ExceptionCode.0 as u32, record.ExceptionAddress as usize
                ));
            }
            if !pointers.ContextRecord.is_null() {
                let context = unsafe { &*pointers.ContextRecord };
                let rip = context.Rip;
                let rsp = context.Rsp;
                let rbp = context.Rbp;
                report.push_str(&format!(
                    "regs rip=0x{rip:X} rsp=0x{rsp:X} rbp=0x{rbp:X}\n"
                ));
                // Raw stack scan: any value on the faulting stack that points
                // into a known module is a candidate return address.
                let modules = MODULES.get();
                let stack = unsafe {
                    std::slice::from_raw_parts(rsp as *const usize, 2048)
                };
                report.push_str("stack-scan:\n");
                let mut hits = 0;
                for (index, value) in stack.iter().enumerate() {
                    if let Some(modules) = modules {
                        if let Some((base, name)) =
                            modules.iter().rev().find(|(base, _)| *base <= *value)
                        {
                            report.push_str(&format!(
                                "  rsp+{:4} {}+0x{:X}\n",
                                index * 8,
                                name,
                                value.saturating_sub(*base)
                            ));
                            hits += 1;
                            if hits > 40 {
                                break;
                            }
                        }
                    }
                }
            }
        }
        let mut frames = [std::ptr::null_mut(); 64];
        let count = unsafe { RtlCaptureStackBackTrace(0, &mut frames, None) };
        for index in 0..count as usize {
            let address = frames[index] as usize;
            let label = MODULES
                .get()
                .and_then(|modules| modules.iter().rev().find(|(base, _)| *base <= address))
                .map(|(base, name)| format!("{name}+0x{:X}", address.saturating_sub(*base)))
                .unwrap_or_else(|| format!("0x{address:X}"));
            report.push_str(&format!("  #{index} {label}\n"));
        }
        if let Some(log_path) = CRASH_LOG.get() {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_path)
            {
                let _ = file.write_all(report.as_bytes());
                let _ = file.flush();
            }
        }
        IN_CRASH.store(false, Ordering::SeqCst);
        0
    }

    unsafe {
        AddVectoredExceptionHandler(1, Some(handler));
    }
}

pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
// ---------------------------------------------------------------------------
// C ABI wrappers
// ---------------------------------------------------------------------------

fn to_c_string(value: String) -> *mut c_char {
    match CString::new(value) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn from_c_string(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned())
}

#[no_mangle]
pub extern "C" fn siren_string_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        unsafe {
            drop(CString::from_raw(pointer));
        }
    }
}

#[no_mangle]
pub extern "C" fn siren_trace(id: u32) {
    LAST_TRACE.store(id, Ordering::Relaxed);
    let _ = START_TIME.get_or_init(std::time::Instant::now);
}

#[no_mangle]
pub extern "C" fn siren_version() -> *mut c_char {
    to_c_string(version())
}

#[no_mangle]
pub extern "C" fn siren_init_logging() {
    init_logging();
}
















#[no_mangle]
pub extern "C" fn siren_begin_start_mesh(config_json: *const c_char) -> *mut c_char {
    to_c_string(begin_start_mesh_inner(from_c_string(config_json).unwrap_or_default()))
}

#[no_mangle]
pub extern "C" fn siren_poll_start_mesh() -> *mut c_char {
    to_c_string(poll_start_mesh_inner())
}

#[no_mangle]
pub extern "C" fn siren_mesh_snapshot(id: u64) -> *mut c_char {
    to_c_string(mesh_snapshot_json_inner(id))
}

#[no_mangle]
pub extern "C" fn siren_mesh_set_gain(id: u64, stream: u64, gain_db: f32) -> i32 {
    i32::from(mesh_set_gain_db_inner(id, stream, gain_db))
}

#[no_mangle]
pub extern "C" fn siren_mesh_set_muted(id: u64, stream: u64, muted: i32) -> i32 {
    i32::from(mesh_set_muted_inner(id, stream, muted != 0))
}

#[no_mangle]
pub extern "C" fn siren_mesh_set_target_ms(id: u64, ms: f32) -> i32 {
    i32::from(mesh_set_target_ms_inner(id, ms))
}

#[no_mangle]
pub extern "C" fn siren_mesh_set_master_gain_db(id: u64, db: f32) -> i32 {
    i32::from(mesh_set_master_gain_db_inner(id, db))
}

#[no_mangle]
pub extern "C" fn siren_mesh_stop(id: u64) -> i32 {
    i32::from(mesh_stop_inner(id))
}

#[no_mangle]
pub extern "C" fn siren_mesh_release(id: u64) -> i32 {
    i32::from(mesh_release_inner(id))
}

#[no_mangle]
pub extern "C" fn siren_mesh_is_running(id: u64) -> i32 {
    i32::from(mesh_is_running_inner(id))
}





