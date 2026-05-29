<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 53 — Sanitizer CI lever (UB detection before rustc-release roulette)

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue (UB shapes Miri/ASan surface) | 🔴 not started — gated on PLAN52 closure |
| B — Mechanism investigation (per UB shape) | 🔴 not started |
| C — Fix design (OPTIONAL) | ⏸️ pending Stage B |
| D — Implementation (CI job + per-cluster fixes) | ⏸️ pending Stage B |

**Hard dependency: this plan does NOT start until [@PLAN52](../../finished/52-value-block-borrow-cleanup/README.md) is closed.**
PLAN52's cluster I (post-consumer `OpFreeText` on a borrowed `Str`)
is the canonical UB this lever will detect, and running Miri / ASan
against an unfixed PLAN52 working tree would produce a flood of
findings that all alias to PLAN52's one root cause — drowning out
any *other* UB the sweep would reveal.  Wait until PLAN52's Set H
+ Sets A/B/C/D/E/F/G all PASS on both backends before Stage A here
begins.

**Trigger:** the @P383 / rustc 1.96 incident.  loft's IR carried a
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

Empty until Stage A starts.  Each UB shape Miri/ASan surfaces gets
one row + its own `cluster-<id>-<slug>.md` investigation doc.

| ID | Cluster | Severity | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| _(TBD)_ | _to be populated after first sanitizer run against post-PLAN52 main_ | — | — | — | — |

## Probe suite (REQUIRED — populated during Stage A)

Probes for this plan are different in shape from PLAN51/PLAN52:
each probe is a small loft program designed to **exercise a code
path likely to harbour UB**, paired with a sanitizer invocation
that turns a finding into a deterministic PASS/FAIL.  The probe is
not "loft program that produces wrong output" but "loft program
that, under sanitizer, produces zero diagnostics".

Empty until Stage A starts.

| File | Shape | Cluster | Status under Miri | Status under ASan |
|---|---|---|---|---|
| _(TBD)_ | _to be populated_ | — | — | — |

**Probe naming**: `NN-<descriptive>.loft` matching PLAN51 / PLAN52.

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

| Tool | Status | Used for |
|---|---|---|
| Sanitizer CI job (`.github/workflows/ci.yml` new job) | **Missing — the deliverable** | The gating job this plan ships |
| `cargo +nightly miri test` runner | Available upstream — not yet wired | Strictest UB detector for the interpreter subset |
| `RUSTFLAGS=-Zsanitizer=address` runner | Available upstream — not yet wired | Real-allocator UB detector for the full suite |
| Miri ignore annotations (`#[cfg_attr(miri, ignore)]`) | Available upstream — none in tree yet | Mark FFI-heavy tests Miri cannot run; Stage A audits which tests need this |
| Sanitizer-only test profile in `Cargo.toml` / `.cargo/config.toml` | **TBD during Stage D** | Avoid contaminating default test runs with sanitizer overhead |

Tools added during this plan are part of its output, not separate
work.  Closing the plan should leave the CI job + any test
annotations in tree.

## Status & next-session roadmap

Each step has a binary exit criterion.  The plan is provably closed
iff Step D8's exit criteria all hold.

