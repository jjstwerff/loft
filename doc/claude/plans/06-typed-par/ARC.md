<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-06 — Real-value Development Arc

## Why this exists

PRIORITY.md (the "spine") and README.md (the "phases") have drifted
apart.  Phases were authored as work surfaced; the spine reordered
them; step 8 has since absorbed phases 5b, 6, and most of 10.  The
result is a planning surface where:

- Step status looks tidier than reality (e.g. headline "−250 LOC"
  while the working tree is already at −583).
- Workarounds passing today's tests get marked DONE without flagging
  the latent risk (e.g. 8d.3's per-thread 16-slot cap).
- Off-spine items (4d.A.2, 4d.C, 4e, 5c) are technically open but
  effectively orphaned.
- Real value — fewer dispatch arms, fewer marshalling shapes, real
  browser parallelism, the closing of 8 ignored canaries — is
  spread across multiple bookkeeping layers.

This document is **the single source of truth** for what's left, in
the order it should ship.  Each arc step is one PR-sized unit with
hard scope locks, a named acceptance test, links to the existing
designs and known problem records (PROBLEMS.md P-IDs), and concrete
file-site changes.

## Ground rules (non-negotiable)

1. **One arc step per PR.**  No "Aₙ.1, Aₙ.2, Aₙ.3" sub-step expansion
   inside one branch.  If a step turns out larger than estimated,
   close what's done, file the rest as the next arc step.
2. **No uncommitted multi-day stash.**  At session end, either the
   work commits or it reverts.  If a step needs more than one
   session, close the first session at a green-tests boundary.
3. **Every step closes a canary, deletes code, or unlocks the next
   step.**  Steps that do none of these are paperwork — fold them
   into a step that does.
4. **Bench gate every step.**  `bench/11_par` ±5 % on the loft-interp
   and loft-native columns vs. THREADING.md "Plan-06 phase 0
   baseline" (loft-interp 44 ms, loft-native 12 ms).  No silent
   regressions.
5. **No partial-DONE in this doc.**  A step is OPEN, IN-FLIGHT, or
   DONE.  If it's DONE-with-caveat, the caveat becomes the next step.
6. **Goldens regen lives in the step that shifted them.**  Not a
   trailing chore.

## Acceptance state — what "plan-06 done" means

Three concrete criteria, all measurable:

1. **Single dispatcher family.**  After A8: one generic
   `run_parallel_queue<R>` + `run_parallel_discard` +
   `run_parallel_reduce`.  `grep -c "fn run_parallel_" src/parallel.rs`
   returns ≤ 3.
2. **Single user surface.**  `par_light` removed from
   `default/01_code.loft`; auto-light decided at compile time from
   the worker's effect annotation (DESIGN.md D8 / phase 5e).
3. **Zero ignored par canaries.**  `grep -c "^#\[ignore"
   tests/threading_chars.rs` returns 0.  Today: 8.

When all three hit, plan-06 closes.  Until all three hit, it doesn't.

## Inventory — what's actually in flight

### Uncommitted working tree (as of 2026-04-30)

```
src/native.rs    -345 LOC  (parallel_execute_and_collect, DispatchMode, helpers gated)
src/parallel.rs  -238 LOC  (run_parallel_direct, run_parallel_ref deleted)
```

Heavy `parallel_execute_and_collect` retired.  Light path
(`parallel_light_execute_and_collect`) still alive.  2 goldens
(`25_runtime_panic_builtin`, `28_runtime_unwrap_none`) need regen.
`n_parallel_for` now delegates to `n_parallel_for_light`.

### 8 ignored canaries

| Test | Line | Blocker | Plan link |
|---|---|---|---|
| `par_vec_of_fns_input_t4` | 530 | fn-ref vector storage; codegen stack tracker disagrees with `execute_at_raw_primitive_input_wide` | 4d.A.2 cascade ([04d-followups.md](04d-followups.md)) |
| `par_struct_to_vector_t4` | 639 | vector return — needs hidden-arg per-worker destination | 4e ([04-typed-input-output.md:459](04-typed-input-output.md)) |
| `par_struct_to_keyed_collection_t4` | 663 | keyed-collection return type; engine rejects | phase 4 typed surface |
| `par_struct_to_fn_t4` | 686 | fn-ref return; closure DbRef sentinel offset bug | 4e (same root cause as G6 via different shape) |
| `par_tuple_return_int_int` | 749 | tuple return; var_size=16 > 8 byte cap | T1.8a + phase 9c ([09-tuple-support.md](09-tuple-support.md)) |
| `par_tuple_return_int_text` | 767 | tuple return | T1.8a + phase 9c |
| `par_tuple_return_struct_text` | 785 | tuple return | T1.8a + phase 9c |
| `par_tuple_destructure` | 807 | fused destructure binding | phase 9d |

### Known fragilities (not canaries — won't be caught by tests)

- **8d.3 per-thread reserved-slot cap = 16.**  Workers needing > 16
  fresh stores per element panic.  Latent until a workload exceeds
  it.  No test covers this today.
- **`worker_slot_offset != 0` edge case.**  If parent's allocations
  table is empty when reservation runs, `offsets[0] == 0` and the
  legacy push-at-end fallback kicks in.  Harmless in practice,
  surprising in pathological setups.
- **`mem::swap` after worker join trusts that every worker
  allocation went through the offset-aware path.**  No assertion
  enforces this.  A regression that adds a path bypassing
  `database_named` would silently leak data.

### Cross-references to existing PROBLEMS.md entries

| P-ID | Title | Relevant arc step |
|---|---|---|
| **P196** | Tuple-of-fn-ref native codegen `(u32, DbRef).0 as i32` | A6.c — closes when fn-ref storage redesign (4d.C) lands |
| **P198** | `tests/scripts/95-alias-copy.loft` leaks Database 3 — regression on `roadmap-lsp-eclipse` | A1 — investigate before commit; possible plan-06 par-safety culprit |
| **P199** | Native codegen E0499 double `&mut stores` borrow on `n_assert(..., n_add_pair(...), ...)` | A7 (gated on T1.8a tuple-return convention) |
| **P200** | Native codegen E0308 width mismatch on `f += <integer>` against binary file | Out of scope for ARC; tracked separately |
| **P201** | `tests/html_wasm.rs` Mutex-poison cascade | Out of scope for ARC; test infra |

---

## The arc — 11 steps in shippable order

Each step has: scope-locked bullet list, **design** section, **acceptance
test** (named), **risks** specific to that step, and **out of scope**.

Effort estimates: S = small (≤ 0.5 session), M = medium (~1 session),
L = large (1.5–2 sessions), XL = 2+ sessions.

---

### A1 — Land the uncommitted step-8e cuts cleanly

**Status:** DONE 2026-04-30 (commits `b9ad7af` + `7153390` + the
ARC/THREADING update commit that follows this section).
**Effort:** S (~0.25 session) — actual: ~0.5 session.
**Acceptance test:** `make ci` clean; `find_problems.sh` shows zero
unexpected failures; bench 11 ±5 %.

**Closure notes:**

- Code deletes (commit `b9ad7af`): `parallel_execute_and_collect`
  (~170 LOC), `run_parallel_direct` (~190 LOC), `run_parallel_ref`
  (~40 LOC); `n_parallel_for` collapsed to delegate to
  `n_parallel_for_light`; 4 cfg-gated symbols verified clean under
  `cargo build --no-default-features`.
- Goldens regen (commit `7153390`):
  `25_runtime_panic_builtin.expect` and `28_runtime_unwrap_none.expect`
  re-blessed via `UPDATE_GOLDEN=1`; diffs are pure pc shifts
  (`303→285`, `279→261`).
- P198 alias-copy-leak (commit `7153390`): test still fails at the
  new pc=4842 (was 4788 pre-A1).  Confirmed not A1-caused — the
  retired code paths are unreachable from non-par scripts.  Note
  appended to PROBLEMS.md P198; remains a candidate for an
  A2-prerequisite spot fix in `scopes.rs::scan_set` Span/ParFor
  passthrough.
- Bench-11 ±5%: passed against host-relative `main` baseline
  (interp +3 % median: 98ms main → 101ms branch).  The 44ms
  THREADING.md absolute baseline was a different host; native
  column blocked by P199 (re-opens under A7).  Recorded in
  THREADING.md § "ARC.md A1 host-relative check".
- Clippy debt swept (commit `afc9f70`): branch had accumulated 40
  pre-existing clippy errors from spine commits 87c2ce8 onwards
  (Discard/Queue/Purity/ParFor work).  All cleared via clippy
  autofix + targeted `#[allow]` on legitimate too-many-args /
  similar-names / single-char patterns, and re-flowed doc list
  continuation indents.  No semantic changes.
- `cargo nextest run --profile ci`: passes everywhere except
  `loft::html_wasm::moros_editor_html_smoke` — confirmed
  pre-existing P199 manifestation (`OpCopyRecord(stores,
  n_build_chunk(stores, …), …)` E0499).  Same shape closes under
  A7's hoist-inner-`&mut stores` fix.  Documented at PROBLEMS.md
  P199 § "A1 status".

#### Why first

The working tree has been carrying −583 LOC across session
boundaries.  Every additional change risks tangling.  Get back to a
committed baseline before subsequent steps branch from it.

#### Design

Three concerns in one commit (atomic — no split):

1. **Code deletes already in working tree.**
   - `parallel_execute_and_collect` (~170 LOC) removed from
     `src/native.rs`.
   - `run_parallel_direct` (~190 LOC) and `run_parallel_ref` (~40
     LOC) removed from `src/parallel.rs`.
   - `n_parallel_for` body collapsed to delegate
     `n_parallel_for_light(stores, stack)`.
   - `DispatchMode` enum and tuple-input helpers gated
     `#[cfg(feature = "threading")]` (already done — verify).

2. **Goldens regen.**  Two test fixtures shifted by native.rs line
   movement:
   - `tests/error_messages/cases/25_runtime_panic_builtin.loft`
   - `tests/error_messages/cases/28_runtime_unwrap_none.loft`

   Run `cargo test --release --test error_messages baselines_are_locked_in
   -- --ignored` (or whichever sub-flag generates new baselines per
   `tests/error_messages.rs`'s `BLESS_BASELINES` env var).  Diff the
   updated `.expected` files; verify only line-number / pc shifts.

3. **P198 investigation gate.**  Before committing, run
   `cargo test --release p146_script_95_alias_copy_leak`.  If it
   passes today on `roadmap-lsp-eclipse`, ignore (P198 may already
   be moot).  If it fails, file a one-paragraph note on PROBLEMS.md
   P198 with the current symptom — but **do not block A1 on it**.
   The leak existed before A1's diff and is independent.

#### Risks

| Risk | Mitigation |
|---|---|
| Goldens regen reveals additional unrelated failures | Investigate; if related to par work, document in CHANGELOG.  If unrelated, file as new PROBLEMS.md entry and proceed. |
| `n_parallel_for` → `n_parallel_for_light` delegation breaks `par_light`-explicit user-surface tests | Audit `default/01_code.loft` to confirm both surfaces still resolve to native fns; only the runtime path is now shared. |
| The 4 `cfg`-gated symbols (`InputKind`, `INPUT_PRIMITIVE_MAX_BYTES`, `tuple_first_arg_types`, `input_kind_for_first_arg`) drift between `--features threading` and `--no-default-features` | `cargo build --no-default-features` already verified clean (this session); add to `make ci` if not present. |

#### Out of scope

Narrow-prim Queue extension (A3).  fn-ref work (A6).  Doc rewrites (A11).

---

### A2 — Stress-test the per-thread slot cap; resolve the fragility

**Status:** OPEN
**Effort:** M (~1 session)
**Acceptance test:** new test
`tests/threading.rs::par_queue_ref_unbounded_allocations_per_element`
panics today, passes after fix; `assert!` in `database_named`
catches a synthetic offset-bypass.

#### Why second

8d.3 shipped with a known panic risk.  We need to either prove
unreachability or remove it before extending Queue further.  Doing
this before A3 keeps the allocator design honest as new return
shapes get added.

#### Design

Three sub-tasks in one PR.

##### A2.1 — Stress test (write the failing test first)

```rust
// tests/threading.rs
#[test]
fn par_queue_ref_unbounded_allocations_per_element() {
    // Each worker invocation allocates ~30 fresh stores
    // (more than SLOTS_PER_THREAD=16).
    let src = r#"
        struct Big { fields: vector<integer> }
        fn balloon(x: const integer) -> Big {
            b: Big = Big { fields: [] };
            for i in 0..30 { b.fields += [i]; }
            b
        }
        fn run() -> integer {
            xs: vector<integer> = [1, 2, 3, 4];
            total = 0;
            for x in xs par(b = balloon(x), 4) { total += b.fields[0]; }
            total
        }
    "#;
    assert_eq!(run_loft_int(src, "run"), 4 + 6); // never reached today
}
```

This test panics in current 8d.3 because `worker_slot_local_count`
exceeds `SLOTS_PER_THREAD = 16`.

##### A2.2 — Pick the structural fix

Three options carried from the original investigation
(`/home/ubuntu/.claude/plans/investigate-8d-3-on-plans-jiggly-koala.md`):

| Option | Cost | Cap risk | When to pick |
|---|---|---|---|
| **(a) Dynamic per-thread `Vec<u16>`** of allocated indices, replacing the fixed-size range | 1 session | None (grows) | If existing code paths can be refit cleanly |
| **(b) Atomic `fetch_add` counter** in `parallel_workers` for per-worker offset reservation, growing on demand | 1 session | None | If contention is acceptable; cleanest semantics |
| **(c) Post-process rebase** — drop reservation entirely; each worker pushes at end via `disable_slot_reuse=true`; parent rebases via thread-id-prefixed map | 1.5 sessions | None | Original PRIORITY.md 8d.3 design; rejected as complex but is structurally correct |

**Recommendation: (a)**.  Smallest delta from current code.  The
per-thread slot range becomes a per-thread `Vec<u16>` of indices;
`reserve_worker_slots` returns `(start, capacity_hint)` instead of
a fixed range; `database_named` pushes a fresh slot via
`free_bits.push(false); allocations.push(Store::new(100))` (in the
parent vec, under the per-thread mutex) and records the index in
the worker's `Vec<u16>`.  The post-worker swap iterates the
per-thread `Vec<u16>` instead of `[off, off+limit)`.

If (a) turns out to need more than 1 session — for instance because
`free_bits` mutation under contention forces locking — fall back to
(b).  Don't bundle.

##### A2.3 — Invariant assertion

Add a debug-only assertion in `database_named` that catches the
bypass case:

```rust
#[cfg(debug_assertions)]
fn assert_offset_path(&self, where: &str) {
    if self.worker_slot_limit > 0 {
        // We're a worker; every named alloc must go through the
        // offset-aware path.  If we reach here without honouring it,
        // panic loudly.
        debug_assert!(
            self.went_through_offset_path,
            "{}: worker bypassed offset-aware allocation",
            where,
        );
    }
}
```

Add a test that synthesises the bypass (call `database` directly
without going through `database_named`) and confirms the assertion
fires.

#### Risks

| Risk | Mitigation |
|---|---|
| Option (a) hits a contention point that turns the simple `Vec<u16>` into a `Mutex<Vec<u16>>` and degrades parallelism | Bench 11 gates the merge.  If perf regresses, fall back to (b). |
| The stress test passes today by accident (the workload doesn't actually exceed 16) | Increase the allocation count in the worker until it does panic; that's the real lower bound. |
| The fix exposes an unrelated DbRef-rebase bug that was masked by the cap | Triage; if the new bug existed before A2 it's a separate finding to file, not an A2 blocker. |

#### Out of scope

Narrow-prim Queue (A3).  Don't bundle.

---

### A3 — Extend Queue dispatch for narrow-primitive returns

**Status:** OPEN
**Effort:** L (~1.5 sessions)
**Acceptance test:** the existing `par_struct_to_bool_t4`,
`par_struct_to_byte_t4`, `par_struct_to_single_t4`,
`par_struct_to_character_t4`, `par_struct_to_float_t4` re-route
through Queue with no behaviour change; `grep
"parallel_light_execute_and_collect" src/parser/ src/state/` returns
zero call sites.

#### Why third

Gating work for retiring the light Concat path.  Without narrow-prim
Queue support, A4 cannot ship.  After A2 the allocator is stable
enough to extend.

#### Design

The current `n_parallel_queue` returns `Vec<u64>` (8-byte rows).
Narrow returns (1, 4 bytes; signed and unsigned variants) need
either width-parameterised reads or a separate buffer stack.

**Decision: width parameter on the existing buffer + a new getter.**
A second buffer stack would be cleaner but doubles the per-call
clone-reset cost; the width parameter is uglier but cheaper at
runtime, matching plan-06's "everything is a store" axiom (rows are
opaque bytes).

##### A3.1 — Buffer-stack changes

```rust
// src/database/mod.rs
pub struct Stores {
    // Existing:
    pub par_buffer_stack: Vec<Vec<u64>>,
    // Each entry is (raw bytes, per-element stride in bytes)
    // for narrow returns.  Stride = 1, 2, 4 — 8B uses par_buffer_stack.
    pub par_narrow_buffer_stack: Vec<(Vec<u8>, u8)>,
    // ...
}
```

`par_narrow_buffer_stack` mirrors `par_buffer_stack`'s 4 init sites
(constructor, Clone, both `WorkerStores::new` paths).

##### A3.2 — Native fns

```loft
# default/01_code.loft
pub fn parallel_queue_narrow(... return_size: integer) -> integer
    #rust "n_parallel_queue_narrow"
pub fn parallel_buf_get_narrow(idx: integer, return_size: integer,
                                signed: boolean) -> integer
    #rust "n_parallel_buf_get_narrow"
pub fn parallel_buf_drop_narrow()
    #rust "n_parallel_buf_drop_narrow"
```

`n_parallel_queue_narrow` runs the worker as today but pushes raw
bytes (return_size each) into a `Vec<u8>`; `_buf_get_narrow` reads
those bytes, sign-extends if `signed`, returns as `i64`.

##### A3.3 — Parser gate

Extend `route_through_queue` in `src/parser/collections.rs`:

```rust
fn route_narrow_queue(ret_type: &Type) -> Option<(u8, bool)> {
    use Type::*;
    match ret_type {
        Boolean | Enum(_, false, _) => Some((1, false)),
        Single | Character          => Some((4, false)),
        Integer(spec) if spec.size < 8 => Some((spec.size, spec.signed)),
        // 8-byte ints / Float go through the existing Queue (i64 buffer).
        _ => None,
    }
}
```

Float (8B) keeps using the regular `n_parallel_queue` — its bit
pattern fits in `u64`; the consumer reinterprets via `from_bits`.

##### A3.4 — Codegen extras

Both push-extras (state/codegen.rs:1948) and subtract-extras
(state/codegen.rs:2117) need entries for `n_parallel_queue_narrow`,
`n_parallel_buf_get_narrow`, `n_parallel_buf_drop_narrow`.  This is
the same shape as the 8c regression — adding only one site silently
drifts the codegen tracker by 8 bytes per call.

**Hardening:** add a regression test that exercises the
push/subtract symmetry directly.  Walk the codegen tables and assert
every entry in the push-extras map has a matching entry in the
subtract-extras map.

#### Risks

| Risk | Mitigation |
|---|---|
| Sign extension wrong for narrow signed ints (`i8` → `i64` reading bit 7 as sign) | Test each width individually; add `par_struct_to_i8_t4` if not present. |
| `n_parallel_buf_get_narrow`'s 3-arg shape blows the existing 2-arg buf-get codegen path | This is new code; design the codegen entry from scratch with the right arity. |
| Boolean / Enum-no-payload returns hit the `Type::Enum(_, false, _)` arm but get the wrong stride for an enum with > 256 variants | Loft caps single-byte enums at 256 variants; document this in the gate. |
| Codegen extras drift recurs | The proposed push/subtract symmetry test catches drift at parse time. |

#### Out of scope

Retiring `parallel_light_execute_and_collect` itself (A4).  Adjusting
the ref-buffer stack (untouched).

---

### A4 — Retire the light Concat path

**Status:** OPEN
**Effort:** S (~0.5 session)
**Acceptance test:** `git grep -l "parallel_light_execute_and_collect"`
returns 0; bench 11 ±5 %.

#### Why fourth

Pure delete pass once A3 has unhooked every caller.  Independent PR
so the delete is isolated and reverts cleanly if a corner case
surfaces.

#### Design

After A3:

- `n_parallel_for` and `n_parallel_for_light` are still call sites
  for `parallel_light_execute_and_collect`.  Both are reachable
  because the parser routes some return types here.
- The parser (`build_parallel_for_ir` in
  `src/parser/collections.rs`) now picks one of:
  Discard / Queue (i64) / Queue_text / Queue_ref / Queue_narrow
  for **every** return type.

##### A4.1 — Code removal

Delete:
- `parallel_light_execute_and_collect` in `src/native.rs` (~300 LOC).
- `n_parallel_for_light` in `src/native.rs` (now a delegating
  wrapper).
- The `n_parallel_for` delegating body — restore it to a
  full implementation that picks the Queue / Discard / Reduce
  dispatcher based on the parser's hint, mirroring 4c's
  `DispatchMode` ladder but with the legacy Concat arm gone.
- The `Stitch::Concat` arm in any remaining dispatch ladder.
- The `result_db = stores.null()` allocation at the dispatcher top.

##### A4.2 — Native fn registry pruning

In the `FUNCTIONS:` table at `src/native.rs:95`:
- `n_parallel_for_light` entry: remove.
- `n_parallel_for` entry: keep but its body now routes parser
  picks directly to Queue family.

##### A4.3 — Stdlib decl pruning

In `default/01_code.loft`:
- `parallel_for_light` decl: remove (the user-surface pruning lands
  in A9; for now the `par_light` keyword still resolves but routes
  to the same Queue family).

#### Risks

| Risk | Mitigation |
|---|---|
| A4 drops a call path the parser still emits → SIGSEGV or panic | A3's acceptance test (`grep "parallel_light_execute_and_collect"` returns zero) is the gate; if it shows residual hits, those are A3 omissions to fix in A3, not A4. |
| `n_parallel_for_light` removal breaks the `par_light` user-surface keyword today | The keyword stays alive routing to `n_parallel_for` until A9; this PR only removes the Concat path it called. |
| Bench 11 regresses because narrow-prim returns now go through Queue's per-row Vec push instead of the light path's raw memcpy | Profile.  If the regression is real, the Queue path needs a memcpy fast path for fixed-stride narrow types — fold into A3. |

#### Out of scope

`par_light` user-surface removal (A9).  Trait dispatch unification (A8).

---

### A5 — `Stitch::Reduce` runtime + `par_fold` surface

**Status:** OPEN
**Effort:** M (~1 session)
**Acceptance test:** new tests `par_fold_int_sum`, `par_fold_max`,
`par_fold_string_concat`, `par_fold_empty_input`,
`par_fold_nested_inside_par`.

#### Why fifth

With Concat retired, Reduce is the last missing Stitch policy.  After
this all common par shapes have a non-materialising runtime path.

#### Design

Per DESIGN.md D7, Reduce is the third policy.  Each worker keeps a
**partial accumulator**; main thread combines partials with the
user-supplied fold operator.

##### A5.1 — Runtime

```rust
// src/parallel.rs
pub fn run_parallel_reduce<R, F>(
    stores:        &mut Stores,
    program:       &Program,
    fn_pos:        u32,
    input:         DbRef,
    init:          R,           // identity
    fold:          F,           // (R, R) -> R: monoid combine
    n_threads:     u32,
    extra_args:    &[u8],
    return_size:   u32,
) -> R
where
    R: Send + Clone,
    F: Fn(R, R) -> R + Sync,
{
    parallel_workers(n_threads, |t, ws| {
        let mut acc = init.clone();
        for row_idx in worker_slice(t, n_threads, input.len()) {
            let row_result = execute_at_raw(ws, program, fn_pos, row_idx, ...);
            acc = fold(acc, row_result);
        }
        acc
    }).into_iter().fold(init, fold)
}
```

The monoid contract (associativity, identity) is the user's
responsibility — the runtime preserves left-to-right order within
each worker, then combines partials in worker-completion order.
Documentation calls this out.

##### A5.2 — Surface

```loft
# default/01_code.loft
pub fn par_fold<T, U>(
    input:    vector<T>,
    init:     U,
    fold:     fn(U, T) -> U,
    threads:  integer,
) -> U #rust "n_parallel_fold"
```

User writes:
```loft
total = par_fold(items, 0, fn(acc, x) -> acc + x.score, 4)
```

##### A5.3 — Auto-detection

Phase 5's walker (`Parser::check_par_result_singlepass` in
`src/parser/mod.rs`) already classifies par-result uses.  Extend it
to detect fold patterns:

```loft
total = sum(parallel_for(items, score_of, 4))
// rewrites to:
total = par_fold(items, 0, fn(acc, x) -> acc + score_of(x), 4)
```

The walker matches `Sum(ParCall) | Max(ParCall) | Min(ParCall) |
Concat(ParCall) | Product(ParCall)` and rewrites.  Any other use is
already a hard error per spine step 7.

##### A5.4 — Native fn

`n_parallel_fold` mirrors `n_parallel_queue`'s shape: pop input,
init, fold-fn, threads from the runtime stack; call
`run_parallel_reduce`; push the scalar result.  Codegen extras
both sites.

#### Risks

| Risk | Mitigation |
|---|---|
| Reduce's "user-supplied fold operator" type-checks differently from a simple closure (e.g. capturing closures need the 4d.C closure-storage redesign) | Initial scope: bare named fn only.  Capturing closure-fold deferred to follow-up. |
| Auto-detection rewrites a non-monoid `sum(...)` (user wrote a lossy op) | Rewrite rule applies only to recognised monoidal ops in stdlib (`sum`, `max`, `min`, `product`, text `concat`).  User-defined "sum" — leave alone. |
| Empty input edge case: rayon dispatches zero workers, identity returns | Test `par_fold_empty_input` covers this; default to identity, no special arm. |

#### Out of scope

Trait-dispatch unification of Queue variants — that's A8.  Auto-light
selection for Reduce workers — A9.

---

### A6 — Close the 4 fn-ref / vector / keyed-collection canaries

**Status:** OPEN
**Effort:** L (~2 sessions)
**Acceptance test:** all 4 canaries un-`#[ignore]`'d and passing.

#### Why sixth

These are the type-coverage canaries plan-06 was chartered to close.
Each represents user-facing functionality that doesn't work today.
They cluster because they share infrastructure (4e hidden-arg
destination, 4d.C closure storage).

A6 is split into 4 sub-PRs because each canary has an independent
fix path.  Each sub-PR closes exactly one canary.

#### A6.a — `par_struct_to_vector_t4` (vector return)

**Design:** Implement phase 4e (`ref_return()` hidden-arg
destination machinery, designed in
[04-typed-input-output.md:459](04-typed-input-output.md)).

Five concrete changes:

1. **Detect hidden args at the dispatcher.**  In
   `src/native.rs::n_parallel_for` (or A4's successor that picks
   Queue), inspect `def.attributes` for entries with `hidden = true`.
   Build a `Vec<usize>` of hidden-arg indices.

2. **Extend dispatch to carry destination kind.**  Add
   `OutputKind::RefHiddenDest { type_nr: u16 }` so the dispatcher
   knows it must allocate per-row destinations:
   ```rust
   enum OutputKind {
       Primitive { size: u8 },
       Text,
       RefDirect,
       RefHiddenDest { type_nr: u16 },  // new
   }
   ```

3. **Per-row destination allocation in workers.**  Before invoking
   the worker, the dispatcher allocates a destination DbRef in the
   worker's output store of `type_nr` and pushes it as the hidden
   first extra.

4. **Result collection.**  Worker writes into the destination; on
   thread join, the destination DbRef is rebased into the parent
   namespace via existing 8d.3 per-thread machinery.

5. **Stitch::Queue plumbs the per-row DbRef into
   `par_ref_buffer_stack`.**  The body's `_get_ref(idx)` reads the
   destination, which now holds the worker's vector.

**Closes:** `par_struct_to_vector_t4` (line 639); G6 in
`01-output-store.md`.

#### A6.b — `par_struct_to_fn_t4` (fn-ref return)

**Design:** Same root cause as A6.a (hidden-arg destination), but
the destination is a 20-byte fn-ref instead of a vector.

Per the canary's ignore message: "stack snapshot at Return time
shows d_nr correctly at stack[4..11] (i64 523) but the closure DbRef
sentinel appears at stack[20..23] (0xFFFF) instead of stack[12..23]
— i.e. the InitRefSentinel(var[24]) wrote 4 bytes higher than
expected."

The fix is to reconcile the codegen `stack_pos` accounting for
`n_pick` with `execute_at_raw_to`'s 20-byte return frame.  After
A6.a's 4e machinery is in place, the fn-ref destination becomes a
particular case of `RefHiddenDest`, with `type_nr` resolving to the
synthetic `__fn_ref` struct introduced in 4d.C.

**Dependency:** the closure-storage redesign in
[04d-fn-ref-closure-storage.md](04d-fn-ref-closure-storage.md) is a
prerequisite if the destination must hold a 16-byte (d_nr +
closure DbRef) layout.  If 4d.C ships first, A6.b becomes mechanical.

**Closes:** `par_struct_to_fn_t4` (line 686); G4 in
`01-output-store.md`; closes P196 if 4d.C is in tree.

#### A6.c — `par_vec_of_fns_input_t4` (fn-ref vector input)

**Design:** Resolve the 4d.A.2 cascade documented in
[04d-followups.md](04d-followups.md).  Three remaining bugs:

1. **Vector storage layout** — fn-ref vector elements need 4-byte
   stride (current parser fix landed) but read-back assumes 8 bytes.
2. **Worker entry path** — `execute_at_raw_primitive_input_wide`
   reads the wide first arg; for fn-ref, the apply opcode's
   compile-time stack tracker disagrees with the worker entry's
   runtime stack layout (tracker says `stack_pos=24` at `CallRef`,
   runtime is 36).
3. **Native codegen** — undefined `t65` reference; closure
   `(u32, DbRef)` type mismatch.

Each bug needs a focused fix.  The closure-storage redesign (4d.C)
makes the layout consistent — 16 bytes everywhere — eliminating
bugs 1 and 3.  Bug 2 (codegen tracker) is the residual deep work.

**Closes:** `par_vec_of_fns_input_t4` (line 530).

#### A6.d — `par_struct_to_keyed_collection_t4` (keyed-collection return)

**Design:** Worker fn returns a keyed collection (sorted / hash /
index / spacial).  Today's engine rejects with "Parallel worker
return type ... is not supported".

Keyed collections are `Reference<KeyedT>` under the hood.  After
8d's Queue ref-return path is stable, accepting keyed-collection
returns is a parser gate adjustment in `route_ref_queue`:

```rust
match ret_type {
    Type::Reference(_, _)
    | Type::Enum(_, true, _)
    | Type::Vector(_, _)
    // After A6.d:
    | Type::Sorted(_, _) | Type::Hash(_, _, _)
    | Type::Index(_, _, _) | Type::Spacial(_, _) => true,
    _ => false,
}
```

The worker's output store must be type-compatible (the keyed
collection's underlying records).  4d.B's
`materialise_keyed_for_par` already handles keyed-collection
**input**; this is the symmetric case for output.

**Closes:** `par_struct_to_keyed_collection_t4` (line 663).

#### Risks (A6 overall)

| Risk | Mitigation |
|---|---|
| 4d.C closure-storage redesign is its own session of work and lands as A6.c.0 | Acceptable; file as such in this doc.  Don't bundle into A6.c. |
| A6.b stack_pos reconciliation drags codegen rewrites unrelated to par | Time-box at 1 session.  If it bleeds into a second, extract codegen fixes as a separate arc step. |
| Sub-PRs land in different orders → integration drift | Pick a fixed order (a → c → b → d) so each builds on stable prior work. |

#### Out of scope

Tuple returns — A7.  Tuple inputs — out of plan-06.

---

### A7 — Close the 4 tuple canaries (gated on T1.8a)

**Status:** OPEN (BLOCKED on T1.8a)
**Effort:** M (~1 session, post T1.8a)
**Acceptance test:** all 4 tuple canaries pass; D11b "✅ when tuples
land" placeholder retires.

#### Why seventh

Tuple support is an orthogonal feature (T1.8a — function
tuple-return convention) that benefits any `-> (A, B)` function.
Plan-06 surfaces the rough edges, but T1.8a is the underlying
work and should ship as a standalone milestone first.

#### Design

##### A7.0 — T1.8a function-return convention (prerequisite)

Designed in [09-tuple-support.md:99 §9a](09-tuple-support.md).

Tuple returns use a **caller-supplied destination**: the tuple's
backing store is the caller's stack frame.  Worker writes elements
to known offsets; caller pops the tuple as a contiguous block.

Five-arg case (`(int, int, int, int, int)`) is the smoke test;
mixed-type case (`(int, text)`) requires per-element inflation
via `read_tuple_at_wide` — already in place from P189d.

T1.8a ships as its own arc step (or separate plan); A7 punts if
it slips.

##### A7.1 — Wide-return path

After T1.8a: route tuple returns through Queue's wide-return path.

`route_through_queue` extension:
```rust
match ret_type {
    Type::Tuple(elems) if total_size(elems) <= 64 => RouteWide,
    Type::Tuple(_) => RouteRef,  // > 64 bytes uses ref-return
    // ...
}
```

64-byte cap mirrors phase 4d.A's `INPUT_PRIMITIVE_MAX_BYTES` —
larger tuples fall through to ref-return.

##### A7.2 — Destructure binding

Phase 9d: fused `for (a, b) in pairs par(...) { ... }` binds
elements directly without an intermediate var.  Parser rewrite in
`build_parallel_for_ir`:

```loft
for (a, b) in pairs par(r = work(a, b), 4) { use(r) }
// desugars to:
for _t in pairs par(r = work(_t.0, _t.1), 4) { use(r) }
```

#### Risks

| Risk | Mitigation |
|---|---|
| T1.8a slips beyond plan-06's window | A7 punts; ARC.md closes without A7; D11b placeholder remains. |
| P199 (native E0499 double-borrow on tuple) lands as a blocker for native tuple compilation | Document in PROBLEMS.md P199 follow-up; A7 covers interpreter mode first; native-mode tuple par becomes A7.1.  Note: A1 confirmed P199 also fires in `tests/html_wasm.rs::moros_editor_html_smoke` (`OpCopyRecord(stores, n_build_chunk(stores, …), …)`) and in `bench/11_par/bench.loft` native column (`format_float(&mut s, t_5float_round(stores, …), …)`) — A7's hoist-inner-`&mut stores` fix closes all three simultaneously. |

#### Out of scope

Tuple input through par (`vector<(T, U)>` input) — out of plan-06.

---

### A8 — Collapse Queue / Queue_text / Queue_ref / Queue_narrow into one

**Status:** OPEN
**Effort:** M (~1 session)
**Acceptance test:** `grep -c "fn run_parallel_" src/parallel.rs` ≤ 3
(Discard / Queue / Reduce); all par tests green; ~30 generated test
fixtures regenerated cleanly.

#### Why eighth

With all canaries closed and all return shapes working, the four
Queue dispatchers diverge only in their result buffer shape.  A
trait + generic dispatcher unifies them — the final simplification
PRIORITY.md called for in 3b.2.

#### Design

##### A8.1 — Trait

```rust
// src/parallel.rs
pub trait QueueResult: Send + 'static {
    type Buffer: Send + Default;
    fn push_row(buf: &mut Self::Buffer, raw: &[u8]);
    fn read_row(buf: &Self::Buffer, idx: usize) -> Self;
}
```

##### A8.2 — Implementations

Four impls match the existing per-shape dispatchers:

- `impl QueueResult for i64` — `Buffer = Vec<u64>`, push_row reads
  8B, read_row extracts.
- `impl QueueResult for String` — `Buffer = Vec<String>`, push_row
  reads 16B Str + clones, read_row clones into scratch.
- `impl QueueResult for DbRef` — `Buffer = (Vec<DbRef>, Vec<u16>)`
  (tuple of refs + adopted store_nrs), push_row rebases via
  per-thread allocator, read_row returns the rebased ref.
- `impl QueueResult for NarrowBytes` — `Buffer = (Vec<u8>, u8)`,
  push_row writes `stride` bytes, read_row reads `stride` bytes
  with sign extension as a parameter.

##### A8.3 — Generic dispatcher

```rust
pub fn run_parallel_queue<R: QueueResult>(
    stores: &mut Stores,
    // ... same shape as today's run_parallel_queue ...
) -> R::Buffer {
    parallel_workers(n_threads, |t, ws| {
        let mut buf = R::Buffer::default();
        for row in worker_slice(...) {
            let raw = execute_at_raw(...);
            R::push_row(&mut buf, &raw);
        }
        buf
    }).into_iter().reduce(merge_buffers::<R>).unwrap_or_default()
}
```

##### A8.4 — Native fn collapse

Today: 4 fns (`n_parallel_queue`, `_text`, `_ref`, `_narrow`).
After A8: 1 fn (`n_parallel_queue`) with a `stitch_id` parameter
that selects the impl at the call site:

```rust
fn n_parallel_queue(stores: &mut Stores, stack: &mut DbRef) {
    let stitch_id = pop_u8(stack);
    match stitch_id {
        0 => run_dispatch::<i64>(...),
        1 => run_dispatch::<String>(...),
        2 => run_dispatch::<DbRef>(...),
        3 => run_dispatch::<NarrowBytes>(...),
        _ => panic!(),
    }
}
```

`stitch_id` set by parser based on `route_through_queue`'s outcome.

##### A8.5 — Generated fixture regen

PRIORITY.md flags ~30 generated test fixtures.  Confirm count via
`find tests/generated -name '*.rs' | xargs grep -l n_parallel_queue
| wc -l`.  Regenerate via the existing `cargo run --bin gen_fixtures`
(or whatever the per-test code-gen path is).

#### Risks

| Risk | Mitigation |
|---|---|
| Trait dispatch loses inlining → bench 11 regresses | Mark trait impls `#[inline]`; bench gates the merge.  If perf regresses > 5 %, revisit (likely: monomorphise the four cases via `#[inline(always)]` + `match` on `stitch_id` directly). |
| 30+ fixtures regen takes longer than expected | Run regen as the first commit of the PR; subsequent commits are the dispatcher refactor proper. |
| `String` impl's clone-into-scratch path differs from current `n_parallel_buf_get_text` semantics | Direct port — the impl mirrors today's behaviour; assertion in test for round-trip equivalence. |

#### Out of scope

Reduce trait abstraction.  `par_light` user-surface removal (A9).

---

### A9 — Drop `par_light` user surface; auto-light via fixed-point

**Status:** OPEN
**Effort:** S (~0.5 session, after 5e-style fixed-point lands)
**Effort with 5e:** M (~1 session)
**Acceptance test:** `grep "par_light" default/*.loft tests/*.loft
tests/scripts/*.loft bench/*/*.loft` returns 0 hits.

#### Why ninth

`par_light` is now redundant — auto-light selection from the worker's
effect annotation decides it at compile time.  Phase 5's stdlib
annotation sweep (5a, done) provides the data.  This step closes
the public-API simplification.

#### Design

##### A9.1 — Phase 5e fixed-point analyser

Per [05-auto-light.md:372 §5e](05-auto-light.md), implement
`analyse_purity_fixpoint`:

```rust
fn analyse_purity_fixpoint(data: &Data) -> HashMap<u32, bool> {
    let callers = data.build_caller_index();          // 5b' shipped
    let mut classification = HashMap::new();
    for d_nr in 0..data.definitions() {
        let initial = match data.def(d_nr).purity {
            Purity::Pure => true,
            Purity::Impure | Purity::Unknown => is_user_fn(d_nr, data),
        };
        classification.insert(d_nr, initial);
    }
    let mut worklist: VecDeque<u32> = data.user_fn_d_nrs().into_iter().collect();
    while let Some(d_nr) = worklist.pop_front() {
        if !classification[&d_nr] { continue; }
        if !walk_with_current_classification(&data.def(d_nr).code, &classification) {
            classification.insert(d_nr, false);
            for &caller in callers.get(&d_nr).unwrap_or(&vec![]) {
                if classification[&caller] { worklist.push_back(caller); }
            }
        }
    }
    classification
}
```

Termination: monotonic — classifications only flip `true → false`.
Cost: ≤ 2× walk per user fn.

##### A9.2 — Compiler emit

When parser emits a par call:
```loft
for x in items par(r = worker(x), 4) { ... }
```

Look up `classification[&worker_d_nr]`:
- `true` (light-eligible) → emit `n_parallel_queue` with light flag.
- `false` (full-eligible) → emit `n_parallel_queue` with heavy flag.

The runtime's heavy/light split lives in `clone_for_worker` —
unchanged.  Only the dispatcher chooses the path based on flag.

##### A9.3 — User-surface removal

In `default/01_code.loft`:
```loft
# REMOVED (was: pub fn par_light(...) #rust "n_parallel_for_light")
pub fn par(...)   #rust "n_parallel_queue"
```

The `par_light` keyword in user code becomes a deprecation pass:
parser detects `par_light` invocation, emits warning ("par_light
deprecated; use par"), routes to `par` with the same args.

##### A9.4 — Test corpus migration

`tests/scripts/22-threading.loft` and similar use `par(...)` already.
Confirm via `grep -rn par_light tests/ bench/` — current tree is
clean (the keyword is documented but not used in tests).

#### Risks

| Risk | Mitigation |
|---|---|
| Fixed-point analyser flip-flops on a malformed annotation | Stdlib annotations are `Purity::{Pure, Impure}` + `Unknown`; `Unknown` defaults to `false` (heavy).  No reverse flips. |
| User has a custom fn the analyser classifies wrong | Compile-time error (DESIGN.md D8 — par-safety verdict is binary).  User fixes the fn. |
| `par_light` keyword removal breaks existing programs | Deprecation pass emits a warning, not an error.  Migration is mechanical (search/replace). |
| 5c (Arc-wrap parent stores) is needed for some workers but not in scope | True — file as separate work; A9 ships without it. |

#### Out of scope

Phase 5c Arc-wrap parent stores.  Non-`par_light` deprecation paths.

---

### A10 — Browser parallel via wasm-bindgen-rayon

**Status:** OPEN
**Effort:** XL (2-3 sessions, split into 3 sub-PRs)
**Acceptance test:** the gallery's `bricks_par.loft` example runs at
≥ 2× single-thread speed in Chrome with `crossOriginIsolated`;
output equivalence vs native par; WebGL goldens unaffected.

#### Why tenth

The streaming model (post-A4) maps cleanly to Web Worker postMessage
transfer.  Phase 8's original sub-phases (8a/8b/8c/8d/8e/8f/8g)
collapse considerably once Concat is gone — no result-vector to
transfer, just per-row Queue results.

A10 splits into 3 sub-PRs; each is testable in isolation.

#### A10.a — Build infrastructure

Add `wasm-bindgen-rayon` dependency.  Configure rayon thread pool
under `#[cfg(target_arch = "wasm32")]`:

```rust
// src/parallel.rs
#[cfg(target_arch = "wasm32")]
fn parallel_workers<R, F>(n_threads: u32, f: F) -> Vec<R>
where R: Send + 'static, F: Fn(usize, &mut WorkerStores) -> R + Sync + Send + 'static
{
    use wasm_bindgen_rayon::init_thread_pool;
    static POOL_INIT: std::sync::Once = std::sync::Once::new();
    POOL_INIT.call_once(|| {
        init_thread_pool(n_threads as usize).await; // top-level await
    });
    // dispatch via rayon's existing scope
    rayon::scope(|s| { ... })
}
```

COOP/COEP headers:
```rust
// web/serve.rs
.header("Cross-Origin-Opener-Policy",   "same-origin")
.header("Cross-Origin-Embedder-Policy", "require-corp")
```

JS shim runtime check:
```js
if (!self.crossOriginIsolated) {
    console.warn("crossOriginIsolated false — falling back to single-threaded");
    return single_threaded_dispatcher();
}
```

Hashed WASM filenames (`loft.${git_sha}.wasm`) prevent stale cache
hits during deployment.

#### A10.b — Per-worker output Stores survive postMessage transfer

Phase 2's `StoreRebase` runs after transfer to rewrite worker-local
`store_nr` fields (DESIGN.md D13).

Workers in browser are SharedArrayBuffer-backed; `Store::new`'s
`Vec<u64>` becomes a slice into shared memory.  The 8d.3 per-thread
allocator (post A2) maps cleanly: the per-thread `Vec<u16>` of slot
indices is per-Web-Worker.

Test infrastructure: `tests/browser/` runs headless under Chrome
(via `wasm-bindgen-test` + browser harness).  Parallelism gates:
- output equivalence vs. native par on the same workload
- ≥ 2× speedup at threads=4 vs. threads=1
- DbRef rebase verification: every result DbRef in the parent
  resolves; no worker-local indices leak.

#### A10.c — Parallelism + visual gates

Bench harness in `scripts/browser/`:
- `bricks_par.loft` example runs under Chrome with timing.
- WebGL goldens (existing `tests/wasm_goldens/`) unaffected.
- Crash recovery: if one Web Worker crashes mid-dispatch, the others
  must complete or panic cleanly — no zombie workers.

#### Risks

| Risk | Mitigation |
|---|---|
| `wasm-bindgen-rayon` requires nightly Rust features | Document the pinned toolchain; lock via `rust-toolchain.toml`. |
| SharedArrayBuffer not available in some hosting setups | A10.a's runtime check fails closed; user gets single-threaded experience instead of a crash. |
| `init_thread_pool` is async (top-level await) → main loft entry must adapt | Wrap entry in async block; rayon init is the only async surface. |
| 3 sub-PRs land in different orders | A10.a (infra) must land first; b and c can land in either order. |

#### Out of scope

Firefox / Safari beyond confirming `crossOriginIsolated` detection.
Browser-pool tuning beyond fixed 4-worker.  Service-worker variants.

---

### A11 — Final cleanup + documentation rewrite

**Status:** OPEN
**Effort:** S (~0.5 session)
**Acceptance test:** all 3 acceptance criteria from "what plan-06
done means" hit; `make ci` clean; D11 type-spectrum tracker shows
every canary closed.

#### Why last

After A1–A10 the runtime is genuinely simple.  This step writes that
down.

#### Design

##### A11.1 — Doc rewrites

Rewrite `doc/claude/THREADING.md` to reflect the streaming-only
model:
- Remove the "3 native fns × 4 getters" table (no longer accurate).
- Add the "1 Stitch trait, 3 policies, 1 dispatcher" diagram.
- Document `par_fold` surface, deprecation of `par_light`,
  auto-light fixed-point.
- Update bench numbers from THREADING.md "Plan-06 phase 0
  baseline" with post-arc measurements.

##### A11.2 — Plan-06 README final pass

Mark all phases done in `doc/claude/plans/06-typed-par/README.md`.
Link to ARC.md as canonical history.  PRIORITY.md gets a final
"superseded" marker (already in place from earlier this session).

##### A11.3 — CHANGELOG entries

`CHANGELOG.md` (user-facing):
```
- Parallel `par(...)` simplified: single user surface, three
  Stitch policies (Discard / Queue / Reduce), automatic
  light/heavy selection.  par_light is deprecated; rewrite as par.
```

`CHANGELOG_TECHNICAL.md` (technical):
```
- plan-06 closed.  Cumulative: 5 → 3 dispatchers, 8 → 4 native
  fns, 2 → 1 user surface, 8 → 0 ignored canaries, ~−1100 LOC
  net.  See doc/claude/plans/06-typed-par/ARC.md for the arc.
```

##### A11.4 — D11 type-spectrum tracker

`doc/claude/plans/06-typed-par/00-baseline-and-bench.md § 0d`:
update each row to show closing arc step.  D11b's "✅ when tuples
land" placeholder retires.

##### A11.5 — Ground-truth checks

Run all three acceptance criteria checks one last time:
```bash
grep -c "fn run_parallel_" src/parallel.rs           # ≤ 3
grep -c "par_light" default/01_code.loft             # 0
grep -c "^#\[ignore" tests/threading_chars.rs        # 0
```

If any returns the wrong number, fix before merging.

#### Risks

None substantive — A11 is mechanical doc work.

#### Out of scope

`par_to_vec` opt-in materialiser — separate arc when needed.

---

## Cumulative shape after each arc step

| After | Dispatchers | Native fns | User surfaces | Ignored canaries | LOC vs baseline | Bench 11 expected |
|---|---|---|---|---|---|---|
| Today (committed) | 4 (`run_parallel_queue` + `_ref` + `_text` + `_discard`; legacy `parallel_execute_and_collect` still present) | 8 | 2 (par/par_light) | 8 | 0 | 44 ms / 12 ms |
| A1 | 4 (no heavy Concat; `parallel_light_execute_and_collect` still alive) | 7 | 2 | 8 | −583 | 44 ms / 12 ms |
| A2 | 4 | 7 | 2 | 8 | −600 | 44 ms / 12 ms |
| A3 | 5 (Queue_narrow added) | 8 | 2 | 8 | −540 | 44 ms / 12 ms |
| A4 | 4 (Light retired) | 6 | 2 | 8 | −840 | 44 ms / 12 ms |
| A5 | 5 (Reduce added) | 7 | 2 | 8 | −690 | 44 ms / 12 ms |
| A6 | 5 | 7 | 2 | 4 | −690 | 44 ms / 12 ms |
| A7 | 5 | 7 | 2 | 0 | −690 | 44 ms / 12 ms |
| A8 | **3** (Discard / Queue / Reduce) | 5 | 2 | 0 | −900 | 44 ms / 12 ms |
| A9 | 3 | 4 | **1** (par only) | 0 | −1000 | 44 ms / 12 ms |
| A10 | 3 | 4 | 1 + browser | 0 | varies | + browser numbers |
| A11 | 3 | 4 | 1 | 0 | **−1100** | unchanged |

Three of plan-06's headline numbers — "1 dispatcher" (close: 3
because Stitch policies stay distinct), "1 user surface" (yes),
"~−1500 LOC" (close: −1100 net) — fall within hailing distance of
the original projection.  The shortfall is honest: phase 2's rebase
machinery (~150 LOC) stays as library code for future `par_to_vec`,
and per-thread allocator work added ~50 lines that the original
plan didn't anticipate.

## What this arc explicitly does NOT cover

These items appeared in plan-06's earlier framing but are
**out of scope** for closing plan-06:

- **`par_to_vec` opt-in materialiser** (was phase 11).  Re-add only
  when a real user needs it.  Phase 2's `StoreRebase` machinery
  stays in tree as `#[allow(dead_code)]` library; `par_to_vec`
  becomes a separate ~half-session arc when needed.
- **Phase 5c Arc-wrap parent stores.**  Standalone perf work,
  unrelated to par's runtime simplification.  File as a separate
  PERFORMANCE.md item.
- **Phase 9 tuple input** (`vector<(T, U)>` input).  A7 covers
  tuple *return*; tuple *input* through par is a separate
  ergonomic feature, not a simplification target.
- **P200, P201** — out of scope for ARC; tracked separately.

## Risk register (cross-step)

Honest list of what could derail the arc:

| Risk | Mitigation |
|---|---|
| A2's stress test reveals the per-thread approach is structurally wrong → larger rewrite needed | A2 explicitly carries 3 fix options (a/b/c).  If all three exceed budget, fall back to PRIORITY.md's unified-rebase design. |
| A3's narrow-prim Queue extension surfaces a codegen mismatch like 8c's missing extras-subtract | The 8c lesson is locked in: every new native fn must add **both** push-extras AND subtract-extras entries.  A3.4 adds a regression test that asserts symmetry. |
| A6.c's fn-ref vector storage (4d.A.2 cascade) is bigger than 1 PR's worth | Acceptable to ship the storage redesign as a separate arc step (call it A6.c.0).  Don't bundle. |
| T1.8a tuple-return convention slips | A7 punts to a future arc.  Plan-06 closes without tuple-par support; D11b's placeholder remains. |
| A8 trait dispatch loses inlining → bench 11 regresses by > 5 % | Investigate before merging; monomorphise via `#[inline(always)]` + match-on-stitch-id if needed. |
| A9's 5e fixed-point analyser is more work than budgeted (cycle handling, mutual recursion) | The algorithm in [05-auto-light.md:372](05-auto-light.md) is concrete and bounded.  If implementation surfaces edge cases, document them, ship a conservative version (assume `false` for any cycle), file the optimistic version as follow-up. |
| A10.a's `init_thread_pool` async semantics force broader async refactor in main entry | Wrap top-level entry in an async block; rayon init is the only new async surface.  No deep refactor. |
| Bench 11 regresses by > 5 % at any step | Investigate before merging.  No "we'll fix it in the next step" — that's how complexity creeps in via the back door. |
| P198 (alias-copy regression) turns out to be plan-06 par-safety series breakage | Investigate during A1; if confirmed, file the fix as a new arc step before A2.  Don't proceed with leaks. |

## Cross-references — where to find each design

| Topic | Reference |
|---|---|
| Per-worker output stores | [01-output-store.md](01-output-store.md) |
| StoreRebase machinery | [02-stitch-not-copy.md](02-stitch-not-copy.md), `src/parallel.rs` `StoreRebase` |
| 4e hidden-arg destination | [04-typed-input-output.md:459](04-typed-input-output.md) |
| 4d.C closure-storage redesign | [04d-fn-ref-closure-storage.md](04d-fn-ref-closure-storage.md) |
| 4d follow-ups (P196, etc.) | [04d-followups.md](04d-followups.md) |
| Auto-light fixed-point (5e) | [05-auto-light.md:372](05-auto-light.md) |
| `Value::ParFor` IR shape (D7) | [DESIGN.md:558](DESIGN.md) |
| `is_par_safe` analyser (D8) | [DESIGN.md:598](DESIGN.md) |
| Caller-graph (D12) | [DESIGN.md:869](DESIGN.md) |
| Browser rebase (D13) | [DESIGN.md:912](DESIGN.md), [08-browser-workers.md](08-browser-workers.md) |
| Tuple support / T1.8a | [09-tuple-support.md](09-tuple-support.md) |
| Stream-only no-output-vector | [10-no-output-vector.md](10-no-output-vector.md) |
| Bench numbers / D11 tracker | [00-baseline-and-bench.md](00-baseline-and-bench.md) |
| Spine (historical) | [PRIORITY.md](PRIORITY.md) |
| Phase status (per-topic detail) | [README.md](README.md) |

## Out of band — what to do with PRIORITY.md and the README

- **PRIORITY.md** stays as historical artifact (header already
  added: `> SUPERSEDED by ARC.md`).  Spine steps 1–7 are committed
  there; tail (8d.4 / 8e / 9 / 10) lives in ARC's A1–A11.
- **README.md's status column** updates per arc step.  When a
  README phase fully closes (all sub-items in ARC), strike it.

## How to use this doc going forward

When starting a session on plan-06:

1. Find the lowest-numbered OPEN arc step.
2. Read its scope-locked bullet list and design.  Do not exceed
   scope.
3. Hit the named acceptance test.  No partial-DONE.
4. Update this doc's status field for the step (OPEN → IN-FLIGHT →
   DONE).
