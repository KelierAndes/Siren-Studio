#!/usr/bin/env python3
"""用脉冲标记量端到端延迟：在录音里找 1 kHz 脉冲，与发送音里的理论发出时刻对齐。

脉冲排布由 collect_1h.pulse_wav 决定：每 `--every` 秒一个 10 ms 的 1 kHz 脉冲，
在每个 1 s 栅格内偏移 0.125 s。延迟 = 到达 − 发出；绝对值需要发送/录音两台
机器的共同起点，这里只有 timeline.csv 的 hub_started / pc_started 两个启动标记，
而 pc_started 打在 PowerShell 装载 wav 之前，装载时间混进了绝对量，所以绝对列
只能当上界；真正可用的是 jitter_ms —— 对到达时刻做线性拟合去掉两台机器时钟
漂移后的残差，它完全不需要对时。

    python scripts/measure_pulse_latency.py recording.wav --timeline timeline.csv --out T1/latency.csv --phase T1
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path

import numpy as np

SR = 48_000
PULSE_HZ = 1000.0
PULSE_OFFSET = 0.125


def read_wav(path: Path) -> np.ndarray:
    blob = path.read_bytes()
    fmt = blob.index(b"fmt ") + 8
    channels = int.from_bytes(blob[fmt + 2 : fmt + 4], "little")
    rate = int.from_bytes(blob[fmt + 4 : fmt + 8], "little")
    bits = int.from_bytes(blob[fmt + 14 : fmt + 16], "little")
    data = blob.index(b"data", fmt) + 8
    payload = blob[data:]
    if bits == 16:
        samples = np.frombuffer(payload, dtype="<i2").astype(np.float64) / 32768.0
    elif bits == 32:
        samples = np.frombuffer(payload, dtype="<f4").astype(np.float64)
    else:
        raise SystemExit(f"unsupported {bits} bit")
    mono = samples.reshape(-1, channels).mean(axis=1) if channels else samples
    return mono / (rate / SR) if rate != SR else mono


def pulse_times(mono: np.ndarray, every: float, window: float = 0.005) -> list[float]:
    """5 ms 匹配滤波找 1 kHz 脉冲；阈值取 6× 中位数与 0.08 的较大者。

    脉冲只有 10 ms，窗口必须比它短，否则被同一窗口里的 440 Hz 测试音稀释
    （12 ms 窗口下峰值比值不到 1，永远过不了阈值）。
    """
    n = int(window * SR)
    t = np.arange(n) / SR
    frames = mono.size // n
    blocks = mono[: frames * n].reshape(frames, n)
    energy = np.abs(blocks @ np.exp(-2j * np.pi * PULSE_HZ * t)) * 2 / n
    threshold = max(6.0 * float(np.median(energy)), 0.08)
    hits = np.where(energy > threshold)[0]
    times: list[float] = []
    run: list[int] = []
    for index in hits:
        if run and index - run[-1] > 4:
            times.append((run[0] + int(np.argmax(energy[run]))) * n / SR)
            run = []
        run.append(index)
    if run:
        times.append((run[0] + int(np.argmax(energy[run]))) * n / SR)
    return times


def marks(timeline: Path, phase: str) -> tuple[float, float]:
    """取该阶段自己的起点标记：timeline.csv 是所有阶段共用的。"""
    hub = pc = 0.0
    for line in timeline.read_text(encoding="utf-8").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if phase and row.get("phase") != phase:
            continue
        if row.get("action") == "hub_started":
            hub = row["ts_ms"] / 1000.0
        if row.get("action") == "pc_started":
            pc = row["ts_ms"] / 1000.0
    return hub, pc


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("recording")
    parser.add_argument("--timeline")
    parser.add_argument("--every", type=float, default=20.0)
    parser.add_argument("--out", required=True)
    parser.add_argument("--phase", default="", help="timeline.csv 里的阶段名")
    args = parser.parse_args()

    mono = read_wav(Path(args.recording))
    arrivals = pulse_times(mono, args.every)
    if not arrivals:
        print("no pulses found", file=sys.stderr)
        return 1
    # 相邻脉冲隔几个周期是整数，直接从第一个检测点递推，避免最近邻取整在
    # 偏移接近 ±every/2 时跳格；第一个脉冲的序号未知，只影响绝对量的常数。
    index = [round((a - arrivals[0]) / args.every) for a in arrivals]
    # 两台机器的时钟本来就有漂（实测 0.1 % 量级），所以先对到达时刻做一次线性拟合：
    # 拟合斜率就是录音时钟下的真实脉冲周期，残差才是链路抖动。
    design = np.column_stack([np.array(index, dtype=float), np.ones(len(index))])
    slope, intercept = np.linalg.lstsq(design, np.array(arrivals), rcond=None)[0]
    jitter = [(a - (slope * i + intercept)) * 1000.0 for a, i in zip(arrivals, index)]
    first_grid = round((arrivals[0] - PULSE_OFFSET) / args.every) * args.every + PULSE_OFFSET
    hub = pc = 0.0
    if args.timeline:
        hub, pc = marks(Path(args.timeline), args.phase)
    launch_gap = (hub - pc) if (hub and pc) else 0.0

    with open(args.out, "w", newline="", encoding="utf-8") as out:
        writer = csv.writer(out)
        writer.writerow(
            ["ts_ms", "pulse", "emission_s", "arrival_s", "latency_ms", "jitter_ms"]
        )
        for number, (arrival, i) in enumerate(zip(arrivals, index)):
            emission = first_grid + i * args.every
            # 绝对量 = 到达(录音时钟) − 发出(播放时钟)；两套时钟用启动标记换算。
            # 注意标记打在 PlaySync 之前，PowerShell 装载 288 MB 的时间混在这里，
            # 所以 latency_ms 只能当上界看，jitter_ms 才是可用值。
            latency_ms = (arrival - emission + launch_gap) * 1000.0
            writer.writerow(
                [
                    int((hub + arrival) * 1000) if hub else int(arrival * 1000),
                    number,
                    f"{emission:.3f}",
                    f"{arrival:.3f}",
                    f"{latency_ms:.1f}",
                    f"{jitter[number]:.1f}",
                ]
            )
    values = np.abs(np.array(jitter))
    print(
        json.dumps(
            {
                "pulses": len(arrivals),
                "period_ms": round(slope * 1000.0, 1),
                "drift_ppm": round((slope / args.every - 1.0) * 1e6, 0),
                "abs_jitter_p50_ms": round(float(np.median(values)), 1),
                "abs_jitter_p95_ms": round(float(np.percentile(values, 95)), 1),
                "abs_jitter_max_ms": round(float(values.max()), 1),
                "absolute_anchor_used": bool(hub and pc),
            },
            ensure_ascii=False,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
