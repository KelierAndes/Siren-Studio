# SirenStudio

把局域网内多台设备（Windows / Android / Linux）的声音实时互连：每台设备既是发送端也是混音端，自动发现、互收互发，无需指定中心。

每台设备运行同一个 Rust 核心：抓取系统声音（Windows WASAPI 进程回环 / Android AudioPlaybackCapture / Linux PulseAudio monitor）、麦克风或测试音，经 UDP 发给所有对端；收到的每一路都做独立抖动缓冲与时钟漂移校正，线性混音、软限幅后播放。Windows 与 Android 提供 Flutter 界面，Linux 提供无头守护进程 + 浏览器 WebUI。

- 48 kHz / 立体声 / f32 统一传输格式
- 低延迟 UDP 自研帧协议，支持 **Raw PCM**（零编码延迟）与 **Opus**
- 每路独立环形缓冲 + PI 自适应重采样，长时间运行不漂移、不累积延迟；缓冲目标对已加入与后续加入的流都生效
- mDNS 零配置发现；Wi-Fi 主载波 + 蓝牙自动兜底，无需手动选接入方式
- 实时电平、缓冲水位、丢包/欠载统计，可写 JSONL
- 当前离线分析：频谱、RMS、静音块

> 当前版本已完成 PC ↔ 手机 ↔ Linux 三端组网的真机验证：Windows/Android Flutter 界面、Linux 无头 WebUI、三平台系统内录/麦克风采集与本地播放。

## 架构

```
      ┌──────────────┐   48 kHz 立体声 / 5 ms 一帧        ┌──────────────┐
      │  Windows PC  │ ◀────── Wi-Fi UDP（主载波）───────▶ │   Android    │
      │  桌面界面     │       蓝牙 RFCOMM（自动兜底）        │   手机界面    │
      └───────┬──────┘                                     └───────┬──────┘
              │                                                      │
              └────────────────▶ ┌──────────────┐ ◀─────────────────┘
                                 │  Linux 无头   │
                                 │  浏览器 WebUI │
                                 └──────────────┘

每个节点：抓系统声音 → 发给所有对端 → 每路独立抖动缓冲 + 漂移校正 → 混音 → 本地播放
```

- 每路缓冲目标默认 **100 ms**（桌面/手机界面；CLI 默认 30 ms），可随时调整，对已加入与后续加入的流都生效。
- 混音不做“平均”，直接浮点相加；最后 `tanh` 软限幅防削波。
- 组网是去中心化的：没有固定的中心设备，hub 只决定蓝牙转发角色（按节点编号选出）。
- `siren center` / `siren send` 是单中心/单发送的 CLI 子命令，保留给脚本与端到端测试。

## 环境要求

- Windows 10 / 11（WASAPI；建议 Windows 10 build 20348+，以启用进程回环排除自身）
- Debian 12 / 主流桌面发行版（PulseAudio ≥ 15 或 PipeWire 的 pulse 兼容层）
- Android 8.0+（系统内录需 Android 10+ 与录屏授权）
- Rust 1.85+
- Visual Studio Build Tools（“使用 C++ 的桌面开发”，自带 MSVC 与 CMake）
- Python 3（只用于端到端验证脚本）

