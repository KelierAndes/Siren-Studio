#!/usr/bin/env python3
"""Deploy the current tree to the Linux test host and restart the daemon.

Safety rules after the 2026-11-05 hangs:
* the remote build runs `nice -n 19` with `-j 2` — a full-tilt release build
  on all 4 cores while the audio mesh is streaming is a suspected contributor
  to both crashes;
* the daemon is restarted exactly once, as user `kelierandes`, with the
  session env (DBUS_SESSION_BUS_ADDRESS + XDG_RUNTIME_DIR) — a root-owned
  daemon loses the Pulse socket ("Connection refused");
* never run this while a netem rate profile is applied (see netem.sh SAFETY).

Usage: python scripts/deploy_nas.py --password <pw> [--host user@192.168.2.196]
"""

from __future__ import annotations

import argparse
import base64
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HOSTKEY = "SHA256:xvTGpWJxeqqcQb/aPVpkOP6YTMfIq7sl8tWQ4iwTSRU"

DAEMON_CMD = (
    "runuser -u kelierandes -- env DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
    "XDG_RUNTIME_DIR=/run/user/1000 setsid nohup /usr/local/bin/siren --log info web "
    "--listen 0.0.0.0:8090 --isolate pipewire --output-sink play_sink --autostart "
    "--source tone --freq 440 --name Linux-ISO --port 47900 >/tmp/siren-web.log 2>&1 &"
)


def ssh(host: str, password: str, command: str, timeout: float = 600) -> str:
    cmd = ["plink", "-batch", "-ssh", "-hostkey", HOSTKEY, "-pw", password, host, command]
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0 and "password for" not in result.stderr:
        raise RuntimeError(f"ssh failed ({result.returncode}): {result.stderr.strip()[:300]}")
    return result.stdout


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--password", required=True)
    parser.add_argument("--host", default="kelierandes@192.168.2.196")
    parser.add_argument("--build-jobs", type=int, default=2,
                        help="remote build parallelism (default 2 of 4 cores)")
    args = parser.parse_args()

    # 1. Ship the source (tar; the workspace needs ui-flutter/rust to exist).
    tar = ROOT / "target" / "siren-src.tgz"
    tar.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [
            "tar", "czf", str(tar),
            "--exclude=target", "--exclude=dist", "--exclude=.git",
            "--exclude=NVIDIA Corporation",
            "crates", "Cargo.toml", "Cargo.lock", "scripts", "ui-flutter/rust",
        ],
        cwd=ROOT, check=True,
    )
    print("uploading source ...", flush=True)
    upload = subprocess.run(
        ["pscp", "-batch", "-hostkey", HOSTKEY, "-pw", args.password,
         str(tar), f"{args.host}:/tmp/siren-src.tgz"],
        capture_output=True, text=True, timeout=300,
    )
    if upload.returncode != 0:
        raise RuntimeError(f"upload failed: {upload.stderr.strip()[:300]}")

    # 2. Build with limited parallelism and low priority (overload guard).
    print("building remotely (nice, -j%d) ..." % args.build_jobs, flush=True)
    out = ssh(args.host, args.password,
              "rm -rf ~/siren-build && mkdir -p ~/siren-build && "
              "tar xzf /tmp/siren-src.tgz -C ~/siren-build && cd ~/siren-build && "
              f"nice -n 19 ~/.cargo/bin/cargo build --release -p siren-cli -j {args.build_jobs} "
              "2>&1 | tail -1", timeout=1800)
    print(out.strip())

    # 3. Swap the binary and restart the daemon as the user (proven pattern:
    #    top-level `&` so plink's session can close; setsid detaches).
    print("deploying ...", flush=True)
    ssh(args.host, args.password,
        f"echo {args.password} | sudo -S pkill -x siren 2>/dev/null; sleep 2; "
        f"echo {args.password} | sudo -S cp ~/siren-build/target/release/siren "
        f"/usr/local/bin/siren && echo swapped", timeout=120)
    ssh(args.host, args.password,
        f"echo {args.password} | sudo -S runuser -u kelierandes -- env "
        f"DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus "
        f"XDG_RUNTIME_DIR=/run/user/1000 setsid nohup /usr/local/bin/siren --log info web "
        f"--listen 0.0.0.0:8090 --isolate pipewire --output-sink play_sink --autostart "
        f"--source tone --freq 440 --name Linux-ISO --port 47900 >/tmp/siren-web.log 2>&1 & "
        f"sleep 3", timeout=120)
    out = ssh(args.host, args.password,
              "ps -C siren -o pid= | wc -l; siren --version", timeout=60)
    print(out.strip())
    return 0


if __name__ == "__main__":
    sys.exit(main())