5. Single PR; commit; move to the next.

If a step turns out infeasible at its budgeted size, **do not
expand it inline**.  Close what's done, file a new arc step at the
right point, update the status table.  The arc is allowed to grow
forward, never sideways.

---

## Dependency graph

Hard dependencies (must land first) and soft dependencies (could be
parallel but recommended order).  `→` is hard, `⤳` is soft.

```
A1 (commit WIP) ──→ A2 (slot-cap fix) ──→ A3 (narrow-prim Queue) ──→ A4 (retire light)
                                                                          │
                                                                          ↓
                                                                       A5 (Reduce)
                                                                          │
                                                                          ↓
                                                       ┌──────────────── A6 ─────────────────┐
                                                       │      (4 sub-PRs in order a→c→b→d)    │
                                                       │                                      │
                                                  A6.a (vector return)         A6.c (vec-of-fns input)
                                                       │                                      │
                                                       ↓                                      ↓
                                                  A6.b (fn-ref return)         A6.c.0 (4d.C closure storage)
                                                       │                          (extracted if A6.c overflows)
                                                       ↓
                                                  A6.d (keyed-collection return)
                                                       │
                                                       ↓
                                                  A7 (tuple canaries) ⤳ T1.8a (external prerequisite)
                                                       │
                                                       ↓
                                                  A8 (trait dispatch unification)
                                                       │
                                                       ↓
                                                  A9 (drop par_light + 5e fixed-point)
                                                       │
                                                       ↓
                                                  A10 (browser parallel) — 3 sub-PRs (a → b ⤳ c)
                                                       │
                                                       ↓
                                                  A11 (cleanup + docs)
```

