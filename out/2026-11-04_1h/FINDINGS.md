# 日志异常点与可能的问题

证据来源：`out/2026-11-04_1h_handoff/*`（裁剪后的异常日志 + CSV）与 `out/2026-11-04_1h/*`（原始日志、录音）。
按严重度排序，每条给出现象、量级、可能原因、以及能证伪它的下一步实验。

---

## A1 手机侧 Wi-Fi 双向 1.1 s 级停顿，随后一次性涌入并触发 750 ms 级修剪

现象（手机 logcat，UTC，全部来自同一台机器的时钟）：

```
17:57:55.007  W siren: mesh: send to 192.168.2.107:47900 failed: udp send: Try again (os error 11)  ← 上行阻塞
17:57:55.011  W siren: mesh: send to 192.168.2.196:47900 failed: udp send: Try again (os error 11)
17:57:55.963  I siren: udp: stream ... audible gap 1167 ms before this wifi packet (previous carrier wifi)
17:57:55.986  I siren: trace: zerorun point=mix run_ms=1075.7 streams=2
17:57:56.510  I siren: trace: trim stream=6410979546482776522 fill_before_ms=846.6 dropped_ms=746.6 ratio=1.00200 path=wifi ms_since_last_trim=43
17:57:56.547  I siren: trace: trim ... fill_before_ms=353.8 dropped_ms=253.8
17:57:56.590  I siren: trace: trim ... fill_before_ms=306.0 dropped_ms=206.0
17:58:04.031  W siren: mesh: send ... Try again (os error 11)     ← 同一模式再发生一次
17:58:04.525  audible gap 1083 ms → zerorun 961.0 → trim 594.3 → trim 500.0
```

量级（T1，12 min，手机侧日志）：58 次修剪，26 次 `dropped>200 ms`，最大 `fill_before=851.6 / dropped=751.6 ms`；混音零值游程 28 次（20 次 `>100 ms`，合计 6.78 s）；`audible gap>300 ms` 4 条（2 个事件，因两路流各记一次）。

**发送与接收同时中断**：`udp send: Try again (EAGAIN)` 对 `.107` 和 `.196` 两个目的地同一毫秒失败，说明不是某条路径的问题，而是**手机网卡的发送队列被堵住**；同一时刻它对两路下行也报了 1.1 s 的 audible gap。收发同时停 = 射频层停摆，不是丢包。

频次不对称：同样的量在 T4_A / T4_B / T6_N2（各 4 min）里都是 **0** 次；T6_N4（施加损伤）12 次。所以这不是"一直存在"的常驻缺陷，更像是特定时刻的射频事件。

可能原因（尚未定论，按可能性排序）：
1. 射频层被别的业务占用（MIUI 周期扫描 / 蓝牙 `page` + L2CAP 建立）。但 T1 的整段日志里 `cancelPeriodWifiScan` 只出现 1 次（17:57:53.389，距第一个洞 2.6 s），**不足以解释 2 个事件**；蓝牙拨号每相次都在发生（各阶段都 ~10 次/分钟），而 4 分钟那几阶段却 0 洞 —— **共存假设被这个不对称削弱**。
2. Wi-Fi 省电（u-APSD / DTIM）唤醒失败，`EAGAIN` 与两路同时中断符合"网卡不在信道上"。
3. 驱动/固件层的短暂 hang（高通 `ibs_handler`/wake 相关日志在同一窗口出现）。

排除项：不是应用层丢包、不是单路径故障、不是缓冲策略本身造成的（缓冲策略只是把后果放大成 750 ms 的修剪）。

下一步（能一锤定音）：手机开飞行模式只留 Wi-Fi、关蓝牙跑 12 分钟，看是否仍有 EAGAIN + 1.1 s 洞；若消失 → 共存/蓝牙；若仍在 → 省电或固件。

---

## A2 手机蓝牙拨号风暴：12 分钟 470 次，全部失败

