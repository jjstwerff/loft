<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN54 — design for the remaining arcs (S9, S4, S5, S1, S7, S8)

> Companion to [README.md](README.md). **Done already:** S2 (TSan), S3
> (`LOFT_POISON`, both halves), S6 (native-backend ASan). This doc designs the
> rest with concrete, command-level steps, each grounded in what the S2/S3/S6
> work actually established about the toolchain — so they are recipes, not
> guesses. Every arc names its **invariant**, its **positive control** (a green
> gate is vacuous without one — the recurring lesson of this plan), its **CI
> shape**, and an honest **effort/risk**.

## Toolchain facts this plan established (reused below)

- **ASan tolerates linking against an *uninstrumented* libloft** for a binary —
  no `-Zbuild-std` (S6). It uses `--target x86_64-unknown-linux-gnu` + global
  `RUSTFLAGS=-Zsanitizer=address` + nextest (drops doctests: the curve25519
  proc-macro `E0463`). This is the `asan` job's shape.
- **TSan / MSan need an *instrumented* std** (`-Zbuild-std` + `rust-src`), and the
  sanitizer flag must be **target-scoped** (`--config target.<triple>.rustflags`),
  NOT global `RUSTFLAGS`, or host proc-macros get sanitized and ABI-mismatch (S2).
- **A cross-target build splits deps:** normal rlibs → `target/<triple>/release/deps`,
  **proc-macros (host artifacts) → `target/release/deps`** (S9). libloft pulls ~6
  proc-macros.
- **A non-standard target dir needs a `default/` stdlib symlink** beside the
  binary (`ln -s ../../../default target/<triple>/release/default`) — the loft
  binary resolves the stdlib relative to itself (S9).
- **Positive control per sanitizer** is mandatory: poison → read an unwritten
  slot; TSan → two threads race one byte; ASan → raw-pointer OOB. Each fired.

---

## S9 — mixed-boundary (C71) cdylib ASan — the hard remaining half

Full analysis + the three falsifying probes are in
[NATIVE_ASAN_DESIGN.md § S9](NATIVE_ASAN_DESIGN.md). This is the finish plan.

**Invariant:** a cross-boundary use-after-free / out-of-bounds on the `*mut Stores`
an interpreted host shares with a dlopen'd native cdylib is caught by ASan, not
silent. Requires BOTH sides instrumented: the host (so the store allocation is
ASan-tracked — malloc intercepted) and the cdylib (so its accesses are checked).

**Candidate design (probe-grounded, UNVALIDATED):** under `LOFT_NATIVE_ASAN`, make
`build_shared_cdylib` link the **complete-deps stable `target/release` libloft**
(probe 3: proc-macros resolve, cdylib compiles clean) while still compiling the
cdylib with `-Zsanitizer=address` and loading it into the **ASan host**. Rests on
two facts to prove first.

**Steps:**

1. **Prove fact (a) — an ASan cdylib may link a non-ASan libloft.** Compile the
   `datalib` cdylib `.rs` with `-Zsanitizer=address` against the *stable*
   `target/release/deps/libloft.rlib` + stable deps; confirm the `.so` builds and
   is instrumented (`nm …_datalib.so | grep -c __asan` > 0). (Probe 3 showed it
   *compiles/runs*; this step confirms it is actually ASan-instrumented.) S6 makes
   this very likely — it is the cdylib analogue of the S6 binary result.
2. **Prove fact (b) — two libloft copies share the store.** Implement the resolver
   override: when `LOFT_NATIVE_ASAN` is set, `build_shared_cdylib` picks the
   stable `target/release` libloft.rlib (via a `cache::stable_libloft_dir()` that
   ignores the running binary's cross-target location), not `find_loft_rlib()`.
   Then run the `datalib` mixed corpus under the **ASan loft host** (built
   `RUSTFLAGS=-Zsanitizer=address cargo build --target x86_64-unknown-linux-gnu`,
   `default/` symlink in place). Assert: correct output AND the loaded `.so` is
   ASan (`nm`), i.e. an ASan cdylib carrying a stable libloft dlopen'd into an
   ASan host carrying an ASan libloft shares `*mut Stores` correctly (the `Store`
   struct layout is ASan-invariant — redzones wrap allocations, not fields).
3. **Positive control (non-vacuity).** Add a `datalib` native fn (or a temporary
   patch to the generated cdylib) that reads one element past the shared vector's
   backing, e.g. `store[len]`. Under the ASan host + ASan cdylib it must report
   `heap-buffer-overflow` at the boundary; without ASan it is silent (today's
   gap). Keep it as a throwaway (delete after), like the S2/S6 controls.
4. **Fallback if fact (b) fails** (dlopen rejects two libloft SVHs, or the store
   is mis-shared): switch to the per-proc-macro `--extern` approach — enumerate
   libloft's ~6 host proc-macros (`curve25519_dalek_derive`, `displaydoc`,
   `thiserror_impl`, `yoke_derive`, `zerofrom_derive`, `zerovec_derive`),
   version-match each host `.so` against libloft's metadata (the same technique as
   `native_lib::loft_ffi_for_libloft`), and add explicit `--extern name=<so>` to
   the ASan cdylib build. Deterministic, fragile to dep-graph changes; use only if
   the cleaner design is unsound.
