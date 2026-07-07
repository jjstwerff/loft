<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN94 Check C — under-free / leak detection (design, measured before committing)

Design Protocol 1: a throwaway gated prototype (`LOFT_OWN_UNDERFREE`) was built and **measured
against real corpora BEFORE the real design** — store-lifetime is an exact-invariant domain, so the
numbers, not intuition, fix the scope. The prototype has been reverted; its measurements are the
foundation below.

## The scope the measurements forced (read this first — it is narrower than it looks)

Under-free is **not** the mirror image of over-free. Two obstacles a static check faces that the
over-free checks (A/B) do not:

1. **Runtime-path leaks are statically invisible.** `LOFT_NO_JOIN_OWN` leaks a store on `local_source`
   (`leak=1`) — but the prototype flagged the SAME count (12) on the correct plan and the leaky one.
   Its leak is a `Join` store freed **conditionally** (`OpFreeRefIfDistinct`): the free op is
   statically PRESENT, it just does not execute on the leaking path. A static "no free op exists"
   check cannot see that. **This class is structurally the runtime leak-check's** (`check_store_leaks`).
2. **Transfer-completeness governs the false-positive rate.** A leak is "an owned store never freed
   AND never transferred out." Miss a transfer path → cry wolf on a moved value.

So Check C's honest target is the **DEFINITE leak**: an `Owned` heap-store local with **no free op on
any path** and no transfer out — a store leaked on *every* execution. Sound for that subclass (a flag
is a real leak, modulo transfer-completeness); it does **not** catch conditional/`Join` leaks.

## C.0 — the enabling PREREQUISITE (found while building C.1; it is a BLOCKER)

Building the transfer-out set (below) drove the false-positive rate to 0–4/file — then the residuals
exposed a deeper constraint. `n_main`'s free set came back **empty** (`freed={}`) even though its
compiled IR has 7 `OpFreeRef`s. Cause: **the oracle observes the PRE-codegen IR.** `oracle(data)` is
the third line of `scopes::check` (`src/scopes.rs:1554`); `check` ends at line 1762; `get_free_vars`
(which inserts the frees) runs at ~1877, during codegen, **after** `check`. So at oracle time a
freshly-checked USER function's `def.code` has **no frees yet** — they are appended later.

Consequences:
- **Check C cannot run at the current call site** — its whole premise is reading the emitted frees.
- **Check B is near-VACUOUS on user functions today** — the same reason. Its 0-RED across 377 scripts
  is partly because it had no user-function frees to inspect (it meaningfully checks only pre-cached
  stdlib, whose `def.code` carried frees from an earlier compile). This is a real limit of the
  shipped check, surfaced here — worth widening when C.0 lands.
- **Check A is unaffected** — it is fact-based (needs no frees), which is why the A1b catch works.

**C.0 = give the free-based checks a POST-codegen view. ✅ BUILT (dev tier, 2026-07-07).**
`ownership_cfg::oracle_free_checks` runs at the END of `scopes::check` (after `get_free_vars`),
gated on `LOFT_OWN_ORACLE=check-dev`; Check A stays at the pre-codegen site. It is a pure observer
(SI-1 held) and NEVER on the default `check` path.

**C.0 immediately exposed the REAL blocker (the 377-script sweep, not the tiny corpus).** With frees
now visible, Checks B + C produce **153 findings across `tests/scripts`** — and they are dominated by
ONE root cause: **the ownership fact is not materialisation-aware.** A struct copy `r1 = a` (a param)
reads `Borrowed(a)`, but the shipped code COPIES it and frees the owned copy — so:
- Check B fires: `free-of-borrowed r1 is Borrowed(a)` (it IS freed, legitimately, as an owned copy);
- Check C fires: loop/iteration/container temps whose owned-copy frees the fact does not see as owning.
This is the `n_choose` gap, pervasive. **The work Check C needs is not more transfer-set tuning — it
is a materialisation-aware FACT** (classify a copied `x = borrowed-source` as `Owned`, matching what
codegen emits). That is a Phase-3-level precision upgrade.

**The workflow (so the checks are never reverted again): a gated dev tier + a FP RATCHET.** The dev
checks live in the code behind `check-dev`; `tests/ownership_oracle.rs::oracle_dev_free_check_ratchet`
(`#[ignore]`) counts their findings over `tests/scripts` and asserts `≤ DEV_FP_BASELINE` (currently
**153**). Every fact/transfer improvement LOWERS the baseline; a regression fails; at 0 they promote
into `check`. Progress is a number going down, not code toggled in and out.

The false-positive MACHINERY already works: the heap-type filter (70–124 → 9–35/file) and the
transfer-out set (returned ∪ consumed-into-container, → 0–4/file on the small corpus) are in
`run_free_checks`; the residual 153 is the fact-precision blocker above, not the machinery.

## Coexistence — what it adds, what it does NOT replace

Check C runs BESIDE the shipped **runtime** leak-check, catching a **complementary** sub-class:

| | catches | misses |
|---|---|---|
| **runtime leak-check** (`check_store_leaks`, `LOFT_STORES=warn`) | any leak on an EXECUTED path (incl. conditional/`Join`) | leaks on paths a given run does not take |
| **Check C** (static) | DEFINITE leaks on ALL paths — incl. a deleted free the tests never execute | conditional/runtime-path leaks (the `Join` class) |

Neither replaces the other; the pair covers more. This EXTENDS the oracle from the over-free class
(A/B) to the under-free class — the direction the user asked for — without pretending to subsume the
runtime detector.

## The invariant

