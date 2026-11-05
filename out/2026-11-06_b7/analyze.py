import re, sys
# Parse phone siren logcat: stream status lines + trim/zerorun/gap lines.
# Counters are cumulative per stream id; take last value per stream.
streams = {}
last = None
trims = []
zero = []
gaps = []
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    m = re.search(r"udp: stream (\d+) (\w+) seq (\d+) pkts (\d+) lost (\d+) resets (\d+) underruns (\d+) buffer ([\d.]+) ms target ([\d.]+) rms ([\d.-]+) dB", line)
    if m:
        streams[(m.group(1), m.group(2))] = dict(seq=int(m.group(3)), pkts=int(m.group(4)),
            lost=int(m.group(5)), resets=int(m.group(6)), underruns=int(m.group(7)),
            buf=float(m.group(8)), target=float(m.group(9)), rms=float(m.group(10)))
        last = (m.group(1), m.group(2))
        continue
    m = re.search(r"trace: trim stream=(\d+) fill_before_ms=([\d.]+) .*dropped_ms=([\d.]+).*ms_since_last_trim=(\d+)", line)
    if m:
        trims.append((float(m.group(2)), float(m.group(3)), int(m.group(4)))); continue
    m = re.search(r"zerorun point=(\w+) run_ms=([\d.]+)", line)
    if m:
        zero.append((m.group(1), float(m.group(2)))); continue
    m = re.search(r"audible gap (\d+) ms", line)
    if m:
        gaps.append(int(m.group(1))); continue

print("== streams (last cumulative) ==")
for k, v in streams.items():
    loss = 100.0 * v["lost"] / (v["pkts"] * 240 + v["lost"]) if v["pkts"] else 0
    print(f"  {k[0][-6:]} {k[1]:4}: pkts {v['pkts']} lost% {loss:.2f} resets {v['resets']} underruns {v['underruns']} buf {v['buf']:.0f}/{v['target']:.0f} rms {v['rms']}")
print(f"== trims: {len(trims)} events; dropped>60ms: {sum(1 for t in trims if t[1] > 60)}; max dropped {max((t[1] for t in trims), default=0):.0f} ms")
print(f"== zerorun (mix silence): {len(zero)} events; >100ms: {sum(1 for z in zero if z[1] > 100)}; total {sum(z[1] for z in zero)/1000:.1f} s; max {max((z[1] for z in zero), default=0):.0f} ms")
print(f"== audible gaps: {len(gaps)}; max {max(gaps, default=0)} ms")