**Critical observations:**

- **A1 → A2 → A3 → A4 is a hard chain.**  No reordering possible.
- **A5 (Reduce) can run in parallel with A6** in principle — they
  touch different files.  Recommended sequential to avoid merge
  conflicts in `default/01_code.loft` and `src/native.rs`'s
  `FUNCTIONS:` table.
- **A6 sub-PRs land in order a → c → b → d.**  A6.a establishes the
  `RefHiddenDest` machinery; A6.b reuses it for fn-refs; A6.c is
  independent (4d.A.2 cascade) and lands earlier so A6.b can rely on
  4d.C's closure layout; A6.d is the smallest and lands last.
- **A7 is BLOCKED on T1.8a** (external).  If T1.8a slips, A7 punts —
  the rest of the arc still closes.
- **A8 needs all canaries closed first** so the trait abstraction
  doesn't have to handle special cases.
- **A9 needs A8** because the per-call `stitch_id` lives in the same
  dispatch table that `par_light` removal modifies.
- **A10 can run in parallel with A11**, but A10 should land first
  (A11 documents the final state; A10 changes it).

## A1 starter checklist (next session)

When ready to start A1, the concrete sequence:

```bash
# 1. Confirm working tree state
git status
git diff --stat src/native.rs src/parallel.rs
# Expected: ~-583 LOC across the two files; no other touched files.

# 2. Check the find_problems.sh background run finished (if still queued)
ls -la /tmp/loft_problems.txt
# If exists: cat /tmp/loft_problems.txt and triage.
# Expected failures: only the 2 baseline goldens (25_runtime_panic_builtin,
# 28_runtime_unwrap_none).  Anything else needs investigation.

# 3. Regenerate the 2 goldens
BLESS_BASELINES=1 cargo test --release --test error_messages baselines_are_locked_in
# Or whichever bless flag tests/error_messages.rs accepts; check the file.

# 4. Verify only line-number / pc shifts in the regenerated baselines
git diff tests/error_messages/cases/25_runtime_panic_builtin.loft
git diff tests/error_messages/cases/28_runtime_unwrap_none.loft
# If anything beyond pc / line numbers changed → STOP, investigate.

# 5. P198 gate (alias-copy regression check)
cargo test --release p146_script_95_alias_copy_leak
# If it passes today: ignore for A1.
# If it fails: file a one-paragraph note on PROBLEMS.md P198 with
# current symptom; do NOT block A1 on it.

# 6. CI gate
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --no-default-features
make ci

# 7. Bench gate
cd bench/11_par && make bench
# Verify loft-interp ≤ 46 ms, loft-native ≤ 13 ms (44 ms ± 5 % / 12 ms ± 5 %).

# 8. Commit
git add -p src/native.rs src/parallel.rs
git add tests/error_messages/cases/25_runtime_panic_builtin.loft \
        tests/error_messages/cases/28_runtime_unwrap_none.loft
git commit -m "$(cat <<'EOF'
plan(06-arc-A1): retire heavy parallel_execute_and_collect

Removes ~570 LOC of legacy Concat dispatch:
- parallel_execute_and_collect (~170 LOC)
- run_parallel_direct (~190 LOC)
- run_parallel_ref (~40 LOC)
- DispatchMode + tuple-input helpers gated behind threading feature
- n_parallel_for delegates to n_parallel_for_light (light path
  remains for narrow-prim returns; A3/A4 retire it)

Goldens regenerated for 25_runtime_panic_builtin and
28_runtime_unwrap_none for native.rs line shifts.

See doc/claude/plans/06-typed-par/ARC.md A1 for the design.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"

# 9. Update ARC.md status: A1 OPEN → DONE
```

