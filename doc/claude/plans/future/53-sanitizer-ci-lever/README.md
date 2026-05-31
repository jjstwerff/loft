<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 53 — Sanitizer CI lever (UB detection before rustc-release roulette)

## Status

**Wave 1 — SHIPPED.**  All five UB clusters fixed + the full sanitizer CI stack
live on `main`.  The plan is intentionally kept open to host Wave 2 and will be
moved to `finished/` after the next wave lands.

| Wave | Status |
|---|---|
| **Wave 1** — clusters 1-5 fixed + V2-aligned-stack production default + sanitizer CI stack (per-PR guard + nightly Miri/ASan/toolchain-matrix + sticky-comment reporter) | ✅ **SHIPPED** — PR #235 (aligned default + sanitizer engine) + PR #236 (V1 removal + CI stack) |
| **Wave 2** — macOS-ARM nightly leg, ThreadSanitizer, fuzzing, `LOFT_POISON` keystone, + further coverage expansion | 🔵 **OPEN** — see [§ Wave 2](#wave-2--next-wave-open) below |

*Historical note:* PLAN52 was the founding hard dependency (its cluster I was the
dominant noise that would have drowned out any other sanitizer finding).  PLAN52
closed via PR #230; that gate is now satisfied.

**Trigger (Wave 1):** the @P383 / rustc 1.96 incident.  loft's IR carried a
latent use-after-free for many releases; rustc 1.94/1.95 happened
to mask it via libmalloc free-fill behaviour, rustc 1.96 (LLVM 21)
changed allocator/codegen and the UB surfaced deterministically on
macOS as silent data corruption.  PLAN52 closes that specific
cluster.  This plan installs the CI machinery so the **next** latent
UB in the IR-to-bytecode lowering is caught on `main` the day it
lands — not months later when a toolchain bump exposes it.  Without
this lever, every rustc release is a roll of the dice against
whatever UB still lives in the interpreter / native codegen / store
ops surface.

**Scope:**
1. Pick a sanitizer (Miri vs `-Zsanitizer=address` vs both) — option
   comparison, not pre-decided.
2. Catalogue the UB the chosen sanitizer surfaces against today's
   test suite (post-PLAN52 baseline).  Each distinct UB shape is a
   cluster.
3. Fix or explicitly defer each cluster, one commit per cluster.
4. Wire the sanitizer into `.github/workflows/ci.yml` as a gating
   job so future UB lands red.

## Goal

Ship a sanitizer-gated CI job that catches use-after-free,
dangling-pointer, and uninitialised-read UB in the loft interpreter
and native codegen pipeline, with the existing UB surface either
fixed or explicitly catalogued as known-deferred.

## Current instability baseline (as of 2026-05-29)

Recorded here so post-PLAN53 state can be compared to a concrete
starting point — *not* to motivate pausing development.  Loft is
shippable today; this baseline quantifies the strategic exposure
that motivates the CI lever.

**One root cause family, multiple shapes still open.**  Not 50
unrelated bugs — one architectural pattern (use-after-free on
borrowed handles into scope-local temporaries) recurring across
code paths.

| Plan | Status | Clusters | Probes | Root |
|---|---|---|---|---|
| [@PLAN51 hidden-buffer-aliasing](../../finished/51-hidden-buffer-aliasing/README.md) | closed 2026-05-29 | 5 | 62 | `Reference` / heap-buffer borrows freed under the consumer |
| [@PLAN52 value-block-borrow-cleanup](../../finished/52-value-block-borrow-cleanup/README.md) | active | 7 | 84 (33 still failing interpret) | Text borrows from `_ncc_N` value-blocks; self-described "6th cluster of the PLAN51 family" |
| Successor (hypothetical) | — | unknown | — | Likely a 7th/8th expression of the same pattern; will keep recurring until detected mechanically |

**What's actually broken on `main` today:**

- **1 loud failure** — @P383 (PLAN52 cluster I), macOS + rustc 1.96, deterministic CI fail on `repro_p323_index_coalesce`.
- **~33 silent-corruption shapes** in interpreter — exercised by PLAN52 probes, **NOT** by any shipped loft code today (PLAN52 cluster V scan of `lib/*` was clean).  Real exposure starts when upcoming `lib/server` config-lookup patterns land.
- **2 hard crashes** (SIGBUS, PLAN52 probes 46/49) on method-chain consumers — niche shape; process kill if hit.
- **Native side is mostly clean** — IV-Hash/Sorted/Index/Enum + VII chained-call closed 2026-05-29; IV-Vec + IV-Tuple still open on both backends.
- **`SCRIPTS_NATIVE_SKIP` is empty** — every script test runs on both backends; no hidden gating masking native UB.

**Risk profile by surface (educated estimate, NOT measured — the
sanitizer sweep in Stage A2 will replace these guesses with
data):**

| Surface | UB risk | Why |
|---|---|---|
| `src/scopes.rs` free-emission | **HIGH** | Origin of both PLAN51 and PLAN52 root causes |
| `src/parser/` value-block lowering | **HIGH** | Where `_ncc_N` / `__work_N` / `__ref_N` temporaries get planted |
| `src/state/fill.rs` opcode bodies | MEDIUM | 233 opcodes, many touch `DbRef`/`Str` lifetimes |
| `src/generation/emit.rs` native codegen | MEDIUM | PLAN52 clusters IV/VII surfaced here; predicate-emit fix landed but newer patterns will keep arriving |
| `src/store.rs` / `src/database/` | LOWER | Heavily tested via the existing stores-leak gate |
| `src/parallel.rs` / threading | **UNKNOWN** | Not yet exercised by sanitizers; Windows half of @P229 still open |

**Unknown surface area: ~426 `unsafe` blocks** in `src/` (mostly
legitimate store byte-access — but not borrow-checked).  Until
Stage A2 runs, the count of remaining latent UB is *unknown by
construction*.

**Why rustc releases keep surfacing this:**

P383 was not bad luck.  The UB has been latent for many releases;
rustc 1.94/1.95 happened to mask it via libmalloc free-fill, rustc
1.96 / LLVM 21 changed allocator behaviour and the UB became
visible.  Each future rustc bump has nonzero probability of
surfacing a new shape — not because rustc is unstable, but because
the masking behaviour shifts and previously-invisible UB becomes
visible.  This will recur until either:

- (a) the family fully closes — PLAN52 + likely 1-2 successor
  plans grind through every shape, OR
- (b) **this plan's sanitizer CI catches the residue in one sweep
  and gates new UB from landing.**

(b) is strictly cheaper than (a) once the dominant cluster (PLAN52
cluster I) is closed.  Hence the dependency ordering: PLAN52
removes the noise, PLAN53 finds what's left.

**Strategic vs immediate exposure:**

- *Immediate* (today's users): small.  One CI fail; zero shipped-library exposure.  Continuing feature work is safe as long as new code doesn't lean heavily on value-block returns of text borrows.
- *Strategic* (next 12-24 months): material.  Each rustc bump rolls dice; each new library (server, game_client, IDE) brings code patterns that may exercise dormant UB; each new language feature (coroutines, tuples) likely adds new buffer-aliasing sites.  Validation matrices in S-tier (plans 18/19/20/43) help by exercising adjacent shapes early — they're complementary to this plan, not redundant.

The baseline figures in this section are the **before-snapshot**
for closure: if Stage A2's sweep surfaces clusters at materially
different scale than the rows above predict, that's itself a
finding and goes in `cluster-0-tooling-decision.md` § Calibration.

## In-plan vs spinoff policy (default: in-plan)

UB shapes discovered during Stage A stay **in-plan** by default.
Spin off only when:

1. The fix surface is large enough to balloon this plan beyond
   reviewer-friendliness (e.g. a whole new dep-tracking redesign).
2. The shape is truly an edge case users won't hit (e.g. UB in a
   `#[cfg(test)]` harness).

Default in-plan reasoning matches PLAN52: cumulative probe coverage
IS the regression guard.  Splitting clusters across plans collapses
that guard.

## Sanitizer option matrix (Stage A — pick one or both)

| Option | Detects | Speed | FFI compat | Platforms | Notes |
|---|---|---|---|---|---|
| **Miri** (`cargo +nightly miri test`) | UB per Rust's abstract machine — UAF, dangling pointers, uninitialised reads, type-punning, alignment | 10-100× slower than `cargo test` | ✗ — blocks on most FFI (PNG, OpenGL, threads, file I/O outside MIRIFLAGS allowlist) | Linux, macOS, Windows (any host with nightly) | Strictest detector; catches more than ASan but excludes any test that exercises real OS / GPU |
| **AddressSanitizer** (`RUSTFLAGS=-Zsanitizer=address`) | UAF, heap-buffer-overflow, use-after-return, double-free | ~2-3× slowdown | ✓ — works with full FFI surface | Linux, macOS (Apple Silicon supported); Windows is `-Zsanitizer=address` capable on MSVC nightly | Runs real allocator with red-zones; closer to production behaviour |
| **Combination** | Both detectors run as separate CI jobs over disjoint test subsets | Sum of both | Both | Both | Highest coverage; double CI minutes |

The pick lives in `cluster-0-tooling-decision.md` once Stage A starts.
Pre-decision considerations:

- Miri's FFI block matters: most of `lib/png`, `lib/server`, `lib/fs_*`,
  `lib/process` would be excluded from a Miri job.  The core
  bytecode interpreter (the one P383 lived in) and `tests/issues.rs`
  would fit.
- ASan can run the full suite including `--native` codegen, which
  Miri cannot.
- The PLAN52 trigger (post-consumer `OpFreeText` on a freed
  `String`) is a real heap UAF — ASan would catch it on Linux at
  PR-CI time.  Miri would too.  Both are viable for the P383-class.

## Cluster catalogue (REQUIRED — populated during Stage A)

First sanitizer findings, 2026-05-29 (Stage A1 spike — ran against
the PLAN52 working tree, *not* yet a closed-PLAN52 baseline; both
findings are independent of PLAN52's closure state).  Each UB shape
gets one row + its own `cluster-<id>-<slug>.md` doc.

| ID | Cluster | Severity | Backend asymmetry | Detector | Doc |
|---|---|---|---|---|---|
| **1** ✅ | **Unaligned `&mut T` into the bytecode buffer** — `code_add::<T>` / `code_put::<T>` / the `code<T>()` read accessor cast a `*u8` into the byte-granular `Vec<u8>` code buffer; at odd offsets with `T=u16`/`u32` this constructs an unaligned reference (UB).  Fires inside `byte_code` → universal to every program.  **FIX LANDED 2026-05-31** (PR #235; `write_unaligned`/`read_unaligned`; `code<T>` returns by value) — Miri-confirmed clean. | Latent UB (masked on x86-64 rustc 1.95; @P383-class toolchain exposure risk) | Universal (both backends compile via `byte_code`) | **Miri** (ASan blind — alignment UB) | [`cluster-1-unaligned-bytecode.md`](cluster-1-unaligned-bytecode.md) |
| **2** ✅ | **Unaligned typed access to the byte-packed eval stack** — `set_string`/stack push-pop wrote `Str`/`DbRef`/8-byte values via `addr_mut::<T>` at byte-granular `stack_pos` → unaligned `&mut Str` (UB).  **FIX LANDED 2026-05-31** (PR #235 + #236): full stack alignment (V2 allocator) became the production default; the `LOFT_ALIGN`/`LOFT_SLOT_V2` flags and the entire V1 allocator were removed — one layout now.  Fix path (A) (align slots in the variable-positioning code) was the approach ultimately taken; path (B) (unaligned-accessors behind a seam) was superseded.  GOALS.md Goal B recorded as "B1 LANDED." | Latent UB (masked on x86-64 rustc 1.95; @P383-class) | Interpret (native has no byte-packed eval stack) | **Miri** (ASan blind — alignment UB) | [`cluster-2-unaligned-store-access.md`](cluster-2-unaligned-store-access.md) |
| **3** ✅ | **Store-aliasing reborrow in cross-store copy** — mutably borrowing the same store twice in `get_disjoint_mut`-precursor code → aliasing `&mut`.  **FIX LANDED 2026-05-31** (PR #236; `get_disjoint_mut`). | Aliasing UB | Interpret + native | **Miri** | `cluster-0-tooling-decision.md` § Progress |
| **4** ✅ | **Uninitialised padding in fn-ref slot read** — reading a fn-ref slot as `[u8;20]` reads the 4-byte alignment padding as initialised bytes (uninit UB).  **FIX LANDED 2026-05-31** (PR #236; `MaybeUninit` for the fn-ref read). | Uninit UB | Interpret | **Miri** | `cluster-0-tooling-decision.md` § Progress |
| **5** ✅ | **`free_text` leak** — `free_text` called `String::clear` + `shrink_to_fit` without deallocating the buffer; the String was then dropped without a heap free → leak.  **FIX LANDED 2026-05-31** (PR #236; `clear` before `shrink_to_fit` removes the leak). | Memory leak | Interpret | **Miri** (leak check) | `cluster-0-tooling-decision.md` § Progress |
| **(PLAN52-I)** | `??`-on-text value-block returns a `Str` borrowing into block-local `_ncc_N`, freed before the consumer reads → heap-use-after-free at the consumer's `copy_nonoverlapping`.  **Owned by [@PLAN52](../../finished/52-value-block-borrow-cleanup/README.md) cluster I — confirmed here under ASan; CLOSED by PLAN52 (#230).** | Heap-UAF (silent corruption; masked on x86-64 rustc 1.95) | Interpret | **ASan** (Miri couldn't reach it — masked behind cluster 1's compile-stage abort) | PLAN52 `cluster I` |

**Wave 1 complete.**  All five PLAN53-owned clusters fixed; PLAN52-I owned and
closed by PLAN52 (#230).  The Miri curated gate (single `p213` test) was
shipped as `D-final` (PR #236); the per-PR `stack_align_guard` and nightly
Miri/ASan/toolchain-matrix workflows are live on `main`.

## Case-finding strategy — actively hunt, don't wait

The point of this plan is **not** to passively catalogue whatever
one spike happens to surface and stop.  The @P383 lesson is that
loft's UB lives latent for many releases and only becomes visible
when something shifts (a rustc bump, an allocator change, a new
consumer's access pattern).  So the working posture is **adversarial
discovery**: assume more problematic cases exist, and go find them
*before* a toolchain roulette does.  Two clusters fell out of a
single trivial program in one afternoon — that is a floor, not a
ceiling.

Active hunting lanes, in priority order:

1. **Sanitizer-driven peeling (in flight).**  Each fix removes a
   *gating* finding and reveals the next layer (cluster 1's fix
   immediately exposed cluster 2).  Re-run Miri/ASan after every
   cluster fix and treat the next abort as the next lead.  Keep
   peeling until a full suite run is clean, not until the first
   finding is fixed.
2. **Differential generation (interpret vs native).**  loft's
   `cross_mode!` harness already runs both backends; most of the
   UB family manifests as interpret↔native divergence.  Build a
   generator that emits random *valid* loft and flags any
   output/exit divergence.  On rustc 1.95 the masked bugs agree on
   both backends (no divergence) — so generated programs MUST run
   under a sanitizer for the masked family; differential alone
   catches the unmasked divergences (native compile errors, logic
   mismatches).
3. **Homegrown arena poison-on-free (the keystone).**  Miri, ASan,
   and Valgrind are all blind to loft's *store-internal* lifetime —
   "freeing" a record is loft's own bookkeeping, not a libc `free()`,
   so the @P377/@P378 dangling-`DbRef` family is invisible to every
   off-the-shelf tool.  A `LOFT_POISON=1` debug mode that fills
   freed store records + freed stack slots with a sentinel turns
   *silent* use-after-free (which on rustc 1.95 reads back stale-
   but-correct bytes) into *loud, deterministic* garbage at the
   dangling read — on any rustc, no nightly.  This is the move that
   makes the arena-internal family machine-detectable; equivalent
   to teaching ASan/Valgrind about the arena via
   `__asan_poison_memory_region` / `VALGRIND_MAKE_MEM_NOACCESS`
   client-requests, but homegrown and zero-dependency.
4. **Coverage-guided fuzzing (`cargo-fuzz` + `arbitrary`).**  A
   structure-aware fuzz target over parse → `byte_code` → execute,
   built with ASan, driving the arena into corner states the
   hand-written probes miss.  Combine with lane 3's poisoning so
   masked UAF fails loudly under the fuzzer.
5. **Targeted slot/stack fuzzing.**  Generate programs with many
   overlapping variable lifetimes, reused names across scopes
   (cf. @P344), nested blocks — to stress `validate_slots` and the
   two-zone slot model directly (a known-fragile subsystem; see
   [SLOTS.md](../../../SLOTS.md)).

Lanes 1-2 use what already exists.  Lane 3 is the highest value-
per-effort and covers the blind spot the external tools share —
build it early.  Lanes 4-5 are heavier and scale coverage once the
detectors are in place.

**Disposition:** new shapes these lanes surface are clusters in the
catalogue above (in-plan, per the policy) — never PROBLEMS.md rows.

## Probe suite (REQUIRED — populated during Stage A)

Probes for this plan are different in shape from PLAN51/PLAN52:
each probe is a small loft program designed to **exercise a code
path likely to harbour UB**, paired with a sanitizer invocation
that turns a finding into a deterministic PASS/FAIL.  The probe is
not "loft program that produces wrong output" but "loft program
that, under sanitizer, produces zero diagnostics".

For the homegrown cluster-2 (eval-stack alignment) work the "sanitizer
invocation" was: run under `LOFT_ALIGN=1 LOFT_SLOT_V2=drive loft --interpret`,
one probe per subprocess under `LOFT_TIMEOUT`.  **Those flags are now gone**
(V2 is the only layout; `LOFT_ALIGN`/`LOFT_SLOT_V2` were removed in PR #236).
The probes in [`probes/`](probes/) record the historical mechanism; the
"flag-OFF PASS / aligned FAIL" distinction no longer applies — all probes pass
on production.  The run script [`probes/run.sh`](probes/run.sh) still works as
a regression smoke-check (all 35 probes must pass with no flags).

**Pass 1 — sub-cluster 2a (generator argument mis-offset across `yield`),
authored 2026-05-30.**  Verified mechanism: a generator reading its own
argument after a `yield` reads it back 4 bytes high (`n=42` → `42<<32`).
See [`probes/README.md`](probes/README.md) for the full table + mechanism.

| File | Shape | Cluster | Aligned now | Flag-OFF |
|---|---|---|---|---|
| `2a-01-gen-arg-single-yield.loft` | `yield n` once — MIN REPRO | 2a | **FAIL** (`42<<32`) | PASS |
| `2a-02-gen-constant-yield-ref.loft` | `yield 7` (no arg) — ref | 2a | PASS | PASS |
| `2a-03-gen-no-arg-while-ref.loft` | `while i<3` (no arg) — ref | 2a | PASS | PASS |
| `2a-04-gen-arg-two-yields.loft` | `yield n; yield n+1` | 2a | FAIL | PASS |
| `2a-05-gen-arg-while-hang.loft` | `while i<n` (p210) | 2a | **HANG** | PASS |
| `2a-06-gen-arg-for-range-hang.loft` | `for i in 0..n` | 2a | HANG | PASS |
| `2a-07-gen-text-arg-format-crash.loft` | text arg + format (p218) | 2a | **CRASH** | PASS |

**All four sub-families now authored — 35 probes covering all 27 aligned-mode
failures** (2a generator-arg ×11, 2b sorted-iter ×8, 2c hash-iter ×7, 2d
composite-format/misc ×9).  `probes/run.sh` exits 0 (every probe PASSES
flag-OFF; every reference PASSES aligned).  Full per-family tables + verified
mechanisms in [`probes/README.md`](probes/README.md).  Distinct mechanisms:
2a shifts an argument value 4 bytes; 2b drops all sorted elements (dead
cursor); 2c adds a phantom leading hash element (off-by-one gather); 2d renders
composite format/json as empty (mis-stepped DbRef handle).

**Probe naming**: `<sub-cluster><NN>-<descriptive>.loft` (e.g. `2a-01-…`);
sub-cluster prefix groups the cluster-2 sub-families.  Bare `NN-…` for
single-cluster plans, matching PLAN51 / PLAN52.

**Promotion gate**: a probe graduates to `tests/scripts/NN-plan53-…`
only when it passes the standard four gates (assertions, clean exit,
no leak, bounded runtime) AND **runs cleanly under the chosen
sanitizer**.  The sanitizer-clean condition is what makes this
plan's promotion gate stricter than PLAN52's.

### Curated probe sets (defined when probe count ≥ 20)

Deferred until probes exist.  Expected shape mirrors PLAN52:
Set A (Miri-safe interpreter), Set B (ASan-safe full suite), Set H
(baselines that must always PASS under both), Set Z (known-deferred
shapes with one-line reason each).

## Reference ↔ problem pairings (populated when probes ≥ 5)

Empty until probes exist.

## Tool gaps

Wave 1 shipped the core sanitizer CI stack.  The table tracks both
Wave-1 final state and Wave-2 gaps.

| Tool | Status | Used for |
|---|---|---|
| Per-PR `stack_align_guard` sweep (`.github/workflows/ci.yml`) | ✅ **SHIPPED** (PR #235 + #236) | Cheap per-PR alignment-UB detector; gates every PR |
| `cargo +nightly miri test` runner (nightly Miri job) | ✅ **SHIPPED** (PR #235; curated `p213` test; `miri.yml`) | Gold-standard hard-UB (alignment/OOB/UAF/uninit) over the interpreter subset |
| `RUSTFLAGS=-Zsanitizer=address` runner (nightly ASan job) | ✅ **SHIPPED** (PR #235; `miri.yml`) | Real-allocator UAF/OOB sweep over the full interpreter corpus |
| Rustc toolchain-matrix (beta + nightly) | ✅ **SHIPPED** (PR #235; `miri.yml`; non-blocking) | Early-warning for toolchain-sensitivity — the @P383 trigger class |
| PR "Nightly health" sticky-comment reporter | ✅ **SHIPPED** (PR #235; `ci.yml` `nightly-status` job) | Surfaces per-job nightly conclusions on each PR; informational, never a merge gate |
| Miri ignore annotations (`#[cfg_attr(miri, ignore)]`) | Partially in tree | Mark FFI-heavy tests Miri cannot run; curated set covers `p213`; rest still open |
| `#[cfg(not(miri))]` gate on `crash_report::install` | **Missing** | Lets the loft *binary* run under Miri (`libc::sigemptyset` is unshimmed); only needed for binary-under-Miri, not `cargo miri test` |
| **`LOFT_POISON=1` arena poison-on-free** (store-record + stack-slot fill on free) | **Missing — recommended Wave-2 keystone** | Makes store-internal use-after-free (the @P377/@P378 dangling-`DbRef` family) detectable on any rustc — the blind spot Miri/ASan/Valgrind all share |
| Differential generator (random valid loft → interpret vs native diff, run under sanitizer) | **Spun off to @PLAN55** | Mine the interpret↔native divergence family + masked UB at scale; reuses `cross_mode!` — see [55-program-level-fuzzing/](../55-program-level-fuzzing/README.md) |
| `cargo-fuzz` target (`fuzz_target!` + `arbitrary`, ASan) — program-level | **Spun off to @PLAN55** | Coverage-guided fuzzing of parse → byte_code → execute; stresses parser/compiler/stack on unseen programs — see [55-program-level-fuzzing/](../55-program-level-fuzzing/README.md) |
| Structure-aware fuzz target for database collections (vector/hash/tree/radix + store allocator) | **Missing** | Direct fuzz target: clean Rust APIs, documented invariants (DATABASE.md), ASan oracle in place |
| ASan / Valgrind custom-allocator annotations (`__asan_poison_memory_region` / `VALGRIND_MALLOCLIKE_BLOCK`) | **Missing** | Teach the *external* tools about the loft arena (alternative to the homegrown `LOFT_POISON` lane) |
| Valgrind Memcheck (informational lane) | Available upstream — not wired | Uninitialised-read detection on the full native binary with no rebuild; complements ASan |
| **ThreadSanitizer (TSan)** | **Missing** | Data-race detector over the parallel/threading suite (`par`/`par_light`); loft has ZERO race coverage today |
| **macOS-ARM nightly leg** | **Missing** | macOS-ARM is where @P383 surfaced; the ubuntu-only nightly would not have caught it |
| **Native-backend ASan** | **Missing** | ASan currently instruments only the in-process interpreter; the `--native` codegen runtime is uninstrumented |
| **MSan (MemorySanitizer)** | **Missing** | Uninitialised-read detection corpus-wide; painful setup (needs instrumented std); lower priority |
| **OSS-Fuzz onboarding** | **Spun off to @PLAN55** | Scale-up from nightly time-box to sustained coverage-guided fuzzing — see [55-program-level-fuzzing/](../55-program-level-fuzzing/README.md) |
| **Failure→issue notifier for the nightly** | **Missing** | Opens/updates a deduped GitHub issue on nightly failure; avoids silent red nightly going unnoticed (per-job conclusions hidden behind `continue-on-error`) |

Wave-2 tools are part of this plan's continued output.  Moving the
plan to `finished/` should leave the CI job + any test annotations in
tree.

## Wave 2 — next wave (open)

Wave 1 installed the sanitizer CI stack and fixed the five UB clusters found by
the initial sweep.  The detector is now live; Wave 2 expands its coverage and
addresses the categories it cannot yet see.  Priorities, in order:

1. **macOS-ARM nightly leg.**  The nightly is ubuntu-only.  @P383 — the founding
   incident — surfaced exclusively on macOS-ARM; a ubuntu-only nightly would not
   have caught it.  Adding a macOS-ARM runner for the toolchain-matrix job (and,
   once affordable, Miri/ASan) closes the largest known platform blind spot.
   Highest priority.

2. **ThreadSanitizer (TSan) over the parallel/threading suite.**  loft executes
   real parallel workloads via `par`/`par_light` under a store-isolation model
   (see THREADING.md).  ZERO race coverage exists today: Miri runs with
   stacked-borrows disabled (not a race detector), ASan and the guard do not see
   races.  TSan is the standout *new category* gap — a different tool class than
   any of the Wave-1 detectors.

3. **Structure-aware / property-based fuzzing of the database collections +
   store allocator, under ASan.**  The prime *direct* fuzz target: clean Rust
   APIs, documented invariants (DATABASE.md), and the ASan oracle is now in
   place.  Random op-sequences on `vector`/`hash`/`tree`/`radix` checked against
   a reference model (`std::HashMap` / `BTreeMap` / `Vec`).  Note: the *stack*
   is not a good direct fuzz target (it is bytecode-driven; random bytecode is
   invalid-by-construction) — it is best fuzzed indirectly via item 4 below.

4. **Program-level loft-source fuzzing** (`cargo-fuzz` over parse → `byte_code`
   → execute) **under ASan + the guard feature.**  Stresses parser/compiler/stack
   on unseen programs; seeds from the ~2000 existing `.loft` tests.  Expect an
   initial robustness/panic-triage wave (malformed input surfaces `unwrap`/panic
   paths first).  Cross-references Case-finding strategy lanes 4-5 above (expand
   there, don't duplicate).  **Spun off to @PLAN55 — see
   [`plans/future/55-program-level-fuzzing/`](../55-program-level-fuzzing/README.md).**

5. **`LOFT_POISON=1` arena poison-on-free keystone.**  Already described in
   Case-finding strategy lane 3; still **missing**.  Fills freed store records
   + freed stack slots with a sentinel, turning silent store-internal UAF (the
   @P377/@P378 dangling-`DbRef` family) into loud, deterministic garbage at the
   dangling read — on any rustc, no nightly required.  High value-per-effort;
   pairs with the fuzzers.

6. **Differential fuzzing (interpret ≡ native ≡ wasm).**  Fold fuzzing into
   Goal C (cross-backend parity): the same program-level fuzzer from item 4, run
   on all three backends, flags output divergence as a finding.  Reuses
   `cross_mode!` infrastructure.  **Spun off to @PLAN55 (F3) — see
   [`plans/future/55-program-level-fuzzing/`](../55-program-level-fuzzing/README.md).**

7. **Grow the Miri curated set** beyond the single `p213` test.  Add the
   cluster 1-5 reproducers + representative text/fn-ref/par shapes so the Miri
   gate covers more of the hard-UB surface without an unbearable runtime.

8. **Triage the LeakSanitizer baseline** (~108 live-at-exit allocations) so
   ASan leak detection can be turned on corpus-wide.  Currently `detect_leaks=0`
   in `miri.yml`.  Cluster 5 was a leak; there are likely others.

9. **Native-backend ASan.**  ASan currently instruments only the in-process
   interpreter.  The `--native` codegen runtime is uninstrumented and is where
   @P229/G2/G3 lived.

10. **OSS-Fuzz onboarding.**  A nightly time-box is the start; OSS-Fuzz is the
    scale-up for genuinely sustained, coverage-guided fuzzing with a much larger
    budget than CI allows.  **Spun off to @PLAN55 (F5) — see
    [`plans/future/55-program-level-fuzzing/`](../55-program-level-fuzzing/README.md).**

11. **MSan (MemorySanitizer) corpus-wide.**  Uninitialised-read detection beyond
    what Miri covers; lower priority (setup requires a fully instrumented std).

12. **Failure→issue notifier for the nightly.**  A job that opens/updates a
    deduped GitHub issue when the nightly fails, reading per-*job* conclusions
    (not just the overall run status, which `continue-on-error` holds green even
    when matrix legs are red).

## Status & next-session roadmap

Wave 1 is complete.  The table below covers Wave 2 steps.

| # | Step | Exit criteria | Effort | Risk |
|---|---|---|---|---|
| ~~**0**~~ | ~~WAIT — verify PLAN52 closure~~ | PLAN52 closed (#230) — done | done | — |
| ~~**A1**~~ | ~~Tooling decision~~ | `cluster-0-tooling-decision.md` committed | done | — |
| ~~**A2**~~ | ~~First sweep~~ | Cluster catalogue populated (clusters 1-5 + PLAN52-I) | done | — |
| ~~**A3**~~ | ~~Probe authoring~~ | 35 probes in `probes/`; all PASS production | done | — |
| ~~**B-D**~~ | ~~Per-cluster mechanism + fix commits~~ | Clusters 1-5 all fixed (PR #235/#236) | done | — |
| ~~**D-final**~~ | ~~Wire the CI job~~ | per-PR guard + nightly Miri/ASan/matrix + sticky-comment live on `main` | done | — |
| **W2-1** | **macOS-ARM nightly leg** — add macOS-ARM runner to `miri.yml` toolchain-matrix job | macOS-ARM leg green on `main`; nightly badge reflects it | 1 session | LOW |
| **W2-2** | **ThreadSanitizer job** — add a `tsan` job to `miri.yml` running the parallel/threading suite under `RUSTFLAGS=-Zsanitizer=thread` | TSan job green on `main`; any races found catalogued or fixed | 1-2 sessions | MEDIUM (TSan setup) |
| **W2-3** | **`LOFT_POISON=1` keystone** — implement arena poison-on-free for store records + stack slots | `LOFT_POISON=1 cargo test` green; @P377/@P378-class reads produce sentinel-value panics rather than silent stale data | 1-2 sessions | MEDIUM |
| **W2-4** | **Database collections fuzz target** (structure-aware, under ASan) | `cargo fuzz run db_collections` runs 10 min with no ASan finding | 2-3 sessions | LOW-MEDIUM |
| **W2-5** | ~~**Program-level loft-source fuzz**~~ — **spun off to @PLAN55** ([`plans/future/55-program-level-fuzzing/`](../55-program-level-fuzzing/README.md)) | Tracked in @PLAN55 | — | — |
| **W2-6** | **Grow the Miri curated set** | Cluster 1-5 reproducers + par shapes in the Miri job; job runtime ≤ 20 min | 1 session | LOW |
| **W2-final** | **Close the plan** — move to `plans/finished/` | All Wave-2 items done or explicitly deferred with a one-line reason; sanitizer CI green; `make ci` green | 1 session | none |

### "We know we're clear" — binary close criteria

**Wave 1 criteria (all satisfied 2026-05-31):**

1. Clusters 1-5 fixed; PLAN52-I owned and closed by PLAN52 (#230). ✅
2. Per-PR `stack_align_guard` + nightly Miri/ASan/matrix + sticky-comment reporter
   live on `main`. ✅
3. `make ci` green; no existing gate broken. ✅
4. PLAN52 probe sets A-I still PASS against the post-Plan53 tree. ✅
5. rustc nightly baseline recorded: nightly 1.98.0 (2026-05-28) in
   `cluster-0-tooling-decision.md`. ✅

**Wave 2 close criteria (the plan moves to `finished/` when ALL hold):**

1. All Wave-2 step exit criteria in the table above are met, or the step
   is explicitly deferred with a one-line reason in this README.
2. Sanitizer CI (per-PR guard + nightly) still green on `main`.
3. `make ci` green.
4. rustc-version-aware note updated for the nightly the Wave-2 CI was
   last green against.

If any criterion fails, the offending step grows a "Fix iterations" note.

### Aggregate effort

**Wave 1 — actual cost:** ~2 weeks (PLAN52 gate + 5-cluster fix arc +
CI wiring).  The first sweep surfaced 5 clusters (2 alignment-class
caught by Miri, 3 more after peeling); the V2 aligned-stack work grew
beyond the original scope but shipped as a clean arc.

**Wave 2 — estimated cost:** 3-6 weeks depending on how many fuzz
findings surface.  The `LOFT_POISON` keystone and macOS-ARM leg are
the quickest wins (1-2 sessions each).  TSan and the fuzz targets are
the bulk of the work.  OSS-Fuzz and the failure-notifier are
stretch items with unbounded upstream dependency.

## Fix-application discipline

Inherits verbatim from PLAN52 / @PLAN51: **one cluster per commit,
pushed before the next cluster begins**.  See the investigation-
plan template's § Fix-application discipline for the full rationale.

Two plan-specific notes:

- **The CI job ships as its own commit**, at the end of the fix
  arc.  Don't bundle the workflow change with a cluster fix — the
  workflow either turns red against today's tree (because UB still
  exists) or green (because all UB is fixed), and that signal
  needs its own commit to be interpretable.
- **Sanitizer findings sometimes alias** — fixing cluster X may
  silence cluster Y's finding without addressing Y's root cause.
  After each cluster commit, re-run the full sanitizer sweep
  (not just the per-cluster probe) and update the cluster
  catalogue: mark any incidentally-closed clusters with
  `✅ closed incidental to <X>` and verify Y's probe under
  sanitizer to confirm — `sanitizer is quiet` is not proof of
  fix, only of detection-quietness.

## See also

- [`plans/future/55-program-level-fuzzing/`](../55-program-level-fuzzing/README.md) — spinoff plan; owns program-level loft-source fuzzing, schema-coupled collection fuzzing (tree/hash/sorted via real programs), differential fuzzing, and OSS-Fuzz onboarding (Wave-2 items #4, #6, #10).
- [`plans/finished/52-value-block-borrow-cleanup/`](../../finished/52-value-block-borrow-cleanup/README.md) — founding hard dependency (now satisfied, closed via PR #230); the cluster-I heap-UAF was the dominant noise that had to be removed before this plan's sweep was meaningful.
- [`plans/finished/51-hidden-buffer-aliasing/`](../../finished/51-hidden-buffer-aliasing/) — sibling investigation; canonical layout reference for cluster docs and probe organisation.
- [`doc/claude/PROBLEMS.md`](../../../PROBLEMS.md) §@P383 — the trigger incident; the failure mode this plan's CI lever would have caught months earlier.
- [`doc/claude/TESTING.md`](../../../TESTING.md) — sanitizer-CI documentation (shipped D-final, PR #236).
- [`.github/workflows/ci.yml`](../../../../../.github/workflows/ci.yml) — per-PR `stack_align_guard` job + nightly-status sticky-comment reporter (shipped).
- [`.github/workflows/miri.yml`](../../../../../.github/workflows/miri.yml) — nightly Miri + ASan + toolchain-matrix job (shipped).
- [`Makefile`](../../../../../Makefile) — `make ci-sanitizer` target (Wave 2: not yet added; Wave 1 wired the CI job directly).
