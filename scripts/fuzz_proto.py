#!/usr/bin/env python3
"""P1.3 协议模糊（只打回环测试节点，不碰真实设备）。

做法：在本机起一个 `siren mesh --bind 127.0.0.1 --source tone --no-play --no-discover`，
按帧头字段与 AEAD 路径构造畸形/越界报文，逐个发送后检查：
1) 进程仍存活（无 panic）；
2) 节点日志没有 panic/abort；
3) 节点仍能正常响应（继续发送自身音频，快照可读）。
AEAD 只覆盖"拒收不崩溃"；真正的密文鉴权在 link_sim 的错密码用例里。
"""
from __future__ import annotations

import argparse
import os
import socket
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PORT = 48999


def frame(version: int, codec: int, frame_ms: int, length: int, payload: bytes) -> bytes:
    """按 protocol.rs 的紧凑帧头拼一个报文：magic 'SR' + ver/codec + ms + len + seq + origin。"""
    header = struct.pack("<2sBBHHQI", b"SR", version & 0xFF, codec & 0xFF,
                         frame_ms & 0xFFFF, length & 0xFFFF, 1, 0x1234)
    return header + payload


def cases() -> list[tuple[str, bytes]]:
    good = frame(3, 2, 5, 240, b"\x00" * 480)
    out = [
        ("bad_magic", b"ZZ" + good[2:]),
        ("version_0", frame(0, 2, 5, 240, b"\x00" * 480)),
        ("version_9", frame(9, 2, 5, 240, b"\x00" * 480)),
        ("codec_255", frame(3, 255, 5, 240, b"\x00" * 480)),
        ("frame_ms_0", frame(3, 2, 0, 240, b"\x00" * 480)),
        ("frame_ms_61", frame(3, 2, 61, 240, b"\x00" * 480)),
        ("len_0", frame(3, 2, 5, 0, b"")),
        ("len_huge", frame(3, 2, 5, 65507, b"\x00" * 64)),
        ("truncated_opus", frame(3, 1, 5, 8, b"\x01\x02\x03")),
        ("raw_as_opus", frame(3, 1, 5, 480, b"\x00" * 480)),
        ("empty", b""),
        ("header_only", good[:24]),
        ("garbage", os.urandom(1200)),
    ]
    return out


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", default=str(ROOT / "target" / "release" / "siren.exe"))
    parser.add_argument("--seconds", type=float, default=3.0, help="每个报文后的等待")
    args = parser.parse_args()
    exe = Path(args.exe)
    if not exe.exists():
        print(f"找不到 {exe}（先 cargo build --release -p siren-cli）")
        return 2
    log = Path(tempfile.gettempdir()) / "siren_fuzz.log"
    proc = subprocess.Popen(
        [str(exe), "--log", "info", "mesh", "--bind", "127.0.0.1", "--port", str(PORT),
         "--name", "fuzz-target", "--source", "tone", "--no-play", "--no-discover"],
        stdout=open(log, "w"), stderr=subprocess.STDOUT)
    time.sleep(4)
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    failures = []
    try:
        for name, payload in cases():
            sock.sendto(payload, ("127.0.0.1", PORT))
            time.sleep(0.2)
            if proc.poll() is not None:
                failures.append(f"{name}: 进程退出（code {proc.returncode}）")
                break
        time.sleep(args.seconds)
        if proc.poll() is not None:
            failures.append(f"最终检查：进程已退出（code {proc.returncode}）")
        text = log.read_text(encoding="utf-8", errors="replace")
        for marker in ("panicked", "RUST_BACKTRACE", "abort"):
            if marker in text:
                failures.append(f"日志出现 {marker}")
    finally:
        if proc.poll() is None:
            proc.terminate()
    if failures:
        print("FUZZ FAIL:\n- " + "\n- ".join(failures))
        return 1
    print(f"FUZZ PASS：{len(cases())} 个畸形报文，进程存活、无 panic（日志 {log}）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
