
# Stack Slot Assignment — Design and Implementation

This document describes how `assign_slots` assigns stack positions to local
variables and the invariants codegen enforces.

---

## Overview

`assign_slots` (`src/variables/slots.rs`) runs after `compute_intervals` and
before codegen.  It assigns `stack_pos` to every local variable.  Codegen
(`src/state/codegen.rs::generate_set`) reads the pre-assigned position and
asserts it matches the runtime stack pointer (TOS).

**Key invariant:** `assign_slots` is the single authority for slot positions.
Codegen never moves variables.  If a variable's slot doesn't match TOS at
first assignment, that's a bug in `assign_slots` — not something codegen
should silently fix.

---

## Frame Layout

```text
┌──────────────────┐  ← frame base (args + return address)
│  zone 1: small   │  ≤8-byte types, greedy interval colouring
│                  │  pre-claimed via OpReserveFrame (Blocks only)
├──────────────────┤
│  zone 2: large   │  >8-byte types (text, refs, vectors)
│                  │  placed sequentially in IR-walk order
└──────────────────┘  ← TOS
```

**Blocks** use both zones.  `OpReserveFrame(var_size)` pre-claims zone 1
at block entry.  `generate_block` cleans up with `OpFreeStack` at exit.

**Loops** skip zone 1 entirely.  `var_size = 0`, no `OpReserveFrame`.
All loop variables (small and large) are placed sequentially via the
zone 2 IR-walk.  This is because:
- Loop variables persist across iterations (can't re-reserve each time)
- `OpFreeStack` after loop exit corrupts nested/parallel loop patterns
- Zone 1 slot reuse is pointless for loops (nothing dies mid-loop)

---

## Zone 1: Greedy Interval Colouring (Blocks only)

Small variables (≤ 8 bytes) in a Block scope are packed densely at
`frame_base`.  Dead variables' slots are reused within the same scope
if sizes match.  The `zone1_hwm` (high-water mark) determines the
`var_size` written into the Block node.

---

## Zone 2: Sequential IR-Walk Placement

Large variables (> 8 bytes) and ALL loop-scope variables are placed at
`*tos` in the order they appear during the IR tree walk.  `*tos` advances
by `v_size` after each placement.

### Special cases in the IR-walk

- **Block-return:** `Set(v, Block([..., result]))` — the block starts at
  `v`'s slot (non-Text refs share the frame).
- **Inner pre-assignments:** `Set(v, Insert([Set(__lift, ...), Call(...)]))` —
  the Insert's preamble Sets are processed first so `__lift` vars get
  lower slots than `v`.
- **Orphaned variables:** Variables whose scope has no Block/Loop node in
  the IR tree are placed by `place_orphaned_vars` after the main walk,
  using interval colouring against already-placed variables.

---

## Codegen Invariants

### `gen_set_first_at_tos` assertion

```rust
assert!(pos == stack.position, "slot={pos} but TOS={}", stack.position);
```

Every first assignment goes through `gen_set_first_at_tos` (if `pos >= TOS`)
or `set_var` (if `pos < TOS`).  The assertion catches any case where
`assign_slots` placed a variable above codegen's TOS.

### `set_stack_pos` assertion

```rust
debug_assert!(pre_assigned_pos == u16::MAX || pre_assigned_pos == pos || argument);
```

Codegen never moves a variable after `assign_slots` has placed it.

### `gen_loop` — no OpReserveFrame

Loops do not emit `OpReserveFrame`.  All loop variables are placed at TOS
by codegen on first encounter.  `clear_stack` at the end of each iteration
resets TOS to the loop start.

---

## Invariant validation (from plan-04)

`src/variables/validate.rs::validate_slots` runs at the end of every
codegen pass in debug / test builds (`src/state/codegen.rs:155-160`)
and checks the following invariants against the final variable
table.  All fire as panics with a `[I#]` prefix so the diagnostic
is searchable.

| ID | Check |
|---|---|
| **I1** | No slot overlap between variables with overlapping live intervals. |
| **I2** | Args live in the argument region; locals live above. |
| **I3** | (placeholder; no check today). |
| **I4** | Every variable with a first-def has a placed slot. |
| **I5** | Slot-kind consistency — no mixing of `Inline` and `RefSlot` on a shared slot (drop-opcode safety). |
| **I6** | Loop-iteration safety — no slot shared across loop-body boundary without a reset. |
| **I7** | **Scope-frame consistency** — each variable's `stack_pos` lies within its declared scope's frame region `[frame_base(scope), frame_base(scope) + var_size(scope))`.  Catches the "slot above TOS" runtime panic class at compile time. |

I7 is the new addition (2026-04-22 close-out of plan-04).  The
remaining invariants I1–I6 were authored during plan-04 Phase 2 as
correctness gates for V2; they continue to run against V1's output
unchanged.

**I8 — orphan-iterator-alias** (dep-chain-aware aliasing check
guarding P185's failure shape) is scoped but not yet built — see
`doc/claude/plans/05-orphan-placer-elimination/`.

## Plan-04 status

Plan-04 (`doc/claude/plans/04-slot-assignment-redesign/`) aimed to
replace this two-zone allocator with a single-pass, scope-blind
algorithm.  The retirement attempts (codegen-is-allocator and
V2-drive) both failed on variables declared at outer scope but
first-Set in inner scope — see the plan's README § Status for
details.  V1 remains the production allocator.  V2
(`src/variables/slots_v2.rs`) stays as a shadow validator invoked
via `LOFT_SLOT_V2=validate`.

---

## Diagnostic Tools

### `LOFT_ASSIGN_LOG=<name>`

Set to a function name (or `*` for all) to trace `assign_slots` placement
decisions.  Only active in debug builds (`#[cfg(debug_assertions)]`).

### `validate_slots` (debug only)

After codegen, scans all assigned variables for spatial+temporal overlaps.
Panics with variable names, slots, and live intervals if a conflict exists.

### `.slots()` test assertions

The `Test::slots(spec)` harness in `tests/testing.rs` captures the
assigned-slot layout for `n_test` after codegen and compares it against
a multi-line visual spec ("name(scope)+size=slot [first_def..last_use]"
with depth bars).  Calling `.slots("")` triggers a panic that prints
the calculated layout ready for copy-paste — the intended workflow for
adding a new fixture.  The check fires in every build profile.

Two fixture catalogues use this harness:

- `tests/strings.rs` — string-scope regressions (2 fixtures).
- `tests/slot_v2_baseline.rs` — the **Phase 0 fixture catalogue for the
  slot-assignment redesign** (see
  [`plans/04-slot-assignment-redesign/`](plans/04-slot-assignment-redesign/)).
  Every fixture locks one specific placement decision so V2 (single-pass
  allocator) can be validated against V1 before the Phase 3 switchover.

Unit tests in `src/variables/slots.rs` also verify slot assignments for
specific IR shapes without running codegen, by constructing synthetic
`Function` / `Value` structures directly.

---

## Phase 0 Fixture Catalogue (plan 04)

Every pattern documented below has an explicit fixture in
`tests/slot_v2_baseline.rs` that locks the exact slot layout
produced by today's two-zone allocator.  The V2 allocator (built in
Phase 2) must reproduce every layout before the switch in Phase 3.

| # | Pattern | Fixture | Status |
|---|---------|---------|--------|
|  1 | Zone-1 reuse (non-overlapping small ints share slot) | `zone1_reuse_two_ints_same_block` | ✅ |
|  2 | Loop-scope small vars placed sequentially | `loop_scope_small_vars_sequential` | ✅ |
|  3 | Text block-return vs child text | `text_block_return_vs_child_text` | ✅ |
|  4 | Insert preamble (P135 lift) ordering | `insert_preamble_lift_ordering` | ✅ |
|  5 | Sibling scope reuse (If-expression arms) | `sibling_scopes_share_frame_area` | ✅ |
|  6 | Sequential lifted calls (`body += pad(i)`) | `sequential_lifted_calls` | ✅ |
|  7 | Comprehension then literal (P122p) | `p122p_comprehension_then_literal` | ✅ |
|  8 | Sorted range comprehension (P122q) | `p122q_sorted_range_comprehension` | ✅ |
|  9 | Par loop with inner for (P122r) | `p122r_par_loop_with_inner_for` | ⚠ `#[ignore]` — codegen panic on `par()` outer iterator |
| 10 | Many parent refs + child loop index | `parent_refs_plus_child_loop_index` | ✅ |
| 11 | Call with Block arg (vector-comprehension in arg position) | `call_with_block_arg` | ✅ |
| 12 | Parent var Set inside child scope | `parent_var_set_inside_child_scope` | ✅ |
| 13 | P178 — `is`-capture in Insert-rooted body | `p178_is_capture_body` | ✅ |
| 14 | P185 — late local after inner text-accumulator loop | `p185_late_local_after_inner_loop` | ⚠ `#[ignore]` — V2 required to pick non-aliasing slot |
| 15 | Local after args-heavy signature (args-region isolation) | `fn_with_only_arguments` | ✅ |
| 16 | Nested If with Block branches | `nested_if_block_branches` | ✅ |
| 17 | Large vector followed by small int (zone-1/2 mixing) | `large_vector_then_small_int` | ✅ |
| 18 | Two sibling Blocks with shared outer var | `two_sibling_blocks_shared_outer` | ✅ |
| 19 | For-loop with two loop-scope locals | `for_loop_two_loop_locals` | ✅ |
| 20 | Nested for-in-for (two loop scopes) | `nested_for_in_for` | ✅ |
| 21 | Match with per-arm bindings | `match_with_arm_bindings` | ✅ |
| 22 | Vector block-return (non-Text frame-sharing) | `vector_block_return_non_text` | ✅ |
| 23 | Nested call chain `f(g(h(x)))` | `nested_call_chain` | ✅ |
| 24 | Vector accumulator loop (`acc += [...]`) | `vector_accumulator_loop` | ✅ |
| 25 | Early return from nested scope | `early_return_from_nested_scope` | ✅ |
| 26 | Method-mutation extends var lifetime | `method_mutation_extends_lifetime` | ✅ |

**Legend:** ✅ layout locked and passing; ⚠ fixture present but
`#[ignore]`-d pending an orthogonal fix (see the fixture's doc comment
for the specific blocker).


---

## Scope shapes and orphan placement

`place_orphaned_vars` (`src/variables/slots.rs:58`) is the post-walk
catch-net for variables the `process_scope` / `place_large_and_recurse`
IR-walk never visits.  The Phase 0 audit
([`plans/04-slot-assignment-redesign/00a-audit.md`](plans/04-slot-assignment-redesign/00a-audit.md))
identifies three structural triggers for orphan status; each is covered
by a fixture in `tests/slot_v2_baseline.rs`:

| Scope-shape trigger | Why the main walk misses it | Fixture |
|---------------------|-----------------------------|---------|
| Function body root is `Value::Insert` (not `Block`/`Loop`) | `process_scope` returns early when `block_val` isn't Block/Loop; every local becomes an orphan | `p178_is_capture_body`, `insert_preamble_lift_ordering` |
| Parent-scope `Set` inside a child Block's `operators` | The child-scope walk never sees variables from the parent's scope number; they remain unplaced until the orphan pass collects them | `parent_var_set_inside_child_scope` |
| Insert preamble (`Value::Insert([Set(__lift_N, ...), ...])`) wrapping a Call or format-string | Lift vars have the enclosing function's scope, not the Insert's — the IR walk doesn't descend into the preamble as a named-scope child | `insert_preamble_lift_ordering`, `sequential_lifted_calls` |

`place_orphaned_vars` takes `local_start` as the floor for its
candidate-slot search (fix for P178) so orphans cannot overlap the
argument + return-address region at `[0, local_start)`.  Arguments have
`stack_pos == u16::MAX` during `assign_slots`; without the `local_start`
floor, the per-variable conflict check at `slots.rs:382–389` could not
see them and would happily assign slot 0.

The Phase 0 audit also catalogues 20 dispatch points in today's
allocator (5 on variable size, 11 on scope kind, 1 on Text type, 3 on
set cardinality) that V2's single-pass algorithm must subsume into a
uniform formula — see `00a-audit.md` for the per-line mapping.

---

## Files

| File | Role |
|------|------|
| `src/variables/slots.rs` | `assign_slots`, `process_scope`, `place_large_and_recurse`, `place_orphaned_vars` |
| `src/state/codegen.rs` | `generate_set`, `gen_set_first_at_tos`, `gen_loop`, `generate_block` |
| `src/variables/mod.rs` | `set_stack_pos` assertion, `Function` struct |
| `src/scopes.rs` | `scan_set` (Insert flattening), `inline_struct_return` (P122 lift) |
| `src/stack.rs` | `size_code` (eval stack size), `loop_position` |
