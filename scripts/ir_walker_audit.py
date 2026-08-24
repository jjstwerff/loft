#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# ir_walker_audit.py — two questions about `Value`, the IR tree, and the cross-check
# that settles the second one.
#
# `rule_predicate_audit.py` asks where one rule is spelled as a TYPE LIST more than once.
# This asks the two questions that live one level up, over the IR tree itself:
#
#   walkers   Who hand-rolls `Value`'s tree shape?  `Value::for_each_child` is the
#             keystone (data.rs) — its match is exhaustive on purpose, "so a new `Value`
#             variant forces a decision here and every walker inherits the edge".  A
#             walker that recurses by its own match does NOT inherit it: when a variant
#             is added, or an existing one is reached by a new route, that walker goes
#             silently blind.  Measured twice already — `inline_ref_set_in`'s hand-rolled
#             predecessor "treated `BreakWith` as a leaf and missed a `Set` inside its
#             value" (parser/expressions.rs), and `scopes::walk_check` had the same hole.
#
#   producers Which variants can never come into existence?  A variant whose every
#             construction is a REBUILD (inside its own match arm), a DESERIALIZER, or a
#             test is a closed cycle with no source: nothing creates the first instance,
#             so nothing can deserialize one either.  It still costs every walker an arm,
#             and no test can ever reach those arms to check them.
#
#   dead      `producers` INTERSECTED with a census of what the front end actually emits
#             over the 854-program corpus.  Neither half is an oracle and the failures go
#             opposite ways, which is the whole reason this mode exists: `Loop`/`Single`/
#             `Parallel` are screened as producerless (their origin is a helper the screen
#             reads as a rebuild) yet are plainly alive, and `Iter`/`RawExpr` are absent
#             from the census (lowered away before the snapshot, or built after it) yet
#             have real producers.  Only a variant flagged by BOTH is dead.
#
# A REPORT, never a gate.  Every mode is heuristic — `producers` classifies by construction
# SITE, so read the sites it prints before acting.  Verdicts live in
# doc/claude/formal/IMPLEMENTATIONS.md.
#
# Usage:  python3 scripts/ir_walker_audit.py [walkers|producers|dead|both]
#         `dead` needs a built binary (target/debug/loft, or $LOFT_BIN) and takes ~1 min.

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "src")
DATA_RS = os.path.join(SRC, "data.rs")

FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+([a-z_][a-z0-9_]*)")
VARIANT = re.compile(r"\bValue::([A-Za-z_][A-Za-z0-9_]*)")
KEYSTONE = ("for_each_child", ".any_node(")
# Files that only ever build a `Value` to put it back on the wire, or to read one off it.
SERIALIZERS = ("ir_read.rs", "ir_schema.rs", "ir_store.rs", "ir_node.rs", "ir_schema_gen.rs")


def rust_files():
    return sorted(glob.glob(os.path.join(SRC, "**", "*.rs"), recursive=True))


def rel(path):
    return os.path.relpath(path, ROOT)


def value_variants():
    """The `Value` variant names, read from the enum itself."""
    out, inside = [], False
    for line in open(DATA_RS, encoding="utf-8"):
        if line.startswith("pub enum Value"):
            inside = True
            continue
        if inside:
            if line.startswith("}"):
                break
            m = re.match(r"\s+([A-Z][A-Za-z0-9]*)\s*[(,{]", line)
            if m:
                out.append(m.group(1))
    return out


def functions(path):
    """Yield (name, start_line, body_text) for each fn in the file."""
    lines = open(path, encoding="utf-8").read().split("\n")
    cur, start, body = None, 0, []
    for i, line in enumerate(lines, 1):
        m = FN.match(line)
        if m:
            if cur:
                yield cur, start, "\n".join(body)
            cur, start, body = m.group(1), i, []
        elif cur is not None:
            body.append(line)
    if cur:
        yield cur, start, "\n".join(body)


