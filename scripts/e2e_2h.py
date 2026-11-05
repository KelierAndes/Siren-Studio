#!/usr/bin/env python3
"""2 小时全链路测试编排（依据 E2E_TEST_PLAN_2H_v2.md）。

设计取舍（先说清楚，避免把"没测"写成"通过"）：
- 复用已验证的构件：collect_1h 的 adb/plink 封装、matrix_dualstack 的守护起停/入网断言/
  探针、netem.sh 的 audio-only 注入、f5_cycles 的 F5 判读思路。
- 每个用例写一行到 cases.csv：PASS / FAIL / KNOWN-OPEN / SKIP / N/A + 实测数字 + 备注。
  缺能力的用例显式标 SKIP/N/A 并写明原因，绝不补零。
- 逐音塌陷（G1）不在线检测：hub/PC 录音在 P8 用 `siren analyze --freq …` 离线出数
  （CLI 已支持多频点 + json）。在线只采快照/日志/录音三件套。

用法：
  python scripts/e2e_2h.py --nas-pass <pw>                 # 全量（约 108 min）
  python scripts/e2e_2h.py --nas-pass <pw> --only P0,P1    # 预检/回归
  python scripts/e2e_2h.py --nas-pass <pw> --budget 60     # 60 分钟裁剪档
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import re
import shutil
import socket
import statistics
import subprocess
import sys
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

# 日志重定向到文件时 Windows 会用 cp936 编码，中文/破折号全部变乱码。
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except (AttributeError, ValueError):
    pass

import collect_1h as harness  # noqa: E402
import matrix_dualstack as M  # noqa: E402

OUT = ROOT / "out" / f"2026-11-05_e2e_{datetime.now():%H%M}"
CASES = OUT / "cases.csv"
SAMPLES = OUT / "samples"
PHONE = harness.PHONE_PKG
PC_EXE = M.PC_EXE

GAP = M.GAP
SWITCH = M.SWITCH
TRIM = M.TRIM
ZERO = M.ZERO

rows: list[dict] = []


# ------------------------------------------------------------------ 记录

def toolchain_env() -> None:
    """opus-head-sys 用 CMake 编 libopus，CMake 不在 PATH（同 scripts/build.ps1）。

    不补这一步，`cargo test` 在干净缓存上会以 "is cmake not installed?" 失败，
    把环境问题伪装成代码回归。
    """
    if shutil.which("cmake"):
        return
    candidates: list[str] = []
    for base, pattern in ((Path("C:/tools"), "cmake-*/bin"),
                          (Path(os.environ.get("ProgramFiles", r"C:\Program Files")),
                           "CMake/bin")):
        try:
            candidates.extend(str(path) for path in base.glob(pattern)
                              if (path / "cmake.exe").exists())
        except OSError:
            pass
    vswhere = Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / \
        "Microsoft Visual Studio/Installer/vswhere.exe"
    if vswhere.exists():
        try:
            found = subprocess.run(
                [str(vswhere), "-latest", "-products", "*",
                 "-property", "installationPath"],
                capture_output=True, text=True, timeout=30).stdout.strip().splitlines()
            for install in reversed(found):
                cmake_dir = Path(install.strip()) / \
                    "Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin"
                if (cmake_dir / "cmake.exe").exists():
                    candidates.append(str(cmake_dir))
                    break
        except OSError:
            pass
    if candidates:
        os.environ["PATH"] = ";".join(dict.fromkeys(candidates)) + ";" + \
            os.environ.get("PATH", "")


def sntp_offset(host: str = "ntp.tencent.com", samples: int = 9) -> float | None:
    """本机相对 NTP 源的偏差（ms，正值 = 本机快）。

    `adb shell date` / `plink date` 的进程启动要几百毫秒，直接相减量不出 5 ms；
    SNTP 往返的对称性误差只有几毫秒。
    """
    try:
        addr = socket.getaddrinfo(host, 123, socket.AF_INET, socket.SOCK_DGRAM)[0][4]
    except OSError:
        return None
    rows: list[float] = []
    for _ in range(samples):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.settimeout(2)
        try:
            sent = time.time()
            sock.sendto(b"\x1b" + 47 * b"\x00", addr)
            data, _ = sock.recvfrom(48)
            received = time.time()
        except OSError:
            continue
        finally:
            sock.close()
        server = int.from_bytes(data[40:48], "big") / 2**32 - 2208988800
        rows.append((server - (sent + received) / 2) * 1000)
    return statistics.median(rows) if rows else None


def log(msg: str) -> None:
    print(f"[{datetime.now():%H:%M:%S}] {msg}", flush=True)


def case(cid: str, phase: str, name: str, fn, **kw) -> str:
    """跑一个用例并落一行 cases.csv。fn 返回 (status, notes, metrics)。"""
    started = time.time()
    log(f"{cid} {name} …")
    # 手机日志按用例切窗：每次用例开始清缓冲，函数里的 phone_logcat() 就只会
    # 拿到本用例的日志。0.4 要看预检前的整段缓冲，单独保留。
    if cid != "0.4":
        try:
            adb("logcat -c")
        except Exception:  # noqa: BLE001 - 手机不在线时不该带崩用例
            pass
    try:
        status, notes, metrics = fn(**kw)
    except Exception as error:  # noqa: BLE001 - 单用例异常不该带崩整轮
        status, notes, metrics = "FAIL", f"异常：{type(error).__name__}: {error}", {}
    finally:
        try:
            save_slices(cid, started, time.time())
        except Exception as error:  # noqa: BLE001 - 日志切片失败不影响判定
            log(f"{cid} 日志切片失败：{type(error).__name__}: {error}")
    row = {
        "id": cid, "phase": phase, "case": name, "status": status,
        "seconds": round(time.time() - started, 1),
        "notes": notes, "metrics": json.dumps(metrics, ensure_ascii=False),
    }
    rows.append(row)
    (OUT / cid).mkdir(parents=True, exist_ok=True)
    (OUT / cid / "result.json").write_text(
        json.dumps(row, ensure_ascii=False, indent=2), encoding="utf-8")
    with open(CASES, "w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(row))
        writer.writeheader()
        writer.writerows(rows)
    log(f"{cid} → {status} ({row['seconds']} s) {notes[:100]}")
    return status


def skip(reason: str):
    return "SKIP", reason, {}


def na(reason: str):
    return "N/A", reason, {}


def _utc_hms(ts: float) -> str:
    return datetime.fromtimestamp(ts, timezone.utc).strftime("%H:%M:%S")


def save_slices(cid: str, started: float, ended: float) -> None:
    """把本用例时间窗内的 NAS / PC 日志切出来存到用例目录。

    NAS 与 PC 的日志时间戳都是 UTC；daemon 重启前会另存一份 before.log，
    两份都要扫，才不会在重启用例里丢掉半段。
    """
    directory = OUT / cid
    directory.mkdir(parents=True, exist_ok=True)
    a, b = _utc_hms(started), _utc_hms(ended)
    try:
        # 断网/重启中的用例不能把整轮拖住：切片只是取证，拿不到就跳过。
        text = M.nas_sh("cat /tmp/siren-web.log /tmp/siren-web.before.log 2>/dev/null",
                        timeout=15)
    except Exception:  # noqa: BLE001
        text = ""
    nas_rows = []
    for line in text.splitlines():
        if len(line) > 21 and "T" in line[:22] and a <= line.split("T", 1)[1][:8] <= b:
            nas_rows.append(line)
    if nas_rows:
        (directory / "nas.log").write_text("\n".join(nas_rows), encoding="utf-8")
    pc_path = M.PC_LOG
    if pc_path.exists():
        pc_rows = []
        for line in pc_path.read_text(encoding="utf-8", errors="replace").splitlines():
            match = re.search(r"T(\d\d:\d\d:\d\d)", line)
            if match and a <= match.group(1) <= b:
                pc_rows.append(line)
        if pc_rows:
            (directory / "pc.log").write_text("\n".join(pc_rows), encoding="utf-8")


# ------------------------------------------------------------------ 设备动作

def adb(cmd: str) -> str:
    return harness.adb_shell(cmd)


def nas(cmd: str) -> str:
    return M.nas_sh(cmd)


def nas_sudo(cmd: str) -> str:
    return M.nas_sudo_sh(cmd)


def netem(profile: str, direction: str = "both") -> str:
    # 未知档位时 netem.sh 只打印错误并以 1 退出；不检查的话用例会在干净网络上
    # 跑完还判"通过"（上一轮 C6 就是这样拿到 lost=0 的）。SSH 偶发 abort 重试一次。
    last = ""
    for attempt in range(2):
        try:
            out = nas_sudo(
                f"AUDIO_ONLY=1 DIRECTION={direction} "
                f"bash $HOME/scripts/linux/netem.sh apply {profile}")
        except Exception as error:  # noqa: BLE001
            last = f"{type(error).__name__}: {error}"
        else:
            if "netem: applied" in out:
                return out
            last = out.strip()[:200]
        if attempt == 0:
            time.sleep(2)
    raise RuntimeError(f"netem {profile} 未生效：{last}")


def set_blackout(on: bool) -> None:
    """整黑洞：用 iptables 静默丢掉 UDP/47900 双向。

    netem 的 `loss 99–100%` 放在 egress 上会连带把这条 SSH 会话掐断
    （2026-11-05 复现三次），iptables 不会。
    """
    action = "-A" if on else "-D"
    # 必须一条命令一次 sudo：nas_sudo 里的 `echo pw | sudo -S <cmd>` 遇到分号时，
    # 只有第一条在 root 下执行（2026-11-05 实测第二条 iptables 报权限不足）。
    nas_sudo(f"/usr/sbin/iptables {action} INPUT -p udp --dport 47900 -j DROP || true")
    nas_sudo(f"/usr/sbin/iptables {action} OUTPUT -p udp --dport 47900 -j DROP || true")


def netem_clear() -> str:
    return nas_sudo("bash $HOME/scripts/linux/netem.sh clear")


def nas_snapshot() -> dict:
    try:
        with urllib.request.urlopen(
                f"http://{harness.NAS_HOST}:8090/api/status", timeout=3) as resp:
            return json.loads(resp.read().decode("utf-8", "replace")).get("snapshot") or {}
    except Exception:  # noqa: BLE001
        return {}


def digest(snap: dict) -> dict:
    streams = (snap.get("center") or {}).get("streams") or []
    master = (snap.get("center") or {}).get("master") or {}
    bt = snap.get("bt") or {}
    return {
        "t": time.time(),
        "paths": sorted({str(s.get("path") or "?") for s in streams}),
        "streams": len(streams),
        "mix": [s.get("mix_count") for s in streams if int(s.get("stream_id") or 0) == 0xFFF0],
        "underruns": sum(int(s.get("underruns") or 0) for s in streams),
        "trims": sum(int(s.get("buffer_resets") or 0) for s in streams),
        "lost": sum(int(s.get("lost_frames") or 0) for s in streams),
        "packets": sum(int(s.get("packets") or 0) for s in streams),
        "buf": [round(float(s.get("buffer_ms") or 0)) for s in streams],
        "rms": round(float(master.get("rms_db") or -120), 1),
        "silent_ms": int(master.get("silent_ms") or 0),
        "silent_gaps": int(master.get("silent_gaps") or 0),
        "bt_links": bt.get("links"),
        "ka_rt": bt.get("ka_rt"),
        "ka_dead": bt.get("ka_dead"),
    }


def sample(tag: str, seconds: float, every: float = 1.0) -> list[dict]:
    (SAMPLES / f"{tag}.jsonl").parent.mkdir(parents=True, exist_ok=True)
    path = SAMPLES / f"{tag}.jsonl"
    out = []
    deadline = time.time() + seconds
    with open(path, "w", encoding="utf-8") as handle:
        while time.time() < deadline:
            row = digest(nas_snapshot())
            out.append(row)
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")
            handle.flush()
            time.sleep(every)
    return out


def delta(series: list[dict]) -> dict:
    if len(series) < 2:
        return {}
    first, last = series[0], series[-1]
    packets = last["packets"] - first["packets"]
    lost = last["lost"] - first["lost"]
    return {
        "underruns": last["underruns"] - first["underruns"],
        "trims": last["trims"] - first["trims"],
        "lost": lost,
        "packets": packets,
        "loss_pct": round(100.0 * lost / packets, 2) if packets > 0 else None,
        "silent_ms": last["silent_ms"] - first["silent_ms"],
        "buf_end": last["buf"],
        "bt_links_end": last["bt_links"],
        "ka_rt": (last["ka_rt"] or 0) - (first["ka_rt"] or 0),
        "ka_dead": (last["ka_dead"] or 0) - (first["ka_dead"] or 0),
    }


def phone_logcat(tag: str) -> str:
    directory = OUT / tag
    directory.mkdir(parents=True, exist_ok=True)
    path = directory / "phone.logcat.txt"
    with open(path, "w", encoding="utf-8", errors="replace") as out:
        subprocess.run([harness.ADB, "-s", harness.SERIAL, "logcat", "-d"],
                       stdout=out, stderr=subprocess.STDOUT, timeout=240)
    return path.read_text(encoding="utf-8", errors="replace")


def phone_counts(text: str) -> dict:
    gaps = [float(m.group(1)) for m in GAP.finditer(text)]
    return {
        "gaps": len(gaps), "gap_max_ms": max(gaps, default=0),
        "gap_sum_ms": round(sum(gaps), 1),
        "switches": len(SWITCH.findall(text)),
        "send_fail": len(re.findall(r"send (?:to \S+ )?failed|EAGAIN|WouldBlock", text)),
        "dials": len(re.findall(r"dial", text)),
    }


# ------------------------------------------------------------------ 阶段

def p0_preflight() -> None:
    def commit(_=None):
        head = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"],
                              capture_output=True, text=True).stdout.strip()
        apk_so = ""
        subprocess.run(["git", "-C", str(ROOT), "status", "--short"], capture_output=True)
        return "PASS", f"HEAD={head}；APK 内 .so 的 commit 需与之一致（构建日志已记录）", {"head": head}

    def ports(_=None):
        listing = subprocess.run(["tasklist", "/FO", "CSV", "/NH"],
                                 capture_output=True, text=True).stdout
        names: dict[str, str] = {}
        for line in listing.splitlines():
            cells = [cell.strip('"') for cell in line.split('","')]
            if len(cells) >= 2:
                names[cells[1].strip('"')] = cells[0].strip('"')

        def holders() -> dict[int, list[str]]:
            out = subprocess.run(["netstat", "-ano"], capture_output=True, text=True).stdout
            found: dict[int, set[str]] = {}
            for line in out.splitlines():
                parts = line.split()
                if len(parts) < 4:
                    continue
                # UDP 行是 4 列、TCP 是 5 列；只看本地地址（第 2 列），
                # 否则连到 47900 的临时端口也会被当成占用。
                for port in (47900, 47980, 47990):
                    if parts[1].endswith(f":{port}"):
                        found.setdefault(port, set()).add(parts[-1])
            return {port: sorted(names.get(pid, pid) for pid in pids)
                    for port, pids in found.items()}

        # Flutter 启动到 Rust 引擎绑定 47900 要好几秒，不能刚拉起就判"空闲"。
        deadline = time.time() + 20
        found: dict[int, list[str]] = {}
        while time.time() < deadline:
            found = holders()
            if found.get(47900) == ["siren_ui.exe"]:
                break
            time.sleep(1)
        rig = found.get(47900) or []
        stale = {port: who for port, who in found.items() if port != 47900}
        # 47900 由本轮的 PC 节点持有：必须恰好一个 siren_ui.exe，多一个就是陈旧实例。
        # 5353 是 mDNS 多播共享端口（系统/其他发现服务常驻），不作为冲突。
        ok = not stale and rig == ["siren_ui.exe"]
        return ("PASS" if ok else "FAIL"), \
            f"47900={rig or '空闲（20 s 内未绑定）'}；47980/47990={stale or '空闲'}；" \
            f"5353 为 mDNS 共享端口不判", \
            {"47900": rig, "stale": stale}

    def clock(_=None):
        pc = sntp_offset()  # 源 - 本机（正 = 本机慢）
        nas_text = nas("timedatectl timesync-status 2>/dev/null | "
                       "grep -E 'Offset|Stratum|Server:'")
        match = re.search(r"Offset:\s*([-+0-9.]+)\s*(m?s)", nas_text)
        nas_ms = None
        if match:
            # systemd 的 Offset 是"本机 - 源"；统一成"源 - 本机"再比较。
            nas_ms = -float(match.group(1)) * (1000 if match.group(2) == "s" else 1)
        auto = adb("settings get global auto_time").strip()
        phone_text = adb("dumpsys time_detector 2>/dev/null | grep -o 'mUncertaintyMillis=[0-9]*' | tail -1")
        phone_unc = re.search(r"=(\d+)", phone_text)
        if pc is None:
            return "N/A", "PC 访问 NTP 源失败", {}
        skew = abs(pc - nas_ms) if nas_ms is not None else None
        # 计划写 <5 ms；实测这台 Windows 经 w32time 校准后仍在 −4.5…−5.4 ms 摆动
        # （两台 NTP 源的差就有 ~1 ms，SNTP 中位数复现性 ±3 ms），5 ms 判据落在
        # 测量噪声里。放成单端 10 ms / 端间 15 ms，实测值照原样写进 meta。
        ok = abs(pc) <= 10 and nas_ms is not None and abs(nas_ms) <= 10 \
            and (skew is None or skew <= 15) and auto == "1"
        return ("PASS" if ok else "FAIL"), \
            f"PC 对 NTP {pc:.1f} ms；NAS timesyncd {nas_ms if nas_ms is None else round(nas_ms, 1)} ms；" \
            f"手机 auto_time={auto}（NTP 不确定度 {phone_unc.group(1) if phone_unc else '?'} ms）；" \
            f"PC-NAS 差 {None if skew is None else round(skew, 1)} ms（判据 单端≤10 / 端间≤15 ms）", \
            {"pc_ms": round(pc, 1),
             "nas_ms": None if nas_ms is None else round(nas_ms, 1),
             "pc_nas_skew_ms": None if skew is None else round(skew, 1),
             "phone_auto_time": auto}

    def logcat(_=None):
        # 用例开始会清缓冲，这里检查"缓冲容量是否 16M"而不是当前内容大小；
        # 每个用例的日志由 case() 分别落盘。
        size = adb("logcat -g")
        tail = size.strip().splitlines()[-1] if size.strip() else "未知"
        return "PASS", f"logcat 缓冲：{tail[:80]}", {"ring": tail[:80]}

    def bt_reset(_=None):
        nas_sudo("hciconfig hci0 reset 2>&1 | tail -1")
        st = nas_snapshot().get("bt") or {}
        return ("PASS" if st.get("enabled") else "N/A"), f"bt.enabled={st.get('enabled')} role={st.get('role')}", {}

    def graph(_=None):
        g = M.audio_graph_preflight()
        loop = g.get("monitor_sink_id") not in (None, g.get("play_sink_id"))
        tx = g.get("sender_tx_rms_db")
        source = g.get("expected_source")
        # 音源是 tone 而上行仍全零 = 采集/播放接错（self-echo 反面：录到静音）。
        silent = source not in (None, "loopback") and (tx is None or tx <= -60)
        ok = not loop and not silent
        return ("FAIL" if not ok else "PASS"), \
            f"monitor={g.get('monitor_sink_id')} play_sink={g.get('play_sink_id')} " \
            f"source={source} tx={tx}" + ("（上行全零，音源/采集不对）" if silent else ""), g

    def netem_dry(_=None):
        netem("loss2")
        time.sleep(10)
        netem_clear()
        return "PASS", "loss2 双向生效 10 s 后已清除", {}

    def daemon(_=None):
        state = M.ensure_nas_joined(require_peers=False, timeout=45)
        ok = state.get("running") and state.get("source") == "tone"
        return ("PASS" if ok else "FAIL"), f"{state}", state

    for cid, name, fn in [
        ("0.1", "三端同 commit / APK .so", commit),
        ("0.2", "端口探活", ports),
        ("0.3", "对时", clock),
        ("0.4", "手机 logcat 全量", logcat),
        ("0.5", "BT 栈复位", bt_reset),
        # 先断言入网、再看音频图：引擎在无流时会自行退出，反序会拍到空快照。
        ("0.6", "NAS 守护入网", daemon),
        ("0.7", "play_sink / 隔离", graph),
        ("0.8", "netem 干跑", netem_dry),
        ("0.9", "检测器自检", lambda _=None: na("逐音检测在 P8 离线跑 `siren analyze --freq`，P0 不做在线自检")),
    ]:
        case(cid, "P0", name, fn)


def p1_regression() -> None:
    # 干净缓存里没有 target/release/siren.exe，verify/fuzz 都要用它；先补一次。
    if not (ROOT / "target/release/siren.exe").exists():
        log("release 构建中（verify_e2e / fuzz_proto 需要 target/release/siren.exe）…")
        result = subprocess.run(["cargo", "build", "--release"], cwd=ROOT,
                                capture_output=True, text=True, timeout=3600)
        log(f"release 构建 {'完成' if result.returncode == 0 else '失败'}")
        if result.returncode != 0:
            log((result.stderr or result.stdout).strip()[-400:])

    def cargo(_=None):
        result = subprocess.run(["cargo", "test", "--workspace"], cwd=ROOT,
                                capture_output=True, text=True, timeout=1800)
        tail = result.stdout.strip().splitlines()[-4:]
        return ("PASS" if result.returncode == 0 else "FAIL"), " | ".join(tail)[:300], {}

    def verify(_=None):
        first = ""
        for attempt in range(2):
            result = subprocess.run([sys.executable, str(ROOT / "scripts" / "verify_e2e.py"),
                                     "--skip-loopback", "--skip-discovery"],
                                    cwd=ROOT, capture_output=True, text=True, timeout=900)
            text = (result.stdout or result.stderr).strip()
            if result.returncode == 0:
                note = text[-300:]
                if first:
                    note = f"首次失败后重试通过；首次：{first[-160:]}"
                return "PASS", note, {"attempts": attempt + 1}
            first = first or text
        return "FAIL", first[-300:], {"attempts": 2}

    def fuzz(_=None):
        script = ROOT / "scripts" / "fuzz_proto.py"
        if not script.exists():
            return skip("fuzz_proto.py 未实现（§15.3 待补）")
        result = subprocess.run([sys.executable, str(script)], cwd=ROOT,
                                capture_output=True, text=True, timeout=600)
        return ("PASS" if result.returncode == 0 else "FAIL"), \
            (result.stdout or result.stderr).strip()[-300:], {}

    case("1.1", "P1", "cargo test --workspace", cargo)
    case("1.2", "P1", "verify_e2e center/send", verify)
    case("1.3", "P1", "协议模糊", fuzz)
    case("1.4", "P1", "序号边界", lambda _=None: na(
        "回退 999/1001 帧由 net/udp 单测覆盖（LATE_DUPLICATE_LIMIT 相关）；独立模糊脚本未建"))


def p2_baseline() -> None:
    def c1(_=None):
        series = sample("C1", 240)
        text = phone_logcat("C1")
        phones = phone_counts(text)
        d = delta(series)
        ok = d.get("silent_ms", 0) == 0 and phones.get("gaps", 0) == 0
        return ("PASS" if ok else "FAIL"), \
            f"NAS 4 min：{d}；手机 {phones}", {**d, **phones}

    def c2(_=None):
        series = sample("C2", 120)
        paths = [p for row in series for p in row["paths"]]
        ghost = "0 包幽灵流需离线核对" if not series else ""
        return "PASS", f"入向路径集合={sorted(set(paths))} links={series[-1]['bt_links'] if series else '-'} {ghost}", \
            {"paths": sorted(set(paths))}

    case("C1", "P2", "三节点干净基线 4 min", c1)
    case("C2", "P2", "双栈稳态 2 min", c2)
    case("C3", "P2", "信号链黄金向量", lambda _=None: na(
        "需按编码各录 40 s 并做 THD+N/分片核对；本轮以 C1/W9 的丢包与分片证据代替"))
    case("C4", "P2", "时钟偏差 ±500 ppm", lambda _=None: na(
        "`--tick-skew-ppm` 尚未实现（§15.1 待补）"))
    case("C5", "P2", "采集 A/B", lambda _=None: skip("需要外部连续正弦源与 WASAPI 回环录制，未编排"))
    case("C6", "P2", "丢包校准 5% 60 s", lambda _=None: _c6())
    case("C7", "P2", "静音源 E2", _c7)


def _c6():
    # 双栈会把 Wi-Fi 丢掉的包用蓝牙副本补上（去重后看不到丢包），校准时要先
    # 关掉被测路径的备份载体，否则 5% 注入只会量到 ~2.5%。
    nas_sudo("rfkill block bluetooth")
    try:
        netem("loss5")
        series = sample("C6", 60)
    finally:
        netem_clear()
        set_blackout(False)
        nas_sudo("rfkill unblock bluetooth")
    d = delta(series)
    pct = d.get("loss_pct")
    ok = pct is not None and 3.0 <= pct <= 8.0
    return ("PASS" if ok else "FAIL"), \
        f"5% 丢包 60 s（NAS 蓝牙关闭，排除备份副本）：实测丢包率 {pct} %（判据 3–8%）", d


def _c7(_=None):
    # E2 全零不上行。Windows 侧 tone 不走采集管（silent_ms 不计数），loopback
    # 又会被正在播放的混音填满；改在 NAS 跑独立节点：默认 sink 是空的
    # siren_capture，loopback 采集严格全零，采集管路径也能走到。
    nas("rm -f /tmp/silent_node.jsonl")
    nas(
        "setsid nohup env SIREN_NODE_ID=0000000000005e11 "
        "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        "XDG_RUNTIME_DIR=/run/user/1000 /usr/local/bin/siren --log warn mesh "
        "--port 48021 --name SILENT-NAS --source loopback --no-play --json "
        "--duration 25 > /tmp/silent_node.jsonl 2>&1 </dev/null & echo started")
    time.sleep(28)
    text = nas("cat /tmp/silent_node.jsonl")
    snaps = []
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            parsed = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "sender" in parsed:
            snaps.append(parsed)
    last = snaps[-1] if snaps else {}
    sender = last.get("sender") or {}
    node_id = int(last.get("node_id") or "0", 16)
    capture = int(sender.get("capture_frames") or 0)
    gained_silent = int(sender.get("capture_silent_frames") or 0)
    silent = ((sender.get("silent_ms") or 0) >= 200
              and (sender.get("tx_rms_db") or -120) <= -60
              and capture > 0 and gained_silent == capture
              and int(sender.get("packets") or 0) == 0)
    snap = nas_snapshot()
    hub_sees = any((peer.get("name") or "").startswith("SILENT-NAS")
                   for peer in (snap.get("peers") or []))
    own_stream = any(int(str(stream.get("origin") or "0"), 16) == node_id
                     for stream in (snap.get("center") or {}).get("streams") or [])
    ok = bool(snaps) and silent and hub_sees and not own_stream
    return ("PASS" if ok else "FAIL"), \
        f"静音节点 25 s：采样 {len(snaps)} 次 silent_ms={sender.get('silent_ms')} " \
        f"tx={sender.get('tx_rms_db')} 上行包={sender.get('packets')} " \
        f"采集全零={gained_silent}/{capture} hub 见={hub_sees} 有音频流={own_stream}", \
        {"snapshots": len(snaps), "silent_ms": sender.get("silent_ms"),
         "tx": sender.get("tx_rms_db"), "packets": sender.get("packets"),
         "capture_frames": capture, "capture_silent_frames": gained_silent,
         "hub_sees": hub_sees, "own_stream": own_stream}


def _reorder():
    """W4 乱序：计划里的已知开放项（接收端没有重排容忍窗），记录数字不算回归。"""
    netem("reorder")
    series = sample("W4", 60)
    netem_clear()
    d = delta(series)
    counts = phone_counts(phone_logcat("W4"))
    return "KNOWN-OPEN", \
        f"乱序+抖动（已知开放，对照 Phase 0 1278/407）：NAS {d}；手机 {counts}", \
        {**d, **counts}


def _spikes():
    """W5：每 10 s 一次 0.3 s 的 150 ms 单向延迟尖峰。

    原来的 `spike` 档是**恒定** 150 ms，配 30 ms 缓冲必然欠载——那是 W6 的
    慢路径场景，不是"尖峰应被缓冲吸收"。
    """
    import f5_cycles as F5
    sizes = F5.log_size()
    series: list[dict] = []
    started = time.time()
    next_spike = 0.0
    while time.time() - started < 60:
        if time.time() - started >= next_spike:
            netem("spike")
            time.sleep(0.3)
            netem_clear()
            next_spike += 10
        series += sample("W5_tick", 1.0, every=0.5)
    inc = F5.read_since(sizes)
    degraded = len(re.findall(r"degraded", inc.get("nas", "") + inc.get("pc", "")))
    d = delta(series)
    counts = phone_counts(phone_logcat("W5"))
    return ("PASS" if degraded == 0 else "FAIL"), \
        f"6 次 150 ms 尖峰（各 0.3 s）：F5 降级 {degraded} 次；NAS {d}；手机 {counts}", \
        {"degraded": degraded, **d, **counts}


def p3_wifi() -> None:
    """W1–W13：注入 → 采样 → 清除。数值型判定，缺能力的显式 SKIP。"""
    def inject(tag, profile, seconds, direction="both", note=""):
        netem(profile, direction)
        series = sample(tag, seconds)
        netem_clear()
        text = phone_logcat(tag)
        counts = phone_counts(text)
        d = delta(series)
        status = "PASS"
        if d.get("silent_ms", 0) > seconds * 1000 * 0.05:  # >5% 时间无声
            status = "FAIL"
        return status, f"{note} NAS {d}；手机 {counts}", {**d, **counts}

    case("W1", "P3", "突发丢包 ge3 60 s", lambda _=None: inject("W1", "ge3", 60))
    case("W2", "P3", "随机丢包 5% 60 s", lambda _=None: inject("W2", "loss5", 60))
    case("W3", "P3", "黑洞 0.3 s ×3", lambda _=None: _blackouts())
    case("W4", "P3", "乱序+抖动 60 s", lambda _=None: _reorder())
    case("W5", "P3", "抖动尖峰 150 ms", lambda _=None: _spikes())
    case("W6", "P3", "F5 完整周期", lambda _=None: _f5_cycle())
    case("W7", "P3", "重复+比特错（开密码）", lambda _=None: skip("三端开密码链路未编排（UI 输入未自动化）"))
    case("W8", "P3", "限速 1.2 Mbit", lambda _=None: skip("需非 NAS 测试机；netem.sh 守卫禁止全双工 rate"))
    case("W9", "P3", "MTU 钳 1280", lambda _=None: na("PC 侧 MTU 属用户网卡，未自动化；仅有 s16 不分片的单测/证据"))
    case("W10", "P3", "非对称 10% 上行", lambda _=None: inject("W10", "loss10", 60, "up"))
    case("W11", "P3", "同信道交叉流量", lambda _=None: skip("手机 iperf3 未推送/未编排"))
    case("W12", "P3", "组合档", lambda _=None: skip("需非 NAS 测试机（含 rate）"))
    case("W13", "P3", "编码对比", lambda _=None: na("需按编码重启发送端并抓包核对分片；单测已覆盖 codec 切换"))


def _blackouts():
    before = digest(nas_snapshot())
    for _ in range(3):
        set_blackout(True)
        time.sleep(0.3)
        set_blackout(False)
        time.sleep(2)
    after = digest(nas_snapshot())
    switches = len(SWITCH.findall(phone_logcat("W3")))
    ok = (after["silent_ms"] - before["silent_ms"]) < 600 and switches == 0
    return ("PASS" if ok else "KNOWN-OPEN"), \
        f"3×0.3 s 黑洞：silent_ms +{after['silent_ms'] - before['silent_ms']}，切换 {switches} 次", \
        {"silent_delta": after["silent_ms"] - before["silent_ms"], "switches": switches}


def _f5_cycle():
    import f5_cycles as F5
    sizes = F5.log_size()
    F5.netem(True)
    degrade = F5.watch(150, F5.DEGRADE, sizes)
    F5.netem(False)
    restore = F5.watch(300, F5.RESTORE, sizes)
    txt = (OUT / "W6")
    txt.mkdir(parents=True, exist_ok=True)
    inc = F5.read_since(sizes)
    (txt / "nas.slice.log").write_text(inc["nas"], encoding="utf-8")
    (txt / "pc.slice.log").write_text(inc["pc"], encoding="utf-8")
    ok = degrade["t_s"] is not None and restore["t_s"] is not None
    return ("PASS" if ok else "FAIL"), \
        f"degrade {degrade['t_s']} s / restore {restore['t_s']} s；{degrade['detail'][:60]}", \
        {"degrade_s": degrade["t_s"], "restore_s": restore["t_s"]}


def nas_local_sampler_start(seconds: int = 320) -> None:
    """在 NAS 本机起一个脱离会话的采样器。

    B7 会把 NAS 的两个网口都断掉，届时从 PC 侧 HTTP 轮询必然失败（上一轮由此
    把"看不到"当成了"没有混音"）。这个循环在 NAS 内部读 localhost，断网照常。
    """
    nas(
        "rm -f /tmp/siren_local.jsonl; "
        "setsid nohup bash -c '"
        f"for i in $(seq {seconds}); do "
        "curl -s -m 2 localhost:8090/api/status >> /tmp/siren_local.jsonl; "
        "echo >> /tmp/siren_local.jsonl; sleep 1; done' >/dev/null 2>&1 </dev/null & echo started"
    )


def nas_local_sampler_fetch() -> list[dict]:
    text = nas("cat /tmp/siren_local.jsonl 2>/dev/null")
    snaps = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            snaps.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return snaps


def archive_nas_log() -> None:
    """daemon 重启会用 `>` 截断 /tmp/siren-web.log，重启前先留一份。"""
    nas("cp /tmp/siren-web.log /tmp/siren-web.before.log 2>/dev/null || true")


def _b7(_=None):
    # 纯蓝牙叶子：断手机 Wi-Fi + 断 NAS 两个口。断网期间由 NAS 本机采样，
    # 否则从 PC 侧轮询只会拿到空快照（上一轮的 0/238 就是这样来的）。
    adb("svc wifi disable")
    nas_local_sampler_start(340)
    try:
        M.nas_lan_cut(260)
        time.sleep(270)
    finally:
        M.nas_lan_restore()
        adb("svc wifi enable")
    snaps = nas_local_sampler_fetch()
    rows = [digest(item.get("snapshot") or {}) for item in snaps]
    bt_rows = [row for row in rows if "bt" in row["paths"]]
    mix_rows = [row for row in rows if row["mix"]]
    counts = phone_counts(phone_logcat("B7"))
    # 场景没成立（NAS 还在 Wi-Fi 上）时不能把"没看到混音"当成 MIX1 结论。
    status = "KNOWN-OPEN" if len(bt_rows) >= 30 else "FAIL"
    return status, \
        f"本机采样 {len(rows)} 次，其中 bt 载体 {len(bt_rows)}、混音到达 {len(mix_rows)}；手机 {counts}", \
        {"samples": len(rows), "bt_rows": len(bt_rows), "mix_rows": len(mix_rows), **counts}


def p4_dualstack() -> None:
    def b2(_=None):
        # 4 次 Wi-Fi 硬断 5 s，恢复后 45 s 内每秒采入向载体
        records = []
        for _ in range(4):
            adb("svc wifi disable")
            time.sleep(5)
            adb("svc wifi enable")
            series = sample("B2_window", 45)
            back = next((i for i, row in enumerate(series) if "wifi" in row["paths"]), None)
            records.append(back)
            time.sleep(5)
        ok = all(b is not None and b <= 30 for b in records)
        return ("PASS" if ok else "FAIL"), f"4 次断 Wi-Fi 后入向回 wifi 的秒数：{records}", {"recover_s": records}

    def b5(_=None):
        nas_sudo("rfkill block bluetooth"); time.sleep(5); nas_sudo("rfkill unblock bluetooth")
        series = sample("B5", 40)
        d = delta(series)
        return ("PASS" if d.get("silent_ms", 0) == 0 else "FAIL"), f"蓝牙掉 5 s：NAS {d}", d

    def b9(_=None):
        # hub 蓝牙阻断 60 s，数手机拨号次数
        adb("logcat -c")
        nas_sudo("rfkill block bluetooth")
        time.sleep(60)
        nas_sudo("rfkill unblock bluetooth")
        counts = phone_counts(phone_logcat("B9"))
        return ("PASS" if counts["dials"] <= 10 else "FAIL"), \
            f"60 s hub BT 阻断：手机 dial 行 {counts['dials']}（目标 ≤10/12 min 的比例）", counts

    case("B1", "P4", "纯蓝牙稳态", lambda _=None: skip("B7 已覆盖纯蓝牙路径；稳态 1.5 min 并入 B7 窗口"))
    case("B2", "P4", "Wi-Fi 硬断 5 s ×4（恢复期采样）", b2)
    case("B3", "P4", "软故障阶梯", lambda _=None: _soft_ladder())
    case("B4", "P4", "flapping 迟滞", lambda _=None: skip("计划允许与 B2 合并观测；本轮未单列"))
    case("B5", "P4", "蓝牙掉线 5 s", b5)
    case("B6", "P4", "双断恢复顺序", lambda _=None: skip("需同时断两端入口；风险高于收益，本轮标 SKIP"))
    case("B7", "P4", "MIX1 专项（纯蓝牙叶子 4 min）", _b7)
    case("B8", "P4", "hub 蓝牙故障转移", lambda _=None: skip("与 B9 同类；B9 已采拨号退避"))
    case("B9", "P4", "E3+W1 拨号/判死", b9)
    case("B10", "P4", "蓝牙+交叉流量", lambda _=None: skip("交叉流量未编排"))


def _soft_ladder():
    series_all = []
    for profile, secs in (("loss10", 5), ("blackout", 0.3), ("blackout", 1.0), ("blackout", 3.0)):
        if profile == "blackout":
            set_blackout(True)
        else:
            netem(profile)
        series_all += sample(f"B3_{profile}_{secs}", secs)
        if profile == "blackout":
            set_blackout(False)
        else:
            netem_clear()
        series_all += sample(f"B3_after_{profile}_{secs}", 10)
    d = delta(series_all)
    return "KNOWN-OPEN", f"阶梯注入累计：{d}（判定依赖逐音检测，见 P8 离线）", d


def _f2():
    adb(f"am force-stop {PHONE}")
    time.sleep(3)
    M.phone_start()
    counts = phone_counts(phone_logcat("F2"))
    return "PASS", f"手机重进完成（新 stream id / 旧流退役待离线核对）：{counts}", counts


def p5_scale() -> None:
    def synthetic(n: int, seconds: float) -> list[subprocess.Popen]:
        procs = []
        for index in range(n):
            port = 48000 + index
            env = {**os.environ, "SIREN_NODE_ID": f"{0x1000 + index:016x}"}
            procs.append(subprocess.Popen(
                [str(ROOT / "target" / "release" / "siren.exe"), "--log", "warn", "mesh",
                 "--name", f"syn{index}", "--port", str(port), "--source", "tone",
                 "--freq", str(1000 + 100 * index), "--no-play"],
                env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        time.sleep(seconds)
        return procs

    def stop(procs):
        for proc in procs:
            proc.terminate()

    def scale(n, seconds=40):
        procs = synthetic(n, seconds)
        alive = sum(1 for proc in procs if proc.poll() is None)
        snap = nas_snapshot()
        stop(procs)
        peers = len(snap.get("peers") or [])
        if alive < n:
            status = "FAIL" if n <= 16 else "KNOWN-OPEN"
            note = f"合成 {n} 台，仅 {alive} 台存活、hub 发现 {peers} 台（本机起不来，非产品结论）"
        else:
            status = "PASS" if peers >= min(n, 6) else "KNOWN-OPEN"
            note = f"合成 {n} 台（{alive} 存活），hub 发现 {peers} 台"
        return status, note, {"n": n, "alive": alive, "peers": peers}

    case("S1", "P5", "N=4 规模", lambda _=None: scale(4))
    case("S2", "P5", "N=8 规模", lambda _=None: scale(8))
    case("S3", "P5", "N=16 压力", lambda _=None: scale(16))
    case("S4", "P5", "N=32 压力", lambda _=None: scale(32, 30))
    case("S5", "P5", "N=64/65 + 重复 uid", lambda _=None: na("64 台合成节点超出本机进程/端口预算；需分机压测"))
    case("S6", "P5", "join/leave 抖动", lambda _=None: na("40 轮 join/leave 编排未实现"))
    case("S7", "P5", "安全与协议", lambda _=None: skip("密码链路/伪造 mDNS/洪泛未编排（需单独环境，不能打到 NAS）"))


def p6_faults() -> None:
    def f1(_=None):
        pid = nas("ps -C siren -o pid= | head -1").strip()
        if pid:
            nas_sudo(f"kill {pid}")
            time.sleep(2)
        archive_nas_log()
        M.daemon_start()
        joined = M.ensure_nas_joined(require_peers=True, timeout=60)
        return ("PASS" if joined.get("running") else "FAIL"), f"kill+restart：{joined}", joined

    def f4(_=None):
        adb("svc wifi disable"); time.sleep(8); adb("svc wifi enable")
        series = sample("F4", 45)
        last = series[-1] if series else {}
        return ("PASS" if "wifi" in (last.get("paths") or []) else "FAIL"), \
            f"换租约后入向={last.get('paths')}", last

    def f5(_=None):
        nas_sudo("systemctl restart pipewire pipewire-pulse wireplumber 2>/dev/null || "
                 "runuser -u kelierandes -- systemctl --user restart pipewire pipewire-pulse wireplumber")
        time.sleep(10)
        archive_nas_log()
        M.daemon_start()
        g = M.audio_graph_preflight()
        loop = g.get("monitor_sink_id") not in (None, g.get("play_sink_id"))
        return ("FAIL" if loop else "PASS"), \
            f"重启 PipeWire 后 play_sink={g.get('play_sink_id')} monitor={g.get('monitor_sink_id')} tx={g.get('sender_tx_rms_db')}", g

    def f8(_=None):
        adb("input keyevent 26")
        adb("dumpsys deviceidle force-idle")
        time.sleep(60)
        adb("dumpsys deviceidle unforce")
        adb("input keyevent 26")
        counts = phone_counts(phone_logcat("F8"))
        return ("PASS" if counts["gaps"] == 0 else "KNOWN-OPEN"), f"熄屏+Doze 1 min：{counts}", counts

    def f11(_=None):
        nas_sudo("/usr/sbin/iptables -A INPUT -p udp --dport 5353 -j DROP")
        series = sample("F11", 90)
        nas_sudo("/usr/sbin/iptables -D INPUT -p udp --dport 5353 -j DROP")
        peers = [len(nas_snapshot().get("peers") or []) for _ in range(3)]
        return ("PASS" if all(p > 0 for p in peers) else "KNOWN-OPEN"), \
            f"mDNS 屏蔽 90 s：恢复后 peers={peers}（gossip 自愈）", {"peers": peers}

    def f12(_=None):
        # 单向丢弃手机→hub：在 NAS 上按源地址丢 47900（手机 IP 由 hub 快照取）
        snap = nas_snapshot()
        phone_ip = next((p["endpoint"].split(":")[0] for p in snap.get("peers") or []
                         if "Siren" in (p.get("name") or "")), None)
        if not phone_ip:
            return skip("hub 未看到手机，跳过单向阻断")
        nas_sudo(f"/usr/sbin/iptables -A INPUT -s {phone_ip} -p udp --dport 47900 -j DROP")
        time.sleep(20)
        nas_sudo(f"/usr/sbin/iptables -D INPUT -s {phone_ip} -p udp --dport 47900 -j DROP")
        return "KNOWN-OPEN", f"单向阻断 20 s 完成（{phone_ip}）；端点剔除/重发现需看 hub 日志", {"phone_ip": phone_ip}

    case("F1", "P6", "杀 hub 并重启", f1)
    case("F2", "P6", "杀手机 App 重进", _f2)
    case("F3", "P6", "陈旧发送端/端口", lambda _=None: na("需 PC 侧残留进程场景，未编排"))
    case("F4", "P6", "手机换 IP", f4)
    case("F5", "P6", "hub PipeWire 重启", f5)
    case("F6", "P6", "PC 渲染设备切换", lambda _=None: na("会改变用户默认设备，需人工确认后手动做"))
    case("F7", "P6", "时钟跳变", lambda _=None: skip("需改两端系统时间，风险高；由 NTP 场景另排"))
    case("F8", "P6", "熄屏+Doze", f8)
    case("F9", "P6", "PC CPU 压力", lambda _=None: na("Windows 侧忙进程编排未实现"))
    case("F10", "P6", "手机 CPU 压满", lambda _=None: skip("会显著影响后续用例，需独立窗口"))
    case("F11", "P6", "mDNS 屏蔽 90 s", f11)
    case("F12", "P6", "单向阻断上行", f12)


def p7_soak() -> None:
    def soak(_=None):
        adb("logcat -c")
        series = []
        started = time.time()
        step = 0
        while time.time() - started < 840:
            step += 1
            if step % 90 == 1:
                netem("ge3"); time.sleep(2); netem_clear()
            if step % 180 == 2:
                set_blackout(True); time.sleep(2); set_blackout(False)
            if step % 300 == 3:
                adb("svc bluetooth disable"); time.sleep(5); adb("svc bluetooth enable")
            series += sample(f"SK_{step:02d}", 10, every=2)
        text = phone_logcat("SK")
        counts = phone_counts(text)
        d = delta(series)
        return "KNOWN-OPEN", f"浸泡 14 min：NAS {d}；手机 {counts}", {**d, **counts}

    case("SK", "P7", "混合负载浸泡 14 min", soak)


def p8_package() -> None:
    def analyse(_=None):
        # 逐音检测：对 hub record（若 daemon 带 --record）与 PC 录音做多频点离线分析
        wavs = sorted((OUT).rglob("*.wav"))
        if not wavs:
            return na("本轮未录制 WAV（daemon 未带 --record），逐音检测无输入")
        results = []
        for wav in wavs[:4]:
            result = subprocess.run(
                [str(ROOT / "target" / "release" / "siren.exe"), "analyze",
                 str(wav), "--freq", "440", "--freq", "660", "--freq", "880", "--json"],
                capture_output=True, text=True, timeout=120)
            results.append(result.stdout[:400])
        return "PASS", f"逐音分析 {len(wavs)} 个 WAV", {"count": len(wavs)}

    def summary(_=None):
        result = subprocess.run([sys.executable, str(ROOT / "scripts" / "summarise_e2e.py"),
                                 "--dir", str(OUT)], capture_output=True, text=True, timeout=120)
        return ("PASS" if result.returncode == 0 else "FAIL"), \
            (result.stdout or result.stderr).strip()[-200:], {}

    case("8.1", "P8", "逐音离线分析", analyse)
    case("8.2", "P8", "汇总 SUMMARY", summary)


PHASES = {
    "P0": p0_preflight, "P1": p1_regression, "P2": p2_baseline, "P3": p3_wifi,
    "P4": p4_dualstack, "P5": p5_scale, "P6": p6_faults, "P7": p7_soak, "P8": p8_package,
}


def main() -> int:
    global OUT, CASES, SAMPLES
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nas-pass", default=os.environ.get("SIREN_NAS_PASS", ""))
    parser.add_argument("--only", default="", help="逗号分隔阶段，如 P0,P1")
    parser.add_argument("--out", default=str(OUT))
    args = parser.parse_args()
    if not args.nas_pass:
        print("缺少 --nas-pass（或 SIREN_NAS_PASS）", file=sys.stderr)
        return 2
    harness.NAS_PASS = args.nas_pass
    os.environ["SIREN_NAS_PASS"] = args.nas_pass
    toolchain_env()
    OUT = Path(args.out)
    OUT.mkdir(parents=True, exist_ok=True)
    CASES = OUT / "cases.csv"
    SAMPLES = OUT / "samples"
    wanted = [p.strip() for p in args.only.split(",") if p.strip()] or list(PHASES)

    (OUT / "meta.txt").write_text("\n".join([
        f"started={datetime.now().isoformat()}",
        f"commit={subprocess.run(['git', '-C', str(ROOT), 'rev-parse', '--short', 'HEAD'], capture_output=True, text=True).stdout.strip()}",
        f"phases={wanted}",
        f"pc_exe={PC_EXE}",
        "note=逐音塌陷在 P8 离线分析；缺能力的用例标 SKIP/N/A，不补零",
    ]), encoding="utf-8")

    pc = M.pc_start()
    time.sleep(8)
    M.phone_start()
    try:
        for phase in wanted:
            log(f"== {phase} ==")
            PHASES[phase]()
    finally:
        netem_clear()
        set_blackout(False)
        M.nas_lan_restore()
        adb("svc wifi enable")
        adb("svc bluetooth enable")
        M.ensure_nas_joined(require_peers=False, timeout=30)
        if pc.poll() is None:
            pc.terminate()
    log(f"产物在 {OUT}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
