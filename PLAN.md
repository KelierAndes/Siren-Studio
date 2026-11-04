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
| F3 | bt_overlay 断言：Wi-Fi 恢复后混音必须让位直连（delta≤10 包/2 s）；既有掩码仲裁即缺失集收敛 | `tests/bt_overlay.rs` | 断言通过（Phase 1 提交内含）；N−1 混音流退役豁免补丁见 Phase 2.5 E4 | ✅ 2026-11-04 |

## Phase 2 · 叶子稳定（1 周）✅ 2026-11-04 完成

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| MIX1 (#64) | F8 冻结+渐隐替代销毁：超时 → flush+冻结 30 s（保留流状态）→ 包到即渐入恢复；修剪后不再重新预热（started 保持）+ 输出渐入 | `net/udp.rs` retain 路径、`dsp/resampler.rs` trim | 冻结/恢复/修剪单测 3 个全绿；真实修剪不再产生静音窗 | ✅ 2026-11-04 |
| F13 (#68) | `UdpSender::refresh_route()`：每 30 s 重连 socket 重解析路由（不动 packer，stream id/seq 不变） | `net/udp.rs` + `mesh.rs` 发送循环 | Wi-Fi 恢复后 30 s 内回到 Wi-Fi 载体（需真机断网验证） | ✅ 2026-11-04（机制落地，真机断网待验）|
| F7 | 双端检测：接收端 origin 出现两个源地址 → 告警；mesh 检测 mix_tap 中自己的 uid → 告警 + 重生成持久化 node id（下次启动生效） | `net/udp.rs`、`mesh.rs` | 检测逻辑落地；克隆场景单测待造 | ✅ 2026-11-04 |
| LATE_DUPLICATE_LIMIT | `-250_000` → `-1_000`（~5 s）；发送端重启单测 + 陈旧 BT 副本仍为重复的单测 | `net/udp.rs` | 重启后立即重同步（8/8 稳定）；5 s 内迟到副本仍被静默丢弃 | ✅ 2026-11-04 |
| 幽灵流 | 载体门移到建流之前：Wi-Fi 健康时 BT 副本不再建出 0 包僵尸流（首触 BT 流仍放行） | `net/udp.rs` create_peer 前 | 载体门逻辑前移；既有仲裁单测全绿 | ✅ 2026-11-04 |

## Phase 2.5 · 现场暴露问题 🟨 E0-E4/E7 落地 + 真机验证；E1/E5 真机证据收集；E5 三组对照与 E8 补测待约

来源：`out/2026-11-04_1h/`（T1/T3/T4/T6 六阶段，1 小时）与结论 `FINDINGS.md`、复核 `VERIFICATION.md`。顺序即优先级：先让数据可信（E0），再修用户能直接听出来的（E1/E2），再定位（E5）。

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| E0 | 采集基座可信化：阶段启动前检查 47900 占用并清理残留 `siren.exe`；`pc.log/pc.stats` 按 run 命名；NAS 开 NTP；包内默认带 `master.csv`/`streams.csv` + 手机 I 级运行行 | `scripts/collect_1h.py`、打包脚本 | 清理/端口探活/按 run 命名/NTP 均落地（`clear_stale_senders`、`pc_<stamp>.*`、`set-ntp`）；"连跑两轮"验收待下次采集执行 | 🟨 代码落地 |
| E1 | 修剪改限速削（A3/A4，承接 MIX1）：单次 ≤60 ms、间隔 ≥200 ms，逐步回落到 target+10 ms；等突发结束再削；push 侧加硬上限，禁止 ring 一次吃进 1 s | `dsp/resampler.rs`、`net/udp.rs` push 路径 | 限速削全部落地（≤60 ms/次、≥200 ms 冷却、突发缓削、4× 天花板紧急直落）；push 侧以 ring 容量(1 s) + `overflow_packets` 计数为界，未另加软上限；T1 型真机 12 min 复测待做 | ✅ 2026-11-04（真机复测待）|
| E2 | 全零不上行（A5）：采集连续 ≥200 ms 全零则不发音频帧（保心跳），快照 `sender.silent_ms`；WebUI 状态区标"源静音" | `mesh.rs` 发送循环、`metrics.rs`、WebUI | 抑制逻辑 + 字段 + UI 徽标落地；真机起播段复测待做 | ✅ 2026-11-04（真机复测待）|
| E3 | 拨号指数退避（A2）：1/2/4/8/16/30 s 封顶，连续 5 次失败后降为 5 min 探测 | `hybrid.rs` 离线拨号 | 真机 12 min（BT 阻断）拨号 **0 次**（≤10 达标）；注意：手机侧半开 RFCOMM 的缓冲写（tx 7.8 KB/s）骗过双向看门狗，链路未被判死故未触发重拨——看门狗 tx 判据需改进（新发现，待排期） | ✅ 2026-11-05 真机 |
| E4 | 端点回打与发现失效（A6/A7）：mix_tap 记收包源地址回填对端 IP；发现 TTL 60→30 s；发送失败 3 次剔除端点触发重发现；换 stream id 后旧流 3 s 内退役（N−1 混音流豁免） | `net/udp.rs`、`mesh.rs` | 全部落地；真机阶段启动复测待做 | ✅ 2026-11-04（真机复测待）|
| E5 | 定位 A1：三组 12 min 对照（只关蓝牙 / 只关 Wi-Fi 省电 / 都关），手机 stats 增加 `tx_eagain`、roam/rssi、wakeup 计数，定时抓 `dumpsys wifi` | `scripts/collect_1h.py`、Android stats | 产出"EAGAIN + 1.1 s 洞是否随某项消失"判定表；三项均无效则转固件/AP，不再改软件 | ⬜ |
| E6 | 弱网自动降级（A10/T6_N4）| — | **s16 默认已化解主场景**：1.58 Mbps < `rate 2mbit`，T6_N4 型塌水位不再发生；<1.6 Mbps 的 Opus 自适应切换留待 F5 | ✅（以 s16 化解，自适应切换归并 F5）|
| E7 | 可观测补齐（A12）：mesh 端 `sender.silent_ms` + `first_frame_epoch_us`（绝对延迟可测）；hub 侧 `master.silent_ms/gaps` 早已在 | `mesh.rs`、`metrics.rs`、WebUI | mesh 引擎字段 + WebUI 显示落地；手机（APK）/PC 独立 center-send 引擎的对应上报未做 | 🟨 mesh 侧完成 |
| E8 | 双栈补测：hub 以 `--bt` 启动，重跑 T3 的 S1–S3 | `scripts/collect_1h.py` | 出现 wifi→bluetooth→wifi 的切换记录与三段耗时（当前为空白） | ⬜ |

估算：E0 0.5 天 · E1 2–3 天 · E2 1 天 · E3+E4 2 天 · E5 2–3 天 · E6 3–5 天 · E7+E8 2 天。若只有 3 天窗口，只做 **E0 + E1 + E3**（覆盖跳音、启动无声、拨号风暴）。

## Phase 3 · 安全（1–2 周，需一次设计评审）✅ 2026-11-04 核心落地（评审待开）

- F6：组密码 → 帧 AEAD（ChaCha20-Poly1305，PSK=SHA-256(密码)，Wi-Fi 全帧 + 心跳，31 B 开销）；选举签名（mDNS TXT `sig=HMAC(...)`，无效剥除 bit/bt）；WebUI 默认绑 `127.0.0.1`。蓝牙载体本期未加密（明示为弱载体）。
- 落点：`net/crypto.rs`（新）、`net/udp.rs`（收发）、`net/discovery.rs`（TXT）、`mesh.rs`（选举/发现校验）、`siren-cli`（CLI `--password`、WebUI）。
- 验收：未持密码的节点无法注入（AEAD 拒收，link_sim 错密码 0 包）/接管（签名剥除选举凭据）；"无密码 = 降级模式 + WARN"兼容策略写入 DEVELOPMENT.md §17。

## Phase 4 · 规模化与平台 ✅ 2026-11-05 核心落地

- F5 ✅：`UdpSender::set_codec`（保 stream id+seq 中途换码）；发送循环按心跳 RTT P99 s16↔Opus48k 自适应（>400 ms 降级、健康 30 s 恢复）；Opus 编码器开 in-band FEC（预期丢包 15%，承接 E6 残余）。真机弱网复测待做。
- F15 ✅：`GossipTable` + `GP01` 单播 gossip（每 5 s 全表广播，16 条目、30 s 有效期），mDNS 失效时网状自愈；与密码同路加密封送。
- F16 ✅：mDNS TXT `proto=MESH_PROTO_VERSION(3)`；更高版本对端跳过 + 告警（旧版兼容）。
- F20 ✅：`snapshot_json_contract_is_stable` 钉死 WebUI/FFI JSON 契约（hex u64、咔哒/静音/首帧 epoch/RTT 字段名）。
- F14 ✅：DEVELOPMENT.md §18 规模上限表（64 路掩码 / BT hub 2–3 ACL / gossip 64 / 单流带宽账 / ≤6 节点建议）。
- F12 🟨：`SirenPlaybackService`（mediaPlayback 前台服务）+ Manifest 权限已落地；**APK 真机回归待做**；channel→SPP 回退链维持（UUID-first 需 hub 注册 SDP 记录，见 §18 备注）。

## 已知风险与运维坑（来自三设备实测）

- `state 5` 半开 ACL：长时间反复重连后所有拨号被挡，需 `sudo hciconfig hci0 reset`；长跑前先复位蓝牙栈。
- NAS 上 `pkill -f` 会自伤，用 `ps -C siren -o pid=,args=` + 精确 PID。
- MIUI 卸载后 `adb install` 需机主在开发者选项重新允许 USB 安装。
- 手机 raw 双流上行 ~12% 丢包 + 修剪/欠载每秒 ~4 次是 MIX1 的活体复现场景（Phase 2 的对照现场）。
- PC↔NAS 的 RFCOMM 链路至今未建立成功——P0-1 内存 BT 传输不能拿它当参照，按手机↔NAS 的实测行为建模。
- NAS 时钟比 PC 快 50–60 s 且持续漂移（≈0.5%）：`timeline.csv`（PC 钟）与 hub/手机日志不能相减，跨机分析只用相对时间；绝对延迟必须由发送端自己打"首帧入队"标记。
- 采集只录 hub 的 monitor 时，手机的收听故障完全不可见（本轮 `Connection refused` 持续 20 s 就是这样漏掉的）；任何"三端可用性"结论都要有手机侧数据。
- 手机侧关键事实（`trace:`/`audible gap`/`hybrid:`）是 logcat 的 I 级，日志裁剪只留 E/W 会把证据全部丢掉。

## 变更记录

- 2026-11-03：按"先测后改"排期建档；Phase 0 启动。
- 2026-11-04（Phase 1）：s16/斜坡/限幅落地，实测 loss2 丢包 1.96%（分片叠加消除）、咔哒归零；数据见 docs/baselines/2026-11-04-phase1/。
- 2026-11-04（Phase 2）：冻结+渐隐（MIX1/F8）、LATE_DUPLICATE_LIMIT 收紧至 ~5 s、幽灵流载体门前移、F13 路由周期刷新、F7 克隆检测。统一验收（7×10 矩阵）：咔哒全 0、无流全 0、修剪全 0；loss2 1.99%/loss10 10.05%（分片叠加消除）；bw1800 26.7%→0%；delay200/jitter50 仍为重排主导（未排期）。见 docs/baselines/2026-11-04-phase2/。
- 2026-11-04：Phase 0 完成。基线要点（真实 Wi-Fi，PC↔NAS）：clean 0% 丢包；loss2 实测 ~4%（netem 2% 配置 ×2 待校准）；**delay200/jitter50 的高"丢包"（43%/67%）主要来自 netem 抖动重排 + 接收端无重排容忍窗口**（jitter50 欠载 1278/重置 407——这是 F2/F5 的直接靶子）；bw1800 欠载 3816。NAS 已部署新二进制（心跳/咔哒生效）。
- 2026-11-04（Phase 2.5 建档）：1 小时六阶段采集完成，暴露 12 项异常（`out/2026-11-04_1h/FINDINGS.md`、复核 `VERIFICATION.md`）。关键量化：手机侧一次修剪扔 751.6 ms（fill 851.6 ms，目标是 100 ms）且 80 ms 内连发 3 次；手机 12 min 内蓝牙拨号 470 次全失败（无退避）；发送端起播前持续发全零 8.0–8.3 s；手机上行 `EAGAIN` 与双向 1.1 s 空洞同时发生（T1 两次，四个 4 分钟阶段为 0）；`rate 2mbit` 下水位塌到 0、5 分钟 209.6 s 无声。排期见 Phase 2.5（E0–E8）。
- 2026-11-05（真机测试日）：F12 前台服务/后台存活通过；F5 真机降级+恢复路径验证（三个真因修复：adaptive 作用域/丢包率触发/TTL 存活回填；防振荡指数退避）；E3 真机 12 min 拨号 0 次；E1 同窗证据（12 min 零 EAGAIN/零咔哒/零修剪，流丢包 0.47%）；T-D 发现并修复 E4 回填回环地址缺陷（BT 隧道注入的 127.0.0.1 曾被写进对端地址）。NAS 三次挂起归因 netem 全双工限速 + 音频洪流，netem.sh 已加守卫、部署已限核降载。
- 2026-11-05（Phase 4）：F5/F15/F16/F20/F14 落地、F12 前台服务落地（APK 真机回归待做）；NAS 部署后 clean 双向 0 丢包、loss2 1.7–2.7%、自适应未误触发。
- 2026-11-04（Phase 2.5+3 执行）：E0（采集加固）/E1（修剪限速 ≤60ms·≥200ms）/E2（全零不上行 + silent_ms）/E3（拨号指数退避，470 次→≤10 次）/E4（IP 回填 + TTL 30s + 失败重发现 + 旧流 3s 退役）/E7（首帧 epoch）落地；E6 以 s16 默认化解；E5/E8 待真机。Phase 3 F6 核心落地（AEAD + 选举签名 + WebUI 绑定，蓝牙载体未加密，评审待开）。