After A1 commits cleanly, the working tree returns to a known
baseline.  A2 can then branch off it without entanglement.

## Per-step verification commands

Quick lookup for "is this step done?" — paste into any session.

```bash
# A1 — heavy Concat retired
git grep -l "parallel_execute_and_collect" src/        # expect 0 hits
git grep -l "run_parallel_direct" src/                 # expect 0 hits
git grep -l "run_parallel_ref" src/                    # expect 0 hits

# A2 — slot cap removed
grep -n "SLOTS_PER_THREAD" src/database/                # expect 0 hits or const removed
cargo test --test threading par_queue_ref_unbounded_allocations_per_element

# A3 — narrow-prim Queue
grep -n "n_parallel_queue_narrow" src/native.rs        # expect 1 entry
grep -n "par_narrow_buffer_stack" src/database/mod.rs  # expect ≥4 init sites

# A4 — light Concat retired
git grep -l "parallel_light_execute_and_collect" src/  # expect 0 hits
git grep -l "n_parallel_for_light" src/                # expect 0 hits

# A5 — Reduce shipped
grep -n "run_parallel_reduce\|n_parallel_fold" src/parallel.rs src/native.rs
grep -n "par_fold" default/01_code.loft

# A6 — canaries closing
grep -n "^#\[ignore" tests/threading_chars.rs          # 8 → 4 → 0

# A7 — tuple canaries
grep -n "par_tuple_return\|par_tuple_destructure" tests/threading_chars.rs | grep ignore  # expect 0

# A8 — trait dispatch
grep -c "^fn run_parallel_" src/parallel.rs            # expect ≤ 3
grep -n "trait QueueResult" src/parallel.rs            # expect 1 hit

# A9 — par_light gone
grep -n "par_light\|parallel_for_light" default/01_code.loft  # expect 0 hits
grep -rn "par_light" tests/scripts/ bench/             # expect 0 hits
grep -n "analyse_purity_fixpoint" src/scopes.rs        # expect 1 hit

# A10 — browser parallel
grep -n "wasm-bindgen-rayon" Cargo.toml                # expect 1 entry
grep -n "crossOriginIsolated" web/                     # expect runtime check

# A11 — final acceptance gate
grep -c "^fn run_parallel_" src/parallel.rs            # ≤ 3
grep -c "par_light" default/01_code.loft               # 0
grep -c "^#\[ignore" tests/threading_chars.rs          # 0
```

