#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Monthly bug-review aid: which MECHANISM classes are still producing bugs.

Reports four things and judges none of them (see BUG_REVIEW.md):

  1. the population this cycle reviews;
  2. each mechanism class's share of bugs over time, so a class that is still
     firing is visible next to one that has gone quiet;
  3. the payoff check — for every keystone already landed, whether its class's
     bug share actually fell afterwards;
  4. enumeration exposure — how often each child-bearing `Value` variant is
     omitted from a hand-written walker, beside how many bugs it has carried.

Bugs are bucketed by ISSUE NUMBER, not close date: the tracker is young and a
release-month close-out lands hundreds of old issues at once, which makes any
calendar window read as "everything is recent".

Usage:
    scripts/bug-review.py                      # fetch from gh
    scripts/bug-review.py --cache issues.json  # re-run offline
    scripts/bug-review.py --bands 4            # coarser/finer time slicing
"""
import argparse, json, pathlib, re, subprocess, sys
from collections import defaultdict

# Mechanism signatures, matched against the issue TITLE.  loft issue titles state
# a mechanism, which is what makes title-matching usable here; the classes overlap
# on purpose (one bug can belong to several) so shares are read per class, never
# summed.
CLASSES = {
    "generic/monomorph": r"generic|monomorph|type variable|template|instantiat",
    "tuple":             r"\btuple",
    "null/sentinel":     r"\bnull|sentinel|nullable",
    "narrow-int/width":  r"narrow|width|\bbyte|u8|u16|i32",
    "ownership/free":    r"\bfree|leak|use-after|UAF|owner|borrow|double-",
    "enum/variant":      r"\benum\b|variant|discriminant",
    "keyed collections": r"\bhash\b|sorted|radix|trie|spatial|keyed",
    "wasm/browser":      r"wasm|browser|html",
    "packages/registry": r"registr|package|loft\.toml|install|publish",
    "par/coroutine":     r"\bpar\b|parallel|worker|coroutine|yield|generator",
    "traversal/reach":   r"reachab|walk|traver|descend|prune|unmarked|unreachable",
}

# Keystones already landed, and the class each was meant to retire.  The payoff
# check reads this: a keystone whose class did NOT fall afterwards is the useful
# finding, because it means the fact was not the one manufacturing the bugs.
KEYSTONES = [
    ("IntegerSpec::range_to_width",  "narrow-int/width",  700),
    ("Stores::for_each_owned_child", "keyed collections", 715),
    ("Value::for_each_child",        "traversal/reach",   700),
]

# Child-bearing IR variants — the set `IrNode::for_each_child` is exhaustive over.
CHILD_BEARING = ["Call", "CallRef", "Insert", "Tuple", "Parallel", "Block", "Loop",
                 "Set", "Return", "BreakWith", "Drop", "Yield", "TuplePut", "Span",
                 "If", "Iter", "ParFor"]
FN_RE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(const\s+)?fn\s+([a-z_0-9]+)")


def load(cache):
    if cache:
        return json.loads(pathlib.Path(cache).read_text())
    out = subprocess.run(
        ["gh", "issue", "list", "--state", "all", "--limit", "1200",
         "--json", "number,title,labels,state,closedAt"],
        capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"gh failed: {out.stderr.strip()}\n"
                 f"(offline? re-run with --cache <file.json>)")
    return json.loads(out.stdout)


def classify(issues):
    hits = defaultdict(list)
    for i in issues:
        for name, pat in CLASSES.items():
            if re.search(pat, i["title"], re.I):
                hits[name].append(i)
    return hits


def walker_omissions():
    """How often each child-bearing variant is left out of a PARTIAL walker.

    Counts only walkers that recurse, carry a wildcard arm, and do not delegate
    to a keystone — a delegating or exhaustive walker cannot omit anything.
    """
    present, total = defaultdict(int), 0
    for p in sorted(pathlib.Path("src").rglob("*.rs")):
        lines = p.read_text(errors="replace").splitlines()
        cur, body, fns = None, [], []
        for line in lines:
            m = FN_RE.match(line)
            if m:
                if cur:
                    fns.append((cur, body))
                cur, body = m.group(4), []
            if cur is not None:
                body.append(line)
        if cur:
            fns.append((cur, body))
        for name, body in fns:
            txt = "\n".join(body)
            arms = set(re.findall(r"Value::([A-Z]\w+)", txt))
            if len(arms) < 4:
                continue
            if not re.search(r"\b" + re.escape(name) + r"\s*\(", txt[txt.find("{"):]):
                continue
            if "for_each_child" in txt:            # delegates — total by construction
                continue
            if not re.search(r"^\s*(_|other)\s*(\||=>)", txt, re.M):
                continue                            # exhaustive — cannot omit
            total += 1
            for v in CHILD_BEARING:
                if v in arms:
                    present[v] += 1
    return present, total


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cache", help="issues JSON from a previous gh call")
    ap.add_argument("--bands", type=int, default=4, help="time slices (default 4)")
    a = ap.parse_args()

    issues = load(a.cache)
    bugs = [i for i in issues if any(l["name"] == "bug" for l in i["labels"])]
    if not bugs:
        sys.exit("no `bug`-labelled issues found")
    nums = sorted(i["number"] for i in bugs)
    lo, hi = nums[0], nums[-1]
    step = max(1, (hi - lo) // a.bands)
    bands = [(lo + k * step, lo + (k + 1) * step if k < a.bands - 1 else hi + 1)
             for k in range(a.bands)]

    print(f"\n=== 1. Population ===")
    print(f"  {len(bugs)} bug issues, #{lo}-#{hi}   "
          f"({sum(1 for i in issues if i['state'] == 'OPEN')} open overall)")
    print(f"  bucketed by issue number into {a.bands} bands of ~{step}")

    hits = classify(bugs)
    counts = {n: {b: 0 for b in bands} for n in CLASSES}
    tot = {b: 0 for b in bands}
    for i in bugs:
        for b in bands:
            if b[0] <= i["number"] < b[1]:
                tot[b] += 1
    for name, lst in hits.items():
        for i in lst:
            for b in bands:
                if b[0] <= i["number"] < b[1]:
                    counts[name][b] += 1

    print(f"\n=== 2. Mechanism class share by band (RISING = still firing) ===")
    hdr = "  " + "class".ljust(20) + "".join(f"#{b[0]}-{b[1]}".rjust(13) for b in bands) + "   trend"
    print(hdr + "\n  " + "-" * (len(hdr) - 2))
    rows = []
    for name in CLASSES:
        sh = [100 * counts[name][b] / tot[b] if tot[b] else 0.0 for b in bands]
        peak = max(sh[:-1]) if len(sh) > 1 else sh[0]
        # Measured against the PEAK, not band 0.  A class that did not exist in the
        # first band, rose, and has since fallen is FALLING; comparing it to zero
        # would call it rising and point the cycle at work already done.
        rows.append((sh[-1] - peak, name, sh, [counts[name][b] for b in bands], peak))
    for delta, name, sh, c, peak in sorted(rows, reverse=True):
        cells = "".join(f"{c[j]:3d} ({sh[j]:4.1f}%)".rjust(13) for j in range(len(bands)))
        mark = "RISING" if delta > 2 else ("falling" if delta < -2 else "flat")
        print(f"  {name:<20}{cells}   {mark} {delta:+.1f}pp vs peak {peak:.1f}%")

    print(f"\n=== 3. Payoff check — did each landed keystone move its class? ===")
    for keystone, cls, landed in KEYSTONES:
        if cls not in counts:
            print(f"  {keystone:<32} {cls}: no class signature — add one to CLASSES")
            continue
        before = [b for b in bands if b[1] <= landed]
        after = [b for b in bands if b[0] >= landed]
        if not before or not after:
            print(f"  {keystone:<32} landed at #{landed}: not enough bands either side yet")
            continue
        nb = sum(counts[cls][b] for b in before)
        sb = 100 * nb / max(1, sum(tot[b] for b in before))
        sa = 100 * sum(counts[cls][b] for b in after) / max(1, sum(tot[b] for b in after))
        # A class with (almost) no bugs BEFORE the keystone cannot show a fall after
        # it — there was nothing to remove.  Abstain rather than print a verdict the
        # data does not carry: a false "NO EFFECT" would send the cycle to re-open a
        # premise that was never tested.
        if nb < 3:
            verdict = f"cannot judge — only {nb} bug(s) in this class before it landed"
        elif sa < sb - 1:
            verdict = "PAID OFF"
        else:
            verdict = "NO EFFECT — re-open the premise"
        print(f"  {keystone:<32} {cls:<18} {sb:5.1f}% -> {sa:5.1f}%   {verdict}")

    present, total = walker_omissions()
    print(f"\n=== 4. Enumeration exposure ({total} partial walkers scanned) ===")
    print("  a variant is dangerous when it is BOTH often-omitted and often-used")
    print(f"  {'variant':<12}{'omitted':>9}{'bugs':>7}")
    bugcount = {"Tuple": len(hits.get("tuple", [])),
                "Parallel": len(hits.get("par/coroutine", [])),
                "ParFor": len(hits.get("par/coroutine", [])),
                "Yield": len(hits.get("par/coroutine", []))}
    for v in sorted(CHILD_BEARING, key=lambda v: -(total - present[v])):
        om = 100 * (total - present[v]) / total if total else 0
        bc = bugcount.get(v)
        print(f"  {v:<12}{om:8.1f}%{(str(bc) if bc is not None else '-'):>7}")
    print()


if __name__ == "__main__":
    main()