```
470  I siren: bt: dialing E0:8F:4C:16:9F:58 channel 16
470  W siren: bt: channel connect failed (read failed, socket might closed or timeout, read ret: -1), trying SPP
470  I siren: bt: dial failed
118  I siren: hybrid: offline, dialing remembered hub ADMINISTRATOR (mesh)
3564 logcat 行 bt_l2cap: ... LOWER_LAYER_DISCONNECT_IND
```

计数校验：`grep -c "bt: dialing " T1/phone.logcat.txt` = 470，`grep -c "bt: dial failed"` 同样 470。
两个节拍：`hybrid: offline` 是严格的 6.00 s 定时器（118 次 / 720 s）；每次定时器内部连打约 4 次拨号，所以拨号间隔 P50 ≈ 1.1–1.6 s（范围 0.14–5.84 s）。**不是"每 6 s 一次拨号"，而是"每 6 s 一轮、每轮约 4 次"**。

每次尝试的实际过程（17:57:45 窗口）：`RFCOMM_CreateConnection` → `L2CAP channel is opened: handle=19, lcid=65` → 立即 `RFCOMM connection closed ... reason=Closed[19]` → 应用判失败并回落 SPP。伴随 `vendor.qti.bluetooth wake_lock Acquire` 与 UART 时钟投票，即每次都在唤醒 BT 子系统。对端 Linux hub 的蓝牙是关的，所以永远失败。

可能问题：
- **没有退避**：重连应当指数退避；现在无限定频重试。
- 每次失败都唤起 BT 芯片与 UART（耗电、发热、射频共存）。
- 副作用之一：`notifyPowerKeeper: SecurityException: Need BLUETOOTH_ADMIN permission`（17:57:45.181），说明还有一条权限缺失的调用路径在异常分支里反复执行。

---

## A3 手机侧控制器长期贴 +0.2% 限幅；hub/PC 侧远未到限

