# Phase 1 — Checked arithmetic (C54.G or C54.G′)

Status: **not started** — blocked by Phase 0 decision.

## Context

Semantic fix for C54's silent-wrong-result channels.  After Phase 1
lands, these behaviours change across all three backends (interpreter,
native, WASM):

| Today | After Phase 1 (G) | After Phase 1 (G′) |
|---|---|---|
| `(i32::MAX + 1)` returns `i32::MIN` (debug: abort) | runtime trap | null |
| `a / 0` returns `i32::MIN` sentinel | runtime trap | null |
| `a % 0` returns `i32::MIN` sentinel | runtime trap | null |
| Explicit `i32::MIN` literal preserved | preserved | preserved |

G′ additionally composes with `x = a * b ?? default` — the `??` catches
the null; G traps before `??` sees it.

The Phase 0 audit picks G vs G′.  This phase opens with the picked
option named in the title (e.g. "Phase 1 — Checked arithmetic via
C54.G′ null-on-overflow") and the audit result linked inline.

## Scope

- Interpreter (`src/fill.rs`): `Op{Add,Sub,Mul,Div,Mod}{Int,Long}`
  handlers switch to `checked_*` + branch-on-None.
- Native codegen (`src/generation/` + `src/codegen_runtime.rs`): emit
  checked arithmetic + explicit div-by-zero guard.  Same user-visible
  behaviour as interpreter.
- WASM path: confirm / port equivalent semantics.  WASM's 32-bit trap
  behaviour on overflow differs from x86_64's wrap — Phase 1 unifies.

## Out of scope for Phase 1

- Widening storage width (Phase 2).
- Adding `u32` (Phase 3).
- Deprecating `long` (Phase 4).
- Reclaiming opcodes (Phase 5).

## Test plan (from QUALITY.md § 518-521)

Un-ignore:
- `c54g_checked_add_traps_on_overflow` (rename to `_nulls_` if G′)
- `c54g_checked_mul_traps_on_overflow`
- `c54g_div_by_zero_traps`
- `c54g_mod_by_zero_traps`
- `c54g_interp_and_native_agree_on_trap`
- `c54g_explicit_i32_min_literal_preserved`

Plus per-backend WASM parity tests (new).

## Deliverables

- Patch to `src/fill.rs` arithmetic handlers.
- Patch to `src/codegen_runtime.rs` + `src/generation/*.rs` native
  paths.
- Patch to WASM emission if divergent today.
- Un-ignored tests.
- PROBLEMS.md entry: C54.G / G′ marked Closed (with the chosen option).

## Budget

**240 minutes for the implementation + parity verification.**  If the
native codegen path requires more than a one-line `checked_*`
substitution, escalate by opening `01a-native-checked-arith.md` as a
sub-phase.
