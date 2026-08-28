#!/usr/bin/env python3
"""Rank test units by CPU time from a `LOFT_TEST_TIMING` file.

Read this before acting on any "what is slow" question, because every previous answer in
this repo came from wall clock and every one was wrong:

  * JUnit `time` under load measures CONTENTION (255x inflation measured on one test);
  * re-running one test in isolation measures the BUILD (19.1s vs 0.096s for the same test);
  * a figure recorded in a document is a claim with a date on it (136s -> 0.139s).

CPU is contention-invariant, so it survives a loaded box and a second checkout's gate.

Usage:  scripts/test-timing.py <file>... [--top N] [--by wall|cpu|own|kids]

GIVE IT SEVERAL RUNS.  One run is not a measurement: three recordings of the IDENTICAL
configuration measured 12.2s, 16.0s and 8.1s here — a 2x spread, which swallowed a "24%
win" that was really noise.  With 2+ files each unit reports its MINIMUM (the run least
disturbed by whatever else the box was doing) and the observed spread, so a difference
smaller than the spread is visibly not a result.

The WALL/CPU ratio is the diagnosis, and the report prints it:
  cpu ~ wall   real work        -> make it do less
  cpu << wall  waiting          -> contention, I/O or a sleep; parallelism or a fixture
  kids >> own  a subprocess     -> the cost is rustc/loft/browser, not the test body
"""
import sys
from collections import defaultdict

def main() -> int:
    # Positional files only: skip each flag AND the value that follows it, or `--top 8`
    # leaves an `8` in the file list and the tool dies on a missing file named "8".
    argv, args, skip = sys.argv[1:], [], False
    for i, a in enumerate(argv):
        if skip:
            skip = False
            continue
        if a in ("--top", "--by"):
            skip = True
            continue
        if a.startswith("--"):
            continue
        args.append(a)
    if not args:
        print(__doc__)
        return 1
    top = 25
    key = "cpu"
    for i, a in enumerate(sys.argv):
        if a == "--top" and i + 1 < len(sys.argv):
            top = int(sys.argv[i + 1])
        if a == "--by" and i + 1 < len(sys.argv):
            key = sys.argv[i + 1]

    # Per file, then per unit: MIN across files.  Minimum rather than mean because the
    # noise is one-sided — contention only ever ADDS time, so the smallest observation is
    # the closest to the unit's own cost.  (Same reason `make speed` takes min-of-7.)
    per_file = []
    for path in args:
        acc = defaultdict(lambda: [0.0, 0.0, 0.0, 0.0, 0])
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                f = line.rstrip("\n").split("\t")
                if len(f) < 5:
                    continue      # a torn append; skip rather than mis-column
                try:
                    wall, own, kids, cpu = (float(x) for x in f[:4])
                except ValueError:
                    continue
                r = acc["\t".join(f[4:])]
                r[0] += wall; r[1] += own; r[2] += kids; r[3] += cpu; r[4] += 1
        per_file.append(acc)

    rows = defaultdict(lambda: [0.0, 0.0, 0.0, 0.0, 0])
    spread = {}
    for name in {k for a in per_file for k in a}:
        seen = [a[name] for a in per_file if name in a]
        best = min(seen, key=lambda r: r[3])
        rows[name] = best
        walls = [r[0] for r in seen]
        spread[name] = (max(walls) / min(walls)) if len(walls) > 1 and min(walls) > 0 else 1.0

    if len(args) == 1:
        print("⚠ ONE run — not a measurement.  Contention alone has produced a 2x spread "
              "on this suite; pass several recordings so the report can take the minimum "
              "and show you the spread.")

    if not rows:
        print(f"{args[0]}: no rows — was LOFT_TEST_TIMING set for the run?")
        return 1

    idx = {"wall": 0, "own": 1, "kids": 2, "cpu": 3}.get(key, 3)
    tot_wall = sum(r[0] for r in rows.values())
    tot_cpu = sum(r[3] for r in rows.values())
    print(f"{len(rows)} units, {sum(r[4] for r in rows.values())} runs — "
          f"wall {tot_wall/1000:.1f}s, cpu {tot_cpu/1000:.1f}s "
          f"(cpu/wall {tot_cpu/tot_wall:.2f} — below 1 means the suite spent time WAITING)")
    worst = max(spread.values()) if spread else 1.0
    if len(args) > 1:
        print(f"min of {len(args)} runs; worst per-unit wall spread {worst:.1f}x — "
              f"treat any difference below that as noise")
    print(f"{'cpu ms':>10} {'own':>9} {'kids':>9} {'wall ms':>10} {'c/w':>5} {'sprd':>5}  unit")
    for name, r in sorted(rows.items(), key=lambda kv: -kv[1][idx])[:top]:
        ratio = r[3] / r[0] if r[0] else 0.0
        print(f"{r[3]:10.1f} {r[1]:9.1f} {r[2]:9.1f} {r[0]:10.1f} {ratio:5.2f} "
              f"{spread.get(name, 1.0):5.1f}  {name}")
    return 0

if __name__ == "__main__":
    sys.exit(main())
