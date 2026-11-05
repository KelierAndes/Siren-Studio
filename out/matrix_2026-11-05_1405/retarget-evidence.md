# 复验补充（pc.slice.log 不在版本库内：.gitignore 排除了 *.log）

发送端 retarget 次数（`grep -c 'retargeted' <场景>/pc.slice.log`），同一台 PC、同一套三设备：

| 场景 | 1338 批（地址一变就跟） | 1405 批（心跳无应答才改绑） |
| :-- | --: | --: |
| S1_pc_phone_dual | 1 | 1 |
| S2_pc_phone_wifi_off | 0 | 0 |
| S3_tri_dual | 39 | 0 |
| S4_tri_phone_wifi_off | 46 | 0 |
| S5_tri_phone_nas_wifi_off | 4 | 1 |

1338 的抖动全部是同一个键 `836c47c621873a1e`（NAS 的 node id）在 192.168.2.159（eth）与
192.168.2.196（Wi-Fi）之间来回翻：双网卡机器的 mDNS 轮流返回两个 A 记录，而发现层每次都
覆盖 peer.addr。日志里没有任何 `peer ... moved (from received audio)` 行，证明抖动不来自
E4 的收包回填，而来自发现层。

1405 剩下的 2 次是真故障跟随，且落点为 Wi-Fi 地址：

```
mesh: 836c47c621873a1e retargeted to 192.168.2.196:47900 (downlink unanswered, hb loss 41 %)
```

同批 NAS 侧收益：S3 里 PC→NAS 那路的丢帧增量 15120 帧（≈315 ms 音频）→ 1440 帧（≈30 ms），
NAS 末水位由 28/55 ms 回到 30/33 ms。
