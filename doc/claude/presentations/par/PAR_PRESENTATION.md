<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `for ... par(...)` — a presentation

A long-form retrospective of how the parallel for-loop construct in
loft was conceived, redesigned, and reduced.  The narrative is
deliberately thicker than [THREADING.md](../../THREADING.md) (which
documents the *current* state) — it traces *why* each layer of the
design exists.

## Contents
- [Timeline](#timeline)
- [Main insight shifts](#main-insight-shifts)
- [Current state of the plan](#current-state-of-the-plan)
- [What is still open](#what-is-still-open)
- [Same workload in three languages](#same-workload-in-three-languages)

---

## Timeline

Dates are git-anchored; commit hashes refer to the loft repository.

### Pre-history — the primitive era (2026-03-15 → 2026-04-12)

| Date | Event |
|---|---|
| 2026-03-15 | Initial commit (`bf012e4`).  Already shipping `parallel_for_int`, `parallel_for`, `par(...)` for-loop syntax, and first-class `fn <name>` refs.  Three internal native fns (`n_parallel_for_native`, `_text_native`, `_ref_native`); six runtime variants in `src/parallel.rs`; four `parallel_get_*` getters because results lived outside any Store. |
| 2026-03-23 | First par benchmark lands (#58, `126ddcb`). |
| 2026-03-29 | "Light" path lands (`c00689e`) — workers without allocation skip the heavy clone. |
| 2026-03-31 | `par_light(...)` exposed as a separate user-facing function (#83, `7f156d7`). |
| 2026-04-04 | A15 — `parallel { ... }` block uses real OS threads via `std::thread::scope` (#103, `2012eed`). |
| 2026-04-12 | Interfaces + `join()` + general thread-safety pass (#157, `10ba66f`). |
| 2026-04-21 | integer→i64 migration cascades through par extras (`864dafe`); par worker extra-arg widening fixes follow (`7bf3558`, `edbc9f3`). |

### Plan-06 opens — "everything is a store" (2026-04-25)

A single day during which the redesign goes from idea to scaffolded
plan with baseline numbers:

| Commit | Step |
|---|---|
| `36ec942` | Plan-06 opened: typed-par "everything is a store" initiative. |
| `9742ddd` | 0a — characterisation suite (`tests/threading_chars.rs`, 16 positives + 17 ignored canaries) + four surface-gap inventory (G1 struct-enum return, G2 primitive-input garbage, G3 wasm-par codegen rejects, G4 native-par was sequential despite the keyword). |
| `6c80cd8` | DESIGN.md D11 — full type spectrum on input *and* output (the matrix the canaries enumerate). |
| `eeab071` / `989a7b7` | 0b — realistic perf bench (`bench/11_par/`) with python + rust + loft-wasm columns; D11 type-coverage tracker pre-populated. |
| `fd95b97` | 0c — record baseline in THREADING.md (loft-interp 44 ms, loft-native 12 ms, rust 4 ms, python 33 ms). |
| `cd22d01` | Redesign around the read-only-parent rule + `Arc<Store>` sharing (this is the load-bearing decision — see insight 2). |
| `0c9148d` | D1c — drop input-order guarantee from `par()` results. |

### Phase-1 native parallelism (2026-04-25 → 2026-04-26)

| Commit | Step |
|---|---|
| `943bbe7` → `66a5ccd` | Steps 2–5 — workers write to per-worker output slots; `mpsc::channel` removed from text/ref/raw/int paths; `parallel_workers<R, F>` template extracted. |
| `2aa04a7` | 1.5 — switch to a shared rayon work-stealing pool. |
| `a52c605`, `b91d647` | G4 closed across all three native paths.  Native-par column drops from 12 ms to 6 ms. |
| `ce7583b`, `7ab13ac` | Phase 2a — `StoreRebase` + `rebase_walk_record` infrastructure (rebase machinery for ref returns). |

### Phase-4 typed surface (2026-04-26 → 2026-04-28)

| Commit | Step |
|---|---|
| `2a388fb`, `78940fc` | 4c — `Stitch::Concat` lands; `ConcatLegacy` retired; runtime sizes derived from the worker fn's typed signature. |
| `beab636`, `f63e057` | 4d.A — typed wide-input dispatch (tuple-as-vector-element); P189 family of bugs surfaces. |
| `d83eb0c` | 4d.B — par-over-sorted via materialise-then-route desugar. |
| `2538de2` | Text output stores — workers intern strings into per-worker slots (G3-text closed). |

### Spine reorder + the strategic shift (2026-04-29)

The most consequential 24 hours in @PLAN06's history.  Realising
the materialised result vector was the source of most remaining
complexity, the topical phases got reordered into a single
**complexity-reducing spine** ([PRIORITY.md](../../plans/finished/06-typed-par/PRIORITY.md)),
and the new phase 10 was added to retire materialisation entirely:

| Commit | Step |
|---|---|
| `25c697a` | PRIORITY.md introduced — fast-track order by complexity reduction. |
| `11b101e` | Plan-06 phase 10 — drop the materialised result vector entirely.  Stream-only par (Discard / Reduce / Queue); `par_to_vec` becomes opt-in. |
| `87c2ce8` | Spine 2 — `Stitch::Discard` runtime. |
| `f1e9e70` → `be5d020` | Spine 3 — `Value::ParFor` IR + parser empty-body lowering to `n_parallel_discard`. |
| `bc1d6c3` | Spine 4 — `Stitch::Queue` runtime. |
| `24a0299`, `faafb96` | Spine 5 — par-result use-site analyser; warns on materialise + unused. |
| `3bad980` | Spine 6 — audit confirms the test corpus is already streaming-only. |
| `0498fa9` *(eve)* | Spine 7 — warning promoted to error.  Materialising par results no longer compile. |
| `cdbd989` → `0c8a748` | Spine 8a–8c — `par_buffer_stack` infrastructure + Queue dispatch wired for primitive / DbRef / text returns. |

### Spine 8d + ARC.md replaces the spine (2026-04-30)

| Commit | Step |
|---|---|
| `6dda65c`, `ef6b958`, `5c291ee` | Spine 8d.0/8d.1/8d.2 — Queue dispatch for ref returns via `WorkerStores::take_all_owned` adopt + rebase. |
| `e2b439d` | ARC.md introduced as the scope-locked successor to the spine.  PRIORITY.md becomes historical. |
| `b9ad7af` | A1 — heavy `parallel_execute_and_collect` retired (-565 LOC, the biggest single deletion of the plan). |
| `217b3ac` | A2 — fixed `SLOTS_PER_THREAD = 16` per-thread reservation retired in favour of a shared `Arc<AtomicU16>` dispenser. |

### A6 canary closure + A3/A5 (2026-05-01 → 2026-05-03)

| Commit | Step |
|---|---|
| `b9f7fc1`, `17bb33f`, `f048d20`, `792ea7b` | A6.a / A6.b / A6.c / A6.d — four fn-ref / vector / keyed-collection canaries closed in a single session.  P196 (tuple-of-fn-ref native codegen) closed at the same time. |
| `125a9f5`, `63cf494` | A3 / A3.5 — narrow-Integer + Boolean/Character/Enum-no-payload Queue paths.  Float/Single still pending IR bit-cast support. |
| `3be821e` | A5 — `Stitch::Reduce` runtime + `n_parallel_fold` native fn.  User-facing `par_fold` builtin still pending. |

### Type-spectrum + T1.8a unblock (2026-05-04)

| Commit | Step |
|---|---|
| `9db18fd` | Plan-06 type-spectrum: 3 new canaries (`par_tuple_return_three_arity`, `par_tuple_return_nested`, `par_vec_of_capturing_fns_t4`) + D11 corrections.  A7's target list grows from 4 to 7. |
| `023ca15` *(@PLAN14 branch)* | T1.8a closed — function tuple-return convention.  ARC step A7 unblocked (the original ~200-LoC `OpReturnTuple` design collapsed to ~30 LoC of type-context routing). |

---

## Main insight shifts

The seven shifts that changed how par works.  Each corresponds to a
moment when a sentence we'd been writing for weeks suddenly meant
the opposite of what we thought.

### 1. "Par is a clever primitive" → "everything is a store" *(2026-04-25)*

The original implementation branched on type and return shape: three
native fns, six runtime variants, three codegen dispatch arms, four
getter primitives.  The realisation: the whole interpreter is
already store-organised — every allocation, every variable, every
parameter lives in `stores.0..stores.N`.  The reason `par` had three
native fns was that **the output of a parallel call was the only
place in the interpreter that did not follow this rule**.  Primitive
results were written through raw byte pointers; text via owned
`String` channels; references via ad-hoc `copy_block`.  If workers
write into per-worker output Stores like every other loft fn does,
the marshalling collapses to "stitch N output stores into one"
regardless of element type.

Consequence: ~1500 LOC of bespoke marshalling on the retirement list.

### 2. "Workers may write to parent" → "parent stores are read-only — language rule, not runtime convention" *(2026-04-25, redesign commit `cd22d01`)*

The full-clone path silently swallowed worker writes (writes hit a
private clone, dropped at join — silent data loss in release).  The
light path raced (writes through raw pointers — undefined behaviour).
**Neither was a legitimate execution mode.**  Both were compatibility
hacks that hid a class of user bugs.

Plan-06 promotes read-only-parent to a **compile-time error** with
an `is_par_safe` analysis backed by `#pure` / `#impure` annotations
on stdlib fns.  Workers that try to write parent state fail to
compile with a fix-it suggestion.  The full-clone path
(`clone_for_worker`, the heavy variant) has no semantic purpose
under the new rule, so it is on the deletion list (~520 LOC).

### 3. "Preserve input order" → "completion order" *(2026-04-25, DESIGN.md D1c)*

Order-preserving stitching forces either even chunking (bad for
unbalanced workloads) or serialised writes into a shared buffer
(defeats parallelism).  Drop the guarantee → the runtime stays fast
and run-to-run order variation becomes **direct evidence of
parallelism** for tests.  Users who need order include the input
index in the worker's return value and re-sort.

### 4. "Materialise the result vector by default" → "stream-only par; result vector is opt-in" *(2026-04-26 → 2026-04-29, phase 10 + spine 7)*

The materialised result vector was *the* load-bearing source of
remaining complexity: the Concat path, deep-copy infrastructure
(`copy_block` + `copy_claims`), `StoreRebase` (just to make
materialise fast), the `par` vs `par_light` user split, the per-row
Stitch policy decisions, the result-vector-layout debate.

Phase 10 dropped it.  `par(...)` becomes one of:
- **Discard** — worker side effects only (logging, host I/O).
- **Reduce** — per-worker partial accumulator + final monoid combine.
- **Queue** — fused `for x in items par(r = fn(x), N) { use(r) }` consumes results in completion order.

`par_to_vec(input, fn, threads) -> vector<S>` is the opt-in helper
for the rare case that genuinely needs random access, multi-pass,
or storage in a `vector<S>` field.  Materialisation cost becomes
visible at the call site instead of the implicit default.

A test-corpus audit (spine step 6) confirmed the entire test suite
was already streaming-only.  Within 24 hours of adding the
materialise-warning, the warning became an error (spine step 7).

### 5. "Many topical phases" → "single-spine ordering by complexity reduction" *(2026-04-29, PRIORITY.md)*

Phase numbering by *topic* (output store / stitch / one native fn /
typed surface / auto-light / cleanup / fused / browser / tuples /
no-vector) made the question "what should I work on next?" hard to
answer.  PRIORITY.md reordered the same phases as a sequential
spine, each step chosen to retire the most complexity per unit
effort, routing through phase 10's strategic shift as quickly as
possible.

### 6. "Spine + phases" → "ARC.md scope-locked steps" *(2026-04-30)*

Even the spine drifted from the phases — step 8 absorbed phases 5b,
6, and most of 10.  ARC.md became the single source of truth: 11
steps (A1–A11), one PR per step, hard scope locks, named acceptance
tests.  No partial-DONE: a step is OPEN, IN-FLIGHT, or DONE; if it
ships with a caveat, the caveat becomes the next step instead of
paperwork.  A1 retired `parallel_execute_and_collect` (-565 LOC) —
the biggest single deletion of the plan — within hours of the new
doc landing.

### 7. Plan-06 as a structured fuzz/bug-hunt *(recognised 2026-05-02, doc commit `fdf312c`)*

A surprising emergent value: the canary-driven type-spectrum probe
surfaced **14+ P-issues (P188–@P201)** at the type-system × native-codegen
× parallel-runtime intersection.  Several (P191, P195, P196, @P198,
@P199) are bugs that ordinary doc-tests do not surface — they
require the specific type-shape interactions the canaries force.
The headline metric "~1100 LOC retired" undersells the work; the
plan functions as a curated fuzz of the par × type system surface,
and **as long as canaries keep firing, @PLAN06's per-day yield is
high**.

---

## Current state of the plan

Two parallel structures coexist and should not be confused:

- **THREADING.md** documents the *current shipped behaviour* — the
  user-facing `par(...)` clause, the native primitives behind it,
  the runtime invariants.  This is what users see today.
- **plans/finished/06-typed-par/** documents the *redesign in progress* — the
  ARC.md execution sequence, the historical PRIORITY.md spine, the
  per-phase design docs, the cross-cutting DESIGN.md.

Today (2026-05-06) the user-facing surface is mature enough that
real loft programs (the par benchmark, `tests/scripts/22-threading.loft`,
breakout, the multiplayer editor) use it without contortions.  The
redesign has shipped enough machinery that the test corpus is
streaming-only, the materialise path is a compile error, the
light-Concat path is the only major retirement still queued, and
the runtime is one strategic refactor (A8 trait collapse) away from
the "≤ 3 `run_parallel_*` fns" target.

### "Plan-06 done" — three measurable acceptance criteria

From [ARC.md § Acceptance state](../../plans/finished/06-typed-par/ARC.md):

| Criterion | Today |
|---|---|
| Single dispatcher family — `grep -c "fn run_parallel_" src/parallel.rs` ≤ 3 | More than 3 today (Discard / Queue / Queue_text / Queue_ref / Queue_narrow / Reduce / etc.); A8 collapses |
| Single user surface — `par_light` removed from `default/01_code.loft` | `par_light` still resolves; A9 retires it after 5e fixed-point lands |
| Zero ignored par canaries — `grep -c "^#\[ignore" tests/threading_chars.rs == 0` | 4 today (all par-tuple); A7 closes (now unblocked by T1.8a) |

When all three hit, @PLAN06 closes.

---

## What is still open

ARC step status, in shipping order.  DONE steps (A1, A2, A6) elided.

| ARC step | Status | Effort | What it does |
|---|---|---|---|
| **A3** narrow-prim Queue | narrow-Integer DONE; Boolean/Single/Character/Float/Enum-no-payload PENDING | S | Extends `Stitch::Queue` to all narrow-primitive return widths.  Float/Single need IR bit-cast support. |
| **A4** retire light Concat path | OPEN | S | Pure delete pass once A3 has unhooked every caller of `parallel_light_execute_and_collect` (~300 LOC). |
| **A5** Reduce + `par_fold` | runtime DONE 2026-05-03; user-facing parser builtin OPEN | M | Runtime + native fn already shipped.  Outstanding: parser-level builtin `par_fold(items, init, fn_name, threads)` so loft programs do not write `d_nr`s by hand, and/or auto-detection of `sum(parallel_for(...))` patterns. |
| **A7** par-tuple canaries | OPEN — unblocked 2026-05-04 by T1.8a | M | Routes tuple returns through Queue's wide-return path.  Closes 5 of the 7 tuple canaries (3 originally tracked + 2 broader-coverage from the type-spectrum audit) plus the fused-for tuple-destructure binding. |
| **A8** collapse Queue dispatchers | OPEN | M | Trait `QueueResult` with 4 impls (`i64`, `String`, `DbRef`, `NarrowBytes`) replaces the 4 per-shape `run_parallel_queue_*` fns with one generic dispatcher.  ~30 generated test fixtures regenerate. |
| **A9** drop `par_light`; auto-light | OPEN | M with 5e | Phase 5e fixed-point analyser classifies user fns Pure/Impure transitively; the parser auto-selects light vs heavy at every par call site.  `par_light` keyword becomes a deprecated alias, then is removed. |
| **A10** browser parallel via `wasm-bindgen-rayon` | OPEN — strategic showcase track | XL (3 sub-PRs) | Web Worker pool + `crossOriginIsolated` + per-worker output Stores survive `postMessage`.  Phase 2's `StoreRebase` runs after transfer.  Acceptance: gallery `bricks_par.loft` ≥ 2× speedup at threads=4.  Does not displace validation work. |
| **A11** final cleanup + doc rewrite | OPEN | S | Rewrite THREADING.md to the streaming-only model; mark all phases done; close the type-spectrum tracker. |

Adjacent un-blocked work that does not block @PLAN06 closure:
- Phase 5c — `Arc`-wrap parent stores (filed as a PERFORMANCE.md
  item; out of scope for ARC).
- Phase 11 — `par_to_vec(input, fn, threads) -> vector<S>` opt-in
  materialiser.  Out of scope for ARC; spawns its own arc when a
  user needs it.

---

## Same workload in three languages

A representative parallel reduction: square each integer in
`1..=1000` and sum the results, dispatching across 4 workers.  Same
algorithm, three surfaces.

### Loft

The `par(...)` clause attaches to the for-loop directly.  Workers
return one value per element; the loop body sees that value as `r`.

```loft
fn square(x: const integer) -> integer { x * x }

fn main() {
    items: vector<integer> = [];
    for i in 1..1001 { items += [i]; }

    total = 0;
    for x in items par(r = square(x), 4) {
        total += r;
    }
    println("sum of squares: {total}");
}
```

After A5's user-facing `par_fold` lands the same workload becomes
the more direct:

```loft
total = par_fold(items, 0, fn(acc, x) -> acc + x * x, 4);
```

### Java

Java exposes parallelism as a *property of streams*, not a control
construct.  `parallel()` on the stream pipeline tells the
`ForkJoinPool.commonPool()` to dispatch.  The thread count is
implicit (defaults to one less than `Runtime.availableProcessors()`).

```java
import java.util.stream.IntStream;

public class SumOfSquares {
    public static void main(String[] args) {
        long total = IntStream.rangeClosed(1, 1000)
                              .parallel()
                              .mapToLong(x -> (long) x * x)
                              .sum();
        System.out.println("sum of squares: " + total);
    }
}
```

### Go

Go has goroutines and channels but **no parallel-for construct**.
Anything resembling `par(...)` has to be hand-rolled: pre-split the
input, spawn one goroutine per chunk, accumulate per-worker
partials, join via `sync.WaitGroup`, combine on the caller side.

```go
package main

import (
    "fmt"
    "sync"
)

func main() {
    const n = 1000
    const workers = 4

    items := make([]int, n)
    for i := range items {
        items[i] = i + 1
    }

    partials := make([]int64, workers)
    var wg sync.WaitGroup
    chunk := n / workers
    for t := 0; t < workers; t++ {
        wg.Add(1)
        go func(t int) {
            defer wg.Done()
            lo, hi := t*chunk, (t+1)*chunk
            if t == workers-1 {
                hi = n
            }
            var sum int64
            for _, x := range items[lo:hi] {
                sum += int64(x) * int64(x)
            }
            partials[t] = sum
        }(t)
    }
    wg.Wait()

    var total int64
    for _, p := range partials {
        total += p
    }
    fmt.Println("sum of squares:", total)
}
```

### What the three surfaces show

- **Loft** treats the parallel for-loop as a first-class control
  construct.  The worker function is named, type-checked, and
  enforced read-only against parent state at compile time.  Thread
  count is explicit; result delivery order is "completion" not
  "input" by deliberate choice (DESIGN.md D1c).
- **Java** treats parallelism as a stream pipeline property.  Pure
  data-parallel — no notion of a per-element "worker call site".
  Thread count is implicit (a `ForkJoinPool` global); error handling
  is exception propagation through the stream.
- **Go** has no language-level parallel-for.  The user composes
  goroutines + `WaitGroup` + manual chunking + per-worker partials
  + final combine themselves.  The flexibility is high; the
  boilerplate is also high; the type system does not enforce that
  each goroutine touches only its own slice.

The loft surface is closer to Java's intent (a single declarative
construct) but with Go's explicit thread count and named worker fn.
The internal redesign documented above is precisely the work of
making that surface compile to runtime machinery as small as Go's
hand-rolled version while keeping Java's "one line at the call
site" ergonomics.

---

## See also

- [THREADING.md](../../THREADING.md) — current shipped behaviour, runtime
  invariants, and the per-thread safety analyses (P1-R1 … P1-R5).
- [plans/finished/06-typed-par/README.md](../../plans/finished/06-typed-par/README.md) —
  per-phase design docs and the bug-yield ledger.
- [plans/finished/06-typed-par/ARC.md](../../plans/finished/06-typed-par/ARC.md) — the
  active execution sequence (A1–A11), single source of truth for
  "what ships next".
- [plans/finished/06-typed-par/DESIGN.md](../../plans/finished/06-typed-par/DESIGN.md) —
  cross-cutting decisions referenced from every phase: Stitch
  policy enum, parent-store relationship, type spectrum, browser
  rebase across the worker boundary.
- [plans/finished/06-typed-par/PRIORITY.md](../../plans/finished/06-typed-par/PRIORITY.md)
  — historical spine; superseded by ARC.md but retained as the
  record of how steps 1–7 were sequenced.
- [bench/11_par/bench.loft](../../../../bench/11_par/bench.loft) — the
  100 K-element × 50-iter Newton's-sqrt workload that gates ±5 %
  regression at every @PLAN06 step boundary.
