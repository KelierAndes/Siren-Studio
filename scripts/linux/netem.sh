#!/usr/bin/env bash
# Bidirectional tc netem control for the SirenStudio fault matrix (F9 / PLAN P0-2).
#
# Shapes BOTH directions independently: egress goes through a root netem on
# the interface, ingress is mirrored to an ifb device and shaped there.
#
# Usage (on the Linux test host):
#   sudo ./netem.sh apply <profile> [iface]
#   sudo ./netem.sh apply raw "<netem params>" [iface]
#   sudo ./netem.sh clear  [iface]
#        ./netem.sh status [iface]
#
# Iface defaults to the default-route interface. Profiles:
#   clean     no impairment (qdiscs removed)
#   loss2     2% independent loss
#   loss10    10% independent loss
#   burst     10% loss with 25% correlation (clustered drops)
#   delay200  200 ms one-way delay (+-10 ms)
#   jitter50  50 ms delay with 25 ms jitter
#   bw1800    1800 kbit/s ceiling (raw f32 needs ~3100 kbit/s)
#   raw       arbitrary netem params, e.g.
#             sudo ./netem.sh apply raw 'loss 30%'
#             sudo ./netem.sh apply raw 'loss 3% delay 40ms 30ms rate 2mbit' enp1s0

set -euo pipefail

ACTION="${1:-status}"

iface_for() {
    # apply <profile> [iface] [egress] keeps the iface in $3; clear/status in $2.
    case "$ACTION" in
        apply) echo "${3:-}" ;;
        *) echo "${2:-}" ;;
    esac
}

# Rate profiles applied in full duplex have hung this host twice; refuse
# them unless egress-only. Loss/delay profiles drop immediately and are safe.
is_rate_profile() {
    case "$1" in
        bw1000|bw1800) return 0 ;;
        *) return 1 ;;
    esac
}

IFACE="$(iface_for)"
IFACE="${IFACE:-$(ip route show default 2>/dev/null | awk '{print $5; exit}')}"
IFB="${IFB:-ifb0}"

if [ -z "$IFACE" ]; then
    echo "netem.sh: no default-route interface found; pass one explicitly" >&2
    exit 1
fi

params_for() {
    case "$1" in
        raw)      echo "${RAW_PARAMS:-}" ;;
        clean)    echo "" ;;
        loss2)    echo "loss 2%" ;;
        loss10)   echo "loss 10%" ;;
        burst)    echo "loss 10% 25%" ;;
        delay200) echo "delay 200ms 10ms distribution normal" ;;
        jitter50) echo "delay 50ms 25ms distribution normal" ;;
        bw1800)   echo "rate 1800kbit" ;;
        bw1000)   echo "rate 1000kbit" ;;
        # 2h 计划（E2E_TEST_PLAN_2H_v2 §7/§8）用的注入档；配合 AUDIO_ONLY=1 使用，
        # 只整 udp/47900，不碰 SSH/控制面。
        ge3)      echo "loss gemodel 0.15% 5% 100% 0%" ;;
        blackout) echo "loss 100%" ;;
        spike)    echo "delay 150ms 10ms" ;;
        reorder)  echo "delay 20ms 15ms distribution normal reorder 25% 50%" ;;
        dup)      echo "duplicate 5%" ;;
        corrupt)  echo "corrupt 0.5%" ;;
        *) echo "netem.sh: unknown profile '$1' (clean loss2 loss10 burst delay200 jitter50 bw1800 raw)" >&2; exit 1 ;;
    esac
}

clear_all() {
    tc qdisc del dev "$IFACE" root 2>/dev/null || true
    tc qdisc del dev "$IFACE" ingress 2>/dev/null || true
    tc qdisc del dev "$IFB" root 2>/dev/null || true
    ip link set "$IFB" down 2>/dev/null || true
}

