<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# lib-plan 29 — library-owned wasm bridges (`--html` extension model)

**Status:** OPEN (W1 in-progress 2026-05-29 — extract `lib/imaging`'s
wasm bridge as the proof-of-concept; W2–W4 follow once W1 lands).

Driven by [@P321(c)'s browser-WASM dimension landing on `p377-fix`
(2026-05-29)](../../../PROBLEMS.md): the bridge that makes
`lib/imaging::load_png` work in `--html` bundles introduced
library-specific knowledge in **four** compiler-crate locations.
That conflicts with [`lib_plans/12-library-extraction`'s "drain
library code from compiler"](../../12-library-extraction/README.md)
direction and broke the
[`tests/extraction_hygiene.rs::forbidden_library_symbols_absent_from_src`](../../../../../tests/extraction_hygiene.rs)
gate (now patched via a `WASM_BRIDGE_ALLOWLIST` as the bridge for
this plan).

## Why

Plan 25 (`25-ffi-dispatch`) generalised the INTERPRETER-side FFI so
new native libraries register their own bridges (`#[loft_native]` +
`loft_register_bridges!`).  Plan 12 extracts library Rust code into
external repos.  But the **browser-WASM path** still needs four kinds
of per-library glue, and right now ALL of them live in the compiler /
tooling crate:

| Today's location | Per-library content |
|---|---|
| `src/wasm_imaging.rs` | Rust `pub fn` bridges + extern import declarations + dimension/copy ABI for `lib/imaging` |
| `src/generation/mod.rs::WASM_BRIDGE_FNS` | hard-coded `n_<sym> → loft::wasm_imaging::*` routing table |
| `doc/loft-gl-wasm.js` | JS host imports (`imaging_query`, `imaging_copy_rgb`, `imaging_save`, `host_asset_exists`) + `decodeLoftAssets` preload |
| `tools/wasm_repro.mjs` | matching JS imports for the test harness + a `node:zlib` PNG decoder |

A second wasm-using library (e.g. an audio library calling Web Audio,
a webgl-extras library calling Canvas2D) would need its author to
patch all four — each in a different repo if `lib/imaging` ever
extracts.  That's the same shape plan 25 fixed for the interpreter,
applied to the wasm path.

## What we want

Each library carries its OWN wasm extension.  Sketch:

```
lib/imaging/
  wasm/                                  ← NEW per-library subtree
    src/lib.rs                           ← pub fn bridges (today: src/wasm_imaging.rs)
    host.js                              ← loft_gl import bodies + preload helpers
    Cargo.toml                           ← own crate name (loft-imaging-wasm)
  loft.toml
    [wasm.bridge]                        ← NEW manifest section
    crate = "loft-imaging-wasm"
    host_js = "wasm/host.js"
    routes = ["n_load_png:imaging_load_png", "n_save_png:imaging_save_png"]
```

Compiler responsibilities (the rewire):

- Read every dep's `[wasm.bridge]` at codegen time → build the routing
  table dynamically (replaces `WASM_BRIDGE_FNS`).
- For `--html`: link each dep's wasm-bridge crate into the standalone
  binary (replaces the hard-coded `loft::wasm_imaging::*` references).
- For `--html`: concatenate each dep's `host_js` file into the HTML
  preamble (replaces the bundled `doc/loft-gl-wasm.js` for library
  content; the file stays for stdlib `loft_gl_*` + asset preload).
- `tools/wasm_repro.mjs` becomes generic — discover the same `host.js`
  files at test time and `import()` them.

End result: the compiler / tooling crate has zero `lib/<X>`-specific
naming.  The `WASM_BRIDGE_ALLOWLIST` in `extraction_hygiene.rs`
deletes; the hygiene gate's bare-name grep covers wasm bridges too.

## What we have already

| Piece | Where | Use |
|---|---|---|
| `lib/imaging` wasm bridge (in-tree, M-effort to extract) | `src/wasm_imaging.rs`, `src/generation/mod.rs::WASM_BRIDGE_FNS`, `doc/loft-gl-wasm.js::{imaging_*, host_asset_exists, decodeLoftAssets}`, `tools/wasm_repro.mjs` | the prototype that this plan generalises |
| `vector::alloc_vector_from_bytes` (the public helper the bridge uses) | `src/vector.rs:80` | stays in compiler crate (stdlib surface, not library-specific) |
| `Store::buffer` / `set_long` / `set_str` / `set_u32_raw` | `src/store.rs` | stable host-FFI surface bridges call (no plan-29 changes here) |
| `host_call_raw` (wasm-bindgen flavour) | `src/wasm.rs:39` | unused by `--html` (only the `feature="wasm"` path); kept for the interpreter-wasm playground |
| `[native.functions]` / source-scan pattern | `lib/*/loft.toml` + `loft-ffi-build` (plan 25 + plan 12) | analogue we're matching for the wasm side |

## Phases

| Phase | Subject | Status |
|---|---|---|
| W1 | Extract `lib/imaging`'s wasm bridge — Rust crate at `lib/imaging/wasm/`, manifest reader for `[wasm.bridge].crate` only, codegen routing reads from manifest, `src/wasm_imaging.rs` deleted | OPEN (in-progress 2026-05-29) |
| W2 | Extract the JS half — `lib/imaging/wasm/host.js` + `host_js` manifest key + `--html` HTML-preamble concatenation step; `doc/loft-gl-wasm.js` loses imaging-specific blocks | OPEN |
| W3 | Test-harness generalisation — `tools/wasm_repro.mjs` discovers `host.js` files at runtime; `tests/html_wasm.rs` passes the per-test dir; delete `WASM_BRIDGE_ALLOWLIST` in `extraction_hygiene.rs` | OPEN |
| W4 | Documentation + second-consumer audit — PACKAGES.md gets a `## Wasm bridges` section; PROBLEMS.md @P321(c) cross-link updated; sweep `lib/*` for any other library that would benefit (audio? graphics text rasteriser? net?) | OPEN |

W1's W1-only acceptance: imaging's wasm tests pass with `src/wasm_imaging.rs` deleted and the routing driven by `lib/imaging/loft.toml::[wasm.bridge]`.  W1 sketches the manifest shape that W2 + W3 extend.

## Reconciling with plan 12 + plan 25

- **Plan 12** drains library `n_*` symbols from the compiler crate.  Plan
  29 does the analogous drain for the wasm-bridge surface.  Same
  spirit ("library code lives with the library"), different surface.
- **Plan 25** generalised the INTERPRETER FFI dispatch via
  `loft-ffi-build` + `loft_register_bridges!`.  Plan 29 is the WASM
  analogue: a per-library wasm-extension crate registered through the
  package manifest.
- Order: plan 29 can move independently of plan 12's chunk-extraction
  cadence — even with `lib/imaging` in the monorepo, W1's manifest +
  per-library crate layout is the right shape.

## Triggers to demote / drop

- `lib/imaging` is the only candidate today; if a second wasm-bridge
  consumer doesn't materialise within a release cycle, W4 may collapse
  into "document the pattern, leave imaging's bridge in src/ until
  there's >1 consumer" and the rest defers.
