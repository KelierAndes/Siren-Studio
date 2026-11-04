#!/usr/bin/env python3
"""把 siren 的 trace 日志行解析成数据采集计划要的 CSV。

识别三类插桩行（I1 修剪 / I2 载波切换 / I3 零值游程）以及 analyze_gaps.py 的
JSON 输出。所有 CSV 第一列统一为 ts_ms（epoch 毫秒，UTC）。

    python scripts/parse_traces.py siren.log --outdir T1 [--gaps gaps.json]
"""

from __future__ import annotations

import argparse
import csv
import json
import re
import sys
from datetime import datetime, timezone

STAMP = re.compile(r"\[(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})\.(\d{3})Z")

PATTERNS = {
    "trims": (
        "ts_ms,stream,fill_before_ms,fill_after_ms,target_ms,ms_since_last_trim,ratio,path,dropped_ms",
        re.compile(
            r"trace: trim stream=(?P<stream>\S+) fill_before_ms=(?P<before>[\d.]+) "
            r"fill_after_ms=(?P<after>[\d.]+) target_ms=(?P<target>[\d.]+) "
            r"ms_since_last_trim=(?P<since>[-\d.NA]+) ratio=(?P<ratio>[\d.]+) "
            r"path=(?P<path>\S+) dropped_ms=(?P<dropped>[\d.]+)"
        ),
        ["stream", "before", "after", "target", "since", "ratio", "path", "dropped"],
    ),
    "switches": (
        "ts_ms,stream,from,to,no_packet_ms,synth_ms,refill_ms,total_mute_ms,target_ms,kind",
        re.compile(
            r"udp: stream (?P<stream>\S+) switch (?P<from>\w+) -> (?P<to>\w+) measured: "
            r"(?P<no_packet>[\d.]+) ms with no packet at all, (?P<synth>[\d.]+) ms synthesized "
            r"as silence, then (?P<refill>[\d.]+) ms re-filling; total mute (?P<total>[\d.]+) ms "
            r"\(target (?P<target>[\d.]+) ms\)"
        ),
        ["stream", "from", "to", "no_packet", "synth", "refill", "total", "target"],
    ),
    "zeroruns": (
        "ts_ms,point,run_ms",
        re.compile(r"trace: zerorun point=(?P<point>\S+) run_ms=(?P<run>[\d.]+)"),
        ["point", "run"],
    ),
    "mutes": (
        "ts_ms,stream,gap_ms,accepted_on,previous_carrier",
        re.compile(
            r"udp: stream (?P<stream>\S+) audible gap (?P<gap>[\d.]+) ms before this "
            r"(?P<carrier>\w+) packet \(previous carrier (?P<previous>\w+)\)"
        ),
        ["stream", "gap", "carrier", "previous"],
    ),
}


def epoch_ms(stamp: re.Match) -> int:
    base = datetime.strptime(stamp.group(1), "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc)
    return int(base.timestamp() * 1000) + int(stamp.group(2))


def parse_log(path: str, outdir: str) -> dict:
    counts = {}
    files = {}
    for name in PATTERNS:
        header, _, _ = PATTERNS[name]
        target = f"{outdir}/{name}.csv"
        files[name] = open(target, "w", newline="", encoding="utf-8")
        files[name].write(header + "\n")
        counts[name] = 0
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            stamp = STAMP.search(line)
            ts = epoch_ms(stamp) if stamp else 0
            for name, (_, pattern, fields) in PATTERNS.items():
                match = pattern.search(line)
                if not match:
                    continue
                row = [ts] + [match.group(key) for key in fields]
                if name == "switches":
                    row.append("measured")
                files[name].write(",".join(str(value) for value in row) + "\n")
                counts[name] += 1
                break
    for handle in files.values():
        handle.close()
    return counts


def gaps_to_csv(payload: bytes, outdir: str) -> int:
    if isinstance(payload, (bytes, bytearray)):
        text = payload.decode("utf-8", errors="replace")
        if "\ufffd" in text:
            text = payload.decode("gbk", errors="replace")
    else:
        text = payload
    data = json.loads(text)
    with open(f"{outdir}/gaps.csv", "w", newline="", encoding="utf-8") as out:
        out.write("ts_ms,start_s,duration_ms,kind,depth_dbfs,step,cause,explained,streams\n")
        for event in data.get("list", []):
            out.write(
                ",".join(
                    [
                        str(int(event["start"] * 1000)),
                        f"{event['start']:.3f}",
                        f"{event['ms']:.1f}",
                        event["kind"],
                        f"{event['depth_db']:.1f}",
                        f"{event['step']:.3f}",
                        (event["cause"] or "").replace(",", ";"),
                        "1" if event["explained"] else "0",
                        "; ".join(event.get("streams", [])).replace(",", ";"),
                    ]
                )
                + "\n"
            )
    return len(data.get("list", []))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log")
    parser.add_argument("--outdir", required=True)
    parser.add_argument("--gaps", help="analyze_gaps.py --json 的输出文件")
    args = parser.parse_args()
    counts = parse_log(args.log, args.outdir)
    rows = 0
    if args.gaps:
        rows = gaps_to_csv(open(args.gaps, encoding="utf-8").read(), args.outdir)
    print(json.dumps({"log": counts, "gaps_rows": rows}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
