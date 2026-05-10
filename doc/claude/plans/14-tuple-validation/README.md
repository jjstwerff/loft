<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 14 — Tuple validation: full element × destination matrix

**Status: phases 00 + 01 + 02 + 07 shipped.  Phase 02 — full
matrix wiring done 2026-05-11; 5/5 e3 cells green
(e3_d1_nested_local, e3_d1_nested_deep, e3_d1_text_inside,
e3_d1_elem_elem_assign, e3_d2_nested_arg).  P212 panic fix
shipped 2026-05-04 (recursive `emit_tuple_put_ops`).  Two more
bugs filed + closed during the matrix wiring: P247 (nested-tuple
text move in format strings — closed via `nested_tuple_clone`
flag in dispatch.rs + work-ref `.clone()` in TupleGet emit +
Block-result wrap in format_text) and P248 (element-of-element
assignment — closed via `extract_nested_tuple_lhs` extractor
in expressions.rs + `Type::Tuple` arm in codegen.rs::TuplePut).
Also extended `src/generation/dispatch.rs::output_set` with a
recursive `tuple_has_text_leaf` check so
`((i64, String), (i64, String))` literal construction triggers
the inner `.to_string()` wrap (was: E0308).  Full tuple_matrix
suite: 22/22 green under `--ignored`.  Phases 03-06 untouched;
phase 08 still deferred.**

## Goal

Make tuples **fully typed and round-trip-correct** across every element
type the language admits, written into every storage destination the
language offers, validated by reading the value back and asserting it
unchanged **AND identical between the interpreter and `--native`**
(plus WASM where the cell is reachable from a browser context).

The driving question is no longer "does syntax X parse?" but
**"given a tuple of element type E written to destination D, does the
interpreter and the native build agree on the read-back value, byte
for byte?"**.  A green matrix means: every cell has a test that
(1) runs in interpreter mode, (2) runs in `--native` mode,
(3) cross-compares the two outputs and fails the test if they differ
even when both individually succeed.  A pass under one mode that
diverges from the other is a recorded failure, not a partial green.

A small number of cells today fail or are blocked behind known
limitations (T1.8c struct-ref move semantics, T1.11a field rejection).
This plan either fixes them in-place or documents the remaining gap as
a closed-by-decision so the matrix has no "unknown" cells.

## Why now, why a plan

Tuples shipped in 0.8.3 (T1.1–T1.7, T1.9–T1.11) but were validated
mostly against *mixed* element types in *one* destination (a local
variable bound by destructuring).  Plan-06 phase 9 is the first
consumer that pushes tuples through the worker boundary and the test
matrix is too thin to catch silent breakage.

A focused validation pass also makes the next dependent work cheap:
plan-06 phase 9 inherits a known-good baseline; T1.8a (tuple return
convention) lands on top of a regression net rather than blind; the
"why not in struct fields" question gets answered once and recorded in
DESIGN_DECISIONS.md instead of resurfacing every quarter.

## The matrix

Two axes.  Every cell is either a passing test, a fix-then-test, or a
documented closed-by-decision.

### Axis 1 — element types

| ID | Element type | Notes |
|---|---|---|
| E1 | **Basic scalar** — `integer`, `integer not null`, `float`, `boolean`, `character` | All by-value; no heap dep. |
| E2 | **Text** — `text`, `text not null` | Heap-allocated; `OpFreeRef` on scope exit. |
| E3 | **Nested tuple** — `((A, B), C)`, `(A, (B, C))`, `((A, B), (C, D))` | Recursive layout; `TupleGet(TupleGet(v,0),0)`. |
| E4 | **Closure** — `Type::Function(args, ret, dep)` | Closure record DbRef; ownership semantics per LIFETIME.md. |
| E5 | **Reference (object)** — `Type::Reference(struct, dep)` | DbRef into a struct store; T1.8c move-vs-copy lives here. |
| E6 | **Struct value** — interpretation of "structure" as the inline struct payload | Today loft has no by-value structs separate from `Reference`; this row is folded into E5 with a documented rationale. |
| E7 | **Vector / collection** — `vector<T>`, `hash`, `sorted`, `index` | Out of scope in 0.8.3 (Non-goals in TUPLES.md); revisit only if a real consumer appears. |

### Axis 2 — storage destinations

| ID | Destination | Today |
|---|---|---|
| D1 | **Local variable** — `t = (...)`; `let t: (A, B) = (...)` | Works; primary tested path |
| D2 | **Direct stack** — intermediate expressions: function argument (`fn(t)` where the tuple is built at the call site, not previously bound), tuple-typed function return (T1.8a), tuple in `match` subject built inline, tuple element built inline (`x = (a, b).0`), tuple inside an `if`/`else` arm result, tuple inside a `match` arm result | Partial — T1.8a deferred; inline-in-expression untested |
| D3 | **Struct field** — `struct S { t: (A, B) }`, write via `s.t = (...)` and read via `s.t.0` | Rejected today by T1.11a — this plan revisits the rejection |

