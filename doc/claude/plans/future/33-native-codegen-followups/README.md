<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Native codegen — open follow-ups

`--native` is the production execution mode (CI-gated, all
108/108 native tests pass).  Architecture, history, and the
N1-N8 phase content live in
[`../../../NATIVE.md`](../../../NATIVE.md) as reference
documentation.

This plan tracks the **open follow-up items** still to be
built.  Each item points at the existing design content in
NATIVE.md; the plan exists so the open work is visible in
the `plans/future/` index and can be picked up cleanly.

## Status

| Item | NATIVE.md section | Status |
|---|---|---|
| **N8b.3** — `yield from` delegation | [`../../../NATIVE.md` § N8b.3](../../../NATIVE.md) | Open — design drafted, not implemented.  Native coroutines support `yield value` (N8b.1 + N8b.2 shipped) but NOT `yield from <inner_iterator>` delegation.  Marked `CO1.3d` in NATIVE.md line 944. |
| **N8c.1** — Audit generic text-return | [`../../../NATIVE.md` § N8c.1](../../../NATIVE.md) | **Probably overlaps shipped work.**  The N8c.1 audit predicts text-return wrapping issues in monomorphized functions like `t_4text_identity`.  The plan-17 closure landed P237 / P238 / P242 fixes addressing exactly this shape (`Value::Tuple` recursion in `substitute_type_in_value`; `tuple_text_to_string` flag handling).  **Action:** un-skip `tests/scripts/48-generics.loft` and re-run; if green, mark N8c.1 closed. |
| **N8c.2** — Fix generic text-return | [`../../../NATIVE.md` § N8c.2](../../../NATIVE.md) | Same overlap.  N8c.1 audit determines whether N8c.2 is needed at all. |
| **N20a** — Add `ops` import to generated `fill.rs` | [`../../../NATIVE.md` § N20a](../../../NATIVE.md) | Open — trivial single-line add in `src/create.rs::generate_code()`. |
| **N20b** — Run `cargo fmt` on generated `fill.rs` | [`../../../NATIVE.md` § N20b](../../../NATIVE.md) | Open — small, runs `rustfmt` on the generated file so formatting matches the hand-maintained version. |
| **N10** intro paragraph in NATIVE.md | [`../../../NATIVE.md` § N10](../../../NATIVE.md) | **Stale.**  Says "6 fail, 34 skip of 85 files"; current state shows 108/108 native tests pass.  N10 sub-steps (N10a etc.) are diagnostic recipes for failures that no longer exist.  **Action:** when this plan ships, prune the N10 intro paragraph + sub-steps in NATIVE.md and replace with a "see git history" note. |

## Why these items are here, not in NATIVE.md

NATIVE.md is reference documentation — it describes how the
native codegen pipeline works (architecture, `codegen_runtime`,
per-Op dispatch, the N1-N8 history of how it got built).
Anyone reading or modifying the codegen subsystem reads
NATIVE.md.

The open follow-ups don't fit that purpose: they're items
to BUILD, not architecture to understand.  Per the plans
direction (`major dev → plans path; bugs → direct`), they
belong in `plans/future/`.  Keeping them visible in the
`plans/future/` index ensures they don't get lost as
NATIVE.md grows or gets re-organized.

The pointer-plan shape (this README references NATIVE.md
sections rather than copying their content) avoids
duplication — design details stay in one place.  When an
item ships, the work in NATIVE.md gets trimmed and this
plan's row moves to a closure record.

## Phase ordering

Suggested sequence when this plan unpauses:

1. **N8c.1 audit** — fastest item.  Un-skip
   `tests/scripts/48-generics.loft` and re-run.  Likely
   green after plan-17 closures.  If green: close N8c.1 +
   N8c.2 in one commit.
2. **N20a + N20b** — trivial pair.  One commit each, or
   bundle.
3. **N8b.3 yield from** — actual feature work.  Needs the
   state-machine lowering extension to handle delegation;
   touches `src/generation/coroutine.rs` (or wherever the
   N8b.1 transform lives).
4. **NATIVE.md cleanup** — once N8b.3 + N8c.x + N20 close,
   prune NATIVE.md's N10 + N20 sections (they become
   historical).

## See also

- [`../../../NATIVE.md`](../../../NATIVE.md) — full native
  codegen architecture + N1-N8 history (reference)
- [`../../../INTERMEDIATE.md`](../../../INTERMEDIATE.md) —
  IR Value tree structure
- [`../../../INTERNALS.md`](../../../INTERNALS.md) §
  Native — runtime support library for native binaries
- [`../../../COROUTINE.md`](../../../COROUTINE.md) — coroutine
  semantics; N8b.3 implements the `yield from` half of the
  shipped coroutine spec
- [`../17-template-validation/`](../../finished/17-template-validation/)
  (in finished/) — closed plan-17 work that may already
  cover N8c.x
