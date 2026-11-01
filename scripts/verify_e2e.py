#!/usr/bin/env python3
"""End-to-end verification for SirenStudio.

Starts real center/sender processes, records the mixed audio, then checks the
WAV spectrum and the JSONL runtime stats. Writes target/e2e/report.json.

Usage:
    python scripts/verify_e2e.py
    python scripts/verify_e2e.py --skip-loopback
"""

from __future__ import annotations

import argparse
import json
import os
import struct
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "siren.exe"
OUT = ROOT / "target" / "e2e"
LOGS = OUT / "logs"
PROCS: list[subprocess.Popen] = []
REPORT: dict = {"tests": {}, "ok": True}


class TestFailure(Exception):
    pass


def ensure(condition: bool, message: str) -> None:
    if not condition:
        raise TestFailure(message)


def start(args: list[str], name: str, stdout_to: Path | None = None) -> subprocess.Popen:
    LOGS.mkdir(parents=True, exist_ok=True)
    log = open(LOGS / f"{name}.log", "w", encoding="utf-8")
    stdout = open(stdout_to, "w", encoding="utf-8") if stdout_to else log
    proc = subprocess.Popen(
        [str(BIN), *args],
        stdout=stdout,
        stderr=log,
        cwd=ROOT,
        creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0,
    )
    PROCS.append(proc)
    return proc


def terminate_from(mark: int) -> None:
    for proc in PROCS[mark:]:
        if proc.poll() is None:
            proc.terminate()


def wait(proc: subprocess.Popen, timeout: float) -> int:
    try:
        return proc.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        proc.terminate()
        raise TestFailure(f"process {proc.pid} timed out")


def wav_duration(path: Path) -> float:
    with open(path, "rb") as handle:
        header = handle.read(12)
        ensure(header[:4] == b"RIFF" and header[8:12] == b"WAVE", "not a WAV file")
        sample_rate = channels = 0
        while True:
            chunk = handle.read(8)
            if len(chunk) < 8:
                raise TestFailure("WAV has no data chunk")
            cid = chunk[:4]
            size = struct.unpack("<I", chunk[4:])[0]
            if cid == b"fmt ":
                body = handle.read(size)
                fmt = struct.unpack("<HHIIHH", body[:16])
                channels, sample_rate = fmt[1], fmt[2]
            elif cid == b"data":
                return size / (sample_rate * channels * 4)
            else:
                handle.seek(size + (size & 1), 1)


def stream_window(stats_path: Path) -> tuple[float, float]:
    first = last = None
    for line in stats_path.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        line_data = json.loads(line)
        if line_data.get("event") == "summary" or not line_data.get("streams"):
            continue
        elapsed = line_data["elapsed_s"]
        first = elapsed if first is None else min(first, elapsed)
        last = elapsed if last is None else max(last, elapsed)
    ensure(first is not None, "center never received a stream")
    return float(first), float(last)


def analyze(wav: Path, stats: Path, freqs: list[float], label: str) -> tuple[dict, float, float]:
    duration = wav_duration(wav)
    first, last = stream_window(stats)
    span = max(0.0, last - first)
    # Use the middle of the stream: startup prebuffering and the tail after a
    # sender stops are not representative of steady-state playback.
    start = first + span * 0.25
    end = last - span * 0.25
    if end - start > 3.0:
        middle = (start + end) * 0.5
        start, end = middle - 1.5, middle + 1.5
    start = max(0.0, start)
    end = min(duration, end)
    ensure(end - start >= 1.0, f"stream window too short ({end - start:.2f}s)")

    cmd = [
        str(BIN),
        "analyze",
        str(wav),
        "--presence-db",
        "-45",
        "--skip-start",
        f"{start:.3f}",
        "--skip-end",
        f"{duration - end:.3f}",
        "--json",
    ]
    for freq in freqs:
        cmd += ["--freq", str(freq)]
    result = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, timeout=120)
    ensure(result.returncode == 0, f"analyze failed: {result.stderr.strip()}")
    data = json.loads(result.stdout)
    (OUT / f"{label}_analysis.json").write_text(json.dumps(data, indent=2), encoding="utf-8")
    return data, start, end


