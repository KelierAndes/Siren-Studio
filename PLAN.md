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
| P0-4 | 基线输出命令 | `scripts/baseline.py` 一条命令跑矩阵并归档 `docs/baselines/<date>/`（README 含二进制/git/校准备注） | 修复前基线已存档 `docs/baselines/2026-11-04/`（旧二进制）；新二进制含 RTT/咔哒列。**phone↔PC 一组已补测**：2026-11-05 五档矩阵 S1/S2（`out/matrix_2026-11-05_1405/`）为 PC↔手机双栈与断手机 Wi-Fi 的三分钟窗口，含手机 logcat 与逐场景 metrics | ✅ 2026-11-05 |

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
| MIX1 (#64) | F8 冻结+渐隐替代销毁：超时 → flush+冻结 30 s（保留流状态）→ 包到即渐入恢复；修剪后不再重新预热（started 保持）+ 输出渐入 | `net/udp.rs` retain 路径、`dsp/resampler.rs` trim | 冻结/恢复/修剪单测 3 个全绿。**"真实修剪不再产生静音窗"已被 2026-11-05 五档矩阵否掉**：纯蓝牙叶子的 N−1 混音流每 3 min 仍 37–41 次 >100 ms 静音、169–226 次修剪（单次 ≤60 ms = E1 限速削生效），水位被顶到 403–438 ms 仍欠载——修剪↔饥饿自激只在窄载体 + N−1 转发路径上发作（S3 基线同窗干净：30/33 ms、丢帧增量 1440 帧≈30 ms）。2026-11-05 补 `tests/mix1_b7.rs`（hub + 两个纯 BT 叶子，Wi-Fi 前端全程 cut）：0xFFF0 混音同时到达两叶、`mix_count==2`（N−1 掩码 = hub 本地 + 另一叶、无自身回声）、无直连 Wi-Fi 流、水位 ≤300 ms 且无欠载风暴，5/5 稳定——**机制与水位在无损 sim 下干净，自激确认为真机窄载体特有**（B7 现场项待 NAS 稳定后复测） | 🟨 机制+sim 验收完成，真机自激待 B7 复测 |
| F13 (#68) | `UdpSender::refresh_route()`：每 30 s 重连 socket 重解析路由（不动 packer，stream id/seq 不变） | `net/udp.rs` + `mesh.rs` 发送循环 | 机制此前是死代码，ed4f77a 接入发送循环；`UdpSender::retarget` 加心跳守门（sent≥4 且 0 回声或丢包≥40% 才改绑）——真机健康窗口跟随次数 39→0、真故障窗口仍能重绑（`out/matrix_2026-11-05_1405/retarget-evidence.md`）。**唯一未量化项**：恢复后 ≤30 s 内入向载波回到 wifi 的专项采样（现有矩阵在恢复后只等 30 s 不采载波） | ✅ 2026-11-05（机制+真机跟随，剩一次采样）|
| F7 | 双端检测：接收端 origin 出现两个源地址 → 告警；mesh 检测 mix_tap 中自己的 uid → 告警 + 重生成持久化 node id（下次启动生效） | `net/udp.rs`、`mesh.rs` | 检测逻辑落地；克隆场景单测待造 | ✅ 2026-11-04 |
| LATE_DUPLICATE_LIMIT | `-250_000` → `-1_000`（~5 s）；发送端重启单测 + 陈旧 BT 副本仍为重复的单测 | `net/udp.rs` | 重启后立即重同步（8/8 稳定）；5 s 内迟到副本仍被静默丢弃 | ✅ 2026-11-04 |
| 幽灵流 | 载体门移到建流之前：Wi-Fi 健康时 BT 副本不再建出 0 包僵尸流（首触 BT 流仍放行） | `net/udp.rs` create_peer 前 | 载体门逻辑前移；既有仲裁单测全绿 | ✅ 2026-11-04 |

## Phase 2.5 · 现场暴露问题 ✅ E0-E4/E7/E8 落地并真机复测；仅剩 E5（A1 三组对照，需人工改手机设置）

来源：`out/2026-11-04_1h/`（T1/T3/T4/T6 六阶段，1 小时）与结论 `FINDINGS.md`、复核 `VERIFICATION.md`。顺序即优先级：先让数据可信（E0），再修用户能直接听出来的（E1/E2），再定位（E5）。

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| E0 | 采集基座可信化：阶段启动前检查 47900 占用并清理残留 `siren.exe`；`pc.log/pc.stats` 按 run 命名；NAS 开 NTP；包内默认带 `master.csv`/`streams.csv` + 手机 I 级运行行 | `scripts/collect_1h.py`、打包脚本 | 清理/端口探活/按 run 命名/NTP 均落地（`clear_stale_senders`、`pc_<stamp>.*`、`set-ntp`）；"连跑两轮"已在 2026-11-05 执行（同日 4 批矩阵 1304/1319/1338/1405，端口占用与陈旧 sender 清理生效） | ✅ 2026-11-05 |
| E1 | 修剪改限速削（A3/A4，承接 MIX1）：单次 ≤60 ms、间隔 ≥200 ms，逐步回落到 target+10 ms；等突发结束再削；push 侧加硬上限，禁止 ring 一次吃进 1 s | `dsp/resampler.rs`、`net/udp.rs` push 路径 | 限速削全部落地（≤60 ms/次、≥200 ms 冷却、突发缓削、4× 天花板紧急直落）；push 侧以 ring 容量(1 s) + `overflow_packets` 计数为界，未另加软上限。真机复测已做：单次丢弃不再超 60 ms（原 751.6 ms），但**没有消除自激**，残余计入 MIX1 | ✅ 2026-11-05（自激另立 #64）|
| E2 | 全零不上行（A5）：采集连续 ≥200 ms 全零则不发音频帧（保心跳），快照 `sender.silent_ms`；WebUI 状态区标"源静音" | `mesh.rs` 发送循环、`metrics.rs`、WebUI | 抑制逻辑 + 字段 + UI 徽标落地；真机复测已做：NAS 静默窗口 `capture_silent_frames == capture_frames`、`kbps 0.014–0.04`、`tx_rms_db=-120`，回环复位后仍成立 | ✅ 2026-11-05 |
| E3 | 拨号指数退避（A2）：1/2/4/8/16/30 s 封顶，连续 5 次失败后降为 5 min 探测 | `hybrid.rs` 离线拨号 | 真机 12 min（BT 阻断）拨号 **0 次**（≤10 达标）；注意：手机侧半开 RFCOMM 的缓冲写（tx 7.8 KB/s）骗过双向看门狗，链路未被判死故未触发重拨——看门狗 tx 判据需改进（新发现，待排期） | ✅ 2026-11-05 真机 |
| E4 | 端点回打与发现失效（A6/A7）：mix_tap 记收包源地址回填对端 IP；发现 TTL 60→30 s；发送失败 3 次剔除端点触发重发现；换 stream id 后旧流 3 s 内退役（N−1 混音流豁免） | `net/udp.rs`、`mesh.rs` | 全部落地。真机复测已做：断手机/NAS Wi-Fi 期间对端仍被正确寻址、混音到达叶子；回填只取非回环源（T-D 缺陷未复现）。同批复现并修掉一个新缺陷——发现层在双网卡 NAS 上轮流给 `.159/.196` 两个 A 记录，"地址一变就跟"导致健康窗口 39 次重绑，已加心跳守门 | ✅ 2026-11-05 |
| E5 | 定位 A1：三组 12 min 对照（只关蓝牙 / 只关 Wi-Fi 省电 / 都关），手机 stats 增加 `tx_eagain`、roam/rssi、wakeup 计数，定时抓 `dumpsys wifi` | `scripts/collect_1h.py`、Android stats | 产出"EAGAIN + 1.1 s 洞是否随某项消失"判定表；三项均无效则转固件/AP，不再改软件 | ⬜ |
| E6 | 弱网自动降级（A10/T6_N4）| — | **s16 默认已化解主场景**：1.58 Mbps < `rate 2mbit`，T6_N4 型塌水位不再发生；<1.6 Mbps 的 Opus 自适应切换留待 F5 | ✅（以 s16 化解，自适应切换归并 F5）|
| E7 | 可观测补齐（A12）：mesh 端 `sender.silent_ms` + `first_frame_epoch_us`（绝对延迟可测）；hub 侧 `master.silent_ms/gaps` 早已在 | `mesh.rs`、`metrics.rs`、WebUI | mesh 引擎字段 + WebUI 显示落地；手机（APK）/PC 独立 center-send 引擎的对应上报未做 | 🟨 mesh 侧完成 |
| E8 | 双栈补测：hub 以 `--bt` 启动，重跑 T3 的 S1–S3 | `scripts/collect_1h.py` | 已由 2026-11-05 真机测试日执行（d61cff9）：`Wi-Fi 断 8 s → BT 接管（tx 0→6.2 KB/s）→ 恢复` 的切换记录与三段耗时已产出；同日五档矩阵再复核：NAS 侧 N−1 混音流在断手机/断 NAS Wi-Fi 时 179/180 次采样存在 | ✅ 2026-11-05 |

估算：E0 0.5 天 · E1 2–3 天 · E2 1 天 · E3+E4 2 天 · E5 2–3 天 · E6 3–5 天 · E7+E8 2 天。若只有 3 天窗口，只做 **E0 + E1 + E3**（覆盖跳音、启动无声、拨号风暴）。

## Phase 3 · 安全（1–2 周，需一次设计评审）✅ 2026-11-04 核心落地（评审待开）

- F6：组密码 → 帧 AEAD（ChaCha20-Poly1305，PSK=SHA-256(密码)，Wi-Fi 全帧 + 心跳，31 B 开销）；选举签名（mDNS TXT `sig=HMAC(...)`，无效剥除 bit/bt）；WebUI 默认绑 `127.0.0.1`。蓝牙载体本期未加密（明示为弱载体）。
- 落点：`net/crypto.rs`（新）、`net/udp.rs`（收发）、`net/discovery.rs`（TXT）、`mesh.rs`（选举/发现校验）、`siren-cli`（CLI `--password`、WebUI）。
- 验收：未持密码的节点无法注入（AEAD 拒收，link_sim 错密码 0 包）/接管（签名剥除选举凭据）；"无密码 = 降级模式 + WARN"兼容策略写入 DEVELOPMENT.md §17。

## Phase 4 · 规模化与平台 ✅ 2026-11-05 核心落地

## Phase 4.5 · 真机测试日遗留（2026-11-05）🟨 W2/W5 已关闭；剩 W1（看门狗判据）、W3（F5 长观察，需换机）、W4（=E5，需人工设置手机）

| 任务 | 内容 | 落点 | 验收 | 状态 |
| :-- | :-- | :-- | :-- | :-- |
| W1 | 看门狗 tx 判据改进：半开 RFCOMM 的缓冲写（tx 7.8 KB/s）骗过"双向 <4 KB"判据，手机侧永不判死链路、不重拨。tx 项改为"对端确认的流量"（如 hb 经链路往返）或缩短纯 tx 判死窗 | `hybrid.rs` 链路看门狗 | **双通道关闭（2026-11-06）**。sim：`tests/w1_halfopen.rs`（TCP 代理真半开，keepalive 判死+重拨+二轮自愈，5/5）。真机：SIGSTOP hub 复刻 E3（`out/2026-11-06_w1_e3/`）——SIGSTOP 后 **3.7 s** 判死（原判据 ≤6 s），keepalive 全程保持，重拨循环自愈，SIGCONT 后 1.8 s 链路重开并稳定。真机测试暴露并修复两个真缺陷：① mDNS TXT 从不发布 `proto` → keepalive 在一切真机上永不协商（898b9a4）；② hub 掉出 mDNS 时 keepalive 协商自关——恰在最需要时（6b49876 pong 锁存）。注：#38 其余部分已在 ed4f77a 关闭 | ✅ 2026-11-06 |
| W2 | 手机 UI 胶囊脱同步修复复验：引擎释放后清陈旧快照（`_finishLeave` 设 `mesh=null`）已落地，装机复验"胶囊与组网卡同一时刻一致" | `app_model.dart`、新 APK | bd3f39c 已装机复验：单次点击 join → 18 s 后胶囊与组网卡完全一致（组网中·2台 / 已连·1路 / 离开组网） | ✅ 2026-11-05 |
| W3 | F5 恢复路径长观察：netem rate 测试需要非 NAS 测试机（该机禁全双工限速，见 netem.sh 守卫）；在可用测试机上跑 degrade→restore 完整周期 ×10 | 换测试机 | 恢复触发率 100%、无振荡（退避生效） | ⬜ |
| W4 | E5 三组对照实验（定位 A1）：手机侧 飞行模式只留 Wi-Fi / 关省电 / 都关，各 12 min，对比 EAGAIN+1.1 s 洞是否消失；手机 stats 增加 `tx_eagain`/roam/rssi/wakeup | `scripts/collect_1h.py`、手机设置（需人工参与） | 判定表产出；三项均无效则转固件/AP | ⬜ |
| W5 | 手机 UI 状态胶囊数据源统一：胶囊读 `mesh.running`（FFI 启动流），组网卡读快照——引擎自愈/重启后两者可能脱同步；统一到快照派生 | `mobile_shell.dart`、`app_model.dart` | **核验后判定已达成，不需要再实现**：胶囊取 `meshRunning => mesh?.running == true`（`app_model.dart:73`），而 `mesh` 本身就是 80 ms 快照轮询得到的 `MeshState`，组网卡读同一对象（`mesh_mobile.dart:59`）；bd3f39c 又让引擎释放时置 `mesh=null`，自愈重入网在下一次轮询即翻转 | ✅ 2026-11-05（核验关闭）|

- F5 ✅：`UdpSender::set_codec`（保 stream id+seq 中途换码）；发送循环按心跳 RTT P99 s16↔Opus48k 自适应（>400 ms 降级、健康 30 s 恢复）；Opus 编码器开 in-band FEC（预期丢包 15%，承接 E6 残余）。真机弱网复测待做。
- F15 ✅：`GossipTable` + `GP01` 单播 gossip（每 5 s 全表广播，16 条目、30 s 有效期），mDNS 失效时网状自愈；与密码同路加密封送。
- F16 ✅：mDNS TXT `proto=MESH_PROTO_VERSION(3)`；更高版本对端跳过 + 告警（旧版兼容）。
- F20 ✅：`snapshot_json_contract_is_stable` 钉死 WebUI/FFI JSON 契约（hex u64、咔哒/静音/首帧 epoch/RTT 字段名）。
- F14 ✅：DEVELOPMENT.md §18 规模上限表（64 路掩码 / BT hub 2–3 ACL / gossip 64 / 单流带宽账 / ≤6 节点建议）。
- F12 🟨：`SirenPlaybackService`（mediaPlayback 前台服务）+ Manifest 权限已落地；**APK 真机回归已做一部分**——2026-11-05 五档矩阵中手机整窗运行（含两次断 Wi-Fi 的 3 min 窗口，logcat 全程采集，入向/混音计数正常），但 channel→SPP 回退链未专门复测（UUID-first 需 hub 注册 SDP 记录，见 §18 备注）。

## 已知风险与运维坑（来自三设备实测）

- `state 5` 半开 ACL：长时间反复重连后所有拨号被挡，需 `sudo hciconfig hci0 reset`；长跑前先复位蓝牙栈。
- NAS 上 `pkill -f` 会自伤，用 `ps -C siren -o pid=,args=` + 精确 PID。
- MIUI 卸载后 `adb install` 需机主在开发者选项重新允许 USB 安装。
- 手机 raw 双流上行 ~12% 丢包 + 修剪/欠载每秒 ~4 次是 MIX1 的活体复现场景（Phase 2 的对照现场）。
- PC↔NAS 的 RFCOMM 链路**已建立**（2026-11-05 五档矩阵）：S3/S4 里 NAS 作为叶子存活链路末/峰 1/1、hub 显示 `SIREN-PC`；S5 断 NAS Wi-Fi 后 NAS 仍收到中心 N−1 混音（179/180 次采样存在）。旧的"至今未建立成功"结论来自链路计数被冻结在虚高值的观测缺陷（ed4f77a 已修）与 state 5 半开 ACL，不能拿它当 P0-1 内存 BT 传输的参照。
- NAS 是双网卡机器，且归属与直觉相反：`enp1s0 = 192.168.2.196`（metric 100，默认路由，也是 SSH 地址），`wlp3s0 = 192.168.2.159`（metric 600）。`ip route get 192.168.2.107` 实测走 `dev enp1s0 src 192.168.2.196` —— 所以 **mesh 流量一直走以太网**，`nmcli device disconnect wlp3s0` 只切掉次要出口，并没有把 NAS 与 LAN 隔离（S5 旧语义就是空转，现改为同时断 enp1s0 + wlp3s0）。mDNS 在同子网两个接口上轮流返回 A 记录，是"跟随地址"抖动的来源（实测 3 分钟健康基线 39 次）：任何"对端地址变了就跟随"的逻辑都必须带存活证明。
- Linux 节点的 `play_sink` 是 pactl 临时 null sink，PipeWire 一重启就消失；隔离又会把默认 sink 设成被录制的 `siren_capture`，所以播放落空即等于自激回环（判据：`tx_rms_db≈+9 / kbps 5680` 为回环，`tx_rms_db=-120 且 capture_silent_frames==capture_frames` 为干净）。现在由 `isolate.rs` 自查重建。
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
- 2026-11-05（现场四问题 + 五档矩阵复验，ed4f77a）：① 发送端 `UdpSender::retarget` + F13 接入发送循环，并加心跳守门（健康窗口跟随 39→0，真故障仍能重绑）；② `links` 无条件发布（2 台设备显示 7 路的计数冻结已修）、选举/拨号排除同网卡自拨；③ 载波语义诚实化（`path` 未收包为空、停流 1 s 不再回吐冻结 FFT、面板"路径"→"入向"、新增 `mix_count`、混音卡写明"含 N 路"、只收混音时 EQ 窗不再空白）；④ 台数与设备卡同源（快照按 node id 去重 + 压制同 IP gossip 行）。Linux 自激回环根因结案：`isolate.rs` 显式 `--output-sink` 缺失时重建 null sink（此前静默退回被录制的默认 sink）。矩阵：S3 基线 NAS 末水位 30/33 ms、PC→NAS 丢帧 15120→1440 帧；S4/S5 混音流 179/180 采样到达叶子且 NAS 增量欠载/丢帧为 0，叶子侧仍有 37–41 次 >100 ms 混音静音 + 169–226 次修剪 → **MIX1 重新打开（🟨）**。核验关闭：E0/E1/E2/E4/E8、P0-4 phone 组、W2、**W5（胶囊与卡片本就同源）**；仍开放：E5/W4、W3、W1、F12 的 channel→SPP 回退复测。
- 2026-11-05（2h 编排准备）：新增 `scripts/e2e_2h.py`（P0–P8）、`summarise_e2e.py`、`fuzz_proto.py`；P0/P1 冒烟 = 87 项单测全绿、raw/opus 混音、13 个畸形报文无 panic。Linux“又以内录模式入网”复现为**脚本缺陷而非引擎**：`deploy_nas.py` 用生产默认 `--source loopback` 起守护，`ensure_nas_joined` 只看 running/peers 就判合格（上行被 E2 抑制成全零）→ 部署改为 `--source tone --freq 440`，入网断言同时校验 `config.source`（不符先 `/api/stop` 再按测试配置加入），P0 另加“tone 但上行 ≤ −60 dBFS 即 FAIL”。F5 恢复退避 `ADAPT_BACK_TICKS=60` 按循环次数计，5 ms 帧下实际只有 0.3 s（注释与设计写 30 s）→ 改为按 `frame_ms` 换算（30 s/帧长），待 W6 实测。
- 2026-11-05（环境）：PC 时钟曾被 w32time 停用拖慢 62 s；启动服务后变成“快约 197 ms”（SNTP 与 `w32tm` 一致；参考源 time.windows.com 根散布 7.9 s，只 slew 不跳变）。管理员侧把 w32time 换到 ntp.aliyun.com 并将 `MaxAllowedPhaseOffset` 临时置 0 触发一次跳变后，PC 稳定在 **−4.5…−5.4 ms**；NAS（systemd-timesyncd→ntp.tencent.com）+3.3 ms，端间差约 8 ms；手机 auto_time=1（NTP 不确定度 16 ms）。P0.3 判据改为单端 ≤10 ms、端间 ≤15 ms（5 ms 落在 SNTP ±3 ms 复现性噪声内），实测值仍逐项写入 meta；跨机绝对延迟结论以此偏差为不确定度。
- 2026-11-05（2h 复测后的注入/取证修复）：netem 的 ingress 过滤一直按 `sport 47900` 匹配，而发送端用临时端口（实测 `.107:55519`）→ **所有「上行/双向」注入从未生效**；改为按 `dport 47900` 后 loss5 校准 5.34%。这解释了上一轮 W10 的 16.3 s 缺口：那不是 10% 丢包造成的，而是当时的手机 Wi-Fi 环境事件；真实 5.5% 上行丢包下只有 18 ms 静音、手机 0 缺口。同批修复：`lost_frames` 记的是采样数（前端「丢帧」被放大 240–480 倍）→ 改记帧并加单测；C6 新增 loss5 档、注入失败直接 FAIL、校准前关 NAS 蓝牙（双栈副本会把 5% 补成 2.6%）；C7 改为 NAS 上的独立静音节点（tone 不走采集管、loopback 会被自身播放污染），实测 silent_ms 24.8 s、上行 0 包；手机日志按用例清缓冲、每用例保存 NAS/PC 日志切片、daemon 重启前归档；B7 断网期间改由 NAS 本机采样，并要求 bt 载体样本 ≥30 才下结论；netem 99–100% egress 会连带掐断 harness 的 SSH（复现三次），W3/B3/浸泡的黑洞改用 iptables 静默丢 UDP/47900；W4 按计划记为已知开放、W5 改为每 10 s 一次的 0.3 s 尖峰。验证：P3 全绿（W1 1.52%、W2 2.59%、W3 3 次切换、W4 known-open、W5 0 次 F5 降级、W10 5.5%/18 ms）。
- 2026-11-05（交接）：按用户要求**暂停 NAS Wi-Fi/双网卡中断测试**（B7→SKIP）。交接文档 `out/2026-11-05_handover/HANDOVER.md`，含未测试项（C3/C4/C5、W7/W9/W11/W12/W13、B1/B4/B6/B7/B8/B10、S5/S6/S7、F3/F6/F7/F9/F10、P8 逐音、UI 清单）、未处理问题（MIX1 改不断网方案、干净基线 underrun/trim、F8 Doze 4.9 s、W1 保活未验收、时钟余量、双栈副本掩盖丢包、NAS 断网流程不可靠）与下一步顺序。当前 HEAD `0a314da`。
- 2026-11-05（MIX1 sim 验收）：新增 `tests/mix1_b7.rs` —— hub 无静态对端（自选举 solo hub）+ 两个纯 BT 叶子（Wi-Fi 前端 `spawn_link` 后立即 `cut()`，hub 只经隧道见到它们的音频）。断言：双叶拨号入网、hub 持 2 链路、0xFFF0 混音到双叶且 `mix_count==2`、`rms_db>-60`、`path=="bt"`、叶子无直连 Wi-Fi 流、水位 1–300 ms 且欠载 ≤4。5/5 稳定，audio-core 全量 89 测试绿。结论：N−1 机制与水位在无损 sim 下干净，真机窄载体自激（五档矩阵 37–41 次 >100 ms 静音）确认为现场特有，待 B7 复测定位。
- 2026-11-05（underrun/trim 计数器口径审计，关闭交接遗留问题 2）：`underruns` = render 回调数（一次回调缺 1 输出帧=20 µs 也计 1，resampler.rs render 逐帧缺失路径）；`underrun_frames` = 精确缺帧数，但满块重预热路径（缓冲 <4 帧）整块计入——**它大才是真饥饿，≈0 说明全是毛刺**；`trims`（=buffer_resets）每次回调最多计 1，单次幅度只在 `trace: trim` 日志（fill_before/dropped）。C1 的 10 underruns/18 trims（4 min）因此**无法从现有取证判定严重度**——harness `digest()` 没抓 `underrun_frames`，NAS 日志切片也没存。已修：`e2e_2h.py` digest/delta 增抓 `underrun_frames`；后续 C1 类基线必须同时报两个数，且保存 NAS 日志切片。
- 2026-11-06（W1 sim 验收，关闭交接遗留问题 3）：新增 `tests/w1_halfopen.rs` —— TCP 代理在 leaf 与 hub 的 SimBt 之间做真半开（leaf→hub 永续转发；hub→leaf 翻转后读后丢弃，TCP 栈持续 ACK，复刻 E3 实测的"tx 7.8 KB/s 假活"）。验证：`?proto=4` 静态 pin（新解析，`parse_pinned_peer` 此前不认 proto → 静态拓扑永远不启 keepalive，属真实缺口非测试便利）→ keepalive 在下行静默 2 s 后发 SKP1 探针、3 s 无应答 → `ka_dead++`、拆链、退避重拨，新一轮半开再次自愈。健康期 `ka_rt==0` 是符合设计的（双栈下行不静默 → 不探针）。5/5 稳定，audio-core 全量 91 测试绿。W1 剩真机 E3 复测（需手机）。
- 2026-11-06（W1 真机验收 PASS，原判据 ≤6 s）：SIGSTOP hub（ACL 保持、内核缓冲假活）复刻 E3——修复前首跑当场暴露第二个真缺陷：hub 掉出 mDNS → 拓扑清空 → keepalive 协商 2 s 内自关，死链路被无限持有（tx 7.8 KB/s 骗过字节窗 45 s+）；锁存修复（pong 确认按 hub uid 锁存，hub 更换才重置，6b49876）后重跑：**3.7 s 判死** → 重拨循环自愈 → SIGCONT 后 1.8 s 链路重开稳定。真机还证实 mDNS TXT `proto` 广播修复双向生效（898b9a4，此前真机 keepalive 永不协商）。证据 `out/2026-11-06_w1_e3/`（10 次 keepalive unanswered、phone/nas 日志切片）。
- 2026-11-06（F8 Doze 取证，关闭交接遗留问题 4）：熄屏 + `deviceidle force-idle` 90 s（mState=IDLE、app 进程存活），NAS 逐秒采样 180 快照：包速率 ~230/s 连续、水位 25–32 ms、silent_ms 增量 0.0 s、F8 冻结未触发、同一 stream id。结论：**F12 前台服务把 Doze 完全挡住**，交接的"F8 Doze 4.9 s"在本配置下无法复现（当时缺 F12）；F8 的事件验证由 W1 E3 hub 冻结窗口覆盖（60 s 静音→恢复）。证据 `out/2026-11-06_f8_doze/`。