# A bare `_ =>` / `other =>` arm.  This is the difference that matters: an EXHAUSTIVE
# match (no catch-all) already gets the keystone's guarantee for free — adding a variant
# breaks the build and forces a decision.  A partial match with a catch-all is the shape
# that goes silently blind, because the new edge just falls into `_`.
CATCHALL = re.compile(r"^\s*(?:_|other|rest)\s*(?:if\b[^=]*)?=>", re.M)


def audit_walkers():
    hand, keyed, exhaustive = [], 0, 0
    for path in rust_files():
        if os.path.basename(path) == "data.rs":
            continue  # the keystone's own home
        for name, start, body in functions(path):
            variants = set(VARIANT.findall(body))
            if len(variants) < 3:
                continue
            if not re.search(rf"\b{re.escape(name)}\s*\(", body):
                continue  # not recursive
            if any(k in body for k in KEYSTONE):
                keyed += 1
            elif not CATCHALL.search(body):
                exhaustive += 1
            else:
                hand.append((f"{rel(path)}:{start}", name, len(variants)))
    total = keyed + exhaustive + len(hand)
    print(f"recursive multi-variant `Value` walkers: {total}")
    print(f"  descend via the keystone         : {keyed:>4}   inherit every edge")
    print(f"  exhaustive match, no catch-all   : {exhaustive:>4}   a new variant breaks the build")
    print(f"  partial match + catch-all        : {len(hand):>4}   a new edge falls into `_`, silently")
    print()
    print("  The last group, fewest variants first — the least covered has the most to miss:")
    for site, name, n in sorted(hand, key=lambda r: r[2])[:30]:
        print(f"  {site:<42} {name:<36} {n:>2} variants")
    if len(hand) > 30:
        print(f"  … and {len(hand) - 30} more")


def constructor_sites(variant):
    """Lines that plausibly CONSTRUCT `Value::<variant>`, with a classification.

    Three things are NOT a producer, and the point of this mode is to tell them apart:
      * `serializer` — ir_read/ir_schema/ir_store/ir_node build a node to put it on the
        wire or read one off it.  A deserializer can only ever produce what a producer
        once wrote, so it cannot be the origin.
      * `test`       — inside a `#[cfg(test)]` module.
      * `rebuild`    — the construction sits in a function that also MATCHES this
        variant, so it rewrites an instance that already existed.  This is the one the
        first cut of this script missed, and it is the one that hides a dead variant:
        `Scopes::scan` rebuilds every variant it walks.
    """
    # Unit variants (`Value::Null`) take no parens; the rest do.
    pat = re.compile(rf"\bValue::{variant}\s*\(")
    unit = re.compile(rf"\bValue::{variant}\b(?!\s*\()")
    arm = re.compile(rf"\bValue::{variant}\b[^=]*=>")
    out = []
    for path in rust_files():
        text = open(path, encoding="utf-8").read()
        lines = text.split("\n")
        test_line = next((i for i, l in enumerate(lines, 1) if "#[cfg(test)]" in l), None)
        # Enclosing-function body for each line, so a rebuild can be recognised.
        fn_of = {}
        for name, start, body in functions(path):
            end = start + body.count("\n") + 1
            for i in range(start, end + 1):
                fn_of[i] = (name, body)
        for i, line in enumerate(lines, 1):
            if not (pat.search(line) or unit.search(line)):
                continue
            stripped = line.strip()
            # Comments and string literals mention a variant without building one.
            if stripped.startswith(("//", "*", "#")) or f'"Value::{variant}' in line:
                continue
            if "matches!" in line or "=>" in line:
                continue  # pattern position
            # `Value::X(_` / `Value::X(_,` binds nothing: a pattern whose `=>` is on a
            # later line (`Value::Return(_) | Value::Break(_) | …` wraps in control.rs).
            if re.search(rf"\bValue::{variant}\s*\(\s*_", line):
                continue
            if stripped.startswith("|") or re.search(r"\b(?:if|while)\s+let\b", line):
                continue
            if re.search(rf"let\s+Value::{variant}\b", line):
                continue
            if os.path.basename(path) in SERIALIZERS:
                kind = "serializer"
            elif test_line is not None and i > test_line:
                kind = "test"
            elif i in fn_of and arm.search(fn_of[i][1]):
                kind = "rebuild"
            else:
                kind = "code"
            out.append((f"{rel(path)}:{i}", kind, stripped[:88]))
    return out


