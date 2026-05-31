<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 55 — Program-level fuzzing

## Status

Open — not started.  Spun off from @PLAN53 Wave 2 (items #4, #6, and the
OSS-Fuzz row).  @PLAN53 shipped the **direct** structure-fuzz targets
(`store_alloc`, `vector_collection`) and the Wave-2 CI kickoff (PR #237)
before closing (2026-05-31).  The remaining Wave-2 coverage items (macOS-ARM
leg, TSan, `LOFT_POISON` keystone, Miri-set growth, LSan triage, native-ASan)
are owned by [@PLAN56](../../future/56-sanitizer-coverage-expansion/README.md).
This plan owns **program-level fuzzing** (loft source → parse → `byte_code`
→ execute) and the schema-coupled collections (tree/hash/sorted), which
require real loft programs rather than standalone harnesses.

The `fuzz/` cargo-fuzz crate already exists (set up under @PLAN53 Wave 2).
This plan adds targets to it.

## Goal

A coverage-guided, ASan-instrumented fuzzer over `loft source → parse →
byte_code → execute` that drives the interpreter and the schema-coupled
collections (tree/hash/sorted via real programs) into corner states the
hand-written probe suite misses, with each finding either fixed or recorded
as a cluster in the catalogue below.

## Effort + design

- **Effort:** H (fuzzer bring-up is fast; finding-triage is open-ended)
- **Design:** ~ (sub-arc design below; F3/F4/F5 need further detail)
- **Last touched:** 2026-05-31 (plan opened)

## Motivation

@PLAN53 Wave 2 kicked off **structure-aware cargo-fuzz** of loft's internal
data structures.  The first two direct targets — `store_alloc` and
`vector_collection` — proved the engine and immediately found real latent
bugs the ~2000-test suite never caught: a genuine out-of-bounds read in
`vector::remove_vector` (off-by-one element shift) and a
`print_ir`/`vector_append` pair that broke any debug build.  Both are
@P383-class (correct in release, latent UB).

The **tree / hash / sorted** collections cannot be fuzzed the same
standalone way: they are **schema-coupled** — `Key{type_nr, position}`
indexes the `Stores` type registry, and node layout interleaves user fields,
the comparison key, and RB links — so a hand-built standalone harness must
reconstruct the type schema + record layout by hand, where any mistake yields
harness crashes indistinguishable from real bugs.  The right tool is to drive
them through **real loft programs**, where loft's own schema construction is
correct by construction.

Program-level fuzzing is also strictly broader: one target exercises the
parser, compiler, slot allocator, interpreter, AND every collection
(vector/tree/hash/sorted) through real schemas — covering schema-coupled
structures and stressing the whole front-end in a single sweep.  This is the
path that can exercise the @P377/@P378 dangling-`DbRef` family (store-internal
UAF invisible to Miri/ASan/Valgrind) once @PLAN53's `LOFT_POISON` keystone is
in place.

## Sub-arcs

| Item | Description | Exit criterion | Status |
|---|---|---|---|
| **F1** | **Mutational source fuzz target** — `fuzz_target!` that takes raw bytes as loft source, runs parse → `byte_code` → execute under ASan + `--features stack_align_guard`, seeded from the ~2000 existing `.loft` test files. Expect an initial robustness/panic-triage wave: malformed input surfaces `unwrap`/panic paths in the parser/compiler first; that hardening is valuable output. | Target builds; runs clean over the seed corpus; panic paths triaged and either fixed or recorded. | Open |
| **F2** | **Structure-aware AST generation** — `arbitrary`-derived loft AST → pretty-print → run. Generates valid-by-construction programs that exercise sorted/hash/index collections, nested scopes, closures, many overlapping variable lifetimes (stresses the slot allocator). | Generator emits compiling programs; collection ops covered. | Open |
| **F3** | **Differential interpret ≡ native ≡ wasm** on fuzzed programs — any output/exit divergence is a finding.  Reuses the `cross_mode!` harness.  Folds @PLAN53 Wave-2 item #6 (differential fuzzing) into this plan. | Differential target green over the corpus. | Open |
| **F4** | **`LOFT_POISON` pairing** — run the fuzzers with @PLAN56's arena poison-on-free active so store-internal use-after-free (the @P377/@P378 dangling-`DbRef` family) fails loudly under the fuzzer.  **Depends on @PLAN56 S3 (`LOFT_POISON` keystone) landing first.** | Poisoning active under the fuzz targets; any @P377/@P378-class read produces a sentinel-value panic rather than silent stale data. | Blocked on @PLAN56 S3 |
| **F5** | **OSS-Fuzz onboarding** — submit loft to OSS-Fuzz for sustained, continuously-running, coverage-guided fuzzing at scale.  Folds @PLAN53 Wave-2 item #10 into this plan. | loft accepted into OSS-Fuzz; targets running. | Open |

## Phase ordering

1. **F1 first** — the mutational target over real source is the highest
   value-per-effort step: no new infrastructure beyond the existing `fuzz/`
   crate, and the panic-triage wave it surfaces hardens the front-end for all
   subsequent fuzzing.
2. **F2 concurrently or after F1** — the AST generator is independent of F1
   but benefits from the panic hardening it produces (valid-by-construction
   programs are less noisy; knowing the parser/compiler is panic-clean makes
   AST-gen divergences easier to triage).
3. **F3 after F1** — the differential target adds a third assertion
   (output agreement) on top of F1's corpus; needs F1 stable first.
4. **F4 after @PLAN56 S3** — the poisoning keystone is a hard dependency.
5. **F5 last** — OSS-Fuzz onboarding requires stable, non-crashy targets
   (otherwise the continuous run becomes noise).

## Findings catalogue (populated as findings arrive)

| ID | Shape | Severity | Detector | Fixed | Notes |
|---|---|---|---|---|---|
| — | — | — | — | — | No findings yet; plan not started |

Per the @PLAN53 investigation-plan policy: findings from this plan go in this
catalogue, **not** in PROBLEMS.md as P-issues.

## Cross-arc dependencies

- **@PLAN53** (closed 2026-05-31) — shipped the `fuzz/` crate and the direct
  structure-fuzz targets (`store_alloc`, `vector_collection`).  This plan adds
  program-level targets to the same crate; coordinate to avoid target-name
  collisions.
- **@PLAN56 S3 (`LOFT_POISON`)** — F4 depends directly on the arena
  poison-on-free keystone landing.  Tracked in @PLAN56.

## See also

- [`plans/finished/53-sanitizer-ci-lever/`](../../finished/53-sanitizer-ci-lever/README.md) — predecessor plan (closed 2026-05-31); shipped the `fuzz/` crate and direct structure-fuzz targets; Wave 2 items #4, #6, and OSS-Fuzz spun off to this plan.
- [`doc/claude/DATABASE.md`](../../../DATABASE.md) — store allocator, collection invariants, and key-schema layout that program-level fuzzing exercises.
- [`doc/claude/SLOTS.md`](../../../SLOTS.md) — two-zone slot model that F2's overlapping-lifetime programs stress.
- `fuzz/` (repo root) — the existing cargo-fuzz crate that this plan adds targets to.