`ratio` 是 PI 控制器的输出（`1.0 + correction`，`correction` 被 `max_correction = 0.002` 夹住，[resampler.rs:81](file:///E:/SirenStudio/crates/audio-core/src/dsp/resampler.rs#L81)、[:212-214](file:///E:/SirenStudio/crates/audio-core/src/dsp/resampler.rs#L212-L214)），不是"实测到达速率"。三端实测：

| 端 | `ratio` 范围 | 说明 |
| :-- | :-- | :-- |
| hub（对手机流） | 1.00020–1.00064 | 200–640 ppm |
| PC（对手机流） | 1.00039–1.00069 | 390–690 ppm |
| 手机（对 hub 流） | **1.00200（多次等于上限）**、1.00145、1.00179 | 打满 +0.2% |

手机侧 `fill_before` 最高 851.6 ms（目标 100），控制器想以 +0.2% 加快消耗；但按 0.2% 排掉 746 ms 需要 **373 s**，远慢于堆积速度，所以只能靠修剪一次性清空。这是设计层面的结论：**±0.2% 的速率修正吸收不了 1 s 级停顿，修剪路径是唯一出路**。

补充：手机自己接收缓冲的常态在 71–88 ms（目标 100），本来就偏饥饿，任何额外抖动都会先欠载再补积。

---

## A4 修剪一次扔几百 ms；手机侧 80 ms 内连发 3 次

天花板 `2×target+10 = 210 ms`（[resampler.rs:27-29](file:///E:/SirenStudio/crates/audio-core/src/dsp/resampler.rs#L27-L29)）只在每个渲染帧检查一次，而环形缓冲可容纳 1 s，突发一次性灌入就会远超天花板。两个视角的数字不同，别混用：

- **hub 侧**（`T1/trims.csv`）：14 次，`fill_before` 245–310 ms，`dropped` 145–210 ms，间隔呈簇（0.46–1.0 s 内连发 3–4 次，簇间 20–156 s）。
- **手机侧**（logcat）：58 次，最大 `fill_before=851.6 ms / dropped=751.6 ms`，17:57:56.510 / .547 / .590 三次落在 **80 ms 内**。

可能问题：修剪是"一步跳到目标"而不是限速分次削，用户听到的是瞬间跳过 0.75 s；连发说明削完立刻又超标（因为同一批突发还没放完）。

---

## A5 发送端在真正出声前持续发送全零音频（8.0–8.3 s）

PC 侧 `trace: zerorun point=capture` 每阶段一次 8.0–8.3 s 的长游程；hub 侧对应看到 PC 流的第一包就是全零：

```
T1/hub.log:38     stream 5074374508783123908 first packet: RawF32 1920 B peak 0.0000 [00 00 00 …]
T4_A/hub.log:29   stream 6396053934851429632 first packet: RawF32 1920 B peak 0.0000 [00 00 00 …]
```

T4_A 因此产生 16.8 s 的"混音后无声"（按最后一路入网 +10 s 划线，这 16.8 s 全属于启动段，稳态只有 5 ms）。

可能问题：**满水位但内容是数字静音**，所有健康指标（buffer 100 ms、underrun 0、lost 0）都显示正常，收听者却什么都听不见。应在采集侧检测"持续全零"并 either 不发包/降级为 keepalive，either 在 UI 明示"源静音"。

---

## A6 每次阶段开头，hub→手机发包被拒 12–16 次，约 20 s

```
09:55:43.407 WARN mesh: send to 192.168.2.124:47900 failed: udp send: Connection refused (os error 111)
...（每 1–2 s 一次，到 09:56:02 停止；之后 10:03:40 / 10:04:11 / 10:07:24 各一次）
```

ICMP port unreachable = 那个地址上没有人 listen。手机这时正在以另一个源端口往 hub 发流，说明**发现记录与实际监听端点不一致**（或手机节点重启了监听而 hub 仍用旧端点）。手机 IP 还会变（T1 是 `.124`，T4/T6 是 `.248`）。

后果：手机自己的收听在开头 ~20 s 是断的。这个故障在本轮数据里**完全不可见**——我们只录了 hub 的 monitor。

可能问题：端点更新滞后 / 没有用收包源的 `(ip,port)` 直接回打（对称路由），以及 stale peer 地址没有及时失效。

---

## A7 手机在同一次会话里换 stream id 重新入网（T1、T6_N4 各 2 个 id）

```
09:55:31.312 stream 1589260020601853262 from 192.168.2.124:45491 (raw, origin 72a176...) joined
09:56:05.428 stream 8426333216690285339 from 192.168.2.124:37535 (raw, origin 72a176...) joined
```

同一 origin、新流、新源端口。hub/PC 两条流都会各自建缓冲并参与混音；如果旧流走的是 F8 冻结保留而不是立即结束，就会**同一手机出现两路混音**（电平、延迟都对不上）。

下一步：查这两条流共存期间 hub 的 `active_streams` 与混音电平（本轮 T1 全程 active=2，说明旧流已被移除，但需要确认移除耗时）。

---

## A8 主输出电平 6.5–10 s 周期缓慢泵动（−10 → −26 dB）

T1 的 `master.rms_db` std 4.86 dB、范围 −26.4…−10.0，去趋势后主周期 6.5–10 s。原因是 hub（`--source tone --freq 440`）和 PC（`pulse.wav` 载波也是 440 Hz）在放**同一个频率**，两路之间的相对延迟随 200–340 ppm 的时钟差漂移 → 梳状干涉。

- 测试设计问题：两端测试音必须用不同频率（例如 440 / 660），否则混音层看到的永远是干涉。
- 但它也暴露真实风险：**同频内容（音乐、语音里的稳态元音）多路混音时会做同样的音量起伏**，这不是软件 bug 而是 mesh 求和的固有性质，需要产品层解释（或做延迟对齐）。

---

## A9 T3 的恢复时间全部由 Wi-Fi 重连主导，最坏 21 s

| 场景 | 音频洞 |
| :-- | :-- |
| S1 关/开 Wi-Fi 5 s | 21.0 / 6.7 / 6.7 / 6.7 s |
| S2 30% 丢包 5 s | 0.47–1.63 s |
| S3 黑洞 3 s | 2.08–3.34 s |

S1 的 6.7 s 远大于"关 5 s"本身，说明**关掉再打开之后还要 2–7 s 才恢复出声**；首轮 21 s。可能问题：重新关联/DHCP 后 mDNS 重新发现慢、发送 socket 未复用（#68）、以及"先攒够水位再出声"的策略在重连时被完整走一遍。用户感知就是"Wi-Fi 抖一下，7 秒没声"。

另外本轮**没有任何一次 Wi-Fi→蓝牙切换**（hub `bt.enabled=false`），所以双栈备份路径没被检验过——这是最大的一块空白。

---

## A10 T6 注入档位与实测丢包不一致，N4 直接崩

**`lost_frames` 的单位是"帧"，已从代码确认**：`let lost = gap.saturating_mul(frame_frames)`，随后 `lost_frames.fetch_add(lost)`（[udp.rs:1166-1206](file:///E:/SirenStudio/crates/audio-core/src/net/udp.rs#L1166-L1206)）。一包 = 240 帧（5 ms @ 48 kHz）。所以不能拿 `lost_frames` 和"包数"直接比大小。

T6_N2 实测（`hub.stats.jsonl` 最后一帧，[streams.csv](file:///E:/SirenStudio/out/2026-11-04_1h_handoff/T6_N2/streams.csv) 可复核）：

| 流 | packets | lost_frames | 丢包率 = lost / (packets×240 + lost) |
| :-- | --: | --: | --: |
| `72a176cebfe88db4#48061` | 49 723 | 1 150 560 | 8.8 % |
| `687ad46a475a5f7c#5332` | 54 207 | 1 159 440 | 8.2 % |

配置是 `loss 5%`，实测 8–9 %。多出来的部分合理怀疑是叠加了手机侧自身的共存损耗（见 A1），也可能是 egress + ifb 镜像 ingress 对同一路流量各丢一次；需要用"只连 PC↔hub"的最小场景复测校准。

另外 `seq gap` 日志行只在 `gap > 50` 包（>250 ms）且 2 s 限速时打印（[udp.rs:1149](file:///E:/SirenStudio/crates/audio-core/src/net/udp.rs#L1149)），所以 T6_N2 一条都没有——**只看日志会误判成"没丢包"**，只有计数器可信。

T6_N4 的 `rate 2mbit` 低于 raw f32 需要的 ≈3.1 Mbit/s：rx 报文数从 10.5 万掉到 3.5 万，水位塌到 0，5 分钟里 209.6 s 无声。当前不会在带宽不足时自动降级到 Opus，属于**能力缺失**而不是抖动。

---

## A11 三台机器没有可靠对时，跨机时间量测全部带 ±1 s～50 s 不确定度

NAS 时钟比 PC 快 49.8 s，且在这一小时里继续漂到 55 s（≈0.5%）；PC↔hub 的音频时钟差 600–2000 ppm。影响：
- `timeline.csv`（PC 钟）与 hub/手机日志（各自钟）不能相减；切换/恢复的分段耗时只能用同一台机器内部的时间戳。
- 绝对端到端延迟本轮没测出来（`pc_started` 打在 PowerShell 装载 288 MB wav 之前）。

产品侧注意：**心跳 RTT 是相对量、不受影响**（本轮 T1 7/10 ms，其余 5/5 ms），但任何"跨端事件对齐"的指标（例如未来的多端同步播放）都需要先做对时。

---

## A12 观测盲区

1. 只录了 hub 的 monitor；**手机和 PC 的输出音频没有被记录**，A6/A7/A9 造成的收听端故障在数据里看不见。
2. `master.silent_ms/silent_gaps` 只在 hub 侧有；手机/PC 没有对应指标。
3. 采集侧零值游程只在发送端日志里（`*.pc.csv` / 手机 logcat），hub 是播放节点没有采集路径，所以 hub 的 `zeroruns.csv` 只有 `point=mix`。
4. T3/T6_N4 的 PC 侧文件因残留进程占端口（`os error 10048`）而损坏/缺失。
