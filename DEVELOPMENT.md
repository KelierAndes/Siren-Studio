# SirenStudio 开发说明

本文面向修改核心、接入界面或移植平台的开发者。用户使用说明见 [README.md](README.md)。

## 1. 目标与边界

SirenStudio 是一个星型音频汇聚系统：

- **发送端**：抓取系统内录 / 麦克风 / 测试音，按 5 ms（可配）切片，编码后通过 UDP 发往中心。
- **中心**：识别多路来源，为每一路维护独立抖动缓冲与时钟漂移校正，线性混音、软限幅，输出到声卡或 WAV。
- **控制层**：当前是 CLI；数据模型（`EngineCommand` / `EngineEvent`）已经按 FFI 与 WebSocket 的需求抽象，后续 Flutter 与 Linux WebUI 复用同一套核心。

当前只实现 Windows 采集。其余模块（协议、DSP、网络、发现、播放、录音）是平台无关的。

## 2. 工程结构

```
Cargo.toml                     # workspace
crates/
  audio-core/                  # 可复用核心库
    src/
      format.rs                # 48k / stereo / f32 常量与转换
      protocol.rs              # UDP 帧头与校验
      types.rs                 # EngineCommand / EngineEvent（FFI / WebUI 数据模型）
      metrics.rs               # 每路与主总线指标（原子变量 + 快照）
      platform.rs              # 定时器精度、线程优先级
      dsp/
        resampler.rs           # 抖动缓冲、PI 漂移校正、Hermite 插值
        mixer.rs               # 多轨混音、软限幅、无锁命令队列
      net/
        codec.rs               # Raw f32 / Opus 编解码封装
        udp.rs                 # UDP 发送、接收、分流、PLC、超时回收
        discovery.rs           # mDNS/DNS-SD
      capture/
        tone.rs                # 正弦测试源
        wasapi_capture.rs      # WASAPI loopback / 麦克风
      output/
        cpal_out.rs            # 播放（采样率/声道适配）
        wav.rs                 # 实时 WAV 录音
      analyze.rs               # 离线 WAV 频谱/静音分析
  siren-cli/                   # siren 可执行文件
scripts/
  build.ps1                    # 定位 cmake + cargo build/test
  verify_e2e.py                # 端到端验证
```

## 3. 传输格式与帧协议

固定格式：**48 000 Hz / 2 声道 / f32**。发送端负责在采集边界转换到该格式；中心输出时再适配设备。

帧头 20 字节（小端）：

| 偏移 | 长度 | 字段 | 说明 |
| :-- | :-- | :-- | :-- |
| 0 | 2 | magic | `"SR"` |
| 2 | 1 | version | 当前 `1` |
| 3 | 1 | codec | `0` = raw f32，`1` = Opus |
| 4 | 2 | frame_ms | 帧长（毫秒），1..60 |
| 6 | 2 | stream_id | 发送端随机会话 ID |
| 8 | 4 | seq | 序号，u32 回绕 |
| 12 | 8 | timestamp_us | 发送端 Unix 微秒时间戳 |
| 20 | … | payload | 一个音频帧 |

- **Raw**：`frame_ms × 48 × 2 × 4` 字节。5 ms → 1920 B，加头 1940 B，200 pps ≈ 3.1 Mbps。
- **Opus**：每包一帧，CELT low-delay，目标 192 kbps；5 ms 帧约 113 B/包（实测 226 kbps 含 UDP/IP 头）。
- 一个 UDP 包只放一帧，接收端按 `(源地址, stream_id)` 分流。
- `frame_ms` 决定接收端丢包时应补多少帧（PLC / 补零）。

协议测试在 `crates/audio-core/src/protocol.rs`，覆盖回环、坏 magic、短包、未知 codec。

## 4. 线程模型与无锁路径

```
UDP 接收线程 ──push──▶ 每流 rtrb 环 (1 s) ──pull──▶ 混音线程（声卡回调或录音时钟）
     │                                                    │
     └─ 指标原子变量 ◀───────── 电平/水位/欠载 ────────────┘

CLI/UI 线程 ── MixerHandle 命令队列（rtrb） ──▶ 混音线程
                           └─ gain / mute 原子变量（无锁读）
```

- 每条远端流一个 **SPSC** 环形缓冲（`rtrb`），生产者是 UDP 接收线程，消费者是混音线程。
- 声道增删通过 128 槽的 `MixerCommand` 队列传递；`StreamChannel`（含预分配的插值缓冲）在音频线程外构造，再整体移入。
- 增益/静音是 `AtomicU32` / `AtomicBool`，界面线程随时改，音频线程每个块读一次。
- 音频线程不做分配、不加锁、不做系统调用。

## 5. 抖动缓冲与漂移校正

实现：`dsp/resampler.rs`。

每路状态：

- 本地输入窗口（最多 8192 帧），从环缓冲按需拉取；
- 目标水位 `target_ms`（默认 30 ms，按输出块计算）；
- 平滑水位 `fill_ema`（一阶低通，时间常数 0.3 s）；
- 分数读取位置 `pos`，每次输出前进 `ratio` 帧；
- 4 点 Catmull-Rom/Hermite 插值，避免线性插值的毛刺；
- PI 控制器：

```
error     = (fill_ema - target) / target
integral  = integral * (1 - 0.2·dt) + 0.005·error·dt     // 带泄漏
ratio     = clamp(1 + 0.02·error + integral, 1 ± 0.002)
```

参数含义：

| 参数 | 值 | 作用 |
| :-- | :-- | :-- |
| `FILL_SMOOTHING_S` | 0.3 s | 抑制 5 ms 包到达抖动导致的音高抖动 |
| `kp` | 0.02 | 比例项 |
| `ki` | 0.005 | 积分项，跟住真实晶振/节拍差 |
| `INTEGRAL_LEAK_PER_S` | 0.2 | 防止启动时的积分风量持续几十秒 |
| `max_correction` | ±0.002（0.2 %，约 3.4 音分） | 音高变化上限 |
| `MAX_FILL_RATIO` | 2× 目标 + 10 ms | 超过则丢弃最旧数据，防止卡顿后延迟长期偏高 |
| 启动水位 | 100 % 目标 | 避免启动后长时间“慢速灌水”造成音高漂移 |

关键行为：

- **启动**：水位达到目标才开始出音；之前输出静音。
- **欠载**：可用帧不足时输出静音，`started=false`，下次重新预缓冲；计入 `underruns` / `underrun_frames`。
- **大缓冲回落**：`buffer_reset=true`，计入 `buffer_resets`；会带来一个约 5 ms 的空隙，这是为了避免数分钟的额外延迟。
- **目标水位语义**：统计里的 `buffer_ms` 是每个输出块开始前的可用水位，和 `--target-ms` 可直接比较。

单元测试（`resampler.rs`）模拟 48 001 / 47 999 Hz 发送端连续 120 s，断言缓冲落在目标附近、ratio 收敛、无欠载；另测 200 ms 洪水触发回落与恢复。

## 6. 混音与限幅

实现：`dsp/mixer.rs`。

