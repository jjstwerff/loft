# Slot assignment failure analysis and design

## Modes

Three environment-variable modes select the slot-assignment strategy:

| Mode | Env var | Strategy |
|------|---------|----------|
| safe sequential (default) | — | `assign_slots_safe`: fresh slot per variable, high-watermark |
| optimised greedy | `LOFT_ASSIGN_SLOTS=1` | `assign_slots`: greedy interval colouring with small-type reuse |
| legacy | `LOFT_LEGACY_SLOTS=1` | no pre-pass; `claim()`-at-TOS in codegen |

---

## Failure matrix

| Test | Safe | Optimised | Legacy | Bug |
|------|------|-----------|--------|-----|
| filter_integers | pass | pass | pass | A — **Fixed by A6.3a** |
| map_integers | pass | pass | pass | A — **Fixed by A6.3a** |
| fn_ref_conditional_call | pass | **FAIL** | pass | B — open |
| n10_char_cast_in_generated_code | pass | **FAIL** | pass | C — open |
| ref_param_append_bug | **FAIL** | **FAIL** | **FAIL** | unrelated (store.rs bug) |

Bug A was fixed by the `needs_early_first_def` change in A6.3a (`compute_intervals` no
longer sets early `first_def` for `Type::Vector`).  Bugs B and C only affect the optimised
greedy mode (`LOFT_ASSIGN_SLOTS=1`).  Safe mode and legacy mode are fully passing.

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
// src/variables.rs  assign_slots  (inner loop, dead-slot-overlap path)

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
// src/variables.rs  compute_intervals
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
| A — comprehension aliasing | **Fixed (A6.3a)** | `src/variables.rs` `compute_intervals` | `needs_early_first_def` excludes `Type::Vector` (and Long/Float) |
| B — narrow→wide reuse | **Open** | `src/variables.rs` `assign_slots` | Add `\|\| var_size != j_size` to the dead-slot-overlap guard |
| C — Iter not traversed | **Open** | `src/variables.rs` `compute_intervals` | Add `Value::Iter` case that recurses into `create`, `next`, `extra_init` |

Fixes B and C are additive; neither removes existing behaviour.  No changes needed to
`generate_set`, `assign_slots_safe`, or any other codegen logic.

After B and C, all currently-failing slot-related tests are expected to pass with
`LOFT_ASSIGN_SLOTS=1`.

---

## A6.4 — Remove `claim()` (deferred until A6.3b is stable)

### Goal

After A6.3b, the greedy `assign_slots` pre-assigns ALL variables with correct slots.
`generate_set` can trust `pos` directly; the `claim()` fallback is no longer needed.

### What changes

**1. `src/state/codegen.rs` — `generate_set` first-alloc path**

```rust
// BEFORE (A6.3a):
stack.function.set_stack_allocated(v);
if pos > stack.position {
    // Pre-assigned slot above TOS or u16::MAX from assign_slots_safe.
    stack.function.claim(v, stack.position, &Context::Variable);
}
let pos = stack.function.stack(v);

// AFTER (A6.4):
stack.function.set_stack_allocated(v);
// Trust the pre-assigned slot. Advance TOS to cover it (no-op for dead-slot reuse).
stack.position = stack.position.max(pos.saturating_add(var_size));
let pos = stack.function.stack(v);
```

`pos.max(stack.position)` is correct because:
- Large types (`!can_reuse` in `assign_slots`): slot is always at the current watermark,
  so `pos == stack.position` and `max` is a no-op advance.
- Primitive dead-slot reuse: `pos < stack.position`, `max` leaves TOS unchanged —
  the `else` path (OpPutX) fires as before.
- Primitive fresh slot: `pos == stack.position`, `max` advances by `var_size`.

**2. `src/state/codegen.rs` — argument setup (~line 32)**

```rust
// BEFORE:
stack.position = stack.function.claim(v, stack.position, &Context::Argument);

// AFTER (inline what claim() did):
stack.function.variables[v as usize].stack_pos = stack.position;
stack.position += size(stack.function.tp(v), &Context::Argument);
```

**3. `src/scopes.rs` — remove env-var gates**

```rust
// BEFORE (A6.3):
if std::env::var("LOFT_LEGACY_SLOTS").is_ok() {
    // legacy
} else if std::env::var("LOFT_ASSIGN_SLOTS").is_ok() {
    assign_slots(vars, local_start);
} else {
    assign_slots_safe(vars, local_start);
}

// AFTER (A6.4):
assign_slots(vars, local_start);
```

**4. `src/variables.rs` — delete**

- `pub fn claim(...)` — no longer called
- `pub fn assign_slots_safe(...)` — no longer called
- `LOFT_DEBUG_SLOTS` env-var debug block inside `assign_slots` — optional cleanup

### Invariants to verify after removal

- `validate_slots` passes on the full test suite with no env-var gates.
- `cargo clippy --tests -D warnings` clean.
- The `pos < stack.position` (OpPutX dead-slot reuse) branch still fires correctly for
  same-size primitive reuse (covered by `assign_slots_sequential_reuse` unit test).

### Commit sequence (DEVELOPMENT.md)

1. No new tests needed (existing suite is the gate).
2. Code: `generate_set` + argument setup + delete `claim()` + delete `assign_slots_safe`.
3. `scopes.rs`: remove env-var gates unconditionally.
4. `cargo test && cargo clippy --tests -D warnings && cargo fmt --check`.
5. Docs: CHANGELOG, PLANNING, ASSIGNMENT, PROBLEMS, ROADMAP.
