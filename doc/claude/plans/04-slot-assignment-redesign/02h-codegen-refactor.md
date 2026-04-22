<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 2h — Codegen is the allocator (V1/V2 retirement)

## Status — 2026-04-22 retraction

**This design is retracted.**  V1 remains the production allocator.
See [`README.md`](README.md) § Status for the overall retraction
context.

**Why it fails:** the "codegen already tracks TOS" premise is true,
but not sufficient.  `scopes::scan_if`'s `small_both` pre-registration
(and the `pre_inits` dance for Reference / Vector / Text types)
registers some variables at their **outer** declared scope even
though their first `Set` appears deep inside a nested branch — the
canonical shape is a match arm's pattern binding, e.g. `_mv_width_3`
in `tests/issues.rs::p162_return_match_struct_enum_native`, declared
at body scope (1) but first-Set inside match_arm(4).  Under
codegen-is-allocator, the first-Set claims `slot = stack.position`
at the inner TOS (say 24); when a sibling match_arm(6) later
re-Sets or reads the same variable at body-TOS 16, the slot is
above TOS → `generate_var` panics.

V1's **zone-1 pre-pass** (`slots.rs:82-172`) is the load-bearing
piece: before descending into any child scope it collects every
variable whose `scope == current_block.scope` and greedy-colours
their slots at the parent's frame_base.  Codegen alone cannot do
this without a scope pre-pass (which means it IS an allocator, just
inside codegen.rs) and V2's original IR-walk design has the same
blind spot — tried via a V2-drive attempt in the same session, same
failure mode, same fixture.

**Retraction delta to the tree:**
- `scopes::check` reverted to call V1 `assign_slots` (as before the
  2h.3 "skip the pre-allocator" change).
- Codegen's `gen_set_first_at_tos` restored to the committed form
  that respects pre-assigned slots.
- `OpInitText` / `OpInitRef` added in 2h.1 are **kept** as clean
  positional primitives with no active callers; they're cheap to
  maintain and may prove useful in future work (register VM, native
  frame emission).
- V2 (`src/variables/slots_v2.rs`) stays as a shadow validator
  (`LOFT_SLOT_V2=validate`).

