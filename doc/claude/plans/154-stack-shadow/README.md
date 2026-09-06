<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 154 — A stack shadow that knows a slot is uninitialised, the wrong width, or stale

Tracker: [@PLN154](https://github.com/loft-lang/plans/issues/154).

## Status

**Future — designed, nothing built.**  A third of the machinery is already in the tree:
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
| **0** — the bypass census | this README | A byte count per corpus script (`put_stack` vs. raw store writes) + the site list.  RED if the bypass fraction makes accessor-keyed shadowing untenable, which re-routes the shadow to the store write path before anything is built on it | Open |
| **1** — `uninit` | @PLN154 | Silent corpus-wide at HEAD on `--interpret`; REPORTS under `make falsify GUARD=<loft#1386's guard> REF=964bab93`.  Ships `LOFT_VERIFY_STACK_INJECT=1` in the same commit | Open |
| **2** — `width + kind` | @PLN154 | Reports on `tests/scripts/1070-generic-arm-local-type-row.loft`, `1028-generic-null-typed-per-monomorph.loft` and `1016-generic-null-default-instantiation.loft` at the parent of each fixing commit; silent corpus-wide at HEAD | Open |
| **3** — `stale-on-grow` | @PLN154 | Reports on the guards for loft#1373 / #1377 / #1384 at their recorded `@falsified-at:` refs; silent corpus-wide at HEAD | Open |
| **4** — yield vs. the falsification corpus | this README | The 277 guards carrying a real `@falsified-at:` ref, each run under the shadow on the build it was written to catch.  The yield is the REPORT; the gate is the other direction — a shadow report on a build its guard calls clean is a false positive, and is red | Open |
| **5** — arm it in the nightly | [CI_BUDGET.md](../../CI_BUDGET.md) | The sweep itself: it goes red on a false positive phase 4 did not cover | Open |

## Phase ordering

1. **Phase 0 before anything.**  The bypasses are the whole engineering problem and the repo
   has paid for this lesson once already: the comment at `state/mod.rs:2249` records that an
   untracked `copy_block` slide *was* `LOFT_UAF_GEN`'s residual false positive — *"a loop
   calling a struct-returning function reported `gen 0 at push` on every backend, on programs
   with no stale read at all"*.  A census of which bytes reach the stack outside
   `put_stack` either confirms two chokepoints are enough or moves the shadow to the store
   write path, and it costs a compile.
2. **1 → 2 are one mechanism widened**, each validated on its own: `uninit` is the absence of
   a tag, `width + kind` is the tag.  The tag is free at the write — `put_stack` is generic
   over `T`, and `LOFT_UAF_GEN` already reads `TypeId::of::<T>()` there.
3. **3 is a second mechanism** — a hook at the grow/realloc site plus a shadow scan — and does
   not depend on 2.  It can be taken before 2 if the stale class is the more urgent one.
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

1. **The legal-pun list (phase 2).**  loft puns on purpose — `__nullable<S>` sentinels, enum
   variants sharing one slot ([DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C89), tuple
   packing, `LOFT_ALIGN`'s stepped slots — so a strict *types must match* rule reports the
   language's own idioms.  The admitted set has to be written down as a `@FR-` rule
   ([formal/layout.md](../../formal/layout.md) is its home); no rule states it today, which
   is worth something independently of this plan.
2. **Zone-1 slot reuse.**  A reused slot must have its tag cleared by whatever reuses it, and
   `assign_slots` — not the runtime — is the authority on when that happens
   ([SLOTS.md](../../SLOTS.md)).  Whether the clear belongs at the reuse site or at scope
   entry is a phase-1 call, and the matrix cell above is what decides it.
3. **Cost, and therefore where it can run.**  `LOFT_STRICT_STORES` is probe-only because
   never reusing a slot walks a long run off the `u16` store space; this shadow has no such
   ceiling, so the question is only instructions.  Phase 4's measurement decides whether
   phase 5 is a nightly sweep or a probe-only lever.

## See also

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