If any check returns the wrong value, the corresponding step isn't
done — regardless of what its status field says.

## Pre-flight checklist (every step)

Before opening any arc-step PR:

- [ ] Branch from `main`, named `plan-06-arc-AN-<short>` (e.g.
      `plan-06-arc-a1-commit-cuts`).
- [ ] Read the step's design section in this file.  Do not skip the
      "Out of scope" line.
- [ ] Check the dependency graph above — confirm prior steps are
      DONE in this arc's status table.
- [ ] Run `make ci` on the unmodified branch tip to confirm a clean
      starting point.
- [ ] Run `bench/11_par/bench.sh` (or equivalent) and record the
      baseline numbers in your scratchpad.
- [ ] If the step touches `src/native.rs`'s `FUNCTIONS:` table, plan
      to update **both** push-extras (state/codegen.rs:1948) and
      subtract-extras (state/codegen.rs:2117) — the 8c regression
      teaches us this drift is silent.
- [ ] If the step adds a new buffer-stack field, plan all 4 init
      sites (Stores::new, Clone, both WorkerStores::new paths).

After the PR's first commit:

- [ ] `make ci` clean.
- [ ] `find_problems.sh --bg` running (or full suite green).
- [ ] Bench 11 ±5 % vs. recorded baseline.
- [ ] Acceptance test from the step passes.
- [ ] ARC.md updated: status field OPEN → IN-FLIGHT → DONE.
- [ ] CHANGELOG_TECHNICAL.md entry recording the step's delta.
- [ ] If the step closes a canary: un-`#[ignore]` it; if it opens a
      new one: file the canary first, fix later.