```
out[i] = Σ_k stream_k[i] · gain_k        // 32-bit float 线性相加
final[i] = tanh(out[i] · master_gain)
```

- 不使用平均法，避免某路静音时其他路音量跳变。
- 每路贡献分别计算峰值/RMS，主总线计算混音后的峰值/RMS/削波计数。
- `peak_hold` 记录整场运行的最大峰值，用于结束摘要（避免最后一帧静音把电平显示成 −120 dB）。
- 静音通道仍然消费数据，防止环缓冲溢出。

## 7. 网络接收语义

实现：`net/udp.rs`。

- 新流：创建 1 s 环缓冲、`StreamMetrics`、`StreamChannel`，通过 `MixerHandle::add` 交给音频线程。
- 序号：`wrapping_sub` 得到有符号差值。
  - 重复/倒退 < 1000 包：丢弃（乱序迟到）。
  - 倒退 ≥ 1000：视为发送端重启，重置解码器。
  - 间隔 > 1：按 `frame_ms` 计算丢帧。
    - Raw：补零，单次上限 100 ms。
    - Opus：最多 6 帧 PLC（`opus_decode_float(NULL, 0, …)`），其后重置解码器并补零。
- 3 秒无包：从混音器与指标表移除，计入超时。
- 每路指标：包数、丢帧、PLC 帧、溢出包、最后收包时间、水位、ratio、欠载、回落次数。

Opus 封装在 `net/codec.rs`，通过 `opus-head-sys`（vendored libopus）调用，`unsafe` 只出现在封装内部。编码参数：48 kHz / stereo / `OPUS_APPLICATION_RESTRICTED_LOWDELAY` / 192 kbps / complexity 5 / music。

## 8. Windows 采集

实现：`capture/wasapi_capture.rs`。

- **系统内录**：取默认 Render 端点，但 `initialize_client` 传入 `Direction::Capture` —— 这是 WASAPI 的 loopback 开关。不能传 `Direction::Render`，否则拿到的是渲染流，`get_audiocaptureclient` 会返回 `AUDCLNT_E_WRONG_ENDPOINT_TYPE (0x88890003)`。
- 请求 **48 kHz / 2ch / f32**，`PollingShared { autoconvert: true }`。失败时回退到设备 mix format，软件转换 16/24/32-bit PCM 或 f32，并要求 mix 采样率必须是 48 kHz（否则给出明确错误）。
- 轮询间隔 2 ms；`AUDCLNT_BUFFERFLAGS_SILENT` 会显式补零，保持时间线。
- 采集线程把 f32 帧推入 2 s 环缓冲，发送循环按 5 ms 节拍拉取；环不够时补零。
- 采集线程提升为 `THREAD_PRIORITY_ABOVE_NORMAL`，进程调用 `timeBeginPeriod(1)`。

麦克风走默认 Capture 端点、同样的初始化与转换逻辑。

## 9. 输出与录音

- **播放**：`output/cpal_out.rs`。使用设备默认配置；若设备不是 48 kHz 或不是立体声，用线性插值把混音结果适配到设备采样率，并按设备声道数映射（单声道取 L/R 平均，多于两声道其余补零）。
- **录音**：`output/wav.rs`。软件时钟每 5 ms 渲染一块并写 32-bit float WAV。如果操作系统卡顿导致落后，**不补渲染积压块**（补渲染会瞬间抽干抖动缓冲），而是把截止时间重置到现在。
- 进程启动时 `timeBeginPeriod(1)`，关键线程 `ABOVE_NORMAL`，减少包/录音抖动。

## 10. mDNS 发现

实现：`net/discovery.rs`。

- 服务类型：`_sirenmesh._udp.local.`
- TXT：`role=center|sender`、`name=…`、`codec=raw,opus`、`version=…`
- 中心启动时广播，`--to auto` 浏览服务并选择第一个 `role=center`。
- 地址解析优先私有 IPv4，排除回环；找不到其他地址时回退到回环（同机测试）。
- 当前 `discover` 只显示节点；发送端尚未广播自身（没有监听端口）。

## 11. CLI 与统计输出

`center --stats-json <file>` 每秒写一行：

```json
{"ts_ms":..., "elapsed_s":..., "master":{"rms_db":...,"peak_db":...,"peak_hold_db":...,...},
 "streams":[{"label":"127.0.0.1:50000","codec":"raw","rms_db":...,"buffer_ms":...,
             "target_ms":30,"ratio":1.00002,"underruns":0,"buffer_resets":0,...}]}
```

退出时追加一行 `"event":"summary"`，包含 `bind`、`name`、`seconds`、`master`、`streams`、`receiver`、`record`。发送端 `--json` 的 stdout 只有最终摘要，进度信息全部在 stderr。

## 12. 为 Flutter / Linux WebUI 预留的接口

`types.rs` 已定义纯数据模型：

- `EngineCommand`：`SetGain`、`SetMute`、`SetTargetBufferMs`、`SetMasterGainDb`、`RemoveStream`。
- `EngineEvent`：`StreamAdded`、`StreamRemoved`、`Stats`、`Error`。

核心侧控制面：

- `MixerHandle::add/remove/set_target_ms/set_master_gain_db`
- `StreamMetrics::set_gain_db` / `muted` 原子变量
- `MixerMetrics` / `MetricsRegistry::snapshots()`（60 fps 节流由上层做）
- `net::udp::spawn`、`capture::CapturePipe`、`output::start_playback` / `record_to_wav`

推荐的下一层封装（尚未实现）：

```
Engine::start(CenterConfig) -> EngineHandle
EngineHandle::send(EngineCommand)
EngineHandle::subscribe() -> Receiver<EngineEvent>   // 30–60 fps 节流
EngineHandle::snapshot() -> (MasterSnapshot, Vec<StreamSnapshot>)
```

