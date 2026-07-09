<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN54 — Sanitizer coverage expansion

## Status

**LIVE — real, unstarted, valuable work; NOT parked (ledger reconciled 2026-07-09).**
Opened 2026-05-31 as the successor to
[@PLAN53](../finished/53-sanitizer-ci-lever/README.md), which shipped the base
sanitizer CI stack (Wave 1 + Wave 2 kickoff) and routed program-level fuzzing
to [@PLN53](../53-program-level-fuzzing/README.md).  This plan owns the
remaining Wave-2 sanitizer-coverage items that are not fuzzing.  It is the
**sanitizer half of loft's #1 stability priority** (proving the store/heap
model sound — the bar that gates crypto trust), so it stays active-able rather
than deferred.  `status:future` retained only because no slice is in flight
right now.

**Ledger vs the CI reality (grepped 2026-07-09), highest-value first:**

- **S2 (TSan) — unstarted; the biggest gap.** No `tsan` job exists anywhere.
  loft runs real `par`/`par_light` parallel workloads under store-isolation
  (THREADING.md) with **zero data-race coverage** (Miri runs
  stacked-borrows-off; ASan/`stack_align_guard` are not race detectors).
- **S3 (`LOFT_POISON`) — ✅ EFFECTIVELY MET (2026-07-09).** Store-record
  poison-on-free built (`keys.rs::poison_enabled` + `allocation.rs::free_named`,
  both backends); the 23-bug campaign drove `LOFT_POISON=1 cargo test` green
  (2026-07-03), **re-verified green on current `main` today (1498/1498 on the
  interpreter suites)**; and the **CI gate landed** — the nightly `poison` job
  in `miri.yml` (nothing ran `LOFT_POISON=1` before, so that green is no longer
  an undefended one-off). The remaining "poison freed STACK slots" half was
  **investigated and re-scoped** (§ Concrete steps S3.3): no single chokepoint
  (~30 TOS-lowering sites), the pop primitive returns a reference into the
  vacated bytes (naive poison is unsound), and the crash-class (stale *DbRef*
  stack values) is **already covered by the sound `LOFT_UAF_GEN` gen-stamping
  detector** — so it is a low-priority follow-on, not a blocker.
- **S9 (cdylib mixed-boundary ASan) — unstarted; high heap-trust value.** The
  C71 path (an interpreted script sharing its `*mut Stores` with a compiled
  cdylib by raw pointer) is **the one cross-boundary surface no sanitizer
  sees**.  `win-cdylib.yml` builds cdylibs but does not ASan them.
- **S6 (native ASan) — unstarted.** ASan instruments only the in-process
  interpreter (`detect_leaks=0`); the `--native` codegen path is
  uninstrumented.  Shares the ASan-on-a-generated-build mechanism with S9.
- **S4 (LSan triage) — unstarted.** CI pins `detect_leaks=0` explicitly pending
  the ~108 live-at-exit baseline triage; flipping to `=1` is the deliverable.
- **S1 (macOS-ARM leg) — mostly moot.** `v2-validation.yml` already runs the
  **full suite** on macOS-ARM (macOS-latest = ARM64); only the *sanitizer*
  (Miri/ASan) leg remains ubuntu-only, which is the narrow residual.
- **S5 / S7 / S8 — low priority.** Grow the Miri curated set; add a nightly
  failure→issue notifier; MSan (heavy upstream setup).

**Recommended entry point (updated 2026-07-09):** S3 is now effectively met (CI
gate landed; DbRef stack-UAF already covered by `LOFT_UAF_GEN`). The next
highest-value slice is **S2 (TSan)** — the only entirely-uncovered tool class
over loft's real `par` parallel workloads — then **S9 + S6** (ASan over a
generated build, one shared mechanism).

## Goal

Expand the sanitizer CI coverage that @PLAN53 established, closing the
platform, tool-class, and corpus blind spots that survive the Wave-1+kickoff
stack.  Exit criterion: each item below is either green on `main` or explicitly
deferred with a one-line reason.

