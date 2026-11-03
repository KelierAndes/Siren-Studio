# SirenStudio 修复排期与项目计划

制定日期：2026-11-03。原则：**用户先听到的优先，先测后改**——每个修复都必须能用 Phase 0 的基线表对照验证，没有基线不动手。

- 状态标记：⬜ 未开始 / 🟨 进行中 / ✅ 完成
- 测试环境：Linux 测试机 `kelierandes@192.168.2.196`（Debian 12 无头 NAS，PipeWire 无头，跑 `siren web`）；三端 = Windows PC + Android 手机（M2012K11G）+ NAS。
- 构建注意：PC 端构建先停 app（见 `scripts/build*.ps1`）；APK 的 Rust 失败会静默沿用旧 `.so`。

## Phase 0 · 测量基座（3–4 天）✅ 2026-11-04 完成

目标：任何后续修复都有可对照的数字。验收 = 一条命令输出 phone↔PC / leaf↔hub 两组基线表（丢包、P99 延迟、切换次数、欠载、修剪、咔哒）。

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| P0-1 | 内存 Link 模拟器驱动 HybridNode 单测：可编程延迟/抖动/丢包/断链（F19），CI 可跑 | `net/link.rs`（延迟/抖动/独立丢包/Gilbert 突发/带宽帽/硬断链，种子化 RNG）+ `tests/link_sim.rs`（7 场景）+ `tests/bt_overlay.rs`（SimBt 环回 TCP 模拟 RFCOMM，静态对端 `?bt=&id=` 种 topology）| `cargo test` 全绿（51 库 + 7 链路 + 2 BT）；含选举、拨号、双栈去重、Wi-Fi 断开走 N−1 混音的完整验证 | ✅ 2026-11-04 |
| P0-2 | Linux 侧 `tc netem` 脚本：双向、每种故障 ≥10 次（F9） | `scripts/linux/netem.sh`（ifb 双向整形，已实测）+ `scripts/verify_scenarios.py`（SSH 驱动 PC↔NAS，自动保护/恢复 NAS 常驻 daemon，CSV+markdown 汇总） | ✅ 7 场景 ×10 次 ×8s 矩阵在 192.168.2.196 跑完（~25 min），结果归档 `docs/baselines/2026-11-04/` | ✅ 2026-11-04 |
| P0-3 | 每路端到端延迟（500 ms 心跳 + RTT）与"咔哒"计数器（样本间阶跃检测），进快照（F10） | `net/udp.rs`：HB01 心跳（`UdpSender` 闭环请求/回显 + `PathHealth` P50/P99）、`Peer::count_clicks` 阶跃检测（阈值 0.5）；快照 `peers[].rtt_ms/loss_pct`、`streams[].clicks`；WebUI 设备卡加"咔哒"、组网卡加"延迟"；真实链路实测 clean RTT P99 5–15 ms、loss2 心跳丢包 3.57% 与音频一致 | ✅ 2026-11-04 |
| P0-4 | 基线输出命令 | `scripts/baseline.py` 一条命令跑矩阵并归档 `docs/baselines/<date>/`（README 含二进制/git/校准备注） | 修复前基线已存档 `docs/baselines/2026-11-04/`（旧二进制）；新二进制含 RTT/咔哒列。**遗留**：phone↔PC 一组待 Android 可无头驱动后补测 | ✅ 2026-11-04（phone 组遗留）|

## Phase 1 · P0 听感（1.5–2 周）⬜

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| F1 | 先抓包确认分片与丢包相关（NAS/PC tcpdump）；再在 **s16 载荷（新 codec id）** 与 **2.5 ms 帧** 之间二选一（用 Phase 0 基线比较） | `protocol.rs` 加 codec id；`net/codec.rs` 加 RawS16 编解码；`net/udp.rs` 帧/打包 | 弱链路丢包 <2%，ABX 无音质台阶 | ⬜ |
| F2 | 启动/欠载/Flush/切载波各加 2–5 ms 斜坡；切换做短交叉淡化 | `dsp/resampler.rs`、`net/udp.rs`（Flush 路径）、载波仲裁处 | 事件次数不变，波形无阶跃、听不到咔哒（用 P0-3 咔哒计数对照） | ⬜ |
| F4 | 软膝 + 前瞻限幅替代裸 tanh | `dsp/mixer.rs` | −6 dBFS 与多路叠加无可听失真，峰值不削 | ⬜ |
| F3 | 补"混音 vs 直连不重复"单测；若 Phase 0 观察到重复，再按缺失集收敛 | `mesh.rs` + link_sim 单测 | 单测复现当前行为；如观察到重复，hub 只混叶子缺失集 | ⬜ |