- **Windows 桌面（已实现）**：`ui-flutter/` 是 Flutter 控制台，通过 `ui-flutter/rust` 的**手写 C ABI**（`#[no_mangle] extern "C" siren_*`，字符串用 `CString`、引擎实例存全局注册表按 `u64` 寻址、快照跨桥为 JSON）接入同一核心；Dart 侧绑定在 `lib/rust_api.dart`（dart:ffi，Windows 加载 `rust_lib_siren_ui.dll`，Android 加载 `librust_lib_siren_ui.so`）。长任务（中心/发送端启动、mDNS 发现）用 `begin_*` + `poll_*` JSON 状态对，Dart 定时轮询。音频线程不进入 Dart。
- **Android（已实现，发送端验证）**：`scripts/build_apk.ps1`（内部 `flutter build apk --release --no-tree-shake-icons`），minSdk 26（cpal/oboe 链接 `-laaudio` 需要 API 26）。已验证链路：手机 App 测试音 → WiFi UDP → Windows 中心混音播放。Android 限制：
  - **本地播放已实现**：`MainActivity` 通过 `System.loadLibrary` 触发 `JNI_OnLoad`（Rust 侧拿到 JavaVM），`onCreate` 调用 `nativeSetContext(applicationContext)` 初始化 `ndk_context`（JavaVM + 全局 Context），之后 cpal 的 Oboe 后端即可正常打开播放设备。注意 JNI 符号名对包名下划线要做 `_1` 转义（`Java_com_sirenstudio_siren_1ui_MainActivity_...`）。
  - 已验证：手机组网「本地播放」开启后，PC 的测试音经手机扬声器实时播放（混音电平正常）。
  - 设备枚举仍返回空（cpal 枚举在 Android 上不可用，播放固定走系统默认输出）。
  - **采集已实现**（`crates/audio-core/src/capture/android_capture.rs`）：
    - 麦克风：cpal/Oboe 输入流，软件转 48 kHz 立体声 f32；
    - 系统内录：Kotlin `AudioPlaybackCapture`（MediaProjection 用户授权）→ JNI `nativePushCapture` → 进程级环形缓冲。Android 14 要求 MediaProjection 必须由 `mediaProjection` 类型的前台服务持有（`CaptureService`），且必须用 **PCM 16-bit**（MIUI 上 FLOAT 采集返回全零）。
    - 平台能力通过「Java 桥」注册进 audio-core（`install_java_bridge`），audio-core 不依赖 jni crate。
    - 被 JNI 反射调用的 Kotlin 方法必须加 `@Keep`（R8 混淆会改名，否则 `NoSuchMethodError`）。
    - 已验证：手机麦克风（房间声 -21 dB）与系统内录（回采 440 Hz tone -15 dB）均实时推送到 PC 混音播放。
  - 两个鲁棒性修复：UDP 发送遇瞬时 `EAGAIN`（ARP 解析/网卡重连）改为连续 1 秒失败才放弃；`CapturePipe` 停止时不再 join 采集线程（Oboe 关闭可能长时间阻塞，会卡住发送线程的收尾）。
  - 经验教训（已废弃的备选路径）：AAudio 直连在 MIUI 上不可用——legacy AudioTrack 包装器在第一次数据回调后返回垃圾帧数并自停（`onMoreData() callback returned invalid result`），MMAP 又被 ROM 拒绝；OpenSL ES 的 `GetInterface(SL_IID_ENGINE)` 会无限挂起。均因缺少 Java 上下文所致，勿再尝试纯原生路径。
  - `rust_builder/cargokit` 有两处本地补丁：`gradle/plugin.gradle` 补传 `FLUTTER_ROOT`（gradle daemon 无此环境变量时 dart 静默失败）；`build_tool/lib/src/android_environment.dart` 为 cmake 类 `-sys` crate 传 `ANDROID_NDK`/`CMAKE_TOOLCHAIN_FILE`/Ninja，并用 NDK 的 API 级 `*-clang.cmd` shim 直接作 cargo linker。
  - 手机端 UI 是独立的移动层 `lib/mobile/`（底部导航：中心/发送/发现 + 日志抽屉），`main.dart` 按 `Platform.isAndroid/isIOS` 分流，桌面 `shell.dart` 不受影响；两端共用 AppModel 与主题令牌。
