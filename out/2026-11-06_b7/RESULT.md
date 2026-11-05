# B7 复测：纯 BT 叶子的 N−1 混音自激定位（2026-11-06 01:28–01:54）

## 场景（最保守：零 netem，仅 iptables 黑洞）
- 拓扑：NAS Linux-ISO = hub（tone 440），手机 6471a65（.248，tone）= leaf。
- 基线 3 min：双栈直连流（wifi），手机侧 0 修剪/0 欠载/0 静音。
- 切换：NAS `iptables` INPUT+OUTPUT 掐 UDP 47900（仅 .248↔.196）→ 手机变纯 BT 叶子。
  - 01:28:33 手机直连流按 F8 冻结→30 s 宽限逐出（符合设计）。
  - hub 侧 N−1 混音（0xFFF0，Opus 10 ms，target 250 ms）经 BT 隧道下发。
- 测量窗 4 min（01:49–01:53）：手机侧逐包计数 + trim/zerorun/gap 全量解析。

## 结果
| 指标 | 上轮矩阵（11-05，叶子侧 3 min） | 本轮（手机叶子 4 min） |
|---|---|---|
| >100 ms 混音静音 | 37–41 次 | **0** |
| 修剪 | 169–226 次 | **0** |
| 欠载 | 反复 | **0** |
| 丢包 | — | 0%（36542 包） |
| 水位 | 403–438 ms 仍欠载 | 224–244 / 250 ms 稳定 |

**自激未复现。** 与 MIX1 sim 验收（tests/mix1_b7.rs，无损链路干净）结论一致。

## 口径说明（为什么不能直接宣布 #64 关闭）
- 上轮自激观察对象是 **NAS 作叶子 + target 30 ms 窄水位**；本轮叶子是手机（live target 250 ms，水位宽裕）。窄水位 + BT 抖动的组合未在本轮复测（对调角色需要 NAS 断 Wi-Fi 类操作，用户已要求暂停该类测试）。
- 因此 #64 状态：🟨→**缩窄**——手机叶子（250 ms）场景干净；NAS 叶子（30 ms）场景保留待测。
- 本轮另发现：**hub relay 曾被卡死**（01:28–01:46 期间 tap 已有合法 BT 目的地但 tx 恒 0，NAS 守护重启后立即恢复）——疑似 W1 SIGSTOP/链路 churn 残留态。已在 relay 路径加 60 s 限频诊断日志（`relay dbg`），复发时可从日志直读各门状态；该日志同时正确捕捉了回切（解锁后 bt_flags=[false]→停发）。

## 证据
- `baseline.logcat.txt`：3 min 双栈基线（0 事件）。
- `btonly.logcat.txt`：4 min 纯 BT 混音窗（0 事件）。
- `nas_relay_dbg.txt`：relay 诊断行（含回切 bt=false 样本）。
- `analyze.py`：解析口径（cumulative 计数 + trim/zerorun/gap 事件）。
