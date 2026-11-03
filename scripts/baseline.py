#!/usr/bin/env python3
"""One-command baseline capture (PLAN P0-4 / F10 acceptance).

Runs the netem fault matrix on the real Wi-Fi path (PC leaf <-> NAS hub) via
verify_scenarios.py, then archives the tables under docs/baselines/<date>/
together with the environment facts. Every later fix is compared against the
most recent archived baseline.

The phone<->PC leg needs the phone app to join and is not automated yet;
run the same matrix with the phone as the leaf once the Android side can be
driven headlessly (tracked as Phase 0 leftover).

Usage:
    python scripts/baseline.py --password <pw>
    python scripts/baseline.py --password <pw> --runs 5 --scenarios clean,loss10
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "target" / "e2e" / "scenarios"
BASELINES = ROOT / "docs" / "baselines"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--password", required=True)
    parser.add_argument("--host", default="kelierandes@192.168.2.196")
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--seconds", type=int, default=8)
    parser.add_argument("--scenarios", default="clean,loss2,loss10,burst,delay200,jitter50,bw1800")
    parser.add_argument("--label", default=None, help="archive directory name; default YYYY-MM-DD")
    args = parser.parse_args()

    started = time.strftime("%Y-%m-%d %H:%M")
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "verify_scenarios.py"),
            "--password", args.password,
            "--host", args.host,
            "--runs", str(args.runs),
            "--seconds", str(args.seconds),
            "--scenarios", args.scenarios,
        ],
        cwd=ROOT,
    )
    if result.returncode != 0:
        print("matrix failed; nothing archived", file=sys.stderr)
        return result.returncode

    stamp = args.label or time.strftime("%Y-%m-%d")
    target = BASELINES / stamp
    target.mkdir(parents=True, exist_ok=True)

    # Environment facts: binary version + git head.
    version = ""
    try:
        out = subprocess.run(
            [str(ROOT / "target" / "release" / "siren.exe"), "--version"],
            capture_output=True, text=True, timeout=10,
        )
        version = out.stdout.strip() or out.stderr.strip()
    except Exception:
        pass
    head = ""
    try:
        head = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"], cwd=ROOT,
            capture_output=True, text=True, timeout=10,
        ).stdout.strip()
    except Exception:
        pass

    for name in ("report.md", "results.csv"):
        source = OUT / name
        if source.exists():
            (target / f"leaf-hub-{name}").write_text(
                source.read_text(encoding="utf-8"), encoding="utf-8"
            )

    header = [
        "# SirenStudio 基线存档 — leaf(hub NAS) ↔ hub(PC leaf) over real Wi-Fi",
        "",
        f"- 时间：{started}",
        f"- 二进制：{version or 'unknown'}（git {head or 'dirty'}）",
        f"- 场景：{args.scenarios}，每场景 {args.runs} 次 × {args.seconds}s，netem 双向",
        "- 指标：丢包%（接收端 gap 计数 ÷240 折包）、心跳 RTT P99（500 ms 心跳）、"
        "咔哒（样本阶跃检测）、欠载、修剪（接收环溢出）、缓冲重置（切换/重同步代理）",
        "",
    ]
    (target / "README.md").write_text(
        "\n".join(header)
        + (OUT / "report.md").read_text(encoding="utf-8"),
        encoding="utf-8",
    )
    print(f"archived to {target}")
    print((target / "README.md").read_text(encoding="utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
