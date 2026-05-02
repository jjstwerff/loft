<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 6 — Cleanup, documentation, CHANGELOG

**Status: open**

## Goal

Concretise the ~1100-line net retirement claimed in plan-06's
overview.  After phases 1–5 land, several legacy paths still exist
in the source tree as transitional scaffolding.  Phase 6 deletes
them, rewrites the relevant docs to describe the new world, and
ships the CHANGELOG entry that frames the user-visible story.

Phase 6 has **no behaviour change**.  Every test still passes;
phase-0 characterisation runs unchanged.  The deliverable is
purely the cleanup pass.

## What to delete

### Runtime variants in `src/parallel.rs`

After phase 1 (output stores), phase 2 (rebase), and phase 3 (one
native fn), the following functions are unreachable from any
opcode dispatch:

| Function | Lines (approx) | Status after plan-06 |
|---|---|---|
| `run_parallel_direct` (6 cfg variants) | 120 | superseded by `n_parallel_native` with `Stitch::Concat` for primitives |
| `run_parallel_raw` | 90 | superseded by primitive arm of `n_parallel_native` |
| `run_parallel_text` | 70 | superseded by text arm |
| `run_parallel_ref` | 110 | superseded by reference arm |
| `run_parallel_int` | 80 | legacy string-based dispatch; removed when `parallel_for_int` retired in phase 4b |
| `run_parallel_light` | 50 | superseded by `Stitch::ConcatLight` from phase 5c |

Total: ~520 lines.  Verify by `grep -c '^fn run_parallel' src/parallel.rs` before / after.

### Native fns in `src/codegen_runtime.rs`

After phase 3:

| Function | Lines |
|---|---|
| `n_parallel_for_native` | 90 |
| `n_parallel_for_text_native` | 80 |
| `n_parallel_for_ref_native` | 100 |
| `n_parallel_get_int` | 12 |
| `n_parallel_get_long` | 12 |
| `n_parallel_get_float` | 12 |
| `n_parallel_get_bool` | 12 |
| `n_parallel_get_ref` | 18 |

Total: ~336 lines.  After deletion, the par section of
`codegen_runtime.rs` shrinks from ~224 lines to ~80 lines (the new
`n_parallel_native` polymorphic dispatcher).

### Worker-clone variants in `src/database/allocation.rs`

