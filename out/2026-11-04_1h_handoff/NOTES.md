# SirenStudio 数据采集记录（1 小时版实跑）

2026-11-04 17:54–19:09（UTC+8），commit `4083861`，三端同一构建。

## 1. 条件

| 项 | 值 |
| :-- | :-- |
| PC 发送端 | Windows，WASAPI 进程回环（排除本应用），`--no-play`，播放 `pulse.wav`（440 Hz 测试音 + 每 20 s 一个 10 ms 1 kHz 脉冲） |
| Linux hub | 192.168.2.196，Debian 6.18，`play_sink` 输出 + `parecord` 录 monitor，目标水位 100 ms |
| 手机 | M2012K11G / Android 14 / serial 6471a65，系统内录 |
| 编码 | Wi-Fi raw f32 48 kHz 立体声（≈3.1 Mbit/s），蓝牙 Opus 24 kbps |
| 帧长 | 5 ms |
| 阶段 | T1 12 min · T3 15 轮注入 · T4_A 4 min · T4_B 4 min · T6_N2 4 min · T6_N4 4 min |
| 注入 | T3/T6_N4 用 `netem.sh apply raw`，只塑形 UDP 47900 双向；T6_N2 用旧脚本塑形整块网卡 |

四张结论表在 `SUMMARY.md`（脚本 `summarise_1h.py` 生成，可重跑）。

## 2. 结论

**① 静默按位置归类 —— 全部发生在“混音之后”，采集侧一次都没有。**

| 阶段 | hub 混音后（稳态） | PC 采集侧零值游程 |
| :-- | :-- | :-- |
| T1 | 37 ms / 12 min（折算 174 ms/小时） | 37 次，P50 35 ms，单次最长 8.3 s（起播前） |
| T4_A | 5 ms | 4 次，P50 108 ms，最长 8.1 s（起播前） |
| T4_B | 64 ms | 9 次，P50 25 ms，最长 8.1 s（起播前） |
| T6_N2 | 783 ms（P50 5 ms 的小洞） | 8 次，P50 106 ms，最长 8.1 s（起播前） |
| T3 | 13.4 s（注入造成） | 不可用，见 §4 |
| T6_N4 | 209.6 s（码率上限造成） | 无数据 |

PC 侧那几次 8.0–8.3 s 的长零值游程都出现在 SoundPlayer 真正起播之前：**发送端在播放开始前就持续送全零帧**，把接收端缓冲填满的是静音。这不是运行时中断，但它解释了每次阶段开头的“长时间无声”。

**② 修剪是突发，不是缓慢漂移。**

T1 hub 视角 14 次、PC 视角 15 次，全部落在手机流（`8426333216690285339`），路径都是 `wifi`：

- `fill_before` 245–310 ms（目标 100），单次丢 145–210 ms；
- `ratio` 1.00020–1.00064（200–640 ppm），即手机采集时钟比 hub 输出时钟快；
- 间隔：簇内 0.46–1.0 s 连发 3–4 次，簇间 20–156 s —— 水位一旦越过修剪线就会连打，然后要几十秒才再次越线。

**③ 切换各段耗时（T3，15 轮注入，hub 侧量测）。**

| 场景 | 注入 | 音频洞 |
| :-- | :-- | :-- |
| S1 手机 Wi-Fi 关/开 5 s | 5 轮 | 21.0 s（首轮含重新入网）、6.7、6.7、6.7 s |
| S2 30 % 丢包 5 s | 5 轮 | 0.56、1.63、0.55、0.47、0.47 s |
| S3 100 % 丢弃（黑洞）3 s | 5 轮 | 2.98、2.08、3.34 s |

接收端三段（`switches.csv`）：无包 P50 556 ms / P95 2975 ms，合成静音 P50 550 ms，重新填充 P50 5 ms，总静音 P50 560 ms / P95 2982 ms。

**本轮所有切换都是 `wifi → wifi`，没有一次切到蓝牙**（原因见 §3）。`switches.csv` 里的 18 条是两路流各记一次，实际 9 次重选。所以这些数字是“Wi-Fi 恢复时间”，不是“载波切换时间”。

**④ 网络质量与水位的关系。**

| 阶段 | 水位 P50/P95 | 丢帧 | 稳态静音 | RTT P50/P95 |
| :-- | :-- | :-- | :-- | :-- |
| T1 无损伤 | 102/156 ms | 160 k 帧（手机流） | 37 ms | 7/10 ms |
| T6_N2 丢 5 % | 98/103 ms | 1.16 M 帧 | 783 ms | 5/5 ms |
| T6_N4 丢 3 % + 40 ms 延迟 + 2 Mbit | 0/102 ms（塌底） | 0.90–0.93 M 帧 | 209.6 s | 5/5 ms |

- 5 % 独立丢包能被 100 ms 水位吸收，只在混音输出上留下 5 ms 级的小洞；
- `rate 2mbit` 低于 raw f32 需要的 ≈3.1 Mbit/s，链路必然崩塌：水位半小时内反复塌到 0，5 分钟里 87 % 的时间无声。这档若要“可用”，需换 Opus 或把上限提到 3.5 Mbit 以上。

