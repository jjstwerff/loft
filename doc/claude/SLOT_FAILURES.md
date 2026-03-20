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

| Test | Safe | Optimised | Legacy |
|------|------|-----------|--------|
| filter_integers | **FAIL** | **FAIL** | **FAIL** |
| map_integers | **FAIL** | **FAIL** | **FAIL** |
| fn_ref_conditional_call | pass | **FAIL** | pass |
| n10_char_cast_in_generated_code | pass | **FAIL** | pass |
| ref_param_append_bug | **FAIL** | **FAIL** | **FAIL** |

`ref_param_append_bug` panics in `src/store.rs` ("Unknown record 5") — unrelated to slot
assignment and excluded from this analysis.

---

## Bug A — Comprehension aliasing (filter / map, all modes)

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

Require exact size match for dead-slot reuse:

```rust
// was:  let can_reuse = var_size <= 4;
let can_reuse = var_size <= 4 && var_size == j_size;
```

`j_size` is already computed in the inner loop (`size(&function.variables[j].type_def, ...)`).
A 4-byte `f` may not reuse a 1-byte `flag` slot; it gets a fresh slot at the watermark.

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

## Solution summary

Three independent changes, two functions:

| Bug | File | Change |
|-----|------|--------|
| A — comprehension aliasing | `src/variables.rs` `compute_intervals` | Don't set `first_def` early for `Vector` types; only do so for `Text` and `Reference` |
| B — narrow→wide reuse | `src/variables.rs` `assign_slots` | Add `&& var_size == j_size` to the dead-slot reuse guard |
| C — Iter not traversed | `src/variables.rs` `compute_intervals` | Add `Value::Iter` case that recurses into `create`, `next`, `extra_init` |

No changes to `generate_set`, `assign_slots_safe`, or any codegen logic.
All three fixes are additive; none removes existing behaviour.

After these fixes, all currently-failing slot-related tests are expected to pass in all
three modes.
