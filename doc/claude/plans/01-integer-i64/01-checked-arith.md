# Phase 1 — Checked arithmetic (C54.G / G′)

Status: **not started** — blocked by Phase 0 decision.  Title and
content update once Phase 0 commits the G / G′ choice.

## The surprise that makes this cheap

Loft already uses `checked_*` everywhere internally.  Look at
`src/ops.rs::checked_int!` (lines 33–52):

```rust
macro_rules! checked_int {
    ($result:expr, $fallback:expr) => {{
        match $result {
            Some(v) if v != i32::MIN => v,
            Some(_) => {
                debug_assert!(false, "arith produced i32::MIN");
                i32::MIN
            }
            None => {
                debug_assert!(false, "arith overflow");
                $fallback
            }
        }
    }};
}
```

Every arithmetic handler (`op_add_int`, `op_mul_int`, etc. at lines
531–571; `op_add_long` at 313; `_nn` variants at 367–405) already
routes through `checked_add` / `checked_mul` / `checked_sub` /
`checked_div` / `checked_rem`.  The ONLY remaining channel for
silent-wrong-result is the **`$fallback` arm that returns
`i32::MIN`** — i.e. overflow silently becomes the null sentinel.

So Phase 1's surface area is small and centralised: **change the
`None` and "result is sentinel" arms of the two macros
(`checked_int!` + `checked_long!`) from "return sentinel" to "trap"
(G) or "return null but lift the hole-gated coercions" (G′)**.

## Scope if Phase 0 picks G (expected)

### Primary edit: `src/ops.rs`

Change `checked_int!` and `checked_long!` macros:

```rust
// G-semantics — trap on overflow / sentinel result.
macro_rules! checked_int {
    ($result:expr, $context:expr) => {{
        match $result {
            Some(v) if v != i32::MIN => v,
            _ => panic!("arithmetic overflow or sentinel result in {}", $context),
        }
    }};
}
```

(Or `diagnostic!` / dedicated error type rather than panic; the
handler at the interpreter loop catches the trap and reports line:col
of the operator that tripped it.)

Update all callers (`op_add_int`, `op_sub_int`, `op_mul_int`,
`op_div_int`, `op_rem_int`, the `_long` siblings, and the `_nn`
variants).  The callers' up-front `v != i32::MIN` guards stay — they
detect nullable INPUTS; the macro now traps on a nullable OUTPUT.

### Division / modulo by zero

Today `op_div_int` at line 561 and `op_rem_int` at line 571 both
guard `v2 != 0` and return `i32::MIN` on divide-by-zero.  Change:
when `v2 == 0`, trap with a div-by-zero error (distinct diagnostic
from overflow).  Same for `_long` and `_nn` variants.

### Interpreter trap handler

The handler needs a landing spot — probably `src/state/mod.rs` where
the opcode dispatch loop lives.  When `checked_int!` (or its new
trap function) fires, the runtime walks the current PC back to the
source line/col via the existing debug-map infrastructure and emits:

```
arithmetic overflow at file:line:col (expr: a * b, lhs=..., rhs=...)
```

Then exit non-zero.  Do NOT continue execution.

### Native codegen path

**Auto-covered**.  Native codegen emits `ops::op_*` calls directly
(see `src/generation/mod.rs:474` — `writeln!(w, "use loft::ops;")`).
Changing the macro changes the behaviour of every emitted path.
No separate native edit needed.

### WASM path

**Auto-covered**.  WASM compiles to bytecode and executes via the
same interpreter (`src/state/mod.rs:14` — `use crate::fill::OPERATORS`).
No separate WASM edit.

## Scope if Phase 0 picks G′

G′ requires BOTH changing the macros AND tightening the `not null`
enforcement surface so nulls are caught at contract boundaries.
Since Phase 0 is expected to return "≥1 holes," this branch is
unlikely.  If it fires, open sub-phases for each hole identified in
Phase 0 and land them in sequence BEFORE the macro change.

## Test plan

Un-ignore (or create) and make pass on all three backends:

| Test | Purpose |
|---|---|
| `c54g_checked_add_traps_on_overflow` | `(i32::MAX + 1)` now traps, not returns MIN |
| `c54g_checked_sub_traps_on_underflow` | `(i32::MIN + 1) - 2` traps |
| `c54g_checked_mul_traps_on_overflow` | `(i32::MAX * 2)` traps |
| `c54g_checked_div_traps_on_abs_min` | `i32::MIN / -1` traps (the only `checked_div` overflow case) |
| `c54g_div_by_zero_traps` | `a / 0` traps with dedicated "division by zero" diagnostic |
| `c54g_mod_by_zero_traps` | `a % 0` traps |
| `c54g_long_add_traps_on_overflow` | i64 version |
| `c54g_long_div_by_zero_traps` | i64 version |
| `c54g_explicit_i32_min_literal_preserved` | `x = -2_147_483_648` — literal stays, not a trap |
| `c54g_nullable_input_not_a_trap` | `null + 1` — propagates null (doesn't trap), because the macro's up-front `v != i32::MIN` guards keep nullable-input semantics |
| `c54g_interp_and_native_agree_on_trap` | same program under interpreter and native emits matching diagnostics |
| `c54g_interp_and_wasm_agree_on_trap` | same program under interpreter and WASM emits matching diagnostics |

Plus full workspace suite (`scripts/find_problems.sh --bg --wait`) —
0 failures, confirming that existing stdlib / libs don't accidentally
rely on the silent-sentinel behaviour.  Any site that DOES rely on it
needs rewriting with `??` (post-G′) or explicit guards (pre-G′).

## Critical files

- `src/ops.rs` — primary edit (macros + per-op functions, lines
  33–571 for int, 251–480 for long + `_nn`).
- `src/state/mod.rs` — trap handler landing (opcode dispatch loop).
- `src/fill.rs` — confirm no additional arithmetic paths bypass
  `ops.rs` (they shouldn't, based on audit).
- `default/01_code.loft` — verify every arithmetic operator
  definition routes through the changed ops (they do today).

## Risks

1. **Stdlib / lib silent-sentinel dependence.**  If any loft code
   currently relies on `a * b` returning `i32::MIN` instead of
   trapping, the full suite will catch it.  Each such site is a
   trap under G but a bug regardless — fix at the site.
2. **Debug-build behaviour change.**  Today `checked_int!` has
   `debug_assert!(false, ...)` — debug already panics.  G just
   extends the behaviour to release.  No debug-vs-release drift.
3. **Diagnostic quality.**  Trap message must include operator
   location.  Without good diagnostics, users can't find the
   overflow site.  Budget 30 min for the diagnostic path.

## Budget

**240 minutes** for implementation + parity verification across
backends + full suite.  If a single stdlib site breaks under G,
budget `01a-stdlib-overflow-fix.md` as a sub-phase.

## Deliverables

- `src/ops.rs` macro rewrite + trap landings.
- `src/state/mod.rs` trap-handler hook.
- New / un-ignored tests.
- QUALITY.md C54 entry: G (or G′) marked Done.
- Initiative README phase-table status flip.