## Status table

Single source of truth for arc step status.  Update on every commit
that advances a step.

| Step | Status | Effort | PR / commit |
|---|---|---|---|
| A1  | DONE 2026-04-30 | S | b9ad7af + 7153390 |
| A2  | OPEN | M  | — |
| A3  | OPEN | L  | — |
| A4  | OPEN | S  | — |
| A5  | OPEN | M  | — |
| A6.a | OPEN | M | — |
| A6.b | OPEN | M | — |
| A6.c | OPEN | M | — |
| A6.d | OPEN | S | — |
| A7  | BLOCKED on T1.8a | M | — |
| A8  | OPEN | M  | — |
| A9  | OPEN | M  | — |
| A10.a | OPEN | M | — |
| A10.b | OPEN | M | — |
| A10.c | OPEN | M | — |
| A11 | OPEN | S  | — |

**Total budget: ~13–17 sessions** distributed across these steps
(A6 split into 4 + A10 split into 3 → 16 sub-PRs).  At a session a
day, plan-06 closes in 2-3 working weeks.  Stretch budget for
unforeseen rework: + 30 %.

## Final note on scope discipline

Plan-06's earlier two-layer structure (spine + phases) drifted
because every absorbed scope addition felt small in isolation.  The
arc's scope locks are the antidote, but only if they're enforced.

Three tells that scope is creeping:

1. A step's Acceptance test grew between its design and its PR.
2. A step's "Out of scope" list got items removed mid-flight.
3. A step's commits start with "while we're at it…".

If any of these surface during a step, **stop, close the partial
work, file the rest as a new arc step.**  The arc is the contract;
work that doesn't fit goes in the next slot, never inside the
current one.
