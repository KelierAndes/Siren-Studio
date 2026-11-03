#!/usr/bin/env python3
"""Fault-matrix driver for the real Wi-Fi path (F9 / PLAN P0-2).

Runs the scenario matrix from the Windows PC against the Linux test host:
applies `tc netem` profiles on the host (both directions), starts one mesh
node on each side, and collects per-stream stats from the JSONL snapshots.
Each fault is exercised `--runs` times (default 10) and the results are
aggregated into a CSV plus a markdown baseline table under target/e2e/scenarios/.

Prerequisites on the host: /usr/local/bin/siren installed, sudo via password,
and netem profiles available (scripts/linux/netem.sh is copied over on first
run). SSH uses plink (PuTTY) with a pinned host key.

Usage:
    python scripts/verify_scenarios.py --password <pw> [--host kelierandes@192.168.2.196]
    python scripts/verify_scenarios.py --password <pw> --runs 3 --scenarios clean,loss10
"""

from __future__ import annotations

import argparse
import base64
import csv
import json
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WIN_BIN = ROOT / "target" / "release" / "siren.exe"
OUT = ROOT / "target" / "e2e" / "scenarios"
LINUX_BIN = "/usr/local/bin/siren"
LINUX_TMP = "/tmp/siren_scenarios"
# StreamSnapshot.lost_frames counts sample frames; 5 ms frames = 240 per packet.
SAMPLE_FRAMES_PER_PACKET = 240

SCENARIOS = ["clean", "loss2", "loss10", "burst", "delay200", "jitter50", "bw1800"]

# The host usually runs a persistent `siren web` daemon; the matrix kills every
# siren process per run, so remember the daemon command line and bring it back
# at the end (restart recipe per DEVELOPMENT/device notes).
DAEMON_ENV = "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus"


def daemon_cmdline(host: str, password: str, hostkey: str | None) -> str | None:
    out = ssh(host, password, hostkey, "ps -C siren -o args=", timeout=20)
    for line in out.splitlines():
        line = line.strip()
        if line.startswith("siren web") or " web " in line:
            return line
    return None


def restore_daemon(host: str, password: str, hostkey: str | None, cmdline: str | None) -> None:
    if not cmdline:
        return
    ssh(
        host,
        password,
        hostkey,
        f"{DAEMON_ENV} setsid nohup {cmdline} >/dev/null 2>&1 & sleep 1",
        timeout=20,
    )
    print(f"restored host daemon: {cmdline}")


def ssh(host: str, password: str, hostkey: str | None, command: str, timeout: float = 30) -> str:
    """Run a command on the test host via plink; returns stdout."""
    cmd = ["plink", "-batch", "-ssh"]
    if hostkey:
        cmd += ["-hostkey", hostkey]
    cmd += ["-pw", password, host, command]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError(f"ssh {host} failed ({result.returncode}): {result.stderr.strip()}")
    return result.stdout


