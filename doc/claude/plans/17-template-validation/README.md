<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 17 — Bounded-generic / interface validation

**Status: phase 01 — 3 of 3 pre-flight bugs closed (interpreter); native (B) follow-up filed as P208.**

Closed (2026-05-04):
- **(C) built-in `to_text` impls.**  Six `to_text` impls added at
  the end of `default/01_code.loft` (after every OpFormat /
  OpAppend declaration so `"{self}"` interpolation resolves) —
  one each for `integer`, `float`, `single`, `boolean`,
  `character`, and `text`.  Built-in types now satisfy
  `Printable` automatically as the docs claimed.  Pinned by
  `plan17_printable_integer_satisfies` in `tests/issues.rs`.
- **(A) `substitute_type` recurses through `Type::Tuple`.**
  Before the fix, a `<T: Bound>(a: T, b: T) -> (T, T)` had
  parameters substituted to `i64` but the return type stayed
  `(DbRef, DbRef)` (parametric T form), causing native E0308.
  Fix: `Type::Tuple(elems)` arm in `Parser::substitute_type`
  maps each element through the substitution.  Pinned by
  `plan17_generic_tuple_return_with_annotation`.  **Caveat:** the
  fix requires an **explicit element-type annotation** on the
  receiving variable (`t: (integer, integer) = …`).  Implicit
  type-inference from a generic-call result doesn't yet
  propagate the substituted return type to the receiving slot,
  so `t = min_max(7, 3); t.0` still trips the parser.  Tracked
  as the phase-01 follow-up alongside (B) — likely the same
  root cause.

- **(B) bounded-T method-call return type inference (interpreter).**
  *Closed 2026-05-04.*  Two coordinated fixes:
  - `src/parser/fields.rs` — the I7 bounded-method-dispatch
    path now runs on **both passes** (was second-pass-only).  The
    first-pass branch was returning `Type::Unknown(0)` after
    consuming args; the receiving variable then stayed Unknown
    because `change_var_type` is a no-op for Unknown assignments,
    and downstream operators like `s + "!"` rejected.
  - `src/parser/definitions.rs` — bound resolution and t-stub
    creation now run on **both passes** (was second-pass-only).
    Forward-decl tolerated via silent skip when the interface
    isn't yet known on first pass; the second pass catches
    genuinely-unknown bounds with the error.  Necessary for the
    I7 dispatch's `find_fn(stub_name)` and `has_bound_for_method`
    queries to succeed on first pass.
  Pinned by `plan17_b_bounded_method_return_type_propagates` in
  `tests/issues.rs` (now PASS).  Originally hypothesised as
  shared root cause with (A) caveat — turned out to be DIFFERENT
  root cause; (A) is in the `try_generic_instantiation` path
  (free-fn calls), not the method-dispatch path.

Open:
- **(B-native) P208 — native E0282 on the same shape.**  After
  the parser fix landed, the `--native` build still rejects
  `<T: Printable>(x: T) -> text { x.to_text() + "!" }` with
  "type annotations needed".  Generated code wraps the inner
  `Value::Return` text-result in `stores.scratch.push(...)` AND
  then wraps the surrounding expression in another scratch-push;
  the inner `return` makes the outer push unreachable, so rustc
  can't infer the unreachable expression's type.  Filed as
  P208 in PROBLEMS.md.  Likely fix in
  `src/generation/emit.rs::Value::Return` text-wrapping path:
  detect-and-suppress the redundant wrap when the inner is a
  returning text expression.  Workaround: use `--interpret`.

- **(A) caveat — implicit type-inference of generic-tuple call
  results.**  `t = min_max(7, 3); t.0` still rejects with
  "Expect token ;".  My initial hypothesis that this shared bug
  B's root cause turned out wrong; (A) is in the
  `try_generic_instantiation` path (free-fn calls), gated on
  `!self.first_pass` at `parser/mod.rs:1065` — same first-vs-
  second-pass timing pattern as B but in a different code
  path.  The fix shape is similar (run on both passes) but
  needs care because `try_generic_instantiation` creates a new
  monomorphised function definition, not just dispatches a
  call.  Phase 01 follow-up.

## Goal

Validate that loft's bounded-generic / interface system
(`<T: Bound>` constraints; structural satisfaction; static-dispatch
monomorphisation) round-trips every meaningful **T-parameter usage**
through every meaningful **bound shape**, with **interp/native
byte-identical stdout** asserted by the cross-mode harness from
plan-14 phase 00.

The driving question: "given a bounded generic `fn f<T: Bound>(...)`
used in shape U with bound shape B, does the monomorphised behaviour
agree between interp and native, and does it accept the types it
should accept and reject the types it shouldn't?"

This plan inherits all infrastructure from plan-14: the `cross_mode!`
macro, the `tests/common/cross_mode.rs` harness, the `#[ignore]`
discipline, P-id rules.  The only new artefacts are the
template-specific matrix, phase plans, and cell tests in a new
`tests/template_matrix.rs` binary.

## Why now — exceptionally high known bug rate

Pre-flight survey (2026-05-04, 5 quick tests) found **3 distinct
bug categories in 5 minutes**:

