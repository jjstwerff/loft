<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Inline-lift safety — initiative

## Status — DONE 2026-04-18

Reference for the inline-lift invariant + the dep-merge gate that
implements it lives in
[`../../../LIFETIME.md`](../../../LIFETIME.md) §
"Inline-lift safety — the `OpCopyRecord | 0x8000` invariant"
(line ~642): gate mechanism, dep merging from return expressions
(`text_return` / `ref_return` helpers), lock-bracket second line
of defence, known trade-offs (owned-fallback leak on mixed-return,
WASM unconditional-clear, Vector mid-body deferral), history.

This file is the closure record for the initiative; the per-phase
plan files in this directory remain as historical archaeology.

### Phase outcome

| File | Phase | Outcome |
|---|---|---|
| `00-p181-diagnostic.md` | Variant inventory + bug-site confirmation + fix-direction pick | **Done** — Option B chosen |
| `01-p181-fix.md` | Gate `0x8000` on callee return-dep at two emission sites (`gen_set_first_ref_call_copy`, `generate_set` reassignment path) | **Done** — covers consistent-view callees |
| `01b-return-dep-inference.md` | Teach return-dep inference to UNION over return paths so mixed-return accessors get tagged borrowed | **Done** — Reference + Enum arms.  Vector arm deferred (would promote globals/locals to hidden ref args, breaking callers — see LIFETIME.md trade-off #3) |
| `02-audit-adjacent-sites.md` | Audit every `OpCopyRecord` emission + cross-ref P143/P150/P152/P155 | **Done** — clean, no new bugs surfaced |
| `02a-multi-inline-lifts.md` | Multi-call shapes in one expression | **Closed** — variants 17, 20, 22 PASS post-Phase-1b (closed by the dep-merge fix, not a separate gate) |
| `02b-native-codegen-emission.md` | Audit `src/generation/dispatch.rs` direct-emission sites | **Closed** — subsumed by Phase 2 audit (native sites never set 0x8000) |
| `01c-dynamic-dispatch.md` | CallRef (fn-ref / interface-method) safe-default | **Closed** — variants 08 + 21 PASS; hidden-ref-arg mechanism protects this path |
| `01d-owned-with-aliasing.md` | `Value::Var`-only lock filter for OWNED-return callees aliasing an expression arg | **Closed** — variant 09 PASSES; no reachable shape found |
| `03-spec.md` | Document the inline-lift invariant as a language commitment | **Done** — section added to `LIFETIME.md` |

## Snippet inventory — regression-test net

Fixtures in `snippets/` probe specific expression shapes.  Status
below is post-closure; re-run any snippet to re-confirm.

| # | File | Shape | Status |
|---|---|---|---|
| 01  | `01_field_access.loft`         | `{f(o.x).n}` format-interp (consistent view)         | **PASS** (was SIGSEGV pre-Phase-1) |
| 01b | `01b_without_lift.loft`        | Same body, no inline-lift (control)                   | PASS |
| 01c | `01c_inline_only.loft`         | Minimal: one inline-lift line                         | **PASS** (was SIGSEGV pre-Phase-1) |
| 01d | `01d_var_arg_inline.loft`      | Inline-lift, Var arg (control)                        | PASS |
| 04  | `04_owned_control.loft`        | Owned-result callee, inline-lift (control)            | PASS |
| 07  | `07_mixed_return.loft`         | Mixed-return callee (view + owned fallback)           | **PASS** (was SIGSEGV pre-Phase-1b) |
| 08  | `08_dynamic_dispatch.loft`     | fn-ref call with borrowed-view result                 | PASS |
| 09  | `09_owned_with_aliasing.loft`  | Owned-return callee mutating an expression arg        | PASS |
| 10  | `10_inline_in_condition.loft`  | Single inline-lift in `if` condition                  | PASS |
| 11  | `11_inline_in_return.loft`     | Single inline-lift in `return expr`                   | PASS |
| 12  | `12_inline_in_for.loft`        | Single inline-lift as for-iterator                    | PASS |
| 13  | `13_inline_in_assign.loft`     | Single inline-lift on assignment RHS / `+=`           | PASS |
| 14  | `14_mixed_return_various_contexts.loft` | Mixed-return in condition / assign (single calls)  | PASS |
| 15  | `15_println_format.loft`       | SINGLE mixed-return inline in `println` format        | PASS |
| 16  | `16_single_call_assert.loft`   | SINGLE mixed-return in assert cond, literal msg       | PASS |
| 17  | `17_println_two_calls.loft`    | TWO mixed-return inline calls in one `println` fmt    | **PASS** (was SIGSEGV pre-Phase-1b) |
| 18  | `18_tuple_destructure.loft`    | Tuple destructure of two struct-returning calls       | PASS |
| 19  | `19_vector_mixed_return.loft`  | Vector mixed-return inline in `println` format        | PASS |
| 20  | `20_vector_two_calls_both_branches.loft` | Vector mixed-return, TWO calls hitting both branches | PASS |
| 21  | `21_fnref_mixed_return.loft`   | fn-ref dispatch to mixed-return, inline + owned fallback | PASS |
| 22  | `22_chained_mixed_returns.loft` | Chained `get_inner(h.w, i).n` with transitive mixed deps | PASS |

### Loose-end validation (2026-04-18)

After closing the initiative, four follow-up probes verified the
"likely-closed" phases and the Vector deferral:

- **Variant 19** — Vector-returning mixed callee inline in `println`
  PASSES.  The callee's signature gets `[empty]` dep via the existing
  hidden-ref-arg mechanism in `block_result`'s tail-side Vector arm;
  the Phase 1 codegen gate fires, `0x8000` clears, no corruption.
  **Vectors are safe by construction**; the Vector-arm skip in
  Phase 1b is validated.
- **Variant 20** — Same shape, TWO calls (one view, one owned
  fallback).  PASSES.
- **Variant 21** — fn-ref dispatch to a Reference-returning mixed
  callee.  PASSES.  No `OpCopyRecord | 0x8000` emission path exists
  for fn-ref dispatch (uses hidden-ref-arg mechanism).  Phase 1c
  has no reachable corruption class.
- **Variant 22** — Chained `get_inner(h.w, i).n` with transitive
  dep propagation.  PASSES.  Phase 2a has no reachable corruption
  class for chained / nested mixed-return calls.

## Provenance

- **Surfaced**: moros_sim walkable-editor Step 21 uncovered P181 in
  `lib/moros_sim/tests/picking.loft::test_edit_at_hex_raise`.
- **Root cause identified**: session on branch
  `moros_walk_steps_9_10` at commit `65a174c` (P181 entry in
  PROBLEMS.md).
- **Pre-fix workaround**: hoist inline struct-returning calls into
  locals before referencing in format strings / chained accessors
  (no longer needed for the shapes covered above).

## See also

- [`../../../LIFETIME.md`](../../../LIFETIME.md) — reference for
  the inline-lift invariant (gate, dep merging, lock-bracket,
  trade-offs, history)
- [`../../PROBLEMS.md`](../../../PROBLEMS.md) — P181 entry
- `src/state/codegen.rs::gen_set_first_ref_call_copy` /
  `generate_set` — the two gated emission sites
- `src/parser/control.rs::text_return` /
  `src/parser/control.rs::ref_return` — dep-merge helpers
  (LIFETIME.md table at line ~686)
- `tests/lib/p181_*.loft` + `tests/issues.rs` regression-fixture
  pins (the snippets above are the per-shape probes; the lib
  fixtures are the CI-locked regression net)