## Phase 2 · 叶子稳定（1 周）⬜

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| MIX1 (#64) | 修剪↔欠载自激：F8 用"冻结 + 渐隐"替代直接销毁流 | `dsp/resampler.rs`、`net/udp.rs` 超时路径 | NAS 现场（手机双流 ~12% 丢包）修剪/欠载从每秒 ~4 次降到趋近 0 | ⬜ |
| F13 (#68) | 网络变化时重建发送 socket（或按目的地选源） | `net/udp.rs` 发送路径 | Wi-Fi 关→开后音频回 Wi-Fi（现有已知故障验证用例） | ⬜ |
| F7 | 同 uid 不同地址检测 → 重生成 uid + 告警日志 | `mesh.rs` 发现路径 | 克隆镜像场景单测通过 | ⬜ |
| LATE_DUPLICATE_LIMIT | 从 21 分钟收紧到 ~5 s（seq 按包递增，5 ms 帧 → 1000 包 = `-1_000`，现为 `udp.rs:827` 的 `-250_000`）；补"发送端重启"单测 | `net/udp.rs` | 单测：发送端重启后旧副本不再被接受为迟到包 | ⬜ |
| 幽灵流 | 把载体门移到建流之前 | `net/udp.rs` / `mesh.rs` | 无帧载体不再建出不可播流 | ⬜ |

## Phase 3 · 安全（1–2 周，需一次设计评审）⬜

- F6：组密码 → 帧 AEAD（XChaCha20-Poly1305）或独立握手；选举签名；WebUI 默认绑 `127.0.0.1`。
- 落点：`protocol.rs`（帧封装）、`mesh.rs`（选举）、`siren-cli/src/webui.rs`。
- 验收：未持密码的节点无法注入/接管；"无密码模式降级 + 告警"的兼容策略写进 DEVELOPMENT.md。

## Phase 4 · 规模化与平台（按需）⬜

F5 按链路质量选码率 + in-band FEC；F15 gossip 替代单 mDNS；F16 TXT 加 `proto`；F20 JSON schema 校验；F14 写明规模上限；F12 Android 改 UUID 版 socket + `mediaPlayback` 前台服务。

## 已知风险与运维坑（来自三设备实测）

- `state 5` 半开 ACL：长时间反复重连后所有拨号被挡，需 `sudo hciconfig hci0 reset`；长跑前先复位蓝牙栈。
- NAS 上 `pkill -f` 会自伤，用 `ps -C siren -o pid=,args=` + 精确 PID。
- MIUI 卸载后 `adb install` 需机主在开发者选项重新允许 USB 安装。
- 手机 raw 双流上行 ~12% 丢包 + 修剪/欠载每秒 ~4 次是 MIX1 的活体复现场景（Phase 2 的对照现场）。
- PC↔NAS 的 RFCOMM 链路至今未建立成功——P0-1 内存 BT 传输不能拿它当参照，按手机↔NAS 的实测行为建模。

## 变更记录

- 2026-11-03：按"先测后改"排期建档；Phase 0 启动。
- 2026-11-04：Phase 0 完成。基线要点（真实 Wi-Fi，PC↔NAS）：clean 0% 丢包；loss2 实测 ~4%（netem 2% 配置 ×2 待校准）；**delay200/jitter50 的高"丢包"（43%/67%）主要来自 netem 抖动重排 + 接收端无重排容忍窗口**（jitter50 欠载 1278/重置 407——这是 F2/F5 的直接靶子）；bw1800 欠载 3816。NAS 已部署新二进制（心跳/咔哒生效）。
