#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""classify_vs_runtime.py — @PLN85 ownership-analysis GAP instrument.

The inert `Owned|Borrowed|Join` classifier (src/use_analysis.rs, dumped under
LOFT_MATERIALIZE_DUMP as `OWN fn=…`) is the INPUT the Stage-3 compiler fix reads.
Before wiring any free site we must know its GAPS — where the classification
disagrees with the ground truth (the actual runtime over-free outcome).

This correlates, per probe cell:
  CLASSIFICATION  — the OWN dump (per-fn return class + heap reassign sites)
  RUNTIME         — over-free outcome on BOTH backends (CRASH=UAF / LEAK), with
                    compile-errors and asserts separated OUT (they are not
                    over-free signals).

The four quadrants (the gap map):
  flagged + overfree  : analysis correctly flags; compiler must act here.
  flagged + clean     : over-flag — a safe Borrow, or a Join no free site touches
                        (scalar element / not-source-freed). Compiler must NOT
                        materialise these (regression risk) — scope at the FREE
                        SITE, which only fires for record-element stores.
  NOT-flagged + overfree : *** SOUND MISS *** — a live borrow classified pure
                        Owned; the compiler would free it → UAF. Must be EMPTY.
  NOT-flagged + clean : fine.

Usage:  python3 classify_vs_runtime.py [CORPUS_DIR]   (default: a freshly
        generated 54-cell matrix via grammar_gen.py into a temp dir)
"""
import subprocess, os, re, sys, glob, tempfile

LOFT = os.environ.get("LOFT", "target/release/loft")
# the probe's OWN escape fns: defined in the file, minus the boilerplate helpers
HELPERS = {"main", "filler", "e_default", "m_none", "dflt"}
FN_DEF = re.compile(r"^fn (\w+)", re.M)


def user_fns(path):
    with open(path) as fh:
        return {n for n in FN_DEF.findall(fh.read()) if n not in HELPERS}


def classify(path):
    keep = user_fns(path)
    env = dict(os.environ, LOFT_MATERIALIZE_DUMP="1", LOFT_TIMEOUT="20", LOFT_NO_CACHE="1")
    p = subprocess.run([LOFT, "--interpret", "--check", path],
                       capture_output=True, text=True, env=env, timeout=60)
    if "error:" in p.stderr and "OWN fn=" not in p.stderr:
        return None, "COMPILE-ERR"        # never type-checked → no classification
    rows = []
    for line in p.stderr.splitlines():
        m = re.match(r"OWN fn=n_(\w+) return=(\w+)", line)
        if m and m.group(1) in keep:
            rows.append(f"{m.group(1)}:ret={m.group(2)}")
        m = re.match(r"OWN fn=n_(\w+) reassign v=\d+\((\w+)\) (prior=\w+ rhs=\w+)", line)
        if m and m.group(1) in keep:
            rows.append(f"{m.group(1)}:reassign({m.group(2)}) {m.group(3)}")
    return rows, "ok"


def runtime(path, mode):
    env = dict(os.environ, LOFT_TIMEOUT="30", LOFT_STORES="warn",
               LOFT_NO_CACHE="1", LOFT_POISON="1")  # POISON → UAF is churn-independent
    if mode == "--native":
        env["LOFT_NATIVE_LEAK_CHECK"] = "1"
    try:
        p = subprocess.run([LOFT, mode, path], capture_output=True, text=True, env=env, timeout=70)
    except subprocess.TimeoutExpired:
        return "HANG"
    rc, err = p.returncode, p.stderr
    if "error[E" in err or ("error:" in err and "aborting" in err):
        return "COMPILE-ERR"              # native rustc / type error — NOT over-free
    if "not freed" in err:
        return "LEAK"
    if rc in (139, 134) or rc < 0:
        return "CRASH"
    if "assertion failed" in err:
        return "ASSERT"
    if rc != 0:
        return f"ERR{rc}"
    return "clean"


def is_overfree(o):
    return o in ("CRASH", "LEAK")


def flagged(rows):
    """Anything other than pure-Owned at an escape: a Join return, a Borrowed
    return, or a reassign whose prior was Owned (the displaced-store signal)."""
    if not rows:
        return False
    return any(("ret=Join" in r) or ("ret=Borrowed" in r) or
               ("prior=Owned rhs=Join" in r) or ("prior=Owned rhs=Borrowed" in r)
               for r in rows)


def main():
    corpus = sys.argv[1] if len(sys.argv) > 1 else None
    if not corpus:
        corpus = tempfile.mkdtemp(prefix="own_gen_")
        here = os.path.dirname(os.path.abspath(__file__))
        subprocess.run([sys.executable, os.path.join(here, "grammar_gen.py"),
                        "--out", corpus], check=True, capture_output=True)
    files = sorted(glob.glob(os.path.join(corpus, "*.loft")))
    miss = []
    print(f"{'probe':<34}{'interp':<11}{'native':<11}{'flag':<6}{'classes'}")
    print("-" * 110)
    for f in files:
        rows, st = classify(f)
        ri, rn = runtime(f, "--interpret"), runtime(f, "--native")
        fl = flagged(rows)
        over = is_overfree(ri) or is_overfree(rn)
        if over and not fl and st != "COMPILE-ERR":
            miss.append(os.path.basename(f))
        tag = "MISS!" if (over and not fl) else ("flag" if fl else "")
        cls = "; ".join(rows) if rows else (st if st != "ok" else "(all Owned)")
        name = os.path.basename(f).replace(".loft", "")
        print(f"{name:<30} {ri:<11}{rn:<11}{tag:<6}{cls}")
    print("-" * 110)
    print(f"SOUND MISSES (over-free but classified pure-Owned): {miss or 'NONE'}")


if __name__ == "__main__":
    main()
