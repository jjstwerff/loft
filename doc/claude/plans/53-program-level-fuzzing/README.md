<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN53 — Program-level fuzzing

## Status

**PARKED (`status:parked`) — deferred with a concrete trigger; owner-authorized
2026-07-09.**  Floor shipped, remaining phases paused pending a driver consumer.
This plan is not "not started": its highest-value slices landed
**opportunistically inside sibling stability plans**, which is exactly how
program-level fuzzing has proven to get built here — a stability plan pulls in
the fuzz coverage it needs, rather than this plan driving a standalone 5-arc
push.  What actually shipped since this plan opened (2026-05-31):

- **F4 dependency cleared + F4 substantially done** — @PLN25 landed the
  `LOFT_POISON` arena poison-on-free keystone (@PLN54 S3), and the
  program-level target below already runs with `poison_free` set on the store
  arena, so a store-internal stale read is a loud panic, not silent luck.
- **F2 partially delivered** — @PLN85 built `fuzz/fuzz_targets/program_ownership.rs`:
  a valid-by-construction, self-checking, **in-process** program-level fuzzer
  (parse → scopes → bytecode → interpret) over the ownership-composition
  grammar.  It is the F2 shape, but scoped to the ownership/over-free grammar;
  it explicitly leaves **keyed containers (`hash`/`sorted`) as a follow-up
  axis**.
