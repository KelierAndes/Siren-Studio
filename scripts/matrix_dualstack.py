#!/usr/bin/env python3
"""双栈切换与音频保活矩阵（5 场景 × 3 分钟）。

场景（每轮：稳定 20 s → 故障 180 s → 恢复观察 30 s；无故障轮只观察 180 s）：

  S1  PC↔手机      Wi-Fi+蓝牙 都在
  S2  PC↔手机      断手机 Wi-Fi 3 分钟（只剩蓝牙）
  S3  PC↔Linux↔手机 Wi-Fi+蓝牙 都在
  S4  PC↔Linux↔手机 断手机 Wi-Fi 3 分钟
  S5  PC↔Linux↔手机 断手机 Wi-Fi + 断 Linux 的 wlp3s0 3 分钟

三端都跑同一个构建；PC 与手机用应用（双栈由 HybridNode 自动接管），Linux 用
`siren web` 守护（同样 bt_enabled=true）。指标只取每个节点自己时钟里的量，
不做跨机亚秒对齐（NAS 时钟与 PC 差约 50 s 且在漂）。

    python scripts/matrix_dualstack.py --nas-pass <pw>
    python scripts/matrix_dualstack.py --nas-pass <pw> --only S1,S3 --minutes 1
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import threading
import time
import urllib.request
from datetime import datetime
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "scripts"))
import collect_1h as harness  # noqa: E402  复用 adb/plink 封装

ADB = harness.ADB
SERIAL = harness.SERIAL
PHONE_PKG = harness.PHONE_PKG
NAS = f"{harness.NAS_USER}@{harness.NAS_HOST}"
PC_EXE = REPO / "ui-flutter/build/windows/x64/runner/Release/siren_ui.exe"
PC_LOG = PC_EXE.parent / "siren_ui.log"
DAEMON_CMD = (
    # 测试形态必须给 Linux 节点真实音频：它是一台不放声音的机器，用 `--source loopback`
    # 时 E2 会把整段静音上行抑制掉（capture_silent_frames == capture_frames、kbps≈0），
    # 于是每一轮里 NAS 都不参与混音——面板/矩阵看到的"3 台"其实只有 2 路有声。
    # （`siren web --source` 的产品默认仍是 loopback=内录本机声音；`siren mesh` 默认 tone。）
    "runuser -u kelierandes -- env DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
    "XDG_RUNTIME_DIR=/run/user/1000 setsid nohup /usr/local/bin/siren --log info web "
    "--listen 0.0.0.0:8090 --isolate pipewire --output-sink play_sink --autostart "
    "--source tone --freq 440 --name Linux-ISO --port 47900 >/tmp/siren-web.log 2>&1 &"
)

SCENARIOS = [
    ("S1_pc_phone_dual", "pair", None),
    ("S2_pc_phone_wifi_off", "pair", "phone_wifi"),
    ("S3_tri_dual", "trio", None),
    ("S4_tri_phone_wifi_off", "trio", "phone_wifi"),
    # S5 旧语义只断 wlp3s0，而实测 NAS 的默认路由在 enp1s0(.196) 上，Wi-Fi(.159)
    # 只是次要出口 —— 那样根本没把 NAS 与 LAN 隔离，等于空转。现在两个都断。
    ("S5_tri_phone_nas_lan_off", "trio", "phone_wifi+nas_lan"),
]
NAS_LAN_IFACES = ("enp1s0", "wlp3s0")

SWITCH = re.compile(r"switch (\w+) -> (\w+)")
GAP = re.compile(r"audible gap ([\d.]+) ms before this (\w+) packet")
TRIM = re.compile(r"trace: trim stream=(\S+) fill_before_ms=([\d.]+).*dropped_ms=([\d.]+)")
ZERO = re.compile(r"trace: zerorun point=(\w+) run_ms=([\d.]+)")
EVIDENCE = re.compile(
    r"udp: stream (\S+) (wifi|bt) seq (\d+) pkts (\d+) lost (\d+) resets (\d+) "
    r"underruns (\d+) buffer ([\d.]+) ms target ([\d.]+)"
)
BT_LINES = re.compile(r"bt: (dialing|dial failed|channel connect|link|relay|accepted)")
# 链路存活数只能由生命周期行推出来："opened" 加一，任何收尾行减一。
LINK_LINE = re.compile(r"bt: (?:duplicate )?link (\d+)([^\n]*)")
LINK_CLOSE_WORDS = ("closed", "reader ended", "writer failed")
HYBRID = re.compile(r"hybrid: (\w[\w -]{0,30})")

state: dict = {}


def log(msg: str) -> None:
    print(f"[{datetime.now():%H:%M:%S}] {msg}", flush=True)


# ------------------------------------------------------------------ 节点控制


def nas_sh(cmd: str, timeout: int = 120) -> str:
    return harness.nas(cmd, timeout=timeout)


def nas_sudo_sh(cmd: str, timeout: int = 180) -> str:
    return harness.nas_sudo(cmd, timeout=timeout)


def daemon_running() -> bool:
    return nas_sh("pgrep -x siren | wc -l").strip() not in ("", "0")


def nas_status() -> dict | None:
    try:
        with urllib.request.urlopen(
                f"http://{harness.NAS_HOST}:8090/api/status", timeout=3) as resp:
            return json.loads(resp.read().decode("utf-8", "replace"))
    except Exception:  # noqa: BLE001
        return None


def ensure_nas_joined(require_peers: bool = False, timeout: float = 60.0) -> dict:
    """守护进程活着 ≠ 已入网；已入网 ≠ 音源正确。

    `--autostart` 只在进程启动时生效；有人（或页面）点过"离开组网"之后，进程还在、
    `snapshot` 却是 null，此后所有场景测的都是两节点网络（2026-11-05 A1 三组就是这样
    白跑的）。这里用 /api/status 断言"真的入网了"；`deploy_nas.py` 用生产默认的
    `--source loopback` 起守护，入网照样成功但上行全零（E2 抑制），所以音源也要断言。
    """
    payload = {
        "name": "Linux-ISO", "source": "tone", "freq": 440.0, "output": True,
        "target_ms": 30.0, "port": 47900, "codec": "raw", "frame_ms": 5,
        "output_sink": "play_sink",
    }
    deadline = time.time() + timeout
    started = False
    while time.time() < deadline:
        status = nas_status() or {}
        snap = status.get("snapshot") or {}
        source = (status.get("config") or {}).get("source")
        running = bool(snap.get("running"))
        peers = snap.get("peers") or []

        def join() -> None:
            body = json.dumps(payload)
            nas_sh("curl -s -m 3 -X POST -H 'Content-Type: application/json' "
                   f"-d '{body}' localhost:8090/api/start")

        if running and source != "tone" and not started:
            # /api/start 在运行中返回 409，必须先离开再按测试音源加入。
            nas_sh("curl -s -m 3 -X POST localhost:8090/api/stop")
            time.sleep(3)
            join()
            started = True
        elif running and source == "tone" and (peers or not require_peers):
            return {"running": True, "peers": len(peers), "source": source,
                    "restarted": started}
        elif not running and not started:
            join()
            started = True
        time.sleep(3)
    status = nas_status() or {}
    snap = status.get("snapshot") or {}
    return {"running": bool(snap.get("running")), "peers": len(snap.get("peers") or []),
            "source": (status.get("config") or {}).get("source"),
            "restarted": started, "timed_out": True}


def daemon_start() -> None:
    if not daemon_running():
        # runuser 在 /usr/sbin，plink 的用户 shell 里没有 PATH，必须走 sudo（sudo 的
        # secure_path 找得到）；守护被 kill 时隔离层的 Drop 不会执行，play_sink 也可能
        # 已消失，两者叠加会让节点把混音播回自己采集的 monitor（自激测试音）。
        nas_sudo_sh(
            "runuser -u kelierandes -- env DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
            "XDG_RUNTIME_DIR=/run/user/1000 setsid nohup /usr/local/bin/siren --log info web "
            "--listen 0.0.0.0:8090 --isolate pipewire --output-sink play_sink --autostart "
            "--source tone --freq 440 --name Linux-ISO --port 47900 "
            ">/tmp/siren-web.log 2>&1 & sleep 4"
        )
        time.sleep(3)
    # 进程活着 ≠ 已入网：有人点过"离开组网"后进程仍在、snapshot 为 null，而
    # --autostart 只在进程启动时生效。这里始终断言"真的入网了"，必要时重新 POST start。
    state = ensure_nas_joined()
    log(f"NAS 守护就绪：running={state.get('running')} peers={state.get('peers')}"
        + ("（已重新入网）" if state.get("restarted") else ""))


def audio_graph_preflight() -> dict:
    """守护起来之后先确认没有自激：默认 sink、monitor 输出去向、发送侧电平。"""
    env = ("export XDG_RUNTIME_DIR=/run/user/1000 "
           "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; ")
    text = nas_sh(env + "pactl list sinks short; echo '---'; pactl get-default-sink; "
                        "echo '---'; pactl list sink-inputs; "
                        "echo '---'; curl -s -m 3 localhost:8090/api/status")
    blocks = text.split("---")
    sinks = {}
    for line in blocks[0].splitlines():
        parts = line.split("\t")
        if len(parts) > 1:
            sinks[parts[1]] = parts[0]
    default = blocks[1].strip()
    monitor_sink = None
    for block in blocks[2].split("Sink Input #")[1:]:
        sink = re.search(r"^\tSink: (\d+)", block, re.M)
        media = re.search(r'media\.name = "?([^"\n]+)', block)
        if media and sink and media.group(1).startswith("mesh monitor"):
            monitor_sink = sink.group(1)
    tx_db = None
    try:
        snap = json.loads(blocks[3].strip()).get("snapshot") or {}
        tx_db = round((snap.get("sender") or {}).get("tx_rms_db", 0), 1)
    except (json.JSONDecodeError, TypeError):
        pass
    return {
        "sinks": sinks,
        "default_sink": default,
        "monitor_sink_id": monitor_sink,
        "play_sink_id": sinks.get("play_sink"),
        # 测试形态是 tone（见 DAEMON_CMD），所以"发送侧有电平"是预期，不能当回环证据；
        # 回环唯一可靠的判据是 mesh monitor 被路由进了正在被采集的那个 sink。
        "expected_source": "tone",
        "sender_tx_rms_db": tx_db,
    }


def daemon_stop() -> None:
    if daemon_running():
        nas_sudo_sh("pkill -x siren")
        time.sleep(2)


def phone_start() -> None:
    harness.adb_shell(f"am force-stop {PHONE_PKG}")
    time.sleep(2)
    harness.adb_shell(f"monkey -p {PHONE_PKG} -c android.intent.category.LAUNCHER 1")
    time.sleep(10)
    for _ in range(3):
        harness.adb_shell("input swipe 540 1800 540 900 400")
        time.sleep(1)
    harness.adb_shell("input tap 600 1944")  # 加入组网
    time.sleep(12)


def pc_start() -> subprocess.Popen:
    PC_LOG.unlink(missing_ok=True)
    env = {
        **os.environ,
        "SIREN_UI_AUTOSTART": "1",
        "SIREN_UI_MESH_NAME": "SIREN-PC",
        "SIREN_UI_MESH_SOURCE": "tone",
        "SIREN_UI_MESH_OUTPUT": "1",
    }
    return subprocess.Popen([str(PC_EXE)], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def wifi_phone(on: bool) -> str:
    return harness.adb_shell(f"svc wifi {'enable' if on else 'disable'}")


def nas_wifi_state() -> str:
    return nas_sh("nmcli -t -f DEVICE,STATE device | grep -E 'enp1s0|wlp3s0'").strip()


def nas_lan_cut(seconds: float) -> str:
    """把 NAS 与 LAN 彻底隔离：同时断 enp1s0(.196，默认路由/SSH) 与 wlp3s0(.159)。

    只断 Wi-Fi 是空转（2026-11-05 实测 `ip route get 192.168.2.107` → `dev enp1s0`）。
    断 enp1s0 会杀掉这条 SSH 会话，所以重连必须由先装好的脱离进程执行；本函数把
    "会话死了"当成预期结果，不抛错。
    """
    seconds = int(seconds)
    heal = (f"sleep {seconds + 40}; nmcli device connect enp1s0; nmcli device connect wlp3s0")
    cmd = (f"setsid nohup bash -c '{heal}' >/tmp/lan_selfheal.log 2>&1 </dev/null & "
           f"sleep 1; nmcli device disconnect wlp3s0; nmcli device disconnect enp1s0")
    try:
        return nas_sudo_sh(cmd)
    except Exception as error:  # noqa: BLE001 - SSH 随 enp1s0 一起消失就是成功
        return f"session closed as expected: {type(error).__name__}"


def nas_lan_restore() -> str:
    try:
        return nas_sudo_sh("nmcli device connect enp1s0; nmcli device connect wlp3s0")
    except Exception as error:  # noqa: BLE001 - 自愈计时器还在跑
        return f"restore deferred to self-heal: {type(error).__name__}"


# ------------------------------------------------------------------ 采样


def poll_nas(stop: threading.Event, path: Path) -> None:
    url = f"http://{harness.NAS_HOST}:8090/api/status"
    with open(path, "w", encoding="utf-8") as out:
        while not stop.is_set():
            try:
                with urllib.request.urlopen(url, timeout=2) as resp:
                    out.write(resp.read().decode("utf-8", "replace").strip() + "\n")
                    out.flush()
            except Exception:  # noqa: BLE001 - 断链期间取不到状态本身就是结果
                out.write(json.dumps({"unreachable": True, "ts": time.time()}) + "\n")
                out.flush()
            time.sleep(1.0)


def pc_slice(directory: Path, start_offset: int) -> int:
    """把 PC 应用日志从上一轮结束处切出来（只读增量，别截断正在写的文件）。"""
    if not PC_LOG.exists():
        return start_offset
    size = PC_LOG.stat().st_size
    with open(PC_LOG, encoding="utf-8", errors="replace") as handle:
        handle.seek(start_offset)
        text = handle.read()
    (directory / "pc.slice.log").write_text(text, encoding="utf-8")
    return size


def phone_slice(directory: Path) -> None:
    with open(directory / "phone.logcat.txt", "w", encoding="utf-8", errors="replace") as out:
        subprocess.run([ADB, "-s", SERIAL, "logcat", "-d"], stdout=out,
                       stderr=subprocess.STDOUT, timeout=180)


# ------------------------------------------------------------------ 指标


def metrics_from_log(text: str) -> dict:
    switches = [m.groups() for m in SWITCH.finditer(text)]
    gaps = [(float(m.group(1)), m.group(2)) for m in GAP.finditer(text)]
    trims = [m.groups() for m in TRIM.finditer(text)]
    zeros = [m.groups() for m in ZERO.finditer(text)]
    evidence = [m.groups() for m in EVIDENCE.finditer(text)]
    last_per_stream: dict[str, tuple] = {}
    for stream, carrier, _seq, pkts, lost, resets, underruns, buffer_ms, _target in evidence:
        last_per_stream[stream] = (carrier, int(pkts), int(lost), int(resets),
                                   int(underruns), float(buffer_ms))
    buffers = [v[5] for v in last_per_stream.values()]
    live_links: set[str] = set()
    link_peak = 0
    for match in LINK_LINE.finditer(text):
        link_id, detail = match.group(1), match.group(2)
        if "opened" in detail:
            live_links.add(link_id)
            link_peak = max(link_peak, len(live_links))
        elif any(word in detail for word in LINK_CLOSE_WORDS):
            live_links.discard(link_id)
    return {
        "switches": len(switches),
        "switch_dirs": {},
        "gap_count": len(gaps),
        "gap_max_ms": max([g[0] for g in gaps], default=0),
        "gap_sum_ms": round(sum(g[0] for g in gaps), 1),
        "trims": len(trims),
        "trim_max_dropped_ms": max([float(t[2]) for t in trims], default=0),
        "zeroruns_mix_over100ms": sum(1 for z in zeros if z[0] == "mix" and float(z[1]) > 100),
        "carrier_seen": sorted({s[1] for s in switches} | {v[0] for v in last_per_stream.values()}),
        "buffer_last_ms": [round(b) for b in buffers],
        "underruns_last": sum(v[4] for v in last_per_stream.values()),
        "lost_frames_last": sum(v[2] for v in last_per_stream.values()),
        "bt_lines": len(BT_LINES.findall(text)),
        "bt_link_end": len(live_links),
        "bt_link_peak": link_peak,
        "hybrid": sorted({m.group(1).strip() for m in HYBRID.finditer(text)})[:8],
    }


def metrics_from_nas(path: Path) -> dict:
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if row.get("unreachable"):
            continue
        rows.append(row)
    if not rows:
        return {"nas_snapshots": 0}
    first, last = rows[0], rows[-1]

    def digest(row):
        node = row.get("snapshot") or row.get("node") or row
        center = node.get("center") or {}
        streams = center.get("streams") or []
        master = center.get("master") or {}
        bt = node.get("bt") or {}
        peers = node.get("peers") or []
        return {
            "streams": len(streams),
            "buffer_ms": [round(s.get("buffer_ms", 0)) for s in streams],
            "underruns": sum(s.get("underruns", 0) for s in streams),
            "lost_frames": sum(s.get("lost_frames", 0) for s in streams),
            "silent_ms": master.get("silent_ms", 0),
            "silent_gaps": master.get("silent_gaps", 0),
            "clicks": sum(s.get("clicks", 0) for s in streams),
            "bt_state": bt.get("state"),
            "bt_role": bt.get("role"),
            "bt_links": bt.get("links"),
            "rtt_ms": [p.get("rtt_ms") for p in peers if isinstance(p.get("rtt_ms"), (int, float))],
        }

    a, b = digest(first), digest(last)
    # 计数器是"按流单调"的：流被重建时把全体求和再做首尾差会倒退（上一轮就报过
    # -2880 丢帧），所以按流 id 各取自己的首末值再相加。
    fields = ("underruns", "lost_frames", "clicks")
    first_by_id: dict[str, dict[str, int]] = {}
    last_by_id: dict[str, dict[str, int]] = {}
    mix_buffer: list[int] = []
    for row in rows:
        node = row.get("snapshot") or row.get("node") or row
        for item in (node.get("center") or {}).get("streams") or []:
            key = str(item.get("id"))
            values = {f: int(item.get(f) or 0) for f in fields}
            first_by_id.setdefault(key, values)
            last_by_id[key] = values
            if int(item.get("stream_id") or 0) == 0xFFF0:
                mix_buffer.append(round(float(item.get("buffer_ms") or 0)))
    delta = {
        f: sum(max(0, last_by_id[k][f] - first_by_id[k][f]) for k in last_by_id)
        for f in fields
    }
    out = {
        "nas_snapshots": len(rows),
        "streams": b["streams"],
        "buffer_ms": b["buffer_ms"],
        "underruns_delta": delta["underruns"],
        "lost_frames_delta": delta["lost_frames"],
        "clicks_delta": delta["clicks"],
        "silent_ms_delta": b["silent_ms"] - a["silent_ms"],
        "silent_gaps_delta": b["silent_gaps"] - a["silent_gaps"],
        "mix_buffer_last_ms": mix_buffer[-1] if mix_buffer else None,
        "mix_snapshots": len(mix_buffer),
        "bt_state_end": b["bt_state"],
        "bt_role_end": b["bt_role"],
        "bt_links_end": b["bt_links"],
        "bt_links_max": max((digest(r)["bt_links"] or 0) for r in rows),
        "rtt_ms": b["rtt_ms"],
    }
    reachable = sum(1 for line in path.read_text(encoding="utf-8").splitlines()
                    if '"unreachable": true' in line)
    out["unreachable_polls"] = reachable
    return out


# ------------------------------------------------------------------ 场景


def nas_snapshot() -> dict | None:
    try:
        with urllib.request.urlopen(
                f"http://{harness.NAS_HOST}:8090/api/status", timeout=2) as resp:
            return json.loads(resp.read().decode("utf-8", "replace")).get("snapshot")
    except Exception:  # noqa: BLE001 - 断链期间取不到状态本身就是观测点
        return None


def nas_bt_counters() -> dict:
    snap = nas_snapshot() or {}
    bt = snap.get("bt") or {}
    return {"ka_rt": bt.get("ka_rt"), "ka_dead": bt.get("ka_dead")}


def carrier_watch(seconds: float, directory: Path, before: dict | None) -> dict:
    """恢复期采样：每 2 s 读一次 NAS 快照，把"对端 Wi-Fi 回来后多久入向载波变回
    wifi"量成数字（F13/#68 的验收判据此前没人采过），同时留蓝牙保活的往返/判死增量。
    """
    rows = []
    deadline = time.time() + seconds
    first_wifi = None
    while time.time() < deadline:
        row = {"t": round(time.time() - (deadline - seconds), 1)}
        snap = nas_snapshot()
        if snap is None:
            row["unreachable"] = True
        else:
            streams = (snap.get("center") or {}).get("streams") or []
            row["paths"] = sorted({str(s.get("path") or "?") for s in streams})
            row["mix"] = [
                {"count": s.get("mix_count"), "path": s.get("path"),
                 "buffer_ms": round(float(s.get("buffer_ms") or 0))}
                for s in streams if int(s.get("stream_id") or 0) == 0xFFF0
            ]
            bt = snap.get("bt") or {}
            row["links"] = bt.get("links")
            row["ka_rt"] = bt.get("ka_rt")
            row["ka_dead"] = bt.get("ka_dead")
            if first_wifi is None and "wifi" in row["paths"]:
                first_wifi = row["t"]
        rows.append(row)
        time.sleep(2)
    (directory / "recovery.jsonl").write_text(
        "\n".join(json.dumps(r, ensure_ascii=False) for r in rows) + "\n", encoding="utf-8")
    end = rows[-1] if rows else {}
    started = before or {}
    return {
        "samples": len(rows),
        "first_wifi_at_s": first_wifi,
        "paths_end": end.get("paths"),
        "links_end": end.get("links"),
        "ka_rt_delta": (end.get("ka_rt") or 0) - (started.get("ka_rt") or 0),
        "ka_dead_delta": (end.get("ka_dead") or 0) - (started.get("ka_dead") or 0),
    }


def run_scenario(name: str, topology: str, fault: str | None, directory: Path,
                 fault_minutes: float, pc_offset: int) -> dict:
    directory.mkdir(parents=True, exist_ok=True)
    timeline = directory / "timeline.jsonl"

    def mark(action: str, **extra) -> None:
        row = {"ts": round(time.time(), 3), "scenario": name, "action": action}
        row.update(extra)
        with open(timeline, "a", encoding="utf-8") as out:
            out.write(json.dumps(row, ensure_ascii=False) + "\n")
        log(f"{name} {action} {extra}")

    if topology == "pair":
        daemon_stop()
    else:
        daemon_start()
        joined = ensure_nas_joined(require_peers=True, timeout=90)
        mark("joined", **joined)
        if not joined.get("running") or not joined.get("peers"):
            raise RuntimeError(
                f"{name}: NAS 节点没有真正入网（running={joined.get('running')} "
                f"peers={joined.get('peers')}）——三节点场景测不了，先修环境再跑")
        graph = audio_graph_preflight()
        (directory / "graph.json").write_text(json.dumps(graph, ensure_ascii=False, indent=2),
                                              encoding="utf-8")
        loop = graph.get("monitor_sink_id") not in (None, graph.get("play_sink_id"))
        mark("graph", **graph)
        if loop:
            mark("graph_invalid", reason="mesh monitor 没落在 play_sink，即自激回环")
            # 带回环继续跑只会产出看起来正常的假数据（上一轮就是这样白跑 3 分钟），
            # 所以直接停下，让 finally 把设备恢复回去。
            raise RuntimeError(
                f"{name}: Linux 节点音频图带回环 — "
                f"play_sink={graph.get('play_sink_id')} default={graph.get('default_sink')} "
                f"tx={graph.get('sender_tx_rms_db')} dBFS")

    mark("begin", topology=topology, fault=fault)
    nas_log_from = None
    if fault:
        wifi_phone(False)
        if fault == "phone_wifi+nas_lan":
            try:
                nas_log_from = int(nas_sh(
                    "stat -c %s /tmp/siren-web.log 2>/dev/null || echo 0").strip() or 0) + 1
            except Exception:  # noqa: BLE001
                nas_log_from = None
            # 断 enp1s0 会连着杀掉这条 SSH，所以 self-heal 与两次 disconnect 装在
            # 同一条脱离命令里，由 NAS 自己在窗口结束后接回来。
            mark("fault_cut", detail=nas_lan_cut(fault_minutes * 60 + 20))
        mark("fault_on", detail=nas_wifi_state())
    time.sleep(20)
    harness.adb_shell("logcat -c")
    stop = threading.Event()
    poller = None
    if topology == "trio":
        poller = threading.Thread(target=poll_nas, args=(stop, directory / "nas.status.jsonl"),
                                  daemon=True)
        poller.start()
    window_start = time.time()
    while time.time() - window_start < fault_minutes * 60:
        time.sleep(5)
    mark("window_end", seconds=round(time.time() - window_start, 1))
    # 恢复期的蓝牙保活增量要有个起点，取故障窗口末尾的计数器。
    recovery_baseline = nas_bt_counters()
    if poller:
        stop.set()
        poller.join(timeout=5)
    if fault:
        if fault == "phone_wifi+nas_lan":
            # 等 self-heal 把两个口接回来（窗口 + 40 s 武装延迟），不要硬连 SSH。
            deadline = time.time() + 90
            while time.time() < deadline:
                try:
                    if "connected" in nas_wifi_state():
                        break
                except Exception:  # noqa: BLE001 - 还没接回来就是继续等
                    pass
                time.sleep(5)
        wifi_phone(True)
        mark("fault_off", detail=nas_wifi_state())
        # 恢复期不是"等 30 s 就完事"：这里逐 2 s 采一次载波与蓝牙保活，
        # 量出对端 Wi-Fi 回来后多久入向变回 wifi（#68 的判据），
        # 以及半开链路的判死/重拨是否由保活接手（W1）。
        recovery = carrier_watch(60.0, directory, recovery_baseline)
        mark("recovery", **recovery)
        if nas_log_from is not None:
            # 隔离期间 NAS 自己不可达，它这段日志是纯蓝牙路径的唯一证据。
            try:
                (directory / "nas.isolated.log").write_text(
                    nas_sh(f"tail -c +{nas_log_from} /tmp/siren-web.log"), encoding="utf-8")
            except Exception:  # noqa: BLE001 - 取不到就只留 PC/手机侧
                pass
        result_recovery = recovery
    else:
        time.sleep(30)
        result_recovery = None
    mark("end")

    result = {}
    pc_offset = pc_slice(directory, pc_offset)
    result["pc"] = metrics_from_log((directory / "pc.slice.log").read_text(
        encoding="utf-8", errors="replace"))
    phone_slice(directory)
    result["phone"] = metrics_from_log((directory / "phone.logcat.txt").read_text(
        encoding="utf-8", errors="replace"))
    if (directory / "nas.status.jsonl").exists():
        result["nas"] = metrics_from_nas(directory / "nas.status.jsonl")
    if result_recovery:
        result["recovery"] = result_recovery
    (directory / "metrics.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    return result, pc_offset


def table(rows: list[tuple[str, dict]]) -> str:
    head = ("| 场景 | 蓝牙链路 末/峰（手机 · PC · NAS） | 保活往返/判死（NAS 恢复期增量） | 载波切换（手机 · PC）\n"
            "| 音频洞（手机）次/最长/合计 ms | 混音静音>100ms（手机 · PC） | 修剪（手机 · PC，最大丢弃 ms）\n"
            "| NAS 欠载/丢帧/无声 增量 | 末水位 ms（NAS · 手机 · PC） | 恢复期入向回到 wifi（s） |\n"
            "| :-- | :-- | :-- | :-- | :-- | :-- | :-- | :-- | :-- |\n")

    def live(device: dict) -> str:
        return f"{device.get('bt_link_end', '-')}/{device.get('bt_link_peak', '-')}"

    out = [head]
    for name, m in rows:
        phone, pc, nas = m.get("phone", {}), m.get("pc", {}), m.get("nas", {})
        links = f"{live(phone)} · {live(pc)} · {nas.get('bt_links_end', '-')}/{nas.get('bt_links_max', '-')}"
        switches = f"{phone.get('switches', 0)} · {pc.get('switches', 0)}"
        gaps = (f"{phone.get('gap_count', 0)}/{phone.get('gap_max_ms', 0):.0f}/"
                f"{phone.get('gap_sum_ms', 0):.0f}")
        mix_silence = (f"{phone.get('zeroruns_mix_over100ms', 0)} · "
                       f"{pc.get('zeroruns_mix_over100ms', 0)}")
        trims = (f"{phone.get('trims', 0)} · {pc.get('trims', 0)}，"
                 f"{max(phone.get('trim_max_dropped_ms', 0), pc.get('trim_max_dropped_ms', 0)):.0f}")
        nas_delta = (f"{nas.get('underruns_delta', '-')} / {nas.get('lost_frames_delta', '-')}"
                     f" / {nas.get('silent_ms_delta', '-')}")
        level = f"{nas.get('buffer_ms', '-')} · {phone.get('buffer_last_ms', '-')} · {pc.get('buffer_last_ms', '-')}"
        rec = m.get("recovery", {})
        keepalive = f"{rec.get('ka_rt_delta', '-')} · {rec.get('ka_dead_delta', '-')}"
        wifi_back = rec.get("first_wifi_at_s", "-")
        out.append(
            f"| {name} | {links} | {keepalive} | {switches} | {gaps} | {mix_silence} | {trims} | "
            f"{nas_delta} | {level} | {wifi_back} |\n"
        )
    return "".join(out)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nas-pass", default=os.environ.get("SIREN_NAS_PASS", ""))
    parser.add_argument("--minutes", type=float, default=3.0, help="故障/观察窗口时长")
    parser.add_argument("--only", default="", help="逗号分隔的场景前缀，如 S1,S3")
    parser.add_argument("--out", default=str(REPO / "out" / f"matrix_{datetime.now():%Y-%m-%d_%H%M}"))
    args = parser.parse_args()
    if not args.nas_pass:
        print("缺少 --nas-pass（或 SIREN_NAS_PASS）", file=sys.stderr)
        return 2
    harness.NAS_PASS = args.nas_pass
    os.environ["SIREN_NAS_PASS"] = args.nas_pass

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "meta.txt").write_text(
        "\n".join([
            f"started={datetime.now().isoformat()}",
            f"commit={subprocess.run(['git', '-C', str(REPO), 'rev-parse', '--short', 'HEAD'],
                                     capture_output=True, text=True).stdout.strip()}",
            f"minutes_per_fault={args.minutes}",
            f"pc_exe={PC_EXE}",
            f"nas_binary={nas_sh('/usr/local/bin/siren --version').strip()}",
            f"phone={harness.adb_shell('getprop ro.build.version.release').strip()}",
            f"nas_wifi_before={nas_wifi_state()}",
        ]), encoding="utf-8")

    log("启动 PC 应用与手机应用")
    nas_sh("export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; "
           "pactl list sinks short | grep -q play_sink || pactl load-module module-null-sink "
           "sink_name=play_sink sink_properties=device.description=play_sink; "
           "pactl set-default-sink play_sink")
    pc = pc_start()
    time.sleep(12)
    phone_start()
    pc_offset = PC_LOG.stat().st_size if PC_LOG.exists() else 0

    rows: list[tuple[str, dict]] = []
    try:
        for name, topology, fault in SCENARIOS:
            if args.only and not any(name.startswith(p) for p in args.only.split(",")):
                continue
            result, pc_offset = run_scenario(
                name, topology, fault, out / name, args.minutes, pc_offset)
            rows.append((name, result))
    finally:
        wifi_phone(True)
        nas_lan_restore()
        try:
            daemon_start()
        except Exception as error:  # noqa: BLE001 - NAS 可能还在自愈，别把收尾也带崩
            print(f"收尾时 daemon 重启失败（稍后手动确认）：{error}", flush=True)
        # 收尾必须把测试音源全部关掉：残留的 tone 发送端会让 Linux 内录里
        # 一直有 440 Hz，下一轮排查会被它带偏。
        harness.adb_shell(f"am force-stop {PHONE_PKG}")
        if pc.poll() is None:
            pc.terminate()
        (out / "matrix.md").write_text(
            f"# 双栈切换与保活矩阵（{datetime.now():%Y-%m-%d %H:%M}）\n\n" + table(rows),
            encoding="utf-8")
        (out / "matrix.json").write_text(json.dumps(dict(rows), ensure_ascii=False, indent=2),
                                         encoding="utf-8")
    print((out / "matrix.md").read_text(encoding="utf-8"))
    log(f"产物在 {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
