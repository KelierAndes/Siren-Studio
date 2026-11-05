#!/usr/bin/env python3
"""把 2h 测试的 cases.csv 汇总成 SUMMARY.md（按阶段分组 + 判定统计 + 已知开放项）。"""
from __future__ import annotations

import argparse
import csv
import json
from collections import Counter, defaultdict
from datetime import datetime
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dir", required=True, help="e2e 产物目录（含 cases.csv）")
    args = parser.parse_args()
    directory = Path(args.dir)
    cases_file = directory / "cases.csv"
    if not cases_file.exists():
        print(f"找不到 {cases_file}")
        return 1
    with open(cases_file, encoding="utf-8") as handle:
        rows = list(csv.DictReader(handle))

    counts = Counter(row["status"] for row in rows)
    by_phase = defaultdict(list)
    for row in rows:
        by_phase[row["phase"]].append(row)

    lines = [
        f"# 2h 全链路测试汇总（{datetime.now():%Y-%m-%d %H:%M}）",
        "",
        f"- 用例数：{len(rows)}　" + "　".join(f"{k} {v}" for k, v in sorted(counts.items())),
        "- 判定口径：PASS/FAIL 为实测；KNOWN-OPEN 是计划 §0.1 的已知开放项（不劣化即通过）；"
        "SKIP/N-A 是缺能力或不在本轮范围，原因逐条写在表内。",
        "",
    ]
    order = ["P0", "P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8"]
    for phase in order:
        group = by_phase.get(phase)
        if not group:
            continue
        lines += [f"## {phase}", "", "| ID | 用例 | 状态 | 秒 | 关键数字 / 备注 |",
                  "| :-- | :-- | :-- | --: | :-- |"]
        for row in group:
            note = (row.get("notes") or "").replace("|", "/")
            lines.append(f"| {row['id']} | {row['case']} | {row['status']} | "
                         f"{row['seconds']} | {note[:180]} |")
        lines.append("")
    (directory / "SUMMARY.md").write_text("\n".join(lines) + "\n", encoding="utf-8")

    stats = directory / "SUMMARY.stats.json"
    stats.write_text(json.dumps({"counts": counts, "cases": len(rows)},
                                ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"SUMMARY.md 已写入 {directory}（{len(rows)} 个用例：{dict(counts)}）")
    return 0


if __name__ == "__main__":
    import sys
    sys.exit(main())
