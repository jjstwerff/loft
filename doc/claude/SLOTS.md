---
render_with_liquid: false
---
# Stack Slot Assignment — Design and Implementation

This document is the working reference for the `assign_slots` redesign.
History of earlier attempts is in ASSIGNMENT.md and SLOT_FAILURES.md.

---

## Contents

- [Background](#background)
- [Current state](#current-state)
- [Diagnostic tools](#diagnostic-tools)
- [The TOS-estimate problem](#the-tos-estimate-problem)
- [New design — two-zone block pre-claim](#new-design--two-zone-block-pre-claim)
- [Implementation details](#implementation-details)
- [Open issues](#open-issues)
- [Stack efficiency comparison](#stack-efficiency-comparison)
- [Remaining steps](#remaining-steps)

---

## Background

`assign_slots` (`src/variables/`) runs after `compute_intervals` and before codegen.
It assigns `stack_pos` to every local variable using greedy interval colouring:
variables with non-overlapping live intervals may share a slot; large types (Text 24 B,
Reference 12 B, Vector 12 B) always get a fresh slot because their init opcodes write
at the current TOS.

Codegen (`src/state/codegen.rs::generate_set`) then reads the pre-assigned `stack_pos`.
Two override cases exist:

- **`pos > stack.position`** — pre-assigned slot above current TOS.  Overrides to TOS for
  any type.
- **`pos < stack.position` + large type** — pre-assigned slot below TOS for a large type.
  Overrides to TOS so the init opcode writes at the correct position.

When `assign_slots` gives a variable the wrong slot, codegen overrides it, potentially
landing it on top of another still-live variable.  `validate_slots` (debug-only) detects
this and panics.

The `pre_assigned_pos` field captures the `assign_slots` value before any override,
making overrides visible in diagnostics.

---

## Current State

### Passing

```
cargo test -p loft --test strings    # 14/14
cargo test -p loft --test enums      #  6/6
cargo test -p loft --test vectors    # 45/45
```

### Reproduction tests (`tests/slots.rs`)

| Test | Bug class | Status |
|------|-----------|--------|
| `text_below_tos_nested_loops` | B-dir | ✅ passes (ignore removed) |
| `vector_iteration_index_inside_vec_slot` | B-stress | ✅ passes (ignore removed) |
| `sequential_file_blocks_read_conflict` | B-binary | ✅ passes (ignore removed) |

### Two-zone design — implementation status

All steps complete.  See [Implementation Status](#implementation-status) for detail.

---

## Diagnostic Tools

### `pre_assigned_pos` on `Variable`

Set by `assign_slots`, never touched by codegen.  Shown as `pre:[lo, hi)` in
`validate_slots` output when it differs from the final slot.

### Seq ranges in scope column

Loop scopes show `seq:[start..end)` to make it clear when their `OpFreeStack` fires
relative to any given `first_def`.

### `.slots()` assertions

`[first_def..last_use]` shown per variable, `[seq S..E]` on loop scope headers.
Pass an empty string to print the calculated layout without asserting.

### Loop/recursion guards

Three `assert!` guards prevent the slot-assignment pass from hanging indefinitely:

- **`process_scope` / `place_large_and_recurse` depth** — `depth` parameter, asserts
  `depth <= 1000`.  Catches infinite IR tree recursion (would indicate a malformed IR).

- **Greedy coloring retry loop** — `retry_count`, asserts `retry_count <= 10_000`.
  Panics with the variable name, size, scope and current candidate if the placement
  loop cannot find a free slot.

- **`is_scope_ancestor` step counter** — asserts `steps <= 10_000` and also has an
  explicit self-loop check (`p == cur`).  Fires if `build_scope_parents` produced a
  cycle in the scope parent map.

---

## The TOS-Estimate Problem

`assign_slots` must know the physical TOS at each variable's `first_def`, so it can
place large types exactly at TOS (required by their init opcodes).

### Three bug classes, one root cause

All three failing tests share the same root: the TOS at a variable's `first_def` cannot
be derived accurately from variable intervals alone, because it depends on exactly when
`generate_block` emits `OpFreeStack` — which is determined by the full recursive
structure of the IR tree, not by any per-variable property.

- **Bug A (dir/last):** A non-loop block scope's estimated exit fires too early because
  it is computed from `max(last_use of direct vars)`, which ignores variables in nested
  child scopes.  Text variable `f` gets a slot below actual TOS.
  **→ Fixed by two-zone design.**

- **Bug B (binary/loft_suite):** `running_tos` stays too high because a dead-but-never-
  freed variable (scope 2 spans the whole function) keeps it elevated.  A ref variable
  is pre-assigned above actual TOS; codegen overrides it downward onto a conflicting slot.
  **→ Partially fixed; residual conflict remains (see Open Issues).**

- **Bug C (stress):** A For-block scope is nested inside an outer loop body in the IR
  even though it appears after the outer loop in source.  `scope_exit` misfires; `sv`
  (vector) gets a slot 4 bytes below actual TOS; cascade conflict with `x#index`.
  **→ Fixed by two-zone design.**

### Why `running_tos` cannot be fixed incrementally

`running_tos` is an attempt to predict when `OpFreeStack` fires by maintaining a
monotonically-updated estimate of TOS.  Every bug fix adds another special case
(scope_exit map, inside_active_loop guard, ...) to compensate for structure that is
only visible in the IR tree.  The model will always be one IR pattern away from the
next bug.  The correct approach reads the IR tree directly.

---

## New Design — Two-Zone Block Pre-Claim

### Core idea

At block entry in codegen, claim all space for **small variables** (≤ 8 B primitives)
upfront via a single `OpReserveFrame` opcode.  Large variables (Text 24 B, Reference
12 B, Vector 12 B) remain placed at TOS in initialization order — but their TOS is now
**exactly known** because the small-variable frame is already accounted for and nested
block TOS movements are modelled directly from the IR tree.

The result: every large variable's pre-assigned `stack_pos` equals `stack.position`
at the exact moment `generate_set` is called for its first assignment.  The two override
cases in `generate_set` become unreachable.

### Why separate small from large

Small primitives (int, bool, long, float, fn-ref; ≤ 8 B) can be written to any stack
position via `OpPutX`.  They can be pre-claimed in bulk and written later.

Large types (Text 24 B, Reference 12 B, Vector 12 B) **must** be initialized at the
current TOS: `OpText`, `OpConvRefFromNull`, `OpCreateStack` all write at
`stack.position`.  Pre-claiming their space before initialization would leave TOS above
the slot, and the init opcode would write to the wrong address.

### Variable frame layout within a block

For scope S with frame base `B`:

```
B + 0                            B + zone1_size      B + zone1_size + zone2_size
│← zone 1: small primitives →│← zone 2: large types, in first_def order →│
│  pre-claimed at block entry  │  placed sequentially as they are initialized │
```

- **Zone 1** (`var_size` bytes): all variables with `size ≤ 8`.  Greedy interval
  colouring within `[B, B + zone1_size)`.  Positions are final at `assign_slots` time.
- **Zone 2**: all variables with `size > 8`.  Placed sequentially starting at
  `B + zone1_size`, in the order their `Value::Set` appears as a direct top-level
  operator of the scope's Block node.

`Block.var_size` stores `zone1_size` — the number of bytes claimed by `OpReserveFrame`.

### Why large-type positions are now exact

When `assign_slots` processes scope S using the IR tree:

1. Frame base `B` is known (computed from ancestors).
2. `zone1_size` is computed by colouring S's small variables.
3. The tree walk iterates through S's Block operators in order:
   - `Value::Set(v, ...)` where `v ∈ S` and `v` is large: place `v` at current `tos`
     and advance `tos += size(v)`.
   - `Value::Block(child)` / `Value::Loop(child)`: recurse into child with its own
     frame base = current `tos`.  After the child returns, `tos` is **unchanged**
     (child block cleans up with its own `OpFreeStack`).
   - `Value::If(cond, then, else)`: process `then` and `else` sub-blocks each from the
     current `tos`; after both arms, `tos` is unchanged (gen_if resets stack.position
     between arms).
   - All other operators: no large variable initialization, `tos` unchanged.

Step 3 uses the same pass ordering that codegen will use, so `tos` exactly tracks
`stack.position` at every `Value::Set` for a large variable.

Because every large variable `v` in scope S has its first `Value::Set` as a direct
top-level operator of S's block (scope assignment in scopes.rs guarantees this), the
walk never misses a placement.

---

## Implementation Details

All steps complete.  Code details are in commit history and CHANGELOG.md.
The summaries below describe the design intent of each component.

### 1. `Block.var_size` (`src/data.rs`)

`u16` field added to `Block`.  Stores the zone-1 pre-claim size in bytes (0 until
`assign_slots` runs).  Default-initialised to 0 in all Block constructors.

### 2. `OpReserveFrame` (`default/01_code.loft`, `src/fill.rs`, `src/state/mod.rs`)

New opcode inserted at index 7 in the `OPERATORS` table.  At runtime, advances
`stack.stack_pos` by its `size: u16` operand.  This is the only change to `fill.rs`
and the interpreter — no structural change to the bytecode format.

### 3. `assign_slots` — new algorithm (`src/variables/`)

Signature: `assign_slots(function, code: &mut Value, local_start)`.

Entry: `process_scope(function, code, local_start, 0)`.

`process_scope` — colours small variables (≤ 8 B) within `[frame_base, frame_base + zone1_size)`,
stores `zone1_size` in `bl.var_size`, then walks the block via `place_large_and_recurse`.

`place_large_and_recurse` — places large variables (> 8 B) at the running `*tos` in IR-walk
order (matching codegen order).  Recurses into child Blocks/Loops via `process_scope`
(child `*tos` unchanged after — child has its own `OpFreeStack`).  If/else arms each
start from a saved `branch_tos` that is restored after both arms.
Special case: `Set(v, Block)` for non-Text large `v` — calls `process_scope` on the Block
with `frame_base = v.stack_pos` (the block runs in-place at v's slot at codegen time).

### 4. `generate_block` — emit `OpReserveFrame` (`src/state/codegen.rs`)

Before the first operator, if `block.var_size > 0`, emits `OpReserveFrame(var_size)` and
advances `stack.position` by `var_size`.  `OpFreeStack` at block exit uses the
pre-`OpReserveFrame` `to` value, correctly freeing both zones.

### 5. `scopes.rs` call site

`assign_slots(&mut d.variables, &mut d.code, local_start)` called once per function after
scope analysis.

### 6. `validate_slots` — scope ancestry check (`src/variables/`)

`find_conflict` skips variable pairs in sibling execution branches (neither scope is an
ancestor of the other).  `build_scope_parents` builds a parent map from the IR tree;
`scopes_can_conflict(sa, sb, parents)` returns `false` for siblings.

**Known limitation:** variables with `scope == u16::MAX` (no scope assigned) are treated
as always-conflicting.  Currently safe because all such synthetics (`_read_N`) are
created without user-facing Set nodes in sibling branches (see Open Issues for latent
`Value::Iter` risk).

---

## Open Issues

### B-binary: `_read_N` scope is `u16::MAX` → false-positive conflict — **FIXED**

**Was:** `validate_slots` panicked because `_read_23.scope == u16::MAX`, making
`scopes_can_conflict` always return `true` → false-positive conflict with `f`.

**Root cause (confirmed):** `f#read(4) as i32;` (a discarded read in the test, line 142)
produces `Value::Drop(Block([Set(_read_23, null_int), ...]))`.  `scopes::scan_inner` did
not handle `Value::Drop` — it fell through to `_ => val.clone()`, skipping the inner
block entirely.  `Set(_read_23, ...)` was never seen by `scan_set`, so `_read_23` was
never inserted into `var_scope`, and `scopes::check` never called `set_scope` for it.

**Fix (one line in `scopes.rs`):**
```rust
Value::Drop(inner) => Value::Drop(Box::new(self.scan(inner, function, data))),
```
Added before the `_ => val.clone()` catch-all in `scan_inner`.  The inner block now gets
fully scanned: `_read_23` is inserted into `var_scope` with its correct scope, and
`scopes::check` sets its `.scope` field accordingly.

### `scan_inner` — `Value::Iter` sub-expressions *(FIXED)*

`scan_inner` in `scopes.rs` now handles `Value::Iter` by recursing into all three
sub-expressions (`create`, `next`, `extra`), mirroring the `compute_intervals` arm.
A full cross-check (2026-03-24) confirmed that `scan_inner`, `build_scope_parents`,
and `compute_intervals` all handle every `Value` variant containing nested expressions.

### `place_large_and_recurse` — Zone-2 ordering invariant

**Status:** documented in code; invariant maintained by parser.

**Assumption:** every large variable's first `Value::Set(v, ...)` appears as a direct
top-level operator of its scope's Block — never nested inside a `Call` argument or other
non-recursed position.  `place_large_and_recurse` only visits Set nodes it encounters
while walking block operators and their directly-recursed children.

**If violated:** `v` would never be visited, keeps `stack_pos = u16::MAX`, and
`generate_set` would panic trying to use that as a stack position.

**Invariant source:** the parser always emits variable first-assignments as block-level
statements.  Any future parser change that produces a Set inside an expression argument
must either update `place_large_and_recurse` or ensure the new Set node is reached via
an already-recursed arm (e.g. `Value::Drop`, `Value::Return`).

### `is_scope_ancestor` — cycle in parent map *(FIXED)*

**Was:** `build_scope_parents` did `parents.insert(bl.scope, parent)` unconditionally.
If a scope number appeared more than once in the IR (e.g., a synthetic block sharing
a scope number with an outer block), the second insert could overwrite the first with
a wrong parent, creating a self-loop `map[S] = S`.  `is_scope_ancestor` then looped.

**Fix (2026-03-24):** `build_scope_parents` now uses `entry().or_insert()` to keep the
first-seen (structurally outermost) parent, and skips the insert entirely when
`bl.scope == parent` (which would be a self-loop).  The step-counter safety guard in
`is_scope_ancestor` is retained as a belt-and-suspenders defence.

---

## Stack Efficiency Comparison

### What the new approach cannot do

Cross-scope slot sharing: a child-scope variable reusing a dead slot from a parent scope.
In the new approach each scope has its own frame; child frames start above the parent
frame even if parent variables are already dead.

### When it doesn't matter

If all variables in every scope overlap with each other (no dead parent slots to share),
the two approaches produce identical layouts.  The `string_scope` test (7 levels of
nesting, all variables live simultaneously) gives **172 bytes** in both approaches.

### When the new approach costs extra

The `loop_variable` test: `a` (block:2, size 4) and `test_value` (block:1, size 4) have
non-overlapping intervals and share slot 28 in the old approach.  In the new approach
`a` is in its own frame starting at 32.  Overhead: 4 bytes.

### Sequential blocks — no overhead

22 sequential `{f = file(...); ...}` blocks: each block pre-claims and releases the same
12-byte zone1 frame.  Identical to old approach.

### Summary

| Pattern | Old | New | Δ |
|---------|-----|-----|---|
| Same-scope colouring | N B | N B | 0 |
| Sequential sibling blocks | reuse | reuse | 0 |
| All vars live simultaneously at every level | N B | N B | 0 |
| Child var reuses dead parent slot | allowed | not allowed | +child size |
| TOS estimate wrong → codegen override → waste | possible | impossible | new wins |

Worst-case overhead per nesting level: size of dead parent-scope variables that could
have been shared.  Typically 0–8 bytes per level; up to 24 bytes for a dead Text
variable.  Immaterial on desktop targets.

---

## Implementation Status

| Step | Description | Status |
|------|-------------|--------|
| 1 | `Block.var_size` field | ✅ |
| 2 | `OpReserveFrame` opcode + interpreter | ✅ |
| 3 | New `assign_slots` / `process_scope` / `place_large_and_recurse` | ✅ |
| 4 | `generate_block` emits `OpReserveFrame` | ✅ |
| 5 | Enable `slots.rs` regression tests | ✅ all 3/3 |
| 6 | Replace override branches with debug assertions | ⚠️ partial — `pos > TOS` guarded by `debug_assert`; `pos < TOS + large_type` retained for `&vector<T>` args |
| 7 | Remove `eager_slots` / `assign_slots_old` dead machinery | ✅ |
| 8 | Fix `Set(v, Block)` ordering in `place_large_and_recurse` (Issue 72) | ✅ |
| 9 | `pos != u16::MAX` release guard + `pos <= stack.position` regression assert | ✅ |
| 10 | Audit `build_scope_parents` for missing IR variants; fix scope-cycle root cause | ✅ |

**Step 10 detail (completed 2026-03-24):** Full cross-check confirmed all `Value` variants
with nested expressions are handled in `build_scope_parents`, `scan_inner`, and
`compute_intervals`.  Scope-cycle root cause fixed: `build_scope_parents` now uses
`entry().or_insert()` and skips self-loops.

---

## See Also

- [ASSIGNMENT.md](ASSIGNMENT.md) — History of A6, P1, P2 proposals
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — Detailed analysis of earlier bugs
- [PROBLEMS.md](PROBLEMS.md) — General known issues tracker
