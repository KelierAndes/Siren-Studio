#!/usr/bin/env python3
"""逐帧检查一段实录里的中断，并判断它是不是欠载或丢帧造成的。

用法：
    python scripts/analyze_gaps.py rec.wav --stats stats.jsonl --log daemon.log

把混音输出按 5 ms 块切开，找三类异常：数字静音（补零）、电平塌陷、块边界的
相位跳变（拼接痕迹）。每个事件再对照同一时刻的快照计数增量与日志，给出归因；
归不了因的就是"既非欠载也非丢帧"的中断，需要单独查。
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field

import numpy as np

SAMPLE_RATE = 48_000
CHANNELS = 2
BLOCK_MS = 5.0
BLOCK = int(SAMPLE_RATE * BLOCK_MS / 1000)

COUNTERS = (
    "underruns",
    "underrun_frames",
    "lost_frames",
    "plc_frames",
    "overflow_packets",
    "buffer_resets",
    "clicks",
    "packets",
)

# 日志里能解释一次中断的事件。
LOG_PATTERNS = (
    ("载体切换", r"carrier|resync|re-?prim"),
    ("流冻结", r"freeze|frozen"),
    ("流超时", r"timed out|timeout"),
    ("溢出修剪", r"trim|overflow"),
    ("缓冲重置", r"buffer reset|reset stream"),
    ("回声门控", r"echo (?:gate|lock)|mute capture|lose lock"),
    ("发送路由", r"refresh_route|route"),
    ("蓝牙链路", r"rfcomm|link (?:up|down|stale)|tunnel"),
    ("克隆 uid", r"cloned node image"),
    ("解码失败", r"decode"),
)

STAMP = re.compile(r"\[(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})\.(\d{3})")


def load_wav(path: str) -> np.ndarray:
    """读 48 kHz / 2 ch 的 WAV；标准库 wave 不支持 IEEE float，这里自己拆 RIFF。"""
    with open(path, "rb") as handle:
        blob = handle.read()
    assert blob[:4] == b"RIFF" and blob[8:12] == b"WAVE", "not a RIFF/WAVE file"
    fmt = blob.index(b"fmt ", 12) + 8  # 跳过 'fmt ' 与块大小
    audio_format = int.from_bytes(blob[fmt + 0 : fmt + 2], "little")
    channels = int.from_bytes(blob[fmt + 2 : fmt + 4], "little")
    rate = int.from_bytes(blob[fmt + 4 : fmt + 8], "little")
    bits = int.from_bytes(blob[fmt + 14 : fmt + 16], "little")
    if audio_format == 0xFFFE:  # WAVE_FORMAT_EXTENSIBLE：真实编码在 SubFormat GUID 的 Data1
        data1 = int.from_bytes(blob[fmt + 24 : fmt + 28], "little")
        audio_format = data1 & 0xFFFF
    data = blob.index(b"data", fmt)
    start = data + 8
    size = int.from_bytes(blob[data + 4 : data + 8], "little")
    if size == 0 or start + size > len(blob):
        # 进程提前退出时 hound 来不及回填长度；按文件实际大小取。
        size = len(blob) - start
    align = channels * (4 if audio_format in (3,) else 2)
    size -= size % align  # 截掉写一半的最后一个样本
    payload = blob[start : start + size]
    assert channels == CHANNELS and rate == SAMPLE_RATE, (channels, rate)
    if audio_format == 3 and bits == 32:
        samples = np.frombuffer(payload, dtype=np.float32)
    elif audio_format == 1 and bits == 32:
        samples = np.frombuffer(payload, dtype=np.int32).astype(np.float64) / 2**31
    elif audio_format == 1 and bits == 16:
        samples = np.frombuffer(payload, dtype=np.int16).astype(np.float64) / 2**15
    else:
        raise SystemExit(f"unsupported WAV format: {audio_format}/{bits} bit")
    return samples.reshape(-1, channels).astype(np.float64)


def block_view(samples: np.ndarray) -> np.ndarray:
    frames = samples.shape[0] // BLOCK
    return samples[: frames * BLOCK].reshape(frames, BLOCK, CHANNELS)


def block_metrics(body: np.ndarray, step_limit: float) -> dict[str, np.ndarray]:
    rms = np.sqrt(np.mean(body**2, axis=(1, 2)))
    rms_db = 20 * np.log10(np.maximum(rms, 1e-12))
    zero = np.all(body == 0.0, axis=(1, 2))
    # 拼接痕迹：块边界上的样本跳变。阈值取"信号自身跳变分布的若干倍"与一个
    # 绝对下限的较大者，这样宽带内容的正常起伏不会被误判成断点。
    prev_last = np.empty_like(body[:, 0, :])
    prev_last[0] = body[0, 0, :]
    prev_last[1:] = body[: body.shape[0] - 1, -1, :]
    step = np.abs(body[:, 0, :] - prev_last).max(axis=1)
    step[0] = 0.0  # 首块没有前驱，不参与跳变判定
    natural = np.abs(np.diff(body[:, :, 0], axis=0))
    reference = float(np.percentile(natural, 99.9)) if natural.size else 0.0
    limit = max(step_limit, 6.0 * reference)
    return {"rms_db": rms_db, "rms": rms, "zero": zero, "step": step, "limit": limit}


def tone_db(body: np.ndarray, freq: float) -> np.ndarray:
    """每块在指定频点上的幅度（dBFS）：测试音一断就能看出来。"""
    mono = body.mean(axis=2)
    t = np.arange(BLOCK) / SAMPLE_RATE
    cos = np.cos(2 * np.pi * freq * t)
    sin = np.sin(2 * np.pi * freq * t)
    re_part = mono @ cos
    im_part = mono @ sin
    amplitude = 2 * np.sqrt(re_part**2 + im_part**2) / BLOCK
    return 20 * np.log10(np.maximum(amplitude, 1e-12))


@dataclass
class Event:
    start: float
    end: float
    kind: str
    depth_db: float
    step: float
    streams: list[str] = field(default_factory=list)
    counters: dict[str, dict[str, int]] = field(default_factory=dict)
    log_hits: list[str] = field(default_factory=list)
    explained: bool = False
    cause: str = ""

    @property
    def duration_ms(self) -> float:
        return (self.end - self.start) * 1000.0


def find_events(
    metrics: dict[str, np.ndarray],
    tone: np.ndarray | None,
    quiet_db: float,
    t0: float = 0.0,
) -> list[Event]:
    rms_db = metrics["rms_db"]
    voiced = rms_db[rms_db > -60]
    reference = float(np.median(voiced)) if voiced.size else -20.0
    floor = max(quiet_db, reference - 30.0)
    collapsed = rms_db < floor
    spliced = metrics["step"] > metrics["limit"]
    flagged = collapsed | metrics["zero"] | spliced

    tone_cut = None
    if tone is not None:
        loud = tone[tone > -70]
        if loud.size:
            tone_cut = tone < (float(np.median(loud)) - 25.0)
            flagged |= tone_cut

    events: list[Event] = []
    index = 0
    total = flagged.size
    while index < total:
        if not flagged[index]:
            index += 1
            continue
        start = index
        while index + 1 < total and flagged[index + 1]:
            index += 1
        stop = index + 1
        span = slice(start, stop)
        kinds = []
        if metrics["zero"][span].any():
            kinds.append("数字静音")
        if collapsed[span].any():
            kinds.append("电平塌陷")
        if spliced[span].any():
            kinds.append("拼接跳变")
        if tone_cut is not None and tone_cut[span].any():
            kinds.append("测试音断")
        events.append(
            Event(
                start=t0 + start * BLOCK_MS / 1000.0,
                end=t0 + stop * BLOCK_MS / 1000.0,
                kind="+".join(kinds) or "异常",
                depth_db=float(rms_db[span].min()),
                step=float(metrics["step"][span].max()),
            )
        )
        index = stop
    return events


def load_stats(path: str) -> list[dict]:
    rows = []
    with open(path, encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def stream_series(rows: list[dict]) -> dict[str, list[tuple[float, dict]]]:
    series: dict[str, list[tuple[float, dict]]] = {}
    for row in rows:
        seconds = float(row.get("seconds", 0.0))
        for stream in row.get("center", {}).get("streams", []) or []:
            key = f"{stream.get('label')}#{stream.get('stream_id')}"
            series.setdefault(key, []).append((seconds, stream))
    return series


def delta_around(points: list[tuple[float, dict]], start: float, end: float):
    before = None
    after = None
    for seconds, stream in points:
        if seconds <= start:
            before = stream
        if seconds >= end and after is None:
            after = stream
    if before is None or after is None or before is after:
        return None
    deltas = {
        name: int(after.get(name, 0)) - int(before.get(name, 0))
        for name in COUNTERS
        if int(after.get(name, 0)) - int(before.get(name, 0)) > 0
    }
    notes = []
    if before.get("path") != after.get("path"):
        notes.append(f"路径 {before.get('path')}→{after.get('path')}")
    if bool(before.get("active")) != bool(after.get("active")):
        notes.append("活跃翻转")
    return deltas, notes


def parse_log(path: str) -> list[tuple[float, str, str]]:
    """返回 (相对秒, 归因名, 原文)；相对秒以第一行带时间戳的日志为零点。"""
    entries: list[tuple[float, str, str]] = []
    base = None
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            stamp = STAMP.match(line)
            if not stamp:
                continue
            groups = [int(value) for value in stamp.groups()]
            seconds = groups[3] * 3600 + groups[4] * 60 + groups[5] + groups[6] / 1000.0
            if base is None:
                base = seconds
            for name, pattern in LOG_PATTERNS:
                if re.search(pattern, line, re.IGNORECASE):
                    entries.append((seconds - base, name, line.strip()[:180]))
                    break
    return entries


def classify(event: Event) -> None:
    merged: dict[str, int] = {}
    for values in event.counters.values():
        for name, value in values.items():
            merged[name] = merged.get(name, 0) + value
    # 包到达率是判据：200 包/秒说明网络没断供，此时仍然欠载就是本地消费端饥饿。
    expected = max(1.0, event.duration_ms / 1000.0 * 200.0)
    arrived = merged.get("packets", 0) / max(1, len(event.counters))
    starved = arrived < 0.8 * expected
    if merged.get("underrun_frames") or merged.get("underruns"):
        event.explained = True
        if starved:
            event.cause = f"欠载 {merged.get('underrun_frames', 0)} 帧（包到达仅 {arrived:.0f}/{expected:.0f}）"
        else:
            event.cause = (
                f"欠载 {merged.get('underrun_frames', 0)} 帧，但包按 {arrived:.0f}/{expected:.0f} 正常到达"
                " → 本地消费/调度饥饿"
            )
    elif merged.get("lost_frames") or merged.get("plc_frames"):
        event.explained = True
        event.cause = f"丢帧 {merged.get('lost_frames', 0)} / PLC {merged.get('plc_frames', 0)}"
    elif merged.get("overflow_packets"):
        event.explained = True
        event.cause = f"溢出修剪 {merged['overflow_packets']} 包"
    elif merged.get("buffer_resets"):
        event.explained = True
        event.cause = f"缓冲重置 {merged['buffer_resets']} 次"
    elif event.log_hits:
        names = sorted({hit.split(" ", 1)[1] for hit in event.log_hits if " " in hit})
        event.cause = "仅日志：" + "、".join(names) if names else "仅日志命中"
    else:
        event.cause = "未解释：计数与日志均无变化"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wav")
    parser.add_argument("--stats", help="siren mesh --stats-json 的输出")
    parser.add_argument("--log", help="守护进程日志（env_logger 毫秒时间戳）")
    parser.add_argument("--tone", type=float, action="append", default=[], help="测试音频点，可重复")
    parser.add_argument("--quiet-db", type=float, default=-55.0)
    parser.add_argument("--step", type=float, default=0.05, help="块边界跳变的绝对下限")
    parser.add_argument("--min-ms", type=float, default=5.0)
    parser.add_argument("--from", dest="t_from", type=float, default=0.0, help="只分析该时刻之后")
    parser.add_argument("--to", dest="t_to", type=float, default=None, help="只分析该时刻之前")
    parser.add_argument("--max-events", type=int, default=120)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    samples = load_wav(args.wav)
    seconds = samples.shape[0] / SAMPLE_RATE
    offset = int(args.t_from * SAMPLE_RATE) // BLOCK * BLOCK
    limit = int(seconds * SAMPLE_RATE) if args.t_to is None else min(int(args.t_to * SAMPLE_RATE), samples.shape[0])
    samples = samples[offset:limit // BLOCK * BLOCK]
    seconds = samples.shape[0] / SAMPLE_RATE
    body = block_view(samples)
    metrics = block_metrics(body, args.step)
    tone = None
    if args.tone:
        tone = np.min(np.stack([tone_db(body, freq) for freq in args.tone]), axis=0)

    events = [
        event
        for event in find_events(metrics, tone, args.quiet_db, args.t_from)
        if event.duration_ms >= args.min_ms
    ]

    series = stream_series(load_stats(args.stats)) if args.stats else {}
    log_entries = parse_log(args.log) if args.log else []
    for event in events:
        for name, points in series.items():
            result = delta_around(points, event.start, event.end)
            if result is None:
                continue
            deltas, notes = result
            if deltas or notes:
                event.counters[name] = deltas
                label = name + ("，" + "，".join(notes) if notes else "")
                event.streams.append(label)
        near = [entry for entry in log_entries if event.start - 1.0 <= entry[0] <= event.end + 1.0]
        event.log_hits = [f"{when:.2f}s {name}" for when, name, _ in near[:4]]
        classify(event)

    unexplained = [event for event in events if not event.explained]
    summary = {
        "wav": args.wav,
        "seconds": round(seconds, 3),
        "blocks": int(metrics["rms_db"].size),
        "median_db": round(float(np.median(metrics["rms_db"])), 1),
        "zero_blocks": int(metrics["zero"].sum()),
        "events": len(events),
        "unexplained": len(unexplained),
        "unexplained_ms": round(sum(event.duration_ms for event in unexplained), 1),
    }
    if args.json:
        payload = dict(summary)
        payload["list"] = [
            {
                "start": round(event.start, 3),
                "ms": round(event.duration_ms, 1),
                "kind": event.kind,
                "depth_db": round(event.depth_db, 1),
                "step": round(event.step, 3),
                "cause": event.cause,
                "explained": event.explained,
                "streams": event.streams,
                "counters": event.counters,
                "log": event.log_hits,
            }
            for event in events
        ]
        print(json.dumps(payload, ensure_ascii=False))
        return 0

    print(
        f"# {args.wav}  {seconds:.1f} s / {summary['blocks']} 个 5 ms 块"
        f"，中位电平 {summary['median_db']} dBFS，数字静音块 {summary['zero_blocks']} 个"
    )
    print(f"异常事件 {len(events)} 个；无法归因于欠载/丢帧/修剪/重置的 {len(unexplained)} 个"
          f"，合计 {summary['unexplained_ms']} ms\n")
    if events:
        print("| 时刻(s) | 时长(ms) | 类型 | 最低电平 | 边界跳变 | 归因 | 流/计数 | 日志 |")
        print("| :-- | :-- | :-- | :-- | :-- | :-- | :-- | :-- |")
        for event in events[: args.max_events]:
            counters = "; ".join(
                f"{name}: {', '.join(f'{k}+{v}' for k, v in values.items()) or '—'}"
                for name, values in event.counters.items()
            )
            print(
                f"| {event.start:.2f} | {event.duration_ms:.0f} | {event.kind} |"
                f" {event.depth_db:.1f} dBFS | {event.step:.3f} | {event.cause} |"
                f" {clean(counters)} | {clean(event.log_hits)} |"
            )
        if len(events) > args.max_events:
            print(f"\n(仅列出前 {args.max_events} 个，共 {len(events)} 个)")
    return 0


def clean(value) -> str:
    if isinstance(value, list):
        value = " / ".join(value)
    text = (value or "").replace("|", "\\|")
    return text or "—"


if __name__ == "__main__":
    sys.exit(main())