**What would be needed to revisit codegen-is-allocator:** either
change `scopes::scan_if` to register `small_both` at each branch's
scope (investigation showed this is load-bearing for
`get_free_vars`' LIFO `OpFreeRef` emission — double-free risk), or
add a scope-tree pre-walk inside codegen before the main emission
walk (which just moves the allocator from `slots.rs` to
`codegen.rs` — net complexity unchanged).

See [`doc/claude/plans/05-orphan-placer-elimination/`](../05-orphan-placer-elimination/README.md)
for the targeted follow-up that actually un-ignores P185.

---

## Original design (archived)

**Status (as written):** design locked; 2h.1 landed; 2h.2 onwards pending.
Blocks: Phase 4 (cleanup / SLOTS.md rewrite / initiative move to
`plans/finished/`).

## Key realisation

V1 and V2 both run an **allocator pass** that walks the IR and
assigns `stack_pos` to each variable.  Codegen then walks the IR
**again**, tracking its own `stack.position` counter, and **hopes
the two walks agree**.  When they don't agree, codegen's fixup at
`gen_set_first_at_tos:1025` silently rewrites `stack_pos` — which
is the layering violation this plan is trying to remove.

**The two walks are redundant.**  Codegen already tracks TOS exactly.
At each `Set(v, _)` first-assignment, `stack.position` is, by
definition, the slot where `v` must live.  If codegen just *writes*
`function.set_stack_pos(v, stack.position)` inline at that point,
there is no pre-pass to agree with — codegen **is** the allocator.

This realisation shrinks Phase 2h from "refactor codegen so it
consumes V2's plan" to "retire the allocator pass entirely; codegen
owns placement."

## Scope

**In scope.**
- **Delete `src/variables/slots.rs` (V1)** entirely — ~1,676 lines,
  including `assign_slots`, `process_scope`, `place_orphaned_vars`,
  all zone-1 / zone-2 machinery, `find_reusable_zone2_slot`, and
  the scope-specific test module.
- **Delete `src/variables/slots_v2.rs` (V2)** entirely — the
  IR-walk allocator and its scaffolding.  The positional init
  opcodes (`OpInitText`, `OpInitRef`) added in 2h.1 stay; they are
  a clean primitive the runtime keeps.
- **Delete `scopes::check`'s call to `assign_slots`** and the
  `LOFT_SLOT_V2` env-var dispatch (validate / report / drive
  modes).  The pre-allocation pass is gone; slots appear during
  codegen's walk.
- **Promote codegen's fixup to the primary path.**  The existing
  logic at `gen_set_first_at_tos:1025`:
  ```rust
  if pos < stack.position {
      stack.function.set_stack_pos(v, stack.position);
  }
  ```
  becomes unconditional.  Rename to `ensure_slot_at_tos` — or
  fold the slot assignment into the caller.  The
  `pos == stack.position` assertion below is retained as
  defence-in-depth but should never fire after this change.
- **Delete the `if pos > stack.position { OpReserveFrame(gap) }`
  branch** (`codegen.rs:1036-1041`) — under codegen-driven
  placement, `pos` never exceeds `stack.position` because `pos`
  IS `stack.position` when set.
- **Simplify `validate_slots`.**  I5 (`SlotKind` consistency on
  shared slots), I6 (loop-iteration-safety), and the `SlotKind`
  enum become structurally vacuous: codegen-driven slot
  assignment has no within-scope slot reuse.  I1, I2, I4 remain
  as defence-in-depth.
- **Un-ignore `p185_slot_alias_on_late_local_in_nested_for`.**
  The orphan-placer bug class can't recur: codegen visits every
  `Set` in IR order and assigns each slot at emission time.  No
  post-pass catches "missed" variables — they were never missed
  because there was never a separate pass.
- **Codegen retains per-block `OpReserveFrame` / `OpFreeStack`.**
  They already work and there's no reason to churn them in this
  phase.  Function-entry-only reservation (the original 2h.3
  goal) is a *separate* optimisation, out of scope here.
- Documentation updates: SPEC.md, SLOTS.md, PROBLEMS.md, README.

**Out of scope.**
- VM architecture change (register-based) — Phase-1.0 territory.
- Function-entry-only `OpReserveFrame(hwm)` — optimisation for a
  later phase; keeps per-block reserves for now.
- Rewiring the 9 existing `OpText` / `OpConvRefFromNull` call
  sites to use the new positional variants (`OpInitText`,
  `OpInitRef`).  The positional forms exist as a primitive; their
  adoption in codegen is a later, separate commit when a concrete
  need arises.

## Why this is simpler than the prior design

The prior Phase 2h plan (before this rewrite) enumerated 9
callsites to rewire, added function-entry `OpReserveFrame`, and
carried a rollback plan for partial completion.  Under the
codegen-is-the-allocator model:

- No callsite rewiring required.  Codegen's existing TOS tracking
  suffices.
- No new runtime contract.  `OpReserveFrame` / `OpFreeStack`
  remain per-block, exactly as today.
- No SPEC § 2 algorithm to specify — the "algorithm" is whatever
  codegen's IR walk already does.
- No coordination between allocator and codegen — they're one.

The effort becomes: **delete, delete, delete.**  Net LOC
reduction is large (~2,000 lines).

## Implementation order

Each step lands as a separate commit with `cargo test --release`
green at every checkpoint.

### 2h.0 — Restore the fixup as the primary path  ✅ DONE (this session)

Already done: the fixup was restored during Phase 2h design work.
The tree is green with V1 driving codegen via the fixup safety
net.  Step 2h.3 below promotes the fixup to the explicit primary
path.

### 2h.1 — Add positional init opcodes  ✅ DONE

- `default/01_code.loft`: `fn OpInitText(pos: const u16);` and
  `fn OpInitRef(pos: const u16);` declared.
- `src/state/text.rs::init_text` and `src/state/mod.rs::init_ref`
  added.
- `src/fill.rs` regenerated (OPERATORS grown 236 → 238).
- Gate: all four test suites green.

These ops stand as a clean init primitive regardless of whether
codegen adopts them immediately.  A future commit may rewire
callsites; not required by this phase.

### 2h.2 — Make codegen assignments explicit  ✅ DONE

Replaced the dual-branch fixup at `codegen.rs:1021-1050` with:

```rust
fn gen_set_first_at_tos(&mut self, stack: &mut Stack, v: u16, value: &Value) {
    let pre_pos = stack.function.stack(v);
    // Gap-fill preserved: when the (still-live) V1 allocator has
    // placed a var ABOVE current TOS (PROBLEMS.md #139), advance
    // TOS so the slot is reachable.
    if pre_pos != u16::MAX && pre_pos > stack.position {
        let gap = pre_pos - stack.position;
        stack.add_op("OpReserveFrame", self);
        self.code_add(gap);
        stack.position += gap;
    }
    // Codegen claims the slot at TOS, unconditionally.  Whatever
    // the allocator planned gets overwritten — codegen is
    // authoritative.
    stack.function.set_stack_pos(v, stack.position);
    // ...
}
```

The `pos > stack.position` branch stays while V1 is still the
allocator (step 2h.3 stops running it).  After V1 is deleted, the
branch becomes dead and is removed in 2h.4.

**Gate status:**
- `cargo test --test issues`: 500 passed.
- `cargo test --test slot_v2_baseline`: 27 passed.
- `cargo test --lib`: 153 passed.
- `cargo test --test wrap loft_suite`: passed.

One detail we learned: attempting to delete the gap-fill branch
prematurely (before V1 retires) broke 7 tests including p120 —
V1's pre-claim places some vars above TOS and expects codegen to
advance via `OpReserveFrame`.  The branch stays until V1 stops
placing vars.

### 2h.3 — Skip the pre-allocator pass

In `src/scopes.rs::check`, stop calling `assign_slots`.  The
variables' `stack_pos` starts at `u16::MAX` and is assigned during
codegen's walk (step 2h.2 made this the contract).

Also drop the `LOFT_SLOT_V2` env-var dispatch (validate / report
/ drive modes) — the shadow path becomes meaningless when there
is no second allocator to compare against.

**Done when:** `cargo test --release` green with `scopes::check`
not calling `assign_slots`; `LOFT_SLOT_V2` env var has no effect.

### 2h.4 — Delete V1 (`src/variables/slots.rs`)

Single commit:
- Delete `src/variables/slots.rs` and its ~1,676 lines of
  two-zone / orphan-placer logic.
- Delete `crate::variables::assign_slots` re-export from
  `src/variables/mod.rs`.
- Delete any `#[allow(dead_code)]` annotations on V1-only helpers
  (`find_reusable_zone2_slot`, `place_zone2`, `place_orphaned_vars`,
  `inner_has_pre_assignments`, `is_loop_scope`'s only caller, etc.).
- Update `doc/claude/SLOTS.md`'s "Files" table to drop the
  `src/variables/slots.rs` row.

**Done when:** `cargo build` passes; `git grep place_orphaned_vars`
returns zero matches; `git grep zone1_hwm` returns zero matches.

### 2h.5 — Delete V2 (`src/variables/slots_v2.rs`)

Single commit:
- Delete `src/variables/slots_v2.rs`.
- Delete `crate::variables::{assign_slots_v2, apply_v2_result,
  LocalInterval, SlotKind, SlotAssignment, AllocatorResult,
  v2_validate_enabled, v2_report_enabled}` re-exports from
  `src/variables/mod.rs`.
- Delete `Function::reset_local_slots` — codegen doesn't need it
  anymore.

**Done when:** `cargo build` passes; `git grep assign_slots_v2`
and `git grep SlotKind` return zero matches outside docs.

### 2h.6 — Simplify `validate_slots`

Drop the now-vacuous checks:
- Delete `SlotKind` / `slot_kind()`.
- Delete `check_i5_kind_consistency`.
- Delete `check_i6_loop_iteration`.
- Delete corresponding unit tests in `invariant_tests`.
- `validate_slots` retains I1 (overlap), I2 (arg isolation),
  I3 (no-op; not currently checked), I4 (every var placed).
  Rename the diagnostic comments so they no longer reference
  "plan-04 I1–I6" verbatim.

**Done when:** `cargo test --lib` green; the trimmed validator
still catches overlap on hand-crafted bad Functions in
`invariant_tests`.

### 2h.7 — Un-ignore P185

- Remove `#[ignore]` from `tests/issues.rs::p185_slot_alias_on_late_local_in_nested_for`.
- Remove `#[ignore]` from `tests/slot_v2_baseline.rs::p185_late_local_after_inner_loop`.
- Update `tests/ignored_tests.baseline` if it tracks either.
- Run the affected tests — they should pass without modification,
  because codegen assigns `key`'s slot at its first-assignment
  in IR order (the orphan-placer bug can't recur).

**Done when:** both P185 tests pass un-ignored; P185 gets marked
Fixed in PROBLEMS.md during Phase 4's doc sweep.

### 2h.8 — Documentation sweep

Minimal updates in Phase 2h itself (the full Phase 4 cleanup
touches SLOTS.md and README more thoroughly):

- SPEC.md: add a short "Phase 2h retired V2 in favour of
  codegen-driven placement" note at the top; the body remains as
  historical record of the V2 design we considered.
- SPEC_GAPS.md: close the remaining items.
- walkthroughs.md: add a footer noting V2 was retired; the
  walk-throughs remain a useful archive of the patterns the old
  design would have handled.
- plan-04 README: move status to "Phase 2h complete; Phase 4
  (cleanup) ready."

Phase 4's deliverables (SLOTS.md rewrite, PROBLEMS.md P178/P185
Fixed column, `slot_allocator_has_no_size_or_shape_branches`
lint, initiative move to `plans/finished/`) remain as Phase 4's
job.

## Affected files inventory

| File | Change | Step |
|---|---|---|
| `src/variables/slots.rs` | Delete entirely | 2h.4 |
| `src/variables/slots_v2.rs` | Delete entirely | 2h.5 |
| `src/variables/mod.rs` | Drop allocator re-exports; trim `reset_local_slots` | 2h.4, 2h.5 |
| `src/variables/validate.rs` | Drop I5, I6, `SlotKind` | 2h.6 |
| `src/state/codegen.rs::gen_set_first_at_tos` | Fixup becomes primary `set_stack_pos` | 2h.2 |
| `src/state/codegen.rs` — `if pos > stack.position` branch | Delete | 2h.2 |
| `src/scopes.rs::check` | Stop calling `assign_slots`; drop V2 env-var dispatch | 2h.3 |
| `tests/issues.rs::p185_*` | Un-ignore | 2h.7 |
| `tests/slot_v2_baseline.rs::p185_*` | Un-ignore | 2h.7 |
| SPEC.md, SPEC_GAPS.md, walkthroughs.md, README | Sweep | 2h.8 |

## Rollback plan

Each of 2h.2 – 2h.7 lands as its own commit, each with
`cargo test --release` green.  If any step fires an unexpected
regression:

- Revert the offending commit.
- Diagnose; adjust the step's scope.
- Re-land when green.

2h.1 (done) is backward-compatible (positional opcodes added
without any callsite using them).  2h.2 – 2h.7 are destructive
deletes, but each is independently testable — no single step
leaves the tree in a half-broken state that subsequent steps
depend on.

## Ground rule — no regressions

From [`plans/README.md`](../README.md): every commit runs
`cargo test --release` green.  Additionally:

- `cargo test --lib` green (unit tests in `validate.rs` still
  cover the retained I1/I2/I4 checks).
- `cargo test --test wrap loft_suite` green (slot-assignment
  patterns in `tests/scripts/96-slot-assign.loft` still pass).
- `cargo test --test slot_v2_baseline` green (the 27 passing
  fixtures + 2 ignored; P185 un-ignore in 2h.7 flips it to 28+1).

## Metrics (expected)

- **LOC delta: ~−2,000 net.**  V1 (~1,676 lines) + V2 (~250
  lines) + related infrastructure (~75 lines) all go.  The
  codegen addition is tiny (~5 lines).
- **Opcode count: unchanged at 238.**  The positional init ops
  (added in 2h.1) stay as a primitive.  Old `OpText` /
  `OpConvRefFromNull` also stay — they remain the canonical
  first-assignment ops in codegen until someone decides to
  rewire.
- **Test count: +2.**  P185 un-ignored in two places.
- **Invariant set: 3 (I1, I2, I4).**  Down from 6; the deletions
  reflect genuine redundancy rather than coverage loss.

## Success criteria

- [ ] `src/variables/slots.rs` deleted.
- [ ] `src/variables/slots_v2.rs` deleted.
- [ ] `git grep 'place_orphaned_vars\|zone1_hwm\|SlotKind'` returns zero hits.
- [ ] `git grep 'LOFT_SLOT_V2'` returns zero hits outside docs.
- [ ] P185 regression tests (both the `tests/issues.rs` and
      `tests/slot_v2_baseline.rs` variants) pass un-ignored.
- [ ] `cargo test --release`, `cargo test --lib`, and
      `cargo test --test wrap loft_suite` green.

## Open questions flagged for implementation

- **Does codegen's fixup-promotion (step 2h.2) introduce any
  infinite loop or stack-corruption edge case?**  The fixup was
  conditional today; promoting it to unconditional should be a
  strict simplification.  Verify with a trace on a known-
  complex fixture (e.g. the P122 family or nested-match).
- **Do any tests rely on `LOFT_SLOT_V2` env vars?**  A quick
  `grep -r LOFT_SLOT_V2 tests/` before 2h.3 confirms.  The
  variable is plumbed only through `v2_validate_enabled` and
  `v2_report_enabled`; no integration tests should read it.
- **Does `native` codegen (`src/generation/`) depend on V1's
  allocator output?**  Earlier audit (Gap 2) said no — it reads
  `stack_pos` opaquely after codegen is done.  After 2h.2–2h.5,
  `stack_pos` is still set (by codegen), just not by a
  pre-pass.  Native codegen should be unaffected.

## Why the positional opcodes stay

Two reasons to keep `OpInitText(pos)` and `OpInitRef(pos)` after
2h.1, even if codegen doesn't emit them:

1. **Cleaner primitive.**  The TOS-only forms conflate "allocate a
   stack slot's worth of bytes + initialise them" with "push
   onto the eval stack."  The positional forms separate these;
   future codegen changes (or future VMs / native bridges) have
   a direct way to initialise a frame slot without an
   accompanying TOS advance.
2. **Optional future adoption.**  If a later phase adopts
   function-entry `OpReserveFrame(hwm)` or moves to a register-
   based VM, the positional primitives are already in place.

Removing them just to tidy 2h.1 is churn for no benefit.