## Sub-arcs

| Item | Description | Exit criterion | Priority |
|---|---|---|---|
| **S1** | **macOS-ARM nightly leg** — add a macOS-ARM runner to `miri.yml`'s toolchain-matrix job (and, when affordable, to the Miri/ASan jobs).  @P383 — the founding incident — surfaced exclusively on macOS-ARM; a ubuntu-only nightly would not have caught it.  **State 2026-07-09: mostly moot** — `v2-validation.yml` already runs the full suite on macOS-ARM (macOS-latest); only the *sanitizer* (Miri/ASan) leg is still ubuntu-only, which is the narrow residual. | macOS-ARM *sanitizer* leg green on `main`; nightly badge reflects it. | Low (was Highest) |
| **S2** | **ThreadSanitizer (TSan)** — add a `tsan` job to `miri.yml` running the parallel/threading suite under `RUSTFLAGS=-Zsanitizer=thread`.  loft executes real parallel workloads via `par`/`par_light` under a store-isolation model (THREADING.md); zero data-race coverage exists today (Miri runs stacked-borrows-off; ASan/guard are not race detectors). | TSan job green on `main`; any races found catalogued or fixed. | High |
| **S3** | **`LOFT_POISON=1` arena poison-on-free keystone — ✅ STORE-RECORD HALF BUILT** (2026-06-29, @PLN85 fuzz-proof: `keys.rs::poison_enabled` + the `allocation.rs::free_named` poison block; both backends — native calls the same `free_named`; positive control proven — exposed a SILENT use-after-free (`elem_accumulate-none`) the cross-backend differential alone missed. **✅ EFFECTIVELY MET (2026-07-09):** `LOFT_POISON=1` re-verified green on current main (1498/1498); the nightly `poison` CI gate landed in `miri.yml`; the "poison freed STACK slots" half was investigated + re-scoped to a low-priority follow-on (no chokepoint; pop returns a ref into vacated bytes; DbRef stack-UAF already covered by `LOFT_UAF_GEN` — see § Concrete steps S3.3).) Fill freed store records + freed stack slots with a sentinel value on free, turning silent store-internal use-after-free (the @P377/@P378 dangling-`DbRef` family) into loud, deterministic garbage at the dangling read — on any rustc, no nightly.  This is the blind spot Miri/ASan/Valgrind all share (loft's arena "free" is not a libc `free()`). | `LOFT_POISON=1 cargo test` green; @P377/@P378-class reads produce sentinel-value panics rather than silent stale data.  **Also unblocks @PLN53 F4.** | ✅ Met |
| **S4** | **Triage the LeakSanitizer baseline** (~108 live-at-exit allocations) — understand each allocation class, fix the avoidable leaks, and turn `detect_leaks=1` on in `miri.yml` for the corpus.  Cluster 5 was a leak; there are likely others. | ASan `detect_leaks=1` passes corpus-wide in CI, or each surviving allocation class has a one-line accepted-leak annotation. | Medium |
| **S5** | **Grow the Miri curated set** beyond the current 4 tests (p213 + clusters 3/4/5) — add cluster 1/2 reproducers + representative text/fn-ref/par shapes so the Miri gate covers more of the hard-UB surface without unbearable runtime. | Miri curated set ≥ 8 tests; job runtime ≤ 20 min on ubuntu. | Medium |
| **S6** | **Native-backend ASan** — instrument the `--native` codegen runtime under ASan (currently ASan instruments only the in-process interpreter; the `--native` path is uninstrumented). | At least one native-mode test corpus passes under ASan; any findings catalogued or fixed. | Medium |
| **S7** | **Failure→issue notifier for the nightly** — a CI job that opens/updates a deduped GitHub issue when the nightly fails, reading per-*job* conclusions (not the overall run status, which `continue-on-error` holds green even when matrix legs are red). | Nightly failure automatically surfaces as a tracked GitHub issue within 24 h of the failing run. | Low |
| **S8** | **MSan (MemorySanitizer) corpus-wide** — uninitialised-read detection beyond what Miri covers.  Painful setup (needs a fully instrumented std); lower priority. | MSan job passes the interpreter subset or deferred with a one-line setup-cost note. | Low |
| **S9** | **Mixed-boundary (C71) cdylib ASan** — instrument the auto-built native-library cdylib *and* the interpreter host under ASan together, covering the [@PLN11](../11-data-as-store/README.md) C71 mixed path: an interpreted script shares its `*mut Stores` with a compiled library cdylib by **raw pointer** (zero-marshalling) — the one cross-boundary surface no current sanitizer sees (ASan = interpreter targets only; the `stack_align_guard` sweep can't see spawned binaries; Miri can't `dlopen` a cdylib).  Propagate `-Zsanitizer=address` into `build_shared_cdylib` when the host is ASan-instrumented, + a nightly job.  **Routed in from @PLN11 N5** (mixed-boundary soundness — the D + E legs landed there, this A leg was tooling-blocked).  Shares the ASan-on-a-generated-build mechanism with **S6**. | the interp-script + native-lib mixed corpus (the `tests/n3_parity.rs` shapes) passes under ASan; a cross-boundary UAF/OOB on the shared store is caught, not silent. | Medium |

## Concrete steps to finish

Ordered by value-per-effort. Each step names the exact file, command, and
acceptance check. **CI convention** (established by @PLAN53): nightly
tool-class sanitizers live in [`.github/workflows/miri.yml`](../../../../.github/workflows/miri.yml)
(non-blocking — a red nightly never blocks a merge); the one cheap per-PR
sanitizer is the `guard` job in [`.github/workflows/ci.yml`](../../../../.github/workflows/ci.yml).
The in-process interpreter suites `--lib --test issues --test wrap --test
strings --test frame_vars` are the sanitizer-relevant surface (native / wasm /
html tests spawn separate uninstrumented binaries and add no coverage — mirror
the existing `asan` / `guard` job filters).

### S3 — `LOFT_POISON` keystone (do FIRST; ~½ day) — nearly done

1. **Re-verify green on current `main`.** ✅ **Done 2026-07-09 — GREEN
   (1498/1498).** `LOFT_POISON=1 cargo test --release --no-fail-fast --lib
   --test issues --test wrap --test strings --test frame_vars` → 0 failures
   across lib + issues + wrap + strings + frame_vars, re-confirming the
   2026-07-03 green still holds after the ~15 intervening commits (incl. the
   @PLN97 store loaders). *Known non-poison noise on a dev box:*
   `codegen_emitter::p310_*` fails on a STALE cached native cdylib (`loft_ffi`
   StableCrateId collision) — it fails identically WITHOUT poison, is
   environmental (a build/toolchain limitation, not a memory bug), and CI's
   clean build does not hit it; clear locally with `make rebuild-native-cdylibs`.
2. **Add the CI gate.** ✅ **Done 2026-07-09** — the `poison` job in `miri.yml`
   (nightly, stable toolchain — poison needs no sanitizer/nightly): `cargo
   nextest run --profile ci --release --lib --test issues --test wrap --test
   strings --test frame_vars -E 'not test(library_suite)'` under `env:
   LOFT_POISON: '1'`. Validated green locally under the exact command before
   landing (1497/1497). *Follow-on (not blocking):* promote to a per-PR `ci.yml`
   job once its wall-clock on a runner is measured acceptable; and reintroduce a
   store-UAF (revert one `OpFreeRefIfDistinct` guard from fuzz-proof-gate.md) as
   a one-off to confirm the gate turns red (the positive control).
3. **Poison freed STACK slots** — S3's unpoisoned second half, **re-scoped
   2026-07-09 after investigation: this is NOT the cheap keystone-completion the
   one-liner implied, and its highest-value case is already covered.** Findings:
   - **No single chokepoint.** `poison_enabled()` today only overwrites freed
     *store records* (`allocation.rs::free_named`). The eval stack is
     `State::stack_bytes` with `stack_pos` (byte TOS); a "freed" stack slot is
     just a lowered `stack_pos`, and `stack_pos` is lowered at **~30 sites**
     (the `get_stack<T>` pop, function-return/frame-release, loop `clear_stack`,
     par-worker entry, coroutine exhaust). There is no one place to hook.
   - **The pop primitive returns a reference INTO the just-vacated bytes.**
     `get_stack<T>` (state/mod.rs:1725) does `stack_pos -= step` then returns
     `&T` at the new `stack_pos` — so poisoning right after a pop corrupts the
     value being read. Naive poison-on-pop is *unsound*; only the **bulk-vacate**
     sites (frame-release, loop-clear) are safe to poison, and even those reset
     `stack_pos` then read the return value back from a computed offset
     (`reenter_ret`), so the poison must spare live return-value/arg bytes.
   - **The crash-class is already covered.** Stale *DbRef* values carried on the
     stack — the dangerous case — are already caught by the sound **`LOFT_UAF_GEN`**
     generation-stamping detector (state/mod.rs:1756; `keys::uaf_gen_enabled`):
     it stamps each DbRef push with its store slot's gen and reports a pop whose
     stamp is older than the slot's current gen (freed+reused since push). Plus
     `LOFT_UAF`/`LOFT_UAF_SRC` frame-scan for a live var holding a DbRef into a
     freed store. So stack poison's *only* residual over what exists is stale
     **non-DbRef** frame reads (ints/lengths/bools), which mostly manifest as
     wrong-values the cross-backend differential oracle already catches.
   - **Proportionate path (if pursued):** poison ONLY the safe bulk-vacate
     regions (frame-release + `clear_stack`), sparing live return-value/arg
     bytes, gated on `keys::poison_enabled()`; run it as a positive-control
     experiment (does it fire on any existing test?) before investing further.
     **Acceptance:** either it surfaces a real non-DbRef stale-frame read (fix
     in-session + graduated `tests/scripts/85-*.loft` guard), or it is
     clean-and-vacuous and S3 is declared complete on the strength of
     store-poison + `LOFT_UAF_GEN` + the differential oracle jointly covering the
     stack-UAF class. **Recommendation:** treat S3 as *effectively met* (gate
     landed; DbRef stack-UAF covered) and keep this as a low-priority follow-on,
     not a blocker.

### S2 — ThreadSanitizer (biggest new tool-class gap; ~1 day)

1. Add a `tsan` job to `miri.yml` (nightly, `dtolnay/rust-toolchain@nightly`,
   explicit `--target x86_64-unknown-linux-gnu` so `RUSTFLAGS` instruments the
   whole build): `RUSTFLAGS: '-Zsanitizer=thread'` running the parallel surface
   — `cargo +nightly nextest run --profile ci --release --target
   x86_64-unknown-linux-gnu --test threading --test threading_chars --test
   parallel_rebase`.
2. **Triage findings against the model:** loft's `par`/`par_light` gives each
   worker DISJOINT stores (THREADING.md), so a TSan report on a shared store
   write is a REAL race; a report inside the runtime's own bookkeeping is either
   a real race or an accepted-and-annotated benign one. Catalogue or fix each.
   **Acceptance:** `tsan` job green on `main`; every finding fixed or annotated.

### S4 — LeakSanitizer triage (~1 day; unblocks a stricter ASan)

1. Reproduce the baseline: build the `asan` job's target locally with
   `ASAN_OPTIONS='detect_leaks=1'` and capture the ~108 live-at-exit stacks.
2. Classify each allocation class (intentional `OnceLock`/lazy-static/interner
   vs avoidable store/String leak). Fix the avoidable ones.
3. Add `lsan_suppressions.txt` for the accepted classes; flip the `asan` job in
   `miri.yml` from `detect_leaks=0` to `detect_leaks=1` +
   `LSAN_OPTIONS=suppressions=lsan_suppressions.txt`. **Acceptance:** ASan
   `detect_leaks=1` passes corpus-wide, or each survivor has a one-line
   accepted-leak annotation.

### S6 + S9 — ASan over a generated build (do together; ~2 days)

Both need the same mechanism: propagate `-Zsanitizer=address` into the rustc
that loft spawns (`src/native_utils.rs`) when the HOST is ASan-instrumented.

- **S6 (native ASan):** thread the sanitizer flag into the `--native` codegen
  rustc invocation; add a native-mode ASan job running a native test corpus.
- **S9 (C71 cdylib mixed-boundary):** propagate the flag into
  `build_shared_cdylib` (`src/native_utils.rs`) so the auto-built cdylib AND the
  interpreter host are instrumented together; add a nightly job running the
  `tests/n3_parity.rs` mixed corpus (interp script sharing `*mut Stores` with a
  compiled cdylib by raw pointer). **Acceptance (each):** at least one native /
  mixed corpus passes under ASan; a cross-boundary UAF/OOB on the shared store
  is caught, not silent.

### S5 — grow the Miri curated set (~½ day)

Extend the `--exact` list in `miri.yml`'s `miri` job beyond the current 4 tests
(p213 + clusters 3/4/5) with cluster-1/2 reproducers + representative
text/fn-ref/par shapes, each Miri-validated first. **Acceptance:** curated set
≥ 8 tests; `miri` job runtime ≤ 20 min.

### S1 — macOS-ARM sanitizer leg (mostly moot; ~½ day or DEFER)

`v2-validation.yml` already runs the full suite on macOS-ARM; only the
*sanitizer* leg is ubuntu-only. Either add `macos-latest` to the `asan`/`miri`
jobs (ASan + Miri both run on macOS-ARM) or **defer with a one-line note** that
the founding @P383 platform risk is already covered by the full-suite macOS-ARM
run. **Acceptance:** macOS-ARM sanitizer leg green, or the deferral note landed.

### S7 — nightly failure→issue notifier (~½ day)

Add a `notify` job to `miri.yml` that reads per-*job* conclusions (not the
overall run status — `continue-on-error`/`fail-fast:false` hold it green) via
the GitHub API and opens/updates a deduped issue on any red leg.
**Acceptance:** a forced nightly failure surfaces as a tracked issue within 24 h.

### S8 — MSan (stretch; DEFER)

Needs a fully instrumented std (heavy upstream setup). Keep deferred with the
one-line setup-cost note until the above land.

## Cross-arc dependencies

- **@PLN53 F4** — blocked on S3 (`LOFT_POISON` keystone) landing.
- **[@PLN11](../11-data-as-store/README.md) N5** — S9 is the routed-in A-leg of @PLN11's mixed-boundary soundness work; the D (differential parity) + E (Goal-E store guard) legs already landed there, so S9 is the remaining (tooling-blocked) sanitizer leg.
- **@PLAN53** (closed) — shipped the `fuzz/` crate, direct structure-fuzz targets, and the base CI stack this plan extends.

## See also

- [`plans/finished/53-sanitizer-ci-lever/`](../finished/53-sanitizer-ci-lever/README.md) — predecessor plan; shipped Wave 1 + Wave 2 kickoff; historical record of the 5-cluster fix arc and CI stack bring-up.
- [`plans/53-program-level-fuzzing/`](../53-program-level-fuzzing/README.md) — sibling spinoff; owns program-level fuzzing (F1-F5); F4 depends on S3 (`LOFT_POISON`).
- [`doc/claude/TESTING.md`](../../TESTING.md) — sanitizer-CI documentation.
- [`.github/workflows/miri.yml`](../../../../.github/workflows/miri.yml) — the nightly Miri + ASan + toolchain-matrix workflow this plan extends.
- [`doc/claude/THREADING.md`](../../THREADING.md) — store-isolation threading model that S2 (TSan) exercises.
- [`doc/claude/DATABASE.md`](../../DATABASE.md) — store allocator + arena semantics relevant to S3 (`LOFT_POISON`).
