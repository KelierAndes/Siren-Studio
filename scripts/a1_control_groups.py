#!/usr/bin/env python3
"""E5 / W4：A1（手机 EAGAIN + 双向 ~1.1 s 空洞）的三组对照实验。

原始现场（out/2026-11-04_1h/FINDINGS.md）：手机 `sender: send failed: … EAGAIN)`
对 `.107`（PC）与 `.196`（NAS）两个目的地**同一毫秒**同时失败，说明不是某条路径的
问题，而是手机网卡自己的发送队列被堵住。三组对照用来判它跟什么相关：

- G1 只关蓝牙：`svc bluetooth disable`（可 adb 驱动，实测生效 bluetooth_on=0）
- G2 只让 Wi-Fi 进省电：`settings put global wifi_sleep_policy 1`
      （设备当前是 2=不省电，所以"关省电"这一组没有可关的东西；能做的实验是反过来
       ——打开省电，看 A1 是否被复现。这比原计划更直接）
- G3 两者都做

每组 12 分钟，采：手机 logcat 里 `send failed`/EAGAIN 次数、`audible gap` 次数与
最长/合计、载波切换次数、`dumpsys wifi` 的 RSSI/LinkSpeed 与 roam 计数、
`dumpsys deviceidle` 的唤醒计数增量。基线（都不动）直接引用同日矩阵的 S3 窗口，
不额外占设备时间。

产物：out/a1_groups_<日期>/G*/{phone.logcat.txt,wifi.dump,metrics.json} + verdict.md
收尾一定会把蓝牙与省电策略改回去（finally）。
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from datetime import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import collect_1h as harness  # noqa: E402
import matrix_dualstack as M  # noqa: E402

SEND_FAIL = re.compile(r"send failed")
GAP = M.GAP
SWITCH = M.SWITCH
ROAM = re.compile(r"[Rr]oam(?:ing)?\s*[:=]\s*(\d+)")
WAKEUPS = re.compile(r"total wakeups[:\s]+(\d+)")

GROUPS = [
    ("G1_bt_off", {"bluetooth": False, "wifi_sleep": None}),
    ("G2_wifi_powersave", {"bluetooth": None, "wifi_sleep": "1"}),
    ("G3_both", {"bluetooth": False, "wifi_sleep": "1"}),
]


def adb(cmd: str) -> str:
    return harness.adb_shell(cmd)


def set_state(bluetooth: bool | None, wifi_sleep: str | None) -> None:
    if bluetooth is not None:
        adb(f"svc bluetooth {'enable' if bluetooth else 'disable'}")
    if wifi_sleep is not None:
        adb(f"settings put global wifi_sleep_policy {wifi_sleep}")
    time.sleep(3)


def read_state() -> dict:
    return {
        "bluetooth_on": adb("settings get global bluetooth_on").strip(),
        "wifi_sleep_policy": adb("settings get global wifi_sleep_policy").strip(),
        "wifi_on": adb("settings get global wifi_on").strip(),
    }


def sample_wifi() -> dict:
    """RSSI / 连接速率 / 漫游计数：A1 若是省电或重关联造成，先在这里露出来。"""
    text = adb("dumpsys wifi | grep -m1 -E 'mWifiInfo'")
    out = {"raw": text.strip()[:400]}
    for key, pattern in (("rssi", r"RSSI: (-?\d+)"),
                         ("link_speed", r"Link speed: (\d+)"),
                         ("frequency", r"Frequency: (\d+)")):
        match = re.search(pattern, text)
        out[key] = int(match.group(1)) if match else None
    idle = adb("dumpsys deviceidle | grep -m1 -i 'total wakeups'")
    match = WAKEUPS.search(idle)
    out["deviceidle_wakeups"] = int(match.group(1)) if match else None
    return out


def run_group(name: str, knobs: dict, minutes: float, directory: Path) -> dict:
    directory.mkdir(parents=True, exist_ok=True)
    set_state(knobs.get("bluetooth"), knobs.get("wifi_sleep"))
    state = read_state()
    (directory / "state.json").write_text(
        json.dumps(state, ensure_ascii=False, indent=2), encoding="utf-8")
    time.sleep(20)
    adb("logcat -c")
    wifi_samples = []
    start = time.time()
    while time.time() - start < minutes * 60:
        wifi_samples.append(sample_wifi())
        time.sleep(30)
    (directory / "wifi.samples.json").write_text(
        json.dumps(wifi_samples, ensure_ascii=False, indent=2), encoding="utf-8")
    with open(directory / "phone.logcat.txt", "w", encoding="utf-8", errors="replace") as out:
        import subprocess
        subprocess.run([harness.ADB, "-s", harness.SERIAL, "logcat", "-d"],
                       stdout=out, stderr=subprocess.STDOUT, timeout=240)
    text = (directory / "phone.logcat.txt").read_text(encoding="utf-8", errors="replace")
    gaps = [float(m.group(1)) for m in GAP.finditer(text)]
    return {
        "group": name,
        "state": state,
        "minutes": minutes,
        "send_failed": len(SEND_FAIL.findall(text)),
        "gaps": len(gaps),
        "gap_max_ms": max(gaps, default=0),
        "gap_sum_ms": round(sum(gaps), 1),
        "switches": len(SWITCH.findall(text)),
        "trims": len(M.TRIM.findall(text)),
        "rssi_min": min([s["rssi"] for s in wifi_samples if s.get("rssi") is not None], default=None),
        "link_speed_min": min(
            [s["link_speed"] for s in wifi_samples if s.get("link_speed") is not None], default=None),
        "wakeups_delta": (
            (wifi_samples[-1].get("deviceidle_wakeups") or 0)
            - (wifi_samples[0].get("deviceidle_wakeups") or 0)
        ),
        "samples": len(wifi_samples),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nas-pass", default=os.environ.get("SIREN_NAS_PASS", ""))
    parser.add_argument("--minutes", type=float, default=12.0)
    parser.add_argument("--out", default=str(ROOT / "out" / f"a1_groups_{datetime.now():%Y-%m-%d_%H%M}"))
    args = parser.parse_args()
    if not args.nas_pass:
        print("缺少 --nas-pass（或 SIREN_NAS_PASS）", file=sys.stderr)
        return 2
    harness.NAS_PASS = args.nas_pass
    os.environ["SIREN_NAS_PASS"] = args.nas_pass

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    before = read_state()
    (out / "state.before.json").write_text(
        json.dumps(before, ensure_ascii=False, indent=2), encoding="utf-8")

    M.nas_sh("export XDG_RUNTIME_DIR=/run/user/1000 DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus; "
             "pactl list sinks short | grep -q play_sink || pactl load-module module-null-sink "
             "sink_name=play_sink sink_properties=device.description=play_sink; "
             "pactl set-default-sink play_sink")
    M.daemon_start()
    pc = M.pc_start()
    time.sleep(12)
    M.phone_start()

    rows: list[dict] = []
    try:
        for name, knobs in GROUPS:
            print(f"[{datetime.now():%H:%M:%S}] {name} 开始（{args.minutes} min）", flush=True)
            rows.append(run_group(name, knobs, args.minutes, out / name))
            print(f"[{datetime.now():%H:%M:%S}] {name} → "
                  f"send_failed={rows[-1]['send_failed']} gaps={rows[-1]['gaps']}"
                  f"/{rows[-1]['gap_sum_ms']} ms", flush=True)
            # 组间回到三机正常态，避免上一组的开关影响下一组的入网
            set_state(True, before.get("wifi_sleep_policy") or "2")
            time.sleep(30)
            M.phone_start()
    finally:
        set_state(True, before.get("wifi_sleep_policy") or "2")
        (out / "state.after.json").write_text(
            json.dumps(read_state(), ensure_ascii=False, indent=2), encoding="utf-8")
        harness.adb_shell(f"am force-stop {harness.PHONE_PKG}")
        if pc.poll() is None:
            pc.terminate()
        M.daemon_start()

    (out / "groups.json").write_text(
        json.dumps(rows, ensure_ascii=False, indent=2), encoding="utf-8")
    header = ("| 组 | 蓝牙 on | Wi-Fi 省电策略 | send_failed | 洞次/最长/合计 ms | 切换 | 修剪 | RSSI 最低 | 速率最低 | idle 唤醒增量 |\n"
              "| :-- | :-- | :-- | --: | :-- | --: | --: | --: | --: | --: |\n")
    body = "".join(
        f"| {r['group']} | {r['state']['bluetooth_on']} | {r['state']['wifi_sleep_policy']} | "
        f"{r['send_failed']} | {r['gaps']}/{r['gap_max_ms']:.0f}/{r['gap_sum_ms']} | "
        f"{r['switches']} | {r['trims']} | {r['rssi_min']} | {r['link_speed_min']} | {r['wakeups_delta']} |\n"
        for r in rows)
    verdict = [
        "# A1 三组对照判定",
        "",
        "基线（同日矩阵 S3 三机双栈窗口，不关任何开关）：`send_failed=0`、`audible gap=0` — "
        "见 out/matrix_2026-11-05_1405/S3_tri_dual/phone.logcat.txt。",
        "",
        header + body,
        "",
        "判定规则：某组里 send_failed 与 ~1.1 s 洞同时出现 → 与该开关相关；三组全为 0 → "
        "软件侧无法定位，按 E5 的原计划转固件/AP（不再改软件）。",
    ]
    (out / "verdict.md").write_text("\n".join(verdict) + "\n", encoding="utf-8")
    print("\n".join(verdict))
    print(f"产物在 {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
