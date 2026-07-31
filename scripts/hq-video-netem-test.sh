#!/usr/bin/env bash
# HQ video engine acceptance helper
# Simulates the network matrix from rustdesk-hq-tcp-development-plan.md §16.2
# Usage: sudo ./scripts/hq-video-netem-test.sh <iface> [profile]
# Profiles: clean | office | motion | congested | lossy
set -euo pipefail

IFACE="${1:-}"
PROFILE="${2:-office}"

if [[ -z "$IFACE" ]]; then
  echo "Usage: $0 <network-interface> [clean|office|motion|congested|lossy]"
  echo "Example: $0 eth0 office"
  exit 1
fi

clear_netem() {
  tc qdisc del dev "$IFACE" root 2>/dev/null || true
}

apply() {
  local rtt="$1" jitter="$2" loss="$3" rate="$4"
  clear_netem
  # netem delay is one-way; RTT ≈ 2 * delay
  local one_way=$((rtt / 2))
  tc qdisc add dev "$IFACE" root handle 1: htb default 10
  tc class add dev "$IFACE" parent 1: classid 1:10 htb rate "$rate"
  tc qdisc add dev "$IFACE" parent 1:10 handle 10: netem \
    delay "${one_way}ms" "${jitter}ms" loss "${loss}%"
  echo "Applied profile=$PROFILE on $IFACE: RTT≈${rtt}ms jitter=${jitter}ms loss=${loss}% rate=$rate"
}

case "$PROFILE" in
  clean)
    apply 10 0 0 "100mbit"
    ;;
  office)
    # §17 office-clear: RTT 80ms, 30 Mbps
    apply 80 10 0.1 "30mbit"
    ;;
  motion)
    # §17 motion-smooth: RTT 50ms, 40 Mbps
    apply 50 5 0 "40mbit"
    ;;
  congested)
    apply 150 20 0.5 "20mbit"
    ;;
  lossy)
    apply 250 50 1 "10mbit"
    ;;
  clear)
    clear_netem
    echo "Cleared netem on $IFACE"
    exit 0
    ;;
  *)
    echo "Unknown profile: $PROFILE"
    exit 1
    ;;
esac

cat <<EOF

Manual acceptance checklist (HQ Video Preview 0.1):
  [ ] Office clear @1080p: stable ~30 FPS, bitrate 12-20 Mbps, text sharper than stock Best
  [ ] Motion smooth @1080p: >=55 FPS with H.265 HW when available
  [ ] Quality monitor shows codec / bitrate / queue delay / QoS state
  [ ] Copy diagnostic report works
  [ ] Congestion: queue P95 < 150ms, recovery < 5s
  [ ] Toolbar status line updates (Codec | Mbps | Queue)

Restore with: $0 $IFACE clear
EOF
