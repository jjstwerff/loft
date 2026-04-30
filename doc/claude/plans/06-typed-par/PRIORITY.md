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

### Step 5 — Lower value-position par to ParFor + warn on materialise (DONE 2026-04-29)

**Effort: M** · **Net Δ lines: +260** · **Branches collapsed: 0** (no retirement yet)

Parser-side use-site analyser for `let r = parallel_for(input, fn,
threads)` patterns.  Two outcomes:

1. **Materialising use** (`r[i]`, multi-pass, store in field, etc.) →
   `Level::Warning` with a "use a fused for-par or `par_to_vec(...)`"
   hint.
2. **Unused result** (`r` bound but never read) → different
   `Level::Warning` pointing users at the discard fused form.
3. **Streaming-eligible** (single `for x in r {}` use) → silent
   today; will rewrite to `Stitch::Queue` dispatch in step 8.

Warning, not error — gives a deprecation window before step 7
promotes to error and step 8 retires the materialised runtime.

**Implementation (5a — DONE):**
- `Parser::check_par_result_singlepass` called at end-of-function
  on the second pass alongside `check_ref_mutations`.
- `collect_par_assignments` walks the body finding every
  `Set(v, Call(n_parallel_for, ...))` site.  Recurses through every
  compound IR variant (Block / Loop / If / Insert / Iter / Span /
  ParFor / Call / CallRef / Return / Drop / Yield / BreakWith /
  TuplePut).
- `classify_var_uses` counts each `Var(v)` read.  A read in `Iter`'s
  init slot OR `ParFor`'s input slot counts as **streaming**;
  everything else counts as **other** (materialising).
- Compiler-generated bindings (`_par_results_N` from fused for-par's
  desugar) are skipped via the `_` prefix check.
- Diagnostics:
  - `streaming == 0 && other == 0` (unused) → "result is never read,
    use fused discard form".
  - `other > 0` → "result used in materialising context, see
    par_to_vec".
  - `streaming == 1 && other == 0` → silent (lowering deferred to 8).

3 tests in `tests/threading.rs`:
- `par_result_random_access_emits_materialise_warning`
- `par_result_unused_emits_unused_warning`
- `par_result_warning_skips_compiler_generated_bindings`

**5b folded into step 8.**  The streaming-rewrite (`let r =
parallel_for(...); for x in r { body }` → fused `Stitch::Queue`
dispatch) requires either:
- A new opcode that runs the body's bytecode per worker result, or
- Inlining body bytecode into the streaming dispatcher.

Both are runtime-architecture changes that bundle naturally with
step 8's Concat retirement — at that point the runtime question
has a clear destination.  Until then, streaming-eligible patterns
keep working through the existing materialised path; the analyser's
silent treatment means no false-positive warnings.

### Step 6 — Audit test suite for materialise sites (DONE 2026-04-29)

**Effort: XS** · **Net Δ lines: 0** (audit-only)

Audited the test corpus + bench + libs for sites that would hit
spine step 5a's materialise / unused warnings.  Findings:

| Source | Par sites | Materialising? |
|---|---|---|
| `tests/scripts/22-threading.loft` | 9 fused `for x in items par(r=fn(x), N) { … }` | No — fused form, compiler-generated `_par_results_N` skipped by analyser |
| `bench/11_par/bench.loft` | 2 fused for-par loops | No — same reason |
| `tests/threading.rs` | 24 Rust-side dispatches via `run_parallel_*` | N/A — bypass parser, never produce the IR shape the warning checks |
| `tests/threading_chars.rs` | 31 fused for-par tests | No |
| `lib/`, `default/` | 0 par calls (only signature decls) | N/A |

Sites my own step 5 tests added:
- `par_result_random_access_emits_materialise_warning` — synthesises
  the materialising pattern intentionally, asserts the warning fires.
- `par_result_unused_emits_unused_warning` — synthesises unused
  pattern intentionally, asserts unused warning fires.

Verification: `cargo test 2>&1 | grep "materialising context\|never read"`
returns zero hits across the full 897-test suite (excluding the two
intentional warning assertions which test the warning shape, not
emit it during normal execution).

**Conclusion:** the test suite is already streaming-only.  No
porting work needed.  Step 6 collapses to a documented audit; step
7 (warning → error) is safe to land without breakage.

### Step 7 — Promote warning to error (DONE 2026-04-29)

**Effort: XS** · **Net Δ lines: 2 words** · **Branches collapsed: 0** (gates retirement)

