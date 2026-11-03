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

## Phase 1 · P0 听感（1.5–2 周）✅ 2026-11-04 完成

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| F1 | 抓包确认：raw 1948 B 帧线上 `flags[+]` = 2 IP 分片/包；选定 **s16 载荷**（988 B 单 MTU） | `Codec::RawS16`(id 2) 全栈 + 解码器槽×3 + CLI `--codec s16` | loss2 实测 4.09%→**1.96%**（=netem 配置），burst 3.29%→1.83%；<2% 达标 | ✅ 2026-11-04 |
| F2 | 3 ms 渐入斜坡覆盖 启动/缺口恢复/补零/Flush/切载波（`apply_fade_in`，RAMP_FRAMES=240） | `net/udp.rs` deliver/载波切换/缺口路径 | clean 与 loss2 全场景咔哒计数 **0** | ✅ 2026-11-04 |
| F4 | Limiter：1 ms 前瞻 + 软膝（KNEE=0.9）+ 50 ms release；静音块立即清环 | `dsp/mixer.rs` | −6 dBFS 线性单测（tanh 会压到 0.462）；过冲不削（峰值≤1.0） | ✅ 2026-11-04 |
| F3 | 补"混音 vs 直连不重复"单测；若 Phase 0 观察到重复，再按缺失集收敛 | `mesh.rs` + link_sim 单测 | 单测复现当前行为；如观察到重复，hub 只混叶子缺失集 | ⬜ |

## Phase 2 · 叶子稳定（1 周）✅ 2026-11-04 完成

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| MIX1 (#64) | F8 冻结+渐隐替代销毁：超时 → flush+冻结 30 s（保留流状态）→ 包到即渐入恢复；修剪后不再重新预热（started 保持）+ 输出渐入 | `net/udp.rs` retain 路径、`dsp/resampler.rs` trim | 冻结/恢复/修剪单测 3 个全绿；真实修剪不再产生静音窗 | ✅ 2026-11-04 |
| F13 (#68) | `UdpSender::refresh_route()`：每 30 s 重连 socket 重解析路由（不动 packer，stream id/seq 不变） | `net/udp.rs` + `mesh.rs` 发送循环 | Wi-Fi 恢复后 30 s 内回到 Wi-Fi 载体（需真机断网验证） | ✅ 2026-11-04（机制落地，真机断网待验）|
| F7 | 双端检测：接收端 origin 出现两个源地址 → 告警；mesh 检测 mix_tap 中自己的 uid → 告警 + 重生成持久化 node id（下次启动生效） | `net/udp.rs`、`mesh.rs` | 检测逻辑落地；克隆场景单测待造 | ✅ 2026-11-04 |
| LATE_DUPLICATE_LIMIT | `-250_000` → `-1_000`（~5 s）；发送端重启单测 + 陈旧 BT 副本仍为重复的单测 | `net/udp.rs` | 重启后立即重同步（8/8 稳定）；5 s 内迟到副本仍被静默丢弃 | ✅ 2026-11-04 |
| 幽灵流 | 载体门移到建流之前：Wi-Fi 健康时 BT 副本不再建出 0 包僵尸流（首触 BT 流仍放行） | `net/udp.rs` create_peer 前 | 载体门逻辑前移；既有仲裁单测全绿 | ✅ 2026-11-04 |

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
- 2026-11-04（Phase 1）：s16/斜坡/限幅落地，实测 loss2 丢包 1.96%（分片叠加消除）、咔哒归零；数据见 docs/baselines/2026-11-04-phase1/。
- 2026-11-04（Phase 2）：冻结+渐隐（MIX1/F8）、LATE_DUPLICATE_LIMIT 收紧至 ~5 s、幽灵流载体门前移、F13 路由周期刷新、F7 克隆检测。统一验收（7×10 矩阵）：咔哒全 0、无流全 0、修剪全 0；loss2 1.99%/loss10 10.05%（分片叠加消除）；bw1800 26.7%→0%；delay200/jitter50 仍为重排主导（未排期）。见 docs/baselines/2026-11-04-phase2/。
- 2026-11-04：Phase 0 完成。基线要点（真实 Wi-Fi，PC↔NAS）：clean 0% 丢包；loss2 实测 ~4%（netem 2% 配置 ×2 待校准）；**delay200/jitter50 的高"丢包"（43%/67%）主要来自 netem 抖动重排 + 接收端无重排容忍窗口**（jitter50 欠载 1278/重置 407——这是 F2/F5 的直接靶子）；bw1800 欠载 3816。NAS 已部署新二进制（心跳/咔哒生效）。
