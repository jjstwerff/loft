#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# o_proxy_check.py — enforce `formal/ownership.md`'s @FR-O-Proxy obligation:
#
#     a site that FREES on the empty-`deps` proxy MUST also consult @FR-O-Override.
#
# WHY there is an obligation at all.  `tp.depend().is_empty()` is how a site asks "does
# this binding own its store?", and it is a PROXY, not the oracle: a borrow whose dep list
# was never populated also reads empty, so the proxy answers "owner" for a borrower.
# `Function::is_skip_free` is the veto that makes it safe at a free site.  Consulting the
# veto only at the scope-exit sweep is what left an unconditional pre-Set free reachable
# inside a loop body, where it landed on the NEXT iteration's store — stale bytes without
# `LOFT_POISON`, SIGSEGV with it (loft#723).
#
# WHAT IS AND IS NOT A VIOLATION — the three discriminations this check makes, each of
# which was a false positive before it made them:
#
#   1. `!tp.depend().is_empty()` is a DIFFERENT QUESTION — "is this a borrow?" — and needs
#      no veto, because a borrow is not freed either way.  Only the positive form concludes
#      ownership.  (8 of the 28 sites are this form.)
#   2. The free must be in the region the condition GATES — the block an `if` opens, or the
#      uses of a `let` it binds — not merely nearby.  A 20-line window bled across function
#      boundaries and accused `dispatch::materialises_element`, a classifier that frees
#      nothing.
#   3. Comments are not code.  Matching `OpFreeRef` in prose accused
#      `codegen.rs`'s element-materialise arm, whose comment DISCUSSES a pre-Set free.
#
# A REPORT that exits 1 on a violation, so it can gate.  Verdicts and the rule map live in
# doc/claude/formal/IMPLEMENTATIONS.md § The variable-lifetime map.
#
# Usage:  python3 scripts/o_proxy_check.py [-v]

import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROXY = re.compile(r"depend\(\)\.is_empty\(\)")
# Emitting a free, not merely naming one.
FREE = re.compile(r"OpFree|free_ref|emit_free")
FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s")
LET = re.compile(r"let\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*=")


def code_only(text):
    """Strip line comments — discrimination 3."""
    return "\n".join(l.split("//")[0] for l in text.split("\n"))


def negated(line, pos):
    """Discrimination 1: is this the `!…is_empty()` is-it-a-borrow form?"""
    i = pos
    while i > 0 and (line[i - 1].isalnum() or line[i - 1] in "_.()[]*&:"):
        i -= 1
    return i > 0 and line[i - 1] == "!"


def gated_region(lines, n, fn_end):
    """Discrimination 2: the statement, and the region its result actually gates."""
    a = n
    while a > 0 and not re.search(r"[;{}]\s*$", lines[a - 1]) and n - a < 14:
        a -= 1
    b = n
    while b + 1 < fn_end and not re.search(r"[;{]\s*$", lines[b]) and b - n < 14:
        b += 1
    stmt = "\n".join(lines[a : b + 1])
    if lines[b].rstrip().endswith("{"):
        depth, j = 0, b
        while j < fn_end:
            depth += lines[j].count("{") - lines[j].count("}")
            j += 1
            if depth <= 0:
                break
        return stmt, "\n".join(lines[b:j])
    m = LET.search(code_only(stmt))
    if m:
        # A `let NAME = <proxy cond>;` gates whatever the `if NAME …` blocks contain — the
        # free is inside the block, not on the line naming NAME.  Collecting only the
        # mentioning LINES is what made this check vacuous on the very regression it exists
        # for, so take each use-line's block too.
        nm = m.group(1)
        out = []
        j = b
        while j < fn_end:
            if re.search(rf"\b{nm}\b", code_only(lines[j])):
                out.append(lines[j])
                if lines[j].rstrip().endswith("{"):
                    depth, k = 0, j
                    while k < fn_end:
                        depth += lines[k].count("{") - lines[k].count("}")
                        k += 1
                        if depth <= 0:
                            break
                    out.extend(lines[j + 1 : k])
                    j = k
                    continue
            j += 1
        return stmt, "\n".join(out)
    return stmt, ""


verbose = "-v" in sys.argv
pos = neg = 0
viol = []
for path in sorted(glob.glob(os.path.join(ROOT, "src", "**", "*.rs"), recursive=True)):
    lines = open(path, encoding="utf-8").read().split("\n")
    starts = [i for i, l in enumerate(lines) if FN.match(l)] + [len(lines)]
    rel = os.path.relpath(path, ROOT)
    for n, line in enumerate(lines):
        if line.lstrip().startswith(("//", "///")):
            continue
        for m in PROXY.finditer(code_only(line)):
            if negated(line, m.start()):
                neg += 1
                continue
            pos += 1
            fn_end = next((s for s in starts if s > n), len(lines))
            stmt, region = gated_region(lines, n, fn_end)
            if FREE.search(code_only(region)) and "skip_free" not in code_only(stmt):
                viol.append((f"{rel}:{n + 1}", line.strip()[:74]))
            elif verbose:
                print(f"  ok   {rel}:{n + 1}")

print(f"@FR-O-Proxy — empty-deps ownership sites: {pos} positive, {neg} negated (is-a-borrow)")
if not viol:
    print("  ok — every site that frees on the proxy also consults @FR-O-Override")
    sys.exit(0)
print(f"\n  {len(viol)} site(s) FREE on the empty-deps proxy without consulting the override:\n")
for site, text in viol:
    print(f"    {site}\n      {text}")
print("""
  An empty dep list does not mean "owner" — it means "nothing recorded a dep here", which
  is also true of a borrow nobody populated.  Freeing on it releases a store someone else
  owns.  Add `&& !<vars>.is_skip_free(v)` to the condition, or read @FR-O-Oracle
  (`use_analysis::ownership_of`) instead of the proxy.

  formal/ownership.md § The facts that answer it.""")
sys.exit(1)