def audit_producers():
    dead, live = [], []
    for v in value_variants():
        sites = constructor_sites(v)
        if any(s[1] == "code" for s in sites):
            live.append(v)
        else:
            dead.append((v, sites))
    print(f"`Value` variants: {len(live) + len(dead)}   with a producer: {len(live)}   without: {len(dead)}")
    print()
    for v, sites in dead:
        seen = sorted({s[1] for s in sites}) or ["none at all"]
        print(f"  {v} — every construction is: {', '.join(seen)}")
        for site, kind, txt in sites:
            if kind != "test":
                print(f"      {kind:<11} {site:<32} {txt}")
        print()


def census(limit=None):
    """How many nodes of each variant does the front end actually PRODUCE?

    `LOFT_DUMP_SNAPSHOT=<path>` writes the parsed `Data` as JSON and exits, so this is
    front-end only — the corpus is never run.  Two caveats that make this a screen and
    not an oracle, both measured:
      * the snapshot is written POST-`scopes::check`, so a variant the parser builds and
        the lowering removes reads as 0 (`Iter`);
      * a variant generated LATER still reads as 0 (`RawExpr`, built in generation/emit).
    So corpus-absent alone proves nothing.  Absent AND with no producer (see `producers`)
    is what makes a variant dead.
    """
    import subprocess, tempfile, collections
    binary = os.environ.get("LOFT_BIN", os.path.join(ROOT, "target", "debug", "loft"))
    if not os.path.exists(binary):
        print(f"  no binary at {binary} — build it, or set LOFT_BIN")
        return None
    progs = sorted(glob.glob(os.path.join(ROOT, "tests", "scripts", "*.loft")))
    if limit:
        progs = progs[:limit]
    counts = collections.Counter()
    tag = re.compile(r'"k":"([A-Za-z]+)"')
    with tempfile.TemporaryDirectory() as td:
        snap = os.path.join(td, "snap.json")
        env = dict(os.environ, LOFT_DUMP_SNAPSHOT=snap, LOFT_TIMEOUT="60")
        for prog in progs:
            subprocess.run([binary, prog], env=env, capture_output=True)
            try:
                counts.update(tag.findall(open(snap, encoding="utf-8").read()))
            except OSError:
                pass
    print(f"  {len(progs)} programs introspected (the stdlib is parsed into every one)")
    return counts


def audit_dead():
    """The intersection: no producer AND never present in the corpus."""
    print("  screening constructions…")
    screened = {v for v in value_variants()
                if not any(k == "code" for _, k, _ in constructor_sites(v))}
    counts = census()
    if counts is None:
        return
    print()
    print(f"  {'variant':<12} {'corpus nodes':>13}   producer   verdict")
    for v in value_variants():
        n = counts.get(v, 0)
        flagged = v in screened
        if not flagged and n:
            continue  # ordinary live variant, nothing to say
        if flagged and n == 0:
            verdict = "DEAD — no origin, and never present"
        elif flagged:
            verdict = "live (built through a helper the screen reads as a rebuild)"
        else:
            verdict = "live — absent only because of the snapshot's STAGE"
        print(f"  {v:<12} {n:>13}   {'none' if flagged else 'yes':<9}  {verdict}")


mode = sys.argv[1] if len(sys.argv) > 1 else "both"
if mode in ("walkers", "both"):
    print("== walkers ==")
    audit_walkers()
    print()
if mode in ("producers", "both"):
    print("== producers ==")
    audit_producers()
if mode == "dead":
    print("== dead variants (no producer AND absent from the corpus) ==")
    audit_dead()
