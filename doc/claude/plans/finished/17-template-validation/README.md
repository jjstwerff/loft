<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN17 — Bounded-generic / interface validation

## Status — DONE 2026-05-09

Validation matrix fully populated.  26 PASS cells in
`tests/template_matrix.rs` covering U1, U2, U3, U6, U7, U8 ×
B0/B1.O/B1.E/B1.A/B1.P/B2/B3/B4 under both interp and native via
the `cross_mode!` harness.  Reference for the validation-matrix
pattern lives in [`../../../TESTING.md`](../../../TESTING.md) §
"Validation matrices".  Per-cell layout in
[`00-matrix.md`](00-matrix.md) (kept as historical archaeology).

### Closeout commits 2026-05-09

| Commit | Phase |
|---|---|
| `ad854e4` | 00 — harness wiring (smoke + 4 PASS-pre cells) |
| `adcc6e6` | 01 — basic body + T-return baseline (6 cells) |
| `61cbf06` | 02 — tuple-of-T returns + @P237/P238 |
| `80d6b49` | 03 — Printable + vector + @P239 |
| `42f9739` | 04 — multi-bound + user-defined interface + @P240 |
| `4854b2d` | 05 — nested + two-T + @P241/P242 |

### Bug-yield outcome — 6 P-issues across 6 phases

Close to the predicted 5–10 range.  All 6 share a root-cause
family: generic-fn codegen emits DbRef-shaped ops for T-typed
values without substituting T's concrete type at monomorphisation.

| P-issue | Surface | Status |
|---|---|---|
| @P237 | Bound-supplied operator INSIDE a tuple constructor element | Closed 2026-05-09 (`Value::Tuple` recursion arm in `substitute_type_in_value`) |
| @P238 | Uniform `(T, T)` return with `T = text` (native only) | Closed 2026-05-09 (`tuple_text_to_string` flag handling) |
| @P239 | `for x in v` over `vector<T>` (consume side) | Open — interp SIGSEGV + native E0610 |
| @P240 | 2+ bound-operator locals + tuple return (cross-mode divergence; backend depends on body side-effects) | Open |
| @P241 | Building / pushing into `vector<T>` (construct side) | Open |
| @P242 | Format-string interpolation of T variable | Closed 2026-05-09 (`try_bound_to_text_call` helper in `parse_object`) |

@P239 / @P240 / @P241 likely close via a unified fix in the
second-pass generic body codegen that substitutes T into all
DbRef-shaped ops.

### Earlier closures (2026-05-04, before @PLAN17 phases ran)

The pre-flight survey predicted "Most B1×U1 cells fail".  Two fixes
that landed BEFORE @PLAN17 phases ran inverted that prediction:

- **(C) Built-in `to_text` impls.**  Six impls added to the end of
  `default/01_code.loft` for `integer` / `float` / `single` /
  `boolean` / `character` / `text`.  Built-in types now satisfy
  `Printable` automatically as the docs claimed.  Pinned by
  `tests/issues.rs::plan17_printable_integer_satisfies`.
- **(A) `substitute_type` recurses through `Type::Tuple`.**  Before
  the fix, `<T: Bound>(a: T, b: T) -> (T, T)` had parameters
  substituted to `i64` but the return type stayed `(DbRef, DbRef)`.
  Fix in `Parser::substitute_type` Tuple arm.  Pinned by
  `plan17_generic_tuple_return_with_annotation`.
- **(A) caveat — implicit type-inference of generic-tuple call
  results.**  Closed 2026-05-04 via `predict_generic_return_type`
  helper in `parser/mod.rs::call`.  First-pass pure-read prediction;
  second-pass full instantiation.  Pinned by
  `plan17_a_implicit_generic_tuple_type_inference`.
- **(B) Bounded-T method-call return-type inference (interpreter).**
  Closed 2026-05-04.  Two coordinated fixes — I7 dispatch +
  bound-resolution now run on both passes (was second-pass-only).
  Pinned by `plan17_b_bounded_method_return_type_propagates`.

The 26 PASS cells form a regression net catching future breakage
of working bounded-generic shapes.

## Goal (achieved)

Validate that loft's bounded-generic / interface system
(`<T: Bound>` constraints; structural satisfaction; static-dispatch
monomorphisation) round-trips every meaningful T-parameter usage
through every meaningful bound shape, with **interp/native
byte-identical stdout** asserted by the `cross_mode!` harness.

## See also

- [`../../../TESTING.md`](../../../TESTING.md) § "Validation
  matrices" — the matrix-binary pattern (cross_mode harness,
  ignored cells, PASS / FIX / CLOSED triage)
- [`../../INTERFACES.md`](../../../INTERFACES.md) — interface design
  + I1–I9 status
- [`../../PROBLEMS.md`](../../../PROBLEMS.md) — @P237 / @P238 / @P239 /
  @P240 / @P241 / @P242 + @P208 (the earlier closure)
- [`../../USER_FACING.md`](../../../USER_FACING.md) — implicit
  generic-tuple type inference workaround row
- [`../../DESIGN_DECISIONS.md`](../../../DESIGN_DECISIONS.md) —
  destination for any future CLOSED rows from this matrix
- [`../14-tuple-validation/00-matrix.md`](../14-tuple-validation/00-matrix.md)
  — peer matrix (donor template for cross_mode + matrix style)
- [`../15-closure-validation/`](../15-closure-validation)
  (SHIPPED 2026-05-12)
  / [`../../16-coroutine-validation/`](../16-coroutine-validation)
  / [`../../future/18-match-validation/`](../../29-match-validation)
  — peer matrix plans following the same shape
- `tests/template_matrix.rs` — the 26 PASS cells + harness wiring
- `default/01_code.loft` — stdlib interfaces (Ordered / Equatable /
  Addable / Numeric / Scalable / Printable + the 6 to_text impls
  added by closure (C))
