#!/usr/bin/env python3
"""W3：F5 自适应「降级→恢复」的长观察（tc netem 只压出口，逐周期计时）。

为什么在 NAS 上压而不是换机器：netem.sh 的 rate 档禁全双工（该机两次挂起都是
全双工限速 + 永不退避的 UDP 音频洪流，见提交 7b41fde），egress 形式才放行。出口
被压时心跳的回程也排队，所以两侧发送者的 RTT P99 都会涨——PC 与 NAS 都可能降级，
两边的日志都要读，事件按来源记下来。

每周期判据：
- degrade 出现一次（日志 "degraded (rtt p99 …) — switching to Opus"）
- restore 出现一次（日志 "recovered — restoring"）
- 单周期翻转次数 == 2；>2 记为振荡
恢复等待按 F5 的指数退避（ADAPT_BACK_TICKS << attempts）本来就该逐次变长，这里把
实测值逐周期记下来，用来区分"退避生效"与"恢复触发不了"。

产物：out/f5_cycles_<日期>/cycles.csv、summary.md、cycleNN/ 两侧的日志切片。
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.request
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import collect_1h as harness  # noqa: E402
import matrix_dualstack as M  # noqa: E402

DEGRADE = re.compile(r"degraded \(rtt p99 ([\d.]+) ms, hb loss ([\d.]+) %\)")
# 破折号两侧用 .{0,6} 兜住：日志里那是 U+2014，一旦哪条链路按错误编码解码就会
# 变成替换符，写死字符会让"其实发生了"的事件读成"没发生"。
RESTORE = re.compile(r"recovered.{0,6}restoring (\w+)")
# 只整 udp/47900、双向、SSH 不受影响（netem.sh 的 raw/audio-only 路径）。
# 为什么不用 bw1000：NAS 的 loopback 采集是静音的（E2 抑制），出口队列排不起来，
# RTT 涨不过 400 ms 判据 —— 实跑 10 轮里 9 轮 "无降级"，是故障没生效而不是 F5 坏了。
# 260 ms 单向 × 双向 ≈ 520 ms RTT，稳稳越过 ADAPT_RTT_US=400 ms。
IMPAIR = "delay 260ms 20ms distribution normal"
NAS_LOG = "/tmp/siren-web.log"
PC_HOST = "192.168.2.107"


def egress_iface() -> str:
    """限速要压在实际承载 mesh 流量的接口上。"""
    out = M.nas_sh(f"ip route get {PC_HOST} 2>/dev/null | head -1")
    match = re.search(r"dev (\S+)", out)
    if not match:
        return "enp1s0"
    found = match.group(1)
    print(f"f5: 到 {PC_HOST} 的路由走 {found}（{out.strip()[:80]}）", flush=True)
    return found


def nas_peer_rtt() -> list[float]:
    """NAS 侧各对端的心跳 RTT（毫秒）：证明故障确实上去了。"""
    try:
        with urllib.request.urlopen(
                f"http://{harness.NAS_HOST}:8090/api/status", timeout=3) as resp:
            snap = json.loads(resp.read().decode("utf-8", "replace")).get("snapshot") or {}
        return [float(p["rtt_ms"]) for p in snap.get("peers") or []
                if isinstance(p.get("rtt_ms"), (int, float))]
    except Exception:  # noqa: BLE001
        return []


def sender_codec() -> str:
    try:
        with urllib.request.urlopen(
                f"http://{harness.NAS_HOST}:8090/api/status", timeout=3) as resp:
            snap = json.loads(resp.read().decode("utf-8", "replace")).get("snapshot") or {}
            return str((snap.get("sender") or {}).get("codec") or "")
    except Exception:  # noqa: BLE001 - 限速期间取不到状态也要继续观察
        return "?"


def log_size() -> dict[str, int]:
    """本周期起点：两侧日志各自的字节偏移，之后只在增量里找事件。

    不这样做的话，上一周期的 "recovered — restoring" 会让本周期第一秒就"恢复成功"。
    """
    pc = M.PC_LOG.stat().st_size if M.PC_LOG.exists() else 0
    nas = int(M.nas_sh(f"stat -c %s {NAS_LOG} 2>/dev/null || echo 0").strip() or 0)
    return {"pc": pc, "nas": nas}


def read_since(sizes: dict[str, int]) -> dict[str, str]:
    pc_text = ""
    if M.PC_LOG.exists():
        with open(M.PC_LOG, encoding="utf-8", errors="replace") as handle:
            handle.seek(sizes["pc"])
            pc_text = handle.read()
    nas_text = M.nas_sh(f"tail -c +{sizes['nas'] + 1} {NAS_LOG} 2>/dev/null")
    return {"pc": pc_text, "nas": nas_text}


def netem(apply: bool) -> str:
    """apply=上故障，False=撤除。netem.sh 自己挑默认路由接口（实测 enp1s0）。

    nas_sudo_sh 已经加过 `echo PW | sudo -S -p ''`，这里不能再套一层 sudo：
    那会把口令喂给内层命令，外层 sudo 等不到口令而卡到超时。
    """
    tail = f"apply raw '{IMPAIR}'" if apply else "clear"
    return M.nas_sudo_sh(f"bash $HOME/scripts/linux/netem.sh {tail}")


def watch(seconds: float, expect: re.Pattern, sizes: dict[str, int]) -> dict:
    """在两侧日志的增量里等 expect；返回耗时、来源与原文，超时记 t_s=None。"""
    started = time.time()
    reads = 0
    rtts: list[float] = []
    while time.time() - started < seconds:
        time.sleep(5)
        reads += 1
        rtts.extend(nas_peer_rtt())
        for name, text in read_since(sizes).items():
            match = expect.search(text)
            if match:
                return {"t_s": round(time.time() - started, 1), "source": name,
                        "reads": reads, "rtt_max_ms": max(rtts, default=None),
                        "detail": match.group(0)[:120]}
    return {"t_s": None, "source": "-", "reads": reads,
            "rtt_max_ms": max(rtts, default=None), "detail": "timeout"}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nas-pass", default=os.environ.get("SIREN_NAS_PASS", ""))
    parser.add_argument("--cycles", type=int, default=10)
    parser.add_argument("--degrade-ceiling", type=float, default=180.0)
    parser.add_argument("--restore-ceiling", type=float, default=600.0,
                        help="等恢复的最长秒数（F5 退避会让它逐次变长）")
    parser.add_argument(
        "--out", default=str(ROOT / "out" / f"f5_cycles_{datetime.now():%Y-%m-%d_%H%M}"))
    args = parser.parse_args()
    if not args.nas_pass:
        print("缺少 --nas-pass（或 SIREN_NAS_PASS）", file=sys.stderr)
        return 2
    harness.NAS_PASS = args.nas_pass

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    M.nas_sh("export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; "
             "pactl list sinks short | grep -q play_sink || pactl load-module module-null-sink "
             "sink_name=play_sink sink_properties=device.description=play_sink; "
             "pactl set-default-sink play_sink")
    M.daemon_start()
    print("启动 PC 节点（tone 上行，让发送端有东西可降级）…", flush=True)
    pc = M.pc_start()
    time.sleep(20)
    iface = egress_iface()

    rows: list[dict] = []
    unreachable = 0
    try:
        for cycle in range(1, args.cycles + 1):
            directory = out / f"cycle{cycle:02d}"
            directory.mkdir(parents=True, exist_ok=True)
            sizes = log_size()
            codec_before = sender_codec()
            netem(True)
            degrade = watch(args.degrade_ceiling, DEGRADE, sizes)
            # 必须先解除限速，恢复才有可能发生（F5 判据是心跳路径变健康）。
            netem(False)
            restore = watch(args.restore_ceiling, RESTORE, sizes) if degrade["t_s"] else {
                "t_s": None, "source": "-", "reads": 0, "detail": "skipped (no degrade)"}
            time.sleep(25)
            increments = read_since(sizes)
            slice_pc, slice_nas = increments["pc"], increments["nas"]
            (directory / "pc.slice.log").write_text(slice_pc, encoding="utf-8")
            (directory / "nas.slice.log").write_text(slice_nas, encoding="utf-8")
            flips = sum(
                len(DEGRADE.findall(text)) + len(RESTORE.findall(text))
                for text in (slice_pc, slice_nas))
            row = {
                "cycle": cycle,
                "codec_before": codec_before,
                "degrade_t_s": degrade["t_s"],
                "degrade_source": degrade["source"],
                "restore_t_s": restore["t_s"],
                "restore_source": restore["source"],
                "flips_in_pc_slice": flips,
                "oscillating": flips > 2,
                "codec_after": sender_codec(),
                # 判据行自带 rtt p99 与心跳丢包，这是最权威的证据；采样 RTT 只当旁证。
                "degrade_rtt_p99_ms": (re.search(r"rtt p99 ([\d.]+) ms", degrade["detail"]) or
                                       [None, None])[1] if degrade["detail"] else None,
                "degrade_hb_loss_pct": (re.search(r"hb loss ([\d.]+) %", degrade["detail"]) or
                                        [None, None])[1] if degrade["detail"] else None,
                "degrade_rtt_max_ms": degrade.get("rtt_max_ms"),
                "restore_rtt_max_ms": restore.get("rtt_max_ms"),
                "degrade_detail": degrade["detail"],
                "restore_detail": restore["detail"],
            }
            rows.append(row)
            # 这台机器被限速压挂过两次：连续两轮取不到状态就收工，不硬跑满 10 轮。
            unreachable = unreachable + 1 if row["codec_after"] == "?" else 0
            (out / "cycles.json").write_text(
                json.dumps(rows, ensure_ascii=False, indent=2), encoding="utf-8")
            print(f"[{datetime.now():%H:%M:%S}] cycle {cycle}: degrade "
                  f"{row['degrade_t_s']} s ({row['degrade_source']}), restore "
                  f"{row['restore_t_s']} s ({row['restore_source']}), flips {flips}", flush=True)
            if unreachable >= 2:
                (out / "aborted.txt").write_text(
                    f"NAS 连续 {unreachable} 轮不可达，停在第 {cycle} 轮", encoding="utf-8")
                print("NAS 连续不可达，提前结束（记录已写 cycles.json）", flush=True)
                break
            time.sleep(20)
    finally:
        netem(False)
        if pc.poll() is None:
            pc.terminate()
        M.daemon_start()

    degraded = sum(1 for r in rows if r["degrade_t_s"] is not None)
    restored = sum(1 for r in rows if r["restore_t_s"] is not None)
    oscillating = sum(1 for r in rows if r["oscillating"])
    waits = [r["restore_t_s"] for r in rows if r["restore_t_s"] is not None]
    summary = [
        f"# F5 降级→恢复长观察（{datetime.now():%Y-%m-%d %H:%M}）",
        "",
        f"- 周期数：{len(rows)}；故障形式：`raw '{IMPAIR}'`，只整 udp/47900 双向（netem.sh 的 audio-only 路径，SSH 不受影响）",
        f"- 降级触发：{degraded}/{len(rows)}",
        f"- 恢复触发：{restored}/{len(rows)}",
        f"- 振荡（单周期翻转 >2 次）：{oscillating}",
        f"- 恢复等待实测：min {min(waits) if waits else '-'} s，max {max(waits) if waits else '-'} s"
        "（F5 的指数退避本应让等待逐次变长；这里要确认的是它仍会恢复、且不发抖）",
        "",
        "| 周期 | 降级 s | 判据行 rtt p99 / hb loss | 恢复 s | 恢复码 | 来源 | 切片内翻转 |",
        "| :-- | --: | :-- | --: | :-- | :-- | --: |",
    ] + [
        f"| {r['cycle']} | {r['degrade_t_s']} | "
        f"{r.get('degrade_rtt_p99_ms')} ms / {r.get('degrade_hb_loss_pct')} % | "
        f"{r['restore_t_s']} | {r.get('restore_detail') or '-'} | {r['restore_source']} | "
        f"{r['flips_in_pc_slice']} |"
        for r in rows
    ]
    (out / "summary.md").write_text("\n".join(summary) + "\n", encoding="utf-8")
    print("\n".join(summary))
    print(f"产物在 {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
