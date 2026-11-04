#!/usr/bin/env python3
"""把采集目录汇总成计划要的四张结论表：静默按位置归类 / 修剪是突发还是漂移 /
切换各段耗时 / 网络质量与水位的关系。

    python scripts/summarise_1h.py out/2026-11-04_1h
"""

from __future__ import annotations

import csv
import json
import re
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

import numpy as np

STAMP = re.compile(r"\[(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})\.(\d{3})Z")
JOINED = re.compile(r"stream \d+ from \S+ \(\w+, origin (?P<origin>[0-9a-f]+)\) joined")


def epoch_ms(stamp: re.Match) -> int:
    base = datetime.strptime(stamp.group(1), "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc)
    return int(base.timestamp() * 1000) + int(stamp.group(2))


def rows(path: Path) -> list[dict]:
    if not path.exists():
        return []
    with open(path, encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def pct(values: list[float], q: float) -> float:
    if not values:
        return float("nan")
    return float(np.percentile(np.array(values, dtype=float), q))


def stats_line(values: list[float]) -> str:
    if not values:
        return "—"
    arr = np.array(values, dtype=float)
    return f"n={len(arr)} P50={np.median(arr):.0f} P95={pct(values, 95):.0f} max={arr.max():.0f}"


def _per_hour(out: Path, phase: Path, ms: float) -> str:
    """稳态静音折算到每小时；时长取 hub 统计的最后一帧 seconds。"""
    stats = phase / "hub.stats.jsonl"
    if not stats.exists():
        return "—"
    last = None
    for line in stats.read_text(encoding="utf-8").splitlines():
        if line.strip():
            last = line
    if last is None:
        return "—"
    secs = json.loads(last).get("center", {}).get("seconds", 0.0)
    if secs <= 0:
        return "—"
    return f"{ms / secs * 3600:.0f} ms"


def per_phase(out: Path) -> list[Path]:
    return sorted(p for p in out.iterdir() if p.is_dir() and (p / "hub.log").exists())


def starts(out: Path) -> dict[str, int]:
    """各阶段"所有源都已入网"的时刻（hub 时钟，epoch ms）+ 10 s。

    不用 timeline.csv 的启动标记：那台 NAS 的时钟比 PC 快 50–60 s 且在漂，
    跨机器的绝对时间不可用；hub.log 自己的时间戳才是同一条时间线。
    """
    marks: dict[str, int] = {}
    for phase in per_phase(out):
        firsts: dict[str, int] = {}
        for line in (phase / "hub.log").read_text(encoding="utf-8", errors="replace").splitlines():
            stamp = STAMP.search(line)
            if not stamp:
                continue
            match = JOINED.search(line)
            if match and match.group("origin") not in firsts:
                firsts[match.group("origin")] = epoch_ms(stamp)
        if firsts:
            marks[phase.name] = max(firsts.values()) + 10_000
    return marks


def main() -> int:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "out")
    phases = per_phase(out)
    if not phases:
        print("没有可分析的阶段", file=sys.stderr)
        return 1

    print("## 1. 静默按位置归类（I3 零值游程）\n")
    print("| 阶段 | 采集后 run_ms | 混音后全部 | 其中启动段 | 稳态总时长 | 稳态折算 / 小时 |")
    print("| :-- | :-- | :-- | :-- | :-- | :-- |")
    begun = starts(out)
    steady_ms: dict[str, float] = {}
    for phase in phases:
        zero = rows(phase / "zeroruns.csv")
        cap = [float(r["run_ms"]) for r in zero if r["point"] == "capture"]
        mix = [float(r["run_ms"]) for r in zero if r["point"] == "mix"]
        edge = begun.get(phase.name, 0)
        boot = [float(r["run_ms"]) for r in zero
                if r["point"] == "mix" and edge and int(r["ts_ms"]) < edge]
        steady = sum(mix) - sum(boot)
        steady_ms[phase.name] = steady
        print(
            f"| {phase.name} | {stats_line(cap)} | {stats_line(mix)} | "
            f"{len(boot)} 次 / {sum(boot):.0f} ms | {steady:.0f} ms | {_per_hour(out, phase, steady)} |"
        )

    print("\n## 2. 修剪：突发还是漂移（I1）\n")
    print("| 阶段 | 次数 | fill_before_ms P50/P95 | 丢弃 ms P50/P95 | 修剪间隔 ms P50/P95 | 修剪前 ratio P50 |")
    print("| :-- | :-- | :-- | :-- | :-- | :-- |")
    for phase in phases:
        trims = rows(phase / "trims.csv")
        if not trims:
            print(f"| {phase.name} | 0 | — | — | — | — |")
            continue
        before = [float(r["fill_before_ms"]) for r in trims]
        dropped = [float(r["dropped_ms"]) for r in trims]
        since = [float(r["ms_since_last_trim"]) for r in trims if r["ms_since_last_trim"] != "nan"]
        ratios = [float(r["ratio"]) for r in trims]
        print(
            f"| {phase.name} | {len(trims)} | {np.median(before):.0f}/{pct(before, 95):.0f} | "
            f"{np.median(dropped):.0f}/{pct(dropped, 95):.0f} | "
            f"{(f'{np.median(since):.0f}/{pct(since, 95):.0f}' if since else '首次')} | "
            f"{np.median(ratios):.5f} |"
        )

    print("\n## 3. 切换各段耗时（I2）\n")
    print("| 阶段 | 切换次数 | 无包 ms P50/P95 | 合成静音 ms P50 | 重新填充 ms P50 | 总静音 ms P50/P95 |")
    print("| :-- | :-- | :-- | :-- | :-- | :-- |")
    for phase in phases:
        switches = rows(phase / "switches.csv")
        if not switches:
            print(f"| {phase.name} | 0 | — | — | — | — |")
            continue
        nop = [float(r["no_packet_ms"]) for r in switches]
        synth = [float(r["synth_ms"]) for r in switches]
        refill = [float(r["refill_ms"]) for r in switches]
        total = [float(r["total_mute_ms"]) for r in switches]
        print(
            f"| {phase.name} | {len(switches)} | {np.median(nop):.0f}/{pct(nop, 95):.0f} | "
            f"{np.median(synth):.0f} | {np.median(refill):.0f} | "
            f"{np.median(total):.0f}/{pct(total, 95):.0f} |"
        )

    print("\n## 4. 网络质量与水位（hub 快照）\n")
    print("| 阶段 | 流 | 水位 ms P50/P95 | 欠载 | 丢帧 | 修剪 | 稳态无声 ms | RTT ms P50/P95 |")
    print("| :-- | :-- | :-- | :-- | :-- | :-- | :-- | :-- |")
    for phase in phases:
        stats = phase / "hub.stats.jsonl"
        if not stats.exists():
            continue
        parsed = [
            json.loads(line)
            for line in stats.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        if not parsed:
            continue
        buffers = defaultdict(list)
        for row in parsed:
            for stream in row.get("center", {}).get("streams", []) or []:
                buffers[stream["label"][:12]].append(stream["buffer_ms"])
        last = parsed[-1]
        master = last["center"]["master"]
        rtts = [
            peer["rtt_ms"]
            for peer in last.get("peers", [])
            if isinstance(peer.get("rtt_ms"), (int, float))
        ]
        rtt_cell = f"{np.median(rtts):.0f}/{pct(rtts, 95):.0f}" if rtts else "—"
        for label, buf in buffers.items():
            stream = next(
                (s for s in last["center"]["streams"] if s["label"][:12] == label), None
            )
            if stream is None:
                continue
            print(
                f"| {phase.name} | {label} | {np.median(buf):.0f}/{pct(buf, 95):.0f} | "
                f"{stream['underruns']} | {stream['lost_frames']} | {stream['buffer_resets']} | "
                f"{steady_ms.get(phase.name, 0):.0f} | {rtt_cell} |"
            )

    print("\n## 5. 端到端延迟（脉冲；jitter 已扣除两台机器的时钟漂移）\n")
    print("| 阶段 | 脉冲数 | 周期 ms（漂移 ppm） | jitter P50 | jitter P95 | min | max |")
    print("| :-- | :-- | :-- | :-- | :-- | :-- | :-- |")
    for phase in phases:
        latency = phase / "latency.csv"
        if not latency.exists():
            print(f"| {phase.name} | — | — | — | — | — | — |")
            continue
        data = rows(latency)
        values = [float(r["jitter_ms"]) for r in data]
        arrivals = [float(r["arrival_s"]) for r in data]
        # 漏检的脉冲会把线性拟合带歪，所以按"相邻间隔 / 取整周期数"取中位数。
        steps = []
        for before, after in zip(arrivals, arrivals[1:]):
            gap = after - before
            periods = max(1, round(gap / 20.0))
            steps.append(gap / periods)
        if steps and max(steps) - min(steps) < 0.05 * 20.0:
            order = np.arange(len(arrivals), dtype=float)
            period = float(np.polyfit(order, np.array(arrivals), 1)[0]) * 1000.0
            cell = f"{period:.1f} ms ({(period / 20000 - 1) * 1e6:+.0f} ppm)"
        elif steps:
            cell = "间隔不稳（含中断）"
        else:
            cell = "—"
        print(
            f"| {phase.name} | {len(values)} | {cell} | {np.median(values):.0f} | {pct(values, 95):.0f} | "
            f"{min(values):.0f} | {max(values):.0f} |"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