- **Linux（已实现，无界面守护 + WebUI）**：同一份 Rust 核心以两种形态运行：
  - `siren mesh`：纯 CLI 守护节点（`--source tone|loopback|mic`、`--no-play`、`--peer`、`--duration`、JSON 统计）。
  - `siren web --listen 0.0.0.0:8080 [--autostart]`：内置 HTTP 控制台，浏览器完成加入/离开、音源、缓冲目标、主增益、每路增益/静音与实时电平。页面 `crates/siren-cli/src/webui/index.html` 由 `include_str!` 打进二进制，无前端构建步骤。API：`GET /api/status`，`POST /api/start|api/stop|api/controls|api/streams/{id}`（页面 700 ms 轮询 JSON）。
  - 采集/播放：`libpulse`（`capture/pulse_capture.rs`、`output/pulse_out.rs`）。系统内录 = 默认 sink 的 monitor（`@DEFAULT_MONITOR@`），麦克风 = 默认 source；统一 48 kHz 立体声 f32，重采样交给服务端。播放线程 2.5 ms 块、20 ms 目标缓冲（兼顾 NAS 调度与回声消除时延稳定性）。
  - 构建：Debian 12 实测 `apt install build-essential pkg-config cmake libpulse-dev libasound2-dev pulseaudio pulseaudio-utils` + rustup；`cargo build --release -p siren-cli`。运行需要 PulseAudio ≥ 15 或 PipeWire 的 pulse 兼容层（`@DEFAULT_MONITOR@`）。
  - 无头部署：`scripts/linux/run-headless.sh`（启动 per-user PulseAudio + `siren_test` 空设备 + WebUI）；`scripts/linux/siren-web.service` 为 systemd 模板（`XDG_RUNTIME_DIR` 必须与 PulseAudio 服务一致）。
  - **可选 PipeWire 节点隔离**：`--isolate pipewire --output-sink <sink> [--capture-sink siren_capture]`。`crates/siren-cli/src/isolate.rs` 通过 `pactl` 创建虚拟 sink `siren_capture` 并设为默认（其他应用播放进来），加载 `module-loopback source=siren_capture.monitor sink=<output>` 让这些声音仍能从播放设备听到；本机 mesh 播放直接走 `<output>`，因此不会进入采集。采集源显式设为 `siren_capture.monitor`，并通过 `SIREN_ECHO_CANCEL=0` 关闭 DSP。退出时卸载回环/虚拟 sink、恢复默认设备；`$XDG_RUNTIME_DIR/siren-isolate.pid` 限制单实例；无 PipeWire/`pactl` 时自动回退 monitor + DSP。
  - 隔离实测（Debian 12 + PipeWire 1.4.2）：节点播放对端 550 Hz 时 `play_sink.monitor` **−15.2 dB**、`siren_capture.monitor` **−180 dB**（数字静音），发送电平 **−120 dB**；第三方 700 Hz 进入默认 `siren_capture` 后发送电平 **−17 dB**；siren 进程 CPU 约 **6%**（monitor + DSP 路径约 31%）。
  - 隔离局限：依赖“默认设备 = 采集 sink”；显式选择设备的应用、WirePlumber 的按应用恢复规则可能绕过；运行期间默认设备会被改写。
  - WebUI 的流 ID 在 JSON 中序列化为**字符串**：流 ID 是 u64，超过 JS `Number.MAX_SAFE_INTEGER` 会被四舍五入，导致按流静音/增益请求打到不存在的 ID（已实测踩中）。页面同时只在结构（流集合/静音状态）变化时重建，其余数值原地更新，避免多网卡 mDNS 端点抖动造成整页重建、点击失效。
  - 真机验证（192.168.2.196，Debian 12，headless）：WebUI 真实点击“离开/选系统内录/改名/加入”通过；播放对端音乐时本机发送电平 **−51 ～ −71 dBFS**（回声已消除），第三方 `playtone` 700 Hz 穿过为 **−20 dBFS**，无失锁/静音抖动。
  - WebUI 不带鉴权，默认面向可信局域网；需要跨网暴露时请置于反向代理/防火墙之后。
  - **Wi-Fi + 蓝牙双栈自动**：不存在接入方式选择。`HybridNode`（`audio-core::hybrid`）同时运行 Wi-Fi mesh 与蓝牙 overlay：
    - mDNS TXT 广播 `bit` / `bt`（本机适配器地址）/ `btch`（RFCOMM 通道 16）；蓝牙能力集合中 **uid 最小者自动成为 hub** 并监听，其余 leaf 自动拨号；本机地址不可读（Android 遮蔽）时只做 leaf。角色切换只拆除/重建蓝牙链路，Wi-Fi 音频不中断。
    - RFCOMM 内置于 `audio-core::rfcomm`（Linux 原生 `AF_BLUETOOTH` socket、Windows Winsock `AF_BTH`、Android 由 Kotlin `startBtBridge` 桥到 `127.0.0.1:47980`），原生 socket 经 `bridge_raw` 双向泵到回环 TCP，再作为 `Tunnel` 的 `Link`。
    - 协议 v2 帧头新增 8 字节 `origin`（节点 uid）；mesh 发送端每 tick 用共享 `FramePacker` **编码一次**，同一段文同时发给 Wi-Fi peers 与蓝牙隧道；接收端按 `(origin, stream_id, seq)` 去重（`PeerKey::Origin`），迟到副本直接丢弃，仅序号真正回绕才重置解码器。
    - `Tunnel` 支持运行中 `add_link`/`clear_links`（hub 随时接受新 leaf），每链路写队列有界（8 帧），RFCOMM socket `SO_SNDBUF` 4 KB——把蓝牙积压限制在百毫秒级，避免链路变慢时把几秒旧音频堆在 kernel 里。
    - 状态在 `MeshSnapshot.bt`（角色/状态/hub/链路数/收发字节），三端 UI 展示；流条目带 `path`（wifi/bt/local）。
    - **分通道编码且共用序号**：Wi-Fi 走用户选择的编码；蓝牙载体用 `FramePacker::with_stream` 编 **Opus 32 kbps**，并调用 `build_compact_with_seq(pcm, primary_seq)` **使用主载波本 tick 的 seq**。两个载体各自计数会让蓝牙副本永久落后，接收端把它当迟到副本整段丢弃——Wi-Fi 一断就要静默“等序号追上”。实测修复前 PC 侧落后 559 帧（2.8 s）。
    - **解码器按 codec 分槽**：流可能先由蓝牙（Opus）建立、之后收到 Wi-Fi（raw）副本；`decoders[codec]` 必须按 codec id 放置。放在索引 0 会让 raw 包用 Opus 解码器解码而全部失败（表现为 rms −120、海量 lost/resets）。解码失败也必须推进 `last_seq`，否则每个后续包都会重新补零。
    - **切载波重同步**：两个载波的到达时间可能相差数百毫秒到数秒（蓝牙中继排队）。载波切换且序号差 >100 帧时走 `MixerCommand::Flush` 清空该流积压并用新载波继续，而不是补零/丢弃等待；只有“落后的载波”在活跃载波已静默 >200 ms 时才接受，避免正常 Wi-Fi 期间被陈旧蓝牙突发反复拉回。
    - **发送失败退避**：目的地址消失（Wi-Fi 关闭、对端离线）时每 5 ms 一次的 `sendto` 失败会在手机端刷出 200 条/秒的日志；连续 20 次失败后 `UdpSender` 暂停 1 s，失败日志最多 1 条/秒。
    - **缓冲目标实时生效**：`ReceiverConfig.live_target_ms` 与 `CenterShared.target_ms` 共享同一个原子值；UI 修改目标会作用于已连接的流，蓝牙降级恢复时也回到该值（以前会退回启动时的 30 ms）。
    - **占空比**：`BT_DUTY_CYCLE = 1`，蓝牙载体每 tick 都发。曾经的 1/10 占空比会在 Wi-Fi 对端存在时把回退链路“饿死”。
    - **N−1 混音转发（hub）**：hub 为每个蓝牙链路只发 **一路混音**——`(hub 本地 + 其它叶子)`、排除目的叶子自身。接收端解码后的 PCM 进 `MixTap`（每 origin 最近 10 ms），hub 发送线程每 10 ms 组一次混音、用 **v4 头（28 B，含 64 位 origin 掩码）** 编码为 24 kbps Opus，并通过本地 0xB3 定向信封只投给该叶子的链路。叶子按掩码仲裁：被混音覆盖且对应直连流缺失时才播放混音，缺失以外的直连流继续播放（hub 断网时仍直连的对端不会陪葬）；所有被覆盖流恢复后立即清空混音缓冲。
    - **按需下行**：只有目的叶子的上行已经走蓝牙（hub 侧 `last_wifi_seen > 300 ms`）时才向该链路发混音；Wi-Fi 健康时下行 **0 B/s**。Wi-Fi 断开后 300 ms 内切换为全速率混音。
    - **链路健康**：看门狗改为双向——6 s 内 **收发都 < 4 KB** 才清空链路（N−1 混合态下行按设计为 0，只看接收会误判死链并触发重拨）。叶子多链路只 `keep_latest_link()`（旧实现 `clear_links()` 会把刚建好的链路一起拆掉，形成拨号回环）；hub 每 5 s 清理 **>10 s 无数据** 的陈旧链路，不主动关闭活动链路。
    - **选举迟滞**：hub 保持直至更小 uid 连续 2 tick 出现；leaf 连接保持、hub 失踪 3 tick 才重选；拨号 3 s 节流、连续失败 3 次重新选举。
    - **离线入网**：叶子成功连接后把 hub 记入 `hub.json`（Android 经 Kotlin `stateDir()` 存 filesDir），无 Wi-Fi（无 mDNS）时按记忆轮拨最近 hub。
    - **蓝牙扫描发现（mDNS 备选）**：hub 通过 `BtTransport::set_discoverable` 周期性可被发现（Linux `bluetoothctl discoverable on`、Windows `BluetoothEnableDiscovery`；Android 无静默可发现 API，只作叶子）；叶子在无拓扑且无记忆（或记忆拨号连续失败）时执行经典蓝牙 inquiry（`BtTransport::scan`），每次最多探测 6 个候选的 RFCOMM 16：连接失败或成功后 2.5s 内没有收到任何帧就判定为非 Siren 设备并拉黑 10 分钟（多轮扫描会继续往后走，直至命中中心），命中后写入 `hub.json`。调试：`siren bt-scan --seconds 8`。Android 清单必须给 `BLUETOOTH_SCAN` 加 `android:usesPermissionFlags="neverForLocation"`，否则 12+ 上 `startDiscovery()` 返回 true 但收不到任何结果。
  - **三设备状态矩阵实测**（2026-10-29，Linux hub + Windows 有线 + Android，5 ms 帧、缓冲目标 98 ms）：
    - 混合（Wi-Fi+BT 同发）：两条流 `path=wifi`，0 丢帧、0 欠载；两端日志无载波切换抖动。
    - 手机 Wi-Fi 断开 30 s：3 s 内切到 `path=bt`，目标自动升到 250–300 ms，丢帧 0；恢复后一次性重同步 ≈20–360 ms，无长时间静默。
    - 中心端 Wi-Fi 阻断 30 s：蓝牙链路保持、流不超时；叶子切到 BT 仅一次 ≈0.65 s 重同步；PC 侧因 RF 抖动欠载约 1 次/秒。
    - 三端纯蓝牙 60 s：hub 两条链路全程保持，手机流 0 丢帧/2 次欠载，PC 流 0 丢帧/约 0.14 次/秒欠载（PC↔NAS RF 链路本身偏弱，缓冲升到 400 ms）。
    - **N−1 终版 5 分钟矩阵**（2026-10-29，24 kbps / 混音 10 ms / v4 掩码）：混合→手机断 Wi-Fi→恢复→hub 断网→恢复→三端纯蓝牙→恢复。全程 2 条链路、无流超时/重建、无拨号回环；丢帧仅两次切换各一次（手机 480 帧、PC 19,200 帧），欠载手机 **3** 次、PC **29** 次（5 min）；蓝牙下行混合态 **0 B/s**、单混音 **≈5.4 KB/s**、双混音 **≈10–11.6 KB/s**（改造前混合态下行 ≈18 KB/s、双流中继 ≈36 KB/s）。
    - 已知物理限制：PC↔NAS 的 RFCOMM 抖动仍会让该路偶发欠载；下一步可用 hub 端 N−1 混音转发（每链路只发 1 路混音）减半空口负载。
  - **自动角色（不显式选择）**：按 mesh 内最小节点编号推导——本机 `bt_bit` == 中心 `bt_hub_bit` 时自动监听，其余节点连接中心蓝牙地址；修改中心/编号会重启蓝牙链路（短暂中断）。实测：Linux（bit1=hub）监听、手机（bit2）连接，双向 **−15 dB** 连续。
  - 说明：当前为“编号+中心地址”推导角色；跨节点自动交换蓝牙地址/最小节点接管（真正拓扑收敛）与 Android/Windows 应用内选择仍是后续工作。