def load_stats(stats: Path) -> tuple[dict | None, list[dict]]:
    lines = [json.loads(line) for line in stats.read_text(encoding="utf-8").splitlines() if line.strip()]
    summary = next((line for line in reversed(lines) if line.get("event") == "summary"), None)
    per_stream = [line for line in lines if line.get("streams") and "event" not in line]
    return summary, per_stream


def check_stream_health(per_stream: list[dict], window: tuple[float, float]) -> None:
    first, last = window
    for line in per_stream:
        elapsed = line["elapsed_s"]
        # Ignore the first second after join: startup prebuffering may hiccup.
        if elapsed < first + 1.0 or elapsed > last + 0.1:
            continue
        for stream in line["streams"]:
            ensure(stream["underruns"] == 0, f"{stream['label']} underran at {elapsed:.1f}s")
            ensure(stream["lost_frames"] == 0, f"{stream['label']} lost frames at {elapsed:.1f}s")


def check_tones(analysis: dict, expected: list[float], present: list[float]) -> None:
    by_freq = {tone["freq_hz"]: tone for tone in analysis["tones"]}
    for freq in present:
        tone = by_freq[freq]
        ensure(tone["present"], f"{freq} Hz missing (tone level {tone['db']:.1f} dBFS)")
        ensure(tone["amplitude"] > 0.15, f"{freq} Hz too weak ({tone['amplitude']:.3f})")
    for freq in expected:
        if freq not in present:
            ensure(not by_freq[freq]["present"], f"unexpected {freq} Hz content")


def record_test(name: str, label: str, port: int, seconds: int, freqs: list[float],
                present: list[float], sources: list[tuple[str, list[str]]],
                start_delay: float = 0.8) -> dict:
    OUT.mkdir(parents=True, exist_ok=True)
    wav = OUT / f"{label}.wav"
    stats = OUT / f"{label}_stats.jsonl"
    for path in (wav, stats):
        path.unlink(missing_ok=True)

    center = start(
        [
            "--log", "warn", "center",
            "--bind", "127.0.0.1", "--port", str(port), "--no-discover",
            "--record", str(wav), "--duration", str(seconds),
            "--stats-json", str(stats),
        ],
        f"{label}_center",
    )
    time.sleep(start_delay)
    senders = []
    for index, (src_name, extra) in enumerate(sources):
        summary = OUT / f"{label}_send_{index}.json"
        summary.unlink(missing_ok=True)
        senders.append(start(
            ["--log", "warn", "send", "--to", f"127.0.0.1:{port}", *extra, "--json"],
            f"{label}_{src_name}",
            stdout_to=summary,
        ))

    for sender in senders:
        ensure(wait(sender, seconds + 10) == 0, "sender failed")
    ensure(wait(center, seconds + 20) == 0, "center failed")

    analysis, window_start, window_end = analyze(wav, stats, freqs, label)
    check_tones(analysis, expected=freqs, present=present)
    summary, per_stream = load_stats(stats)
    ensure(summary is not None, "center summary missing")
    check_stream_health(per_stream, (window_start, window_end))
    ensure(summary["master"]["peak_hold_db"] > -30.0, "mix is silent")
    total_resets = max((s["buffer_resets"] for s in summary["streams"]), default=0)
    ensure(total_resets <= 2, f"too many buffer resets ({total_resets})")
    ensure(analysis["silent_blocks"] <= 4, f"too many silent blocks ({analysis['silent_blocks']})")

    for index in range(len(sources)):
        send_summary = json.loads((OUT / f"{label}_send_{index}.json").read_text(encoding="utf-8"))
        ensure(send_summary["packets"] > 100, "sender sent too few packets")
        REPORT["tests"][name]["senders"].append(
            {"packets": send_summary["packets"], "kbps": send_summary["kbps"]}
        )

    REPORT["tests"][name].update(
        {
            "window_s": [round(window_start, 2), round(window_end, 2)],
            "rms_db": analysis["rms_db"],
            "peak_db": analysis["peak_db"],
            "silent_blocks": analysis["silent_blocks"],
            "tones": [{"hz": t["freq_hz"], "db": t["db"], "amplitude": t["amplitude"]} for t in analysis["tones"]],
            "streams": summary["streams"],
            "ok": True,
        }
    )
    print(f"PASS {name}: {analysis['rms_db']:.1f} dBFS, window {window_start:.1f}-{window_end:.1f}s")
    return analysis


