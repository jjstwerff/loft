<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 9 — Tuple support for `par`

**Status: 9a closed (2026-05-04); 9b/9c/9d/9e open**

> **9a closed.**  Per the original design, 9a was the standalone
> prerequisite that gated 9b–9e.  The actual-error survey
> (@PLAN14 phase 01 follow-up) showed 9a's design was over-engineered
> — the basic `-> (A, B)` return convention already worked end-to-end;
> only tuple-of-text returns under `--native` had a real bug, fixed
> by recursing `rust_type` with `Context::Variable` for tuple
> elements in `Context::Result` plus `tuple_text_to_string` flag
> wiring at the return site.  See PLANNING.md § T1.8a for the full
> note.  The `OpReturnTuple` opcode and `Value::ReturnTuple` IR
> variant the original design proposed were NOT needed.
>
> 9b/9c/9d/9e remain open and are NOT closed by the T1.8a fix —
> they need par-dispatch-specific work in `src/codegen_runtime.rs`
> (worker tuple-output stitch shape, parser support for the fused
> `for (a, b) in pairs par(...) { … }` binding).  The 4 ignored
> canaries in `tests/threading_chars.rs` (`par_tuple_return_int_int`,
> `par_tuple_return_int_text`, `par_tuple_return_struct_text`,
> `par_tuple_destructure_in_for`) confirm via direct error messages
> that the gates are par-side, not general-fn-return-side.

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

## Why this is @PLAN06 scope, not 1.1+

The previous PLANNING.md placement put T1.8a (function tuple
return convention) in a 0.8.3 follow-up bucket and never connected
it to par.  As a result D11b had to mark tuples as
"✅ when tuples land" — a placeholder that quietly meant
"par will not accept tuple returns even after @PLAN06 lands".

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

### 9a — T1.8a function-return convention *(closed 2026-05-04)*

**Closed by commit `023ca15` on branch `plan-14-tuple-validation`.**
The original design (new IR variant + new opcode + caller-pre-
allocated slot, ~200 LoC) was over-engineered.  Actual-error survey
(@PLAN14 phase 01 follow-up) showed:

- `fn make_pair() -> (integer, integer) { (3, 7) }` already worked
  end-to-end on interp + native.
- `(a, b) = make_pair()` (destructure) already worked.
- `match make_pair() { … }` (call as match subject) already worked.
- Only **tuple-of-text returns under `--native`** failed, with three
  coupled type mismatches at signature / body / caller.

Fix landed in three files (`src/generation/{mod.rs,emit.rs,
dispatch.rs}`) totalling ~30 LoC:

1. `rust_type` for `Type::Tuple` in `Context::Result` recurses with
   `Context::Variable` for elements (signature is `(String, …)`).
2. `Value::Return` sets the existing `tuple_text_to_string` flag
   when returning a tuple-of-text literal.
3. `output_set` adds a `tuple_text_elem_clone` arm so destructure
   from a tuple-text element emits `var_t.0.clone()` instead of
   `&var_t.0.to_string()`.

Pinned by `e1_d2_return_int_int` and `e2_d2_return_text_text`
un-ignored cells in `tests/tuple_matrix.rs` (running under both
interp and `--native` with byte-identical stdout via the
@PLAN14 cross-mode harness).  PLANNING.md § T1.8a is updated.

**No `OpReturnTuple` opcode was added** — the original design
called for one but the actual fix needed only existing flags and
type-context routing.  The `Value::ReturnTuple` IR variant was
similarly not needed.  A future contributor should not introduce
those without a concrete failure they would solve.

### Phases 9b–9e — par-specific tuple work (open)

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
- **Reference rules in tuple elements** — DESIGN.md D11c.1 governs:
  `(integer, Reference<ParentSharedStruct>)` is allowed (the ref
  passes through unchanged); `(integer, Reference<WorkerOwnedStruct>)`
  is allowed (the ref's `store_nr` gets translated by the rebase
  walk); `(integer, Reference<PeerWorkerOwnedStruct>)` is forbidden
  (no way for a worker to construct one — type system prevents it
  post-phase-4).  Identical to struct-field rules.