### 节点退出归因与自愈

`center sink: loop exited` + `mesh: MeshEngine dropped` 曾被当成“流超时/蓝牙链路结束时引擎自毁”。实际语义是：**center sink 的循环只在 `stop` 被置位时退出**，所以这两行必然来自一次显式 stop/release；流超时（`net/udp.rs` 的 `peers.retain`）与链路结束（`bt.rs` 的 `Event::LinkClosed`）只删除该流/该链路，从不触碰引擎。为此现在每次退出都带调用者：

- 引擎侧：`CenterEngine::stop` / `MeshEngine::stop` / `HybridNode::stop` 只在 false→true 的那一次记 `... stop requested by <线程名>` 并附 backtrace；`HybridNode` 另有 `HybridNode dropped`。
- 载体侧：FFI 记 `ffi: mesh_stop(id) requested` / `ffi: mesh_release(id) requested`，WebUI 记 `webui: leave requested`。线程名即可分辨来源（FFI 调用来自未命名线程，WebUI 来自 `tokio-rt-worker`，CLI 来自 `main`）。
- 客户端策略（Flutter `AppModel`）：旧代码 `_poll` 里 `if (!running && !stopping) meshRelease`，而 `stopping = running && stop` 恒等于 `running`，条件实际就是“只要没在跑就销毁节点”，任何抖动都会把节点拆掉。现在只有用户点过“离开组网”（`_leaveRequested`）才收尾释放；引擎自己停下时保留注册、提示“请重新点击「加入组网」”，重新加入前先把旧 id 停掉并释放（否则端口/注册位会被死节点占住）。
- WebUI 页面：加入/离开按钮先 `await poll()` 拿服务端真实状态再决定动作，不再用最多滞后一个轮询周期的缓存——旧写法会把“加入”点击变成一次 `/api/stop` 且不再 start，表现即“点了加入但仍是未组网”。
- 守护进程自愈：`webui.rs` 每 2 s 检查已注册节点的 `snapshot.running`，为 false 就用同一份配置重新入网并记 `webui: mesh node stopped unexpectedly, re-joining`；用户显式离开时节点已从状态里取走，看门狗不会与离开对抗。
- 释放路径：`mesh_release_inner` 先在注册表锁内取出节点、在锁外 drop；`HybridNode::drop` 用后台 reaper 线程 join 监督线程（它可能正卡在 RFCOMM 连接里），避免释放卡住 UI 的状态轮询。
- 实测（2026-11-02，Linux hub + Windows 桌面端 + Android 三端）：杀掉叶子设备后，两端各出现 `stream ... timed out` 与 `bt: link ... reader ended`，三端 `running` 保持 true、无 `MeshEngine dropped`、桌面端进程存活；桌面端真实点击“离开组网→加入组网”得到完整归因链（`ffi: mesh_stop(1)` → `hybrid/mesh/center: stop requested` → `ffi: mesh_release(1)` → `MeshEngine dropped` → `ffi: mesh 2 registered`）；WebUI 同样点击链为 `webui: leave requested` → `stop requested by tokio-rt-worker` → 重新入网 `center: ready`，看门狗未误触发。

### 图标字体（release 构建）

- release 构建默认做 **图标树摇**；通过字段/记录间接传递的 `IconData` 可能被漏掉，导致按钮、面板标题、提示图标显示为空白。构建产物里的 `data/flutter_assets/fonts/MaterialIcons-Regular.otf` 若只有几 KB（完整字体约 1.6 MB）即可确认。
- 本项目统一用 `scripts/build_ui.ps1` / `scripts/build_apk.ps1`（带 `--no-tree-shake-icons`）构建；不要直接用 `flutter build ... --release`。`pubspec.yaml` 的 `uses-material-design: true` 必须保留。

### 拟物面板 UI（PC / 手机两套）

