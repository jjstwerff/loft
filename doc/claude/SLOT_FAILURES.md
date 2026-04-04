---
render_with_liquid: false
---
# Slot assignment failure analysis and design

> **HISTORICAL** — All three bugs (A, B, C) documented here are fixed (A6.3a/b, two-zone redesign).
> The failure matrix shows every listed test passing.
> The current slot assignment design is in [SLOTS.md](SLOTS.md).
> Issue 72 (block-return slot conflict) is documented in [PROBLEMS.md](PROBLEMS.md) § 72.

## Modes

As of A6.3b, `assign_slots` (greedy interval colouring) is the unconditional default.
The `LOFT_ASSIGN_SLOTS` and `LOFT_LEGACY_SLOTS` env-var gates have been removed.

| Mode | Status | Strategy |
|------|--------|----------|
| greedy (default) | **Active** | `assign_slots`: interval colouring with same-size primitive reuse |
| legacy | Removed | no pre-pass; `claim()`-at-TOS in codegen |

---

## Failure matrix

| Test | Status | Bug |
|------|--------|-----|
| filter_integers | **pass** | A — Fixed by A6.3a |
| map_integers | **pass** | A — Fixed by A6.3a |
| fn_ref_conditional_call | **pass** | B — Fixed by A6.3b |
| n10_char_cast_in_generated_code | **pass** | C — Fixed by A6.3b |
| ref_param_append_bug | **FAIL** | unrelated (store.rs bug) |

All three slot-assignment bugs (A, B, C) are fixed.  `ref_param_append_bug` is a
pre-existing `store.rs` bug unrelated to slot assignment.

---

## Bug A — Comprehension aliasing (filter / map) — FIXED by A6.3a

### Observed slot layout in `n_test`

| var | slot (all modes) | size | first_def | last_use |
|-----|-----------------|------|-----------|----------|
| r | [88, 100) | 12 | 119 | 176 |
| _filter_result_5 | **[88, 100)** — wrong | 12 | 123 | 171 |
| _filter_vec_2 | cascades down 12 B | 12 | 133 | 170 |

Both `r` and `_filter_result_5` end up at slot 88.  Their live intervals overlap at [123, 171].
`validate_slots` panics in all three modes.

### Why they collide