| # | Step | Exit criteria | Effort | Risk |
|---|---|---|---|---|
| **0** | **WAIT — verify PLAN52 closure.**  Read PLAN52 README; confirm "we know we're clear" criteria 1-5 hold.  If not, this plan stays parked. | PLAN52 README shows all sets A-I PASS on both backends; `make ci` green; PLAN52 moved to `plans/finished/` | 0 — gating check | — |
| **A1** | **Tooling decision** — write `cluster-0-tooling-decision.md`: pick Miri / ASan / both based on a small spike (run each against `tests/issues.rs` on post-PLAN52 main, measure runtime + finding count + FFI-blocked test count). | Decision committed to `cluster-0-tooling-decision.md` with the spike numbers | 1 session | LOW |
| **A2** | **First sweep** — run the chosen sanitizer against the full applicable test surface on post-PLAN52 main.  Catalogue every distinct finding as a cluster row in this README; create one `cluster-<id>-<slug>.md` per shape | Cluster catalogue populated; every finding mapped to a cluster | 1-2 sessions | LOW — read-only |
| **A3** | **Probe authoring** — for each cluster, write a minimal probe in `probes/` that reproduces the finding deterministically; pair with a reference probe that does NOT trigger | Probe table populated; each problem probe paired with a reference; Set H baselines defined | 2-3 sessions | LOW |
| **B**  | **Mechanism investigation** per cluster — populate each `cluster-<id>-<slug>.md` with verified-vs-hypothesised mechanism, fix surface, options ranked | All cluster docs reach "Ready to fix" state per their own readiness table | 1 session per cluster | LOW-MEDIUM |
| **C**  | (OPTIONAL — skip when mechanism uniquely determines fix) Fix-shape comparison for clusters with multiple viable fix options | Chosen option recorded per cluster doc | varies | LOW |
| **D1..N** | **Per-cluster fix commits** — one cluster per commit, pushed before next cluster begins (PLAN52 fix-application discipline applies verbatim) | Each cluster's probe + Set H PASS under the chosen sanitizer; project CI still green; commit pushed | 1-3 days per cluster | varies — scope-pass fixes carry HIGH risk; codegen / store-op fixes vary |
| **D-final** | **Wire the CI job** — add the sanitizer job to `.github/workflows/ci.yml` (and `make ci-sanitizer` Makefile target).  Decide gating policy: required-to-pass vs informational | New CI job green on `main`; required-to-pass for PRs touching `src/state/`, `src/parser/`, `src/scopes.rs`, `src/generation/`, `src/store.rs`, `src/database/` (the UB-relevant surface) | 1 session | LOW |
| **D8** | **Close the plan** — graduate one representative probe per cluster to `tests/scripts/15X-plan53-…`; move cluster docs to `plans/finished/53-…/` | All clusters fixed or explicitly recorded as known-deferred (with one-line reason in this README); sanitizer CI green on `main`; `make ci-full` green; this plan moved to `finished/` | 1 session | none |

### "We know we're clear" — binary close criteria

The plan is provably closed iff ALL of these hold after Step D8:

1. **Chosen sanitizer runs clean against the gated test surface** on
   post-PLAN52 main + this plan's fixes.  Zero unfixed findings; any
   deferred cluster has an explicit one-line reason in this README's
   cluster catalogue.
2. **Sanitizer CI job green on `main`** — verified via the new
   workflow's last run on `main`.
3. **`make ci` + `make ci-full` still green** — the sanitizer job
   does not break any existing gate.
4. **No regression in PLAN52's probe sets A-I** — re-run PLAN52's
   `probes/run_set.sh all` against the post-Plan53 tree; all sets
   still PASS.  PLAN52 acts as the upstream-canary for this plan
   the way moros_* did for PLAN51.
5. **rustc-version-aware notes** — README documents which rustc
   nightly the sanitizer was last green against, so future toolchain
   bumps have a known-good baseline.

If any of (1)-(5) fail, the plan is NOT closed: the offending
cluster doc grows a new "Fix iterations" entry.

### Aggregate effort

Estimated ~1-2 weeks once PLAN52 closes.  The first sweep (A2) is
the biggest unknown: it may surface zero clusters (PLAN52 already
removed the dominant UB), in which case D1..N collapses and the
plan becomes a 2-3-day infrastructure ship.  It may surface a
handful (closure-capture, store-ownership-on-teardown, native
codegen lifetime), in which case ~1 week of fix work follows.

Quickest user-visible win: **D-final wires the CI job even if zero
fixes were needed** — that single commit prevents the next P383.
If A2 finds no clusters, prioritise shipping the CI job before
declaring the plan closed.

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

- [`plans/futu../../finished/52-value-block-borrow-cleanup/`](../../finished/52-value-block-borrow-cleanup/README.md) — **hard dependency**; this plan does not start until PLAN52 closes.
- [`plans/finished/51-hidden-buffer-aliasing/`](../../finished/51-hidden-buffer-aliasing/) — sibling investigation; canonical layout reference for cluster docs and probe organisation.
- [`doc/claude/PROBLEMS.md`](../../../PROBLEMS.md) §@P383 — the trigger incident; the failure mode this plan's CI lever would have caught months earlier.
- [`doc/claude/TESTING.md`](../../../TESTING.md) — destination for the sanitizer-CI documentation once D-final ships.
- [`.github/workflows/ci.yml`](../../../../../.github/workflows/ci.yml) — where the new job is added at D-final.
- [`Makefile`](../../../../../Makefile) — destination for the `make ci-sanitizer` target.