After phase 5:
- `Stores::clone_for_worker` (D2's locked-clone path; ~50 lines)
- `Stores::clone_for_light_worker` (D2's borrowed-arc path; ~30 lines)

These collapse into one `Stores::worker_view(d_nr)` accessor that
inspects `Definition::is_light_safe` and returns either a clone
or an arc-borrow.  Net: −60 lines.

### Loft surface declarations in `default/01_code.loft`

After phase 4 + phase 7c:
- `parallel_for_int` declaration + `#native` annotation: ~10 lines
- `parallel_for_light` declaration + `#native` annotation: ~10 lines
- `parallel_get_int` / `_long` / `_float` / `_bool` / `_ref`
  declarations + `#native` annotations: ~50 lines

Total: ~70 lines.

### Parser branches

- `src/parser/builtins.rs::check_light_eligible` — superseded by
  the auto-light analyser; ~40 lines.
- `src/parser/builtins.rs::parse_parallel_for_int` — string-based
  dispatch path; ~30 lines.

Total: ~70 lines.

### Net total

| Source | Lines retired |
|---|---|
| `src/parallel.rs` | ~520 |
| `src/codegen_runtime.rs` (par section) | ~336 |
| `src/database/allocation.rs` | ~60 |
| `default/01_code.loft` | ~70 |
| `src/parser/builtins.rs` | ~70 |
| **Subtotal retired** | **~1056** |
| New code in plan-06 phases 1–5 | ~250 (output store helpers, rebase pass, Stitch enum, light analyser) |
| **Net** | **−800 lines** |

The plan README's "~1100 net" claim is approximate; phase 6 makes
it concrete.  Final number depends on how aggressively the new
helpers are factored — could land closer to −900.

## Documentation rewrites

### `THREADING.md`

Today's THREADING.md is ~1900 lines covering the par variants in
detail.  Replace the following sections:

| Section today | After phase 6 |
|---|---|
| § "par() variants" (50 lines) | One paragraph: "loft has one parallel primitive — see § Parallel for-loop" |
| § "Light vs. full path" (80 lines) | One paragraph: "the compiler picks light path automatically; users don't choose" |
| § "Result collection" (40 lines covering channel + copy_block) | Updated to describe the rebase pass |
| § "P1-R1 — release-mode silent data loss" | "Closed in plan-06 phase 2 — D2's Rust-borrow-checker enforcement makes this impossible" |
| § "P1-R3 — claims HashSet wasted per worker" | "Closed in plan-06 phase 2e" |
| § "P1-R5 — no Rust-level proof of non-aliasing" | "Closed in plan-06 phase 2 — D2's relationship makes aliasing a compile-time error" |
| § "P2-R6 — no compiler check for yield inside par()" | "Closed in plan-06 phase 5 — auto-light heuristic R2 rejects yield-containing workers" |

New section "§ Parallel for-loop" describes the phase 7 fused
construction, the call-form desugar, and points users at LOFT.md
for syntax.  Net: THREADING.md shrinks from 1900 → ~1100 lines.

### `LOFT.md`

Add new "§ Parallel execution" subsection (8–12 lines):

> Loft has one parallel primitive — the parallel for-loop:
>
> ```loft
> for x in items par(r = score_of(x), 4) {
>     // body — sequential in the parent thread
> }
> ```
>
> If you need the results as a vector, the call form is sugar:
>
> ```loft
> results = par(items, score_of, 4)
> ```
>
> Both forms route through the same runtime; the call form
> auto-allocates the result vector with the right capacity.

Includes a one-line each example for fold, for_each, and
collect-with-break.  Cross-link to THREADING.md for the data-flow
details and to NATIVE_DEBUG.md for debugging multi-threaded loft.

### `STDLIB.md`

Update the parallel-execution subsection:
- Remove `parallel_get_*` entries.
- Remove `par_light` entry (now internal-only per phase 7c).
- Single entry for `par(input, fn, threads) -> vector<U>` with
  prose noting it desugars to the parallel for-loop.

### `CHANGELOG.md`

User-facing entry framing the work as one feature, not seven phases.
Draft text:

> #### Parallel for-loops
>
> Loft now has a parallel for-loop construction:
>
> ```loft
> for x in items par(r = score_of(x), 4) {
>     // body sees both x and r; runs sequentially
>     // in the parent thread while score_of(x) runs in workers
> }
> ```
>
> The body picks what to do with each parallel result: discard it
> (side-effect work), accumulate (`total += r`), append to a vector,
> or `break` early on the first match.
>
> The familiar `par(items, score_of, 4)` call form still works and
> now allocates exactly once with the right capacity — no per-element
> grow.  The compiler picks the lightweight execution path
> automatically when the worker doesn't need scratch memory; the
> old `par_light` keyword has been retired (use `par`).
>
> Behind the scenes, parallel execution went from seven runtime
> variants and three native dispatch arms to one polymorphic
> pipeline.  Existing code keeps working with no changes.

The CHANGELOG entry sits in the 0.9.0 unreleased block when
plan-06 ships.

## Per-commit landing plan

### 6a — runtime cleanup

Delete the unreachable `run_parallel_*` variants from
`src/parallel.rs`.  Verify with `cargo nextest run --profile ci`
green at every step.

### 6b — native-fn cleanup

Delete the `n_parallel_for_*` and `n_parallel_get_*` fns from
`src/codegen_runtime.rs` (and from any `OpCode → fn` mapping
table).

### 6c — worker-clone consolidation

Collapse `clone_for_worker` and `clone_for_light_worker` into
`worker_view(d_nr)`.  Update every call site.

### 6d — surface cleanup

Remove unused declarations from `default/01_code.loft`.  Run
`make ci`; if any test breaks, the test is using a retired surface
and needs migration (most should already be migrated by phase 4 +
phase 7c).

### 6e — parser cleanup

Delete `check_light_eligible` and `parse_parallel_for_int` from
`src/parser/builtins.rs`.

### 6f — doc rewrites

Apply the THREADING.md / LOFT.md / STDLIB.md changes above.  Keep
each in its own commit so the diff is reviewable.

### 6g — CHANGELOG entry

One commit, one ~30-line addition to CHANGELOG.md's 0.9.0
unreleased block.

## Acceptance criteria

- Phase-0 characterisation suite passes byte-for-byte.
- All bench numbers within ±5 % of phase 5 baseline (no perf
  regression from cleanup).
- `src/parallel.rs` line count drops from 683 → ~150 (verified by
  `wc -l`).
- `src/codegen_runtime.rs` par section drops from ~224 to ~80
  lines.
- `default/01_code.loft` drops by ~70 lines.
- THREADING.md drops from ~1900 to ~1100 lines.
- LOFT.md gains a "§ Parallel execution" subsection.
- CHANGELOG.md has the 0.9.0-block entry.
- `grep par_light` across the workspace returns zero matches in
  source / test files; only matches in CHANGELOG-historical
  references and design docs.

## Risks

| Risk | Mitigation |
|---|---|
| A `run_parallel_*` fn is still reachable through some path I missed | Phase 6a deletes one fn at a time; if `cargo build` fails, restore that fn temporarily and investigate.  Most likely a stale opcode mapping in `src/fill.rs` (auto-generated; regenerate via `cargo test --release fill_rs_up_to_date`) |
| THREADING.md's "P1-R*" entries reference fixes from plan-06; readers without plan-06 context get lost | Each closed entry says "Closed in plan-06 phase N — see plans/finished/06-typed-par/" — same convention plan-01/02/03/04/05 already use |
| Worker-clone consolidation in 6c regresses an edge case | The phase-0 panic-propagation fixture and the phase-1 lifetime fixture both stress this path; if either fails after 6c, revert and ship the consolidation in a follow-up |
| LOFT.md's "Parallel execution" subsection becomes the new authoritative source | Mark it as "syntax reference; data-flow details in THREADING.md" so the deeper material has a clear home |
| CHANGELOG entry oversells the simplification | Include the "−800 lines net" number explicitly and link to plans/finished/06-typed-par/ — readers can verify |

## Out of scope

- Anything that introduces new behaviour.  Phase 6 is purely
  subtractive (delete) and documentary (update).

## Hand-off to phase 7

Phase 6 and phase 7 are independent — they can land in either
order.  If phase 7 (fused for-loop construction) lands first, phase
6's cleanup includes retiring `par_light` from the surface (would
otherwise be deferred to phase 7c).  If phase 6 lands first, phase
7c handles the `par_light` retirement.

The plan-06 README marks phase 6 as the last "internal" phase and
phase 7 as the user-facing surface change.  Order in practice:
6 → 7 keeps the runtime invariants stable while the surface lands;
7 → 6 lets the user-visible feature ship sooner with internal
cleanup as a follow-up.  Plan author's call.