Flipped `Level::Warning` to `Level::Error` at both diagnostic sites
in `Parser::check_par_result_singlepass` (`src/parser/mod.rs`).  After
step 6's audit confirmed zero corpus sites trigger the materialise /
unused diagnostics, the deprecation window closed cleanly:
materialising par results (`r[i]`, multi-pass, store as `vector<S>`,
etc.) now fail to compile rather than warn.

Test renames + assertion tightening: each test now also checks for
the literal `Error:` prefix in the diagnostic line so a future
accidental level downgrade is caught immediately.

- `par_result_random_access_emits_materialise_warning`
  → `par_result_random_access_emits_materialise_error`
- `par_result_unused_emits_unused_warning`
  → `par_result_unused_emits_unused_error`
- `par_result_warning_skips_compiler_generated_bindings`
  → `par_result_diagnostic_skips_compiler_generated_bindings`

**Files:** `src/parser/mod.rs` (2 sites + doc-comment),
`tests/threading.rs` (test renames + Level::Error asserts).

### Step 8 — Retire Stitch::Concat runtime + parallel_execute_and_collect (absorbs 5b)

**Effort: S–M** · **Net Δ lines: -250, +50** · **Branches collapsed: 6**

Sub-phased like step 3.  Each sub-step is a single-session unit; all
prior par-suite tests must stay green at every sub-step's tip.

- **8a (DONE 2026-04-29)** — par-buffer infrastructure + native fns.
  Added:
  - `Stores::par_buffer_stack: Vec<Vec<u64>>` — stack of per-call
    result buffers; nesting-safe (`last()` reads the inner-most
    active buffer).  `Clone` resets to empty (runtime state, not
    type schema).  Constructors in `database/mod.rs` (×2) and
    `database/allocation.rs` (×2 worker-clone sites).
  - `n_parallel_queue` native fn — same arg layout as
    `n_parallel_for`, calls `run_parallel_queue` (step 4),
    pushes the resulting `Vec<u64>` onto `par_buffer_stack`,
    returns the row count.
  - `n_parallel_buf_get(idx) -> integer` — reads
    `stores.par_buffer_stack.last()[idx]` as i64.
  - `n_parallel_buf_drop()` — pops `par_buffer_stack`.
  - Codegen extras-push / extras-subtract entries for the new
    `n_parallel_queue` (matches the n_parallel_for / _light /
    _discard pattern).
  - Default `01_code.loft` decls for the three new native fns.
  - Three Rust unit tests in `tests/threading.rs::par_buffer_*`
    locking in push/pop, nesting, and clone-reset semantics.

  **Status:** infrastructure live; no parser consumer yet, so no
  observable behaviour change.  All 30 threading + 31
  threading_chars tests stay green.

