# Phase 1 — Checked arithmetic (C54.G-hybrid primary, pure G fallback)

Status: **not started** — unblocked by Phase 0.  Phase 0 audit result:
ship G (trap on overflow), but preserve the `??`-discharge idiom via
a compile-time-detected **G-hybrid** variant.  Pure G is the fallback
if the detection proves invasive.

## Phase 0's gift

`src/ops.rs::checked_int!` (lines 33–52) and `checked_long!` (54–73)
already gate every arithmetic handler through `checked_add` /
`checked_mul` / `checked_sub` / `checked_div` / `checked_rem`.  Only
the macro's **fallback arm** needs to change.

`src/parser/operators.rs::??` (lines 592–670) already recognises the
null sentinel at runtime and returns the RHS — probe 08 proved this
works for arithmetic overflow today without any change.  The G-hybrid
build benefits from this: inside a `??` context, null-propagate and
let `??` discharge; outside, trap.

## G-hybrid design

### The distinction

Every arithmetic op emits either:

- **`Op{Add,Mul,…}IntTrap`** — the default.  Overflow / div-zero
  → runtime trap with `file:line:col` diagnostic.
- **`Op{Add,Mul,…}IntNullable`** — emitted only when the op's
  result is immediately consumed by `??`.  Overflow / div-zero
  → produce `i32::MIN` (the null sentinel).  `??` then catches.

`_long` siblings mirror this pattern.

Today's `Op{Add,Mul,…}Int` (the nullable-by-default variants) stay
around as the Nullable version; we rename them for clarity and add
the new Trap variants.

### Compile-time detection

At codegen for an arithmetic op, look UP the expression tree for the
immediate parent node.  If the parent is `??`, emit the Nullable
variant; else emit the Trap variant.

Critical site: `src/parser/operators.rs` (binary-op codegen for `+`,
`-`, `*`, `/`, `%`).  Around the dispatch to `OpAddInt` / `OpMulInt`
/ etc., check the enclosing context.  The parser's context stack
already tracks ancestor operators (`??` = `OpNullCoalesce` or
similar).  When the enclosing op is `??` AND the current arithmetic
op is the LHS of that `??`, pick Nullable.

If the detection is ambiguous (e.g. `a * b + c ?? default` — does
only `+` get Nullable, or does `*` too?), fall back to: only the
direct LHS of `??` gets Nullable; everything deeper (including
sub-expressions of that LHS) gets Trap.  That's conservative and
easy to reason about.

### Handler emission

For each arithmetic op, `src/ops.rs` gets two functions:

```rust
// Nullable variant — current behaviour, returns sentinel on overflow.
pub fn op_add_int_nullable(v1: i32, v2: i32) -> i32 { /* existing */ }

// Trap variant — new.  Panic / runtime-error on overflow.
pub fn op_add_int_trap(v1: i32, v2: i32) -> i32 {
    if v1 == i32::MIN || v2 == i32::MIN {
        panic!("arithmetic with nullable input (= null sentinel)");
    }
    match v1.checked_add(v2) {
        Some(v) if v != i32::MIN => v,
        _ => panic!("arithmetic overflow"),
    }
}
```

The panic carries `file:line:col` via the existing debug-map /
stack-trace infrastructure (`src/stack.rs` or similar).

Same pattern for `sub`, `mul`, `div`, `rem` across both `int` and
`long`.

Division / modulo by zero:
- `op_div_int_nullable`: today returns `i32::MIN` on `v2 == 0`.
- `op_div_int_trap`: panic with "division by zero at file:line:col".

### Interpreter trap landing

When the panic fires, `src/state/mod.rs`'s opcode dispatch loop
catches it and reports.  Hook into whichever diagnostic emitter the
rest of the interpreter uses.  Suggested site: the top-level
`execute` loop in `src/state/mod.rs`.

### Native codegen path

`src/generation/mod.rs:474` — native codegen already emits
`use loft::ops;` and calls `ops::op_*` directly.  Change: codegen
picks the Trap vs Nullable function name based on the same ancestor
check that the interpreter codegen uses.  Both paths share the
codegen logic in `src/state/codegen.rs` or `src/parser/operators.rs`.

### WASM path

WASM shares the interpreter (`src/state/mod.rs:14`).  No separate
edit.

## Scope — file list

