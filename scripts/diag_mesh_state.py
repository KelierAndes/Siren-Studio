#!/usr/bin/env python3
"""三端链路状态对照快照：把每个节点"自己看到的"收发关系打在一张表里。

只用各节点自己的日志/接口，不做跨机时间对齐。用途：定位"两端对同一条链路
说法不一致""某些节点收不到混音"这类问题。

    python scripts/diag_mesh_state.py --label A            # 健康态
    python scripts/diag_mesh_state.py --label B --phone-wifi off
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import collect_1h as harness  # noqa: E402

PC_LOG = REPO / "ui-flutter/build/windows/x64/runner/Release/siren_ui.log"
ADB = harness.ADB
SERIAL = harness.SERIAL
PHONE_PKG = harness.PHONE_PKG

EVIDENCE = re.compile(
    r"udp: stream (\S{0,12})\S* (\w+) seq \d+ pkts (\d+) lost (\d+) resets (\d+) "
    r"underruns (\d+) buffer ([\d.]+) ms target ([\d.]+)"
)
DISCOVERED = re.compile(r"mesh: discovered (\S+) \((.*?)\) bit (\S+) bt (\S+)")
SENDING = re.compile(r"mesh: sending to (.*) \(")
TX_LEVEL = re.compile(r"tx level [-\d.]+ dB \((\d+) Wi-Fi link\(s\), (\d+) packet")
BT_LINK = re.compile(r"bt: link quality: (\d+) link\(s\), tx ([\d.]+) KB/s, rx ([\d.]+) KB/s")
JOINED = re.compile(r"stream (\S+) from ([0-9.:]+) \((\w+), origin ([0-9a-f]+)\) joined")


def tail(path: Path, limit: int = 4000) -> str:
    if not path.exists():
        return ""
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    return "\n".join(lines[-limit:])


def phone_log(lines: int = 4000) -> str:
    out = subprocess.run([ADB, "-s", SERIAL, "logcat", "-d", "-t", str(lines)],
                         capture_output=True, text=True, timeout=120, errors="replace")
    return out.stdout or ""


def nas_status() -> dict:
    text = harness.nas("curl -s -m 4 localhost:8090/api/status")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return {"raw": text[:200]}


def view(label: str, text: str) -> dict:
    streams: dict[str, tuple] = {}
    for m in EVIDENCE.finditer(text):
        streams[m.group(1)] = m.groups()[1:]
    joined = [(s, addr, codec, origin) for s, addr, codec, origin in JOINED.findall(text)]
    return {
        "label": label,
        "received_streams": {k: {"carrier": v[0], "pkts": int(v[1]), "lost": int(v[2]),
                                 "underruns": int(v[4]), "buffer_ms": float(v[5])}
                             for k, v in sorted(streams.items())},
        "joined": joined[-8:],
        "discovered": sorted({(name, bt) for _addr, name, _bit, bt in DISCOVERED.findall(text)}),
        "sending_to": sorted({m.group(1) for m in SENDING.finditer(text)}),
        "tx_level": TX_LEVEL.findall(text)[-1:],
        "bt_link": BT_LINK.findall(text)[-1:],
    }


def nas_view(status: dict) -> dict:
    snap = status.get("snapshot") or {}
    center = snap.get("center") or {}
    streams = {
        (s.get("label") or "")[:12]: {
            "carrier": s.get("path") or s.get("carrier"),
            "pkts": s.get("packets"),
            "lost": s.get("lost_frames"),
            "underruns": s.get("underruns"),
            "buffer_ms": round(s.get("buffer_ms", 0), 1),
            "codec": s.get("codec"),
        }
        for s in center.get("streams", []) or []
    }
    bt = snap.get("bt") or {}
    return {
        "label": "NAS",
        "running": status.get("running"),
        "error": status.get("error"),
        "received_streams": streams,
        "peers": [{"name": p.get("name"), "bit": p.get("bit"), "bt": p.get("bt"),
                   "rtt_ms": p.get("rtt_ms"), "loss_pct": p.get("loss_pct")}
                  for p in snap.get("peers", []) or []],
        "topology": snap.get("topology"),
        "bt": {"state": bt.get("state"), "role": bt.get("role"), "links": bt.get("links"),
               "tx_kb": bt.get("tx_bytes"), "rx_kb_per_s": bt.get("rx_kb_per_s"),
               "gap_p99_ms": bt.get("gap_p99_ms")},
        "master": center.get("master"),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", default="A")
    parser.add_argument("--phone-wifi", choices=["on", "off", "keep"], default="keep")
    parser.add_argument("--wait", type=float, default=0, help="改完链路后等多少秒再采样")
    parser.add_argument("--out", default=str(REPO / "out" / "diag"))
    args = parser.parse_args()
    if not harness.NAS_PASS:
        print("需要 SIREN_NAS_PASS", file=sys.stderr)
        return 2

    if args.phone_wifi != "keep":
        harness.adb_shell(f"svc wifi {'enable' if args.phone_wifi == 'on' else 'disable'}")
        time.sleep(max(0, args.wait))
    elif args.wait:
        time.sleep(args.wait)

    report = {
        "label": args.label,
        "ts": round(time.time(), 1),
        "phone_wifi": harness.adb_shell("dumpsys wifi | grep -m1 -oE 'Wi-Fi is (enabled|disabled)'").strip(),
        "pc": view("PC", tail(PC_LOG)),
        "phone": view("手机", phone_log()),
        "nas": nas_view(nas_status()),
    }
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    path = out / f"state_{args.label}.json"
    path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
