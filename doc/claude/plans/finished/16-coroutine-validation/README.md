<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN16 — Coroutine validation: yielded-type × drive-context matrix

**Status: FINISHED 2026-06-04.**  Core goal met — the cross-mode matrix
(`tests/coroutine_matrix.rs`) is **18/18 green on both backends** across
Y1–Y5 × X1–X4, and the interp/native divergences that motivated the plan are
closed.  Phase 02 was repurposed from the original "text row" into the
**layout-driven yield-codec collapse**: the per-shape native codec spray
(`next_i64` / `next_text` / `next_dbref` / a hand-written template per yield
shape) was replaced by **one flatten-walk derived from `T`'s slot kinds**
(`src/coroutine_layout.rs`) — see [02-codec-collapse.md](02-codec-collapse.md).
Three previously-`--native`-broken composite shapes (`(integer,float)`,
`(integer,boolean)`, `(vector,integer)`) now compile and run via the single walk
with **zero per-shape code**.

**Second outcome — this plan was the design-protocol with-arm.**  The codec
collapse was produced *with* the predict-validate procedure (predict → probe →
build); two of the design's claims were falsified **by probe** before any
regression (the store-layout framing; an over-unification that falsely absorbed
@P325), and the build validated the invariant under construction.  That measured
run graduated `DESIGN_VERIFICATION.md § C1` into
[Design Protocol 1](../../../DESIGN_PROTOCOL.md) and seeded the `engineering-rigor`
skill (`.claude/skills/engineering-rigor/`) — the canonical "a design is a
testable hypothesis" record.

**Shipped:** @P210/@P211/@P218/@P219 (state-machine lowering fixes, below);
phase 01 unified transport (`next_into`); phase 02 layout-driven codec
(scalar + DbRef-ref slice); the Y1–Y5 × X1–X4 matrix green on both backends.

**Remaining tail — small, triggered, routed to `COROUTINE.md § Open work` (not a
held plan, per the light-flow lifecycle):**
- **text-*element* yields** inside a tuple (`(text, integer)`) on `--native` — a
  yielded `text` is a `&str`, so riding the codec buffer needs a store intern
  (`codegen_runtime::db_from_text`) with the lifetime question that entails; the
  one excluded codec cell.
- **tuple-through-higher-order / comprehension** composite cells (y4_x3, y4_x4).
- **full legacy-channel deletion** (`next_i64`/`next_text`/`next_dbref` + the
  high-byte channel tag) once every shape routes through the codec — the codec's
  finishing move, pure subtraction.
- @P226 (mixed Simple+ForLoopBody `__vdb_*` arm-scoping) — same family as @P218.

---

The active-phase rationale and per-P-issue closure notes are retained below as
the closure record.

@P210: `collect_segments` in `src/generation/coroutine.rs` now
recognises `Value::Loop` (while) alongside `Value::Block` (for),
so `while …; yield …` generators no longer produce empty state
machines.