- 设计源：`C:/Users/KelierAndes/Documents/Qoder/2026-10-28/2037d347`（BASSMAKER）。已移植到 `ui-flutter/lib/ui_kit/`：`theme.dart`（金属材质/亮暗色 token）、`controls.dart`（旋钮/推子/胶囊/LCD）、`screen_display.dart`（EQ 曲线屏）、`module_block.dart`、`device.dart`（桌面整机 1180×760）、`mobile_device.dart`（手机 390×844）。
- 平台分流：Windows/Linux/macOS 用 `PulsarDevice` + 自绘标题栏（`window_manager` 无边框，1240×800 起）；Android 用 `MobileDevice` 全屏（SafeArea 内等比缩放）。
- **三端同构（2026-10-31）**：`ui-flutter/lib/ui_kit/panels.dart` 抽出共用组件 `TopStatusPill` / `LocalCard` / `MeshCard` / `BottomStrip`，桌面 `device.dart` 与手机 `mobile_device.dart` 都用它；Linux WebUI（`webui/index.html`）按同一结构用 CSS 复刻。三端都是：顶栏品牌 + 状态胶囊（内含 `‹ 页码 ›`）+ 远端路数 → EQ 屏 → 一排卡片（两张设备卡 + 两张系统卡）→ 系统卡下方的底栏。
- **桌面布局按 BASSMAKER 原始效果图比例还原**：画布 1180×760，面板内 1036×596 = 屏 316 + 间隙 12 + 卡片行 268（屏占 53%，与参考图 150/273 一致）。卡片行：`IN 34 | 设备卡 242 | 设备卡 242 | 系统卡 217 | 系统卡 217 | OUT 34`（间隙 10，合计 1036）；两张窄系统卡下方压一条 444×66 的底栏，对应参考图的 `HP —— LP / DRY / WET / BYPASS`。
  - **设备卡**（`stream_card.dart`，242×268；手机 362×232）：标题行右侧三枚 = 连接状态灯 / 信号质量灯 / 一键静音（扬声器图标，静音整键变红、卡片变暗、曲线转淡）；主体左列只有一颗大增益旋钮（`-60 / +12` + `增益`），右列是电平窗（`当前电平` + 数值 + 电平条）与路径/丢帧/欠载。卡片宽度 ≥ 约 330 时电平窗与事实条自动改为左右排（手机就是这样）。
  - **系统卡**（217×189，无旋钮）：本机 = 节点名输入框 + 音源胶囊 + 音调滑块；组网 = 角色/链路读数窗 + 加入/离开按钮。缓冲、主增益、本地播放、BYPASS 全部在底栏；手机底栏用 `twoRow` 排成两行。
  - **手机**：同两页、同一批卡片，纵向堆叠在 `SingleChildScrollView` 里，屏下方多两条 IN/OUT 横向电平表；标题栏有亮/暗切换。设置弹层已删除。
  - **WebUI**：同一结构，设备卡的“旋钮”用 CSS 圆盘 + 指针，上下拖动/滚轮改增益（300 ms 防抖后 POST `/api/streams/{id}`）；EQ 曲线用快照里的 `spectrum` 实时绘制，`◀ 页码 ▶` 翻页；本地播放与 BYPASS 与 Flutter 语义一致（本地播放在下次加入时生效）。
  - **信号质量**：没有 RSSI 可用，按最近事件估算——丢帧/欠载计数一旦增长就记一次事件，1 s 内为红、4 s 内为黄、否则绿，流不在线则灭。
- 主题：标题栏亮/暗切换；浅色 token 的强调色使用压深版本。`SIREN_UI_FORCE_MOBILE=1` 可在桌面预览手机布局。
- 旧 `shell.dart` / `mobile_shell.dart` 不再被 `main.dart` 引用，保留仅作历史实现参考；新代码不依赖 BASSMAKER 工程本身。

### 三端 UI 一致性

- 设计 token 以 `ui-flutter/lib/theme.dart` 为准，Linux WebUI 的 CSS 变量（`--bg/--panel/--text/--muted/--teal/--blue/--amber/--red/--radius`）逐项对齐；卡片 18 px、控件 11–14 px、字体 `Segoe UI` + `Consolas`。
- 流面板不再显示 `origin#stream`：快照同时给出 `origin`（u64，**16 位十六进制字符串**，避免 JS 精度丢失）与 `stream_id`，peer 给出 `uid`；三端按 `origin == uid` 映射到节点名，`stream_id == 0xFFF0` 标注“混音”，mDNS 的 ` (mesh)` 后缀在展示层剥离。
- 品牌资产统一：teal→blue 渐变圆角方块 + 深色波形。Windows 用多尺寸 `app_icon.ico` 且 `WNDCLASSEX` 同时设置 `hIcon/hIconSm`；Android 替换 mipmap 图标；WebUI 用同款内联 SVG favicon。

### 防回环（四层防线）

回环的根因是**本机把自己的播放又录了回去**（WASAPI loopback / AudioPlaybackCapture），再发给其他节点，几百毫秒内叠加成啸叫。本项目按四层处理：

- **L1 采集层排除（主路径，零音质损耗）**
  - Windows：`capture/process_loopback.rs` 通过 `AudioClient::new_application_loopback_client(pid, false)` 激活进程回环端点（`PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE`），音频引擎在混音阶段就剔除本进程树，其他应用（浏览器、播放器）照常采集。该虚拟端点不支持 `get_device_period` / `get_mixformat`，固定 48 kHz / 立体声 / f32 轮询读取；Windows 10 build 20348 以下会激活失败，自动回退到 L4。
  - Android：两层。`AndroidManifest.xml` 的 `<application android:allowAudioPlaybackCapture="false">` 是应用级 `ALLOW_CAPTURE_BY_NONE`，系统在混音阶段就把本应用所有播放轨从内录中剥离；`AudioPlaybackCaptureConfiguration.excludeUid(Process.myUid())` 继续保留作为第二层。
    - **MIUI 实测踩坑**：仅靠 `excludeUid` 时，手机播放 PC 的 1 kHz（手机输出 −17 dB）会把自身播放回传（PC 收到 −4.2 dB）；把手机媒体音量调 0 后回传才消失——说明捕获发生在音量之后，`excludeUid` 在 AOSP 规则里看似注册成功（dex 有该调用、`dumpsys audio` 有 “exclude UID 10360”），但 AudioFlinger 的 Remote Submix 仍混入了本应用播放。
    - **Manifest 属性验证（媒体音量 10）**：手机仍在播放 −17 dB，而 Linux/PC 观察到的手机上行流为 **−120 dBFS**（纯数字静音），logcat 无任何 `echo:` 活动——DSP 完全未介入，属于真正的 L1 剥离。该属性自 API 29 起生效（系统内录本身也要求 API 29+）。
    - 播放端进一步兜底（AAudio `AAUDIO_ALLOW_CAPTURE_BY_NONE` / 改用非 MEDIA usage）需要在自定义 AAudio 播放流上设置；当前 cpal 的 Android 后端未暴露该钩子，Manifest 已覆盖同一策略，暂不需要。
    - **系统内录默认不再跑 DSP**：L1 已保证采集里没有本机播放；而 DSP 的周期锁扫描/卷积会与 AAudio 回调抢 CPU，实测手机进程 106% → 48%、输出欠载从每秒多次降到 0。`SIREN_ANDROID_ECHO=1` 可为忽略 Manifest 策略的 ROM 强制重新启用。
  - Linux（可选，PipeWire）：`--isolate pipewire` 创建独立采集 sink，本机播放走另一个 sink，二者在节点图上分离；实测采集为数字静音（−180 dBFS）。默认路径仍是默认 sink 的 monitor + L4 DSP（兼容纯 PulseAudio/无头服务器）。
  - 真机验证（PC ↔ 手机）：手机播放 PC 送来的 1 kHz 时，PC 收到的手机流为 **−∞ dB**（手机不再回采自己的播放）；同时 PC 上第三方 `playtone` 进程的 1 kHz 仍被正常采集并送达手机（**−15 dB**）。Windows 侧同理：PC 播放手机流时自身采集为 −∞，第三方音频照常穿过。