- **F3 harness delivered (corpus, not fuzzed)** — @PLN89's differential oracle
  (`tests/differential_oracle.rs`, nightly-gated, now a **3-backend** interp ≡
  native ≡ wasm check after this cycle's wasm leg) gives F3's differential
  assertion — but over a **fixed ~29-program corpus**, not coverage-guided
  fuzzed programs.

**Why park (not close, not push):** no active driver, and the highest-risk
slice (ownership over-free + poison) is already covered by `program_ownership`.
But real coverage remains genuinely unbuilt — so this is a *defer*, not a
close-by-decision.  **Residual, all unstarted:** F1
(mutational raw-source target seeded from the ~2000 `.loft` files), **F2's
`hash`/`sorted`/`index` + closure axes** (the schema-coupled collection
coverage loft's heap model actually wants), F3-over-*fuzzed*-programs, and F5
(OSS-Fuzz onboarding).

**Resume trigger — pick this up (→ `status:next`/`active`) when any fires:**
1. A **schema-coupled collection bug** (a `hash`/`sorted`/`index` UAF or
   layout violation) surfaces that `program_ownership`'s ownership grammar
   cannot generate → build **F2's keyed-container axis** then.
2. **@PLN97's layout contract** wants a fuzzer to prove memory/file-layout
   invariants across mutated programs → F1/F2 become its instrument.
3. Appetite for **sustained continuous fuzzing at scale** → F5 (OSS-Fuzz).

The `fuzz/` cargo-fuzz crate already exists (set up under @PLAN53 Wave 2); the
remaining arcs add targets to it.  Issue stays **open** at `status:parked`.

## Goal

A coverage-guided, ASan-instrumented fuzzer over `loft source → parse →
byte_code → execute` that drives the interpreter and the schema-coupled
collections (tree/hash/sorted via real programs) into corner states the
hand-written probe suite misses, with each finding either fixed or recorded
as a cluster in the catalogue below.

## Effort + design

- **Effort:** H (fuzzer bring-up is fast; finding-triage is open-ended)
- **Design:** ~ (sub-arc design below; F3/F4/F5 need further detail)
- **Last touched:** 2026-07-09 (parked — owner-authorized; ledger reconciled to reality)

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
| **F1** | **Mutational source fuzz target** — `fuzz_target!` that takes raw bytes as loft source, runs parse → `byte_code` → execute under ASan + `--features stack_align_guard`, seeded from the ~2000 existing `.loft` test files. Expect an initial robustness/panic-triage wave: malformed input surfaces `unwrap`/panic paths in the parser/compiler first; that hardening is valuable output. | Target builds; runs clean over the seed corpus; panic paths triaged and either fixed or recorded. | **Substantially done** — F1.0 design gate + F1.1 reified oracle (`src/fuzz_oracle.rs`, `fuzzing` feature) + F1.2 falsification tests + F1.3 seed-corpus replay (green: 1333 files, 860 Ran) + F1.4 libfuzzer `program_source` target + F1.5 first fix (F1-1). Triage: F1-1 **fixed**, F1-2 (poison-mode, @PLN85), harness-cache artifact (allowlisted). **Residual:** an actual `cargo +nightly fuzz run` (needs nightly + cargo-fuzz) and the ongoing triage loop. Design: [`F1-DESIGN.md`](F1-DESIGN.md); step decomposition for all open arcs: [`STEPS.md`](STEPS.md). |
| **F2** | **Structure-aware AST generation** — `arbitrary`-derived loft AST → pretty-print → run. Generates valid-by-construction programs that exercise sorted/hash/index collections, nested scopes, closures, many overlapping variable lifetimes (stresses the slot allocator). | Generator emits compiling programs; collection ops covered. | **Partial + design gate for the keyed axis** — @PLN85 shipped `program_ownership.rs` (ownership grammar). **F2.0–F2.5 landed (keyed axis substantially done):** design gate ([`F2-DESIGN.md`](F2-DESIGN.md), exemplars in [`f2-exemplars/`](f2-exemplars/), both backends) + F2.1 `generate_keyed` (`src/fuzz_keyed.rs`) + F2.2 falsification + F2.3 closure/overlapping-lifetime axis + F2.4 poison sweep (120 programs clean) + F2.5 libfuzzer `program_keyed` target (type-checks; run needs nightly+cargo-fuzz) + F2.6 one wide triage pass (1500 seeded-random specs under poison — **clean**, no keyed-collection finding). Residual: further F2.6 passes as findings arrive; an actual nightly `program_keyed` run. |
| **F3** | **Differential interpret ≡ native ≡ wasm** on fuzzed programs — any output/exit divergence is a finding.  Reuses the `cross_mode!` harness.  Folds @PLAN53 Wave-2 item #6 (differential fuzzing) into this plan. | Differential target green over the corpus. | **Partial + design gate** — @PLN89 delivers the 3-backend assertion over a fixed ~29-program corpus. **F3.0–F3.1 landed:** design gate ([`F3-DESIGN.md`](F3-DESIGN.md), subset rule verified byte-identical) + F3.1 `generate_keyed_summary` (printing variant) + `tests/fuzz_differential.rs` (`#[ignore]`, `--features fuzzing`) running a curated corpus on `--interpret`≡`--native` via `run_cross_mode` — **8 generated keyed programs agree**. Remaining: F3.2 triage (as divergences arise), wasm leg, mutated-source (F1) differential. |
| **F4** | **`LOFT_POISON` pairing** — run the fuzzers with @PLN54's arena poison-on-free active so store-internal use-after-free (the @P377/@P378 dangling-`DbRef` family) fails loudly under the fuzzer.  **Depends on @PLN54 S3 (`LOFT_POISON` keystone) landing first.** | Poisoning active under the fuzz targets; any @P377/@P378-class read produces a sentinel-value panic rather than silent stale data. | **Done** — @PLN25 landed the keystone; `program_ownership` runs with `poison_free` on the arena. |
| **F5** | **OSS-Fuzz onboarding** — submit loft to OSS-Fuzz for sustained, continuously-running, coverage-guided fuzzing at scale.  Folds @PLAN53 Wave-2 item #10 into this plan. | loft accepted into OSS-Fuzz; targets running. | **Open** — unstarted |

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
4. **F4 after @PLN54 S3** — the poisoning keystone is a hard dependency.
5. **F5 last** — OSS-Fuzz onboarding requires stable, non-crashy targets
   (otherwise the continuous run becomes noise).

## Findings catalogue (populated as findings arrive)

| ID | Shape | Severity | Detector | Fixed | Notes |
|---|---|---|---|---|---|
| **F1-1** | Undefined operand of a unary PREFIX operator (`-x`, `!x`, `~x`, incl. `-x + 3`) panics codegen (`codegen.rs` — `Incorrect var x[65535]`) instead of a clean "Unknown variable" diagnostic | med (ICE on invalid input; no corruption) | F1.3 seed-corpus replay (`tests/format/unary_minus.loft`) | **Yes** (F1.5) | Root cause: the three unary-prefix branches in `parse_single` (`vectors.rs`) skipped the pass-2 `known_var_or_type` check the binary operators run, so a pass-1 placeholder `Var` (no slot) reached codegen. Fix adds the check to all three branches; verified clean on both backends. Regression: `tests/scripts/535-unary-prefix-undefined-operand.loft`. Was a class (`-`/`!`/`~`), not just `-`. |
| **F1-2** | Read-during-grow store use-after-free: `n = frontier[i]` held across `frontier += [k]` (vector realloc) — a dangling record | high (memory-safety UAF) | F1.3 replay under `LOFT_F1_POISON=1` (`…/86-sandbox-subset-flag/examples/walk.loft`) → SIGSEGV | **No** — open | Runs correct with poison OFF (silently-lucky stale read). Store-lifetime family — @PLN85 / the `program_ownership` grammar's remit, not F1's front-end. Recorded here as the poison-mode catch. |
| F1-harness-cache | Index-out-of-bounds during parse, **harness artifact not a language bug** | — | F1.3 replay only (`…/51-hidden-buffer-aliasing/probes/51-tuple-as-arg.loft`) | n/a | Fires only under the oracle's preloaded-stdlib-cache parse path; does NOT reproduce on a fresh CLI parse. The same clone-cache parse asymmetry `program_ownership` documents. Allowlisted as a known harness limitation. |

Findings from the program-level fuzzing that *did* run landed under the plan
that built the target: `program_ownership`'s over-free findings are recorded
in @PLN85 (`fuzz-proof-gate.md`), and the differential-oracle divergences
(#495/#500/#501) under @PLN89.  Per the @PLAN53 investigation-plan policy, any
findings from *this* plan's future targets go in this catalogue, **not** in
PROBLEMS.md as P-issues.

## Cross-arc dependencies

- **@PLAN53** (closed 2026-05-31) — shipped the `fuzz/` crate and the direct
  structure-fuzz targets (`store_alloc`, `vector_collection`).  This plan adds
  program-level targets to the same crate; coordinate to avoid target-name
  collisions.
- **@PLN54 S3 (`LOFT_POISON`)** — F4 depends directly on the arena
  poison-on-free keystone landing.  Tracked in @PLN54.

## See also

- [`plans/finished/53-sanitizer-ci-lever/`](../finished/53-sanitizer-ci-lever/README.md) — predecessor plan (closed 2026-05-31); shipped the `fuzz/` crate and direct structure-fuzz targets; Wave 2 items #4, #6, and OSS-Fuzz spun off to this plan.
- [`doc/claude/DATABASE.md`](../../DATABASE.md) — store allocator, collection invariants, and key-schema layout that program-level fuzzing exercises.
- [`doc/claude/SLOTS.md`](../../SLOTS.md) — two-zone slot model that F2's overlapping-lifetime programs stress.
- `fuzz/` (repo root) — the existing cargo-fuzz crate that this plan adds targets to.
