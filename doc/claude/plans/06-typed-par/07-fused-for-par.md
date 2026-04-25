<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 7 — Fused `for x in ls par(r = foo(x), 4) { … }`

**Status: open**

## Goal

Replace the planned phase-7 three-variant API (`par_for_each`,
`par_fold`, `par_iter`) with a single syntactic construction that
covers all four "shapes" by writing normal loft code:

```loft
for x in ls par(r = foo(x), 4) {
    // body — runs sequentially in the parent thread,
    //         sees `x` and the parallel result `r`
}
```

Body discards `r` → for_each.  Body accumulates → fold.  Body
appends → collect.  Body yields → iterator.  **One primitive,
four use cases — the body picks the policy.**

The vector result becomes opt-in: users who want one allocate it
explicitly in the body; users who don't pay nothing.

## Why one construction beats three variants

The 4-variant approach (sketched in plan-06 README phase 7 placeholder)
exposes three named entry points: `par_for_each`, `par_fold`,
`par_iter`.  Each is fine in isolation but they fail to compose for
common patterns.  Specifically, all three lose the **(input, result)
pairing** — `par_iter(xs, fn)` yields `iterator<U>`, not
`iterator<(T, U)>`, so a body that needs both must build an explicit
tuple at extra cost.

The fused form pairs them naturally because the body is a
for-loop body that already binds `x`; adding `r` is one assignment
in the loop header.

