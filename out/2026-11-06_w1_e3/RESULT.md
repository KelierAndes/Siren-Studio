# W1 真机验收：半开 RFCOMM 判死与自愈（2026-11-06 01:11–01:12）

## 场景
- 三端：手机 M2012K11G（6471a65，leaf）↔ NAS Linux-ISO（192.168.2.196，BT hub，RFCOMM ch16），Wi-Fi 双栈同开。
- 半开手法：`kill -STOP <siren pid>`（427757）——ACL 保持、hub 停止读写、内核缓冲假活，与 E3 实测"tx 7.8 KB/s 假活"字节级同构；SIGCONT 完全可逆。
- 版本：NAS 与手机 APK 均为本日构建（898b9a4 proto 广播 + 6b49876 pong 锁存）。

## 结果
| 指标 | 修复前（00:41 首跑） | 修复后（01:11 本跑） |
|---|---|---|
| keepalive 协商 | on（proto 广播修复后首次成功） | on（01:08:18，拓扑选举路径） |
| SIGSTOP 后 keepalive | **2 s 后自关**（hub 掉出 mDNS → 拓扑清空） | **全程保持 on**（pong 锁存） |
| 判死时延 | 永不判死（tx 7.8 KB/s 骗过字节窗，45 s+ 死链路） | **3.7 s**（01:11:18 冻结 → 01:11:21.7 `keepalive unanswered for 3000 ms, closed (half-open RFCOMM)`） |
| 重拨自愈 | 无 | link 2（01:11:23）→判死（01:11:28）→ link 3 →判死（01:11:34）→ 持续循环 |
| 恢复 | — | SIGCONT（01:12:18）后 1.8 s link 12 重开并稳定保持，Wi-Fi 流 0 丢 0 欠载 |

## 本轮发现并修复的真缺陷
1. **898b9a4**：mDNS TXT 从未发布 `proto`（浏览侧一直读、广告侧从不发）→ 真机上 keepalive 永不协商，W1 保护是死代码。真机测试直接暴露。
2. **6b49876**：hub 掉出 mDNS（拓扑清空）时 keepalive 协商自动关闭——恰在最需要它的时刻。修复：pong 确认后按 hub uid 锁存，hub 更换或本端转 hub 才重置。

## 判定
W1 真机验收（原判据"E3 场景下手机 6 s 内判死并按退避重拨"）：**PASS**（3.7 s）。sim 验收（tests/w1_halfopen.rs）+ 真机验收双通道关闭；PLAN W1 置 ✅。

## 证据
- `phone.logcat.txt`：10 次 `keepalive unanswered ... closed`；01:02:16/01:08:18 两次 `tunnel keepalive on`。
- `nas.log.txt`：hub keepalive on/accepted 链路序列。
