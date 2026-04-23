<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 3 — Switch codegen to V2, delete V1

**Status:** open.  Blocked by Phase 2.

**Goal:** flip the default slot allocator from V1 to V2.  Delete
`place_orphaned_vars`, the zone 1 / zone 2 split, and every
size-or-shape branch from the Phase 0 catalogue.  Un-ignore the
P178 and P185 regression tests.

## Entry criteria (from Phase 2)

- `LOFT_SLOT_V2=validate cargo test --release` has been green
  continuously for at least one full sprint of unrelated commits
  (i.e., V2 has survived non-allocator IR changes without
  regressions).
- `02b-phase2-divergences.md` has zero open items.
- The walk-through tests in `slots_v2.rs` are part of the default
  `cargo test --release` run.

## Steps

### 3a — Swap the default (one commit)

- In `src/variables/mod.rs`, change the `assign_slots` call site
  to dispatch through V2 by default.
- Preserve V1 behind a new opt-in: `LOFT_SLOT_V1=1` falls back to
  V1 for a release cycle in case of surprise.  Document this in
  SLOTS.md during Phase 4.
- Run the full test suite.  Every test passes against V2 by
  default; `LOFT_SLOT_V1=1 cargo test --release` also passes (V1
  is still there, just not the default).
- **Ground rule check:** if any test fails against V2 now that
  wasn't caught during Phase 2, STOP.  A new divergence post-Phase-2
  means the equivalence harness missed a case.  Add the fixture
  to Phase 0's catalogue, re-run Phase 2 until green, then retry
  3a.

### 3b — Un-ignore the regression tests

- Remove `#[ignore]` from `tests/issues.rs::p178_is_capture_slot_alias`
  (if it's still gated — audit at 3b start).
- Remove `#[ignore]` from `tests/issues.rs::p185_slot_alias_on_late_local_in_nested_for`.
- Update `tests/ignored_tests.baseline` to reflect the drop.
- Remove `#[ignore]` from any other slot-related regression test
  that the Phase 0 audit turned up.

### 3c — Delete V1

One commit, no partial deletes.  Delete in this order (each step
is a local edit — no test runs between, since the previous step
leaves the tree broken):

1. Remove `src/variables/slots.rs` entirely.
2. Remove every call site of `place_orphaned_vars`.
3. Remove `Function::zone1_hwm`, `Function::var_size` (if unused
   after V2 takes over), and any other V1-only fields.
4. Remove the `LOFT_SLOT_V1=1` fallback and the associated
   dispatch wrapper.
5. Remove `validate_v2_report` (V2 IS the allocator now; the normal
   `validate_slots` path runs against it).
6. Rename `slots_v2.rs` → `slots.rs`, `assign_slots_v2` →
   `assign_slots`, update the module declaration in
   `src/variables/mod.rs`.

After the sequence: `cargo build` must compile without V1-related
errors.  If it doesn't, the delete list above is incomplete — add
the missing symbol to the list and retry.  Do NOT `#[allow(dead_code)]`
anything to make the build pass.

### 3d — Green gate

- `cargo fmt`
- `cargo clippy -- -D warnings`
- `cargo test --release`
- `cargo test --release --test native`
- `cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random`
- `cargo test --release --test html_wasm`

All green before commit lands.  Per CLAUDE.md's "plans never
allow regressions" rule: if any gate is red, 3c is reverted (not
patched) and Phase 2 is reopened.

## Non-goals for Phase 3

- No algorithmic changes beyond what SPEC.md specifies.  If a
  test goes red, the fix is in V2's implementation (or SPEC.md),
  not a tactical patch-on-top.
- No SLOTS.md / CAVEATS.md / PROBLEMS.md rewrites (Phase 4).
- No runtime opcode changes.  If Phase 1 picked the "function-
  entry `OpReserveFrame` only" option, the opcodes stay as-is —
  codegen just emits one per function instead of one per block.

## Ground rule — no regressions

Every commit in Phase 3 runs `cargo test --release` green:
- 3a: green against V2 by default.
- 3b: green with the previously-ignored regressions running.
- 3c: compile-only OK after each delete step (they land in one
  commit, so no intermediate test run), and the full suite green
  after the commit.

The commit message for 3c explicitly states the line count
reduction (per success criterion 3 in README.md) and any behaviour
differences V2 introduced (per the divergence log from Phase 2).

## Deliverables

1. `src/variables/slots.rs` is the V2 implementation (after
   rename), ≤ 800 lines, with no size/shape branching.
2. `place_orphaned_vars` is gone from the tree.
3. P178 and P185 regression tests pass without `#[ignore]`.
4. `tests/ignored_tests.baseline` is refreshed.
5. No `LOFT_SLOT_V1` env-var handling remains.

## Done when

- All six gates in 3d pass on the branch head.
- `git grep 'place_orphaned_vars'` returns zero matches.
- `git grep 'zone1_hwm\|zone2_hwm'` returns zero matches (or only
  doc references, which Phase 4 will clean up).
- `wc -l src/variables/slots.rs` ≤ 800.
- P178 and P185 are in the "Fixed" column of PROBLEMS.md (Phase
  4's doc sweep will update the entries themselves).

## Open questions to flag if they surface

- Does the native-codegen path (`src/generation/`) have any
  implicit dependency on V1's placement order?  Phase 2's
  equivalence gate covers the interpreter; native has separate
  tests (`cargo test --release --test native`).  If those go red
  at 3d, Phase 3 pauses until the native-specific behaviour is
  captured in a Phase 0 fixture and V2 passes it.
- Do any external consumers (e.g. the WASM build, packages) carry
  a slot-order assumption?  Phase 3d covers the built-in WASM
  bridge; third-party packages are out of scope but documented
  in CAVEATS.md during Phase 4 if found.
