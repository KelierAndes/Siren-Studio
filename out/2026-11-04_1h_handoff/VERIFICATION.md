# FINDINGS 复核（逐条对照原始日志）

复核对象：另一份审查意见（下称"审查"）。原则：所有数字都能用给出的命令在 `out/2026-11-04_1h/` 原始文件上重算；
跨机器的时间戳只做量级对照，不做亚秒级因果推断。

## 0. 审查看不到证据的原因（我的打包缺陷，已修）

第一版交接包按"只保留异常"裁剪日志，规则是 **E 级全留、W 级只留网络/音频关键词**。但：

- 手机的运行事实（`trace: trim`、`trace: zerorun`、`audible gap`、`hybrid:`、`bt:`、`udp:`）全部是 **I 级**；
- hub 的 `first packet: ... peak 0.0000` 也是 INFO，且不含 `joined`/`WARN` 等关键词；
- `hub.stats.jsonl`（A3/A8/A10 的唯一来源）原本就不在包里。

因此 A1、A3、A4、A5、A8 在旧包里确实"找不到依据"。现已修正打包：手机日志增加 `siren: trace|udp|hybrid|bt|mesh|receiver|capture` 的 I 级行，
每阶段新增 `master.csv`（电平/无声/活跃流）与 `streams.csv`（逐秒每流 buffer/lost/underrun/reset/clicks）。

## 1. 逐条裁决

| 项 | 审查意见 | 复核结果 | 证据（行号/命令） |
| :-- | :-- | :-- | :-- |
| A1 | 无法验证；疑似发送端阻塞 | **成立**，且审查的"发送端阻塞"正是原始结论的一部分 | `T1/phone.logcat.txt:7466-7467`（EAGAIN 到 .107/.196）、`:7496-7497`（audible gap 1167/1168 ms）、`:7498`（zerorun 1075.7）、`:7567`（trim 846.6/746.6）；`:8018-8030` 为第二次 |
| A2 | 次数 149、间隔 P50 1.37 s | **次数错、间隔对** | `grep -c "bt: dialing " T1/phone.logcat.txt` → 470；`grep -c "bt: dial failed"` → 470；`hybrid: offline` 118 次、间隔恒 6.00 s（`T1/phone.logcat.txt:3, 409, 813`） |
| A3 | 包里 ratio 从未到 1.002 | **成立但仅限手机侧**；审查查的是 hub/PC 的 CSV | hub 1.00020–1.00064、PC 1.00039–1.00069（`T1/trims.csv`、`T1/trims.pc.csv`）；手机 `ratio=1.00200` 多次（`T1/phone.logcat.txt:7567` 等）。`ratio` 语义见 §2 |
| A4 | 最大 fill 310 / dropped 210；连发在 2.5 s 内 | **两个视角都成立，需标来源** | hub：`T1/trims.csv` max 310.0/210.0；手机：max 851.6/751.6，且 3 次在 80 ms 内（`.510/.547/.590`） |
| A5 | 成立；`peak 0.0000` 未找到 | **成立**；行号见右 | `T1/hub.log:38`、`T4_A/hub.log:29` |
| A6 | 基本成立，根因是推测 | 同意 | 16 次 `Connection refused`，`T1/logs/hub.anomalies.log` |
| A7 | 成立，旧流移除耗时未验证 | 同意 | origin 72a176 两个 stream（相隔 34 s） |
| A8 | 包内无 stats，无法验证 | **成立**；数据一直在 `hub.stats.jsonl`，现已导出 | `T1/master.csv`：`rms_db` std 4.86 dB、周期 6.5–10 s |
| A9 | 成立 | 同意 | `T3/mutes.csv`、`T3/switches.csv` |
| A10 | 单位不明，不宜下结论 | **单位已定**：帧 | `udp.rs:1166-1206`；T6_N2 8.8 % / 8.2 %（`T6_N2/streams.csv` 末帧） |
| A11 | 成立 | 同意 | NOTES §4 |
| A12 | 成立 | 同意 | — |

## 2. 我对上一版的自身修正

1. **A2 的节拍描述错了**：我写"每 6 s 一次拨号"，实际 6.00 s 是 `hybrid: offline` 定时器；每轮内部连打约 4 次，拨号间隔 P50 1.1–1.6 s。次数 470 不变。
2. **A3 的机制描述要收窄**：`ratio` 是控制器输出（`1 + correction`，clamp ±0.002），不是"实测到达速率"。所以 1.00200 = 控制器打满，**因为** fill 远高于目标（合理），不是量测被突发骗了。
   真正的结论是：0.2 % 的排空速度（746 ms 需 373 s）吸收不了 1 s 级停顿，修剪是唯一出路。
3. **A1 的因果要降级**：蓝牙拨号风暴在四个 4 分钟阶段同样存在（~10 次/分钟），而那几阶段 0 次洞，所以"BT 共存导致停顿"被削弱，只能列为待验证假设。
   T1 的整段日志里 `cancelPeriodWifiScan` 仅 1 次（17:57:53.389，距第一个洞 2.6 s），也不足以解释 2 个事件。
   目前能确定的只有现象本身：**上行 EAGAIN + 双向 1.1 s 空洞同时发生**。
4. **接受审查的提醒**：不同机器的亚秒级比对无意义。A1 的因果链已改为全部使用手机自己的时间戳（EAGAIN → gap → zerorun → trim），不再引用 hub 时间。

## 3. 仍然无法验证的

- A1 的射频触发源（需要关蓝牙/关省电的对照实验，或抓空中接口/`dumpsys wifi`）。
- A7 的旧流是否及时移除、移除耗时（需要 `active_streams` 逐秒序列，`streams.csv` 已能看趋势，但旧流在 hub 侧不区分 id）。
- T6_N2 多出来的 3–4 % 丢包来自 netem 双打还是手机侧背景损耗（需要只连 PC↔hub 的最小场景）。

## 4. 复核命令（可直接重跑）

```bash
cd out/2026-11-04_1h
grep -c "bt: dialing " T1/phone.logcat.txt            # 470
grep -n "Try again\|audible gap\|fill_before_ms=846.6" T1/phone.logcat.txt | head
grep -n "peak 0.0000" T1/hub.log T4_A/hub.log
python -c "import csv;r=list(csv.DictReader(open('T1/trims.csv')));print(len(r),max(float(x['fill_before_ms']) for x in r))"
python -c "import csv;i=[int(r['packets']) for r in csv.DictReader(open('T6_N2/streams.csv'))][-2];l=[int(r['lost_frames']) for r in csv.DictReader(open('T6_N2/streams.csv'))][-2];print(l/(i*240+l))"
```