- **L4 DSP 兜底（旧系统 / 进程回环不可用 / Linux）**：`dsp/echo.rs`。渲染端把送去播放的帧写入参考环形缓冲；采集端互相关锁定回采延迟（1 ms 粗搜 + 样本精搜），再用每声道 256-tap NLMS 吸收增益、抖动与重采样误差。
  - **锁的稳定性**：周期性内容（音乐）会在整段 0–1 s 搜索里出现多个高相关峰，锁到重复峰会完全抵消失败。因此重锁时若旧偏移的相关性仍 ≥ 0.6 就保持不变；首次搜索选择“最早达到最高分 −0.05”的峰（直达路径）；需要重新搜索时先在上一次偏移 ±20 ms 内找。
  - **欠载恢复**：音频引擎欠载会在 monitor 里插入参考中没有的静音，时间轴断裂。连续 12 个块（约 60 ms）相关性近零（<0.05）即快速重锁，而不是等 10 s 周期重锁。
  - **失锁静音只用于“确认真实回声却抵消不动”**（该块与参考相关性 ≥0.25 且 ERLE 差）；相关性低通常是第三方音频或静音，必须原样放行，否则会把要发送的内容静音。
  - Linux 的 monitor 是数字回环：20 ms 播放缓冲 + 2.5 ms 写块把参考与采集的时延抖动控制在滤波器窗口内。实测发送电平在播放对端音乐时 −51 ～ −71 dBFS；独立探针（`cargo run --release -p audio-core --example pulse_echo_probe`）在纯净噪声下可达 >140 dB 抵消。Windows 端点回环回退、Android 防御路径共用该模块。
- **L2 协议层 / L3 拓扑层**：mesh 的每个节点只向所有对端广播**本机原生采集流**，从不转发“混音结果”，网络天然是 1-hop 无环拓扑，因此当前不需要 RTP CSRC 血统位掩码或 Mix-Minus 矩阵。若将来加入中继/转发模式，必须同时补 `OriginMask`（收包发现含自身位即丢弃）与按目标计算的 N−1 混音。

### 蓝牙链式多跳（实验，Phase 1：路由核心）

蓝牙只作为**字节链路**（RFCOMM / LE L2CAP），隧道把现有 Opus/UDP 包原样搬运，音频 DSP/混音/UDP 协议完全复用：

- `crates/audio-core/src/bt.rs`：16 字节路由封装（magic/version/len、64-bit Origin 掩码、TTL、hop），链路为任意可靠字节流（当前 `TcpStream`，平台桥接负责 RFCOMM↔TCP）。
- 规则：`origin & self_bit != 0` 丢弃（自己的声音绕回）、TTL=0 丢弃、按 `(origin, stream_id, seq)` 去重；收到的包注入本地 mesh 并转发到除来源外的所有链路。
- `siren bt --bit N --port 480xx (--listen-link 482xx | --link ip:port) --source tone|loopback|mic --no-play`：本机 mesh 关闭 mDNS（`discovery=false`），只把本地流发给隧道；隧道负责多跳。
- 三节点本地链测试（A 440 Hz ↔ Hub 550 Hz ↔ C 880 Hz，Opus 10 ms）：A/C/Hub 各看到 **2 路 −15 dB** 的远端流，master ≈ −12 dB；A 的音频经 Hub 到达 C，Origin/TTL/去重正常，退出无残留。
- **Phase 1 限制**：逐包转发而非 Mix-and-Forward，链路带宽随活跃源数量增长（3 节点约 2–3 路 Opus/链路）；后续可加 N-1 混音中继。Opus 10 ms 单/立体声 64–128 kbps 足以跑 RFCOMM。
- **平台桥接契约**：每个蓝牙链路在本地暴露为一个 TCP 字节流。Android 由 Kotlin RFCOMM server 桥接；Linux 用 BlueZ RFCOMM；Windows 用 WinRT RFCOMM client。隧道与三端音频逻辑不变。
- 纯蓝牙运行需要设备配对/授权（手机蓝牙目前需手动打开）；三端 RFCOMM 服务 UUID、配对与权限属于 Phase 2。

#### Phase 2：RFCOMM↔TCP 桥（桌面端，已编译/实测链路）

- Linux（BlueZ 原生 RFCOMM）：`siren rfcomm --listen --channel 16 --bridge-base 47980 --accept 1`
- Windows（Winsock `AF_BTH`/RFCOMM）：`siren rfcomm --connect 58:FB:84:3A:F1:EB --channel 16 --bridge 47980`
- 两端再用 `siren bt --bit 1 --port 48001 --link 127.0.0.1:47980 --source tone --target-ms 150 --opus-bitrate 64000` 接入隧道。
- 实测：L2CAP ping 0% 丢包；RFCOMM 通道 16 **未配对即可连接**；双向音频各 −15 dB（440/880 Hz Opus）。
- **链路基线**：绕过隧道用 Python/PowerShell 直传 512 KB，仅 **34–40 KB/s**，最大停顿 **5.1–5.9 s**；链路为 CENTRAL/AUTH/ENCRYPT、质量 255，PC 使用集成 Intel 蓝牙（MAC `E0:8F:4C:16:9F:58`，非 USB dongle）。
- 已排除：Nagle（两端 `TCP_NODELAY`）、Opus 码率（64/192 kbps 均复现）、SNIFF（`sudo hcitool lp <pc> RSWITCH` 后停顿不变）。剩余嫌疑：RFCOMM MTU/流控、Windows 无线电电源管理、2.4 GHz 共存/重传，或需改用 BLE L2CAP CoC。下一步先做手机↔Linux 链路（不同控制器），并补 Android 桥。
- **Android 桥（已实现）**：`MainActivity` 用隐藏的 `createInsecureRfcommSocket(16)`（失败回退 SPP UUID）连接 Linux；每个连接配一个本地 TCP listener（47980），与 Rust 通过 JNI 启动的 `Tunnel + MeshEngine`（tone、Opus 64k、150 ms）对接。启动方式：
  `adb shell am start -n com.sirenstudio.siren_ui/.MainActivity --es bt_connect <linux-mac> --ei bt_bridge 47980 --ei bt_mesh 48101 --ei bt_bit 2 --ef bt_freq 880 --ez bt_output true`
  - Manifest 增加 `BLUETOOTH_CONNECT/SCAN`（运行时授权），`bt-node` 每秒把流/主电平写入 logcat。
- **三端纯蓝牙链已验证**：Linux 作为 Hub（bit 1, 440 Hz）开两个 RFCOMM 监听（手机 ch16 → 47980，PC ch17 → 47990），`siren bt --link 47980 --link 47990` 转发；手机（bit 2, 880 Hz）与 PC（bit 3, 660 Hz）各看到 **2 路远端流（≈−15 dB）**，master ≈ −12 dB；Origin/TTL/去重正常。手机↔Linux 链路稳定；PC↔Linux 仍继承 5–6 s 停顿（PC 侧流会间歇 −120）。

### 缓冲目标下发修复（抖动）

中心的目标值此前只更新快照与**已存在**的流：如果用户在还没有对端时把“缓冲目标”调到 100 ms，之后加入的流仍使用启动时的 30 ms（UI 显示 100、流实际 30），Wi-Fi 抖动直接变成欠载。

