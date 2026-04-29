<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-06 — fast-track order by complexity reduction

The original plan-06 phases are numbered by topic, not by impact.
This doc reorders them as a **single sequential spine** — each step
chosen to retire the most complexity per unit effort.  Anything not
on the spine is explicitly deferred.

## Why a reorder

The materialised result vector is the load-bearing source of par's
internal complexity:

- 3 native fns (`n_parallel_for_native` / `_text_native` / `_ref_native`)
- 6 runtime variants in `src/parallel.rs`
- 3 dispatch arms in `src/generation/dispatch.rs`
- copy_block + copy_claims deep-copy infrastructure (~600 lines)
- StoreRebase + rebase_walk machinery (just to make the materialised
  path fast)
- result-vector layout debate (Path A vs Path B blocking phase 2b)
- per-element Stitch policy decisions
- text encoding subtleties (4-byte vs 16-byte Str)
- `par` vs `par_light` user surface split

Phase 10 (drop materialised vector) eliminates **all of these in one
strategic move**.  The fastest path to a smaller par codebase routes
through phase 10 as quickly as possible; everything else is either a
phase-10 prerequisite or deferred until after the simplification.

## The spine — 10 steps in order

Each step is sized as a **single session's work**.  Net complexity
impact is given as a count of branches collapsed, lines retired, or
new code added (negative = good).  Run `make ci` after each step.

### Step 1 — Audit phase-1 invariants; reality-check the canaries (DONE 2026-04-29)

**Effort: XS** · **Net Δ lines: doc-only** · **Branches collapsed: 0**

Audit of `tests/threading_chars.rs` against the README's "4 phase-1
canaries remain" claim:

| Canary | Status | Notes |
|---|---|---|
| text-output | ✅ closed | No `#[ignore]` for text-output remains in the file. |
| keyed-collection-input dispatch | ✅ closed | Closed in phase 4d.B (`materialise_keyed_for_par`). |
| fn-ref return finish | ⏸ deferred | `par-fn-return` (line 686) — closure DbRef sentinel offset bug; stack-pos tracker mismatch in codegen.  Deep codegen work, belongs to phase 4 (typed surface) or phase 11 (par_to_vec). |
| vector-return Stitch | ⏸ deferred | `par-vector-return` (line 639) — vector-returning workers need hidden-arg per-worker destination machinery; lands as part of phase 7's fused for-par work, not as a phase-1 invariant fix. |

**Conclusion:** the per-worker output store foundation is already in
place for production paths.  The two deferred canaries belong to
later phases by design — they're return-shape support, not
invariant lockdown.  Step 1 collapses to a documentation update;
the spine effectively starts at step 2 (Stitch::Discard).

**Files updated:** this doc; README phase 1 entry; canary comments
re-tagged to point at the final-resting-phase (4 / 7 / 11) instead
of "phase 1 leftover".

### Step 2 — Stitch::Discard runtime (DONE 2026-04-29)

**Effort: S** · **Net Δ lines: +75** · **Branches collapsed: 0** (new code)

Added `run_parallel_discard(stores, program, fn_pos, input,
element_size, n_threads, extra_args, return_size)` to
`src/parallel.rs` — workers run, return value `let _`-bound on each
row, no allocation, no order preservation, no merge.  Worker fn runs
for its side effects (host_io / log_*).

