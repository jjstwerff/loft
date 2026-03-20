# Stack Slot Assignment

## Contents
- [Resolved Issues](#resolved-issues)
- [Open Issues](#open-issues)
- [Resolved Bugs](#resolved-bugs)
- [P1 — Scope-analysis pre-init (complete)](#p1--scope-analysis-pre-init-option-a-sub-3)
- [P2 — Full slot assignment pass (planned as A6)](#p2--full-slot-assignment-pass-option-a)
- [Current status (2026-03-11)](#current-status-2026-03-11)

---

## Resolved Issues

### Issue 1 — Borrowed-reference pre-init (FIXED 2026-03-13)

**Test:** `slot_assign::long_lived_int_and_copy_record_followed_by_ref` — passes without `#[ignore]`.

Borrowed refs first assigned inside a branch are correctly pre-initialized by the
Option A sub-3 work in `scopes.rs`. Also verified: `ref_inside_branch_borrowed` in
`tests/issues.rs`. See [PROBLEMS.md](PROBLEMS.md) #2 for full fix description.

## Open Issues

### Issue 2 — "Different definition of Point." in wrap tests

**Tests:** `wrap::last`, `wrap::dir` — fail with "Different definition of Point."

**Symptom:** A struct named `Point` is registered twice with incompatible types when
`lib/parser.loft` processes function return type references after a struct definition.
This is a pre-existing correctness bug in the loft parser library, unrelated to slot
assignment.  It was previously hidden behind the `validate_slots` panic.

**Status:** Not yet analysed.  Needs its own investigation.

---

### Issue 3 — Full slot assignment pass not implemented

Steps 3 and 4 of Proposal P2 (the assignment pass and removal of `claim()`) have not been
implemented.  The current pre-init approach (P1) is a targeted fix; P2 is the correct
long-term architecture.

**Solved by:** P2

---

## Resolved Bugs

### Bug 1 — `13-file.loft` overlapping allocations (fixed 2026-03-10)

Two variables (`b: Buffer` in scope 12, `f: File alias` in scope 5) were both allocated at
stack position 244.  `b` was still live when `f`'s `PutRef` overwrote slot 244, orphaning
`b`'s store.  The next `ConvRefFromNull` found `allocations[max].free == false` and panicked.

Fixed by ensuring distinct stack positions for variables with overlapping lifetimes.

---

### Bug 2 — `t_4Code_define` slot conflict for owned references (fixed by P1 for owned refs)

`_elm_1` (DbRef, 12 bytes) was claimed at slot 62 after a `CopyRecord` dropped
`stack.position` from 86 to 62, overlapping `res: integer` at [66, 70).  `validate_slots`
detected and panicked.

Fixed by P1: `scopes.rs` now emits `Set(_elm_1, Null)` before the `if`, pre-claiming the
slot at a safe position before the `CopyRecord` fires.  `validate_slots` no longer panics
for `t_4Code_define`.

---

### Bug 3 — `OpCreateStack` used variable number instead of stack offset (fixed 2026-03-11)

`OpCreateStack(pos)` requires a relative stack offset, not a variable index. Fixed in `state.rs::generate_set`: `pos` is now `before_stack - dep_pos` where `before_stack = stack.position - size_of::<DbRef>()` and `dep_pos = stack.function.stack(dep[0])`. Applied to both Reference/Enum-ref and Vector branches.

---

## Proposals

### P1 — Scope-analysis pre-init (Option A sub-3)

**Solves:** Bug 2 (owned refs — done), Issue 1 (borrowed refs — done)

**Status:** Fully implemented 2026-03-11/13.

#### Core idea

`scopes.rs` already pre-emits `Set(dep, Null)` for dependent-type variables before they are
used.  Extend this to Reference/Vector/Text variables first assigned inside an if/else
branch: emit `Set(v, Null)` *before* the `Value::If` node, while `stack.position` is still
safely above all live variable slots.

When codegen reaches `Set(v, actual_value)` inside the branch, `v` is already claimed
(`pos != u16::MAX`).  The assignment takes the re-assignment path in `generate_set`, which
calls `set_var`.  `set_var` generates the value at the current (valid) `stack.position` and
copies the result into `v`'s pre-claimed slot via `OpPutRef` — no bridging needed.

#### Why the pre-init is always at a safe stack position

The pre-init fires before the `Value::If` node is entered.  Any `CopyRecord` that would
lower `stack.position` into the danger zone lives *inside* the if/else branch — it has not
run yet.  Therefore `stack.position == State::stack_pos` and the pre-init's
`OpConvRefFromNull`/`OpText` writes at the correct address.

#### Implementation

**`needs_pre_init` predicate** (`src/scopes.rs`):

```rust
fn needs_pre_init(tp: &Type) -> bool {
    matches!(
        tp,
        Type::Text(_) | Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
    )
}
```

All Reference, Vector, Enum-ref, and Text types are included regardless of `dep`, because
all occupy 12 bytes on the stack and can cause the same slot overlap.  A `deps_ready`
check in `find_first_ref_vars` gates borrowed variables on all deps already being in
`var_scope` (otherwise `OpCreateStack` would reference an uninitialised slot).

**`find_first_ref_vars` method** (`src/scopes.rs`, inside `impl Scopes`):

Recursively walks a `Value` subtree and collects variables that:
- appear as the target of `Value::Set(v, ...)`,
- are not yet in `var_scope`, and
- satisfy `needs_pre_init` and `deps_ready`.

Recurses into nested `If` and `Block` but NOT `Loop` (loop variables have per-iteration
scope management and must not be pre-inited at the enclosing scope).

**Modified `scan` arm for `Value::If`** (`src/scopes.rs`):

1. Call `find_first_ref_vars` on both branches to collect pre-init candidates.
2. Register each candidate in `var_scope` at the current scope.
3. Scan the if normally (branches see the candidates as already assigned).
4. Prepend `Set(v, Null/empty)` for each candidate before the scanned if in a
   `Value::Insert`.

**Pre-init `Set(v, Null)` codegen** (`src/state/codegen.rs::generate_set`):

- Owned Reference/Enum-ref → `OpConvRefFromNull` (pushes a null DbRef)
- Borrowed Reference/Enum-ref → `OpCreateStack(before_stack - dep_pos)`
- Owned Vector → `OpConvRefFromNull` + `OpDatabase` + `OpVarRef` + length init
- Borrowed Vector → `OpCreateStack(before_stack - dep_pos)`
- Text → `OpText` (pushes empty string)

#### Files changed

| File | Change |
|---|---|
| `src/scopes.rs` | `needs_pre_init` free function |
| `src/scopes.rs` | `find_first_ref_vars` method |
| `src/scopes.rs` | `scan` arm for `Value::If` emits pre-inits |
| `src/state/codegen.rs` | `OpCreateStack` offset fixed (Bug 3 above) |

#### Test results (2026-03-11)

```
cargo test --test enums -- polymorph                # PASSES ✓
cargo test --test slot_assign                       # 5 pass (no ignored)
cargo test --test wrap -- last dir                  # FAILS (Issue 2, separate bug)
```

---

### P2 — Full slot assignment pass (Option A)

**Solves:** Issue 3 (long-term correct architecture)

**Status:** Planned; Steps 1 and 2 done, Steps 3–5 not yet implemented. Tracked as **A6** in PLANNING.md / ROADMAP.md.

#### Core idea

Compute all stack slot positions in a dedicated pass *before* code generation, using the
live intervals produced by `compute_intervals`.  Code generation in `state.rs` reads
pre-assigned `stack_pos` instead of calling `claim()`.

The bridging problem is avoided because slots are assigned globally — there is no longer a
moment where `stack.position` drops below a live variable's slot and then a new claim is
made at the wrong position.

#### Data flow

```
Parser (two passes)
  └─ variables.rs: add_variable(), copy_variable()   ← names, types, scopes (unchanged)
       scope_analysis (scopes.rs)                    ← scope IDs, OpFreeText/OpFreeRef
            compute_intervals (variables.rs)         ← first_def/last_use per variable [DONE]
                 assign_slots (variables.rs)         ← assign stack_pos from intervals [TODO]
                      [debug] validate_slots         ← assert no overlapping live slots [DONE]
                           byte_code (state.rs)      ← reads stack_pos, no claim() [TODO]
                                execute()
```

#### Step 1 — `compute_intervals` (DONE — `src/variables.rs`)

`compute_intervals(val, function, free_text_nr, free_ref_nr, seq)` walks the IR in
execution order, recording `first_def` and `last_use` on each `Variable`.  Called from
`scopes::check` after the scope pass.

**Key concepts:**

- `first_def`: sequence number of the `Value::Set(v, …)` that first defines `v`.
- `last_use`: sequence number of the last `Value::Var(v)` (or implicit `OpFreeText`/
  `OpFreeRef`) for `v`.
- Overlapping lifetimes: `u.first_def <= v.last_use && v.first_def <= u.last_use`.

#### Step 2 — `validate_slots` (DONE — `src/variables.rs`)

`validate_slots(function, data, def_nr)` (debug-only) checks every variable pair for
simultaneous live-interval overlap and slot overlap.  Logs a full diagnostic then panics.
Uses the extracted `find_conflict(vars)` helper for testability.

Unit tests in `src/variables.rs` cover non-overlapping intervals, non-overlapping slots,
integer inside wider DbRef slot, and edge cases.

Integration tests in `tests/slot_assign.rs` (5 tests; all pass).

#### Step 3 — `assign_slots` (TODO — `src/variables.rs`)

Add `assign_slots(vars: &mut [Variable], arguments_size: u16)`.

Algorithm (linear scan):

```
active: list of (interval_end, slot_start, slot_size)
free_primitive_slots: list of (slot_start, slot_size)

sort vars by first_def ascending
position = arguments_size   // next free byte after arguments

for each variable v in sorted order:
    // expire slots whose interval ended before v.first_def
    for each slot in active where interval_end < v.first_def:
        if slot is primitive type:
            free_primitive_slots.push(slot)
        active.remove(slot)

    if v is primitive type:
        if free_primitive_slots has compatible slot s:
            v.stack_pos = s.start
            free_primitive_slots.remove(s)
        else:
            v.stack_pos = position
            position += size(v.type_def, Context::Variable)
    else:
        // ref / text: always fresh slot, never reuse
        v.stack_pos = position
        position += size(v.type_def, Context::Variable)

    active.push((v.last_use, v.stack_pos, size(v)))
```

Skip variables with `argument == true` — they already have `stack_pos` set by argument
layout.  `OpFreeText`/`OpFreeRef` insertion by `scopes.rs` must happen before this pass
runs, so those implicit last-uses are visible to liveness.

#### Step 4 — Remove `claim()` from `state.rs` (TODO)

After the assignment pass, `generate_set` reads the pre-assigned `stack_pos` instead of
calling `claim`.  `stack.position` still needs advancing so subsequent offset computations
are correct:

```rust
// In generate_set, when pos != u16::MAX:
stack.position = stack.position.max(pos + var_size);
```

#### Step 5 — Remove `copy_variable` (deferred)

After Steps 3–4 are complete and `validate_slots` is green, `copy_variable` can be
removed.  Variables re-used across sibling scopes will simply have non-overlapping
intervals and can share or not share slots based on the interval check.

#### Invariants to preserve

- Arguments occupy positions 0 … arguments_size−1; skip them in `assign_slots`.
- `OpFreeText`/`OpFreeRef` insertion happens before `assign_slots` runs.
- The runtime stack pointer (`State::stack_pos`) is unchanged by the pass; it still
  advances and retreats during block execution via `OpFreeStack`.
- After `assign_slots`, `state.rs::generate_set` must advance `stack.position` past each
  variable's slot so subsequent ops compute correct relative offsets.

---

## Current status (2026-03-20)

| Step | Status |
|---|---|
| `compute_intervals` | **Done** (loop-carried extension, Iter traversal, write-target last_use) |
| `validate_slots` + `find_conflict` | **Done** (debug-only) |
| Unit tests for `find_conflict` | **Done** (`src/variables.rs`) |
| Integration tests (`tests/slot_assign.rs`) | **Done** (5 tests; all pass) |
| P1: pre-init for owned refs | **Done** |
| P1: pre-init for borrowed refs | **Done** |
| Bug 3: `OpCreateStack` offset | **Fixed** |
| A6.2: shadow mode | **Done** (removed — superseded by A6.3 safe mode) |
| A6.3a: `assign_slots_safe` pre-pass | **Done** (superseded by A6.3b) |
| A6.3b: `assign_slots` greedy mode | **Done** — unconditional default; all tests pass |
| A6.4: remove `claim()` | **Open** — `claim()` and `assign_slots_safe` are dead code |
| P2: remove `copy_variable` | **Deferred** |
| Issue 2: "Different definition of Point." | **Open** (separate bug) |

---

## See also
- [PLANNING.md § A6](PLANNING.md) — A6 backlog item with current phase breakdown
- [SLOT_FAILURES.md](SLOT_FAILURES.md) — Detailed analysis of the three bugs blocking the optimised mode
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
- [INTERMEDIATE.md](INTERMEDIATE.md) — Value/Type enums in detail; 233 bytecode operators; State layout
