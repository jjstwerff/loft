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
- **S3 (`LOFT_POISON`) — partial but NOW UNBLOCKED.** The store-record half is
  built (@PLN85: `keys.rs::poison_enabled` + the `allocation.rs::free_named`
  poison block, both backends).  Its blocker — a green `LOFT_POISON=1 cargo
  test` waiting on the over-free class to land — **cleared** (@PLN85/@PLN90
  drove the ownership register to 0).  Remaining is cheap: poison freed STACK
  slots, confirm `LOFT_POISON=1 cargo test` green, add a CI gate (no workflow
  runs it today).  Highest value-per-effort.
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

**Recommended entry point:** finish **S3** (unblocked, cheap, completes the
poison keystone so store-internal UAF is loud everywhere) or, for the biggest
coverage gain, **S2 (TSan)** — the only entirely-uncovered tool class over
loft's real parallel workloads.

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
| **S3** | **`LOFT_POISON=1` arena poison-on-free keystone — ✅ STORE-RECORD HALF BUILT** (2026-06-29, @PLN85 fuzz-proof: `keys.rs::poison_enabled` + the `allocation.rs::free_named` poison block; both backends — native calls the same `free_named`; positive control proven — exposed a SILENT use-after-free (`elem_accumulate-none`) the cross-backend differential alone missed. **Remaining (NOW UNBLOCKED, 2026-07-09):** poison freed STACK slots; drive `LOFT_POISON=1 cargo test` green — the over-free class / Cluster C blocker it waited on **has landed** (@PLN85/@PLN90 drove the ownership register to 0); add a CI gate running it, since no workflow does today.) Fill freed store records + freed stack slots with a sentinel value on free, turning silent store-internal use-after-free (the @P377/@P378 dangling-`DbRef` family) into loud, deterministic garbage at the dangling read — on any rustc, no nightly.  This is the blind spot Miri/ASan/Valgrind all share (loft's arena "free" is not a libc `free()`). | `LOFT_POISON=1 cargo test` green; @P377/@P378-class reads produce sentinel-value panics rather than silent stale data.  **Also unblocks @PLN53 F4.** | High |
| **S4** | **Triage the LeakSanitizer baseline** (~108 live-at-exit allocations) — understand each allocation class, fix the avoidable leaks, and turn `detect_leaks=1` on in `miri.yml` for the corpus.  Cluster 5 was a leak; there are likely others. | ASan `detect_leaks=1` passes corpus-wide in CI, or each surviving allocation class has a one-line accepted-leak annotation. | Medium |
| **S5** | **Grow the Miri curated set** beyond the current 4 tests (p213 + clusters 3/4/5) — add cluster 1/2 reproducers + representative text/fn-ref/par shapes so the Miri gate covers more of the hard-UB surface without unbearable runtime. | Miri curated set ≥ 8 tests; job runtime ≤ 20 min on ubuntu. | Medium |
| **S6** | **Native-backend ASan** — instrument the `--native` codegen runtime under ASan (currently ASan instruments only the in-process interpreter; the `--native` path is uninstrumented). | At least one native-mode test corpus passes under ASan; any findings catalogued or fixed. | Medium |
| **S7** | **Failure→issue notifier for the nightly** — a CI job that opens/updates a deduped GitHub issue when the nightly fails, reading per-*job* conclusions (not the overall run status, which `continue-on-error` holds green even when matrix legs are red). | Nightly failure automatically surfaces as a tracked GitHub issue within 24 h of the failing run. | Low |
| **S8** | **MSan (MemorySanitizer) corpus-wide** — uninitialised-read detection beyond what Miri covers.  Painful setup (needs a fully instrumented std); lower priority. | MSan job passes the interpreter subset or deferred with a one-line setup-cost note. | Low |
| **S9** | **Mixed-boundary (C71) cdylib ASan** — instrument the auto-built native-library cdylib *and* the interpreter host under ASan together, covering the [@PLN11](../11-data-as-store/README.md) C71 mixed path: an interpreted script shares its `*mut Stores` with a compiled library cdylib by **raw pointer** (zero-marshalling) — the one cross-boundary surface no current sanitizer sees (ASan = interpreter targets only; the `stack_align_guard` sweep can't see spawned binaries; Miri can't `dlopen` a cdylib).  Propagate `-Zsanitizer=address` into `build_shared_cdylib` when the host is ASan-instrumented, + a nightly job.  **Routed in from @PLN11 N5** (mixed-boundary soundness — the D + E legs landed there, this A leg was tooling-blocked).  Shares the ASan-on-a-generated-build mechanism with **S6**. | the interp-script + native-lib mixed corpus (the `tests/n3_parity.rs` shapes) passes under ASan; a cross-boundary UAF/OOB on the shared store is caught, not silent. | Medium |

## Phase ordering

1. **S1 first** — the platform blind spot (macOS-ARM) is the founding motivation; cheapest win (add a runner, no code changes).
2. **S2** — TSan is the standout new tool-class gap; independent of S1.
3. **S3** — `LOFT_POISON` is high value-per-effort and unblocks @PLN53 F4; implement early.
4. **S4 + S5** — corpus hygiene; can proceed in parallel with S1-S3.
5. **S6 + S9** — ASan over a generated build (the `--native` binary for S6, the auto-built cdylib + host for the C71 mixed path in S9); both need the `-Zsanitizer` build-pipeline coordination, so do them together after the above are stable.
6. **S7 + S8** — stretch items; S7 is a workflow change, S8 has heavy upstream setup cost.

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
