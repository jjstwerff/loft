<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 100 — Declarative project build & test phase

> **Live status · lifecycle:** [loft-lang/plans ▸ @PLN100](https://github.com/loft-lang/plans/issues/100) ← single source of truth for lifecycle state (label). This README is the source of truth for **per-phase status + design**.

## Status

**`status:active` — filed 2026-07-08. Slice 1 SHIPPED (2026-07-08); Slices 2–5 open.** A loft
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
| **Slice 2** — `[build]` manifest schema + `loft build` driver (targets + `requires` resolution, doctor-style missing-tool report) | declarative targets | Open |
| **Slice 3** — `[[build.asset]]` custom steps with input/output fingerprinting **+ a freshness `lifetime`** (TTL) for external-source-backed outputs (rebuild on age, no source instrumentation) | custom asset pipelines | Open |
| **Slice 4** — `[test]` phase + `loft test` / `loft check`: run declared tests over each built target and over generated data files (`inputs`), gated on `needs` asset outputs; cache a green run by input fingerprint | declarative test phase | Open |
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
2. **Config vs. convention.** Do the built-in targets (`native`/`html`/`wasi`) need to
   appear in `loft.toml` at all, or are they implicit and only *overridden* there? Lean
   implicit — zero-config projects still build.
3. **Asset step sandboxing.** `[[build.asset]].run` executes arbitrary commands — does it
   run under the existing loft sandbox/capability model (@PLN86), or is a build script
   trusted-by-declaration? Lean trusted (it's the project's own manifest) but flag in the
   UI.
4. **Fingerprint granularity for assets.** Content hash vs. mtime for `inputs` globs —
   reuse `src/cache.rs` hashing for consistency with the runtime rlib gate.
5. **Test-run caching.** Should a test whose `run` script + `inputs` data files + target
   fingerprint are unchanged since its last green run be **skipped** (cache the pass), the
   way the artifact gates skip an up-to-date build? Lean yes — it makes `loft check`
   incremental — but the cache must key on the target-shape too (a green `native` run does
   not vouch for `html`).
6. **Relation to loft's own harness.** The `[test]` phase is the *project-facing* surface;
   loft's in-repo `tests/scripts/*.loft` (auto-run on both backends by `tests/wrap.rs` /
   `tests/native.rs`) is the *compiler's* harness. Keep them separate but let the project
   phase reuse the same "run on each target backend" runner so behavior matches.
7. **Freshness `lifetime` (TTL) mechanism.** The staleness predicate for an asset becomes
   `missing OR inputs-fingerprint-changed OR (lifetime set AND age > lifetime)`, where
   `age` reads a **build timestamp** written to the output's fingerprint sidecar (extend
   `write_native_artifact_fingerprint` to stamp a time alongside the hash) — so a
   TTL-expired external-source asset rebuilds with **no instrumentation of the external
   source**. The typical `lifetime` is **month-scale** (a monthly-refreshed dataset), not hours, so
   the duration units must span days/weeks/months (`30d`, `4w`, `1mo`), and a build that
   sits idle for weeks between runs is normal — the TTL check must be cheap on every
   `loft build`, not a background timer. Open sub-questions: (a) time source is wall-clock
   (never calendar-relative *effort* language — store an absolute instant); (b) `loft build
   --force` / `--fresh` overrides TTL for a deterministic clean build (and CI can pin it);
   (c) does a rebuilt output whose
   *content* is byte-identical to the prior one still re-stamp the time only (cheap) and
   skip re-running downstream tests (their `inputs` fingerprint is unchanged)? Lean yes —
   TTL controls *re-fetch*, content-hash still controls *downstream invalidation*.

## See also

- Reference docs this implements/extends: [WASM.md](../../WASM.md), [HTML_EXPORT.md](../../HTML_EXPORT.md), [NATIVE.md](../../NATIVE.md), [PACKAGES.md](../../PACKAGES.md), [TESTING.md](../../TESTING.md).
- Mechanisms: `src/cache.rs`, `src/native_utils.rs`, `src/manifest.rs`, `Makefile` (`install-artifacts`, `rebuild-native-cdylibs`), `scripts/doctor.sh`.
- Cooperates with @PLN86 (capabilities — asset-step sandboxing, question 3).
- Tracker: [@PLN100](https://github.com/loft-lang/plans/issues/100).
