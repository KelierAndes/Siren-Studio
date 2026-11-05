# SirenStudio 交接：2026-11-05 全量测试与修复轮

> 交接时的代码 HEAD：`0a314da`（本条之后的脚本改动全部已提交）。
> 设备已复原：NAS 双网口 connected、autoconnect=yes、无 iptables/netem 残留、守护以 `--source tone` 入网；PC 应用未运行；手机已装最新 APK（本地与设备 md5 均为 `511cd20d…`）。

## 1. 本轮做了什么

| 提交 | 内容 |
| :-- | :-- |
| `001d7da` | 2h 编排落地（e2e_2h/summarise/fuzz）、F5 退避按帧长换算、NAS 部署断言音源 |
| `843fe30` `037ca0d` | PLAN 记录内录模式根因、PC 时钟修复 |
| `ebdfe7b` `df6a1f3` | P0.3 改为 SNTP/timesyncd 偏差；判据单端 ≤10 ms / 端间 ≤15 ms |
| `e634a34` | P0 先断言入网再查音频图；缺 sender 不再默认 0 dBFS |
| `5848770` `c2a3cd5` | 全量运行记录、无效测量修订、48 vs 120 分钟的口径差 |
| `3b04f10` | **netem 上行按 dport**（原来按 sport，所有上行注入空转）、`lost_frames` 改按帧计、C6/C7/B7 重做、手机日志按用例切片、NAS 日志归档 |
| `632cfa9` | 黑洞改用 iptables（netem 99–100% egress 会掐断 SSH）、W4 记已知开放、W5 改周期尖峰 |
| `f814607` | PLAN 汇总上述取证缺陷 |
| `0a314da` | **暂停 NAS 断网测试**（B7→SKIP）；`nas_lan_cut` 改为改 profile autoconnect（未端到端验证） |

## 2. 可采信的验证结果（最新轮）

- P0：端口/对时/入网/隔离/netem 干跑全 PASS（对时 PC −4 ms、NAS +3 ms、差 7 ms）。
- P1：单测 88 项全绿、raw/opus 混音、13 个畸形报文无 panic。
- P2：C1 PASS（4 min：NAS underruns 8、trims 19、silent 0，手机 0 缺口）；**C6 5.34%**（关 NAS 蓝牙排除双栈副本）；**C7 PASS**（独立静音节点 silent_ms 24.8 s、上行 0 包、采集全零）。
- P3：W1 1.52%、W2 2.59%（5% 注入被双栈副本补掉约一半）、W3 已知开放（3 次切换）、W4 已知开放（表观丢包 37.97%）、W5 PASS（6 次 150 ms 尖峰、0 次 F5 降级）、W6 PASS（degrade 47.1 s / restore 81.8 s）、**W10 PASS（5.5% 上行丢包、静音 18 ms、手机 0 缺口）**。
- P4：B2 PASS（4 次硬断，首次采样即回 Wi-Fi）、B3 已知开放（9.32% 丢包、7.9 s 静音）、B5 PASS、B9 PASS（60 s 内拨号 0 次）。
- P5：S1–S4 全 PASS（存活 4/8/16/32，hub 发现 6/10/18/34）。
- 全量 64 用例的原报告：`out/2026-11-05_e2e_full/`（NOTE 中的四类无效测量已由上述提交修正）。

## 3. 未测试项

| 计划项 | 原因 |
| :-- | :-- |
| C3 信号链黄金向量、C4 `--tick-skew-ppm`、C5 采集 A/B | 检测器/参数未实现或需外部连续正弦源 |
| W7 密码链路重复+错帧、W9 MTU、W11 手机 iperf3、W12 组合限速、W13 编码对比 | 未编排 / 需非 NAS 测试机 |
| W8 限速 1.2 Mbit | 需非 NAS 测试机（netem.sh 禁止全双工 rate） |
| B1 纯蓝牙稳态、B4 flapping、B6 双断顺序、B8 hub BT 故障转移、B10 交叉流量 | 未单列/未编排 |
| **B7 MIX1 专项** | **按用户要求暂停 NAS Wi-Fi/双网卡中断** |
| S5 64/65 节点与重复 uid、S6 join/leave 抖动、S7 安全协议套件 | 需分机/单独环境 |
| F3 陈旧端口、F6 PC 渲染切换、F7 时钟跳变、F9/F10 CPU 压力 | 未编排或需人工确认 |
| P8 逐音离线分析 | daemon 未带 `--record`，无 WAV 输入 |
| UI 功能清单（C1 的加入/离开、台数、入向文案、缓冲目标实时调整、WebUI 契约） | 本轮未做人工/自动化 UI 点击 |
| E5/W4 手机三组对照、E7 手机/PC 独立引擎上报、F12 channel→SPP 回退 | 需人工改设置/独立窗口 |