- Test (un-ignore): `par_tuple_return_int_int`,
  `par_tuple_return_int_text`, `par_tuple_return_struct_text`.
- New test: `par_tuple_return_with_parent_shared_ref` — worker
  returns `(integer, Reference<GlobalConfig>)`; assert the
  `Reference` pointed at the parent stdlib store before AND after
  the rebase (no translation, pass-through).

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
| `par_tuple_return_three_arity` | 9c — pins the "any arity" claim concretely |
| `par_tuple_return_nested` | 9c — pins the nested-tuple-return shape |
| `par_tuple_destructure_in_for` | 9d |

Adjacent canary (NOT closed by phase 9 — different fix surface):
- `par_vec_of_capturing_fns_t4` — fails at vector construction
  (lambda → vector storage path), not at par dispatch.  See
  DESIGN.md D11a row 8 (split row).  Tracked in @PLAN15 D4.

## Loft-side prerequisites

- **Phase 1 (per-worker output Store) must land first** — the
  per-worker Store is what receives tuple-element records.
- **Phase 2 (stitch via rebase) must land first** — the rebase
  pass handles tuple-internal DbRef offsets the same way it handles
  struct-internal ones.
- **T1.8a (function-return convention)** — *closed 2026-05-04 by
  commit `023ca15`.*  See § 9a above for the actual-survey-vs-
  original-design note.  Phase 9c may discover that the par
  worker-dispatch path needs additional shape changes beyond what
  T1.8a delivered (the par canaries still fail with par-specific
  error messages, not general-fn-return errors), but the
  general-purpose return convention is no longer the gate.

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
| ~~9a slips and orphans phases 9b–9e~~ | *Retired 2026-05-04* — 9a closed via commit `023ca15`; phases 9b–9e are now unblocked. |
| ~~T1.8a's caller-pre-allocated-slot convention conflicts with the worker dispatch trampoline~~ | *Retired 2026-05-04* — the actual T1.8a fix did not introduce a caller-pre-allocated-slot convention; it routes tuple-of-text returns through `(String, …)` Variable-context types instead.  No trampoline conflict to mitigate. |
| Tuple-with-text return needs DbRef rebase across tuple element offsets | Phase 2's rebase already walks `owned_elements`; tuples expose the same accessor — no new rebase code.  D11c.1 covers the per-element category rules (worker-own / parent-shared / cross-worker) without tuple-specific runtime logic. |
| Tuple element holds a `Reference<ParentSharedStruct>` (e.g. shared cache) | Per D11c.1: parent-shared references pass through the rebase unchanged.  Test `par_tuple_return_with_parent_shared_ref` (added in 9c) asserts this. |
| Tuple elements with nested vectors / hashes | Out of scope for phase 9 — covered by D11a "nested vector input" canary which closes in phase 4; tuple elements are either primitive, text, or DbRef in @PLAN06 |
| Bench data shapes change between phases (struct → tuple) | Keep both shapes in `bench/11_par/`; mark which is the canonical apples-to-apples reference |

## Out of scope

- **Heterogeneous tuple arities per worker** — every worker
  returns the same tuple type (today's same-type-per-worker rule
  unchanged).
- **Generic tuple types in worker fn signatures** — bounded
  generics over tuples is a future feature; @PLAN06 accepts
  monomorphised tuple types only.
- **Tuple-of-tuples return** — workers returning `((A, B), (C,
  D))` deferred; nested-collection canaries cover the broader
  shape and close in phase 4.

## Cross-references

- [README.md](README.md) — @PLAN06 ladder, phase 9 added.
- [DESIGN.md § D11](DESIGN.md) — type spectrum; tuples promoted
  to first-class.
- [01-output-store.md](01-output-store.md) — per-worker Store
  receives tuple records identically to struct records.
- [02-stitch-not-copy.md](02-stitch-not-copy.md) — rebase walks
  tuple `owned_elements` like struct ones.
- [TUPLES.md](../../../TUPLES.md) — tuple feature design (T1).
- [PLANNING.md § T1.8](../../../PLANNING.md) — T1.8a / b / c
  remaining work; 9a closes T1.8a.
- `src/data.rs` — `Type::Tuple`, `element_size`,
  `element_offsets`, `owned_elements` already exist (T1.1).