3 regression tests in `tests/threading.rs::par_discard_*`:
- `par_discard_runs_without_panic` (50 elements, 4 threads)
- `par_discard_empty_input` (no-op short-circuit)
- `par_discard_does_not_grow_parent_stores` (locks in the invariant
  step 8's Concat retirement depends on)

Marked `#[allow(dead_code)]` — step 3 is the first call-site
consumer.

**Files:** `src/parallel.rs`, `tests/threading.rs`.

### Step 3 — Fused for-par with Discard detection

**Effort: M** · **Net Δ lines: +200, -0** · **Branches collapsed: 0** (new IR shape)

Implement `Value::ParFor` IR node + parser-side desugar for
`for x in input par(r=fn(x), threads) { body(r) }`.  Body-shape
analysis decides Stitch policy: if body never references `r`, emit
`ParFor { stitch: Discard }`.  Other policies stay TODO until later
steps.

**Why third:** phase 7's foundation; everything from step 5 onward
lowers to `Value::ParFor`.

**Files:** `src/data.rs` (new IR variant), `src/parser/control.rs`
(parse_for fused detection), `src/state/codegen.rs` (gen_par_for),
new tests in `tests/threading.rs::par_fused_for_*`.

**Sub-phasing:**

- **3a (DONE 2026-04-29)** — `Value::ParFor(Box<ParForBody>)` variant
  added to `src/data.rs` with the field shape from DESIGN.md D7.
  Walker arms in 9 sites; codegen panics if reached.  2 unit tests.
- **3b (DONE 2026-04-29)** — `n_parallel_discard` native fn added
  to `default/01_code.loft` + `src/native.rs`, wraps
  `run_parallel_discard` from step 2.  Codegen extended to push
  extras + subtract on cleanup for the new fn name.
- **3c (DONE 2026-04-29)** — parser detection of empty-body
  fused-for-par in `parse_parallel_for_loop`: when `block` is empty
  AND `extra_args` is empty, lower directly to
  `Value::Call(n_parallel_discard, args)` instead of building the
  materialised IR.  Bypasses the result-vector allocation, the
  result-element accessor rewrites, and the for-loop wrapper.
  Lowering at parse time (not via `Value::ParFor` codegen) is the
  cleanest minimum — the IR variant from 3a stays for steps 4+
  where scope-analysis-policy interactions matter.  Test:
  `par_fused_empty_body_runs_through_discard` in tests/threading.rs.

  **Tighter detection conditions are deferred:** today only the
  empty-body case routes here.  Bodies that reference only the
  loop variable (not `r`) — e.g. `{ log("done") }` — will lower in
  step 5 (value-position par lowering with use-site walk).

### Step 4 — Stitch::Queue runtime (DONE 2026-04-29)

**Effort: M** · **Net Δ lines: +90** · **Branches collapsed: 0** (new code)

Added `run_parallel_queue(stores, program, fn_pos, input,
element_size, n_threads, extra_args, return_size) -> Vec<u64>` to
`src/parallel.rs`.  Order-preserving: result `i` of the returned
Vec is the worker's output for input row `i`, regardless of how
rayon dispatched workers.  Returns raw u64 bits per row; caller
interprets per `return_size` (1 / 4 / 8).

The MVP collects every batch before returning — true streaming
(workers running while main consumes via bounded queue +
backpressure) is a follow-up once codegen wiring stabilises.  This
shape matches `run_parallel_int` but with the configurable
`return_size` and `extra_args` payload that step 5's value-position
lowering will use.

4 regression tests in `tests/threading.rs::par_queue_*`:
- `par_queue_returns_results_in_input_order` — 30 elements, 4
  threads, every slot matches `worker(input[i])`.
- `par_queue_empty_input` — no-op short-circuit.
- `par_queue_does_not_grow_parent_stores` — invariant lock-in for
  step 8's Concat retirement.
- `par_queue_single_thread_matches_multi` — order invariant holds
  across thread counts.

Marked `#[allow(dead_code)]` — step 5 is the first call-site
consumer.  Until step 5 wires it, the runtime is exercised only by
Rust tests.

**Files:** `src/parallel.rs`, `tests/threading.rs`.

### Step 5 — Lower value-position par to ParFor + warn on materialise (5a DONE 2026-04-29)

**Effort: M** · **Net Δ lines: +200** · **Branches collapsed: 0** (no retirement yet)

Parser-side desugaring of `let r = parallel_for(input, fn, threads)`
followed by single-pass consumption.  Two checks:

1. Walk the result variable's downstream uses.  If single-pass
   (one `for r in result` loop, one fold, one `par_for_each`),
   lower to `Value::ParFor` with the appropriate Stitch policy.
2. If any use is materialising (`r[i]`, multi-pass, store in field,
   etc.), emit `Level::Warning` with a "did you mean
   `par_to_vec(...)` for the explicit materialised form" hint.

Warning, not error — gives a deprecation window for porting the
test suite.

**Sub-phasing:**

- **5a (DONE 2026-04-29)** — use-site analyser + warning.  Added
  `Parser::check_par_result_singlepass` (called at end-of-function
  on the second pass alongside `check_ref_mutations`).  Walks the
  body via two helpers:
  - `collect_par_assignments` — finds every `Set(v, Call(n_parallel_for, ...))`
    site, recursing through every compound IR variant.
  - `classify_var_uses` — counts a `Var(v)` read as **streaming** when
    it appears in `Iter`'s init slot or `ParFor`'s input slot, **other**
    everywhere else.  Recurses through every variant.

  Compiler-generated bindings (names starting with `_`, e.g. the
  `_par_results_N` synthetic var the fused for-par desugar emits) are
  skipped — the warning targets user `let r = ...` patterns, not the
  internal materialised IR step 8 will retire.

  When `other > 0` for a user var, emits `Level::Warning` with the
  full migration message (point at fused for-par or future
  `par_to_vec(...)`).

  2 tests in `tests/threading.rs`:
  - `par_result_random_access_emits_materialise_warning` — confirms
    the warning fires + names the user var.
  - `par_result_warning_skips_compiler_generated_bindings` — confirms
    fused for-par's hidden binding does NOT trigger the warning.

- **5b (open)** — actual lowering of streaming-eligible cases.  When
  `streaming == 1` AND `other == 0`, rewrite `let r = parallel_for(...);
  for x in r { body }` to `Value::ParFor { stitch_id: 3 (Queue) }` +
  body, dispatching through `run_parallel_queue` from spine step 4.
  Replaces the materialised `n_parallel_for` call in this case.

### Step 6 — Port test suite to streaming forms

**Effort: M** · **Net Δ lines: -100, +200** (test code reshape)

Audit every test in `tests/threading.rs`, `tests/threading_chars.rs`,
`bench/11_par/`, every `.loft` script that calls `parallel_for`.
For each:

- If the test consumes the result single-pass → already works
  under step 5's auto-lowering; no change.
- If the test materialises (random access, etc.) → either rewrite
  to streaming form (preferred — matches user-facing direction),
  or annotate as `par_to_vec`-required for the future phase 11.

**Why sixth:** must port before promoting the warning to error.
Keeps step 7 from breaking the build.

**Files:** test files only.

### Step 7 — Promote warning to error

**Effort: XS** · **Net Δ lines: 1 word** · **Branches collapsed: 0** (gates retirement)

Flip `Level::Warning` to `Level::Error` in step 5's check.
Materialising par results now fails to compile.  Test suite from
step 6 stays green.

**Files:** `src/parser/control.rs` one line.

### Step 8 — Retire Stitch::Concat runtime + parallel_execute_and_collect

**Effort: S** · **Net Δ lines: -250, +20** · **Branches collapsed: 6**

Delete:

- `parallel_execute_and_collect` (~170 lines in `src/native.rs`).
- The Stitch::Concat arm in dispatch.
- `run_parallel_ref`'s batch-return shape; replace with Queue path.
- `run_parallel_text`'s string-vector-return shape; replace with Queue path.
- The `result_db = stores.null()` allocation at the par dispatcher's top.

`copy_from_worker[_unowned]` helpers stay in `src/database/allocation.rs`
(they have non-par callers if any; phase 11's `par_to_vec` will
re-import them).

**Why eighth:** first big complexity drop.  After this, par's runtime
shape is **3 stitch variants in one dispatcher** instead of 6+
runtime variants × 3 dispatch arms.

**Files:** `src/native.rs`, `src/parallel.rs`, `src/codegen_runtime.rs`.

### Step 9 — Stitch::Reduce runtime + par_fold surface

**Effort: M** · **Net Δ lines: +150, -0** · **Branches collapsed: 0** (new policy + sugar)

Add the Reduce policy: each worker keeps a partial accumulator;
main thread combines per-worker partials with the user-supplied
fold operator.  Surface `par_fold(input, init, fold, threads) -> U`
as a stdlib fn that compiles to `Value::ParFor { stitch: Reduce }`.

The parser also auto-detects `total = sum(parallel_for(...))` and
similar fold patterns from step 5's walker, lowering them to Reduce
without an explicit `par_fold` call.

**Why ninth:** Reduce closes the third Stitch policy.  After this,
all common par shapes have a non-materialising runtime path.

**Files:** `src/parallel.rs::run_parallel_reduce`, parser
auto-detection, `default/01_code.loft` declarations,
`tests/threading.rs::par_fold_*`.

### Step 10 — Cleanup pass: retire dead code, rewrite docs

**Effort: XS** · **Net Δ lines: -900** · **Branches collapsed: more**

After steps 1–9 land, several pieces are dead:

- `n_parallel_for_native`, `_text_native`, `_ref_native` (3 fns) →
  collapse to one `n_parallel_native(stitch_id)` opcode + dispatcher.
- `run_parallel_raw`, `run_parallel_int`, `run_parallel_light` → folded
  into Queue / Discard / Reduce.
- `parallel_get_int` / `_long` / `_float` / `_bool` getters → unused.
- `par` / `par_light` user split → `par_light` removed.
- ~70 lines of `default/01_code.loft` declarations.

Rewrite `doc/claude/THREADING.md` to reflect the streaming-only model.
Update `CHANGELOG.md` + `CHANGELOG_TECHNICAL.md`.

**Why tenth:** can only land after dead-code is provably unreachable
(steps 1–9 close every other consumer).

**Files:** `src/parallel.rs`, `src/codegen_runtime.rs`,
`src/native.rs`, `default/01_code.loft`,
`doc/claude/THREADING.md`, CHANGELOG.

## Cumulative complexity reduction by step

| After step | Runtime variants | Native fns | Dispatch arms | User surfaces | Net LOC |
|---|---|---|---|---|---|
| Today | 6 | 3 | 3 | 2 (`par`/`par_light`) | baseline |
| Step 1 (audit done) | 6 | 3 | 3 | 2 | 0 (doc-only) |
| Step 2 (Discard runtime live) | 6 + 1 | 3 | 3 | 2 | +75 |
| Step 3 (ParFor IR + n_parallel_discard) | 6 + 1 | 4 | 3 | 2 | +470 |
| Step 4 (Queue runtime) | 6 + 2 | 4 | 3 | 2 | +560 |
| Step 5 (value-position lowering) | 6 + 2 | 4 | 3 | 2 | +660 |
| Step 7 (warning → error) | 6 + 2 | 3 | 3 | 2 | +500 |
| Step 8 (Concat retired) | 2 (Queue+Discard) | 1 | 1 | 1 | +250 |
| Step 9 (Reduce live) | 3 | 1 | 1 | 1 | +400 |
| Step 10 (cleanup) | 3 | 1 | 1 | 1 | **-500** |

After step 10: par's runtime is **one dispatcher, three stitch
policies, one user surface** — the simplification target plan-06's
intro paragraph promises.

## Explicitly deferred — not on the spine

These items remain in plan-06 but **do not block** any spine step.
Pick them up after step 10 lands or when a concrete user need surfaces.

| Item | Why deferred |
|---|---|
| Phase 2b/2c/2d/2e wiring | Folded into phase 11; rebase machinery becomes the materialiser's tool |
| Phase 4 typed input/output sub-phases (4a/4b/4d.A.2/4d.C/4e) | Type-system migration; not on the simplification path.  Phase 10's stream-only contract gives the type system a simpler shape to settle on. |
| Phase 5 fixed-point auto-light | Phase 10's IR walk subsumes the per-result-variable check.  Auto-light's parent-store Arc-wrapping (5c) is unrelated and stays open. |
| Phase 8 browser workers | Feature work, not simplification.  After step 10, the streaming model maps cleanly to Web Worker postMessage transfer. |
| Phase 9 tuple support | Feature work; orthogonal to the materialisation question. |
| Phase 11 `par_to_vec` opt-in | Add only when a real user needs the materialised vector.  Phase 2's machinery is ready; the helper itself is small. |
| Phase 2a-prep work in `src/parallel.rs` (StoreRebase + rebase_walk_record + adopt_worker_excess) | Already shipped (commits `7ab13ac` + `e95ab53`); kept as library code for phase 11.  Marked `#[allow(dead_code)]` until phase 11 wires it. |

## Critical path summary (for quick scan)

```
[1 audit + reality-check]       — XS, doc-only — DONE 2026-04-29
       ↓
[2 Stitch::Discard runtime]     — S, +75 LOC — DONE 2026-04-29
       ↓
[3 fused for-par + ParFor IR]   — M, +200 LOC — DONE 2026-04-29
   3a IR + walker arms          — DONE
   3b codegen for Discard       — DONE
   3c parser detection          — DONE
       ↓
[4 Stitch::Queue runtime]       — M, +90 LOC — DONE 2026-04-29
       ↓
[5 lower value-position par
   + warning on materialise]    — M, +200 LOC
   5a use-site analyser + warn  — DONE 2026-04-29
   5b lowering to Queue         — ← NEXT
       ↓
[6 port test suite]             — M, test reshape only
       ↓
[7 warning → error]             — XS, 1 line
       ↓
[8 retire Concat runtime]       — S, -250 LOC, biggest single drop
       ↓
[9 Stitch::Reduce + par_fold]   — M, +150 LOC
       ↓
[10 cleanup pass]               — XS, -900 LOC, finishes simplification
```

Total: ~9 sessions remaining (step 1 closed as doc-update).  Net
code retired: ~500 LOC (gross +800 added, -1300 retired).  Branches
collapsed: 6 runtime variants → 3, 3 native fns → 1, 3 dispatch
arms → 1, `par`/`par_light` → `par`.

The spine has **no out-of-order dependencies** — each step's
prerequisites are either the previous step or already-landed work.
A session that picks up the spine knows exactly what to do next by
reading this doc.