def run_raw_and_opus(args: argparse.Namespace) -> None:
    REPORT["tests"]["raw_mix"] = {"senders": []}
    mark = len(PROCS)
    try:
        record_test(
            "raw_mix", "raw", 48001, 9,
            freqs=[440.0, 660.0, 1000.0], present=[440.0, 660.0],
            sources=[
                ("440", ["--source", "tone", "--freq", "440", "--codec", "raw", "--duration", "6.5"]),
                ("660", ["--source", "tone", "--freq", "660", "--codec", "raw", "--duration", "6.5"]),
            ],
        )
    except TestFailure as error:
        terminate_from(mark)
        REPORT["tests"]["raw_mix"] = {"ok": False, "error": str(error)}
        REPORT["ok"] = False
        print(f"FAIL raw_mix: {error}")

    REPORT["tests"]["opus_mix"] = {"senders": []}
    mark = len(PROCS)
    try:
        record_test(
            "opus_mix", "opus", 48002, 9,
            freqs=[1000.0, 1500.0, 440.0], present=[1000.0, 1500.0],
            sources=[
                ("1000", ["--source", "tone", "--freq", "1000", "--codec", "opus", "--duration", "6.5"]),
                ("1500", ["--source", "tone", "--freq", "1500", "--codec", "opus", "--duration", "6.5"]),
            ],
        )
    except TestFailure as error:
        terminate_from(mark)
        REPORT["tests"]["opus_mix"] = {"ok": False, "error": str(error)}
        REPORT["ok"] = False
        print(f"FAIL opus_mix: {error}")


def run_loopback(args: argparse.Namespace) -> None:
    if sys.platform != "win32" or args.skip_loopback:
        print("SKIP loopback (not Windows or --skip-loopback)")
        return
    REPORT["tests"]["loopback"] = {}
    mark = len(PROCS)
    label = "loopback"
    port = 48003
    wav = OUT / f"{label}.wav"
    stats = OUT / f"{label}_stats.jsonl"
    for path in (wav, stats):
        path.unlink(missing_ok=True)
    try:
        center = start(
            [
                "--log", "warn", "center",
                "--bind", "127.0.0.1", "--port", str(port), "--no-discover",
                "--record", str(wav), "--duration", "11", "--stats-json", str(stats),
            ],
            f"{label}_center",
        )
        tone = start(
            ["--log", "warn", "playtone", "--freq", "1000", "--amplitude", "0.3", "--duration", "8"],
            f"{label}_playtone",
        )
        time.sleep(1.0)
        sender = start(
            ["--log", "warn", "send", "--to", f"127.0.0.1:{port}", "--source", "loopback",
             "--codec", "raw", "--duration", "7", "--json"],
            f"{label}_send",
            stdout_to=OUT / f"{label}_send.json",
        )
        ensure(wait(sender, 20) == 0, "loopback sender failed")
        ensure(wait(tone, 20) == 0, "playtone failed")
        ensure(wait(center, 30) == 0, "center failed")

        analysis, start_s, end_s = analyze(wav, stats, [1000.0, 440.0], label)
        check_tones(analysis, expected=[1000.0, 440.0], present=[1000.0])
        summary, per_stream = load_stats(stats)
        check_stream_health(per_stream, (start_s, end_s))
        send_summary = json.loads((OUT / f"{label}_send.json").read_text(encoding="utf-8"))
        capture = send_summary["capture"]
        ensure(capture is not None, "no capture stats")
        ensure(capture["dropped_frames"] == 0, "loopback capture dropped frames")
        ensure(capture["read_errors"] == 0, "loopback capture had read errors")
        REPORT["tests"]["loopback"] = {
            "ok": True,
            "capture": capture,
            "tone": analysis["tones"][0],
            "window_s": [round(start_s, 2), round(end_s, 2)],
        }
        print(f"PASS loopback: 1000 Hz at {analysis['tones'][0]['db']:.1f} dBFS, no capture drops")
    except TestFailure as error:
        terminate_from(mark)
        REPORT["tests"]["loopback"] = {"ok": False, "error": str(error)}
        REPORT["ok"] = False
        print(f"FAIL loopback: {error}")


