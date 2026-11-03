# Phase 1 基线对照 — F1(s16)/F2(斜坡)/F4(限幅)

- 时间：2026-11-04；对照对象：Phase 0 基线（docs/baselines/2026-11-04）
- F1a 抓包实锤：raw f32 5 ms 帧 1948 B > MTU，线上 flags[+] = 2 个 IP 分片/包
- F1 (s16)：loss2 实测 4.09% → **1.96%**（=netem 配置），burst 3.29% → 1.83%；
  delay200 不变（重排主导，非分片）
- F2/F4：clean 与 loss2 的咔哒计数全部归零（修复前 clean 亦有零星咔哒）
- 心跳 RTT P99：5–15 ms（clean）
