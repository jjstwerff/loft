<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 100 — Declarative project build & test phase

> **Live status · lifecycle:** [loft-lang/plans ▸ @PLN100](https://github.com/loft-lang/plans/issues/100) ← single source of truth for lifecycle state (label). This README is the source of truth for **per-phase status + design**.

## Status

**`status:active` — filed 2026-07-08. Slices 1–4 SHIPPED (2026-07-08); only Slice 5 (UI) remains.** A loft
project has no declared build phase: what an artifact needs (which toolchain binaries,
which prebuilt runtime rlibs, which generated data files) lives only in `Makefile`
recipes and scattered Rust fallbacks, so it can't be checked or built automatically.
Three consequences drive this plan: (1) you must run a **manual `make`** to get a
correct WASM build; (2) build shapes **cross-invalidate** — `make wasm` (wasm-bindgen)
and `--html` write the *same* `target/wasm32-unknown-unknown/release/libloft.rlib` with
incompatible feature sets and stomp each other (`src/main.rs:5878`); (3) the resulting
staleness makes `html_wasm`/`html_asyncify`/`wasm_debug_relay` **flaky** under the
parallel suite (they pass in isolation). This plan makes the build phase **declared**,
**auto-built on stale-or-missing**, and **isolated** per target.

## Goal

A `[build]` + `[test]` section in `loft.toml` that declares named build **targets**
(triple × feature-shape), custom **asset steps**, and **tests**; loft fingerprints each
declared output and builds/regenerates it automatically, isolating each target so none
invalidates another, then **runs the declared tests over the built targets and the
generated data files** — retiring the manual `make` steps and the flaky-WASM class, with
config form-shaped for a future UI. The pipeline is one loop: **resolve requires → build
targets → run asset steps → run tests**.

## Effort + design

- **Effort:** MH (Slice 1 is S; the config driver + asset steps are M each)
- **Design:** ~ (Slice 1 detailed; Slices 2–3 sketched, schema needs one pass)
- **Last touched:** 2026-07-08

## Composition matrix — N/A

Tooling / build-pipeline plan: it adds no new language value, type, or operation, so
there is no composition surface to matrix. The "spec = every cell green on both backends"
discipline is replaced here by: **each declared artifact is fingerprinted, and the
regression is that a stale/missing/stomped artifact is auto-rebuilt correctly** (Slice 1
graduates the current flaky WASM tests to deterministic by removing the stomp).

## What already exists to build on

Half the engine is present, and it's the hard half:

| Mechanism | Location | Does |
|---|---|---|
| `loft_ffi_fingerprint`, `native_artifact_cache_key` | `src/cache.rs` | content/ABI fingerprint of the loft runtime |
| `native_artifact_fingerprint_matches`, `write_native_artifact_fingerprint` | `src/cache.rs` | stamp + check a built artifact's sidecar |
| `auto_build_native`, `auto_build_native_target` | `src/native_utils.rs` | fingerprint-gated **auto-rebuild-on-stale** of a **user-package** rlib, host + wasm |
| the host-vs-wasm stale gates | `src/native_utils.rs:952`, `:971` | already rebuild a stale/missing *package* rlib on every compile (cheap: a fingerprint read) |
| `install-artifacts`, `rebuild-native-cdylibs` | `Makefile:147`, `:269` | the **manual** recipes this plan replaces — they encode today's undeclared requirements |
| toolchain inventory | `scripts/doctor.sh` | the "what binaries" list, not yet machine-readable |
| `--html` shape self-check + stomp error | `src/main.rs:5878` | already *detects* the stomp; today it errors and tells you to `make`, instead of fixing it |

The gap: none of this covers the **loft-runtime** rlib per target (only user-package
rlibs auto-build); shapes are not isolated; and there is no config surface.

## What the build actually needs (becomes the declaration)

**Binaries (→ `requires`):** `rustc`/`cargo`; rustup targets `wasm32-unknown-unknown`
(`--html`) and `wasm32-wasip2` (`--native-wasm`); `rust-lld` (bundled); optional
`wasm-opt`/binaryen (asyncify — `src/main.rs:5860` already warns when absent);
`wasm-bindgen`/`wasm-pack` (gallery bundle only).

**Data files (→ per-target `outputs` loft owns):** per target, a `libloft.rlib` +
`deps/*.rlib` built from *this* loft source with a *specific* feature set; the stdlib
`default/*.loft`; for `--html`, the shell + JS glue (`loft-gl-wasm.js` externs). Plus
whatever the project declares as `[[build.asset]]`.

## Design sketch (`loft.toml`)

```toml
[build]
default-targets = ["native", "html"]      # what `loft build` makes with no args

[build.target.html]                        # a runtime "shape" = triple × feature-set
triple   = "wasm32-unknown-unknown"
features = ["random"]                       # --no-default-features implied
shape    = "html"                           # names the isolated dir target/loft/html/
requires = { rust-targets = ["wasm32-unknown-unknown"], tools = ["wasm-opt"] }

[build.target.wasi]
triple   = "wasm32-wasip2"
features = ["random"]
shape    = "wasi"

[[build.asset]]                             # custom asset step
name    = "atlas"
run     = "scripts/pack_atlas.loft"         # or any command
inputs  = ["art/**/*.png"]                   # fingerprinted -> rebuild only on change
outputs = ["assets/atlas.bin"]
targets = ["html"]                           # only needed for the html build

[[build.asset]]                             # an asset fed by an EXTERNAL source
name     = "dataset"
run      = "scripts/fetch_dataset.loft"     # pulls data that refreshes upstream
outputs  = ["assets/dataset.pak"]
lifetime = "30d"                            # freshness TTL — month-scale is the typical
                                            # case (a monthly-refreshed dataset): rebuild
                                            # when the output is older than 30d EVEN IF no
                                            # local input changed — no instrumentation of
                                            # the external source

[[test]]                                    # testing phase — runs after build + assets
name    = "smoke"
run     = "tests/smoke.loft"
targets = ["native", "html"]                 # run the SAME suite through each built target
needs   = ["atlas"]                          # gate on the asset being built first

[[test]]
name    = "atlas-integrity"
run     = "tests/check_atlas.loft"
inputs  = ["assets/atlas.bin"]               # a test OVER a generated data file
```

Every field maps cleanly to a form control (Slice 5).

## Sub-arcs

| Item | Ships | Status |
|---|---|---|
| **Slice 1** — target-dir isolation `target/loft/<shape>/` + auto-build the **loft-runtime** wasm rlib via the existing fingerprint gate | kills the stomp + flaky WASM, retires the manual `make` | **SHIPPED** — `native_utils::ensure_loft_runtime_rlib` (`--html` → isolated `target/loft/html/`; `--native-wasm` auto-builds wasip2), keyed on `loft_build_fingerprint` via the `.loft-build-fp` sidecar; `html_wasm` (16) + `wasm_debug_relay` (1) green |
| **Slice 2** — `[build]` manifest schema + `loft build` driver (targets + `requires` resolution, doctor-style missing-tool report) | declarative targets | **SHIPPED** — `[build]`/`[build.target.<name>]`/`.requires` in `src/manifest.rs`; `loft build [target...]` driver + built-in native/html/wasi + doctor-check in `src/build_phase.rs` (native→`--native --check`, html→`--html`, wasi→`--native-wasm`) |
| **Slice 3** — `[[build.asset]]` custom steps with input/output fingerprinting **+ a freshness `lifetime`** (TTL) for external-source-backed outputs (rebuild on age, no source instrumentation) | custom asset pipelines | **SHIPPED** — `[[build.asset]]` in `src/manifest.rs`; staleness (`missing`/`inputs-changed`/`TTL-expired`/`--force`) + glob + content-fingerprint + wall-clock stamp (`.loft/build/<name>.stamp`) + `.loft`/shell runner in `src/build_phase.rs` |
| **Slice 4** — `[test]` phase + `loft test` / `loft check`: run declared tests over each built target and over generated data files (`inputs`), gated on `needs` asset outputs; cache a green run by input fingerprint | declarative test phase | **SHIPPED** — `[[test]]` in `src/manifest.rs`; `loft check` build+test gate + interpret/native backends + `needs` gate + green-run cache (`.loft/test/<name>__<target>.stamp`, keyed on run-script+inputs+target) in `src/build_phase.rs` (html/wasi headless runner = future) |
| **Slice 5** — UI backing the config | authoring UI | Deferred (later) |

## Phase ordering

1. **Slice 1 first.** Isolation is a pure win independent of config: give each
   (triple × shape) its own output dir (`target/loft/<shape>/`) so `html`, `wasi`, the
   wasm-bindgen gallery variant, and native never share a path. Then extend the
   `auto_build_native_target` fingerprint gate to the loft-runtime rlib so `--html` /
   `--native-wasm` build it on stale/missing instead of erroring toward `make`. This
   alone removes the flaky failures and the manual step — land + validate before
   committing to a schema.
2. **Slice 2** builds the config reader on the now-proven isolation + fingerprint model.
3. **Slice 3** generalizes the same fingerprint-driven step runner to arbitrary asset
   scripts.
4. **Slice 4** adds the test phase on top: it *depends* on Slices 2–3 (a test runs over a
   built target and/or an asset output), so it comes after them. `loft test` runs the
   declared suites over each `targets` entry — mirroring how loft's own harness runs a
   script on `--interpret` **and** `--native` — and over `inputs` data files; `loft check`
   = build + test as one gate (the CI / UI entry point).
5. **Slice 5** is a UI over the settled schema.

## Open design questions

1. **Isolation depth.** Is a per-shape *output dir* (`target/loft/<shape>/`) enough, or
   does the host-side `loft-ffi` StableCrateId collision (p171/p310) need a per-shape
   *`CARGO_TARGET_DIR`* (like `target/install-lib`)? Measure: reproduce the collision,
   try output-dir-only first, escalate to full target-dir isolation only if it persists.
2. **Config vs. convention.** ✅ RESOLVED (Slice 2) — the built-in targets
   (`native`/`html`/`wasi`) are **implicit**: a zero-config project builds with no
   `[build]` section. A `[build.target.<name>]` entry overlays a built-in (per-field
   replace) or declares a new named target; `default-targets` picks the no-arg set
   (falls back to `["native"]`). See `build_phase::resolve_target`.
3. **Asset step sandboxing.** `[[build.asset]].run` executes arbitrary commands — does it
   run under the existing loft sandbox/capability model (@PLN86), or is a build script
   trusted-by-declaration? Lean trusted (it's the project's own manifest) but flag in the
   UI.
4. **Fingerprint granularity for assets.** ✅ RESOLVED (Slice 3) — CONTENT hash (not
   mtime), reusing `cache::file_hash` (sha256): `build_phase::inputs_fingerprint` folds
   each glob-matched file's path + content hash. Survives mtime resets; new/removed files
   change the fingerprint too. Asset sandboxing (question 3) stays trusted-by-declaration.
5. **Test-run caching.** ✅ RESOLVED (Slice 4) — yes, cached. A green run is stamped by
   the `(run-script content + inputs content + target)` fingerprint in
   `.loft/test/<name>__<target>.stamp` (`build_phase::test_fingerprint`); an unchanged
   test is skipped, making `loft check` incremental. The key includes the TARGET, so a
   green `native` run does not vouch for `interpret`. `--force` bypasses it.
6. **Relation to loft's own harness.** ✅ RESOLVED (Slice 4) — kept separate: the `[[test]]`
   phase is the project-facing surface (`build_phase::run_test_phase`), reusing a
   "run on each backend" runner over `interpret` + `native` — the same two backends loft's
   in-repo `tests/scripts/*.loft` harness runs on. `html` / `wasi` headless test execution
   has no runner yet (deferred; needs a node/wasmtime/browser host).
7. **Freshness `lifetime` (TTL) mechanism.** ✅ RESOLVED (Slice 3) — staleness =
   `missing OR inputs-fingerprint-changed OR (lifetime set AND age > lifetime) OR --force`
   (`build_phase::compute_staleness`, unit-tested for precedence). Rather than overload the
   rlib's `.loft-build-fp` sidecar, assets get a dedicated `.loft/build/<name>.stamp`
   holding `<inputs-fingerprint>\n<unix-secs>`; `age = now - stamp_time` reads a **wall-clock
   absolute instant** (`SystemTime::now`, cheap on every `loft build` — no timer). Units span
   `s`/`m`/`h`/`d`/`w`/`mo`(=30d)/`y` (`parse_duration`). (a) wall-clock ✓; (b) `--force` /
   `--fresh` overrides ✓; (c) content-vs-time re-stamp for skipping downstream tests →
   deferred to Slice 4 (test-run caching), where the test phase lands.

## See also

- Reference docs this implements/extends: [WASM.md](../../WASM.md), [HTML_EXPORT.md](../../HTML_EXPORT.md), [NATIVE.md](../../NATIVE.md), [PACKAGES.md](../../PACKAGES.md), [TESTING.md](../../TESTING.md).
- Mechanisms: `src/cache.rs`, `src/native_utils.rs`, `src/manifest.rs`, `Makefile` (`install-artifacts`, `rebuild-native-cdylibs`), `scripts/doctor.sh`.
- Cooperates with @PLN86 (capabilities — asset-step sandboxing, question 3).
- Tracker: [@PLN100](https://github.com/loft-lang/plans/issues/100).
