#!/usr/bin/env python3
"""把 matrix_dualstack.py 的产物汇总成双栈/保活报告。

只用同一台机器内部的时间戳做因果判定：恢复时长取"故障解除后最后一条
audible gap 的数值"（该数值本身就是接收端自己量到的空洞长度），不做跨机亚秒对齐。

    python scripts/summarise_matrix.py out/matrix_2026-11-05_1152
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

GAP = re.compile(r"audible gap ([\d.]+) ms before this (\w+) packet")
SWITCH = re.compile(r"switch (\w+) -> (\w+)")
EVIDENCE = re.compile(r"udp: stream \S+ (\w+) seq \d+ pkts \d+ lost (\d+) resets \d+ "
                      r"underruns (\d+) buffer ([\d.]+) ms target ([\d.]+)")
BT_STATE = re.compile(r"bt: (\w[\w -]{0,40})")


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}


def timeline(path: Path) -> list[dict]:
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()]


def node_view(directory: Path, which: str) -> dict:
    """从节点自己的日志里取：切换方向、音频洞列表、末水位。"""
    name = "pc.slice.log" if which == "pc" else "phone.logcat.txt"
    path = directory / name
    if not path.exists():
        return {}
    text = path.read_text(encoding="utf-8", errors="replace")
    gaps = [(float(m.group(1)), m.group(2)) for m in GAP.finditer(text)]
    dirs: dict[str, int] = {}
    for m in SWITCH.finditer(text):
        key = f"{m.group(1)}→{m.group(2)}"
        dirs[key] = dirs.get(key, 0) + 1
    last: dict[str, tuple] = {}
    for m in EVIDENCE.finditer(text):
        last[m.group(0)[:20]] = (m.group(1), int(m.group(2)), int(m.group(3)),
                                 float(m.group(4)))
    return {
        "gaps": gaps,
        "gap_over_500": [g for g in gaps if g[0] > 500],
        "switch_dirs": dirs,
        "carriers": sorted({v[0] for v in last.values()}),
        "buffers": [round(v[3]) for v in last.values()],
        "underruns": sum(v[2] for v in last.values()),
        "lost_frames": sum(v[1] for v in last.values()),
        "bt_mentions": sorted({m.group(1).strip() for m in BT_STATE.finditer(text)})[:6],
    }


def verdict(row: dict) -> str:
    """保活判定：断链窗口里有没有 >500 ms 的可听洞。"""
    worst = max([g[0] for g in row.get("pc", {}).get("gaps", [])]
                + [g[0] for g in row.get("phone", {}).get("gaps", [])], default=0)
    if row.get("fault") is None:
        return "基线"
    if worst <= 200:
        return "蓝牙接管成功（无可听洞）"
    if worst <= 1500:
        return f"接管但有洞（最长 {worst:.0f} ms）"
    return f"未接管/断音（最长 {worst:.0f} ms）"


def main() -> int:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "out")
    scenarios = sorted(p for p in out.iterdir() if (p / "metrics.json").exists())
    if not scenarios:
        print("没有场景目录", file=sys.stderr)
        return 1

    lines = ["# 双栈切换与音频保活矩阵报告\n"]
    meta = (out / "meta.txt")
    if meta.exists():
        lines.append("```\n" + meta.read_text(encoding="utf-8").strip() + "\n```\n")
    lines.append("\n## 1. 场景与判定\n")
    lines.append("| 场景 | 拓扑 | 故障 | 载波切换（手机/PC） | 断链期最长音频洞 | 恢复时长 | 判定 |\n")
    lines.append("| :-- | :-- | :-- | :-- | --: | --: | :-- |\n")
    rows = []
    for directory in scenarios:
        metrics = read_json(directory / "metrics.json")
        marks = timeline(directory / "timeline.jsonl")
        fault = next((m.get("fault") for m in marks if m["action"] == "begin"), None)
        topology = next((m.get("topology") for m in marks if m["action"] == "begin"), "?")
        pc, phone = node_view(directory, "pc"), node_view(directory, "phone")
        gaps = pc.get("gaps", []) + phone.get("gaps", [])
        worst = max([g[0] for g in gaps], default=0)
        # 恢复时长：故障解除之后仍然出现的最后一段音频洞（同一节点时钟内自洽）
        clear = next((m["ts"] for m in marks if m["action"] == "fault_off"), None)
        recovery = max([g[0] for g in gaps if g[0] > 200], default=0) if clear else 0
        dirs = {**pc.get("switch_dirs", {}), **{k: v for k, v in phone.get("switch_dirs", {}).items()}}
        row = {
            "name": directory.name, "topology": topology, "fault": fault,
            "switch_dirs": dirs, "worst_gap_ms": worst, "recovery_ms": recovery,
            "pc": pc, "phone": phone, "nas": metrics.get("nas", {}),
        }
        row["verdict"] = verdict(row)
        rows.append(row)
        lines.append(
            f"| {directory.name} | {topology} | {fault or '无'} | "
            f"手机 {sum(phone.get('switch_dirs', {}).values())} · PC {sum(pc.get('switch_dirs', {}).values())} "
            f"| {worst:.0f} ms | {recovery:.0f} ms | {row['verdict']} |\n")

    lines.append("\n## 2. 切换方向与载波\n")
    lines.append("| 场景 | 切换方向计数 | 手机见过的载波 | PC 见过的载波 | 手机末水位 ms | PC 末水位 ms |\n")
    lines.append("| :-- | :-- | :-- | :-- | :-- | :-- |\n")
    for row in rows:
        lines.append(
            f"| {row['name']} | {row['switch_dirs'] or '—'} | {row['phone'].get('carriers')} | "
            f"{row['pc'].get('carriers')} | {row['phone'].get('buffers')} | {row['pc'].get('buffers')} |\n")

    lines.append("\n## 3. NAS 侧（三端场景）\n")
    lines.append("| 场景 | 流数 | 水位 ms | 欠载Δ | 丢帧Δ | 无声 msΔ | 咔哒Δ | bt.state | bt.role | bt.links 峰值 | RTT ms | 轮询失败 |\n")
    lines.append("| :-- | --: | :-- | --: | --: | --: | --: | :-- | :-- | --: | :-- | --: |\n")
    for row in rows:
        nas = row["nas"] or {}
        if not nas:
            lines.append(f"| {row['name']} | — 两设备场景，守护已停 — |\n")
            continue
        lines.append(
            f"| {row['name']} | {nas.get('streams')} | {nas.get('buffer_ms')} | "
            f"{nas.get('underruns_delta')} | {nas.get('lost_frames_delta')} | "
            f"{nas.get('silent_ms_delta')} | {nas.get('clicks_delta')} | {nas.get('bt_state_end')} | "
            f"{nas.get('bt_role_end')} | {nas.get('bt_links_max')} | {nas.get('rtt_ms')} | "
            f"{nas.get('unreachable_polls')} |\n")

    lines.append("\n## 4. 蓝牙事件（节点日志里的 bt 行为）\n")
    for row in rows:
        lines.append(f"- **{row['name']}** 手机：{row['phone'].get('bt_mentions')} · "
                     f"PC：{row['pc'].get('bt_mentions')}\n")

    text = "".join(lines)
    (out / "matrix_report.md").write_text(text, encoding="utf-8")
    (out / "matrix_rows.json").write_text(json.dumps(rows, ensure_ascii=False, indent=2),
                                          encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
