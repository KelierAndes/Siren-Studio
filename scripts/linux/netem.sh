#!/usr/bin/env bash
# Bidirectional tc netem control for the SirenStudio fault matrix (F9 / PLAN P0-2).
#
# Shapes BOTH directions independently: egress goes through a root netem on
# the interface, ingress is mirrored to an ifb device and shaped there.
#
# Usage (on the Linux test host):
#   sudo ./netem.sh apply <profile> [iface]
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

set -euo pipefail

ACTION="${1:-status}"

iface_for() {
    # apply <profile> [iface] keeps the iface in $3; clear/status in $2.
    case "$ACTION" in
        apply) echo "${3:-}" ;;
        *) echo "${2:-}" ;;
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
        clean)    echo "" ;;
        loss2)    echo "loss 2%" ;;
        loss10)   echo "loss 10%" ;;
        burst)    echo "loss 10% 25%" ;;
        delay200) echo "delay 200ms 10ms distribution normal" ;;
        jitter50) echo "delay 50ms 25ms distribution normal" ;;
        bw1800)   echo "rate 1800kbit" ;;
        *) echo "netem.sh: unknown profile '$1' (clean loss2 loss10 burst delay200 jitter50 bw1800)" >&2; exit 1 ;;
    esac
}

clear_all() {
    tc qdisc del dev "$IFACE" root 2>/dev/null || true
    tc qdisc del dev "$IFACE" ingress 2>/dev/null || true
    tc qdisc del dev "$IFB" root 2>/dev/null || true
    ip link set "$IFB" down 2>/dev/null || true
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
        PARAMS="$(params_for "$PROFILE")"
        clear_all
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