### Cell key

Each cell `(Ei, Dj)` has one of:

- **PASS** — covered by an existing or new test.  Test reference recorded.
- **FIX** — implement + add test in the matching phase below.
- **CLOSED** — documented in DESIGN_DECISIONS.md with rationale.  No
  test, but a parser/check error keeps it surfaced.

The phase plans fill the matrix in order; the final phase asserts that
every cell has one of the three labels and the matrix table in this
README is up to date.

## Phase layout

| Phase | Element rows | Destination cols | Outcome |
|---|---|---|---|
| [00 — matrix + harness](00-matrix.md) | (table only) | (table only) | Frozen matrix; harness macro for "build tuple T, write to D, read back, assert equal"; baseline test inventory cross-referenced.  No production code change. |
| [01 — basic + text scalars](01-scalars.md) | E1, E2 | D1, D2 | Every E1×D1, E1×D2, E2×D1, E2×D2 cell green under interp/native/WASM. |
| [02 — nested tuples](02-nested.md) | E3 | D1, D2 | `((A,B), C)` and friends round-trip through every D1/D2 path; covers two- and three-deep nesting. |
| [03 — closure elements](03-closures.md) | E4 | D1, D2 | Tuple holds a closure; calling `t.0(...)` and storing/reloading via D1/D2 preserves captured state. |
| [04 — struct references](04-references.md) | E5 (= E6) | D1, D2 | Closes T1.8c (`tuple_struct_refs` un-ignores).  Decides move-vs-copy semantics, records in TUPLES.md, ships the fix.  Calls out that "struct value" (E6) is a synonym for E5 in current loft and stays a synonym; no new by-value variant introduced. |
| [05 — D3 decision: tuples in struct fields](05-struct-field.md) | (all element rows) | D3 | Either lift T1.11a (parser allows tuple as struct field, codegen lays it out as inline payload, scope-exit emits per-element cleanup) **or** record the closed-by-decision rationale in DESIGN_DECISIONS.md and keep the parser error.  The decision is made in this phase, not deferred.  If the lift path is chosen, every E1–E5 row gets a D3 cell test in a follow-up sub-phase 05b. |
| [06 — matrix freeze + doc](06-freeze.md) | — | — | Update TUPLES.md "known limitations" + "non-goals" tables to reflect the matrix; update PLANNING.md T1 entries; cross-reference closes T1.8c (and possibly T1.11a) in CHANGELOG_TECHNICAL.md. |
| [07 — P234 runtime: lifetime-bearing tuple returns](07-p234-runtime.md) | E2, E3, E4, E5 | function-return | `fn make() -> (Point, integer)` returns correctly under `--native` (today: `r.1=0`, `r.0.x=null`).  Unified gate: any tuple whose elements have lifetime concerns (Text, Reference, Vector, Enum-struct, keyed collections, or nested tuples containing those) routes through the existing `Reference(__tuple<…>)` synthetic struct.  Pure-value tuples (`(int,int)` etc.) keep the Rust ABI.  Supersedes T1.8a's text-tuple special-case machinery for the return path.  Lexer half (P234) shipped 2026-05-07; runtime half DONE 2026-05-08 (commit `d92d5d3`). |
| [08 — P234 runtime: LOCAL tuple-with-lifetime-concern variables](08-p234-runtime-locals.md) | E2, E3, E4, E5 | D1 (local var) | **Deferred 2026-05-08** after first-cut implementation hit friction with P189b's vector-of-tuple index access.  The rewrite needs P189b's index access to also return `Reference(__tuple<…>)` at the IR level — broader change than the original Phase 08 scope.  Phase 08 is a refactor for uniformity (NOT a bug fix); juice not worth the squeeze right now.  See the phase doc for what was tried and where the leverage point lives for whoever picks this up later. |
| 09 — A7.1 par tuple wide-return runtime | E1 | par worker return | **DONE 2026-05-08** by closing P236 (work-ref unification across If branches in `parser/control.rs::unify_if_branches_work_refs` + scopes.rs `returned_var(If)` recursion + Return-with-ret-var emission), then re-applying the size-based gate widen + recursive `rewrite_tail_tuple_to_synthetic_struct` for If/Block/Insert tails + destructure-path Reference(__tuple<…>) arm.  All 5 `par_tuple_return_*` canaries un-ignored and PASSING.  The synthetic-struct rewrite uses ONE shared work-ref via `rewrite_tail_tuple_with_work_ref`, mirroring the unification pattern P236 uses for struct returns. |

