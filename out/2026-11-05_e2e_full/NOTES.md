# 2h 全量测试 — 运行记录与数据可信度说明

- 开始 2026-11-05 20:37，结束 21:25（48 min；计划 108 min，差异来自 13 个 SKIP / 13 个 N/A 与浸泡只 14 min）
- 代码 HEAD `df6a1f3`；三端产物同源（UI dll 19:51 / APK 19:53 / NAS 20:00）
- 时钟：PC −3.8 ms、NAS +3.3 ms、端间 7.1 ms（P0.3 判据单端 ≤10 / 端间 ≤15 ms）
- 用例表 `cases.csv`，汇总 `SUMMARY.md`，逐用例 `result.json` + `phone.logcat.txt`（部分用例）

## 总计

PASS 30 / FAIL 2 / KNOWN-OPEN 6 / N/A 13 / SKIP 13（64 项）

## 判定修订（脚本缺陷，数字不可直接采信）

| 用例 | 原判定 | 修订 | 证据 |
| :-- | :-- | :-- | :-- |
| C6 丢包校准 5% | PASS | **INVALID（未注入）** | `netem.sh` 没有 `raw5` 档；`lost=0` 与 W2（loss2）的 216,240 对照即知 60 s 内没有任何丢包 |
| C7 静音源 E2 | FAIL | **INVALID（打错对象）** | 读的是 NAS 的 tone 发送端（tx −14.8 dBFS），不是被拉起的静音 PC 节点；且 PC 已在跑，第二个实例无法绑定 47900 |
| B7 MIX1 专项 | KNOWN-OPEN | **INVALID（场景没成立）** | 自称断 NAS 双网卡，但 238/238 次 HTTP 采样全部成功、`paths=["wifi"]`、`streams=1`——NAS 全程在 Wi-Fi 上，`mix 0/238` 不是 MIX1 证据 |
| 手机计数（gaps/switches/dials） | 原值 | **整缓冲累计，按用例窗重算** | `phone_logcat()` 每次 `adb logcat -d` 全量转储；`phone_counts()` 计的是环形缓冲里的全部历史。重算表见 `phone_window_counts.csv`，例如 W10 的 dial=2044 实际窗口内为 0 |
| S4 N=32 | KNOWN-OPEN（0 台） | 记录为**压力档上限** | N=16 发现 18 台，N=32 发现 0 台；属合成本机进程/发现能力边界，尚无产品结论 |
| F8 Doze | 16.7 s 缺口 | **窗口内 1 次 4.9 s 缺口** | 16.7 s 是整缓冲累计值；按 21:08:16–21:09:18 切窗后仅 1 个 4.9 s 缺口 |

## 真实异常（需要跟进）

1. **W10 非对称 10% 上行（FAIL）**：注入 60 s 后 30 s 起，手机出现 12 次静音、最长 16.3 s、累计 26.5 s（窗口 63 s）；NAS 混音静音 23 s、三路缓冲末值 [0,0,0]、65 underruns；PC 日志 `degraded ... switching to Opus`；手机 BT 链路 `p99 2000 ms / stalls>500ms: 5`。10% 丢包不应导致这种量级，且 W10 窗口同时出现 PushService ping 超时（不只是本应用）。**待复测**：需抓包 + NAS 本地日志（本轮 NAS 日志随后续 daemon 重启被截断）。
2. **F8 熄屏 + Doze**：窗口内 1 次 4.9 s 缺口；前台服务保活未完全阻止。
3. **SK 14 min 浸泡**：NAS 108 underruns（G2 ≤1/5min/流 超标）、24 trims、0 lost、缓冲 25/35 ms；手机 0 缺口。underrun 计数口径需确认（是否含极小欠载）。
4. **C1 干净基线**：4 min 内 10 underruns + 18 trims（G2 同样超标），与“干净窗口 0 修剪”的历史基线不一致，需核对计数器语义。

## 已验证（可采信）

- **W6 F5 完整周期 PASS**：degrade 5.7 s / **restore 55.3 s**（修复前 0.3 s；30 s 健康窗 + 检测时间）→ #87 退避单位修复生效。
- **B2 Wi-Fi 硬断 ×4 PASS**：恢复后第一次采样入向即回到 wifi。
- **B9 hub BT 阻断 60 s PASS**：窗口内 dial 0。
- **W1/W2/W3/W4/W5**：0–1 underruns、0 trims；W2 lost 216,240（≈注入 2%）；0.3 s 黑洞 0 切换。
- **F1/F2/F4/F5/F11 PASS**：hub 杀进程重启、手机重进、换租约跟 IP、PipeWire 重启后 play_sink 重建、mDNS 屏蔽 90 s 后 gossip 自愈（peers 1/1/1）。
- `ka_rt / ka_dead` 全程为 0：本轮没有触发 BT 链路判死场景（#38 仍待验收）。

## 日志清单与缺口

- `cases.csv`、`SUMMARY.md`、`SUMMARY.stats.json`、`samples/*.jsonl`（每用例 1 s 快照）
- `phone_window_counts.csv`：按用例时间窗重算的手机事件
- `W10/`、`B7/`、`F8/`：额外保存的 PC / NAS 日志切片（W10 的 PC 切片见 `pc.slice.log`）
- 缺口：NAS `/tmp/siren-web.log` 被 F1/F5 的 daemon 重启覆盖，W10/B7 的 NAS 日志已丢；多数用例未转储手机日志（仅 12 个用例有 `phone.logcat.txt`）；未录 WAV，逐音离线分析无输入。

## 下一轮前的脚本修复

1. `netem()` 检查返回码/未知档位，注入失败直接 FAIL（C6 的教训）。
2. C7 改为用独立 `siren mesh` 节点（`--source loopback --no-play`）并读该节点 stats，或改测 PC hub 快照。
3. 手机日志按用例切片：用例开始 `adb logcat -c` 或用字节偏移增量读取，禁止整缓冲累计。
4. 每个用例结束都保存 NAS/PC 日志切片；daemon 重启前先归档 NAS 日志。
5. B7 必须断言"NAS 真的不在 Wi-Fi 上"（HTTP 不可达 / path=bt / hub 判定 dest_bt），断网期间改由 NAS 本地 detached 脚本记录混音到达与水位。