5. **CI job `mixed-asan`** (nightly, after validation): `dtolnay/rust-toolchain@nightly`
   + `rust-src`; build the ASan loft host (`RUSTFLAGS=-Zsanitizer=address cargo
   build --release --target x86_64-unknown-linux-gnu --bin loft --lib`); symlink
   `default/`; run the `datalib` mixed corpus + the injected-OOB positive control
   under `LOFT_NATIVE_ASAN=1 ASAN_OPTIONS=detect_leaks=0`.

**Acceptance:** the mixed corpus is ASan-clean AND the injected cross-boundary OOB
is caught. **Effort:** a focused session (steps 1–3 are the risk; 4 is the safety
net). **Risk:** medium-high — fact (b) is genuinely unproven.

---

## S4 — LeakSanitizer triage → `detect_leaks=1`

**Invariant:** every allocation live at process exit is either freed or an
explicitly-accepted, annotated intentional leak — so a NEW leak (a real
store/String bug) turns the ASan gate red instead of hiding in the ~108-allocation
baseline the current `detect_leaks=0` mutes.

**Steps:**

1. **Capture the baseline.** Reproduce the `asan` job locally with leaks ON:
   `RUSTFLAGS=-Zsanitizer=address ASAN_OPTIONS=detect_leaks=1 cargo +nightly
   nextest run --profile ci --release --target x86_64-unknown-linux-gnu --lib
   --test issues -E 'not (test(library_suite) | test(fill_rs_up_to_date) |
   test(n9_generated_fill_matches_src) | test(native_rs_functions_up_to_date))'`
   Collect every `Direct leak` / `Indirect leak` stack.
2. **Classify by allocation site.** Expected classes, most benign first:
   **intentional-by-design** — `OnceLock`/`lazy_static` process caches
   (`keys::poison_enabled` & the whole `*_enabled` family, the `rayon_pool`
   `OnceLock`, interners, env-flag caches) leak once and never grow;
   **framework** — the test harness / nextest;  **avoidable** — a store or
   `String` not freed at exit (a real bug, the S4 payload). Cluster-5 was one such;
   assume others.
3. **Fix the avoidable class** at its owner (the same store-lifetime discipline as
   @PLN85 — free at the chokepoint, add a `tests/scripts/85-*` guard if it is a
   store leak).
4. **Suppress the intentional class explicitly.** Write `lsan_suppressions.txt`
   with one `leak:<symbol-or-file>` line per accepted class + a one-line rationale
   comment each (no blanket `leak:*`). Commit it beside the workflow.
5. **Flip the gate.** In `miri.yml` `asan` job: `ASAN_OPTIONS: 'detect_leaks=1'` +
   `LSAN_OPTIONS: 'suppressions=lsan_suppressions.txt'`.

**Positive control:** a `#[test]` (asan-only) that `Box::leak`s a value and asserts
LSan would flag it — or simply trust that `detect_leaks=1` catching real leaks is
self-evident once the baseline is zero; the *real* control is that the baseline
went from ~108 to 0-modulo-suppressions.
**Acceptance:** `detect_leaks=1` passes the interpreter corpus in CI, or each
survivor has a one-line accepted-leak annotation. **Effort:** ~1 day. **Risk:**
low — worst case the suppression file is larger than hoped.

---

## S5 — grow the Miri curated set (≥ 8 tests, ≤ 20 min)

**Invariant:** the Miri hard-UB gate exercises the representative shapes of loft's
UB surface (alignment / OOB / uninit / UAF), not just the 4 seed tests, so a
regression in a common shape lands red within a day.

**Constraint (hard):** Miri **cannot** run threads or FFI — so every candidate
must be a pure-compute `issues` test. `par`, native, wasm, html shapes are OUT.

**Steps:**

1. **Candidate shapes** (fill the gaps in the current p213 + cluster-3/4/5 set):
   a nested-record / struct-in-struct read; an enum-payload extract; a
   text/`String` slice + concat; a fn-ref stored-and-called (closure capture); a
   vector amortised-growth (reallocation); and the poison-era regression shapes
   that are pure-compute (e.g. an `85-*` reproducer ported to a Rust `issues`
   test). Aim for ~6 new → ≥ 10 total.