## Acceptance for the whole plan

- The matrix in [00-matrix.md](00-matrix.md) is fully populated — no
  cell is "unknown".
- Every PASS cell has a test reference and the test runs green under
  `make ci`, `cargo test --release --test native -- --test-threads=1`
  and the WASM fixture suite where the cell is browser-reachable.
- **Cross-mode equivalence is mandatory**: every cell test asserts
  `interp_output == native_output`.  An interp-only or native-only
  pass is a failure.  The harness macro in
  [00-matrix.md](00-matrix.md) implements this comparison once; phase
  tests use it without re-implementing the cross-check.
- Every FIX cell either landed a fix and moved to PASS, or migrated to
  CLOSED with a DESIGN_DECISIONS.md entry explaining why.
- TUPLES.md "known limitations" + "non-goals" tables match the final
  matrix exactly — no out-of-date "deferred" rows.
- The harness macro in 00-matrix.md is reused by every phase test —
  no per-phase re-invention.

## Out of scope (this plan)

- E7 (vector / collection elements in tuples).  Tuples-of-collections
  is a real shape but no current consumer needs it; if plan-06 phase 9
  surfaces one, file a follow-up under T1.x.
- Variadic tuples / generic tuple arities.  Tuples remain
  monomorphised at compile time.
- Single-element tuples and named tuple fields.  Existing non-goals
  carry over unchanged.
- Pattern bindings beyond what T1.9 already ships (no new `match`
  syntax — destructuring uses today's grammar).
- Plan-06 phase 9 itself.  Phase 9b/c/d/e consume this plan's
  validation as their baseline; this plan does not duplicate the par
  worker validation surface.

## Risks

| Risk | Mitigation |
|---|---|
| T1.8a (tuple return convention) is a prerequisite for several D2 cells but ships out of plan-06 phase 9a.  If 9a slips, this plan's D2 column has gaps. | Phase 01 D2 tests that need T1.8a are explicitly marked with `// requires T1.8a` and `#[ignore = "T1.8a"]`; the rest of the column ships independently.  When 9a lands, the ignore tag is removed in a one-line follow-up commit. |
| Closing T1.8c (struct-ref tuple elements) requires committing to either move or copy semantics.  Either choice has user-visible behaviour. | Phase 04 makes the decision in writing (TUPLES.md) before any test changes.  Move semantics is the working hypothesis (matches RHS-passing today); reviewer sign-off recorded in the phase plan's "Decision" section. |
| Lifting T1.11a (D3) introduces a new layout path through `definitions.rs::parse_field` and the struct-record writer; risk of touching a lot of unrelated code. | Phase 05 starts with a feasibility spike (parser change + 1 test); only after the spike compiles cleanly does the implementation phase open.  If the spike shows the change is invasive, the phase pivots to the CLOSED path with a DESIGN_DECISIONS.md entry. |
| Native and WASM modes diverge from interp on a specific cell (often format-string lifetime issues with text-element tuples). | Each cell test runs all three modes.  When a divergence appears, the test is added to PROBLEMS.md as a P-issue with mode tag before the phase ships; the matrix records "PASS (interp), FIX (native)". |
| Validation churn floods PROBLEMS.md with cell-specific bugs. | Group P-issues by phase; one P-id per element row per mode at most.  Don't file a new P for every failing cell — extend the row P-id with a checklist. |

## Cross-references

- [TUPLES.md](../../TUPLES.md) — current design doc; will be updated in
  phase 06.
- [PLANNING.md § T1](../../PLANNING.md) — T1.1–T1.11 entries; T1.8c
  closes in phase 04, T1.11a status decided in phase 05.
- [LIFETIME.md](../../LIFETIME.md) — closure / dep semantics for E4.
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) — destination for
  E7 and (possibly) D3 closed-by-decision entries.
- [06-typed-par/09-tuple-support.md](../06-typed-par/09-tuple-support.md)
  — downstream consumer; inherits this plan's regression net.
- `src/data.rs` — `Type::Tuple`, `Value::Tuple`, `TupleGet`,
  `TuplePut`, `element_size`, `element_offsets`, `owned_elements`.
- `src/parser/control.rs` — tuple match dispatch (T1.9).
- `src/parser/expressions.rs` — tuple literal, destructuring, element
  assign.
- `src/scopes.rs:578` — tuple scope-exit stub (T1.8c lives here).
- `tests/expressions.rs` — current tuple test surface (T1.1, T1.2,
  T1.5–T1.10 sections).
- `tests/parse_errors.rs` — T1.11 negative tests.
