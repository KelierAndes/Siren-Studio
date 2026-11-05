#!/usr/bin/env python3
"""数据采集计划（1 小时版）的执行驱动：T1 基线 → T3 切换 → T4 发送端 A/B → T6 网络质量。

在三端（Linux hub / Windows 发送端 / Android）同一构建上跑，全程写 timeline.csv，
每个阶段单独目录，结束后把日志与录音拉回来解析成计划要求的 CSV。

    python scripts/collect_1h.py                 # 全流程
    python scripts/collect_1h.py --only T1,T6    # 只跑部分
    python scripts/collect_1h.py --quick         # 时间减半（T1 6min、每场景 3 次、T4/T6 各 2min）

依赖：plink/pscp（Linux 主机）、adb（手机）、ffmpeg 或 numpy 生成脉冲测试音。
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
HOSTKEY = "SHA256:xvTGpWJxeqqcQb/aPVpkOP6YTMfIq7sl8tWQ4iwTSRU"
NAS_USER = "kelierandes"
NAS_HOST = os.environ.get("SIREN_NAS_HOST", "192.168.2.196")
NAS_PASS = os.environ.get("SIREN_NAS_PASS", "")
ADB = os.environ.get(
    "ADB", "C:/Users/KelierAndes/AppData/Local/Android/Sdk/platform-tools/adb.exe"
)
SERIAL = os.environ.get("SIREN_PHONE", "6471a65")
PLINK = "C:/Program Files/PuTTY/plink.exe"
PSCP = "C:/Program Files/PuTTY/pscp.exe"
WIN_SIREN = str(REPO / "target/release/siren.exe")
NAS_SIREN = "/home/kelierandes/siren-build/target/release/siren"
NAS_REMOTE = "/home/kelierandes/out1h"
PHONE_PKG = "com.sirenstudio.siren_ui"
TARGET_MS = 100
FRAME_MS = 5

state = {"out": None, "timeline": None, "t0": None}


def now_ms() -> int:
    return int(time.time() * 1000)


def mark(phase: str, action: str, **extra) -> None:
    row = {"ts_ms": now_ms(), "phase": phase, "action": action}
    row.update(extra)
    with open(state["timeline"], "a", encoding="utf-8") as out:
        out.write(json.dumps(row, ensure_ascii=False) + "\n")
    print(f"[{datetime.now().strftime('%H:%M:%S')}] {phase} {action} {extra}", flush=True)


def nas(cmd: str, timeout: int = 120) -> str:
    # 必须显式 utf-8：Windows 上 text=True 会按本地 GBK 解码，NAS 日志里的非 ASCII
    # （例如 mesh.rs 的 "recovered — restoring"，U+2014）会被换成 U+FFFD，
    # 于是按原文写的正则永远匹配不上——观测被当成产品故障（2026-11-05 就是这样
    # 误判 F5 "恢复 0/3"，其实日志里每轮都恢复了）。
    result = subprocess.run(
        [PLINK, "-batch", "-ssh", "-hostkey", HOSTKEY, "-pw", NAS_PASS,
         f"{NAS_USER}@{NAS_HOST}", cmd],
        capture_output=True, text=True, timeout=timeout, errors="replace",
        encoding="utf-8",
    )
    return (result.stdout or "") + (result.stderr or "")


def nas_sudo(cmd: str, timeout: int = 120) -> str:
    return nas(f"echo {NAS_PASS} | sudo -S -p '' {cmd}", timeout=timeout)


def adb_shell(cmd: str, timeout: int = 60) -> str:
    result = subprocess.run([ADB, "-s", SERIAL, "shell"] + cmd.split(),
                            capture_output=True, text=True, timeout=timeout, errors="replace")
    return (result.stdout or "") + (result.stderr or "")


def pull(remote: str, local: Path) -> None:
    subprocess.run([PSCP, "-batch", "-hostkey", HOSTKEY, "-pw", NAS_PASS,
                    f"{NAS_USER}@{NAS_HOST}:{remote}", str(local)],
                   capture_output=True, text=True, timeout=600, errors="replace")


def push(local: Path, remote: str) -> None:
    subprocess.run([PSCP, "-batch", "-hostkey", HOSTKEY, "-pw", NAS_PASS,
                    str(local), f"{NAS_USER}@{NAS_HOST}:{remote}"],
                   capture_output=True, text=True, timeout=600, errors="replace")


# --------------------------------------------------------------------------- 进程


class Hub:
    """Linux 节点：mesh 播放 + parecord 录 monitor + stats-json + 日志。"""

    def __init__(self, phase_dir: Path, env: dict | None = None):
        self.dir = phase_dir
        self.env = env or {}
        self.remote_dir = f"{NAS_REMOTE}/{phase_dir.name}"
        self.proc = None

    def start(self) -> None:
        nas(f"mkdir -p {self.remote_dir}; pkill -x siren || true; pkill -x parecord || true")
        time.sleep(2)
        exports = " ".join(f"{key}={value}" for key, value in self.env.items())
        prefix = (
            "export DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
            "XDG_RUNTIME_DIR=/run/user/1000; "
        )
        if exports:
            prefix += f"export {exports}; "
        # 先起播放节点，再起独立采集进程：录制时钟不参与被测路径。
        node = (
            f"{prefix}setsid nohup {NAS_SIREN} --log info mesh --source tone --freq 440 "
            f"--output-sink play_sink --target-ms {TARGET_MS} --frame-ms {FRAME_MS} "
            f"--stats-json {self.remote_dir}/hub.stats.jsonl --stats-interval 1 "
            f"--name Linux-ISO --port 47900 > {self.remote_dir}/hub.log 2>&1 < /dev/null &"
        )
        nas(node)
        time.sleep(6)
        rec = (
            f"{prefix}setsid nohup bash -c 'timeout 1200 parecord "
            f"--device=play_sink.monitor --format=s16le --rate=48000 --channels=2 "
            f"--file-format=wav {self.remote_dir}/recording.wav' "
            f"> {self.remote_dir}/parecord.log 2>&1 < /dev/null &"
        )
        nas(rec)
        time.sleep(2)
        mark(self.dir.name, "hub_started")

    def stop(self) -> None:
        nas("pkill -x parecord || true; sleep 1; pkill -x siren || true; sleep 1")
        for name in ("hub.log", "hub.stats.jsonl", "recording.wav", "parecord.log"):
            pull(f"{self.remote_dir}/{name}", self.dir / name)
        mark(self.dir.name, "hub_stopped")


class Sender:
    """Windows 节点：loopback 采集（播脉冲测试音），不本地播放。"""

    def __init__(self, phase_dir: Path, wav: Path):
        self.dir = phase_dir
        self.wav = wav
        self.node = None
        self.player = None
        # E0: run-stamped names so a rerun of the same phase never mixes
        # logs/stats from two collections.
        self.stamp = datetime.now().strftime("%H%M%S")

    def start(self) -> None:
        log = open(self.dir / f"pc_{self.stamp}.log", "w", encoding="utf-8", errors="replace")
        self.node = subprocess.Popen(
            [WIN_SIREN, "--log", "info", "mesh", "--source", "loopback", "--no-play",
             "--target-ms", str(TARGET_MS), "--frame-ms", str(FRAME_MS),
             "--stats-json", str(self.dir / f"pc_{self.stamp}.stats.jsonl"), "--stats-interval", "1",
             "--duration", "3600"],
            stdout=log, stderr=subprocess.STDOUT,
        )
        time.sleep(6)
        self.player = subprocess.Popen(
            ["powershell", "-NoProfile", "-Command",
             f'$p = New-Object System.Media.SoundPlayer "{self.wav}"; $p.Load(); $p.PlaySync()'],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        mark(self.dir.name, "pc_started", pid=self.node.pid)

    def stop(self) -> None:
        for proc in (self.player, self.node):
            if proc and proc.poll() is None:
                proc.terminate()
        time.sleep(2)
        mark(self.dir.name, "pc_stopped")


class Phone:
    """Android 节点：应用入网，logcat 被动记录，top 采样 CPU。"""

    def __init__(self, phase_dir: Path):
        self.dir = phase_dir
        self.logcat = None
        self.cpu = None

    def join(self) -> None:
        adb_shell(f"am force-stop {PHONE_PKG}")
        time.sleep(2)
        adb_shell(f"monkey -p {PHONE_PKG} -c android.intent.category.LAUNCHER 1")
        time.sleep(10)
        # 组网卡在页面下方：滚过去再点“加入组网”。
        for _ in range(3):
            adb_shell("input swipe 540 1800 540 900 400")
            time.sleep(1)
        time.sleep(2)
        adb_shell("input tap 600 1944")
        time.sleep(12)
        mark(self.dir.name, "phone_join_tapped")

    def start_logging(self) -> None:
        self.logcat = open(self.dir / "phone.logcat.txt", "w", encoding="utf-8", errors="replace")
        subprocess.run([ADB, "-s", SERIAL, "logcat", "-c"], capture_output=True)
        self.cpu = open(self.dir / "phone.top.txt", "w", encoding="utf-8", errors="replace")

    def sample_cpu(self) -> None:
        if self.cpu is None:
            return
        pid = adb_shell(f"pidof {PHONE_PKG}").strip().split()
        if not pid:
            return
        text = adb_shell(f"top -H -b -n 1 -p {pid[0]}")
        self.cpu.write(f"=== ts_ms={now_ms()}\n{text}\n")
        self.cpu.flush()

    def stop_logging(self) -> None:
        if self.logcat is None:
            return
        subprocess.run([ADB, "-s", SERIAL, "logcat", "-d"],
                       stdout=self.logcat, stderr=subprocess.STDOUT, timeout=120)
        self.logcat.close()
        self.cpu.close()
        self.logcat = self.cpu = None


# --------------------------------------------------------------------------- 阶段


def pulse_wav(path: Path, seconds: int, tone_hz=440.0, pulse_hz=1000.0, every=20.0) -> None:
    """测试音上每 `every` 秒叠一个 10 ms 的 `pulse_hz` 脉冲，用于端到端延迟。"""
    import numpy as np

    sr = 48000
    t = np.arange(int(sr * seconds)) / sr
    signal = 0.18 * np.sin(2 * np.pi * tone_hz * t)
    for start in range(0, seconds, int(every)):
        lo = start * sr + (sr // 8)
        hi = lo + sr // 100
        if hi <= signal.size:
            ramp = np.hanning(hi - lo)
            signal[lo:hi] += 0.35 * ramp * np.sin(2 * np.pi * pulse_hz * np.arange(hi - lo) / sr)
    stereo = np.stack([signal, signal], axis=1)
    pcm = (np.clip(stereo, -1, 1) * 32767).astype("<i2")
    header = b"RIFF" + (36 + pcm.nbytes).to_bytes(4, "little") + b"WAVEfmt " + (
        16).to_bytes(4, "little") + (1).to_bytes(2, "little") + (2).to_bytes(2, "little") + (
        sr).to_bytes(4, "little") + (sr * 4).to_bytes(4, "little") + (4).to_bytes(2, "little") + (
        16).to_bytes(2, "little") + b"data" + (pcm.nbytes).to_bytes(4, "little")
    path.write_bytes(header + pcm.tobytes())


def netem(params: str | None) -> str:
    script = "bash /home/kelierandes/siren-build/scripts/linux/netem.sh"
    action = "clear" if params is None else f"apply raw '{params}'"
    try:
        return nas_sudo(f"{script} {action}", timeout=180)
    except subprocess.TimeoutExpired:
        # 注入本身把控制通道打断过；超时不能让整个采集报废，记进 timeline 就好。
        mark("netem", "timeout", action=action)
        return f"TIMEOUT: {action}"


def clear_stale_senders() -> None:
    """E0/12.4: a leftover siren.exe from a previous run holds UDP ports and
    (worse) keeps sending, which corrupted T3/T6 PC files with 10048."""
    subprocess.run(["taskkill", "/IM", "siren.exe", "/F"],
                   capture_output=True, text=True)
    time.sleep(1)
    probe = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    busy = False
    try:
        probe.bind(("0.0.0.0", 47910))
    except OSError:
        busy = True
    finally:
        probe.close()
    if busy:
        raise RuntimeError("UDP 47910 still occupied after killing siren.exe")


def run_phase(name: str, minutes: float, out: Path, env: dict | None = None,
              impair: str | None = None) -> Path:
    phase = out / name
    phase.mkdir(parents=True, exist_ok=True)
    clear_stale_senders()
    mark(name, "phase_begin", minutes=minutes)
    hub = Hub(phase, env)
    hub.start()
    sender = Sender(phase, out / "pulse.wav")
    sender.start()
    phone = Phone(phase)
    phone.join()
    phone.start_logging()
    if impair:
        print(netem(impair))
        mark(name, "impairment_on", params=impair)
        time.sleep(3)
    deadline = time.time() + minutes * 60
    while time.time() < deadline:
        time.sleep(5)
        phone.sample_cpu()
    if impair:
        print(netem(None))
        mark(name, "impairment_off")
        time.sleep(10)
    phone.stop_logging()
    sender.stop()
    hub.stop()
    analyze(phase, name)
    mark(name, "phase_end")
    return phase


def analyze(phase: Path, name: str) -> None:
    # 子进程的 stdout 指向文件时 Python 会用本地代码页（GBK），JSON 里的中文会写坏。
    env = {**os.environ, "PYTHONIOENCODING": "utf-8"}
    with open(phase / "gaps.json", "wb") as sink:
        subprocess.run([sys.executable, str(REPO / "scripts/analyze_gaps.py"),
                        str(phase / "recording.wav"), "--stats", str(phase / "hub.stats.jsonl"),
                        "--log", str(phase / "hub.log"), "--json"],
                       stdout=sink, stderr=subprocess.DEVNULL, timeout=900, env=env)
    subprocess.run([sys.executable, str(REPO / "scripts/parse_traces.py"),
                    str(phase / "hub.log"), "--outdir", str(phase),
                    "--gaps", str(phase / "gaps.json")],
                   capture_output=True, text=True, timeout=300)


def t3(out: Path, cycles: int, quick: bool) -> None:
    phase = out / "T3"
    phase.mkdir(parents=True, exist_ok=True)
    mark("T3", "phase_begin", cycles=cycles)
    hub = Hub(phase)
    hub.start()
    sender = Sender(phase, out / "pulse.wav")
    sender.start()
    phone = Phone(phase)
    phone.join()
    phone.start_logging()
    scenarios = [("S1", None), ("S2", "loss 30%"), ("S3", "loss 100%")]
    fault_s = {"S1": 5, "S2": 5, "S3": 3}
    for name, params in scenarios:
        for index in range(cycles):
            if name == "S1":
                mark("T3", "inject", scenario=name, n=index + 1, how="phone wifi off")
                adb_shell("svc wifi disable")
                time.sleep(fault_s[name])
                mark("T3", "clear", scenario=name, n=index + 1, how="phone wifi on")
                adb_shell("svc wifi enable")
            else:
                print(netem(params))
                mark("T3", "inject", scenario=name, n=index + 1, params=params)
                time.sleep(fault_s[name])
                print(netem(None))
                mark("T3", "clear", scenario=name, n=index + 1)
            time.sleep(9 if not quick else 6)
    phone.sample_cpu()
    phone.stop_logging()
    sender.stop()
    hub.stop()
    analyze(phase, "T3")
    mark("T3", "phase_end")


def write_meta(out: Path, args) -> None:
    commit = subprocess.run(["git", "-C", str(REPO), "rev-parse", "--short", "HEAD"],
                            capture_output=True, text=True).stdout.strip()
    phone_model = adb_shell("getprop ro.product.model").strip()
    phone_os = adb_shell("getprop ro.build.version.release").strip()
    nas_info = nas("uname -sr; ip route show default | awk '{print $5; exit}'", timeout=60)
    # E0/A11: NTP on the NAS so cross-machine drift stops compounding (the
    # NAS clock used to run 50-60 s fast and kept drifting ~0.5%).
    ntp = nas_sudo("timedatectl set-ntp true; timedatectl show -p NTP -p NTPSynchronized", timeout=60)
    clock = nas("date -u +%H:%M:%S.%3N; printf 'pc '; date +%H:%M:%S.%3N", timeout=60)
    (out / "meta.txt").write_text(
        "\n".join(
            [
                f"run_started_utc={datetime.now(timezone.utc).isoformat()}",
                f"ntp={ntp.strip().replace(chr(10), ' ')}",
                f"commit={commit}",
                "构建=三端同一提交（含修剪不清空修复 8da4250 与心跳未连接套接字 4415a3e）",
                f"Linux hub={nas_info.strip().replace(chr(10), ' | ')}",
                f"Windows sender={os.environ.get('COMPUTERNAME', '?')}",
                f"Android={phone_model} / Android {phone_os} / serial {SERIAL}",
                f"clock_check={clock.strip().replace(chr(10), ' | ')}",
                f"缓冲目标={TARGET_MS} ms 帧长={FRAME_MS} ms Wi-Fi=raw 蓝牙=Opus 24kbps",
                "插桩=I1 修剪(mixer) / I3 零值游程(capture+mix) / I2 载波切换(既有 switch 三段)",
                "未做插桩=发送端 DSP 状态快照、采集环补零计数、解码器重置原因",
                f"计划时长={args.total} 分钟 quick={args.quick}",
            ]
        )
        + "\n",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", default="T1,T3,T4,T6")
    parser.add_argument("--quick", action="store_true")
    parser.add_argument("--total", type=float, default=60.0, help="计划总时长（分钟），只写进 meta")
    parser.add_argument("--out", default=str(REPO / "out" / datetime.now().strftime("%Y-%m-%d_%H%M")))
    args = parser.parse_args()
    if not NAS_PASS:
        print("SIREN_NAS_PASS 未设置", file=sys.stderr)
        return 2

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    state["out"] = out
    state["timeline"] = out / "timeline.csv"
    phases = args.only.split(",")

    mark("prep", "begin")
    if not (out / "pulse.wav").exists():
        pulse_wav(out / "pulse.wav", seconds=1500)
    write_meta(out, args)
    # NAS 上的 netem.sh 必须是 LF 行尾且带 raw profile，否则注入会静默失败。
    netem_local = out / "netem.sh"
    netem_text = (REPO / "scripts/linux/netem.sh").read_text(encoding="utf-8")
    netem_local.write_bytes("\n".join(netem_text.splitlines()).encode("utf-8"))
    push(netem_local, f"{NAS_REMOTE}/netem.sh")
    nas(f"mkdir -p {NAS_REMOTE}; cp {NAS_REMOTE}/netem.sh /home/kelierandes/siren-build/scripts/linux/netem.sh")
    print(nas_sudo("bash /home/kelierandes/siren-build/scripts/linux/netem.sh apply raw 'loss 1%'")[:120])
    print(netem(None)[:120])

    scale = 0.5 if args.quick else 1.0
    if "T1" in phases:
        run_phase("T1", 12 * scale, out)
    if "T3" in phases:
        t3(out, cycles=3 if args.quick else 5, quick=args.quick)
    if "T4" in phases:
        run_phase("T4_A", 4 * scale, out)
        run_phase("T4_B", 4 * scale, out, env={"SIREN_ECHO_CANCEL": "0"})
    if "T6" in phases:
        run_phase("T6_N2", 4 * scale, out, impair="loss 5%")
        run_phase("T6_N4", 4 * scale, out, impair="loss 3% delay 40ms 30ms rate 2mbit")
    mark("collect", "end", out=str(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
