<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 21 — Prebuilt native libraries (no Rust toolchain to *use* a loft library) · `@PLN21` ([loft-lang/plans#21](https://github.com/loft-lang/plans/issues/21))

## Status

**PLANNED (2026-06-13).** The enabling invariant was *proven* this session by the
cdylib re-key work ([CHANGELOG_TECHNICAL] / `cache::loft_ffi_fingerprint`): a registry
cdylib is **loft-ffi-versioned, not loft-versioned, and rustc-independent**. That makes
shipping a prebuilt binary *sound* — this plan turns the proof into distribution.

Tracked as [`@PLN21`](https://github.com/loft-lang/plans/issues/21) (loft-lang/plans) —
the cross-ecosystem id (it touches the registry every library publishes through).

## Goal — invariant

> **Using a native loft library must not require a Rust toolchain.** `use graphics`
> on a fresh machine resolves a **prebuilt cdylib** for the host platform and `dlopen`s
> it — no rustc, no cargo, no `~90s` compile. Building from source is the *fallback*,
> not the default.

Today the opposite holds: the first `use graphics` triggers `auto_build_native` →
`cargo build` of the package's native crate (rustc + cargo + the native dev headers
like `libasound2-dev`/GL), costing ~90s and a full toolchain. That is the barrier this
plan removes.

## Why this is sound now (the evidence, proven 2026-06-13)

The hard question for shipping someone-else's binary is *"is it compatible with my
loft?"* This session answered it, end to end:

- **A cdylib links loft-ffi (the C ABI), never `libloft.rlib`.** Verified by inspection:
  `nm -D` on `libloft_random.so` and `libloft_graphics_native.so` shows **zero loft
  undefined symbols**; their only NEEDED libs are system ones (`libc`, `libGL`,
  `libasound`, …). All loft callbacks arrive at runtime via the loft-ffi `#[repr(C)]`
  function-pointer table when loft `dlopen`s the lib.
- **So compatibility is keyed on loft-ffi, not loft.** Landed as
  `cache::loft_ffi_fingerprint` (a build-time hash of `loft-ffi/src`, stamped by
  `build.rs` into `LOFT_FFI_FINGERPRINT`). A cdylib built against loft-ffi vX is valid
  for *any* loft using loft-ffi vX — matrix-verified (libloft change → cdylib HIT;
  loft-ffi change → cdylib rebuilt).
- **rustc version is a non-issue for the cdylib.** `extensions.rs:2420` already decided
  this deliberately ("NO rustc-version guard here … ANY rustc builds it correctly"). A
  cdylib is `dlopen`d over C, statically embeds its own std, and exposes only
  `extern "C"`/`repr(C)`. A prebuilt built with rustc 1.89 loads into a loft built with
  rustc 1.95 unchanged. **Do NOT put rustc in the validity key** — it would manufacture
  staleness that doesn't physically exist and re-create the per-release churn we just
  removed. The rustc guard belongs only to the rlib-linking paths (`cache.rs:485`,
  `native_lib.rs:806`, `main.rs:5360`).

The one consequence to honour: the distributed artifact is the **cdylib** (`.so`/`.dll`/
`.dylib`), **not the rlib** (the PACKAGES.md format sketch showing `libgraphics.rlib` is
a trap — an rlib *is* SVH-locked to its rustc, E0514).

## What already exists (don't rebuild)

- **The package format defines `prebuilt/<target-triple>/`** (PACKAGES.md:82, *"avoids
  requiring Rust toolchain on consumer machine"*).
- **The resolution flow specifies it** (PACKAGES.md:910: *"Check prebuilt/ for current
  target; if missing/stale → cargo build"*).
- **The WASM path already does the per-target prebuilt check**
  (`native_utils.rs:653`, `prebuilt/wasm32-wasip2/`).
- **A native prebuilt-load hook exists** — `resolve_native_lib` (`extensions.rs:2272`)
  returns a prebuilt `.so` before falling to `auto_build_native`, but only from one
  platform-agnostic `native/<lib>` path, not the per-triple `prebuilt/<host-triple>/` dir.
- **The registry index already reserves the per-platform slot** —
  `registry_index.rs:79-87`: `Version.binaries: BTreeMap<String, BinaryEntry{url, sha256}>`,
  keyed by target triple (resolver deferred to *this* plan; PKG_REGISTRY.md:235 — *"when
  present, consumer skips local cargo build"*). Phase 4/5 wire it, not invent it.
- **dlopen failure is already non-fatal** — `extensions.rs:111-115`:
  `libloading::Library::new` failure is an `eprintln!` + silent continue (the load is
  optional). That is exactly the hook to enrich with classified diagnostics (Phase 3).
- **A host-triple helper exists** — `cache::target_triple()` (`cache.rs:212`) builds
  `<arch>-<os>-<family>` from `std::env::consts` at runtime (private; needs the libc
  refinement in Phase 1's C1).
- **The native fingerprint sidecar exists** — `auto_build_native` stamps
  `<dir>/release/.loft-build-fp` with `loft_ffi_fingerprint()` and validates via
  `native_artifact_fingerprint_matches` (this session's work). A prebuilt reuses the same
  sidecar mechanism, unchanged.

## The design — tiered native resolution

```
resolve_native_lib(pkg, stem):
  1. prebuilt/<host-triple>/<lib>  with a matching loft-ffi fp   → dlopen
  2.   dlopen fails → validate WHY (the dynamic linker IS the validator):
         missing RUNTIME system lib  → actionable error; building won't help
         loft-ffi / glibc mismatch   → fall to (3)
  3. source-build (auto_build_native + loft_ffi_fingerprint gate) → dlopen
  4.   no toolchain / missing build dev-libs → clear, actionable error
```

### Requirement A — system-wide library validation

- **The dynamic linker is the authoritative validator.** loft already `dlopen`s; on
  failure `dlopen`/`LoadLibrary` reports *exactly* what is missing
  (`libasound.so.2: cannot open shared object file`, `version 'GLIBC_2.38' not found`,
  `undefined symbol`). Validation = *attempt the load, catch the failure, respond* — not
  a hand-rolled pre-flight that can drift from reality.
- **The load-bearing distinction: a missing RUNTIME lib cannot be fixed by building.**
  A source-built cdylib links the *same* runtime lib, so it fails to load identically.
  So the tier model is not "prebuilt else build" — there is a real third outcome:
  *needs a system lib, and only the user installing it resolves it.*
- **Packages declare their runtime libs** (manifest, below) so loft turns the raw linker
  error into *"graphics needs libGL — `apt install libgl1`"* (Goal F: never hand the
  programmer a cryptic failure). Split is clean: `glb`/`mesh3d`/`random`/`imaging` need
  only `libc` (fully self-contained); `graphics` adds `libGL`/`libasound` at runtime.

### Requirement B — reliable on-demand build (the fallback)

`auto_build_native` is the path; "reliable" has concrete teeth:

- **Fingerprint-gated** — `loft_ffi_fingerprint` (this session) already ensures it
  builds only when genuinely needed, not the old per-profile churn.
- **Toolchain/dev-lib diagnostics** — distinguish "no rustc" from "missing
  `libasound2-**dev**`" (the *build*-time header, vs the runtime lib above) and name the
  fix.
- **Determinism gap to close** — the repo gitignores `Cargo.lock`, so the on-demand
  build re-resolves deps each run (the `@P388` comment flags this). A reliable build
  wants a pinned/locked resolution so two machines produce the same cdylib.

### Manifest schema — two dep sets

`loft.toml [native]` declares **both**, and the registry surfaces both:

```toml
[native]
crate        = "loft-graphics"
runtime-libs = ["libGL.so.1", "libasound.so.2"]   # validation + prebuilt install hint
build-deps   = ["libgl-dev", "libasound2-dev"]    # rustc + dev headers, for the fallback
```

- `prebuilt/<target-triple>/` ships the **cdylib** (`.so`/`.dll`/`.dylib`), each stamped
  with the `loft-ffi` fingerprint it was built against (the existing `.loft-build-fp`
  sidecar mechanism).
- Registry view: *"graphics: runs with libGL; builds with libGL-dev + glutin."*

## Design decisions (probe before building)

- **C1 — the prebuilt triple form.** A cdylib's portability key is `(arch, os, libc-on-linux)`,
  not the loose `<arch>-<os>` the PACKAGES.md sketch shows: a glibc `.so` won't load on a
  musl host, and `cache::target_triple()` (`env::consts`) doesn't capture libc. **Decision:**
  stamp the *authoritative* host triple in `build.rs` from `env::var("TARGET")` (available to
  build scripts) → `LOFT_BUILD_TARGET` (e.g. `x86_64-unknown-linux-gnu`); name prebuilt dirs
  by it. **Backstop:** even a wrong pick is *safe* — Phase-3's trial `dlopen` catches a
  glibc/abi mismatch and falls to source build; triple precision is efficiency, not
  correctness. *Probe:* confirm a gnu cdylib `dlopen`-FAILS cleanly (not UB) on a musl host.
- **C2 — the fp travels WITH the binary, two homes.** Local/tarball prebuilt → a co-located
  `prebuilt/<triple>/.loft-build-fp` sidecar (reuses `native_artifact_fingerprint_matches`).
  Registry binary → a new `loft_ffi_fp` field on the index `BinaryEntry`, so `loft install`
  rejects an incompatible binary *before* download. *Probe:* an fp mismatch must skip the
  prebuilt, never load it.
- **C3 — a missing runtime lib is terminal, not a build trigger.** Proven by mechanism: a
  source build links the *same* runtime lib, so it fails identically. The validation must
  decide build-vs-error *at resolve time* (a trial `dlopen`), not after a wasted ~90s build.
  *Probe:* a prebuilt needing an absent lib emits the install hint and does NOT build.

## Phases (designed)

### Phase 1 — Per-triple prebuilt resolve (the load hook) · S — **SHIPPED 2026-06-13**
**Goal:** loft loads `prebuilt/<host-triple>/lib<stem>.<ext>` when present and fp-valid, before any build.
**Landed:** `build.rs` stamps `LOFT_BUILD_TARGET`; `cache::host_triple()` reads it;
`resolve_native_lib` checks `prebuilt/<triple>/` (fp-gated via the `.loft-build-fp` sidecar)
before the legacy `native/` path and `auto_build_native`; a hit emits a `prebuilt` timing
event. Probe ✓ both cells: installed prebuilt → loads with **no build**; corrupted sidecar
fp → skipped, falls through to source build. (The wasm-path helper-extraction was dropped —
a wasm *rlib* and a native *cdylib* validate differently, so sharing was net-negative.)
1. **Authoritative host triple** — stamp `LOFT_BUILD_TARGET` in `build.rs` (`env::var("TARGET")`); the building loft *is* the host, so this is exact (C1). Expose `cache::host_triple()` reading it (replaces the lossy runtime `target_triple()` for this use).
2. **Shared helper** — lift the wasm logic (`native_utils.rs:652`, `pkg_dir/prebuilt/<target>/<file>`) into `fn prebuilt_lib(pkg_dir, triple, stem) -> Option<PathBuf>`.
3. **Hook `resolve_native_lib`** (`extensions.rs:2272`) — BEFORE the existing `native/<filename>` check: `prebuilt_lib(pkg_dir, host_triple, stem)`; if found AND `native_artifact_fingerprint_matches(prebuilt_dir, loft_ffi_fingerprint())` → return it; on fp mismatch → skip (never load a wrong-ABI cdylib), fall through to `native/<filename>` → `auto_build_native`.
4. **Prebuilt sidecar** — a prebuilt dir is `prebuilt/<triple>/lib<stem>.so` + `prebuilt/<triple>/.loft-build-fp` (the loft-ffi fp it was built against); reuse the existing sidecar reader unchanged.

   **Probe:** drop a locally-built `librandom.so` + a matching `.loft-build-fp` into `~/.loft/registry/random-0.1.0/prebuilt/<triple>/`; a `use random` run LOADS it (no `cdylib miss` in the ledger, no `cargo build`). Corrupt the sidecar → it falls through and rebuilds.

### Phase 2 — Manifest schema: `runtime-libs` / `build-deps` · S — **SHIPPED 2026-06-13**
**Goal:** packages declare the two dep sets validation + diagnostics need.
**Landed:** `manifest.rs` gains `runtime_libs` / `build_deps: Vec<String>` parsed from
`[native] runtime-libs` / `build-deps` (comma-list via `split_list`); unit test
`parses_native_runtime_and_build_deps` ✓; PACKAGES.md `prebuilt/` sketch corrected to the
cdylib + `.loft-build-fp` sidecar. Consumer wiring (store in `Data`, read in validation)
moves to Phase 3, where it's used.
1. **Struct** (`manifest.rs:36-46`) — add `runtime_libs: Vec<String>`, `build_deps: Vec<String>`; init `Vec::new()` in `default()` (`manifest.rs:79`).
2. **Parse** (`manifest.rs:90-125`, before the `_ => {}` catch-all) — a flat comma-list is least-friction for the hand-rolled scanner: `("native","runtime-libs") => manifest.runtime_libs = value.split(',').map(trim).collect()`, same for `build-deps`. Example `runtime-libs = "libGL.so.1, libasound.so.2"`.
3. **Consume** (`parser/mod.rs::apply_manifest_side_effects`, ~5907) — store both on the native-package record in `Data` (keyed by stem) so Phase 3 reads `runtime_libs` for the hint.
4. **Doc fix** — PACKAGES.md:82 `prebuilt/.../libgraphics.rlib` → `lib<stem>.so` (cdylib); document the two fields + the per-triple sidecar.

   **Probe:** unit test on `read_manifest` — `[native] runtime-libs = "a.so, b.so"` → `vec!["a.so","b.so"]`.

### Phase 3 — Validation + diagnostics (the dlopen-failure path) · M — **SHIPPED 2026-06-13**
**Goal:** a load failure becomes an actionable message, and build-vs-error is decided correctly (C3).
**Landed (all three):**
- **Diagnostics** — `extensions::dlopen_diagnostic` classifies the load-site error: missing
  system lib (names it + "install it; building won't help"), glibc-too-old,
  undefined-symbol/ABI — replacing the raw `eprintln`.
- **Proactive C3 check** — `first_missing_runtime_lib` reads the manifest's `runtime-libs`
  and `dlopen`s each at the TOP of `resolve_native_lib`; a missing one emits the install
  hint and returns `None` BEFORE loading a doomed prebuilt or spending ~90s on a build that
  can't load. (Read the manifest directly from `pkg_dir/loft.toml` — no `Data` wiring
  needed.) Empty `runtime-libs` → no check, no cost.
- **Build-deps diagnostics** — `auto_build_native`'s failure arm splits into "cargo ran but
  failed" (names the declared `build-deps`) vs "cargo couldn't start" (no toolchain).

Unit-tested (`dlopen_diag_tests`, **5 cells**: classifier ×3, missing-runtime-lib detect,
build-deps hint); `n0_fingerprint` regression ✓; `fmt`/`clippy` clean. The trial-`dlopen`
of an *un*declared-dep prebuilt is covered by the existing load-site diagnostic, so the
declared-libs proactive check + the load-site classifier together close C3.
1. **Trial-load in resolve** — when Phase 1 finds an fp-valid prebuilt, `libloading::Library::new` it *there* (cheap). Classify `Err`:
   - missing-lib (`cannot open shared object file`) → **terminal**: extract the lib name, cross-ref the manifest `runtime-libs`, emit *"`<pkg>` needs `<lib>` at runtime — install it"*, return `None`-with-diagnostic, **do not build** (C3).
   - `version 'GLIBC_x' not found` → message + fall to source build (a local build targets the host glibc).
   - `undefined symbol` / other → defensive: skip prebuilt, fall to `auto_build_native`.
2. **Enrich the load site** (`extensions.rs:111-115`) for the non-prebuilt path too — same classifier on the final `eprintln!`.
3. **Build-path diagnostics** — in `auto_build_native`, on a failed `cargo build`: rustc-not-found → "install Rust"; `pkg-config`/linker dev-lib error → cross-ref `build-deps` → "install `<dev-lib>`".

   **Probe:** a prebuilt whose `runtime-libs` names an absent lib → loft emits the hint and spends **no** time building (timed).

### Phase 4 — CI build matrix + publish (produce the binaries) · M — **PRODUCER PRIMITIVE + WORKFLOW SCAFFOLD 2026-06-13**
**Goal:** each native library ships per-triple, fp-stamped cdylibs in the registry.
**Landed (producer):** `loft build-native [pkg-dir]` (`main.rs`) compiles a package's native
crate via `auto_build_native` — running **no program** (a graphics lib needs no display) —
and prints the cdylib path, stem, host triple, and `loft_ffi_fp` (machine-readable for a
publish script). Verified locally on `random` (builds `libloft_random.so`, reports the fp).
`.github/workflows/prebuild-native.yml` (manual-`workflow_dispatch`, non-disturbing) mirrors
`release.yml`'s matrix, checks out a library repo, runs `build-native` per target, and
uploads each cdylib + emits its `binaries[<triple>]` index-entry JSON to the job summary.
**Remaining (the two untestable-locally refinements, noted in the workflow):** (a) a real
end-to-end CI run; (b) linux on an **old-glibc/manylinux** base for portability; (c) the
**publish glue** — a final job / `registry_maintain.sh` turning the artifacts into release
assets and writing the `binaries` entries into the package's `index.json`.
**Landed (install-side — the testable Rust half):** `BinaryEntry` gains `loft_ffi_fp:
Option<u64>` (parsed from the index, stored as a string for u64 precision);
`install::fetch_prebuilt` runs after extraction — when `Version.binaries[host_triple]`
exists AND its `loft_ffi_fp == loft_ffi_fingerprint()`, it downloads the cdylib into
`prebuilt/<triple>/lib<stem>.<ext>` (stem from the package manifest's `[library] native`),
verifies the sha256, and writes the `.loft-build-fp` sidecar — exactly where Phase 1 looks.
Best-effort: any miss (no entry / fp mismatch / offline / download or hash failure)
silently leaves the source path. `fmt`/`clippy` clean; install + registry_index suites pass
(no regression). This closes the install→load loop with Phase 1.
**Remaining (producer-side — infra, needs a real registry + CI to validate):** the
per-triple CI build matrix (mirror `release.yml`; linux on an old-glibc/manylinux base;
`auto_build_native` emits the cdylib + fp), and the `registry_maintain.sh` publish step
that uploads each cdylib as a release asset and writes the `binaries[<triple>] = {url,
sha256, loft_ffi_fp}` index entries. Untestable locally — scoped, not yet built.
1. **Per-triple build workflow** — mirror `release.yml`'s matrix (`x86_64`/`aarch64` × linux/macos/windows). Linux in an **old-glibc (manylinux-style) container**; macOS sets an old `MACOSX_DEPLOYMENT_TARGET`. Each job builds the package's `native/` crate via a loft carrying the target loft-ffi → `auto_build_native` emits `lib<stem>.<ext>` + `.loft-build-fp` (fp is free).
2. **Publish** — extend `scripts/registry_maintain.sh`: upload each cdylib as a release asset; add `binaries["<triple>"] = {url, sha256}` to the version (slot exists, `registry_index.rs`) + a new `loft_ffi_fp` on `BinaryEntry` for pre-download validation.
3. **`loft install`** (`install.rs:87`) — when `Version.binaries[host_triple]` exists AND `loft_ffi_fp == loft_ffi_fingerprint()` → download the cdylib into `~/.loft/registry/<pkg>-<ver>/prebuilt/<triple>/` + write the `.loft-build-fp` sidecar (so Phase 1 finds it); else the source path.

   **Probe:** a test index entry with a `binaries` slot + matching fp → `loft install` downloads + sidecars it → a later `use` loads without building.

### Phase 5 — Registry surfacing · S — **SURFACING DONE 2026-06-13**
**Goal:** the runtime requirement + available prebuilts are visible.
**Landed (the `loft install` surfacing):** `InstallReport` gains a `surface: Vec<String>`;
`install_one` populates a per-package "Native:" block via `prebuilt_status(host, available,
installed)` — "prebuilt installed for `<triple>` — no Rust toolchain needed" / "no prebuilt
for `<triple>` (available: …) — builds from source (needs rustc)" / "a `<triple>` prebuilt
exists but was built against a different loft-ffi" — plus a "runtime libraries: …" line from
the manifest's `runtime-libs`. `format_report` renders it. `prebuilt_status` is pure and
unit-tested (`prebuilt_status_branches`, 4 cells); install suite passes; `fmt`/`clippy` clean.
**Remaining (the submit-CI gate — registry infra):** the REGISTRY_SUBMIT.md gate that
validates each `binaries` entry's `sha256` + `loft_ffi_fp` and requires `runtime-libs` to be
declared when a prebuilt's `objdump -p` NEEDED list has a non-`libc` entry — needs the
registry submit pipeline, like Phase 4's publish glue.

### Phase 6 — Build determinism (the fallback's reliability) · M
**Goal:** the source-build fallback is reproducible across machines.
1. **Pin the native crate's deps** — the `@P388` comment notes `--locked` is unavailable because Cargo.lock is gitignored + deps drift. Resolve: **commit `native/Cargo.lock` in the published tarball** (a registry package pins) and have `auto_build_native` pass `--locked` when a lock is present.
2. This makes the reproducible-build submit gate (REGISTRY_SUBMIT gate 3) deterministic for the native crate.

   **Probe:** build a package's cdylib from two checkouts → identical dep resolution; the reproducible gate passes.

## Order & critical path

`1` and `2` are independent and small — land first (prebuilts become *loadable* + *declared*).
`3` builds on `1`+`2` (trial-load + the `runtime-libs` hint). `4` produces what `1` loads
(depends on `1`; the index slot already exists). `5` depends on `2`+`4`. `6` is independent.
**Critical path to "no rustc to use graphics": `1 → 4`** (a load path + one published prebuilt);
`2`/`3` make the failure modes humane, `5`/`6` are polish.

## Risks / open questions

- **glibc baseline** — prebuilts must be built on an OS old enough that a newer host's
  forward-compatible glibc loads them (the manylinux problem). Decide the baseline per
  platform; this is a build-host choice, *independent of rustc*.
- **Panic across the FFI boundary** — loft-ffi must `catch_unwind`/abort at the edge
  (UB otherwise). A correctness invariant the prebuilt path inherits; audit loft-ffi.
- **The C-ABI discipline is the load-bearing invariant** — rustc-independence holds only
  while loft-ffi stays strictly `repr(C)`/`extern "C"` with no leaked Rust types. If that
  ever broke, rustc *would* matter. `loft_ffi_fingerprint` catches loft-ffi changes; an
  `api-lint`-style check that loft-ffi's public surface is C-ABI-only would guard the
  invariant directly.
- **Windows/macOS specifics** — `.dll` import semantics and macOS `@rpath`/codesigning
  for a downloaded `.dylib` (Gatekeeper) need their own validation in Phase 3/4.

## Connections

- **PACKAGES.md** — the `prebuilt/` format + resolution flow (the design home).
- **PKG_REGISTRY.md** — the publish/install pipeline (Phase 4/5 land here).
- **`cache::loft_ffi_fingerprint` / `extensions::resolve_native_lib`** — the loft-side
  load + validity gate (Phase 1/3).
- **GOALS.md Goal F** (friction-free) + the "loft inherits Rust's ecosystem via
  in-language bindings, minus the crash surface" thesis — this plan is what makes that
  ecosystem usable without a toolchain.
