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
#   1. `!tp.depend().is_empty()` is USUALLY a DIFFERENT QUESTION — "is this a borrow?" —
#      and needs no veto, because a borrow is not freed either way.  But the SYNTAX does
#      not settle it: when the negated test guards an early exit (`continue` / `return`),
#      the free is on the FALL-THROUGH and the site concludes ownership exactly as a
#      positive test would.  Reading the `!` alone is what hid `scopes.rs`'s
#      `tuple_owned_elem_frees`, which frees a tuple element on empty element deps and
#      consulted no override — the coroutine loop variable it fired on was a borrow of the
#      generator's frame.  So: negated AND early-exit is a positive site.
#   2. The free must be in the region the condition GATES — the block an `if` opens, or the
#      uses of a `let` it binds — not merely nearby.  A 20-line window bled across function
#      boundaries and accused `dispatch::materialises_element`, a classifier that frees
#      nothing.
#   3. Comments are not code.  Matching `OpFreeRef` in prose accused
#      `codegen.rs`'s element-materialise arm, whose comment DISCUSSES a pre-Set free.
#   4. An early-exit guard INVERTS the sense of its test (discrimination 1), so its region is
#      what it FALLS THROUGH to, not the block it opens — and only as far as the keyword
#      actually reaches: `continue` leaves the enclosing loop body, `return` the function.
#      Taking the rest of the function for both accused `scopes.rs`'s
#      `null_arm_record_sources` loop, whose body only pushes to a list while the frees sit
#      far below in the same very long function.
#   5. @FR-O-Override is a per-BINDING flag (`Function::is_skip_free(v)`), so a site reading
#      `depend()` off a bare Type — a call's result type, a block result — has no variable to
#      consult it on and cannot discharge the obligation.  Those sites are counted and reported
#      separately (`no-binding`); their fact question is @FR-O-Oracle's, not this one's.
#      Reading them as positives accuses `null_test` and `own_joined_call_arms`, which are
#      handed a `&Type` and never see a variable number.
#      ⚠ ONLY when the region emits no free.  A site that EMITS one frees a store whatever the
#      proxy was spelled off, so it stays measured — `tuple_owned_elem_frees` reads
#      `elems[idx].depend()` (a tuple element type, no `tp(v)`) and is this check's original
#      catch; skipping it on the spelling would retire the one regression the check exists for.
#   6. A free is REACHED, not only emitted.  `get_free_vars` is what actually emits `OpFreeRef`,
#      and a site reaches it by WRITING the fact that sweep reads — so the free vocabulary has
#      to name the ownership-fact writers, not just the emitters.  Matching emitters alone is
#      what made 25 of the 29 positive verdicts vacuous: the site concludes ownership here and
#      the free lands in another function, so nothing in the gated region matched and every one
#      of them passed without proving anything.  The writer API is small and enumerable
#      (`variables/mod.rs`), and only the writes that can cause a free of THE PROXIED BINDING
#      count — which is two shapes, not the whole API:
#        `make_independent` / `without_deps`  strip the deps, so the scope-exit sweep frees it
#                                             (`scan_set`, scopes.rs:4925)
#        `set_skip_free`                      on the proxied binding, this is the spelling of a
#                                             MOVE: the source is vetoed because the TARGET has
#                                             taken its store and will free it — an interior
#                                             pointer if the proxy was wrong (loft#823 SIGSEGV,
#                                             `gen_set_first_ref_var_copy`, codegen.rs:3207)
#      Adding a dep (`depend`) is the restrictive direction and suppresses a free, so it is
#      absent — and so are `mark_inline_ref` and minting (`work_refs`, `create_unique`), which
#      make a NEW binding or suppress an init: neither frees the binding under test.
#   7. A writer counts only when it NAMES the binding the proxy was read on.  Without that,
#      `mark_inline_ref(db)` in `build_vector_list` (a write to the BACKING var, three lines
#      below a proxy read on `vec`) and a `set_skip_free` fifty lines away in the same
#      `parse_assign_op_inner` block both read as frees, and both are unrelated to the binding
#      the condition concluded about.  Two identifiers in one region is exactly the trap
#      IMPLEMENTATIONS.md names: a test that consults only one is consistent with itself and
#      proves nothing.
#
# FALSIFIED 2026-09-03 — each discrimination path was proved to fire by deleting the veto at
# one site and confirming the check goes red, one at a time:
#   scopes.rs `tuple_owned_elem_frees`      direct emitter, read off a tuple element (no `tp(v)`)
#   scopes.rs `scan_set` displaced strip    negated read + `make_independent` on the same binding
#   codegen.rs `gen_set_first_ref_var_copy` `set_skip_free` on the proxied binding (a move)
# A check whose green is never contrasted with a red is the state this one shipped in.
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
FREE_EMIT = re.compile(r"OpFree|free_ref|emit_free")
# Discrimination 6 — WRITING the ownership fact `get_free_vars` reads reaches a free just as
# surely as emitting one, and is how most of these sites do it.
FREE_MANUF = re.compile(r"\b(?:make_independent|without_deps|set_skip_free)\s*\(\s*([^,)]*)")
# Discrimination 5 — the proxy read has to hang off a variable for the veto to be consultable,
# and discrimination 7 needs to know WHICH variable, so capture it.
BINDING = re.compile(r"\.tp\(\s*\*?\s*([A-Za-z_][A-Za-z0-9_]*)\s*\)\s*(?:\.base\(\))?\.depend\(\)")
# The obligation is discharged by the veto — `Function` exposes it as both `is_skip_free` and
# the bare `skip_free`, and `jo_copy_borrowed_arm_yield` uses the second — or by the one home
# that asks it for you.
DISCHARGE = re.compile(r"skip_free|owns_freeable_store")
FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s")
LET = re.compile(r"let\s+(?:mut\s+)?([a-z_][a-z0-9_]*)\s*=")