def run_discovery(args: argparse.Namespace) -> None:
    if args.skip_discovery:
        print("SKIP discovery")
        return
    REPORT["tests"]["discovery"] = {}
    mark = len(PROCS)
    label = "discovery"
    port = 48010
    wav = OUT / f"{label}.wav"
    stats = OUT / f"{label}_stats.jsonl"
    for path in (wav, stats):
        path.unlink(missing_ok=True)
    try:
        center = start(
            [
                "--log", "info", "center",
                "--bind", "0.0.0.0", "--port", str(port), "--name", "E2E Discover",
                "--record", str(wav), "--duration", "16", "--stats-json", str(stats),
            ],
            f"{label}_center",
        )
        time.sleep(2.0)
        result = subprocess.run(
            [str(BIN), "--log", "warn", "discover", "--seconds", "4", "--json"],
            cwd=ROOT, capture_output=True, text=True, timeout=30,
        )
        ensure(result.returncode == 0, f"discover failed: {result.stderr.strip()}")
        peers = json.loads(result.stdout)
        center_peer = next((p for p in peers if p["role"] == "center" and p["port"] == port), None)
        ensure(center_peer is not None, "center not found over mDNS")

        sender = start(
            ["--log", "info", "send", "--to", "auto", "--source", "tone", "--freq", "880",
             "--codec", "raw", "--duration", "6", "--json"],
            f"{label}_send",
            stdout_to=OUT / f"{label}_send.json",
        )
        ensure(wait(sender, 25) == 0, "auto-connect sender failed")
        ensure(wait(center, 35) == 0, "center failed")

        analysis, start_s, end_s = analyze(wav, stats, [880.0], label)
        check_tones(analysis, expected=[880.0], present=[880.0])
        summary, per_stream = load_stats(stats)
        check_stream_health(per_stream, (start_s, end_s))
        REPORT["tests"]["discovery"] = {
            "ok": True,
            "peer": center_peer,
            "tone": analysis["tones"][0],
            "window_s": [round(start_s, 2), round(end_s, 2)],
        }
        print(f"PASS discovery: found {center_peer['addr']}:{center_peer['port']}, 880 Hz mixed")
    except (TestFailure, subprocess.TimeoutExpired, json.JSONDecodeError) as error:
        terminate_from(mark)
        REPORT["tests"]["discovery"] = {"ok": False, "error": str(error)}
        REPORT["ok"] = False
        print(f"FAIL discovery: {error}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-loopback", action="store_true")
    parser.add_argument("--skip-discovery", action="store_true")
    args = parser.parse_args()

    if not BIN.exists():
        print(f"missing {BIN}; run scripts/build.ps1 first", file=sys.stderr)
        return 2
    OUT.mkdir(parents=True, exist_ok=True)
    LOGS.mkdir(parents=True, exist_ok=True)

    try:
        run_raw_and_opus(args)
        run_loopback(args)
        run_discovery(args)
    finally:
        for proc in PROCS:
            if proc.poll() is None:
                proc.terminate()
        REPORT["finished_at"] = time.strftime("%Y-%m-%dT%H:%M:%S")
        (OUT / "report.json").write_text(json.dumps(REPORT, indent=2), encoding="utf-8")
        print(f"\nreport: {OUT / 'report.json'}")

    return 0 if REPORT["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