2. **Validate each under Miri before adding it:** `cargo +nightly miri test --test
   issues -- --exact <name>` with `MIRIFLAGS='-Zmiri-disable-isolation
   -Zmiri-disable-stacked-borrows'` (the job's flags). Keep only Miri-clean ones;
   a test that trips Miri is either a real UB bug (fix it) or a shape Miri can't
   model (drop it, note why).
3. **Add the validated names** to the `--exact` list in `miri.yml`'s `miri` job;
   confirm total job runtime ≤ 20 min (Miri is ~100× slower — budget ~2 min/test).

**Positive control:** the curated set already has one implicitly — the cluster-3/4/5
tests each pinned a real UB fix; a new addition should likewise correspond to a
shape whose UB Miri provably catches (verify by reverting the fix on a throwaway
branch → Miri red). **Acceptance:** ≥ 8 tests, job ≤ 20 min, all green.
**Effort:** ~½ day — **tractable enough to just do** rather than stage.

---

## S1 — macOS-ARM sanitizer leg — RECOMMEND DEFER (with a one-line note)

**State:** `v2-validation.yml` already runs the **full suite** on macOS-ARM
(`macos-latest` = ARM64). Only the *sanitizer* (Miri / ASan) leg is ubuntu-only.
The founding incident (@P383) was a macOS-ARM toolchain-sensitivity bug — but that
class is now caught by the full-suite macOS-ARM run; the residual is only
"sanitizer-detectable UB that manifests ONLY on ARM and NOT on the ubuntu
sanitizer leg", a narrow slice.

**If pursued:** add `strategy.matrix.os: [ubuntu-latest, macos-latest]` to the
`asan` and `miri` jobs (`runs-on: ${{ matrix.os }}`). ASan + Miri both run on
macOS-ARM; the ASan runtime differs (Apple clang) but nightly rustc's
`-Zsanitizer=address` supports `aarch64-apple-darwin`. Watch for: mold is
Linux-only (drop it on macOS), and the `--target` triple must be
`aarch64-apple-darwin`.

**Recommendation:** **defer** — land a one-line note in the S1 row ("covered by
the full-suite macOS-ARM run in v2-validation.yml; sanitizer-on-ARM is a low-value
residual"). **Effort:** ~½ day if built; 1 line if deferred.

---

## S7 — nightly failure → deduped GitHub issue notifier

**Invariant:** a red nightly *job* surfaces as a tracked GitHub issue within 24 h —
because the overall run status is unreliable (`continue-on-error` / `fail-fast:
false` legs hold it green while a matrix leg is red), so per-**job** conclusions
are the signal.

**Steps:**

1. Add a `notify` job to `miri.yml` with `needs: [miri, asan, poison, tsan,
   native-asan, toolchain-matrix, index-hygiene]` and `if: always()` (so it runs
   even when a needed job failed).
2. In the job, read each dependency's `needs.<job>.result`; collect the names
   whose result is `failure`.
3. If any failed, `gh issue` **find-or-update**: search open issues by a stable
   marker (a `nightly-failure` label or a fixed title prefix
   `Nightly sanitizer failure:`); if one exists, add a comment with the run URL +
   failed legs; else `gh issue create` with that label. Dedup so a week of red
   nightlies is ONE issue, not seven. (Model the `gh` usage on the existing
   `stale-plans-audit` job's `GH_TOKEN` pattern.)
4. On an all-green run, optionally auto-close/comment the open failure issue.

**Positive control:** trigger it once via `workflow_dispatch` on a branch with a
deliberately-failing sanitizer step; confirm exactly one issue is opened, and a
second failing run comments rather than duplicates.
**Acceptance:** a forced nightly failure → a tracked issue within 24 h; repeats
dedup. **Effort:** ~½ day. **Risk:** low; needs `issues: write` permission on the
job (`permissions: { issues: write }`).

---

## S8 — MSan (MemorySanitizer) — RECOMMEND DEFER (stretch)

**Goal:** uninitialised-read detection beyond Miri's model, corpus-wide.

**Why deferred:** MSan is the **heaviest** setup of the family — like TSan it needs
`-Zbuild-std` + `rust-src` + target-scoped `-Zsanitizer=memory`, but it is
**stricter**: ANY uninstrumented code that touches memory (the precompiled parts,
any C/FFI) produces false "use-of-uninitialized-value" positives, so it in practice
needs a fully-instrumented dependency closure. loft's crypto/image C-ish deps make
a clean MSan run costly.

**If pursued:** mirror the S2 TSan job with `-Zsanitizer=memory`, scope to the
purest interpreter subset (`--test issues` only), and expect an
initial false-positive triage pass. **Recommendation:** keep deferred with this
one-line cost note until S9/S4/S5/S7 land; Miri already covers much of the
uninit surface the `plan53_cluster4` MaybeUninit fix targeted.

---

## Recommended order (value / effort)

1. **S5** (½ day, tractable now) — cheap coverage, no toolchain risk.
2. **S4** (~1 day) — turns the muted leak baseline into a live gate; low risk.
3. **S7** (½ day) — makes every nightly leg (incl. the 3 new gates) actionable.
4. **S9** (focused session) — the high-value, higher-risk mixed-boundary finish.
5. **S1 / S8** — defer with the one-line notes above; revisit only if a concrete
   need appears.

Each lands as its own commit on the sanitizer branch, each with its positive
control, each flipping its README row to ✅ with the green reading.
