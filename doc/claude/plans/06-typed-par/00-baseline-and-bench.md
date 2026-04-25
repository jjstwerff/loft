<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0 — Baseline and benchmark

**Status: open**

## Goal

Pin down the current `par` behaviour with characterisation tests and
record a perf baseline.  Every later phase compares against this
baseline; a regression past ±5 % blocks the phase.

This phase ships **no implementation change**.  Its only deliverable
is the safety net that lets phases 1–6 refactor confidently.

## Why this is phase 0, not phase 1

Two reasons:

1. **The current behaviour is the hardest spec to recover.**  `par`
   has 7 runtime variants and 3 native-codegen dispatch arms; each
   has subtle edge cases (empty input, single-thread, max-thread
   exceeding rayon's pool, text with embedded NULs, references
   pointing at stores the worker can't see).  If we don't pin them
   first, a phase-1 refactor that "looks fine" on the existing
   tests can silently break a niche path.

2. **Without a measured baseline, the phase-1 store-stitch path
   is impossible to validate.**  The whole pitch ("everything is a
   store") loses its appeal if it costs 30 % more wall-clock time.
   The bench in this phase IS the validation harness.

## Deliverables

1. **`tests/threading_chars.rs` — characterisation suite.**  20–30
   tests covering the matrix below.  Each test runs through the
   public `par(...)` surface and asserts the *exact* current output;
   any future change that alters the output (even non-observable
   ones like store-id reassignment) must update the test deliberately.

2. **`tests/bench/par_baseline.rs` — perf baseline, gated behind
   `#[ignore]` by default.**  Three benchmarks (defined below) with
   a recorded wall-clock budget per platform.  Stored as expected
   ranges in `tests/bench/par_baseline.expected.json`.

3. **THREADING.md addition — "Phase 0 baseline" section** at the end,
   linking to the characterisation tests and citing the per-platform
   numbers.  Lets readers (and Claude in future sessions) see at a
   glance "this is the floor we cannot drop below".

## Characterisation matrix

The combinations are constructed by crossing four dimensions; we
need at least one test per cell that has actually-different code in
the current runtime.  Cells where the runtime path collapses are
marked "covered by …".

| Input element type | Output element type | Threads | Test |
|---|---|---|---|
| `vector<integer>` | integer (8 B) | 1 | `par_int_to_int_t1` |
| `vector<integer>` | integer (8 B) | 4 | `par_int_to_int_t4` |
| `vector<integer>` | float (8 B) | 4 | `par_int_to_float_t4` |
| `vector<integer>` | single (4 B) | 4 | `par_int_to_single_t4` |
| `vector<integer>` | i32 (4 B) | 4 | `par_int_to_i32_t4` |
| `vector<integer>` | u8 (1 B) | 4 | `par_int_to_byte_t4` |
| `vector<integer>` | boolean | 4 | `par_int_to_bool_t4` |
| `vector<integer>` | text | 4 | `par_int_to_text_t4` |
| `vector<text>` | integer | 4 | `par_text_to_int_t4` |
| `vector<text>` | text | 4 | `par_text_to_text_t4` |
| `vector<Struct>` | Struct (Reference) | 4 | `par_struct_to_struct_t4` |
| `vector<integer>` | Struct (Reference) | 4 | `par_int_to_struct_t4` |
| `vector<integer>` | enum (1 B disc) | 4 | `par_int_to_enum_t4` |
| `vector<integer>` | struct-enum (variant w/ fields) | 4 | `par_int_to_struct_enum_t4` |
| `vector<integer>` (empty) | integer | 4 | `par_empty_input` |
| `vector<integer>` (1 element) | integer | 4 | `par_single_element_t4` (worker count clamps to 1) |
| `vector<integer>` (1000 elements) | integer | 16 | `par_max_threads` (over-provisioned) |

Form coverage:

- **Form 1** (function call: `par(input, my_fn)`) — every row above.
- **Form 2** (method: `par(input, .my_method)` — bound to an extra
  receiver argument) — at least one row in `par_form2_method`.
- **Form 3** (captured receiver, currently rejected at parse time per
  `parser/builtins.rs:229`) — one negative test
  `par_form3_rejected_at_parse_time` asserting the error message.

`par_light(...)` coverage:

- The auto-light heuristic (`check_light_eligible` in
  `parser/builtins.rs:362`) selects light when the worker has a
  primitive return AND no recursive store allocation.  Test the
  positive and negative cases:
  - `par_light_auto_selected_for_primitive` — assert the codegen
    chose the light path (visible by emitting a different opcode in
    the dump).
  - `par_full_when_worker_allocates` — assert the codegen chose the
    full path when the worker calls `vector_add` or similar.

## Bench design

Three workloads in `tests/bench/par_baseline.rs`:

1. **`bench_par_int_pure`** — 1 M elements, worker is `|x| x * 2 + 1`,
   pure compute, primitive-return.  Measures the direct-write path
   (currently `n_parallel_for_native`'s 8-byte arm).
2. **`bench_par_struct_alloc`** — 100 K elements, worker constructs
   and returns `Point { x: i, y: i + 1 }`.  Measures the reference
   path (`n_parallel_for_ref_native`).
3. **`bench_par_text_concat`** — 100 K elements, worker returns
   `"item-{i}"`.  Measures the text path (`n_parallel_for_text_native`).

Each benchmark records:
- median wall-clock time over 10 runs,
- 95th percentile,
- threads used (clamped to `min(rayon_pool, 4)` for reproducibility).

Stored in `tests/bench/par_baseline.expected.json`:

```json
{
  "bench_par_int_pure":      { "median_ms": 12.5, "p95_ms": 14.0, "threads": 4 },
  "bench_par_struct_alloc":  { "median_ms": 28.0, "p95_ms": 31.5, "threads": 4 },
  "bench_par_text_concat":   { "median_ms": 35.0, "p95_ms": 38.0, "threads": 4 }
}
```

Numbers are placeholders; phase 0 fills them in from the actual
baseline machine (the loft project's primary CI host).  Subsequent
phases run the same benches on the same host and assert
`median ≤ 1.05 * baseline_median`.  A failure does not auto-block —
the plan author investigates and decides whether to accept the regression
(rare) or rework the phase (default).

## Implementation

Phase 0 has three commits, each landed independently:

### Commit 0a — characterisation suite

- `tests/threading_chars.rs`: 20–30 functions matching the matrix
  above.  Each uses `cargo test --release --test threading_chars`
  and exercises `par` through a `code!()` macro snippet — same
  shape as `tests/issues.rs`.
- Each test asserts the full `Vec<T>` output, not just the length
  or first element.  Matters for catching reordering bugs.
- Run `make ci` and confirm green.

### Commit 0b — bench harness

- `tests/bench/par_baseline.rs`: Criterion-style microbench, gated
  behind `#[ignore]` so it doesn't run in `make ci` by default.
- `make bench-par` target in the Makefile that runs all three
  benchmarks under `--ignored` and writes results to
  `target/bench/par_baseline.json`.
- Comparison helper script `scripts/compare_par_bench.sh` that
  reads `target/bench/par_baseline.json`, compares against
  `tests/bench/par_baseline.expected.json`, and exits non-zero on
  >5 % regression.

### Commit 0c — record the baseline

- Run `make bench-par` on the primary CI host.
- Write the actual numbers into
  `tests/bench/par_baseline.expected.json`.
- Append a "Phase 0 baseline" section to THREADING.md citing the
  numbers and pointing at the harness.
- Add a note to the plan-06 README marking phase 0 done.

### 0d — Type-coverage commitments tracker

Phase 0a's characterisation suite covers what works **today**.
DESIGN.md D11 lists the **full type spectrum** par must accept after
plan-06.  Several shapes in D11 don't work today; some don't even
have a canary test yet.

Phase 0d adds a **single tracking table** in this file that lists
every shape from D11 with its current status, the test name (existing
or planned), and the phase that's expected to close it.  The table
is the contract: every phase commit must update the relevant rows.

| Shape | Today | Test name | Closes |
|---|---|---|---|
| **Inputs** | | | |
| `vector<Struct>` | ✅ | `par_struct_to_*` (positive) | — |
| `vector<integer>` | ❌ garbage | `par_int_to_int_t4_primitive_input` (`#[ignore]`) | phase 4 |
| `vector<float>` | ❌ | `par_float_input_t4` (`#[ignore]`) | phase 4 |
| `vector<i32>` | ❌ | `par_i32_input_t4` (`#[ignore]`) | phase 4 |
| `vector<u8>` | ❌ | `par_u8_input_t4` (`#[ignore]`) | phase 4 |
| `vector<text>` | ❌ | `par_text_input_t4` (`#[ignore]`) | phase 4 |
| `vector<EnumTag>` (plain enum, 1-byte) | ❓ untested | **add** `par_enum_input_t4` (`#[ignore]`) | phase 1 |
| `vector<Reference<X>>` (vector of refs) | ❓ untested | **add** `par_vec_of_refs_input_t4` (`#[ignore]`) | phase 1 |
| `vector<vector<T>>` (nested) | ❓ untested | **add** `par_nested_vector_input_t4` (`#[ignore]`) | phase 4 |
| `vector<fn(T) -> U>` (vector of fn-refs) | ❓ untested | **add** `par_vec_of_fns_input_t4` (`#[ignore]`) | phase 1 |
| `sorted<T[key]>` | ❓ untested | **add** `par_sorted_input_t4` (`#[ignore]`) | phase 4 |
| `hash<T[key]>` | ❓ untested | **add** `par_hash_input_t4` (`#[ignore]`) | phase 4 |
| `index<T[key]>` | ❓ untested | **add** `par_index_input_t4` (`#[ignore]`) | phase 4 |
| **Outputs** | | | |
| primitive scalars (1–8 B) | ✅ | `par_struct_to_int/float/i32/byte/bool` etc. | — |
| text | ✅ | `par_struct_to_text_t4` | — |
| Reference<Struct> | ✅ | `par_struct_to_struct_t4` | — |
| Plain enum | ✅ | `par_struct_to_enum_t4` | — |
| StructEnum (variant w/ fields) | ❌ size > 8 | `par_struct_to_struct_enum_t4` (`#[ignore]`) | phase 1 |
| Large value-struct (size > 8) | ❌ | **add** `par_struct_to_large_struct_t4` (`#[ignore]`) | phase 1 |
| `vector<T>` (worker returns a vector) | ❌ | **add** `par_struct_to_vector_t4` (`#[ignore]`) | phase 1 |
| `hash<T>` / `sorted<T>` / `index<T>` | ❌ | **add** `par_struct_to_keyed_collection_t4` (`#[ignore]`) | phase 1 |
| fn-ref | ❌ | **add** `par_struct_to_fn_t4` (`#[ignore]`) | phase 1 |
| Optional<T> (nullable) | ❓ | **add** `par_struct_to_optional_t4` (`#[ignore]`) | phase 1 |
| Tuple (1.1+) | n/a | (skipped — type doesn't exist yet) | post-1.1 |
| **Negative cases** | | | |
| Cross-worker reference graph (worker A returns ref into B's output) | ❌ rejected | **add** `par_cross_worker_ref_rejected` (positive — asserts the diagnostic) | phase 1 (compile-time enforcement) |

### Per-phase obligation

When a phase commits, it must:

1. **Add the canary** (`#[ignore]`) for any shape from D11 it
   intends to close, if not already in `tests/threading_chars.rs`.
2. **Un-`#[ignore]`** every canary it closes; the test asserts the
   correct (positive) behaviour.
3. **Update the table above** — change the test name's marker from
   `#[ignore]` to "positive" and mark the row's "Today" column as ✅.

A phase that closes a shape without un-ignoring its canary fails its
own acceptance criterion: the gap is unverified.  A phase that adds
a new canary (one that didn't exist before) updates the table to
record it.

### How phase 0d ships

Phase 0d's commit (separate from 0a/0b/0c) adds the **canaries that
don't yet exist** in `tests/threading_chars.rs` — every row in the
table marked **add** + `#[ignore]`.  This pre-populates the file
so each subsequent phase has a concrete entry to un-ignore, rather
than authoring fresh tests as it goes.

Counts:

- 4 new input-side canaries (`par_enum_input_t4`,
  `par_vec_of_refs_input_t4`, `par_nested_vector_input_t4`,
  `par_vec_of_fns_input_t4`).
- 3 keyed-collection input canaries (`par_sorted_input_t4`,
  `par_hash_input_t4`, `par_index_input_t4`).
- 5 output-side canaries (`par_struct_to_large_struct_t4`,
  `par_struct_to_vector_t4`, `par_struct_to_keyed_collection_t4`,
  `par_struct_to_fn_t4`, `par_struct_to_optional_t4`).
- 1 negative case (`par_cross_worker_ref_rejected`).

Total: 13 new `#[ignore]`d canaries, all asserting the eventual
correct behaviour — they fail today (the shape doesn't compile or
gives garbage) and become passing tests when each phase lands.

After 0d, the file has:
- 16 working positive tests (today's correct behaviour).
- 6 today-broken canaries (5 G2 primitive-input + 1 G1 struct-enum
  return) — already in place after 0a.
- 13 not-yet-tested canaries — added by 0d.

That's **35 entries total** covering D11's full spectrum.

## Acceptance criteria

- All 20–30 characterisation tests pass on Linux x86_64, macOS
  aarch64, Windows MSVC.
- `make bench-par` runs in under 60 s end-to-end.
- `scripts/compare_par_bench.sh` passes with the recorded baseline.
- `make ci` green; no test count regression.
- Phase 0d's tracker table covers every shape in DESIGN.md D11.
- Every D11 shape has either a positive test (working today) or
  an `#[ignore]`d canary (with a phase number in the ignore reason).

## Risks

- **Bench variance.**  Wall-clock time on shared CI runners is
  noisy.  Mitigation: 10-run median + 95th-percentile reporting;
  ±5 % threshold gives slack for normal noise but catches real
  regressions.  If the CI runner shows >2 % run-to-run variance
  even on the baseline, raise the threshold to ±10 % and document
  it explicitly rather than ignore it.
- **Form 3 stays unimplemented.**  The negative test pins the parse
  error.  When phase 4 lands typed input/output, Form 3 may become
  trivial to implement; the negative test will need updating.  Not
  blocking phase 0.
- **Auto-light test brittleness.**  Asserting "the codegen chose the
  light path" requires inspecting the IR or the emitted bytecode.
  Use the existing `LOFT_LOG=static` dump and grep for `Op*Light`
  vs. `Op*Full` opcodes.  If the opcode names change in a later
  phase, update the assertion text deliberately.

## Out of scope (deferred to later phases)

- No new runtime code paths.
- No changes to `src/parallel.rs`, `src/codegen_runtime.rs`, or
  `src/parser/builtins.rs`.  Phase 0 only adds tests and a bench.
- No changes to the loft surface or `default/01_code.loft`.
- No design discussion of phase 1's store-stitching — that lives in
  `01-output-store.md`.

## Hand-off to phase 1

Phase 1 begins with the characterisation suite in place and the
baseline numbers recorded.  The phase-1 PR description cites the
phase-0 commit hash so reviewers can verify the safety net is the
one being protected.