def last_summary(stats_path: Path) -> dict | None:
    """The mesh CLI appends snapshot lines to --stats-json; the summary event
    only goes to stdout. Take the last snapshot and lift its per-stream list
    from center.streams to the top level for uniform parsing."""
    lines = [json.loads(line) for line in stats_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    snapshot = next((line for line in reversed(lines) if line.get("center")), None)
    if snapshot is None:
        return None
    snapshot["streams"] = snapshot["center"].get("streams", [])
    return snapshot


def stream_metrics(summary: dict | None) -> list[dict]:
    out = []
    for stream in (summary or {}).get("streams", []):
        lost_packets = stream["lost_frames"] / SAMPLE_FRAMES_PER_PACKET
        packets = stream["packets"]
        out.append(
            {
                "label": stream["label"],
                "codec": stream["codec"],
                "path": stream.get("path", "?"),
                "packets": packets,
                "lost_packets": round(lost_packets, 1),
                "loss_pct": round(100.0 * lost_packets / max(1, packets + lost_packets), 2),
                "underruns": stream["underruns"],
                "buffer_resets": stream["buffer_resets"],
                "buffer_ms": round(stream["buffer_ms"], 1),
                "clicks": stream.get("clicks", 0),
                # 修剪 = 接收环溢出丢弃的整包
                "overflow_packets": stream.get("overflow_packets", 0),
                "underrun_frames": stream.get("underrun_frames", 0),
            }
        )
    return out


def peer_health(summary: dict | None) -> list[dict]:
    """Heartbeat path health per peer from the last snapshot (F10)."""
    out = []
    for peer in (summary or {}).get("peers", []):
        if peer.get("rtt_ms") is None:
            continue
        out.append(
            {
                "endpoint": peer.get("endpoint", "?"),
                "rtt_ms": round(float(peer["rtt_ms"]), 1),
                "rtt_p99_ms": round(float(peer.get("rtt_p99_ms") or 0), 1),
                "loss_pct": round(float(peer.get("loss_pct") or 0), 2),
            }
        )
    return out


def local_ip() -> str:
    """The LAN address this machine would use to reach the internet (no traffic)."""
    import socket

    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.connect(("8.8.8.8", 80))
    ip = sock.getsockname()[0]
    sock.close()
    return ip


def run_one(
    scenario: str,
    run: int,
    seconds: int,
    host: str,
    password: str,
    hostkey: str | None,
    pc_port: int,
    nas_port: int,
    nas_ip: str,
) -> list[dict]:
    OUT.mkdir(parents=True, exist_ok=True)
    pc_stats = OUT / f"{scenario}_{run:02d}_pc.jsonl"
    nas_stats_remote = f"{LINUX_TMP}/{scenario}_{run:02d}_nas.jsonl"
    pc_stats.unlink(missing_ok=True)

    # Clean slate on the host, then apply the profile to both directions.
    ssh(host, password, hostkey, "pkill -x siren 2>/dev/null; sleep 1; true", timeout=20)
    ssh(host, password, hostkey,
        f"echo '{password}' | sudo -S bash $HOME/scripts/linux/netem.sh apply {scenario}",
        timeout=30)

    pc_ip = local_ip()
    total = seconds + 6
    try:
        # Host node ("hub" in the leaf<->hub baseline naming).
        ssh(
            host,
            password,
            hostkey,
            f"rm -f {nas_stats_remote}; setsid nohup {LINUX_BIN} --log warn mesh "
            f"--no-discover --port {nas_port} --peer {pc_ip}:{pc_port} "
            f"--name nas-hub --duration {total} "
            f"--stats-json {nas_stats_remote} --stats-interval 1 "
            f">/dev/null 2>&1 &",
            timeout=20,
        )
        # PC node ("leaf").
        pc_log = open(OUT / f"{scenario}_{run:02d}_pc.log", "w", encoding="utf-8")
        pc_proc = subprocess.Popen(
            [
                str(WIN_BIN),
                "--log", "warn", "mesh",
                "--no-discover", "--port", str(pc_port),
                "--peer", f"{nas_ip}:{nas_port}",
                "--name", "pc-leaf", "--duration", str(total),
                "--stats-json", str(pc_stats), "--stats-interval", "1",
            ],
            stdout=pc_log,
            stderr=subprocess.STDOUT,
        )
        time.sleep(total)
        pc_rc = pc_proc.wait(timeout=20)
        pc_log.close()
        if pc_rc != 0 or not pc_stats.exists():
            raise RuntimeError(
                f"PC node failed (rc={pc_rc}); log: "
                f"{(OUT / f'{scenario}_{run:02d}_pc.log').read_text(encoding='utf-8')[:500]}"
            )
    finally:
        ssh(host, password, hostkey, "pkill -x siren 2>/dev/null; true", timeout=20)
        ssh(host, password, hostkey,
            f"echo '{password}' | sudo -S bash $HOME/scripts/linux/netem.sh clear",
            timeout=30)

    # Fetch the host-side stats.
    remote_text = ssh(host, password, hostkey, f"cat {nas_stats_remote} 2>/dev/null || true", timeout=20)
    nas_stats = OUT / f"{scenario}_{run:02d}_nas.jsonl"
    nas_stats.write_text(remote_text, encoding="utf-8")

    rows = []
    for side, path in (("nas->pc", pc_stats), ("pc->nas", nas_stats)):
        summary = last_summary(path)
        metrics = stream_metrics(summary)
        if not metrics:
            rows.append(
                {
                    "scenario": scenario,
                    "run": run,
                    "direction": side,
                    "error": "no stream seen",
                }
            )
            continue
        for stream in metrics:
            rows.append({"scenario": scenario, "run": run, "direction": side, **stream})
        health = peer_health(summary)
        if health:
            best = min(health, key=lambda h: h["rtt_p99_ms"])
            rows.append(
                {
                    "scenario": scenario,
                    "run": run,
                    "direction": f"{side} heartbeat",
                    "label": best["endpoint"],
                    "rtt_p99_ms": best["rtt_p99_ms"],
                    "hb_loss_pct": best["loss_pct"],
                }
            )
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="kelierandes@192.168.2.196",
                        help="user@host; the host part is used as the NAS audio peer")
    parser.add_argument("--password", required=True, help="SSH + sudo password for the host")
    parser.add_argument(
        "--hostkey",
        default="SHA256:xvTGpWJxeqqcQb/aPVpkOP6YTMfIq7sl8tWQ4iwTSRU",
        help="plink -hostkey fingerprint of the test host",
    )
    parser.add_argument("--runs", type=int, default=10)
    parser.add_argument("--seconds", type=int, default=8, help="measured window per run")
    parser.add_argument("--scenarios", default=",".join(SCENARIOS))
    parser.add_argument("--pc-port", type=int, default=47910)
    parser.add_argument("--nas-port", type=int, default=47901)
    args = parser.parse_args()

    scenarios = [s.strip() for s in args.scenarios.split(",") if s.strip()]
    nas_ip = args.host.rsplit("@", 1)[-1]
    unknown = [s for s in scenarios if s not in SCENARIOS]
    if unknown:
        print(f"unknown scenarios: {unknown}; known: {SCENARIOS}", file=sys.stderr)
        return 1

    daemon = daemon_cmdline(args.host, args.password, args.hostkey)
    if daemon:
        print(f"host runs a persistent daemon; it will be restored afterwards: {daemon}")

    # Ship netem.sh to the host (idempotent).
    script = (ROOT / "scripts" / "linux" / "netem.sh").read_text(encoding="utf-8")
    b64 = base64.b64encode(script.encode()).decode()
    ssh(args.host, args.password, args.hostkey,
        f"mkdir -p $HOME/scripts/linux {LINUX_TMP} && "
        f"echo {b64} | base64 -d > $HOME/scripts/linux/netem.sh && "
        f"chmod +x $HOME/scripts/linux/netem.sh")

    all_rows: list[dict] = []
    for scenario in scenarios:
        for run in range(1, args.runs + 1):
            print(f"[{time.strftime('%H:%M:%S')}] {scenario} run {run}/{args.runs} ...", flush=True)
            try:
                rows = run_one(scenario, run, args.seconds, args.host, args.password,
                               args.hostkey, args.pc_port, args.nas_port, nas_ip)
            except Exception as error:  # keep the matrix going, record the failure
                rows = [{"scenario": scenario, "run": run, "direction": "?", "error": str(error)}]
            all_rows.extend(rows)

    OUT.mkdir(parents=True, exist_ok=True)
    csv_path = OUT / "results.csv"
    fields = ["scenario", "run", "direction", "label", "codec", "path", "packets",
              "lost_packets", "loss_pct", "underruns", "buffer_resets", "buffer_ms",
              "clicks", "overflow_packets", "underrun_frames",
              "rtt_p99_ms", "hb_loss_pct", "error"]
    with open(csv_path, "w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fields)
        writer.writeheader()
        for row in all_rows:
            writer.writerow({key: row.get(key, "") for key in fields})
    print(f"wrote {csv_path}")

    # Markdown summary: one row per scenario, aggregated over runs and directions.
    lines = ["# 故障矩阵基线（tc netem，双向，真实 Wi-Fi 路径）", "",
             f"运行时间 {time.strftime('%Y-%m-%d %H:%M')}，每场景 {args.runs} 次 × {args.seconds}s", "",
             "| 场景 | 样本 | 平均丢包% | P99 丢包% | 心跳P99延迟ms | 咔哒 | 欠载 | 修剪 | 缓冲重置 | 无流次数 |",
             "| :-- | :-- | :-- | :-- | :-- | :-- | :-- | :-- | :-- | :-- |"]
    for scenario in scenarios:
        all_side = [r for r in all_rows if r["scenario"] == scenario and "error" not in r]
        # Stream rows carry loss_pct; heartbeat rows carry rtt/hb_loss_pct.
        rows = [r for r in all_side if "loss_pct" in r]
        errors = sum(1 for r in all_rows if r["scenario"] == scenario and "error" in r)
        if not rows:
            lines.append(f"| {scenario} | 0 | - | - | - | - | {errors} |")
            continue
        losses = sorted(r["loss_pct"] for r in rows)
        p99 = losses[min(len(losses) - 1, int(0.99 * len(losses)))]
        rtts = sorted(float(r["rtt_p99_ms"]) for r in all_rows
                      if r["scenario"] == scenario and r.get("rtt_p99_ms") is not None)
        rtt_p99 = f"{rtts[-1]:.0f}" if rtts else "-"
        lines.append(
            f"| {scenario} | {len(rows)} | {sum(losses)/len(losses):.2f} | {p99:.2f} "
            f"| {rtt_p99} "
            f"| {sum(r.get('clicks', 0) for r in rows)} "
            f"| {sum(r.get('underruns', 0) for r in rows)} "
            f"| {sum(r.get('overflow_packets', 0) for r in rows)} "
            f"| {sum(r.get('buffer_resets', 0) for r in rows)} | {errors} |"
        )
    md_path = OUT / "report.md"
    md_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {md_path}")
    restore_daemon(args.host, args.password, args.hostkey, daemon)
    return 0


if __name__ == "__main__":
    sys.exit(main())