`r = filter(v, fn is_even)` expands to `r = {#Vector comprehension}`.
In `generate_set` the direct-placement path (`pos == stack.position == 88`) calls
`generate(comprehension_body, stack)` **without first advancing `stack.position`**.
When the comprehension body starts, TOS is still 88 (the start of r's slot).
`_filter_result_5` is first-allocated inside the body; `pos(100) > TOS(88)` fires the
claim() fallback, overriding its pre-assigned slot to 88 — the same as `r`.

At runtime both variables point to the same 12-byte DbRef in `__ref_3`; the aliasing is
intentional and correct.  The problem is purely that `validate_slots` sees two distinct
variables with overlapping live intervals at the same slot bytes.

### Root cause in `compute_intervals`

`compute_intervals` handles `Set(v, value)` with this logic for large types (size > 4):

```rust
// Large type: set first_def BEFORE traversing value
if large_type && first_def == u32::MAX {
    v.first_def = *seq;
    *seq += 1;
}
compute_intervals(value, ...);
```

The comment explains the intent: the pre-init opcode for `Text` (`OpText`) and `Reference`
(`OpConvRefFromNull`) fires at TOS **before** the value expression runs, so `first_def` must
be earlier than any inner variable so that `assign_slots` gives `v` the lower slot.

But **`Vector` comprehensions have no pre-init opcode**.  The comprehension body IS the init.
Applying early `first_def` to a Vector type gives `r` a `first_def` (119) that precedes
`_filter_result_5` (123), making their intervals overlap, which `validate_slots` rejects.

### Fix A — one-line change in `compute_intervals`

Skip the early-`first_def` path for `Vector` types:

```rust
// Large-type pre-init opcode fires before value — only true for Text and Reference.
// Vector variables are initialised by a comprehension body; no pre-init is emitted.
let needs_early_first_def = large_type
    && !matches!(function.variables[v].type_def, Type::Vector(_, _));
```

With this change, `r.first_def` is set **after** the comprehension body (seq ≈ 172).
`_filter_result_5.last_use` is 171.  Their intervals no longer overlap.

In all three modes the runtime slot stays 88 (claim() gives `r` TOS=88), so no behaviour
changes.  `validate_slots` sees r.first_def > _filter_result_5.last_use and does not panic.

**Status (2026-03-20):** Applied in A6.3a.  The actual implementation chose the equivalent
but more precise form: `matches!(type_def, Type::Text(_) | Type::Reference(_, _) | Type::Enum(_, true, _))` —
only types that have pre-init opcodes trigger early `first_def`.  The effect is the same:
Vector is excluded.  All tests pass in default mode.

---

## Bug B — Narrow → wide slot reuse (fn_ref_conditional_call, optimised only)

### Observed slot layout in `n_test`

| var | slot (optimised) | size | first_def | last_use |
|-----|-----------------|------|-----------|----------|
| flag | [28, 29) | 1 | 4 | 5 |
| f | **[28, 32)** — wrong | 4 | 8 | 10 |
| result | [28, 32) | 4 | 11 | 18 |

### Why it fails

`assign_slots` allows `f` (4 B fn-ref) to reuse `flag`'s dead 1-byte slot because
`can_reuse = var_size(4) <= 4` and `flag.last_use(5) < f.first_def(8)`.

At codegen time TOS = 29 (advanced by 1 for `flag`).  The reuse path calls `set_var →
OpPutX`:

1. f's value is generated at TOS=29 → fn-ref occupies bytes **[29, 33)**, TOS → 33.
2. `OpPutX` displacement = `stack.position(33) − pos(28) = 5`.
3. `OpPutX(5, size=4)` addresses `stack_top − 5 = 28`, but the value starts at byte **29**.
   The store reads one byte of stale `flag` data as the high byte of the fn-ref definition
   number, producing d_nr = 10661 (out of range), panicking at runtime.

This displacement mismatch only occurs when the dead slot is **narrower** than the new
variable.  If both were 4 bytes (or both 1 byte) the math is exact.

Passes in safe and legacy because claim()-at-TOS always places `f` at the natural TOS=29.

### Fix B — one condition added in `assign_slots`

Require exact size match for dead-slot reuse.  The check lives inside the inner loop
where `j_size` is already computed.  The existing `!can_reuse` guard (large types) is
extended to also reject size-mismatched reuse for small types:

```rust
// src/variables/  assign_slots  (inner loop, dead-slot-overlap path)

// was:
if !can_reuse {
    candidate = j_end;
    continue 'retry;
}

// fix:
if !can_reuse || var_size != j_size {
    candidate = j_end;
    continue 'retry;
}
```

`j_size` is already computed two lines above in the same loop body
(`let j_size = size(&function.variables[j].type_def, &Context::Variable)`).
A 4-byte `f` may not reuse a dead 1-byte `flag` slot; it gets a fresh slot at the
watermark.  Two same-sized primitives (e.g., two dead 4-byte integers) may still share
a slot.

---

## Bug C — Iter variables not tracked by `compute_intervals` (n10_char_cast, optimised only)

### Observed slot layout in `n_count_alpha`

| var | slot (optimised) | size | first_def | last_use |
|-----|-----------------|------|-----------|----------|
| n | [20, 24) | 4 | 1 | 28 |
| c#index | **[20, 24)** — wrong | 4 | 3 | **0** ← bug |
| c#next | [24, 28) | 4 | 5 | 27 |
| _for_result_1 | [28, 32) | 4 | 11 | 17 |
| c | [28, 32) | 4 | 18 | 22 |

`c#index.last_use = 0`.  The interval overlap test becomes
`c#index.last_use(0) >= n.first_def(1)` → **false**, so `assign_slots` treats `c#index` as
dead before `n` is even defined and lets it share slot 20.  At runtime the loop counter
overwrites the accumulator every iteration; `count_alpha("a1!")` returns 3 instead of 2.

### Root cause

`Value::Iter` is not handled in `compute_intervals`:

```rust
// src/data.rs
Iter(u16, Box<Value>, Box<Value>, Box<Value>),
// fields: (index_var, create, next, extra_init)
// extra_init: Null for non-text, v_set(index_var, 0) for text loops
```

```rust
// src/variables/  compute_intervals
_ => {
    *seq += 1;  // Iter falls here — none of create/next/extra_init are traversed
}
```

All variables read inside the iterator's `create`, `next`, and `extra_init` sub-expressions
(including `c#index` in the character-advance next function) never get their `last_use`
updated.  They keep `last_use = 0`, making them appear dead at birth.

`c#next.last_use = 27` is correct because `c#next` happens to be read **outside** the Iter
node somewhere (likely the loop extension fires for it via the surrounding `Value::Loop`).
`c#index` is only read inside the Iter sub-expressions and is therefore invisible.

Passes in safe and legacy because `assign_slots_safe` gives `c#index` a fresh slot regardless
of intervals, and claim()-at-TOS never reclaims a live slot.

### Fix C — add `Value::Iter` case to `compute_intervals`

```rust
Value::Iter(index_var, create, next, extra_init) => {
    // Record last_use for the index variable itself.
    let v = *index_var as usize;
    if v < function.variables.len() {
        function.variables[v].last_use =
            function.variables[v].last_use.max(*seq);
    }
    *seq += 1;
    // Recurse into all three sub-expressions so variables read inside
    // create / next / extra_init get correct last_use values.
    compute_intervals(create, function, free_text_nr, free_ref_nr, seq);
    compute_intervals(next,   function, free_text_nr, free_ref_nr, seq);
    compute_intervals(extra_init, function, free_text_nr, free_ref_nr, seq);
}
```

With this, `c#index.last_use` is updated to a seq inside the loop body.  The loop-carried
extension then fires (`c#index.first_def(3) < seq_start`, `last_use >= seq_start`) and
extends `c#index.last_use` to `loop_last`.  `assign_slots` then sees
`c#index.last_use > n.first_def` → conflict → `c#index` gets a fresh slot.

---

## A6.3b solution summary

| Bug | Status | File | Change |
|-----|--------|------|--------|
| A — comprehension aliasing | **Fixed (A6.3a)** | `src/variables/` `compute_intervals` | `needs_early_first_def` excludes `Type::Vector` |
| B — narrow→wide reuse | **Fixed (A6.3b)** | `src/variables/` `assign_slots` | `\|\| var_size != j_size` added to dead-slot-overlap guard |
| C — Iter not traversed | **Fixed (A6.3b)** | `src/variables/` `compute_intervals` | `Value::Iter` arm recurses into `create`/`next`/`extra_init`; `Value::Set` now updates `last_use` for write targets |

All three bugs are fixed.  `assign_slots` (greedy) is the unconditional default as of
A6.3b.  All tests pass except `ref_param_append_bug` (pre-existing `store.rs` bug).

---

## A6.4 — Remove `claim()` (DONE 2026-03-20)

### What was done

`claim()` replaced by `set_stack_pos()` — a minimal method that only sets the slot
position, with the caller advancing `stack.position` separately.

**TOS-drop case** (`pos > stack.position`): when an if-else restores TOS below a
pre-assigned slot, `set_stack_pos(v, stack.position)` overrides the slot to current
TOS so direct placement fires correctly.  Advancing TOS via `max` (the earlier design)
was incorrect: it advanced the compile-time watermark without filling the gap on the
runtime stack, causing "Variable outside stack" panics.

**1. `src/state/codegen.rs` — `generate_set` first-alloc path (line ~411)**

```rust
// BEFORE (A6.3b):
stack.function.set_stack_allocated(v);
if pos > stack.position {
    // Pre-assigned slot is above current TOS (can happen after if-else branch
    // restores stack.position).  Fall back to claim() so the slot matches TOS.
    stack.function.claim(v, stack.position, &Context::Variable);
}
let pos = stack.function.stack(v);

// AFTER (A6.4):
stack.function.set_stack_allocated(v);
// Trust the pre-assigned slot; advance TOS to cover it.
stack.position = stack.position.max(pos.saturating_add(var_size));
let pos = stack.function.stack(v);
```

`stack.position.max(pos + var_size)` is correct for all four cases:

| Case | Condition | Effect on TOS | Then |
|------|-----------|---------------|------|
| Fresh slot (normal) | `pos == stack.position` | advances by `var_size` | direct placement (`pos == TOS`) |
| Dead-slot reuse | `pos < stack.position` | TOS unchanged | OpPutX path (`pos < TOS`) |
| TOS drop (if-else) | `pos > stack.position` | advances to `pos + var_size` | OpPutX path (`pos < new TOS`) |
| Large types | always fresh, `pos == stack.position` | advances by `var_size` | direct placement |

**TOS-drop case detail:** When an if-else restores `stack.position` to a value below
`v`'s pre-assigned slot, `claim()` overrides the slot to the restored TOS.  After A6.4
the slot is preserved and TOS is advanced past it instead.  The value is generated at
the new TOS, then `OpPutX` copies it into the pre-assigned slot.  The
`debug_assert!(pos < stack.position)` on the OpPutX path still holds because
`pos < pos + var_size == new stack.position`.

**`pos.saturating_add`** prevents overflow if `pos` were ever `u16::MAX`; the `max`
then leaves TOS unchanged instead of wrapping.

**2. `src/state/codegen.rs` — argument setup (~line 32)**

```rust
// BEFORE:
stack.position = stack.function.claim(v, stack.position, &Context::Argument);

// AFTER (inline what claim() did):
stack.function.variables[v as usize].stack_pos = stack.position;
stack.position += size(stack.function.tp(v), &Context::Argument);
```

**Deleted:** `pub fn claim(...)`, `pub fn assign_slots_safe(...)`, and
`LOFT_DEBUG_SLOTS` debug blocks in both `variables/` and `codegen.rs`.

---

## See also
- [SLOTS.md](SLOTS.md) — Two-zone slot-assignment design and current diagnostic tools
- [ASSIGNMENT.md](ASSIGNMENT.md) — Full design history: P1/P2 proposals and the A6 resolution
- [PROBLEMS.md](PROBLEMS.md) — Issues 68–70 (blockers for A12 lazy init) are direct descendants of this analysis
- [TESTING.md](TESTING.md) — `variables` `LOFT_LOG` preset for slot-interval diagnostics