## 4. 未处理的问题（按优先级）

1. **MIX1（#64）**：N−1 混音是否到达纯蓝牙叶子，目前**没有可信结论**。最新一次 B7（PC hub 在场）场景未成立（断网未隔离，bt 样本 0）；更早一次“独立调用”BT 样本 269 但当时没有 hub，不能算数。需要**不断网**的方案，例如：
   - 用一台有线/Wi-Fi 正常但只收 BT 的叶子（软件禁用 Wi-Fi 网卡驱动而非 `nmcli` 断连，或让叶子以 `--no-discover` + 仅 BT 对端入网）；
   - 或在 `link_sim`（内存链路）里直接构造“hub + 两个 BT 目的”，测 mix 0xFFF0 的到达与水位，避开真实断网。
   重新开测前先恢复 `e2e_2h` 的 B7 用例（现在标 SKIP）。
2. **干净基线 underrun/trim**：C1 4 min 有 8–13 次 underruns、19–20 次 trims；浸泡 14 min 有 108 次 underruns（G2 为 ≤1/5min/流）。历史基线是“clean 0 修剪”，需要先核对计数器语义（是否含只持续一帧的欠载）再定位是时钟漂移、mix 的 10 ms 帧还是调度抖动。
3. **F8 熄屏 + Doze**：窗口内 1 次 4.9 s 缺口。`SirenPlaybackService` 前台服务没有完全挡住 Doze；建议对照 `dumpsys deviceidle` 与 `audio` 日志，确认是 AAudio 被暂停还是采集被限。
4. **W1 带内保活（#38）**：所有窗口 `ka_rt/ka_dead` 都是 0，没有任何用例真正打到“带内判死半开链路”。需要一个场景：PC↔NAS 的 RFCOMM 半开（对端静默但写不报错），观察 5 s 内 `ka_dead` +1、`links` 归零。
5. **时钟余量**：PC 对 NTP 在 −4…+6 ms 摆动、PC-NAS 差 7–14.6 ms，已接近 15 ms 判据；w32time 轮询 1024 s，长跑可能超过。建议管理员把 `SpecialPollInterval` 降到 64 s（未改，避免再动系统配置）。
6. **双栈副本掩盖注入丢包**：W2 注 5% 实收 2.6%、W10 注 10% 实收 5.5%。凡是需要“真实丢包率”的校准（C6）必须关掉被测路径的备份载体，已在 C6 实现；其余用例应按“实收”解读。
7. **netem egress 99–100% 会掐断 harness SSH**（复现 3 次）：黑洞已全部改用 iptables；`netem.sh blackout` 档保留但不要再用 egress。
8. **NAS 断网流程不可靠（B7 暂停的直接原因）**：`nmcli device disconnect` 后 NetworkManager 会立刻重连；已把逻辑改成“记录 profile 并修改 `connection.autoconnect`”，但**未端到端验证**（用户已要求停止该类测试）。`nas_lan_cut` 现版本若将来恢复使用，先做一次 10 s 断开 + HTTP 不可达验证再跑用例。
9. **F12 端点剔除/重发现**只做了 20 s 单向丢包，未见 hub 日志层面的“3 次失败剔除”；需要补日志断言。
10. **S4 N=32** 上一轮曾 0 发现，本轮改为断言进程存活后 PASS（32/32、34 台）；仍需确认“进程存活但发现 0”是否还会出现。

## 5. 环境与运行方式

- 全量/子集：`python scripts/e2e_2h.py --nas-pass <pw> [--only P0,P1...] [--out out/<dir>]`
- 产物：每用例 `result.json` + `phone.logcat.txt` + `nas.log`（本用例窗口）+ `pc.log`；汇总 `SUMMARY.md`。
- 离线逐音：`siren analyze <wav> --freq 440 --freq 660 --freq 880 --json`（需要先带 `--record`）。
- 关键路径：`scripts/e2e_2h.py`、`scripts/matrix_dualstack.py`、`scripts/linux/netem.sh`、`out/2026-11-05_e2e_full/NOTES.md`。
- 设备约束（仅此测试机有效，不得外传/他用）：NAS `192.168.2.196`，账号 `kelierandes`，sudo 密码在历史命令中；NAS 上勿用 `pkill -f`；长跑前 `hciconfig hci0 reset`。

## 6. 下一步建议顺序

1. 用 `link_sim` 或“只关叶子 Wi-Fi 驱动、不断 NAS”的方案重开 MIX1（#64）。
2. 查 C1/浸泡的 underrun 计数口径与来源（第 2 项）。
3. 补 F8 的 Doze 证据并修正前台服务（第 3 项）。
4. 设计 W1 半开链路验收（第 4 项）。
5. 把时钟轮询降到 64 s 后再跑一轮全量（若允许改 Windows 时间配置）。