apply_raw_audio_only() {
    # 只整 UDP 音频端口（默认 47900）：整块网卡 root netem 会把 SSH/pscp 一起
    # 打掉，注入脚本自己断在半路，看起来像测试失败其实是控制通道断了。
    #
    # DIRECTION=both（默认）双向；up=只整 ingress（对端→本机，用于非对称用例）；
    # down=只整 egress（本机→对端）。
    local dir="${DIRECTION:-both}"
    local port="${AUDIO_PORT:-47900}"
    if [ "$dir" = "both" ] || [ "$dir" = "down" ]; then
        tc qdisc add dev "$IFACE" root handle 1: prio
        tc qdisc add dev "$IFACE" parent 1:3 handle 30: netem $PARAMS
        tc filter add dev "$IFACE" parent 1: protocol ip prio 1 u32 \
            match ip protocol 17 0xff \
            match ip dport "$port" 0xffff flowid 1:3
    fi
    if [ "$dir" = "both" ] || [ "$dir" = "up" ]; then
        modprobe ifb 2>/dev/null || true
        ip link show "$IFB" >/dev/null 2>&1 || ip link add "$IFB" type ifb
        ip link set "$IFB" up
        tc qdisc add dev "$IFACE" handle ffff: ingress
        tc filter add dev "$IFACE" parent ffff: protocol ip u32 \
            match ip protocol 17 0xff \
            match ip sport "$port" 0xffff \
            action mirred egress redirect dev "$IFB"
        tc qdisc add dev "$IFB" root netem $PARAMS
    fi
    echo "netem: applied '$PARAMS' to udp/$port on $IFACE (direction=$dir, control traffic untouched)"
}

case "$ACTION" in
    status)
        echo "== $IFACE root ==";  tc qdisc show dev "$IFACE" || true
        echo "== $IFACE ingress =="; tc qdisc show dev "$IFACE" ingress || true
        echo "== $IFB root ==";    tc qdisc show dev "$IFB" 2>/dev/null || true
        ;;
    clear)
        clear_all
        echo "netem: cleared on $IFACE"
        ;;
    apply)
        PROFILE="${2:-}"
        if [ "$PROFILE" = "raw" ]; then
            RAW_PARAMS="${3:-}"
            [ -n "${4:-}" ] && IFACE="$4"
        fi
        PARAMS="$(params_for "$PROFILE")"
        clear_all
        # AUDIO_ONLY=1：2h 计划的注入档也走"只整 udp/47900"的路径，
        # 保证 SSH/控制面与 2h 编排不断线（DIRECTION 控制 up/down/both）。
        if [ "$PROFILE" = "raw" ] || [ "${AUDIO_ONLY:-0}" = "1" ]; then
            if [ -z "$PARAMS" ]; then
                echo "netem.sh: profile '$PROFILE' needs params (raw) or is unknown" >&2
                exit 1
            fi
            apply_raw_audio_only
            exit 0
        fi
        if [ -z "$PARAMS" ]; then
            echo "netem: clean (no impairment) on $IFACE"
            exit 0
        fi
        modprobe ifb 2>/dev/null || true
        ip link show "$IFB" >/dev/null 2>&1 || ip link add "$IFB" type ifb
        ip link set "$IFB" up
        # Ingress: mirror everything arriving on $IFACE into $IFB, shape it
        # there (netem cannot shape ingress directly).
        tc qdisc add dev "$IFACE" handle ffff: ingress
        tc filter add dev "$IFACE" parent ffff: protocol ip u32             match u32 0 0 action mirred egress redirect dev "$IFB"
        tc qdisc add dev "$IFB" root netem $PARAMS
        # Egress: shape directly.
        tc qdisc add dev "$IFACE" root netem $PARAMS
        echo "netem: applied '$PARAMS' to $IFACE (both directions)"
        ;;
    *)
        echo "usage: netem.sh {apply <profile> [iface]|clear [iface]|status [iface]}" >&2
        exit 1
        ;;
esac
