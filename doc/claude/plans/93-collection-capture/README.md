<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 93 — Collection capture into closures

## Status

Open — design ready, Approach-A spike done and **to be reverted** (it proved the
naive path is a spray; see the N-count below). No landed implementation yet. Promoted
from [loft-lang/loft#511](https://github.com/loft-lang/loft/issues/511) (a consumer
can't serve a `store_persist_bind`-backed hash from an event-loop handler closure).
The struct-wrap workaround already unblocks that consumer on both backends, so this
plan is an **ergonomics** improvement, not a blocker. Tracked as `@PLN93`.

## Goal

A bare collection variable (`hash` / `vector` / `sorted` / `index` / `spacial`) may be
captured into a closure body and used there for read, index, iteration, and
mutation-through — identically on the interpreter and `--native`.

## Effort + design

- **Effort:** MH
- **Design:** ✓ (detailed — invariant named, chokepoint chosen, claims to falsify listed)
- **Last touched:** 2026-07-06

## The one invariant

> A captured collection is a **borrowed 12-byte DbRef** into the outer scope's
> collection store. The closure record holds the *pointer*, never a copy; every site
> that stores, loads, lays out, or frees the capture treats it as a shared DbRef whose
> referent the outer scope solely owns.

This is exactly the invariant a captured struct `Reference` already satisfies — and
struct captures already work end-to-end on **both backends** (verified: a
`struct Box { h: hash<K[id]> }` captured into a closure reads *and mutates* `b.h[key]`
on interpret and native). A collection variable is *already* a 12-byte DbRef
(`variables/mod.rs` — Vector/Hash/Sorted/Index/Spacial are all `size_of::<DbRef>()`),
and `Parts::DbRef` is a non-owning leaf in the free cascade
(`database/allocation.rs`). The representation already satisfies the invariant; the
work is making every site honor it.

## Design decision — chokepoint, not spray

**Approach A (naive):** keep the attr typed `Hash/Vector/…` + a `share_sentinel` dep,
and teach every storage/codegen site to special-case it. "Treat as DbRef" must be
independently re-stated at **~11 sites**, every omission **silent**: `synthesize_closure_record`,
`typedef.rs` fill_database (keyed arm), `typedef.rs` (vector arm — separate!),
`get_field`, `set_field_check`, native `emit_field`, native `emit_def_create_recurse_fields`,
native read-op, native write-op, the body index/iterate path, and write-through
lvalue. The spike already shows the bite: interp hash-read works, but native panics
(`u16::MAX` used as a content type), vector read is wrong, mutation silently no-ops.
`N × silence` is high — **do not ship Approach A.**

**Approach B (chosen):** store the captured collection attr as a **`Reference`-DbRef —
the same representation struct captures use** — so schema / read / write on both
backends reuse the *proven* Reference path (P1 proves it on native). Recover the
collection *type* for the body from `capture_context`, which already carries it. Two
sites remain, neither silent-if-omitted:

1. `synthesize_closure_record` — emit the collection capture as `Reference`-DbRef.
2. body resolution (`objects.rs:346-364`) — type the captured name as its original
   collection type (from `capture_context`), so `h[key]` / iterate / `+=` type-check,
   while the value is the `OpGetDbRef` 12-byte DbRef.

**N ≈ 2 vs ≈ 11**, and the collapsed sites are proven code, not new special-cases.
**Prediction to validate the build against:** ~2 edited sites + the body override. If
it balloons, that is the alarm that a load-bearing claim (C1/C4) was false.

## Composition matrix — Stage A

Harness: `tests/scripts/`-bound boundary matrix (hash read/miss/iterate, vector,
sorted, index, spacial, multi-capture, mutation-through, non-zero default), every cell
hand-computed, re-run on `--interpret` **and** `--native` plus a `LOFT_STORES=warn`
leak check at each step. The feature is done when every cell is green on both backends
and the probes are graduated to `tests/scripts/NNN-collection-capture.loft`.

**Load-bearing claims to falsify first (Step 0 — expect to falsify):**

| Claim | Probe (both backends) | On falsify |
|---|---|---|
| **C1** hash/vector/… DbRef == struct Reference DbRef in shape | store a capture as Reference-DbRef, read+`h[key]` in body | reassess — invariant wrong |
| **C2** body type-override clean (attr=Reference, body types=collection) | same test — parser accepts `h[key]` on overridden type | reassess |
| **C3** write-through works via the Reference path | mutate inside closure, assert outer changed | **read-only + LOUD parse rejection of mutation (never a silent no-op)** |
| **C4** all 5 kinds one family (vector is position-indexed, separate arm) | run vector cell separately | real domain axis — record, don't force |
| **C5** borrow lifetime — record death frees only record; escape guarded; leak-clean | `LOFT_STORES=warn`; closure escaping the collection scope | rely on / extend the #318 escape guard |

## Sub-arcs

| Item | Status |
|---|---|
| **0** — Falsify C1–C5; revert Approach-A WIP to a clean baseline | Open |
| **1** — hash, read-only, both backends (synthesize Reference-DbRef + body override) | Open |
| **2** — vector / sorted / index / spacial, read-only (confirm C4) | Open |
| **3** — mutation-through (only if C3 held; else the loud rejection) | Open / gated on C3 |
| **4** — escape / lifetime guard (C5) | Open |
| **5** — harden + land: graduate matrix to `tests/scripts/`, full `wrap` + `native` suites, clippy/fmt, doc in LOFT.md, close #511 | Open |

## Out of scope (record, don't absorb)

- Capturing a collection **by value** (copy) — borrow is the semantics, not a copy.
- Non-inline closure sources (returning a capturing fn-ref) beyond the existing #318
  rules.

## See also

- Implements the closures section of [`../../LOFT.md`](../../LOFT.md); ownership beacon
  [`../../OWNERSHIP_MODEL.md`](../../OWNERSHIP_MODEL.md) (DbRef = borrow, non-owning leaf).
- Source issue: [loft-lang/loft#511](https://github.com/loft-lang/loft/issues/511).
- Tracker: `@PLN93` ([loft-lang/plans#93](https://github.com/loft-lang/plans/issues/93)).
- Design method: `.claude/skills/design-protocol` (the N-count + falsify-the-claim discipline).