| Shape tested | Result | Failure mode |
|---|---|---|
| `<T: Ordered>(a: T, b: T) -> (T, T)` — bounded-generic returning tuple | ❌ both backends | Parser rejects tuple element access (`t.0`), tuple destructure (`(lo, hi) =`), and format-string tuple access (`{t.0}`) on bounded-generic results |
| `<T: Printable>(x: T) -> text { x.to_text() ++ "!" }` — body uses `++` to build text | ❌ both backends | "No matching operator '+' on 'unknown(0)' and 'text'" — type inference inside the generic body doesn't propagate `to_text(): text` from the `Printable` bound |
| `<T: Printable>(v: vector<T>)` called with `vector<integer>` | ❌ both backends | "'integer' does not satisfy interface 'Printable': missing to_text" — the loft-write skill and INTERFACES.md both claim built-in types satisfy `Printable` automatically; in fact the stdlib (`default/01_code.loft`) has no `to_text` impl for `integer` |
| `<T: Ordered + Equatable>(a: T, b: T) -> integer` — multi-bound | ✅ both | Works |
| `dbl<T: Addable>(x: T) -> T { x + x }` — multi-monomorphisation | ✅ both | Works |

3/5 fail rate is unprecedented compared to the other matrices:
plan-14 phase 01 found 2 P-issues in 15 cells (13% rate); plan-17's
pre-flight hit 60%.  These aren't variants of one bug — they're
three subsystem-distinct issues:
- **Parser** — generic-tuple element access path missing.
- **Type inference** — bound-supplied method signatures don't
  propagate.
- **Satisfaction** — auto-satisfaction for built-ins doesn't match
  the documented contract (or the doc is wrong).

Each will likely surface several variants during the matrix run.
Conservative estimate: 5–10 P-issues filed and fixed across the
plan, plus 2–3 doc corrections.

## The matrix

Two axes.  Every cell is `PASS:test_name`, `FIX:phase`, or
`CLOSED:reason` with a DESIGN_DECISIONS.md cross-reference.

### Axis 1 — T-parameter usage

| ID | T-usage shape | Notes |
|---|---|---|
| U1 | **Body operation on T** — `fn f<T: B>(x: T) -> ... { x.method(); x + x; ... }` | Type inference in generic body must propagate `B`'s method/operator signatures. |
| U2 | **T as return type** — `fn f<T: B>(x: T) -> T` | Standard generic-return path; P205 closed the text-return variant. |
| U3 | **Tuple of T as return** — `-> (T, T)`, `-> (T, U)` mixed | Pre-flight failure — parser path. |
| U4 | **T as struct field** — `struct Box<T> { val: T }` | Whether loft supports generic struct types is itself a question; baseline check. |
| U5 | **Vector of T as input** — `fn f<T: B>(v: vector<T>)` | Pre-flight failure — satisfaction check on built-ins. |
| U6 | **Vector of T as output** — `-> vector<T>` | Less-tested path. |
| U7 | **Multi-T return** — `fn f<T: B>(x: T) -> (T, T, T)` etc. | Cross-cuts U3 with arity. |
| U8 | **T inside inline expression / format-string** — `print("{x}")` | Pre-flight failure on tuples; check non-tuple variants. |

### Axis 2 — bound shape

| ID | Bound shape | Notes |
|---|---|---|
| B0 | **No bound** — `<T>` | Already mostly works for opaque-T uses; baseline. |
| B1 | **Single stdlib bound** — `<T: Ordered>`, `<T: Equatable>`, `<T: Addable>`, `<T: Printable>` | Each bound has its own row of cells; sub-axis. |
| B2 | **Multiple bounds** — `<T: Ordered + Equatable>` | Pre-flight ✅. |
| B3 | **User-defined interface** — `interface MyShape { fn area(self: Self) -> float }` and `<T: MyShape>` | Op-sugar inside user interfaces (I3.1). |
| B4 | **Op-sugar** — `<T: Addable>` and using `+` (not `OpAdd`) | Pre-flight ✅ (the `dbl<T: Addable>(x: T) -> T { x + x }` test). |
| B5 | **Two type parameters** — `<A: B1, B: B2>(a: A, b: B)` | Stretch; verify monomorphisation handles two-axis substitution. |

### Cell key

Each cell `(Ui, Bj)` has one of:
- **PASS** — covered by an existing or new test.
- **FIX** — implement + add test in the matching phase below.
- **CLOSED** — design decision (e.g. U4 if loft doesn't support
  generic struct types).

## Phase layout

