<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 154 — A stack shadow that knows a slot is uninitialised, the wrong width, or stale

Tracker: [@PLN154](https://github.com/loft-lang/plans/issues/154).

## Status

**All six phases shipped 2026-09-06.  Phase 0 came back RED in the sense it was cut to
allow — the tag moved off the accessor and down to `Store::addr_mut::<T>` before phases 1-3
were built ([phase0-census.md](phase0-census.md)).  Phase 1 shipped `LOFT_VERIFY_STACK=1`
GREEN on its gate and moved its own falsification target: neither loft#1386 nor loft#1254 is
in the `uninit` state, and the witness is the nullable-local pre-init defect instead
([phase1-uninit.md](phase1-uninit.md)).  Phase 2 shipped the tag GREEN on two of its three
named guards, on the HANDLE axis — the width axis reports the frame's own composite slots and
was given up for a measured reason, and loft#1070 is out of a stack shadow's reach because
its defect is a heap record's layout ([phase2-width-kind.md](phase2-width-kind.md)).
Phase 3 shipped the stale-on-grow half GREEN: all three open issues in the class report
exactly one site, the view, and a five-probe matrix with a no-relocation negative control
finds no false positive ([phase3-stale.md](phase3-stale.md)).  Phase 4 shipped its DRIVER
(200 distinct control builds is machine time, not a session) and phase 5 armed the gate in
the nightly at ~2x the in-process corpus, calibrated both ways
([phase5-nightly.md](phase5-nightly.md)).**

A third of the machinery is already in the tree:
`LOFT_UAF_GEN` ([`src/keys.rs:1335`](../../../../src/keys.rs)) keeps an offset-keyed shadow
of the operand stack, stamping each pushed `DbRef` with its store's generation and comparing
at the pop.  [DEBUG.md](../../DEBUG.md) records its ceiling — *"only the window between a
push and its pop — a ref that goes stale sitting in a FRAME slot is invisible to it, which
is why it never saw loft#723"*.  This plan widens that shadow from one tag kind over the
push→pop window to a tag plus a validity state over the whole frame.  The argument, the
evidence per state, and the out-of-scope list live on the issue; this README carries the
phases and what makes each one go red.

## Goal

`LOFT_VERIFY_STACK=1` reports, at the read, a stack slot that no path wrote, a slot read at
a width or kind nobody wrote it at, and a slot whose `DbRef` names a record that has since
moved — silent across the corpus at HEAD, and reporting on the build each of the named
guards was written to catch.

## Effort + design

- **Effort:** MH — six phases, none straddling a day.
- **Design:** ~ — the mechanism is settled (widen the existing shadow); phase 0 is the
  design probe, and phase 2's legal-pun list is the one open call.
- **Value category:** S (silent failure).  Every state this plan makes observable is one
  that answers a plausible wrong value today with nothing said.
- **Last touched:** 2026-09-06.

## Composition matrix — Stage A

The shadow's correctness is *does the tag follow the value*, and the axes that decide it are
the stack-layout axes, not the language's.  Enumerate as `/tmp` probes on `--interpret`
before phase 1, one axis moved per probe, and count the axes held FIXED
(`python3 scripts/matrix_axes.py`):

| Axis | Domain |
|---|---|
| **Slot zone** | zone 1 (≤8 B, greedy interval colouring, reused across dead vars) · zone 2 (>8 B, sequential IR-walk) — [SLOTS.md](../../SLOTS.md) |
| **Write path** | `put_stack::<T>` · the raw `copy_block` return slide (`state/mod.rs:2249`) · `OpReserveFrame` · `clear_stack` at a loop iteration end |
| **Residence** | eval TOS (pushed and popped inside one statement) · a frame slot live across statements · a loop variable live across iterations |
| **Value kind** | scalar 1/2/4/8 B · `DbRef` (12 B) · text · a stepped slot under `LOFT_ALIGN=1` (12 B value in a 16 B slot, 4 B padding) |
| **Origin** | a concrete type · a generic monomorph's local · a `__nullable<S>` sentinel · an enum variant sharing a slot with its siblings (C89) |

Zone 1's slot REUSE and the `LOFT_ALIGN` padding are the two cells most likely to produce a
false positive: a reused slot carries the previous occupant's tag unless the reuse clears it,
and padding bytes are never written by anything.  This matrix also closes
[STABILITY_SWEEP.md](../../STABILITY_SWEEP.md) F5's recorded deferral — *"DEFERRED: full
odd-size adjacency matrix"* — which is the same question asked from the layout side.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **0** — the bypass census | [phase0-census.md](phase0-census.md) | 1106 corpus programs: `put_stack` carries 74.5 % of the bytes, is 1 of 33 write sites, and **no program** is covered by it alone | **Shipped 2026-09-06 — RED** |
| **1** — `uninit` | [phase1-uninit.md](phase1-uninit.md) | 1106 of 1106 runnable corpus programs silent at HEAD on `--interpret`; four distinct sites REPORTED on `64437246` (the nullable-local pre-init control), where loft#1386's control is silent and loft#1254's control moves the `Partial` counter instead.  Ships `LOFT_VERIFY_STACK_INJECT=1` in the same commit | **Shipped 2026-09-06 — GREEN, target moved** |
| **2** — `width + kind` | [phase2-width-kind.md](phase2-width-kind.md) | 1106 corpus programs silent at HEAD on `--interpret`; loft#1028's control reports `handle 12` read as `i64` (and answers `65535`), loft#1016's reports four sites (and answers `4294967198`).  loft#1070's control reproduces and the shadow is silent — its record has the type variable's LAYOUT, so the defect is in a store, not in a frame slot | **Shipped 2026-09-06 — GREEN on 2 of 3; the third is out of scope, measured** |
| **3** — `stale-on-grow` | [phase3-stale.md](phase3-stale.md) | All three issues are OPEN, so HEAD *is* the broken build: loft#1373, #1377 and #1384 each report exactly ONE site, the stale view.  Silent corpus-wide, and a five-probe matrix ([probes/](probes/README.md)) with a no-relocation negative control | **Shipped 2026-09-06 — GREEN** |
| **4** — yield vs. the falsification corpus | [phase4-yield.sh](phase4-yield.sh) | 264 guards carry a real `@falsified-at:` ref across **200 distinct** builds, so the full run is hours of machine time rather than a session: the driver takes the refs covering the most guards first, shares one target directory and removes each worktree as it goes, and a partial run states its own sampling.  The yield is the REPORT; the gate is the other direction — a shadow report on a build its guard calls clean is a false positive, and is red (loft#1373 / #1377 / #1384 excepted: they are still OPEN, so HEAD is a broken build for those) | Driver shipped 2026-09-06; the run is machine time |
| **5** — arm it in the nightly | [CI_BUDGET.md](../../CI_BUDGET.md) | The sweep itself: it goes red on a false positive phase 4 did not cover | Open |

## Phase ordering

1. **Phase 0 ran first, and moved the hook.**  It asked whether the two accessors are where
   the bytes arrive, and the answer is no: `put_stack` is 1 of 33 write sites and no corpus
   program is covered by it alone, so a phase-1 `uninit` check keyed there would have
   reported `OpPutInt` — the language's commonest assignment — as a read of an unwritten
   slot.  The tag goes to `Store::addr_mut::<T>`, which 32 of the 33 sites already call and
   which carries the type phase 2 needs.  Details and what it cannot see:
   [phase0-census.md](phase0-census.md).
2. **1 → 2 are one mechanism widened**, each validated on its own: `uninit` is the absence of
   a tag, `width + kind` is the tag.  The tag is free at the write — `put_stack` is generic
   over `T`, and `LOFT_UAF_GEN` already reads `TypeId::of::<T>()` there.  Phase 1 measured
   how much of the plan's own evidence falls on each side of that line, and the answer is
   that **the width half carries it**: an eval slot is stepped to eight bytes while a
   `boolean` writes one, so a slot is nearly always recycled with *something* in it and the
   pure-absence state is narrow.  Phase 1 counts the `Partial` reads it declines to report,
   which is phase 2's queue.
3. **3 is a second mechanism** — a hook at the grow/realloc site plus a shadow scan — and did
   not depend on 2.  It does depend on 1 and 2 for its EXACTNESS, though, which is worth
   recording: the scan reads only the slots the shadow already says are the base of a handle,
   so it never has to treat an aligned word that happens to look like a record id as a
   reference.  It also needed the store identity phase 0 predicted the plan would owe.
4. **4 after every detector phase it measures**, and it is the phase that decides whether 5
   is worth its CI minutes.

## What phases 2 and 4 are actually for

They are a **search**, not only a regression check.  A width-tagged corpus sweep reports every
monomorph-layout mismatch the suite executes, including the ones nobody has filed — 20 of the
last 500 bug issues are generic-shaped, and #1070 (`sev:high`, `silent-wrong`) is the shape:
*"a local declared inside a generic `if` arm is built against the type variable's layout: a
wrong number, silently"*.  This is also the class the *"rustc already checks it"* argument does
NOT cover: rustc catches the shapes where the generated Rust is ill-typed (#1386, #1355,
#1354, #1325 all report E0308/E0382 on `--native` while the interpreter answers), but a
monomorph whose layout is *consistently* wrong type-checks against its own wrong width, so
native is silent too and #1070 is wrong on both backends.

## Open design questions

1. **The legal-pun list (phase 2) — ANSWERED, by dropping the question.**  The list would
   have been *"every composite slot in the interpreter"*: a strict *types must match* rule
   reported 43 of the first 180 corpus programs, and every class was the frame's own layout
   (the 20-byte fn-ref slot read as an `i64` and a `DbRef`, the iterator state `OpStep` reads
   as two `u32`s, a `boolean` consumed at its stepped eight-byte slot, a null sentinel read
   as the type it stands for).  What phase 2 reports instead is a **handle crossing a
   value**, which needs no list because a slot the compiler typed as a reference is a
   reference on every path.  The width disagreements are counted, so the pun population stays
   measurable for a phase that has the compiler's slot types rather than the accessor's.
   Details: [phase2-width-kind.md](phase2-width-kind.md).
2. **Zone-1 slot reuse.**  A reused slot must have its tag cleared by whatever reuses it, and
   `assign_slots` — not the runtime — is the authority on when that happens
   ([SLOTS.md](../../SLOTS.md)).  Whether the clear belongs at the reuse site or at scope
   entry is a phase-1 call, and the matrix cell above is what decides it.
3. **Cost, and therefore where it can run.**  `LOFT_STRICT_STORES` is probe-only because
   never reusing a slot walks a long run off the `u16` store space; this shadow has no such
   ceiling, so the question is only instructions.  Phase 4's measurement decides whether
   phase 5 is a nightly sweep or a probe-only lever.

## See also

- [phase1-uninit.md](phase1-uninit.md) — what `uninit` witnesses, measured on three control
  builds: the one guard that is in the state, and the two issue-named defects that are not.
- [phase2-width-kind.md](phase2-width-kind.md) — the tag, why the width axis is counted rather
  than reported, and the boundary loft#1070 marks.
- [phase3-stale.md](phase3-stale.md) — the growth half, and [probes/](probes/README.md), the
  five-cell matrix that says the silence is earned.
- [shadow-control.sh](shadow-control.sh) — build a control tree WITH the shadow on it, which
  is what phase 4 needs and what `make falsify` cannot do.
- [phase4-yield.sh](phase4-yield.sh) — the yield run over the falsification corpus, refs by
  coverage so a partial run buys the most evidence.
- [DEBUG.md](../../DEBUG.md) § the detector table — every existing lever this one sits beside,
  and the `LOFT_UAF_GEN` / `LOFT_STRICT_STORES` / `LOFT_POISON` division of labour.
- [SLOTS.md](../../SLOTS.md) — the frame layout the shadow mirrors.
- [STABILITY_SWEEP.md](../../STABILITY_SWEEP.md) F5 — the same question from the layout side,
  with the odd-size adjacency matrix this plan's Stage A closes.
- [TESTING.md](../../TESTING.md) § *A guard that never failed is not a guard* — `make falsify`, and the `@falsified-at:`
  corpus phase 4 measures against.
- [@PLN154](https://github.com/loft-lang/plans/issues/154) — the tracker issue, which carries
  the evidence per state and the out-of-scope list (a static bytecode verifier;
  owner-vs-view tagging).