def code_only(text):
    """Strip line comments — discrimination 3."""
    return "\n".join(l.split("//")[0] for l in text.split("\n"))


def frees(region, binding):
    """Does the region this condition gates reach a free of `binding`? (discriminations 6-7)"""
    code = code_only(region)
    if FREE_EMIT.search(code):
        return True
    if binding is None:
        return False
    return any(m.group(1).strip().lstrip("*&") == binding for m in FREE_MANUF.finditer(code))


def negated(line, pos):
    """Discrimination 1: is this the `!…is_empty()` is-it-a-borrow form?"""
    i = pos
    while i > 0 and (line[i - 1].isalnum() or line[i - 1] in "_.()[]*&:"):
        i -= 1
    return i > 0 and line[i - 1] == "!"


EARLY_EXIT = re.compile(r"^\s*(continue|return\b)")


def early_exit_guard(region):
    """Discrimination 4: does this gated block do nothing but leave?

    A `if !…is_empty() { continue; }` concludes ownership on the FALL-THROUGH, so the
    negated spelling is a positive site and its region is what comes after, not the block.
    Returns the keyword, which decides HOW FAR the fall-through reaches, or None.
    """
    body = [l for l in code_only(region).split("\n")[1:] if l.strip() not in ("", "}", "{")]
    if not body:
        return None
    kinds = {m.group(1) for l in body for m in [EARLY_EXIT.match(l)] if m}
    return kinds.pop() if len(kinds) == 1 and len(kinds) == len(body) else None


def fallthrough_region(lines, n, fn_end, kind):
    """What an early-exit guard at line `n` gates by falling through.

    A `continue` leaves the enclosing LOOP BODY, so the free it would authorise has to be
    in that body; a `return` leaves the function.  Taking the rest of the function for both
    accused `scopes.rs`'s `null_arm_record_sources` loop, whose body only pushes to a list
    while the frees sit far below in the same (very long) function.
    """
    if kind == "return":
        return "\n".join(lines[n:fn_end])
    depth, j = 0, n
    while j < fn_end:
        depth += lines[j].count("{") - lines[j].count("}")
        j += 1
        if depth < 0:
            break
    return "\n".join(lines[n:j])


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
pos = neg = nobind = reaching = 0
viol = []
for path in sorted(glob.glob(os.path.join(ROOT, "src", "**", "*.rs"), recursive=True)):
    lines = open(path, encoding="utf-8").read().split("\n")
    starts = [i for i, l in enumerate(lines) if FN.match(l)] + [len(lines)]
    rel = os.path.relpath(path, ROOT)
    for n, line in enumerate(lines):
        if line.lstrip().startswith(("//", "///")):
            continue
        for m in PROXY.finditer(code_only(line)):
            fn_end = next((s for s in starts if s > n), len(lines))
            stmt, region = gated_region(lines, n, fn_end)
            bm = BINDING.search(code_only(line), max(0, m.start() - 90))
            binding = bm.group(1) if bm is not None and bm.end() >= m.start() else None
            if negated(line, m.start()):
                # Discrimination 4: an early-exit guard inverts the sense, so the site
                # concludes ownership on what it FALLS THROUGH to.  Resolved BEFORE
                # discrimination 5, whose "does this site free at all?" escape hatch has to
                # read the region the site really gates: `tuple_owned_elem_frees` gates a bare
                # `continue` and does its freeing on the fall-through.
                # Discrimination 1, amended: `!…is_empty()` is usually the is-it-a-borrow
                # question, but a region that WRITES the ownership fact concludes on the proxy
                # in the permissive direction whichever way the test is spelled — `scan_set`
                # reads "this still looks like a borrow" and strips the deps so the sweep frees
                # it anyway.  Reading the `!` alone is what hid that site, so such a region
                # falls through to the obligation instead of being dismissed here.
                kind = early_exit_guard(region)
                if kind is not None:
                    region = fallthrough_region(lines, n, fn_end, kind)
                elif not frees(region, binding):
                    neg += 1
                    continue
            if binding is None and not FREE_EMIT.search(code_only(region)):
                # Discrimination 5: no variable, so no `is_skip_free` to consult — and no
                # free emitted here either, so nothing to hold this site to.
                nobind += 1
                if verbose:
                    print(f"  no-binding {rel}:{n + 1}")
                continue
            pos += 1
            reaches = frees(region, binding)
            if reaches:
                reaching += 1
            if reaches and not DISCHARGE.search(code_only(stmt)):
                viol.append((f"{rel}:{n + 1}", line.strip()[:74]))
            elif verbose:
                # Say WHICH green this is.  "discharged" is a proof — the site reaches a free
                # and consults the veto.  "no-free-in-region" is not: nothing in the region
                # this condition gates reaches a free, so the site is either asking a
                # different question or deciding one further away than the region can see.
                why = "discharged" if reaches else "no-free-in-region"
                print(f"  ok   {rel}:{n + 1}  ({why})")

print(
    f"@FR-O-Proxy — empty-deps ownership sites: {pos} positive, "
    f"{neg} negated (is-a-borrow), {nobind} no-binding (@FR-O-Oracle's question)"
)
# The check's own control.  A positive site whose region reaches no free is a verdict that
# proves nothing, and a run where NONE of them reach one is a green with no content — the
# state this check shipped in.  Print the split so a reader can never mistake the second
# kind of green for the first.
print(f"  {reaching} of {pos} reach a free (emitted or written); {pos - reaching} do not")
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