| Capability | 4 variants | Fused form |
|---|---|---|
| Body sees both `x` and `r` | tuple workaround | native |
| Body mutates enclosing locals | partial (fold's combine fn) | native — body runs in parent |
| `break` / `continue` in body | needs `break_with` plumbing | native — for-loop body |
| Side effects on shared state | lock dance per worker | native — body sequential, no contention |
| Composes with future `yield` (CO1) | needs explicit chain | native — body can `yield r` |

The trade-off is parser work for the new syntax (modest) versus
three new stdlib fns + their type signatures + their docs.  The
runtime cost is identical — both shapes use plan-06's store-stitch
pipeline with a bounded queue.

## Surface

```loft
for <var> in <input_expr> par(<result_var> = <worker_expr>, <threads>) {
    <body>
}
```

Where:

- `<var>` is a fresh loft identifier bound to each input element in
  turn (same as a regular `for x in xs`).
- `<input_expr>` is any expression of type `vector<T>`,
  `sorted<T>`, `index<T>`, or `hash<T>`.
- `<result_var>` is a fresh identifier bound to the worker's
  per-element output, visible inside the body.
- `<worker_expr>` is an expression in scope of `<var>` that the
  worker evaluates in parallel.  Currently must be a function call
  or method call; arbitrary expressions are deferred to a follow-up.
- `<threads>` is an `integer` for the worker count.  Reuses the
  existing clamping logic from `par(...)` (clamps to rayon's
  available pool; under WASM single-threaded falls back to a
  sequential for-loop).
- `<body>` is a regular for-loop body — full mutable access to
  enclosing variables, supports `break` / `continue` / `return` /
  `yield`.

### Examples

```loft
// for_each — side effects only, no result vector
for x in items par(_ = process_async(x), 4) {
    log_info("done: {x}")
}

// fold — accumulate
total: float = 0.0
for x in items par(score = score_of(x), 4) {
    total += score
}

// collect — explicit vector allocation
results: vector<Score> = []
for x in items par(score = score_of(x), 4) {
    results += [score]
}

// streaming consumer with `break`
for x in items par(score = score_of(x), 4) {
    if score > threshold {
        notify(x, score)
        break               // signals workers to stop
    }
}

// pair-aware body
for x in items par(score = score_of(x), 4) {
    if score < 0.0 { log_warn("negative score for item {x}") }
    summary[x.id] = score
}
```

### Backward compatibility

`par(input, fn) -> vector<U>` and `par_light(input, fn) -> vector<U>`
keep working unchanged.  The fused form is additive.  Existing
test fixtures and library examples need no edits.

For users who want the *today-shape* (return a vector) with the
*new runtime's* memory profile (no per-element vector grow), one
sugar fn lands alongside:

```loft
pub fn par_collect(input: vector<T>, fn: fn(T) -> U,
                   threads: integer) -> vector<U>;
```

Desugars internally to the fused form with a `vector_with_capacity`
preallocation in the body.  Documented as "the recommended shape
when you do need a vector".

## Implementation

### Parser changes

`src/parser/control.rs::parse_for_loop` extended to recognise the
`par(...)` modifier between the input expression and the body.
Grammar fragment (in pseudo-EBNF):

```
for_loop := 'for' ident 'in' expr [par_clause] '{' body '}'
par_clause := 'par' '(' ident '=' expr ',' expr ')'
```

The keyword `par` is contextual — it's not reserved at expression
position, so existing `par(input, fn)` calls continue to parse as
function calls.  Inside `for ... in expr <here> { body }`, the
parser tries `par_clause` first and falls back to the brace-block.

Lowered IR: a new `Value::ParFor` variant carrying `(input,
worker_fn_d_nr, x_var, r_var, threads, body)`.  Codegen emits a
runtime call that wraps plan-06's store-typed pipeline with a
bounded-queue stitch policy.

### Codegen changes

After phase 1–5 land, the runtime exposes one polymorphic store-typed
parallel call that takes:

- input store + element type,
- worker fn def_nr,
- thread count,
- a stitch policy (concat | discard | reduce | queue).

`Value::ParFor` selects the **queue** policy.  The runtime allocates
a bounded queue Store sized `2 × threads`, spawns workers that push
`(x_idx, x_payload, r_payload)` tuples, and runs the body
sequentially in the parent thread by popping in order.

### Runtime changes

Add `src/parallel.rs::run_parallel_queue` (or a `Stitch::Queue`
variant of the polymorphic dispatcher built in phase 3).
Behaviour:

1. Allocate bounded queue store with capacity `2 × threads`.
2. Spawn `threads` worker threads, each in a loop:
   - claim next input index via shared atomic counter,
   - compute `r = foo(x)`,
   - push `(idx, x, r)` onto the queue (blocks if full),
   - on shutdown signal: drain pending writes and exit.
3. Parent body loop:
   - pop next-in-order tuple from queue (blocks if empty),
   - bind `x_var` and `r_var` in the body's scope,
   - execute body,
   - on `break`: signal shutdown to workers, drain queue.
4. Join workers; deallocate queue store.

In-order delivery: queue is keyed by input index; consumer reads
slot `i` only after the producer for index `i` writes it.  A small
optimisation later (phase 7c?) can detect "body doesn't care about
order" and drop the in-order constraint.

### Test fixtures

`tests/scripts/22-threading.loft` already covers `par(...)` returning
a vector.  Add three fixtures specifically for the fused form:

| Fixture | Body shape | Asserts |
|---|---|---|
| `tests/scripts/par_for_each.loft` | side-effect only | output log lines, no allocation |
| `tests/scripts/par_for_fold.loft` | accumulate to a single value | final accumulator equals serial fold |
| `tests/scripts/par_for_break.loft` | `break` early on first match | workers stopped, no further `foo(x)` calls |

Plus a unit test in `tests/issues.rs::par_for_pair_aware` asserting
that the body sees the original `x` paired with the parallel `r` in
the order of the input vector.

### Documentation

- `LOFT.md` § Control flow: a new "Parallel for-loop" subsection
  showing the four use shapes (for_each, fold, collect, break).
- `THREADING.md`: replace the existing "par variants" section with
  one explanation centred on the fused construction; the existing
  `par(...) -> vector<U>` becomes a side-note for backward compat.
- `STDLIB.md`: `par_collect` entry as the documented "I want a
  vector" shape.
- `CHANGELOG.md`: user-facing entry framing it as "parallel
  for-loops" — the natural audience-facing name.

## Acceptance criteria

- All three new fixtures pass on Linux x86_64, macOS aarch64,
  Windows MSVC.
- The plan-06 phase 0 baseline benchmarks rerun within ±5 % of
  baseline (no regression on existing `par(...) -> vector<U>` users).
- A new microbench `bench_par_for_no_collect` measures the fused
  form's overhead vs. the today-vector form on bench-1 (1 M `i64`):
  expected savings of ~3 ms and ~8 MB peak memory.
- Eight-line snippet in `LOFT.md` shows a complete fused for-loop
  par; reads like idiomatic loft.

## Sequencing

Phase 7 lands AFTER plan-06 phases 1–5 (the store-typed pipeline +
auto-light).  Phase 6 (cleanup) and phase 7 are independent — they
can land in either order.

Implementation order within phase 7:

1. **7a — parser + IR.**  New `Value::ParFor` IR node, parser
   recognition of the `par(...)` modifier in for-loops, parse-error
   tests for malformed shapes.  Body still falls through to the
   today `par(...)` runtime as a stopgap (no perf win yet).
2. **7b — runtime queue store.**  `run_parallel_queue` in
   `src/parallel.rs` (or the polymorphic dispatcher's queue policy).
   Codegen now routes `Value::ParFor` to the queue policy.  Bench
   shows the expected ~3 ms / 8 MB win on bench-1.
3. **7c — `par_collect` sugar.**  Stdlib fn that desugars to the
   fused form with capacity-pre-alloc body.  Optional; can defer
   if no demand surfaces.
4. **7d — doc + CHANGELOG.**  Replace `THREADING.md`'s "par variants"
   section; new `LOFT.md` subsection; CHANGELOG entry.

Each commit lands with `make ci` green.

## Risks

| Risk | Mitigation |
|---|---|
| **Parser ambiguity** between the new `par(...)` modifier and a regular `par(...)` function call | Contextual keyword; parser only looks for the modifier in `for ... in expr <here>` position.  Spike the parse path before committing 7a — if conflict arises, fall back to a different keyword (`par_for`, or `for x in ls.par(r = foo(x), 4) { … }` method-style). |
| **Body-in-parent-thread surprise** — users assume the body parallelises too | Document loudly in LOFT.md and in the parser's error messages.  Add a lint that fires when the body contains another nested `par()` (allowed but flagged). |
| **`break` correctness** — workers must not deadlock on full queue when consumer stops reading | Queue uses bounded channel with a shutdown sentinel.  Worker write checks shutdown flag before block; if set, drops the result and exits.  Mirror of standard rust mpsc shutdown idiom. |
| **Out-of-order delivery temptation** — in-order forces serialisation per index | Phase-7b ships in-order only.  If benchmarks show order-imposed stalls dominating, a follow-up `par(... unordered)` modifier or `par_unordered(...)` opt-in.  Out of scope for 7a/7b. |
| **Worker panic** mid-iteration | Today's `par(...)` panics propagate to join; fused form same.  Documented.  Future improvement: surface the panic to the body's `r` as an enum variant once L1 error recovery lands. |
| **Memory overhead of the queue store** scales with `threads` and element size | Bounded at `2 × threads × size_of<(T, U)>`.  For typical (8-byte primitives, 4 threads): 64 bytes peak.  For struct-heavy workloads (1 KB elements, 16 threads): 32 KB — still trivial vs. the today-vector's MB-scale peak. |

## What this replaces in the plan-06 README

The README's phase-7 line currently reads:

> | 7 | (placeholder for variant entry points) |

Replace with:

> | 7 | [07-fused-for-par.md](07-fused-for-par.md) | open | M | Fused `for x in ls par(r = foo(x), 4) { … }` construction.  One primitive covers for_each / fold / collect / iter — body picks the policy.  Replaces the earlier 3-variant idea (`par_for_each` / `par_fold` / `par_iter`). |

## Out of scope

Deferred to follow-ups (post-plan-06):

- **Out-of-order delivery** (`par_unordered(...)` modifier).
- **Worker pool reuse across calls** — today every fused for-loop
  spawns its own pool.  A workspace-wide pool is a separate
  optimisation (related to A14 `par_light` and W1.14 WASM workers).
- **Nested fused loops** — semantics are clear (outer body runs in
  parent, inner spawns a sub-pool) but worth a fixture once 7b lands.
- **`yield` in the body** — interacts with CO1 coroutines (1.1+).
  Fused form is forward-compatible: when CO1 lands, body can
  `yield r` and the construction becomes a parallel iterator factory.

## Cross-references

- [README.md](README.md) — plan-06 ladder; phase 7 sits at the
  end after the store-stitch runtime + auto-light land.
- [00-baseline-and-bench.md](00-baseline-and-bench.md) — phase 0's
  bench harness gates phase 7's perf claim.
- [../../THREADING.md](../../THREADING.md) — current par design;
  phase 7d rewrites the "variants" section.
- [../../LOFT.md](../../LOFT.md) — phase 7d adds a "Parallel
  for-loop" subsection.
- [../../ROADMAP.md § 1.1+ I13](../../ROADMAP.md#11-backlog) —
  iterator protocol; future-compatible with body `yield`.
