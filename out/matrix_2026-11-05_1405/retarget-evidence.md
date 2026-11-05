# 复验补充（pc.slice.log 不在版本库内：.gitignore 排除了 *.log）

发送端 retarget 次数（`grep -c 'retargeted' <场景>/pc.slice.log`），同一台 PC、同一套三设备：

| 场景 | 1338 批（地址一变就跟） | 1405 批（心跳无应答才改绑） |
| :-- | --: | --: |
| S1_pc_phone_dual | 1 | 1 |
| S2_pc_phone_wifi_off | 0 | 0 |
| S3_tri_dual | 39 | 0 |
| S4_tri_phone_wifi_off | 46 | 0 |
| S5_tri_phone_nas_wifi_off | 4 | 1 |

1338 的抖动全部是同一个键 `836c47c621873a1e`（NAS 的 node id）在 192.168.2.196 与 192.168.2.159
之间来回翻。接口归属后来量清楚了（`ip route get 192.168.2.107` → `dev enp1s0 src 192.168.2.196`，
`ip addr` → enp1s0=**.196** metric 100、wlp3s0=**.159** metric 600）：
**.196 是网线口 enp1s0，.159 才是 Wi-Fi**。也就是说 mesh 流量一直走以太网，
mDNS 在同一子网的两个接口上轮流返回 A 记录，而"断 NAS Wi-Fi"只切掉了次要接口，
并没有把 NAS 与 LAN 隔离（这直接改了矩阵 S5 的语义，见下）。日志里没有任何
`peer ... moved (from received audio)` 行，说明抖动不来自 E4 的收包回填，而来自发现层。

1405 剩下的 2 次是真故障跟随，且落点为 Wi-Fi 地址：

```
mesh: 836c47c621873a1e retargeted to 192.168.2.196:47900 (downlink unanswered, hb loss 41 %)
```

同批 NAS 侧收益：S3 里 PC→NAS 那路的丢帧增量 15120 帧（≈315 ms 音频）→ 1440 帧（≈30 ms），
NAS 末水位由 28/55 ms 回到 30/33 ms。
