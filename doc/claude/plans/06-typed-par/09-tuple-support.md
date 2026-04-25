<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 9 — Tuple support for `par`

**Status: open**

## Goal

`par(...)` must accept tuples on both sides of the type spectrum:

- **Input:** `vector<(T, U)>` (and any tuple arity) iterable just
  like `vector<Struct>`.
- **Output:** worker fn returning `(T, U)` (any arity, any element
  type) collected into `vector<(T, U)>`.
- **Fused for-loop:** `for (a, b) in pairs par(r = work(a, b), 4)`
  destructures the tuple in the loop binding, exactly like the
  sequential form.

After phase 9, the rule "anywhere you can write `fn process(x: T)
-> U`, you can also write `par(xs, process, N)`" extends to tuples
without a footnote.

## Why this is plan-06 scope, not 1.1+

The previous PLANNING.md placement put T1.8a (function tuple
return convention) in a 0.8.3 follow-up bucket and never connected
it to par.  As a result D11b had to mark tuples as
"✅ when tuples land" — a placeholder that quietly meant
"par will not accept tuple returns even after plan-06 lands".

Plan-06's promise is **full type coverage of par**.  A redesigned
runtime that still rejects `(integer, integer)` returns is a
half-finished redesign.  T1.1–T1.7 already shipped (0.8.3); the
only missing piece for tuple-shaped par is T1.8a's return
convention plus the tuple-as-vector-element handling that the
store-typed pipeline gives us anyway.

Effort lift compared to "do T1.8a in 1.1+ and revisit par later"
is small — the per-worker output Store from phase 1 stores a
tuple record exactly the same way it stores a struct record
(contiguous bytes at element offsets).  The work is **plumbing T1.8a's
caller-pre-allocated-slot convention into the worker call site**,
plus parser updates so the typed surface accepts `Type::Tuple`.

## Architecture

### Tuples as records

Tuples are already represented as contiguous-byte records (T1.1):
`(integer, text)` is 16 bytes — 8 for the integer at offset 0, 8
for the text DbRef at offset 8.  `Type::Tuple(Vec<Type>)` carries
element types; `element_size` and `element_offsets` helpers in
`data.rs` give layout.

The store-typed pipeline (phase 1) writes worker results into a
per-worker output Store via ordinary `OpPut*` opcodes.  A tuple
return is just N `OpPut*` opcodes at element offsets — same shape
as a struct.  No new runtime concept.

### Tuple inputs

`for (a, b) in pairs { … }` already destructures tuples in
sequential loops (T1.2/T1.3).  The fused `for (a, b) in pairs
par(r = work(a, b), 4) { … }` form needs:

1. Parser: accept tuple-pattern loop binding before the `par(…)`
   suffix (today rejects with "expected identifier").
2. Worker codegen: pass the tuple as a single record to the worker
   fn; the worker can either take `(T, U)` or destructure on entry.

### Tuple returns — T1.8a convention

T1.8a (PLANNING.md:439) is the missing piece: a function declared
`-> (A, B)` writes its return into the caller's pre-allocated
tuple slot.  For par, "the caller" is the worker dispatch
trampoline; the pre-allocated slot is the worker's output Store
record at the next free offset.

Concrete shape:

```rust
// pseudo: per-worker output Store layout for a (integer, text) result
//   record_offset 0: i64    (first element)
//   record_offset 8: DbRef  (second element, points into output store)
//
// The worker calls work(x), which today returns into a temp slot;
// after T1.8a, work(x) writes directly into output_store[record_offset..].
```

The text element is allocated inside the worker's output Store
(phase 1's per-worker Store covers all worker-side allocations);
the rebase pass (phase 2) translates the DbRef when stitching.

## Per-commit landing plan

### 9a — T1.8a function-return convention (prerequisite)

Land T1.8a as a standalone change before wiring it into par.
Self-contained ~200 LOC; lives outside par scope.

- `Value::ReturnTuple` IR variant.
- `OpReturnTuple(size)` that copies callee stack to caller's
  pre-allocated slot.
- Codegen at the call site: allocate tuple slot, pass slot pointer
  via the call frame, generate `OpReturnTuple` at the return.
- Tests: `tests/tuples.rs` adds `tuple_return_int_int`,
  `tuple_return_int_text`, `tuple_return_struct_text`.
- Closes T1.8a in PLANNING.md; un-blocks phase 9b.

### 9b — Tuple-element vector inputs to par

- Parser-side: typed `parallel_for(input: vector<Type::Tuple(_)>,
  fn, threads) -> vector<U>` accepts tuple element types
  (today rejects with "primitive-element input gives garbage" — the
  same G2 gap, but tuple-flavoured).
- Codegen: tuple element stride from `Type::Tuple::element_size`;
  no special case beyond what struct inputs already use.
- Test (un-ignore): a new `par_tuple_input_*` canary in
  `tests/threading_chars.rs`.

### 9c — Tuple returns from par workers

- Codegen: worker fn return slot is a tuple record in the per-worker
  output Store; uses T1.8a's `OpReturnTuple` to write.
- Stitch (phase 2's rebase) walks tuple records like struct records
  — `Type::Tuple` exposes the same `owned_elements` info that
  struct types do, so the rebase pass needs no tuple-specific code.
- Test (un-ignore): `par_tuple_return_int_int`,
  `par_tuple_return_int_text`, `par_tuple_return_struct_text`.

### 9d — Fused `for (a, b) in pairs par(...) { … }`

- Parser: accept tuple destructuring in the loop binding for the
  fused form.
- Scope analysis: the destructured names (`a`, `b`) are slot-bound
  locals inside the worker; same shape as the sequential loop.
- Test: `for_tuple_par_destructure` in
  `tests/scripts/22-threading.loft`.

### 9e — Update D11b + bench + doc

- DESIGN.md D11a + D11b — replace placeholder rows with
  ✅ first-class tuple support; cross-reference phase 9.
- `bench/11_par/bench.loft` — add a tuple-return benchmark variant
  (worker returns `(integer, integer)`; stitched into
  `vector<(integer, integer)>`).  Compare against today's
  struct-return shape.
- THREADING.md — Plan-06 phase 0 baseline section gains a tuple
  row.
- CHANGELOG entry: "par accepts tuples on both input and output".

## Test inventory

Phase 9 closes these `#[ignore]`d canaries from
`tests/threading_chars.rs` (added in this phase):

