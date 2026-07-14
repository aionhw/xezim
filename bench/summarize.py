#!/usr/bin/env python3
"""Summarize / compare xezim benchmark CSVs across machines.

  ./bench/summarize.py results/*.csv

Reports the MEDIAN of each (bench, variant) per host — median, not mean, so a
single scheduler hiccup cannot move the number — plus the spread, so you can
tell a real platform difference from noise. With >1 host, the fastest host is
the baseline and the rest are shown relative to it.
"""
import csv, sys, statistics
from collections import defaultdict

def med(xs):
    return statistics.median(xs) if xs else 0.0

def main(paths):
    rows = []
    for p in paths:
        with open(p) as f:
            rows.extend(csv.DictReader(f))
    if not rows:
        sys.exit("no rows")

    key = lambda r: (r["bench"], r["variant"])
    hosts = sorted({r["host"] for r in rows})
    data = defaultdict(lambda: defaultdict(list))   # key -> host -> [rate]
    walls = defaultdict(lambda: defaultdict(list))
    prof = defaultdict(lambda: defaultdict(list))

    for r in rows:
        k, h = key(r), r["host"]
        rate = float(r.get("items_per_sec") or 0)
        if rate:
            data[k][h].append(rate)
        walls[k][h].append(float(r["wall_ms"]))
        # subsystem attribution: which phase explains a delta?
        prof[k][h].append(tuple(float(r.get(c) or 0)
                                for c in ("settle_ms", "edges_ms", "nba_ms", "process_ms")))

    label = {h: next(r["cpu"] for r in rows if r["host"] == h) for h in hosts}
    print("hosts:")
    for h in hosts:
        cores = next(r["cores"] for r in rows if r["host"] == h)
        arch = next(r["arch"] for r in rows if r["host"] == h)
        print(f"  {h:12s} {arch:8s} {cores:>3s} cores  {label[h][:46]}")
    print()

    w = 22
    hdr = f"{'benchmark':{w}s}" + "".join(f"{h:>16s}" for h in hosts)
    if len(hosts) > 1:
        hdr += f"{'spread':>10s}"
    print(hdr)
    print("-" * len(hdr))

    for k in sorted(data, key=lambda x: (x[0], x[1])):
        name = f"{k[0]} {k[1]}"
        cells, meds = [], []
        for h in hosts:
            vals = data[k].get(h) or []
            if not vals:
                cells.append(f"{'-':>16s}")
                continue
            m = med(vals)
            meds.append(m)
            # relative spread across reps — a noisy row is not comparable
            rel = (max(vals) - min(vals)) / m * 100 if m else 0
            flag = "!" if rel > 10 else " "
            cells.append(f"{m:>14,.0f}{flag} ")
        line = f"{name:{w}s}" + "".join(cells)
        if len(meds) > 1 and min(meds) > 0:
            line += f"{max(meds)/min(meds):>9.2f}x"
        print(line)

    print("\nunits: items/sec (simulated cycles/sec; randomizations/sec for B5)")
    print("'!' = >10% spread across reps on that host — rerun before trusting it")

    # Hardware counters: this is what tells you WHY a platform differs.
    # Rates, not raw counts — they stay meaningful across clock speeds.
    has_perf = any(float(r.get("ipc") or 0) > 0 for r in rows)
    if has_perf:
        print("\nhardware counters (median)")
        hw = defaultdict(lambda: defaultdict(list))
        for r in rows:
            if float(r.get("ipc") or 0) > 0:
                hw[key(r)][r["host"]].append((
                    float(r["ipc"]),
                    float(r.get("branch_miss_pct") or 0),
                    float(r.get("cache_miss_pct") or 0),
                ))
        print(f"  {'benchmark':{w}s}{'host':12s}{'IPC':>7s}{'br-miss%':>10s}{'cache-miss%':>13s}")
        for k in sorted(hw):
            for h in hosts:
                v = hw[k].get(h) or []
                if not v:
                    continue
                ipc = med([x[0] for x in v])
                brm = med([x[1] for x in v])
                cms = med([x[2] for x in v])
                print(f"  {k[0]+' '+k[1]:{w}s}{h:12s}{ipc:>7.2f}{brm:>10.2f}{cms:>13.2f}")

    if len(hosts) > 1:
        print("\nsubsystem split (median ms: settle / edges / nba / process)")
        for k in sorted(prof):
            print(f"  {k[0]} {k[1]}")
            for h in hosts:
                ps = prof[k].get(h) or []
                if not ps:
                    continue
                cols = [med([p[i] for p in ps]) for i in range(4)]
                print(f"    {h:12s} " + " / ".join(f"{c:7.1f}" for c in cols))

if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    main(sys.argv[1:])