> **An `Owned`, HEAP-typed, LOCAL var (not a parameter) that appears in NO free op anywhere in the
> function body and is not transferred out (returned, moved into a container, or adopted by a callee)
> leaks its store — RED.**

## What the prototype measured (the failure paths, in order of magnitude)

Gated prototype on correct corpora (06-capture, 505, fuzz cells):

- **Raw: 70–124 false positives / file.** Dominated by SCALARS classified `Owned` (loop counters
  `k#index`, `i`, `p`; ints `np`, `depth`) — a scalar has no heap store to leak.
  → **Fix 1 (heap filter): `func.tp(v).heap_dep().is_some()`.** Drops FPs to **9–35 / file**. Cheap,
  done in the prototype.
- **Residual 9–35 / file: ONE class — element/scratch temps consumed into a container.** `_elm_1`,
  `_elm_2` (`OpNewRecord` → `OpFinishRecord(container, _elm_N)`), `_hash_scratch_1`, `_reduce_acc_1`.
  These are **moved** into a collection / reduction, not leaked; the prototype's return-only transfer
  set misses them.
  → **Fix 2 (consume-tracking): the real work — extend the transfer-out set.**

## The transfer-out set (Fix 2 — the load-bearing piece)

A var is transferred out (⇒ NOT a leak even though this frame does not free it) iff it is:

- **returned** — appears in a `Value::Return(…)` (prototype has this), OR its store backs the return
  (named in `def.returned` deps);
- **consumed into a container** — the element/source arg of `OpFinishRecord(container, v, …)`,
  `OpAppendVector(container, v, …)`, or `OpCopyRecord(v, dst, …)` with the source-free bit;
- **adopted by a callee** — passed as an argument to a call whose callee frees that parameter (read
  the callee's own free set / `deps`; conservative default: a heap arg to a non-native call is
  assumed possibly-adopted → excluded, biasing toward no-cry-wolf).

**Bias, stated:** when unsure whether a path transfers, EXCLUDE (treat as transferred). That
under-approximates the leak set — Check C may MISS a leak — which is SAFE for a gate (the runtime
leak-check is the completeness backstop) and keeps it from crying wolf. Mirrors Check B's conservatism
(unconditional frees only).

## Re-assertion sites — N = 1

One pass over the exit-state fact + the (freed ∪ transferred-out) set. No spray; a new container/consume
op is one more entry in the transfer-out set, `log()`ged if unmodeled.

## Steps (each independently committable, each states its gate)

- **C.0 — the free-based checks get a POST-codegen view (BLOCKER, see above).** Add a second oracle
  entry point after codegen (or thread the pre-codegen fact to a post-codegen free-walk); prove Check
  B now sees user-function frees (its coverage was vacuous there). **Gate:** on the A1b test,
  `collect_free_targets(n_h)` is non-empty; SI-1 still holds (still an observer). Everything below
  stacks on this.
- **C.1 — heap filter + returned + consume-into-container.** Re-add the check (gated
  `LOFT_OWN_UNDERFREE` first) with Fix 1 + the return and `OpFinishRecord`/`OpAppendVector`/
  `OpCopyRecord` transfer set. **Gate:** FP on the correct corpus (7 probes + 505 + 54 fuzz cells)
  drops from 9–35/file to a small hand-verifiable set; adjudicate each residual (a real gap → extend
  the set; a real leak → keep).
- **C.2 — callee-adoption.** Add the "heap arg to a call whose callee frees it" exclusion (conservative
  default: exclude). **Gate:** FP → 0 across the correct corpus + fuzzer; SI-1 holds.
- **C.3 — true-positive (the 4.3 injected fault this finally enables).** Hand-delete one `OpFreeRef`
  from a corpus program's emitted plan (or a test fixture) → Check C RED, naming the exact store +
  function; the un-mutated program clean. Document that `LOFT_NO_JOIN_OWN` (a CONDITIONAL leak) is
  NOT caught here — it is the runtime leak-check's, by design.
- **C.4 — promote off the gate + land.** Fold Check C into `check` mode (ungated), add
  `oracle_flags_a_deleted_free` + extend `oracle_clean_on_correct_corpus` to assert 0 under-free REDs
  in `tests/ownership_oracle.rs`. **Gate:** the binary is green.

## Failure paths (enumerated)

- **Transfer set incomplete → false positive** (crying wolf on a moved value). The 9–35 residual is
  exactly this; C.1/C.2 close the known shapes; the conservative-exclude bias caps the blast radius.
- **Transfer set too broad → missed leak** (false negative). Accepted by design — the runtime
  leak-check backstops; a static gate that cries wolf is worse than one that occasionally defers.
- **Conditional/`Join` leak → structurally out of scope** (the `NO_JOIN_OWN` measurement). Not a bug
  in Check C; `log()` nothing — it is simply the other detector's class.
- **Text vars** — freed via `OpFreeText`; include it in the free set (unlike Check B, which excluded
  it — there the issue was over-free of a copied sub; here we need the free to COUNT so text locals are
  not spurious leaks).

## Relation to the formal skeleton

Check C proves the DUAL of Check B's over-free lemma: *every `Owned` heap local is freed exactly once
on every path (no leak)* — the O-Derived "free … once, at scope exit" half. It is the
**freed-exactly-once sub-invariant** the `formal/ownership.md` obligation ledger lists as OUT OF SCOPE
for the over-free proof; Check C brings the DEFINITE-leak fragment of it in scope, the conditional
fragment staying with the runtime witness. Update the ledger when C.4 lands.
