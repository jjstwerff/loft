
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

## Diagnostic Tools

### `LOFT_ASSIGN_LOG=<name>`

Set to a function name (or `*` for all) to trace `assign_slots` placement
decisions.  Only active in debug builds (`#[cfg(debug_assertions)]`).

### `validate_slots` (debug only)

After codegen, scans all assigned variables for spatial+temporal overlaps.
Panics with variable names, slots, and live intervals if a conflict exists.

### `.slots()` test assertions

Unit tests in `slots.rs` verify slot assignments for specific patterns
without running codegen.

---

## Known Patterns and Tests

| Pattern | Test | Status |
|---------|------|--------|
| Many parent refs + child loop index | `parent_zone2_does_not_overlap_child_zone1` | ✅ |
| Call with Block arg (coroutine) | `call_with_block_arg_places_block_vars_first` | ✅ |
| Insert preamble (P135 lift) | `insert_preamble_sets_placed_before_target` | ✅ |
| Sequential lifted calls | `sequential_lifted_calls_slots_match_codegen_tos` | ✅ |
| Parent var Set inside child scope | `parent_var_set_inside_child_scope_operators` | ✅ |
| Text block-return | `text_block_return_no_overlap_with_child_text` | ✅ |
| Sibling scope reuse | `sibling_scopes_share_frame_area` | ✅ |
| Comprehension then literal | `p122p_vector_comprehension_slot_gap` | ✅ |
| Sorted range comprehension | `p122q_comprehension_zone1_zone2_ordering` | ✅ |
| Par loop with inner for | `p122r_par_loop_with_inner_for` | ✅ |

---

## Files

| File | Role |
|------|------|
| `src/variables/slots.rs` | `assign_slots`, `process_scope`, `place_large_and_recurse`, `place_orphaned_vars` |
| `src/state/codegen.rs` | `generate_set`, `gen_set_first_at_tos`, `gen_loop`, `generate_block` |
| `src/variables/mod.rs` | `set_stack_pos` assertion, `Function` struct |
| `src/scopes.rs` | `scan_set` (Insert flattening), `inline_struct_return` (P122 lift) |
| `src/stack.rs` | `size_code` (eval stack size), `loop_position` |
