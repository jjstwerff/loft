#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""
ownership_fuzz.py — @PLN85 fuzz-proof gate, first increment (the GENERATOR).

The cross-backend value/exit/leak ORACLE already exists (tests/differential_oracle.rs,
divergences()) and the leak signal is the same "stores not freed" string on both backends.
What was missing is a program-level GENERATOR feeding it over the ownership-composition axes.
This is that generator, scoped to the cheapest axis that exercises the store-lifetime class:
the CHURN / cardinality axis (loop + pressure counts), because the over-free class only
corrupts once a freed slot is REUSED (probes/over-free-sweep/README.md).

Two stages, per the perf split (native compiles via rustc — too slow for a tight loop):
  fast loop  : run each program on --interpret with leak detection  (catches interp crash + leak)
  replay     : re-run every FLAGGED program (+ a sample) on --native (cross-backend divergence + native leak)

Violation modes (the four the invariant can break into):
  CRASH       : a backend exits non-zero (139 = SIGSEGV → UAF/over-free)
  LEAK        : "stores not freed" on a backend's stderr
  DIVERGENCE  : interp stdout != native stdout, or success disagrees (silent two-owner corruption)

Positive control (--self-test): probes/over-free-sweep/P14-enum-field-vec.loft must be FLAGGED
(interp SIGSEGV, native passes). A harness that does not flag P14 is vacuous — its silence on
generated programs would mean nothing. (engineering-rigor: prove the harness can fail.)

Usage:
  ownership_fuzz.py --self-test                       # prove the harness can fail (P14)
  ownership_fuzz.py --corpus <dir>...                 # baseline: run every .loft through the oracle
  ownership_fuzz.py --corpus <dir>... --mutate 8      # + 8 churn-mutants per seed
  ownership_fuzz.py ... --native-replay               # replay flagged programs on --native too
"""
import argparse, os, re, subprocess, sys, tempfile, pathlib

LOFT = os.environ.get("LOFT", "target/release/loft")
TO_INTERP = os.environ.get("FUZZ_TIMEOUT_INTERP", "60")
TO_NATIVE = os.environ.get("FUZZ_TIMEOUT_NATIVE", "120")
LEAK_SIGNAL = "stores not freed"          # identical on both backends (src/state/mod.rs, generation/mod.rs)
RANGE_RE = re.compile(r"\b0\.\.(\d+)\b")  # `0..N` loop/pressure bounds — the churn axis


def normalise(s: str) -> str:
    return "\n".join(line.rstrip() for line in s.replace("\r\n", "\n").rstrip().split("\n"))


def run(mode: str, path: str):
    """Run one program on one backend; return (exit_code, normalised_stdout, leaked)."""
    env = dict(os.environ)
    env["LOFT_TIMEOUT"] = TO_INTERP if mode == "--interpret" else TO_NATIVE
    if mode == "--native":
        env["LOFT_NATIVE_LEAK_CHECK"] = "1"
    try:
        p = subprocess.run([LOFT, mode, path], capture_output=True, text=True,
                           env=env, timeout=int(env["LOFT_TIMEOUT"]) + 15)
    except subprocess.TimeoutExpired:
        return (-99, "", False)  # hang = a violation (treated as CRASH by the caller)
    return (p.returncode, normalise(p.stdout), LEAK_SIGNAL in p.stderr)


def is_crash(rc: int) -> bool:
    """A MEMORY crash — killed by a signal (UAF/double-free/abort), NOT a clean error exit.
    Python reports a signal as a negative returncode; a subprocess shell maps it to 128+sig.
    A clean `exit=1/2` (compile error, assert failure) is NOT a crash on its own — only a
    DIVERGENCE if the two backends disagree about it (one errors, one succeeds)."""
    return rc < 0 or rc in (132, 133, 134, 135, 136, 137, 139)  # SIGILL/TRAP/ABRT/BUS/FPE/KILL/SEGV


def judge(path: str, native_replay: bool):
    """Return a list of violation strings for `path` (empty = clean / backends-agree)."""
    viol = []
    ie, iout, ileak = run("--interpret", path)
    if is_crash(ie):
        viol.append(f"CRASH(interp signal={-ie if ie < 0 else ie - 128})")
    if ileak:
        viol.append("LEAK(interp)")
    # Run native to cross-check whenever interp did anything non-clean, or when sampling.
    # (A clean interp success is only cross-checked under --native-replay — the slow stage.)
    if ie != 0 or ileak or native_replay:
        ne, nout, nleak = run("--native", path)
        if is_crash(ne):
            viol.append(f"CRASH(native signal={-ne if ne < 0 else ne - 128})")
        if nleak:
            viol.append("LEAK(native)")
        if (ie == 0) != (ne == 0):
            viol.append(f"DIVERGENCE(interp exit={ie} ≠ native exit={ne})")
        elif ie == 0 and ne == 0 and iout != nout:
            viol.append("DIVERGENCE(interp stdout ≠ native stdout)")
    return viol


def mutants(src: str, n: int):
    """Yield up to n churn-mutants: scale every `0..N` bound (the slot-reuse axis)."""
    bounds = RANGE_RE.findall(src)
    if not bounds:
        return
    factors = [2, 3, 4, 6, 8, 5, 7, 10][:n]
    for f in factors:
        # cap so a fuzz run stays bounded (don't explode a 0..1000 into millions)
        yield RANGE_RE.sub(lambda m: f"0..{min(int(m.group(1)) * f, 200)}", src)


def collect(dirs):
    out = []
    for d in dirs:
        for p in sorted(pathlib.Path(d).rglob("*.loft")):
            # real files only — skip the `.loft/` cache dir (and a dir literally named `.loft`)
            if p.is_file() and not p.name.startswith(".") and "/.loft/" not in str(p):
                out.append(str(p))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", nargs="*", default=[])
    ap.add_argument("--mutate", type=int, default=0)
    ap.add_argument("--native-replay", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if not os.access(LOFT, os.X_OK):
        sys.exit(f"no loft binary at {LOFT} (build: cargo build --release)")

    if args.self_test:
        p14 = "doc/claude/plans/85-store-lifetime-retirement/probes/over-free-sweep/P14-enum-field-vec.loft"
        v = judge(p14, native_replay=True)
        ok = any("CRASH(interp" in x for x in v)
        print(f"positive control P14: {v or 'CLEAN'}")
        print("SELF-TEST PASS — harness flags the live bug" if ok
              else "SELF-TEST FAIL — harness is VACUOUS (did not flag P14)")
        sys.exit(0 if ok else 1)

    seeds = collect(args.corpus)
    print(f"# corpus: {len(seeds)} seeds  mutate={args.mutate}  native_replay={args.native_replay}")
    flagged = 0
    total = 0
    with tempfile.TemporaryDirectory() as td:
        for s in seeds:
            programs = [(s, s)]
            if args.mutate:
                src = pathlib.Path(s).read_text()
                for i, mut in enumerate(mutants(src, args.mutate)):
                    mp = os.path.join(td, f"{pathlib.Path(s).stem}.m{i}.loft")
                    pathlib.Path(mp).write_text(mut)
                    programs.append((f"{s}~m{i}", mp))
            for label, path in programs:
                total += 1
                v = judge(path, args.native_replay)
                if v:
                    flagged += 1
                    print(f"VIOLATION {label}: {', '.join(v)}")
    print(f"# done: {flagged}/{total} flagged")
    sys.exit(1 if flagged else 0)


if __name__ == "__main__":
    main()