- **8b (DONE 2026-04-29)** — parser-side IR rewrite for fused
  for-par with **DbRef-input + 8-byte integer-return** workers.
  `build_parallel_for_ir` branches on `route_through_queue`: when
  the gate matches, emit `Set(par_len, n_parallel_queue(...))` +
  `n_parallel_buf_get(idx)` body access + post-loop
  `n_parallel_buf_drop()`.  No heap result vector allocated; the
  buffer lives in `Stores::par_buffer_stack` (8a's infrastructure).

- **8b' (DONE 2026-04-29)** — extend the Queue runtime to mirror
  `run_parallel_direct`'s input-kind dispatch ladder, then drop the
  parser's `dbref_input` gate.  After this commit every primitive
  `vector<T>` input (DbRef / text / primitive-1/4/8 / wide-inline
  tuple / fn-ref) routes through Queue when the worker returns a
  full 8-byte integer.

  **Changes:**
  - `run_parallel_queue` (`src/parallel.rs`) gains
    `primitive_input_size: u32` and `tuple_input_types:
    Option<Vec<Type>>` parameters.  The per-row dispatch now
    branches:
    - `primitive_input_size == u32::MAX` → text input
      (`execute_at_raw_text_input`)
    - `> 8` → wide-inline (tuple / fn-ref via
      `execute_at_raw_primitive_input_wide`)
    - `1..=8` → primitive (`execute_at_raw_primitive_input`)
    - `0` → DbRef (existing path, `execute_at_raw`)
  - `n_parallel_queue` (`src/native.rs`) computes
    `primitive_input_size` from `input_kind_for_first_arg(def)` and
    `tuple_input_types` from `tuple_first_arg_types(def)`, mirroring
    `n_parallel_for`.
  - `tests/threading.rs::par_queue_*` tests updated to pass
    `0, None` for the two new params (DbRef-input dispatch, matching
    the existing test workers).
  - Parser gate (`build_parallel_for_ir`) drops the worker-first-arg
    check.  Comment narrative updated to point at 8c (text-return)
    and 8d (ref-return) plus the per-size buf_get variant for narrow
    integer returns as remaining gating axes.

  **Test corpus impact:** all 31 `tests/threading_chars.rs` + 30
  `tests/threading.rs` tests stay green.  Newly routed through
  Queue (regression-tested):
  - `par_int_to_int_t4_primitive_input` (vector\<integer\> input)
  - `par_i32_input_t4` (vector\<i32\> input — narrow primitive)
  - `par_u8_input_t4` (vector\<u8\> input — 1-byte primitive)
  - `par_text_input_t4` (vector\<text\> input)
  - `par_enum_input_t4` (vector\<Color\> — enum-no-payload)
  - `par_tuple_input_int_int`, `par_tuple_input_int_text`
    (tuple inputs, including text-field inflation via
    `read_tuple_at_wide`)
  - `par_struct_to_int_t4` and friends (DbRef inputs, retained
    from 8b's coverage)

  **Still on the legacy path** (gated by ret_size_8 + integer
  return): every text-return, ref-return, vector-return,
  fn-ref-return, struct-enum-payload-return path; every non-integer
  primitive return (boolean, single, character, enum-no-payload,
  tuple); narrow integer returns (u8, i32, etc.).  These wait for
  8c / 8d / per-size buf_get variants.

- **8c (DONE 2026-04-29)** — extend Queue dispatch to text returns.
  Used option (i) from the design — separate `Vec<Vec<String>>` stack
  alongside the int-buffer's `Vec<Vec<u64>>` — to keep the per-row
  read path tight (no enum match per element).

  **Added:**
  - `Stores::par_text_buffer_stack: Vec<Vec<String>>` (4 init sites
    in `database/mod.rs` + `database/allocation.rs`).
  - `n_parallel_queue_text` native fn — same arg layout as
    `n_parallel_for`; calls `run_parallel_text` (already
    existed for the Concat path); pushes `Vec<String>` onto
    `par_text_buffer_stack`; returns row count as `integer` (i64).
    `Type::Integer` slots are 8 bytes regardless of narrow spec
    (`variables::size`), so `_par_len_N`'s `I32` typing matches
    `OpPutInt`'s 8-byte width.  Computes `n_hidden_text` from the
    worker's `__work_*` attribute count, mirroring `n_parallel_for`.
  - `n_parallel_buf_get_text(idx) -> text` — reads the active
    buffer, clones the String into `stores.scratch`, returns a
    `Str` slot pointing at the new entry.  Follows the standard
    text-return convention used by every other text-producing
    native fn (`t_4text_replace`, `t_4text_to_lowercase`, etc.).
  - `n_parallel_buf_drop_text()` — pops `par_text_buffer_stack`.
  - Codegen extras handling for `n_parallel_queue_text` in **both**
    sites (`state/codegen.rs` lines 1948 push-extras AND lines 2116
    subtract-extras).  *The missing second site was the SIGSEGV
    root cause:* parser pushed 6 args (5 declared + 1 n_extra
    count), the StaticCall codegen subtracted only 5 (44 bytes)
    instead of 6 (52 bytes), so the codegen tracker drifted +8B.
    `generate_block`'s end-of-block residue check then emitted
    `OpFreeStack(0, 8)` for the par-block tail, which discarded 8
    actual bytes from the runtime stack — bytes that weren't there
    — and the resulting 4-byte underflow propagated through the
    function's `Return discard=80` into u32 wraparound, faulting
    on the bogus `code_pos`.
  - `default/01_code.loft` decls for the three new natives.
  - Parser `route_text_queue` gate in `build_parallel_for_ir` for
    `Type::Text` returns; the `route_int_queue` gate stays.  When
    the queue path activates, `results_var` is `None` (skip
    `create_unique` + `defined`) so scope analysis doesn't
    allocate a phantom slot for the unused result vector.

  **Goldens:** `25_runtime_panic_builtin` and `28_runtime_unwrap_none`
  regenerated for the 3-line shift in `src/native.rs` from the new
  dispatch table entries.

  **Codegen hardening (debug-only assertion):**
  `set_var` (`state/codegen.rs`) gained a debug assertion that
  catches `Set(var, value)` width mismatches before emitting an
  `OpPut*` that would silently corrupt adjacent slots.  Surfaced
  during step 8c development when an `i32` return signature
  briefly let a 4-byte push land into an 8-byte slot.  The
  assertion permits Tuple / Function variants (per-leaf put-op
  emit handled separately) and the legitimate `value pushed 0
  bytes` first-set short-circuit.

- **8d** — extend Queue dispatch to ref / fn-ref / vector returns
  via the `WorkerStores::take_all_owned` rebase machinery already
  shipped in phase 2a/2b prep.

  **Sub-phasing** (mirrors 8a/8b/8c's split):

  - **8d.0 (DONE 2026-04-30)** — `run_parallel_queue_ref` Rust
    helper.  Builds on `run_parallel_ref` but post-processes each
    worker's batch into a flat `Vec<DbRef>` ordered by input row,
    with each `DbRef` rebased into the parent's namespace via
    `Stores::adopt_worker_excess` + `rebase_walk_record`.  Returns
    `(refs, adopted_store_nrs)` so 8d.1's `n_parallel_buf_drop_ref`
    can free the adopted stores at body-tail.

    Adoption (vs. deep-copy) is the cost-saving move that makes
    Queue for refs cheaper than the legacy Concat path: workers'
    output stores are moved into the parent's allocations table —
    no per-record memcpy — and DbRef fields are translated in place
    via `rebase_walk_record` so cross-record references stay valid.

    Marked `#[allow(dead_code)]`; tested via two regression tests in
    `tests/threading.rs`:
    - `par_queue_ref_adopts_and_rebases` — 4-element vector input,
      struct-returning worker, asserts adoption grew the parent's
      allocations table by exactly `adopted.len()`, every ref
      resolves in the parent namespace, and field reads return the
      worker's computed values.
    - `par_queue_ref_empty_input` — short-circuit invariant: no
      workers spawn, no allocations touched.

  - **8d.1 (NEXT)** — par_ref_buffer_stack field + 3 native fns
    (`n_parallel_queue_ref` / `_buf_get_ref` / `_buf_drop_ref`) +
    codegen extras.  Pattern matches 8a→8c.

  - **8d.2** — parser-side `route_ref_queue` gate in
    `build_parallel_for_ir`; route Type::Reference / Enum-payload /
    Vector returns through Queue.

  - **8d.3** — fn-ref returns (20-byte slot — needs the wide-return
    path used by `execute_at_raw_to`).

- **8e** — actual Concat retirement: delete
  `parallel_execute_and_collect` (~170 lines), the `Stitch::Concat`
  arm in `parallel.rs`, the `result_db = stores.null()` allocation
  at the dispatcher top.  All paths now route through Queue.

**Why eighth:** first big complexity drop.  After this, par's runtime
shape is **3 stitch variants in one dispatcher** instead of 6+
runtime variants × 3 dispatch arms.

**Files:** `src/native.rs`, `src/parallel.rs`, `src/codegen_runtime.rs`,
`src/database/mod.rs`, `src/database/allocation.rs`,
`src/state/codegen.rs`, `src/parser/collections.rs` (8b),
`default/01_code.loft`.

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
| Step 5 (value-position warning) | 6 + 2 | 4 | 3 | 2 | +710 |
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
   + warning on materialise]    — M, +260 LOC — DONE 2026-04-29
   (5b lowering folded into step 8 — runtime-arch change bundles
    with Concat retirement)
       ↓
[6 audit test suite]            — XS, no changes — DONE 2026-04-29
       ↓
[7 warning → error]             — XS, 2 words — DONE 2026-04-29
       ↓
[8 retire Concat runtime]       — S, -250 LOC, biggest single drop
   8a par buffer infra           — DONE 2026-04-29
   8b parser primitive rewrite   — DONE 2026-04-29 (DbRef-input + i64 return)
   8b' extend Queue dispatch ladder — DONE 2026-04-29 (all input kinds)
   8c text return Queue path     — DONE 2026-04-29
   8d.0 run_parallel_queue_ref   — DONE 2026-04-30 (adopt + rebase)
   8d.1 par_ref_buffer_stack + natives ← NEXT
   8d.2 parser ref-queue gate
   8d.3 fn-ref returns
   8e Concat deletion
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