Opus 由 [`opus-head-sys`](https://crates.io/crates/opus-head-sys) 从源码编译，无需单独安装 libopus。

## 构建

```powershell
powershell -ExecutionPolicy Bypass -File scripts/build.ps1
```

脚本会自动定位 Visual Studio Build Tools 里的 `cmake`，然后执行 `cargo build --release` 与单元测试。产物：

```
target\release\siren.exe
```

如果只想构建不跑测试：`powershell -ExecutionPolicy Bypass -File scripts/build.ps1 -SkipTests`。

## 快速开始

### 1. 找输出设备

```powershell
siren devices
```

带 `*` 的是系统默认输出设备。中心默认使用它，也可以用 `--device` 指定名称片段。

### 2. 启动中心

```powershell
# 播放到默认声卡
siren center

# 录成 WAV（此模式不播放）
siren center --record mix.wav

# 指定输出设备、缓冲 50 ms、运行 60 秒
siren center --device UGREEN --target-ms 50 --duration 60
```

启动后会广播 mDNS，并每秒打印各路电平与缓冲状态。

### 3. 从本机或另一台设备发送

在本机测试：

```powershell
siren send --to 127.0.0.1:47900 --source loopback
```

在另一台设备上（同一局域网）：

```powershell
siren send --to 192.168.1.20:47900 --source loopback
```

或者不用记 IP：

```powershell
siren send --to auto --source loopback
```

`--source` 可选：

| 值 | 含义 |
| :-- | :-- |
| `loopback` | 系统正在播放的全部声音 |
| `mic` | 默认麦克风 |
| `tone` | 内置正弦测试音（配合 `--freq`） |

### 4. 测试音与发现

```powershell
siren send --to 127.0.0.1:47900 --source tone --freq 440 --duration 5
siren discover --seconds 4
siren playtone --freq 1000 --duration 3     # 只在本机播放，用于测试内录
```

### 5. 分析录音

```powershell
siren analyze mix.wav --freq 440 --freq 660 --skip-start 2 --skip-end 2
```

输出 RMS、峰值、直流偏移、静音块数量，以及指定频率的幅度/是否存在。加 `--json` 输出机器可读结果。

## Linux（无头守护 + WebUI）

Debian 12 实测构建：

```bash
sudo apt install build-essential pkg-config cmake libpulse-dev libasound2-dev \
                 pulseaudio pulseaudio-utils
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
cargo build --release -p siren-cli
sudo install -m 755 target/release/siren /usr/local/bin/siren
```

有桌面会话（PulseAudio / PipeWire）时直接启动 WebUI：

```bash
siren web --listen 0.0.0.0:8080 --source loopback --name linux-nas
# 浏览器打开 http://<设备IP>:8080
```

PipeWire 桌面可启用**结构隔离**（本机播放不进入采集，零 DSP）：

```bash
siren web --listen 0.0.0.0:8080 --source loopback \
          --isolate pipewire --output-sink <硬件播放 sink>
```

隔离模式会创建 `siren_capture` 虚拟 sink 并设为默认（其他应用的声音进入它），把它的 monitor 回环到指定播放 sink，同时让本机 mesh 播放直接走该播放 sink；采集只包含其他应用。需要 PipeWire 的 pulse 层与 `pactl`，不可用时自动回退到 monitor + DSP。该模式会临时改写默认设备，且同一用户只允许一个实例。

页面里加入组网后自动发现其他节点；可切换 测试音 / 系统内录 / 麦克风、本地播放、缓冲目标、主增益与每路增益/静音，并实时查看节点、流电平、上行速率与**发送电平**。`--autostart` 可在启动时直接加入。

无桌面会话的测试机可用 `scripts/linux/run-headless.sh`：它会启动 per-user PulseAudio、创建 `siren_test` 空设备，再打开 WebUI。开机常驻使用 `scripts/linux/siren-web.service`（按注释修改 `User` 与 `XDG_RUNTIME_DIR`）。

纯命令行守护（无页面）：`siren mesh --source loopback --name linux-nas`。

默认（monitor 路径）下，本机播放的回声由 DSP 在发送前消除：播放对端音乐时发送电平约 **−50 ～ −70 dBFS**，第三方应用的音频正常穿过。使用 `--isolate pipewire` 时采集与播放彻底分离：播放对端音乐时采集为 **−180 dBFS**（数字静音），CPU 从约 30% 降到约 6%。

## Wi-Fi + 蓝牙双栈（默认自动）

三端默认**同时**使用两条通路，无需选择接入方式：

- Wi-Fi（mDNS 发现 + UDP）始终在线；
- 蓝牙（RFCOMM，固定通道 16）作为并行热备：每台设备把本机蓝牙地址与编号通过 mDNS 广播出去，**编号最小且带蓝牙地址的节点自动成为中心并监听**，其余节点自动拨号；蓝牙不可用的设备（如本机地址被系统遮蔽的手机）只作为叶子参与；
- 每一帧音频**只编码一次**，同时写进两条通路；接收端按 `(来源, 流, 序号)` 去重，哪条先到用哪条。蓝牙写队列与套接字缓冲都有界，迟到副本直接丢弃，不污染抖动缓冲；
- **断网自动接管**：屏蔽 Wi-Fi UDP 后音频在数百毫秒内切到蓝牙（实测 path 由 `wifi` 变 `bt`，声音继续），恢复后自动切回；角色变更（中心离网等）只重建蓝牙链路，Wi-Fi 音频全程不中断。
- **蓝牙扫描发现**（mDNS 备选）：完全没有 Wi-Fi 时，节点会记住上次的中心；首次连接则通过经典蓝牙扫描发现可发现的中心（桌面节点在当中心时会保持可被发现），逐个探测并确认后才入网——实测手机无 Wi-Fi、无历史记录也能靠扫描加入并双向传音。
- **纯蓝牙**：桌面节点记住上次角色（中心继续监听、叶子主动拨号），叶子不再充当中继；单条蓝牙链路上的流自动使用 150 ms 抖动缓冲。两节点纯蓝牙实测 0 丢帧、0 欠载；多节点时所有链路共享同一射频，建议让射频最好的桌面当中心，并尽量改善弱链路（缩短距离 / USB 蓝牙适配器）。

蓝牙 RFCOMM 模块内置于引擎（Linux 原生 `AF_BLUETOOTH`、Windows Winsock、Android 经 Kotlin 桥），不再需要外部桥进程。状态在各端 UI 的「蓝牙通路」中展示（角色 / 状态 / 链路数）。窄带宽蓝牙链路建议在页面选择 Opus 编码以提升蓝牙单独承载时的完整度。

## 纯蓝牙多跳（实验）

无 Wi-Fi 环境可单独使用蓝牙链路（RFCOMM/L2CAP 字节隧道承载 Opus/UDP 音频，TTL + 64 位 Origin 掩码 + 去重，支持链式多跳）：

```bash
# 中继（Hub）：等待两个蓝牙桥接 TCP 链路
siren bt --name Hub --bit 0 --port 48002 --listen-link 48201 --listen-link 48202          --source tone --freq 550 --no-play

# 叶子：各自连接一个桥接端口
siren bt --name A --bit 1 --port 48001 --link 127.0.0.1:48201 --source tone --freq 440 --no-play
siren bt --name C --bit 2 --port 48003 --link 127.0.0.1:48202 --source tone --freq 880 --no-play
```

`--bit` 必须全网唯一（0–63）。当前为逐包转发（带宽随源数增长）。桌面端蓝牙桥：

```bash
# Linux（BlueZ）监听 RFCOMM 通道
siren rfcomm --listen --channel 16 --bridge-base 47980 --accept 1
# Windows（Winsock）连接 Linux
siren rfcomm --connect <linux-mac> --channel 16 --bridge 47980
```

桥接后各自用 `siren bt ... --link 127.0.0.1:47980 --target-ms 150 --opus-bitrate 64000` 接入。

三端纯蓝牙链已验证（Linux Hub 两路 RFCOMM，手机/PC 各收到两路远端流）；PC 侧链路仍有 5–6 s 周期性停顿（射频路径/距离问题）。日常使用建议直接走上面的**双栈自动**模式。

## 常用参数

### `siren center`

| 参数 | 默认值 | 说明 |
| :-- | :-- | :-- |
| `--bind` / `--port` | `0.0.0.0` / `47900` | UDP 监听地址 |
| `--target-ms` | `30` | 每路抖动缓冲目标（5–500 ms） |
| `--record <file>` | — | 录 32-bit float WAV；此模式不播放 |
| `--device <片段>` | 系统默认 | 播放设备名称子串 |
| `--no-play` | — | 不开声卡，仅混音/统计（测试用） |
| `--duration <秒>` | 不限 | 自动退出 |
| `--name <名称>` | 计算机名 | mDNS 显示名 |
| `--no-discover` | — | 不广播 mDNS |
| `--stats-json <file>` | — | 每秒一行 JSON + 末尾 summary |
| `--stats-interval <秒>` | `1` | 统计间隔 |

### `siren send`

| 参数 | 默认值 | 说明 |
| :-- | :-- | :-- |
| `--to <ip:port\|auto>` | `auto` | 中心地址 |
| `--source <loopback\|mic\|tone>` | `tone` | 音频源 |
| `--codec <raw\|opus>` | `raw` | 传输编码 |
| `--frame-ms <毫秒>` | `5` | 网络帧长；Opus 支持 5/10/20/40/60 |
| `--freq <Hz>` | `440` | `tone` 源频率 |
| `--amplitude <0..1>` | `0.25` | `tone` 源幅度 |
| `--duration <秒>` | 不限 | 自动停止 |
| `--json` | — | 结束时只向 stdout 输出一段 JSON 摘要 |
| `--summary-json <file>` | — | 同时把摘要写入文件 |

## Raw 还是 Opus？

| | Raw f32 | Opus |
| :-- | :-- | :-- |
| 单路带宽（5 ms 帧） | ≈ 3.1 Mbps | ≈ 0.23 Mbps（192 kbps 目标） |
| 编码延迟 | 0 | 5 ms 帧 + 少量编码延迟 |
| 丢包处理 | 补零 | PLC（丢包隐藏） |
| 适用 | 有线千兆 / 本机验证 | Wi-Fi、跨设备、带宽敏感 |

局域网优先 `raw`；Wi-Fi 抖动大或设备多时用 `opus`。

## 延迟与稳定性

默认端到端延迟大致为：

```
5 ms 帧长 + 30 ms 抖动缓冲 + 网络 RTT/2 + 声卡一个周期（约 10 ms）
≈ 45–60 ms
```

- `--target-ms 15` 可以更低，但对 Wi-Fi 与系统调度更敏感；`50` 更稳。
- 每路漂移校正上限 ±0.2%（约 3.4 音分），正常听感不可察；长时间播放不会累积延迟。
- 若系统卡顿导致缓冲超过“2×目标 + 10 ms”，会丢弃最旧的一小段把延迟拉回目标，统计里记为 `reset`。频繁出现时请增大 `--target-ms`。

## 防火墙与网络

- 中心需要放行 **UDP 47900**（或你 `--port` 指定的端口）入站。
- mDNS 发现使用 **UDP 5353** 组播，需要与发送端在同一子网。
- 跨设备连不上时，先用 `--to ip:port` 排除发现问题，再检查 Windows Defender 防火墙。
- Windows 首次监听非回环地址时可能弹防火墙授权框；没有弹窗时请手动添加 `siren.exe` 的入站 UDP 规则。

### 无线部署要求

多设备同播对无线的要求比普通应用苛刻：每 5 ms 就有一帧音频要到达，任何一次几百毫秒的停顿都会被听成卡顿。

- **用 5 GHz 或有线**。2.4 GHz 与蓝牙、微波炉、邻居网络共用，重传率一高就会出现"偶发掉一拍"。三台以上设备时，建议至少让播放端（中心）走有线。
- **关闭 AP 隔离 / 客户端隔离**。开启后同一无网内的设备互相收不到单播与组播，表现为"能 ping 通 IP 却听不到声音""设备列表里看不到对方"。
- **保留组播/IGMP Snooping 可用**。节点之间靠 mDNS（UDP 5353 组播）互相发现；若交换机丢弃组播，设备仍能通过已记住的地址继续传输，但新加入的节点可能不被发现。
- **给音频端口打 QoS 标记**（可选，进阶）。若路由器支持 WMM/DSCP，把 UDP 47900 标记为语音/视频优先级（DSCP 46/34），可显著降低同网段下载、看视频带来的抖动。
- **手机请保持屏幕常亮或在前台运行**，并在系统设置里允许该应用后台运行；省电模式会让 Wi-Fi 网卡进入休眠，造成周期性欠载。

## 故障排查

| 现象 | 处理 |
| :-- | :-- |
| `no default output device` | `siren devices` 查看设备；用 `--device` 指定 |
| 中心没有声音 | 默认是播放模式；`--record` 只录音不播放 |
| 内录失败，提示格式转换被拒绝 | 把 Windows 输出设备采样率设为 48 kHz |
| `discover` 找不到中心 | 检查防火墙/同子网；直接用 `--to ip:port` |
| 某一路没有声音但统计里有 stream | 检查该发送端的音源、静音、系统音量 |
| 偶发“咔哒” | 系统长时间卡顿触发缓冲回落；增大 `--target-ms` |
| `--to auto` 找到的不是想要的中心 | 局域网有多个中心时请显式写 IP |

## 自动验证

```powershell
python scripts/verify_e2e.py
```

会依次启动真实进程，验证四件事，并把结果写到 `target/e2e/report.json`：

1. **raw 双路混音**：440 Hz + 660 Hz，无丢包/欠载，频谱双峰；
2. **Opus 双路混音**：1000 Hz + 1500 Hz，验证 PLC 路径与低带宽；
3. **WASAPI 系统内录**：本机播放 1 kHz，环回捕获后经网络混音录音；
4. **mDNS + auto**：发现中心、自动连接并混音 880 Hz。

本次实测结果：

| 场景 | 结果 |
| :-- | :-- |
| raw 双路 | RMS −12.4 dBFS，440/660 Hz 分别 −12.7/−12.7 dB，静音块 0，丢包 0 |
| Opus 双路 | RMS −12.4 dBFS，1000/1500 Hz 均 −12.4 dB，每路 226 kbps，丢包 0 |
| 系统内录 | 1000 Hz（检测 1001.5 Hz）−14.4 dB，捕获 336 000 帧，0 丢弃，0 读错误 |
| mDNS + auto | 发现 `192.168.2.109:48010`，自动连接后 880 Hz（检测 881.75 Hz）−12.3 dB |
| 播放稳定性 | 30 ms 目标水位，15 秒连续播放 0 欠载 |

无音频设备的环境可跳过内录：`python scripts/verify_e2e.py --skip-loopback`。

## 当前边界与路线图

已实现：

- Windows 系统内录 / 麦克风采集、UDP 收发、Raw/Opus、抖动缓冲、漂移重采样、线性混音、软限幅、播放、WAV 录音、mDNS、统计与离线分析。
- 核心已提供 `EngineCommand` / `EngineEvent` 数据模型与无锁控制句柄（增益、静音、目标缓冲），供界面层直接绑定。
- 去中心化组网 UI（Windows 桌面 + Android，Flutter + 手写 C ABI FFI 接入同一 Rust 核心）：每台设备加入组网后自动互相发现、互收互发，无需配置中心端/发送端；实时电平与缓冲/漂移/丢包统计、每路增益/静音、主增益与缓冲目标调节、本地播放开关、运行日志。采集来源：测试音、系统内录（Windows 进程回环 / Android AudioPlaybackCapture）、麦克风。构建见 `scripts/build_ui.ps1`。
- Linux 无头守护进程 + Axum WebUI（`siren web`，内嵌单页无构建步骤）；PulseAudio/PipeWire monitor 系统内录与麦克风，DSP 消除自身播放回声；`siren mesh` 纯 CLI 守护。
- 三平台采集层防回环：Windows 进程回环排除自身；Android 应用级 `allowAudioPlaybackCapture="false"`（部分定制 ROM 的 `excludeUid` 不生效）+ `excludeUid` 第二层；Linux 数字回环 DSP（快速重锁 + 发送电平监控），另提供可选的 PipeWire 节点隔离（`--isolate pipewire`，零 DSP、零残留）。

尚未实现：

- 数据包加密/认证、蓝牙 A2DP Sink。
- Linux 桌面端图形界面（当前为 WebUI）。

技术细节、协议格式、DSP 参数与构建说明见 [DEVELOPMENT.md](DEVELOPMENT.md)。
