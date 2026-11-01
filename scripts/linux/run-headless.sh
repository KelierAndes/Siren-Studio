#!/usr/bin/env bash
# Runs a SirenStudio mesh node with its WebUI on a headless Linux box.
#
# Starts a per-user PulseAudio server (if none is reachable), creates a null
# sink for deterministic testing, then serves the WebUI. On a desktop with
# PipeWire/PulseAudio already running, skip this script and run:
#
#   siren web --listen 0.0.0.0:8080 --autostart --source loopback
#
# Environment: PORT (UDP, 47900), LISTEN (web, 0.0.0.0:8080), NAME, SIREN_BIN.
set -euo pipefail

export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/siren-runtime}"
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"

if ! pactl info >/dev/null 2>&1; then
    pulseaudio -D --exit-idle-time=-1 --log-target=file:/tmp/siren-pulse.log
    sleep 1
fi

if ! pactl list short sinks | grep -q 'siren_test'; then
    pactl load-module module-null-sink sink_name=siren_test rate=48000 channels=2 >/dev/null
fi
if [ "$(pactl get-default-sink 2>/dev/null || true)" != "siren_test" ]; then
    pactl set-default-sink siren_test
fi

exec "${SIREN_BIN:-siren}" --log info web \
    --listen "${LISTEN:-0.0.0.0:8080}" \
    --autostart \
    --source loopback \
    --name "${NAME:-$(hostname)}" \
    --port "${PORT:-47900}"
