# Performance Analysis

This document records current benchmark results, a root-cause analysis of every
performance gap relative to CPython and hand-written Rust, and a prioritised
improvement plan tied to the existing PLANNING.md backlog.

---

## Contents

- [Benchmark results](#benchmark-results)
- [Interpreter vs Python](#interpreter-vs-python)
- [Native vs Rust](#native-vs-rust)
- [wasm vs native](#wasm-vs-native)
- [Improvement priorities](#improvement-priorities)
- [See also](#see-also)

---

## Benchmark results

All times are wall-clock milliseconds, best of one warm run, single core,
Linux x86-64. Run `bench/run_bench.sh` from the project root to reproduce.

| # | Benchmark | Python | interp | native | wasm | Rust | interp/Py | native/Rust |
|---|-----------|-------:|-------:|-------:|-----:|-----:|----------:|------------:|
| 01 | fibonacci (recursive, n=38)      | 3395 | 4819  | 169 | 257 |  92 | 1.42× | 1.84× |
| 02 | sum loop (10 M integers)         |   66 |  584  |  15 |  21 |   8 | 8.85× | 1.88× |
| 03 | prime sieve (n=100 000)          |   49 |  141  |   4 |   6 |   4 | 2.88× | 1.00× |
| 04 | Collatz lengths (1 .. 1 M)       | 7393 | 14379 | 334 | 599 | 149 | 1.94× | 2.24× |
| 05 | Mandelbrot (200×200, 256 iter)   |  135 |  344  |   7 |  10 |   6 | 2.55× | 1.17× |
| 06 | Newton sqrt (1 M calls)          | 1481 | 3437  | 159 | 159 | 152 | 2.32× | 1.05× |
| 07 | string build (500 K appends)     |   70 |   61  |  33 |  68 |  23 | **0.87×** | 1.43× |
| 08 | word frequency (hash map)        |   46 |  169  |  32 |  60 |   2 | 3.67× | 16.0× |
| 09 | dot product (5 M floats)         |  158 |  428  |  36 |  86 |   3 | 2.71× | 12.0× |
| 10 | insertion sort (3 000 integers)  |  131 |  291  |  29 |  56 |   4 | 2.22× | 7.25× |

Ratios below 2× are expected for an interpreter that has not been tuned yet.
Ratios above 5× in native mode signal a structural problem.

---

## Interpreter vs Python

### Summary table

| Group | Benchmarks | Typical ratio | Root cause |
|---|---|---|---|
| Tight integer loops | 02, 04 | 2–9× | Dispatch + null-sentinel per op |
| Recursive calls | 01, 06 | 1.4–2.3× | Per-frame stack allocation |
| Float loops | 05, 09 | 2.5–2.7× | Same dispatch; float ops cheaper |
| Struct/collection | 08, 10 | 2.2–3.7× | Store indirection on every access |
| String building | 07 | **0.87×** | loft format-strings beat CPython object churn |

### Root causes (interpreter)

**1. Match-based bytecode dispatch**

Each execution cycle in `src/fill.rs` is a Rust `match` over a u8 opcode, then a
second match (or trait method) over the value type. CPython uses a C-level computed-goto
switch table with no Rust enum overhead. The cost is constant per opcode but accumulates
in hot loops. The sum-loop benchmark (8.85×) is the extreme: almost every cycle is a
dispatch + type-check with no opportunity to cache or merge opcodes.

**2. Null-sentinel checks on every `long` operation**

`long` uses `i64::MIN` as a null sentinel. Every arithmetic opcode in `fill.rs` checks
for it. Collatz (04) uses `long` for range safety; this adds roughly 1 branch per
arithmetic op. Benchmarks using `integer` (i32) avoid this check.

**3. Per-frame `Vec` allocation for stack frames**

`src/state/mod.rs` allocates a new `Vec<i64>` for each function call's work area.
Fibonacci (01) makes ~39 million recursive calls. Even with short-circuit reuse, the
allocator pressure is visible.

**4. Store-pointer indirection for collections**

Vector element access requires dereferencing a `DbRef` (store_nr, rec, pos) through
`Stores`. A CPython list element access is one C pointer dereference. The word-count
(08) and matrix (09) benchmarks reveal this: Python is faster on hash maps and dot
products because CPython's internal structures are direct C arrays / dicts.

---

## Native vs Rust

### Summary table

| Group | Benchmarks | Typical ratio | Root cause |
|---|---|---|---|
| Pure float compute | 05, 06 | 1.0–1.2× | Near parity — codegen is correct |
| Integer compute | 01, 02, 04, 10 | 1.8–7.3× | codegen_runtime call overhead |
| Data structures | 08, 09 | 7–16× | Runtime helpers vs idiomatic Rust |

### Root causes (native)

**1. `codegen_runtime` helper calls for every collection operation**

The native pipeline (`src/generation.rs`) emits calls to functions in
`src/codegen_runtime.rs` for hash lookup, vector indexing, vector sort, and string
operations. These helpers perform:
- Bounds-check
- Null-sentinel test
- `DbRef` store-pointer indirection
- Type-dispatch

Hand-written Rust performs none of these for a `Vec<i64>` or `HashMap<String, u32>`.
The word-count gap (16×) and dot-product gap (12×) are almost entirely this overhead.

**2. Recursive function call overhead**

Each generated function takes `stores: &mut Stores` and a `codegen_runtime::Frame`
context even for pure functions that never touch the heap. Fibonacci (1.84× gap)
suffers from the combined cost of the context parameters and the inability of `rustc`
to eliminate them during inlining (the generated functions are too large).

**3. `long` null-sentinel in generated code**

The Collatz benchmark (2.24×) uses `long`. The generated Rust checks `i64::MIN`
before every arithmetic operation, matching the interpreter. Hand-written Rust uses
plain `u64` or wrapping arithmetic with no sentinel.

**4. Near parity on float workloads**

Newton sqrt (1.05×) and Mandelbrot (1.17×) show that for pure float compute the native
pipeline produces code that `rustc -O` optimises to near-identical machine code.
This is the target quality for other workloads.

---

## wasm vs native

| Benchmark | native | wasm | ratio | Note |
|---|---:|---:|---:|---|
| fibonacci       | 169 | 257 | 1.52× | Expected wasm overhead |
| sum loop        |  15 |  21 | 1.40× | Expected |
| sieve           |   4 |   6 | 1.50× | Expected |
| Collatz         | 334 | 599 | 1.79× | `long` sentinel + wasm i64 cost |
| Mandelbrot      |   7 |  10 | 1.43× | Expected |
| Newton sqrt     | 159 | 159 | **1.00×** | FPU bound; wasm matches native |
| string build    |  33 |  68 | 2.06× | wasm memory model for strings |
| word frequency  |  32 |  60 | 1.88× | Hash indirection in linear memory |
| dot product     |  36 |  86 | 2.39× | wasm f64 memory layout |
| insertion sort  |  29 |  56 | 1.93× | wasm indirect memory for vector |

The 1.4–1.8× overhead on compute-bound benchmarks is structural wasm cost (linear memory
indirection, no SIMD). The 2× string overhead is a known design limitation (see PLANNING.md
for a planned wasm string optimisation). FPU-bound code (Newton sqrt) reaches parity because
the FPU is the bottleneck, not the memory model.

---

## Improvement priorities

Each item links to the mechanism, the affected benchmarks, the expected gain, and the
implementation pointer.

### P1 — Superinstruction merging in the interpreter (High impact, Medium cost)

**Affected benchmarks:** 02 (8.85×), 04 (1.94×), all tight loops.
**Expected gain:** 2–3× on integer-heavy loops; closes the sum-loop gap from 8.85× to ~3×.

The interpreter currently dispatches one opcode per cycle. Common patterns in compiled
bytecode (`LoadSlot` → `LoadSlot` → `AddInteger` → `StoreSlot`) could be merged into a
single superinstruction that performs all four steps in one Rust match arm. This
eliminates 3 out of 4 dispatch cycles.

**Implementation sketch:**
- Add a peephole pass in `src/compile.rs` after bytecode emission that replaces
  common 2–4 opcode sequences with a new `SuperXxx` opcode.
- Add corresponding `SuperXxx` arms to `fill.rs`.
- Start with the patterns that appear in the benchmarks (load+op+store combos).

**CPython context:** CPython 3.11+ uses "specialising adaptive interpreter" (PEP 659)
that rewrites opcodes inline at runtime. Superinstructions are a compile-time analogue
that is simpler to implement and sufficient for the current gap.

---

### P2 — Stack frame arena / reuse (Medium impact, Medium cost)

**Affected benchmarks:** 01 (1.42× vs Python), 06 (2.32×).
**Expected gain:** 15–30% on call-heavy benchmarks.

Every function call allocates a `Vec<i64>` work area. A simple arena (a `Vec<Vec<i64>>`
free-list keyed by capacity) would allow frame reuse without touching the allocator.

**Implementation pointer:** `src/state/mod.rs` `execute()` — the work-area push/pop
sequence at the top and bottom of each call frame.

---

### P3 — Eliminate null-sentinel for `integer` paths (Low cost, Low–Medium impact)

**Affected benchmarks:** 02, 04, 10.
**Expected gain:** 5–10% on integer loops; larger gain when combined with superinstructions.

`integer` (i32) arithmetic currently goes through the same opcode handlers as `long`
because of a shared dispatch path. Separate `OpAddInteger` / `OpAddLong` variants
already exist in the bytecode; confirm that the `integer` variants never check
`i64::MIN` and add a compile-time test to enforce this.

---

### N1 — Native: eliminate `codegen_runtime` for in-function vectors and hashes
      (Very high impact, High cost)

**Affected benchmarks:** 08 (16×), 09 (12×), 10 (7.25×).
**Expected gain:** 5–15× on data-structure benchmarks; closes the native/Rust gap for
most workloads.

For vectors and hash maps that are:
- declared locally within a function, and
- never passed to another function by reference, and
- never stored in a `Store`

…emit direct Rust `Vec<T>` / `HashMap<K, V>` operations instead of `codegen_runtime`
helper calls. The alias analysis needed is simple: a local variable that is never
assigned to a `Store` slot and whose address is never taken.

**Implementation pointer:** `src/generation.rs` — the code-generation pass that emits
Rust for each IR node. Add a pre-pass that marks qualifying locals as "direct", then
emit `vec[i]` / `map[key]` directly.

**Prerequisite:** A12 (lazy work-variable initialisation) may interact with local
variable lifetime; review before implementing.

---

### N2 — Native: reduce per-call context for pure functions (Medium impact, High cost)

**Affected benchmarks:** 01 (1.84×), 06 (1.05×).
**Expected gain:** 10–30% on recursive benchmarks.

Functions that provably do not read or write any `Store` and do not call any function
that does can be emitted without the `stores: &mut Stores` parameter and the
`Frame` context. `rustc` can then inline and optimise them freely.

**Implementation pointer:** `src/generation.rs` — add a purity analysis pass over
the IR before code generation. Mark pure functions; emit a lean signature for them.

---

### N3 — Native: remove `long` null-sentinel in generated code (Low cost, Medium impact)

**Affected benchmarks:** 04 (2.24×).
**Expected gain:** up to 1.5× on `long`-heavy benchmarks.

In generated Rust code, replace `i64::MIN`-sentinel arithmetic with direct `i64`
arithmetic wrapped in `Option<i64>` or by using a separate null-flag. `rustc` can
optimise `Option<i64>` down to a single register on most paths.

Alternatively, restrict null-sentinel checks to Store I/O boundaries only and use
plain `i64` internally in generated code.

---

### W1 — wasm: native string representation (Low impact, Medium cost)

**Affected benchmark:** 07 (2.06× vs native).
**Expected gain:** Close the 2× string-building gap to <1.1×.

The wasm build represents dynamic strings as heap-allocated structures inside wasm
linear memory, with an extra indirection compared to native Rust `String`. Use
wasm-native string representation (a pointer + length pair in linear memory backed by
`memory.grow`) to eliminate the indirection.

---

## Improvement priority order

| Priority | Item | Expected gain (interp) | Expected gain (native) | Cost |
|---|---|---|---|---|
| 1 | P1 — Superinstructions | 2–3× on tight loops | — | Medium |
| 2 | N1 — Direct collection emit | — | 5–15× data-struct benchmarks | High |
| 3 | P2 — Frame arena | 15–30% call-heavy | — | Medium |
| 4 | N2 — Pure-function context | — | 10–30% recursive | High |
| 5 | N3 — Long sentinel in codegen | — | ~1.5× Collatz | Low |
| 6 | P3 — Integer path sentinel | 5–10% int loops | — | Low |
| 7 | W1 — wasm strings | — | — | Medium |

Items 1–3 have the largest expected impact per engineering hour and should be scheduled
after the current 0.8.3 language-syntax milestone.

---

## See also

- [OPTIMISATIONS.md](OPTIMISATIONS.md) — runtime optimisation opportunities audit
- [PLANNING.md](PLANNING.md) — priority-ordered enhancement backlog
- [INTERNALS.md](INTERNALS.md) — `src/fill.rs`, `src/state/`, `src/generation.rs`
- [NATIVE.md](NATIVE.md) — native code generation design and known issues
- [doc/00-performance.html](../00-performance.html) — rendered benchmark page with bar charts