| Phase | T-usage rows | Bound cols | Outcome |
|---|---|---|---|
| [00 — matrix freeze + harness wiring](00-matrix.md) | (table) | (table) | Frozen matrix; new `tests/template_matrix.rs` binary; reuses `cross_mode!`.  Smoke test exercises the harness against a known-passing cell.  No production change. |
| 01 — basic body + T-return baseline | U1, U2 | B0, B1 | Most B1×U1 cells today fail (type inference gap); fix the inference path so bound-supplied method signatures propagate.  P205 already closed B1×U2 for text; verify the rest of B1×U2 cells. |
| 02 — tuple-of-T returns | U3, U7 | B1, B2 | Active risk — pre-flight surfaced the parser breakage.  Either fix the generic-tuple element-access path OR document a CLOSED row with rationale and emit a clear diagnostic instead of "Expect token ;".  Most likely fix: extend the existing tuple-element parser to recognise `Type::Unknown(<bounded>)` as a Type::Tuple-shaped expression. |
| 03 — Printable / built-in satisfaction | U1, U5, U6 | B1 (Printable specifically) | Decide: does the stdlib add `to_text` impls for `integer`/`float`/`text`/`character` (matching the doc claim), OR does the doc retract the claim?  If add: `default/01_code.loft` gains four impls; INTERFACES.md and the loft-write skill stay as-is.  If retract: docs corrected; users must explicitly impl `to_text` on built-in newtype wrappers.  Decision recorded in this phase's plan file before any code lands. |
| 04 — multi-bound + user-defined interface | B2, B3 | U1–U6 | Multi-bound mostly works (pre-flight); user-defined interfaces less tested.  Op-sugar inside user interfaces (I3.1) gets a row of cells. |
| 05 — op-sugar + two-parameter generics | B4, B5 | U1–U7 | Op-sugar (`+` from Addable) works; verify it works inside more complex shapes (in tuple element, in vector element).  Two-parameter generics (`<A: B1, B: B2>`) is a stretch; unclear if loft monomorphisation handles it. |
| 06 — freeze + doc | — | — | Update INTERFACES.md (close any remaining out-of-scope items; correct any doc claims), the loft-write skill (built-in satisfaction matrix), PLANNING.md I-tier entries.  Move plan to `finished/`. |

## Acceptance for the whole plan

- Matrix in [00-matrix.md](00-matrix.md) fully populated — no
  "unknown" cells.
- Every PASS cell has a `cross_mode!`-driven test in
  `tests/template_matrix.rs` that runs green under
  `cargo test --release --test template_matrix -- --ignored`.
- Cross-mode equivalence is mandatory.
- Every CLOSED cell has a corresponding negative test
  asserting the diagnostic stays stable.
- INTERFACES.md and loft-write skill claims about built-in
  satisfaction match the actual stdlib state.

## Out of scope

- **Dynamic dispatch / interface values** — already non-goal per
  INTERFACES.md.  CLOSED row in the matrix.
- **Composite interfaces / interface inheritance** — already
  non-goal.  CLOSED row.
- **Associated types** — already non-goal.  CLOSED row.
- **Default method implementations on interfaces** — already
  non-goal.  CLOSED row.
- **Variance** — loft's static dispatch sidesteps this; non-goal.

## Risks

| Risk | Mitigation |
|---|---|
| Phase 02 (tuple-of-T returns) reveals a deep parser refactor (the bounded-generic tuple-element path goes through different lookup logic than concrete tuples) | Phase 02 starts with a feasibility spike — one minimal test, parser instrumentation, identify the lookup divergence.  If the refactor is >3 days of work, the phase pivots to CLOSED with a clear diagnostic + DESIGN_DECISIONS.md entry pointing at "use a named struct return instead" workaround. |
| Phase 03 decision (add stdlib impls vs retract doc claim) splits opinion | Decision section in 03 is filled in BEFORE any code lands; reviewer sign-off recorded.  Either path is acceptable; the bad outcome is no-decision-made. |
| Built-in `to_text` impls (if added) require updating every format-string call site that already prints integers/floats | They wouldn't — `print("{x}")` uses the format protocol, not the `Printable.to_text` interface.  The new impls live alongside the format protocol; calls of the form `obj.to_text()` (via a generic) start working without affecting concrete-type prints. |
| Op-sugar phase 05 surfaces complex interactions with bounded type-narrowing (e.g. `<T: Addable>` with `T = i32` narrow integer) | The narrow-integer rules already exist in the type system (P184); op-sugar dispatch should pass through them.  If it doesn't, file a P-issue and stay phase-bounded — the matrix only tests, the fix lands separately if non-trivial. |
| Plan-17 cells balloon the test binary count | Each cell is `#[ignore]`d under the same "tuple_matrix"-equivalent tag.  Default `cargo test` skips them.  Run with `cargo test --release --test template_matrix -- --ignored`. |

## Cross-references

- [INTERFACES.md](../../INTERFACES.md) — interface design + I1-I9
  status.
- [PLANNING.md § I — Interfaces](../../PLANNING.md) — historical
  tracking.
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) — destination
  for any non-goal CLOSED entries surfaced during the plan.
- [plan-14 phase 00](../14-tuple-validation/00-matrix.md) — donor
  template for the cross-mode harness + matrix style.
- [plan-15 closure validation](../15-closure-validation/README.md)
  — peer plan with the same matrix shape.
- [plan-16 coroutine validation](../16-coroutine-validation/README.md)
  — peer plan.
- `default/01_code.loft` — stdlib interfaces (Ordered, Equatable,
  Addable, Numeric, Scalable, Printable).
- `tests/scripts/86-interfaces.loft` — existing interfaces script
  test (covers the generics + for-loop + struct-vector case).