| Canary | Closed by |
|---|---|
| `par_tuple_input_int_int` | 9b |
| `par_tuple_input_int_text` | 9b |
| `par_tuple_return_int_int` | 9c |
| `par_tuple_return_int_text` | 9c |
| `par_tuple_return_struct_text` | 9c |
| `par_tuple_destructure_in_for` | 9d |

## Loft-side prerequisites

- **Phase 1 (per-worker output Store) must land first** — the
  per-worker Store is what receives tuple-element records.
- **Phase 2 (stitch via rebase) must land first** — the rebase
  pass handles tuple-internal DbRef offsets the same way it handles
  struct-internal ones.
- **T1.8a (function-return convention)** — landed in 9a as a
  prerequisite.

## Acceptance criteria

- All six canaries in the test inventory un-ignored and green.
- DESIGN.md D11a + D11b show ✅ first-class for tuple input and
  return (no "when tuples land" caveat).
- `bench/11_par/bench.loft`'s tuple-return variant runs across
  loft-interp, loft-native, loft-wasm with no error and produces
  identical results to the struct-return variant.
- `tests/scripts/22-threading.loft` includes a fused-for tuple
  destructure test that passes under interp and native.
- `make ci` green.

## Risks

| Risk | Mitigation |
|---|---|
| T1.8a's caller-pre-allocated-slot convention conflicts with the worker dispatch trampoline | Keep T1.8a's slot-pointer parameter convention symmetric with how struct returns work today; the trampoline already passes a result-slot pointer |
| Tuple-with-text return needs DbRef rebase across tuple element offsets | Phase 2's rebase already walks `owned_elements`; tuples expose the same accessor — no new rebase code |
| Tuple elements with nested vectors / hashes | Out of scope for phase 9 — covered by D11a "nested vector input" canary which closes in phase 4; tuple elements are either primitive, text, or DbRef in plan-06 |
| Bench data shapes change between phases (struct → tuple) | Keep both shapes in `bench/11_par/`; mark which is the canonical apples-to-apples reference |

## Out of scope

- **Heterogeneous tuple arities per worker** — every worker
  returns the same tuple type (today's same-type-per-worker rule
  unchanged).
- **Generic tuple types in worker fn signatures** — bounded
  generics over tuples is a future feature; plan-06 accepts
  monomorphised tuple types only.
- **Tuple-of-tuples return** — workers returning `((A, B), (C,
  D))` deferred; nested-collection canaries cover the broader
  shape and close in phase 4.

## Cross-references

- [README.md](README.md) — plan-06 ladder, phase 9 added.
- [DESIGN.md § D11](DESIGN.md) — type spectrum; tuples promoted
  to first-class.
- [01-output-store.md](01-output-store.md) — per-worker Store
  receives tuple records identically to struct records.
- [02-stitch-not-copy.md](02-stitch-not-copy.md) — rebase walks
  tuple `owned_elements` like struct ones.
- [TUPLES.md](../../TUPLES.md) — tuple feature design (T1).
- [PLANNING.md § T1.8](../../PLANNING.md) — T1.8a / b / c
  remaining work; 9a closes T1.8a.
- `src/data.rs` — `Type::Tuple`, `element_size`,
  `element_offsets`, `owned_elements` already exist (T1.1).