@P211: native state machine now has a parallel `next_text` channel
(`LoftCoroutine::next_text` + `coroutine_next_text` runtime
helper); text-yielding generators override `next_text` (selected
on the function's `Iterator<T>` return type), and
`OpCoroutineNext` size 16 dispatches to the new helper instead of
falling through to the broken `as i32` arm.  Pinned by
`p211_coroutine_yield_text` + `p211_coroutine_yield_text_while`
in `tests/issues.rs` and the extended
`tests/scripts/51-coroutines.loft`.

@P218 closed 2026-05-05 — `__work_*` text-format buffers used across
multiple state arms (or in the eager-collect factory body for
while/for-yields) are now pre-declared at function scope in both
`emit_next_i64` and `emit_for_body_factory` so they're visible from
every per-state emission.  Pinned by
`p218_coroutine_yield_format_with_param` +
`p218_coroutine_while_yield_format` in `tests/issues.rs` and the
extended `tests/scripts/51-coroutines.loft`.

@P219 closed 2026-05-05 — `emit_for_body_factory` now strips
trailing `Return` ops from the body's operator list before emit,
so `patch_hoisted_returns`' `[Loop, Return(Null)]` →
`[Return(Loop)]` coalesce no longer reaches the factory's body.
Pinned by `tests/issues.rs::p219_vector_for_yield_in_generator`
+ `p219_vector_for_yield_text`.

@P226 (mixed Simple + ForLoopBody yield segments — `__vdb_*`
backing locals scoped to one arm) filed 2026-05-05 during @P219
fix-variant probing.  Same scoping family as @P218 / @P224.

_(Superseded — see the FINISHED status block above: phase 00/01 landed, the
matrix is wired and 18/18 green on both backends, and Y3/Y4/Y5 cells re-probed
under the unified channel + layout codec.  Y4 composite-through-higher-order
cells y4_x3/y4_x4 remain in the routed tail.)_

## Goal

Validate that coroutines (functions returning `iterator<T>` that
suspend with `yield`) round-trip every meaningful **yielded value
type** through every meaningful **drive context**, with
**interp/native byte-identical stdout** asserted by the cross-mode
harness already in place from @PLAN14 phase 00.

The driving question: "given a generator yielding values of type T,
consumed by drive context X, are the values observed in the consumer
the same under interp and native, in the same order, with the same
side-effect timeline?"  A green matrix means: every cell has a test
that runs in interp and native, and the cross-comparison passes.

This plan inherits all infrastructure from @PLAN14: `cross_mode!`,
`tests/common/cross_mode.rs`, the `#[ignore]` discipline, P-id rules.
The only new artefact is `tests/coroutine_matrix.rs` and the
phase-specific cells.

## Why now

Coroutines shipped in 0.8.3 (CO1.1–CO1.6) via state-machine lowering
to stable Rust (per project memory `project_native_and_coroutines`).
State-machine lowering is exactly the kind of code that gets brittle
on edge cases — a yielded text whose backing storage outlives the
suspended frame, a `yield` inside a nested helper, a yield inside a
match arm.  The matrix surfaces those edges before users hit them.

Three known soft spots point at concrete bug-yield potential:

1. **CO1.4 `yield from`** is "deferred to 1.1+" per COROUTINE.md.
   This plan does NOT implement it — but a CLOSED cell asserts the
   parser-or-scope error stays stable until the feature lands.
2. **Stackful semantics** mean `yield` inside a helper called from
   the generator is valid.  A cell tests this; if the harness misses
   the helper-frame serialisation, it surfaces as either a wrong
   value or a panic in the suspended-frame replay.
3. **Yield-of-text** has the same lifetime risk as
   tuple-element-text (@PLAN14 phase 01 found @P207 there).  The
   yielded text's backing String must survive the yield boundary —
   `String` lives on the suspended frame, not the consumer's stack.
   A C2 cell pins the contract.

Per the bug-hunt policy: extend the matrix, find the bugs.  Plan-14
phase 01 found 2 P-issues in 15 cells.  Coroutines, being newer and
more state-machine-heavy, may yield more.

## The matrix

Two axes.  Every cell is `PASS:test_name`, `FIX:phase`, or
`CLOSED:reason` with a DESIGN_DECISIONS.md cross-reference.

### Axis 1 — yielded value type

| ID | Yielded type | Notes |
|---|---|---|
| Y1 | **Basic scalar** — `iterator<integer>`, `iterator<float>`, `iterator<boolean>`, `iterator<character>` | Baseline; no heap dep. |
| Y2 | **Text** — `iterator<text>` | Owned text element; the suspended frame holds the backing String.  Lifetime risk. |
| Y3 | **Reference** — `iterator<Reference<S>>` | DbRef yielded; lifetime risk if the referenced record is owned by the generator. |
| Y4 | **Tuple** — `iterator<(A, B)>` | Cross-references @PLAN14 phase 01–04 (the destructure-on-receive path). |
| Y5 | **Closure** — `iterator<fn(integer) -> integer>` | Yielded fn-ref; the closure's own dep tracking must survive serialisation. |
| Y6 | **Vector** — `iterator<vector<T>>` | Out of scope — yielding owned vectors has no current consumer; CLOSED. |

### Axis 2 — drive context

| ID | Context | Today |
|---|---|---|
| X1 | **for-loop** — `for v in gen() { … }` | Primary tested path. |
| X2 | **manual `next()`** — `it = gen(); v = next(it)` | Direct API exercise. |
| X3 | **higher-order** — `map(gen(), \|v\| { … })`, `filter`, `reduce` | Consumes via the iterator protocol. |
| X4 | **comprehension** — `[v for v in gen()]` | Builds a vector from the iterator. |
| X5 | **yield from** — generator delegating to another | CO1.4 deferred to 1.1+; stays CLOSED until the feature lands. |

### Cell key

Each cell `(Yi, Xj)` has one of:
- **PASS** — covered by an existing or new test.
- **FIX** — implement + add test in the matching phase below.
- **CLOSED** — design decision (Y6, X5).  Cell test asserts the
  parser/runtime error stays stable.

## Phase layout

| Phase | Yield rows | Drive cols | Outcome |
|---|---|---|---|
| [00 — matrix freeze + harness wiring](00-matrix.md) | (table) | (table) | Frozen matrix; new `tests/coroutine_matrix.rs` binary; reuses `cross_mode!`.  No production change. |
| [01 — unified store-backed yield channel](01-unified-channel.md) | (cross-cutting) | (cross-cutting) | One `next_into(stores, &mut [i64])` trait method replaces the per-shape combinatorial expansion (`next_i64` / `next_text` / `next_dbref` / hypothetical `next_tuple_2i64` / `next_tuple_3i64` / …).  Unblocks Y4 native (@P327) and future arities without adding one trait method per shape.  Identified 2026-05-23 from the half-finished hardcoded `next_tuple_2i64` attempt that broke the interp's slot allocator. |
| 01b — basic scalars (Y1) | Y1 | X1, X2, X3, X4 | Every Y1 cell green.  Establishes the cell shape against the simplest yielded type. |
| 02 — text (Y2) | Y2 | X1, X2, X3, X4 | Active risk — yielded-text lifetime through the suspended frame.  Phase 02 either confirms via leak test + cross-mode green, or files a P-issue. |
| 03 — references (Y3) | Y3 | X1, X2, X3, X4 | Yielded DbRef; verifies that the referenced record outlives the resume cycle. |
| 04 — tuples (Y4) | Y4 | X1, X2, X3, X4 | Tuple yielded values; cross-references @PLAN14 destructure-after-yield.  Some cells may be gated on T1.8a (tuple return convention) for the manual-`next()` path. |
| 05 — closures (Y5) | Y5 | X1, X2, X3 | Closure yielded; depends on @PLAN15's closure-validation result for capture-shape correctness through serialisation. |
| 06 — yield from CLOSED + freeze | (X5 only) | X5 | Confirm the deferred-feature error message stays stable; add a regression test in `tests/parse_errors.rs`.  Update COROUTINE.md, PLANNING.md, CHANGELOG_TECHNICAL.md.  Move plan to `finished/`. |

## Acceptance for the whole plan

- Matrix in [00-matrix.md](00-matrix.md) fully populated — no
  "unknown" cells.
- Every PASS cell has a `cross_mode!`-driven test in
  `tests/coroutine_matrix.rs` that runs green under
  `cargo test --release --test coroutine_matrix -- --ignored`.
- Cross-mode equivalence is mandatory.
- Every CLOSED cell has a corresponding negative test asserting the
  diagnostic stays stable until the feature lands.
- COROUTINE.md `Known Limitations` section reflects the matrix
  outcome (e.g. "yield-of-text is verified across N drive contexts").

## Out of scope

- Y6 yielding owned vectors.  Not a real consumer pattern.  CLOSED.
- X5 `yield from`.  Deferred to 1.1+ per CO1.4.  CLOSED with
  diagnostic-stability test.
- Coroutines crossing thread boundaries.  Plan-06 owns par
  interactions; this plan validates the single-threaded contract.
- Generators with side-effects on parent state — the matrix
  checks values yielded, not arbitrary side-effect timing.

## Risks

| Risk | Mitigation |
|---|---|
| Y2 (yielded text) surfaces a state-machine lowering bug that affects every X-column for that row | Phase 02 sub-divides: 02a single drive context (X1) to confirm shape, 02b extends to X2-X4.  If 02a fails, the plan pauses, the bug is filed, and a fix lands before continuing. |
| Y4 (tuple yielded) is partially gated on T1.8a (tuple return convention) — manual `next()` may need T1.8a's caller-pre-allocated slot | Cells that depend on T1.8a get `#[ignore = "T1.8a — plan-06 phase 9a"]` and un-ignore in a one-line follow-up when 9a lands.  Same pattern as @PLAN14 phase 01. |
| Y5 (yielded closures) depends on @PLAN15's closure-validation result | Phase 05 sequences AFTER @PLAN15 phases 01-04 land.  If @PLAN15 stalls, phase 05 stays open until it ships. |
| Stackful semantics (yield inside helper) is harder to test cross-mode if the harness can't observe the suspended frame | Tests assert end-state values, not frame internals.  If a divergence appears, `LOFT_LOG=crash_tail:50` + a manual `--native-emit` inspection is the diagnostic path. |

## Cross-references

- [COROUTINE.md](../../../COROUTINE.md) — coroutine design,
  "Known Limitations" section.
- [LIFETIME.md](../../../LIFETIME.md) — yielded-value lifetime
  rules for text and Reference.
- [@PLAN14 phase 00](../14-tuple-validation/00-matrix.md) — donor
  template.
- [@PLAN15 closure validation](../15-closure-validation/README.md)
  — phase 05 prerequisite (now SHIPPED 2026-05-12).
- `src/state/codegen.rs` — coroutine state-machine lowering.
- `src/data.rs::Type::Iterator` — iterator type.
- `OpCoroutineNext` opcode — drives `for` and `next()` paths.
