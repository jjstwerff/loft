<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Remaining open work — the release gate

Consolidated, prioritised view of what is left, surfaced through the @PLN90 copy/borrow
work and its neighbours. **P0 = genuine correctness/soundness issues that must close before
a wide release. P1 = important optimisations (real wins, but correct-as-is) — lower
priority, done after the P0 issues.** Each item points at its canonical home.

## P0 — genuine issues (must close before release)

### 1. Heap-ownership soundness — loft's stated #1 weakness (wide-release blocker)
[STABILITY_ROADMAP.md § wide-release bar](../../STABILITY_ROADMAP.md) is explicit: the
return-bind-ownership class must be **closed, not merely quiet** (reopened 2026-06-21 after a
dogfood UAF). Three concrete open pieces:

- **A1b — temporary-subject borrow UAF.** A borrowed return (the match-arm `{ items }` now,
  and the struct-field `b.rows` if A2 lands) whose **subject is freed before the result's
  last use** is a use-after-free. `--native` already carries it latently; A1 (`f70a729d`)
  made interp consistent but did **not** close the hole. Fix: the caller materialises a copy
  at a call site where the subject does not out-live the result — the `deps`/lifetime
  decision. Design: [borrow-return/DESIGN.md](borrow-return/DESIGN.md) (slice 2 / failure
  paths F1–F2). **Effort: M.**
- **#462 — residual native record leak.** A native-only `MonsterDef×216` *record* leak in
  the borrowed-view shape (Cluster-A/C borrow-over-free family). Reproduces `--native`,
  interp clean. [STABILITY_ROADMAP.md `462-leak`](../../STABILITY_ROADMAP.md) ·
  [cluster-462](../85-store-lifetime-retirement/cluster-462-slot-reuse-uaf.md). **Gate-1
  note:** same borrow-over-free family as A1b (a borrowed view materialised/freed wrong on
  native) — **likely subsumed by the A1b buffer fix**; re-check `M-462repro` after A1b lands
  before treating it as separate. **Effort: S–M.**
- **Analysis completeness (`O-Complete`).** The `deps`/`ownership_of` analysis is not proven
  **total** — and "an incomplete fact is not a compile error, it is a miscompile or a leak"
  ([OWNERSHIP_MODEL.md § Internal and invisible](../../OWNERSHIP_MODEL.md)). The over-free
  class is being retired (local_source / elem_accumulate done, the match_return crashes P2–P4
  fixed) but soundness is not yet *closed*. This is the umbrella the two items above live
  under. **Effort: L (ongoing).**

### 2. `par`-dispatch over `vector<fn-ref>` — **hang FIXED; native E0308 residual**
Gate-1 re-check: the interpreter **hang is GONE** — the `p4dA2` test (formerly `timeout 15s
exit 143`) now completes in 0.04s and is **un-ignored**. The genuine residual is **native
only**: par-fnref result delivery emits a bare `DbRef` where `(u32, DbRef)` is expected
(`error[E0308]`). Target (to determine before building): the native par-dispatch must
deliver the `(index, value)` tuple the reducer expects, not the bare value. **Effort: S–M
(native codegen).**

### 3. ~~`&` write-back from a call RHS~~ — **RESOLVED** (`@PLN87 #1`)
Gate-1 re-check: `o = mk()` (call RHS) **works on both backends** (`check()` writes back 9);
no parse rejection remains, and the lock-in test `pln87_amp_writeback_from_call_writes_back`
is **un-ignored** (passes). The ownership-transfer machinery already routes a call RHS
through a transferable owned temp. **Done — not a blocker.**

### 4. ~~Parser formatting-sensitivity~~ — DEMOTED (gate-1: not a correctness blocker)
The same source parses to **different IR tree shape by formatting**: single-line →
`else ;` (Null), multi-line → `else { block }` (a real empty block). **Gate-1 capture shows
both lower to IDENTICAL native (`else { DbRef::NULL }`) and identical runtime behaviour** —
post-A1/P3/P4 both the null-else and the empty-block paths emit the correct typed-null. So
this is a **latent fragility, not an active miscompile**: it is what made P2/P3/P4
layout-sensitive, but with those fixed there is no behaviour difference left to gate a
release. Normalising the empty-arm parse so the IR tree is identical is a **robustness**
improvement (moved to P1 below), not a P0 blocker. **Effort: S–M.**

## P1 — important optimisations (lesser priority — correct as-is, do after P0)

These eliminate copies that are *correct but wasteful*; the north-star is the compiler
automatically not copying ([COPY_DIAGNOSTICS.md § North-star](../../COPY_DIAGNOSTICS.md)).

- **A2 — struct-field `b.rows` copy→alias.** `f(b) { b.rows }` still *copies* into `__retbuf`
  via `copy_borrow_tail_into_retbuf`; make it return the alias like the (A1-fixed) match arm.
  Couples with A1b (a borrow needs the temp-subject guard). [borrow-return/DESIGN.md](borrow-return/DESIGN.md)
  slice A2. **Effort: M.**
- **The user-facing copy lint.** Warn on the **Avoidable** bucket only (located, opt-in
  first), with the `&`/restructure hint; stay silent on Implicit, informational on Forced.
  The classification is built (`VerdictRow.class`, `MAT-WORKLIST`); the lint emission is not.
  [COPY_DIAGNOSTICS.md](../../COPY_DIAGNOSTICS.md) phase 2. **Effort: M.**
- **Drain bucket 2 (grow the auto-elision set).** Extend the `Borrow`→`ElidePlan` engine to
  more avoidable copies (var-buffer conservative cases the analysis can't yet prove,
  construction where the source provably out-lives a non-escaping record). Each avoidable row
  in the worklist is a candidate. **Effort: L (incremental).**
- **Explicit copy-intent syntax** (phase 3) — the inverse of `&`: opt into an independent
  copy and silence the lint. **Effort: S (design) + M.**
- **Wasted empty-buffer alloc.** A borrowed return still receives a `__retbuf` it ignores (a
  tiny empty-vector alloc). Optimise away once A2/the ABI settles. **Effort: S.**
- **Normalise the empty-arm parse** (was P0.4). `_ => { [] }` should parse to one canonical
  IR tree regardless of formatting (single-line `else ;` vs multi-line `else { block }`).
  No behaviour difference today (both lower to `DbRef::NULL`), but the divergence is the
  latent fragility that made P2/P3/P4 layout-sensitive — robustness, not correctness.
  **Effort: S–M.**

## Sequencing note

P0.1 (heap-ownership soundness) is the release blocker and the umbrella; **A1b before A2**
(a borrow without the temp-subject guard is the very UAF A1b fixes). The lint (P1) wants the
analysis honest first but does not block release. The parser fix (P0.4) is independent and
cheap.
