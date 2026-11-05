# F8/F12 Doze 取证：熄屏 + 强制 Doze 90 s（2026-11-06 01:15–01:17）

## 场景
- 手机 M2012K11G（6471a65，SirenStudio leaf，测试音源）↔ NAS hub，Wi-Fi 双栈在网。
- `input keyevent 26`（熄屏）→ `deviceidle enable` → `deviceidle force-idle`，确认 `mState=IDLE`，驻留 90 s。
- 期间 app 进程存活（pid 27436 不变，F12 mediaPlayback 前台服务）。
- NAS 侧 1 s 采样 180 个快照（`nas_samples.jsonl`，覆盖 Doze 前中后），手机侧 logcat 全程存档。

## 结果
| 指标 | 值 |
|---|---|
| NAS 收手机流中断 | **0**（180 个快照全部 active，包速率 ~230/s 连续） |
| 接收水位 | 25–32 ms（target 30），无塌陷无堆积 |
| `silent_ms` 增量（Doze 全程） | **0.0 s** |
| 冻结/重建（F8 路径） | **未触发**（同一 stream id，无 `resumed from freeze`） |
| 手机侧异常 | Doze 窗口内 0 条（唯一 `zerorun 59.9 s` 在 01:12:17，属 W1 冻结 hub 的预期静音） |

## 结论
1. **F12 前台服务把 Doze 完全挡住**：媒体型前台服务 + 音频活跃使用下，MIUI/Android Doze 不冻结采集上行，流零中断。交接遗留的"F8 Doze 4.9 s"缺口在本配置下**无法复现**（当时应缺 F12 或被 Doze 白名单外配置影响）。
2. F8 冻结路径因此没有 Doze 场景的证据义务：它只在真正断流时出场（W1/E3 的 hub 冻结窗口已从另一侧验证过静音→恢复行为）。
3. 复测建议：长时（≥1 h）Doze + 省电白名单全关的场景留待 W4/E5 三组对照一并做（用户已暂缓）。

## 判定
F8 Doze 证据项：**PASS（无缺口）**——不需要 F8 动作即无缺口；F12 是有效的 Doze 防线。

## 证据
- `nas_samples.jsonl`：180 快照逐秒记录（silent/streams/buffer/active）。
- `phone.logcat.txt`：451 行，Doze 窗口无异常。
