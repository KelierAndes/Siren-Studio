# F5 降级→恢复长观察（2026-11-05 16:43）

- 周期数：10；故障形式：`raw 'delay 260ms 20ms distribution normal'`，只整 udp/47900 双向（netem.sh 的 audio-only 路径，SSH 不受影响）
- 降级触发：10/10
- 恢复触发：10/10
- 振荡（单周期翻转 >2 次）：0
- 恢复等待实测：min 16.3 s，max 16.4 s（F5 的指数退避本应让等待逐次变长；这里要确认的是它仍会恢复、且不发抖）

| 周期 | 降级 s | 判据行 rtt p99 / hb loss | 恢复 s | 恢复码 | 来源 | 切片内翻转 |
| :-- | --: | :-- | --: | :-- | :-- | --: |
| 1 | 5.7 | 495 ms / 6 % | 16.3 | recovered — restoring raw | nas | 2 |
| 2 | 5.7 | 495 ms / 6 % | 16.3 | recovered — restoring raw | nas | 2 |
| 3 | 5.7 | 495 ms / 6 % | 16.3 | recovered — restoring raw | nas | 2 |
| 4 | 5.7 | 485 ms / 22 % | 16.4 | recovered — restoring raw | nas | 2 |
| 5 | 5.7 | 480 ms / 0 % | 16.4 | recovered — restoring raw | nas | 2 |
| 6 | 5.7 | 500 ms / 22 % | 16.4 | recovered — restoring raw | nas | 2 |
| 7 | 5.7 | 500 ms / 0 % | 16.3 | recovered — restoring raw | nas | 2 |
| 8 | 5.7 | 485 ms / 25 % | 16.3 | recovered — restoring raw | nas | 2 |
| 9 | 5.7 | 500 ms / 9 % | 16.3 | recovered — restoring raw | nas | 2 |
| 10 | 5.7 | 485 ms / 6 % | 16.3 | recovered — restoring raw | nas | 2 |