| File | Change |
|---|---|
| `src/ops.rs:33-52` | Keep existing `checked_int!` macro as the **Nullable** logic.  Extract a new `checked_int_trap!` macro that panics instead of returning `i32::MIN`. |
| `src/ops.rs:54-73` | Same for `checked_long!`. |
| `src/ops.rs:531-571` | Add `op_{add,sub,mul,div,rem}_int_trap` functions paralleling the existing int handlers.  Existing handlers become the Nullable variants (rename for clarity). |
| `src/ops.rs:313-405` | Same for long, including `_nn` variants. |
| `src/fill.rs:477-656` | Register new opcode handlers for the Trap variants.  Existing `OpAddInt` / `OpMulInt` / etc. become `OpAddIntNullable` / `OpMulIntNullable` (or keep old names as Nullable aliases to avoid stdlib rewriting). |
| `default/01_code.loft` | Add the new Trap opcode declarations.  Rename existing `OpAddInt` family to `...Nullable` if we want explicit names, or leave as-is and add `...Trap` siblings. |
| `src/native.rs` | Register the Trap variants. |
| `src/parser/operators.rs` | Arithmetic-op codegen: detect "immediate LHS of `??`" context and pick Nullable vs Trap. |
| `src/state/codegen.rs` | Same detection logic if codegen emits arithmetic from here instead of operators.rs (check during implementation). |
| `src/state/mod.rs` | Trap handler: catch the panic, report `file:line:col` + operator, exit non-zero. |

## Test plan

Un-ignore / create.  All tests run on interpreter + native.

| Test | Purpose |
|---|---|
| `c54g_checked_add_traps_on_overflow` | `i32::MAX + 1` in bare context → trap |
| `c54g_checked_sub_traps_on_underflow` | `i32::MIN + 1 - 2` in bare context → trap |
| `c54g_checked_mul_traps_on_overflow` | `i32::MAX * 2` in bare context → trap |
| `c54g_checked_div_traps_on_abs_min` | `i32::MIN / -1` in bare context → trap |
| `c54g_div_by_zero_traps` | `a / 0` in bare context → trap with "division by zero" |
| `c54g_mod_by_zero_traps` | `a % 0` in bare context → trap |
| `c54g_long_add_traps` | `i64::MAX + 1` (or via `integer` post-Phase-2) traps |
| `c54g_long_div_by_zero_traps` | i64 div-by-zero traps |
| **`c54g_hybrid_nullcoalesce_discharges_overflow`** | `(i32::MAX * 2) ?? 42` returns 42 (G-hybrid preserves idiom) |
| **`c54g_hybrid_nullcoalesce_discharges_div_zero`** | `(a / 0) ?? -1` returns -1 |
| `c54g_nested_arith_inside_nullcoalesce` | `((a + b) * c) ?? 0` — only the immediate LHS of `??` gets Nullable; nested overflow traps |
| `c54g_nullable_input_traps` | `null + 1` in bare context traps with "nullable input" (not silent sentinel) |
| `c54g_explicit_i32_min_literal_preserved` | `x = -2_147_483_648` literal stays intact in storage (no trap) |
| `c54g_interp_and_native_agree_on_trap` | Same program under interpreter and native emits matching trap diagnostics |

Plus full workspace suite — any test that relied on the silent-sentinel
behaviour needs rewriting with explicit `??` or guards.  The `probes/`
fixtures (probes 01-06, 09) also get re-checked: under G-hybrid,
probes 01-06 and 09 now trap (good — the null can't be silently
produced in the first place).

## Risk: the `OP ?? default` detection

The detection's precision matters.  Too liberal (everything inside
`??` is Nullable) and users lose the trap where they expected it.
Too strict (only bare binary ops) and common shapes like
`(a * b + c) ?? 0` trap on the `+` unexpectedly.

**Policy**: only the **outermost arithmetic op whose result directly
becomes the LHS of `??`** gets the Nullable variant.  Sub-expressions
trap.  In `(a * b + c) ?? 0`, `+` is Nullable; `*` traps if it
overflows.  This forces users to split: `inner = a * b; (inner + c)
?? 0`.  Clear, predictable.

Document this rule in LOFT.md's "Arithmetic safety" section (Phase 6).

## Budget

**240-360 minutes** for implementation + parity.  If the compile-time
detection at the operator-codegen site exceeds 100 LoC, pivot to
pure G (documented as fallback — removes the Nullable codepath
entirely, breaks the `??` idiom for overflow, users migrate to
explicit guards).

Sub-phases:
- `01a-native-trap-parity.md` if native diagnostics diverge from
  interpreter.
- `01b-pure-g-fallback.md` if G-hybrid detection proves too complex.

## Deliverables

- `src/ops.rs` Trap + Nullable macros + functions.
- `src/fill.rs` handler registration.
- `src/parser/operators.rs` codegen detection.
- `src/state/mod.rs` trap landing.
- 14 named tests (un-ignored or new) passing on interpreter + native.
- Probes 01-06, 09 re-run under G-hybrid — all now trap instead of
  silent-null.
- QUALITY.md C54.G entry: Closed.
- LOFT.md "Arithmetic safety" section stub (full writeup in Phase 6).