**⑤ 端到端延迟**：绝对量没能测出来（见 §3）。扣除时钟漂移后的相对抖动：T1 P50 −11 / P95 104 ms，T4_A −12/72，T4_B 1/16，T6_N2 1/88；T3、T6_N4 因中断“间隔不稳”。两台机器时钟差 600–2000 ppm 的漂移已在线性拟合里扣掉。

## 3. 与计划不符、本轮没测到的东西

1. **T3 没有载波切换可测**：hub 启动命令里没有 `--bt`，蓝牙链路未建立（pc.log 里 `bt None`，hub 统计 `bt.enabled=false`）。要做“Wi-Fi→蓝牙”切换，下一轮需 hub 带 `--bt` 且三端已配对。
2. **T4_B 的 `SIREN_ECHO_CANCEL=0` 在 Windows 上无效**：该变量只被 PulseAudio 与 Android 采集读取，WASAPI 回环的 AEC 没有开关。A/B 两组实际是同一配置，只能当重复性对照（稳态静音 5 ms vs 64 ms，都不集中在采集后）。
3. **绝对端到端延迟缺一个可信的发出时刻**：`pc_started` 打在 PowerShell `PlaySync` 之前，288 MB wav 的装载时间混进了绝对量；发送端没有自己记录起播时刻。
4. **采集后静默位置只有发送端有数据**：hub 是播放节点，没有采集路径，所以 hub 侧 `zeroruns.csv` 只有 `point=mix`。
5. 按计划允许跳过：S4（延迟抖动）、S5（防火墙阻断）、T2 对照组、Windows/手机的 T4。
6. 未加插桩：发送端 DSP 状态快照、采集环补零计数、解码器重置原因。

## 4. 数据质量问题（分析前必读）

- **NAS 时钟比 PC 快 49.8 s，并在运行期间漂到 55 s（≈0.5 %）**。`timeline.csv` 的 `ts_ms` 是 PC 钟，`hub.log`/各 CSV 的时间戳来自 NAS 钟，两者不能直接相减。跨机对齐只用相对时间；`T3/cycles.csv` 里的注入时刻是 PC 钟。
- **T3 与 T6_N4 的 PC 侧文件不可用**：18:43 那次崩溃留下的 `siren.exe` 一直占着 UDP 47900，第三轮的 PC 发送端 bind 失败（`os error 10048`）。结果 T3/pc.* 是“残留进程 + 失败进程”混写的（文件已损坏），T6_N4/pc.stats.jsonl 是 18:28 第一轮的旧文件。hub 侧录音与 trace 不受影响，本记录的 T3/T6_N4 结论全部来自 hub 侧。
- **T6_N2 与 T3/T6_N4 的注入点不同**：前者塑形整块网卡（含 SSH），后者只塑形 UDP 47900。T6_N2 的数值不能和 T3/T6_N4 直接横比。
- **启动段**：hub 每阶段重启，录音起点比引擎晚约 10 s；`SUMMARY.md` 里的“启动段”按最后一路流入网后 10 s 划分。T4_A 那个 16.8 s 的静音全部属于启动段（PC 送全零帧），稳态只有 5 ms。
- 手机 CPU 采样按线程求和，T6_N2/N6_N4 各有一个 5321 %/4621 % 的解析异常块，当噪声忽略；其余 P50 46–56 %、P95 59–67 %。

## 5. 文件清单

```
T1/ T3/ T4_A/ T4_B/ T6_N2/ T6_N4/
  gaps.csv trims.csv switches.csv zeroruns.csv mutes.csv latency.csv   ← hub 侧，首列 ts_ms
  trims.pc.csv zeroruns.pc.csv switches.pc.csv mutes.pc.csv            ← 发送端侧（T3/T6_N4 无）
  cycles.csv                                                            ← 仅 T3，最后一轮注入序列
  logs/hub.anomalies.log      只留 WARN/ERROR、trace、切换、入网、超时、underrun
  logs/pc.anomalies.log       同上；T3/T6_N4 换成 pc.error.txt
  logs/phone.anomalies.txt    logcat 的 E 行 + 网络/音频相关 W 行（超 6000 行截断并注明）
  logs/phone.cpu.csv          每块采样的应用总 CPU 与最热线程
meta.txt timeline.csv SUMMARY.md NOTES.md
```

原始 WAV 未打包（约 500 MB），需要逐帧复查时用 `out/2026-11-04_1h/<阶段>/recording.wav`，配合 `scripts/analyze_gaps.py`。

## 6. 下一轮建议（未执行）

1. 补 T3 的载波切换：hub 加 `--bt`，先确认三端配对，再跑 S1–S3。
2. 把 A/B 换到 Android 发送端（`SIREN_ANDROID_ECHO` 有开关），或给 WASAPI 回环加开关。
3. 绝对延迟：让发送端在真正起播后自己写一行 epoch（别用 PowerShell 装载前的标记）。
4. T6_N4 若想看“有损但有声”，把上限提到 3.5 Mbit 或改 Opus。
5. NAS 上先做 NTP 对时（当前偏移 50 s 且持续漂移，跨机时间分析都会被它坑）。