- `dsp/mixer.rs` 现在记住最后一次 `SetTargetMs`（`target_override`），在处理 `Add` 时应用到新流；单测 `target_set_before_a_stream_joins_applies_to_it` 覆盖。
- 实测（手机 440 测试音 → PC，100 ms 目标）：PC 端手机流 **丢帧 0 / 欠载 0**；修复前同样链路 30 ms 下 12 分钟累计 10 次欠载。
- PC/手机界面上的目标滑杆现在对“之后加入的流”同样生效；UI 的 `SIREN_UI_MESH_TARGET_MS` 环境变量可指定桌面启动目标（自动化验证用）。

## 13. 构建与依赖

- `opus-head-sys` 从源码编译 libopus，需要 **CMake ≥ 3.16** 与 MSVC。
- VS 2022/2026 Build Tools 的 CMake 不在 PATH；`scripts/build.ps1` 通过 `vswhere` 定位并临时加入 PATH。
- 不要使用 `audiopus`（内置旧版 Opus 的 `cmake_minimum_required(<3.5)` 会被 CMake 4 拒绝）。
- 调试构建也保留优化（`[profile.dev] opt-level=1`，依赖 `opt-level=2`），避免 DSP 在 debug 下过慢。

### Flutter Windows UI 构建

```powershell
powershell -File scripts/build_ui.ps1
```

产物：`ui-flutter/build/windows/x64/runner/Release/siren_ui.exe`（`rust_lib_siren_ui.dll` 同目录）。

已知构建约束：

- **Cargokit 长路径**：MSBuild 在深层临时目录 try-compile 会报 FTK1011；`build_ui.ps1` 设置 `CARGOKIT_TEMP_DIR=C:/siren_build/cargokit` 规避，且 `ui-flutter/build` 是指向 `C:/siren_build/ui-flutter` 的 junction。换机器构建时需重建该 junction（`New-Item -ItemType Junction`）。
- **debug/profile 模式**：`flutter run` 会以 debug profile 重新编译 Rust，opus-head-sys 在该路径下可能失败；验证 UI 请用 release 构建。
- **按钮图标槽位 ≥20px**：本机 Flutter Windows 引擎在「按钮 icon 槽位重建为 <20px 的部件盒」时会原生崩溃（0xC0000005，与动画/渲染后端/FFI 无关）。`AppSpinner` 因此把盒子钳到 20×20，也不要在 `FilledButton.icon`/`IconButton` 的 icon 槽位放更小的部件。

运行期环境钩子（自动化验证用）：

| 变量 | 作用 |
| :-- | :-- |
| `SIREN_UI_AUTOSTART=1` | 启动 800ms 后自动启动中心 |
| `SIREN_UI_MODE=playback/record/silent/...` | 覆盖中心输出模式 |
| `SIREN_UI_ADVERTISE=0` | 关闭 mDNS 广播 |
| `SIREN_UI_DEBUG=1` | 把关键流程事件追加写 `ui_debug.log`（进程工作目录） |

日志：`siren_ui.log` 与 exe 同目录（env_logger + VEH 崩溃报告 + panic hook 都写这里）。

## 14. 测试

单元测试（19 个）：

```powershell
cargo test --workspace
```

覆盖：协议回环/异常包、格式换算、正弦频率、PI 快/慢时钟跟踪、启动预缓冲、缓冲回落、双路混音、软限幅、静音仍消费、Raw/Opus 回环与 PLC、WAV 分析、主机名清洗。

端到端：

```powershell
python scripts/verify_e2e.py
```

启动真实 center/send 进程，读取 `--stats-json` 与录音 WAV，断言：

- 指定频率存在且幅度 > 0.15；
- 非目标频率不存在；
- 活跃区间内 `underruns == 0`、`lost_frames == 0`；
- `master.peak_hold_db > −30`；
- 内录捕获 `dropped_frames == 0`、`read_errors == 0`；
- mDNS 能发现指定端口，`--to auto` 能连通并出声。

结果写入 `target/e2e/report.json`。无音频设备时加 `--skip-loopback`。

## 15. 已知限制

- 只有 Windows 采集；Linux 需要 PipeWire/PulseAudio 模块，Android 需要 `AudioPlaybackCapture` + 前台服务。
- 没有加密与鉴权；当前默认只建议在可信局域网使用。
- 没有按进程内录（Windows Process Loopback）、没有蓝牙 A2DP Sink。
- 混音是等增益线性叠加；每路增益/静音已在核心里，但 CLI 还没有控制入口（下一阶段随界面提供）。
- mDNS 的 `--to auto` 选择第一个中心；多中心场景应显式指定地址。
- 端到端延迟取决于系统调度与目标水位；极端卡顿会触发一次缓冲回落（约 5 ms 空隙）。

## 16. 调参速查

| 目标 | 调整 |
| :-- | :-- |
| 降低延迟 | `--target-ms 15`，Raw + 5 ms 帧，有线网络 |
| 提高 Wi-Fi 稳定性 | `--target-ms 50`，Opus |
| 降低带宽 | Opus；如需更低可改 `net/codec.rs` 的 `OPUS_BITRATE` |
| 更平滑的音高 | 增大 `FILL_SMOOTHING_S` 或降低 `max_correction` |
| 更快收敛 | 增大 `kp` / `ki`，但注意音高抖动 |
| 减少回落 | 增大 `--target-ms` 或放宽 `MAX_FILL_RATIO` |

### 组网（去中心化 Mesh，已实现）

`crates/audio-core/src/mesh.rs` 的 `MeshEngine` = `CenterEngine`（收流 + 混音 + 本地播放）+ 多目标发送线程 + mDNS 发现循环。每个节点同时收发，无需显式指定中心/发送端：

- 广播 `role=mesh` 服务，TXT 携带随机 `node_id`；发现循环以此过滤自身，并按节点 id 去重（多网卡设备会对每个接口各 resolve 一次，不去重会导致同一节点收到重复流并叠加混音）。
- 节点离开：连续两次扫描未出现即移除。
- `MeshConfig.static_peers` 可钉住端点（mDNS 不可用时兜底）；单测 `mesh_nodes_exchange_audio_without_manual_discovery` 覆盖收发链路。
- FFI：`siren_begin_start_mesh / poll_start_mesh / mesh_snapshot / mesh_set_gain / mesh_set_muted / mesh_set_target_ms / mesh_set_master_gain_db / mesh_stop / mesh_release`；UI 增益/静音按当前激活的混音引擎（mesh 优先）路由。
- Android：`MainActivity` 持有 `WifiManager.MulticastLock`（Manifest 需要 `CHANGE_WIFI_MULTICAST_STATE`，否则 SecurityException 崩溃）；Oboe 输出走 `ndk_context`，采集使用 MediaProjection + `AudioPlaybackCapture`（排除自身 UID，见防回环一节）或麦克风。
- UI 只有组网一个页面（桌面侧栏 / 移动端单页），不再有中心端与发送端的配置入口；独立的中心/发送引擎仅保留在 CLI（`siren center` / `siren send`）与测试中。端口可用 `SIREN_UI_MESH_PORT` 覆盖（默认 47900，用于同机多实例测试）；`SIREN_UI_AUTOSTART=1` 会在启动后自动加入组网，来源/输出可用 `SIREN_UI_MESH_SOURCE`（tone/loopback/microphone）与 `SIREN_UI_MESH_OUTPUT`（0=静默）覆盖。
