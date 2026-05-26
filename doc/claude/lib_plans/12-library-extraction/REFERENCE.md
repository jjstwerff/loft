<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library extraction — durable reference

Slow-changing reference for the [library-extraction plan](README.md):
the library inventory, the stdlib-vs-library boundary, the chunk
topology + dependency graph, the per-chunk extraction template, the
CI path, and the open design questions.  The plan README holds
status + the forward path; this file holds the "how it works" that
doesn't change per phase.

---

## Current inventory — extraction candidates

`lib/*/` packages with `loft.toml` (using the package format, ready in
principle to extract once the registry exists):

| Library | Notes | Extraction priority |
|---|---|---|
| `arguments` | CLI argument parsing | **EXTRACTED** (loft-libs-core) |
| `crypto` | Cryptographic primitives | **EXTRACTED** (loft-libs-core) |
| `random` | RNG | **EXTRACTED** (loft-libs-core) |
| `shapes` | 2D shape drawing + collision | **EXTRACTED** (loft-libs-graphics 0.2.0) |
| `gridmesh` | Hex / grid mesh generator | **EXTRACTED** (loft-libs-graphics 0.1.0) |
| `web` | Web utilities | **EXTRACTED** (loft-libs-net 0.1.0) |
| `server` | HTTP server | **EXTRACTED** (loft-libs-net 0.1.0) |
| `game_protocol` | Multiplayer protocol | **EXTRACTED** (loft-libs-net 0.1.0) |
| `time` | Date/time over epoch-ms ([`../21-datetime/`](../21-datetime/)) | Early — pure-loft; `loft-libs-core` |
| `markdown` | Markdown parser / formatter | Early — pure-loft |
| `imaging` | Image manipulation | Mid — `loft-libs-graphics`; native ABI closed (@P321c), browser-WASM open |
| `graphics` | OpenGL / 2D drawing | Mid — `loft-libs-graphics`; coordinate with [`../02-graphics/`](../02-graphics/) |
| `world` | Shared world model (sparse Cell/Chunk/World; expands per Phase 7a) | Mid — TTT v5, audience demo, moros, dryopea |
| `moros_editor` / `moros_map` / `moros_render` / `moros_sim` / `moros_ui` | Moros game libraries (game-specific after the Phase-7a world split) | Late — into the `moros` project (Phase 7b) |
| `audience_crystal` | Audience-demo crystal mesh-gen prototype | **Stays in monorepo** (paired with the audience demo; Phase 8 adds package `tests/` so it joins the CI gates) |

Single-file `lib/*.loft` modules (`code`, `docs`, `lexer`, `logger`,
`parser`, `testlib`, plus `wall` / `overland`) are NOT package-format
adopters.  Destinations decided 2026-05-24: `wall` + `overland` fold
into `lib/world/` (Phase 7a); the rest stay as monorepo-internal
self-hosting / build / test tooling.  The self-hosting cluster
(`code`+`lexer`+`parser`+`docs`) could become an optional
`loft-libs-self` chunk if an external consumer of a programmatic loft
parser ever appears.

## Stdlib vs library — the boundary (settled 2026-05-23)

**stdlib STAYS in the loft compiler crate; libraries are extracted.**
The permanent boundary between the language and its ecosystem — not
subject to drain.

| stdlib (stays in `src/`) | library (extracts to `lib/<X>/native/`) |
|---|---|
| `n_panic`, `n_assert` — fault primitives | `n_sha256` / `n_hmac_sha256` / `n_base64_*` — crypto |
| `n_log_*` — logging | `n_http_*` / web bridges — web |
| `n_json_parse` / `n_json_*` / JsonValue — JSON (P54) | `n_rand*` — random |
| `n_parallel_*` — threading | `n_load_png` / `n_save_png` — imaging |
| `n_now`, `n_ticks` — clock | `n_sleep_ms` — sleep (web/time) |
| `n_path_sep`, `n_hash_sorted` — runtime utility | OpenGL `gl_*` — graphics |
| `n_stack_trace` — introspection | TCP / WebSocket / TLS — server / web |
| `n_get_store_lock` / `n_protect_store_frees` — concurrency runtime | `n_arguments` — CLI parsing |

Stdlib symbols are what every program could plausibly need; library
symbols are opt-in via `use <lib>`.  `tests/extraction_hygiene.rs`
locks the boundary: every drained `n_*` symbol (derived from each
`lib/*/loft.toml::[native.functions]`) must be absent from
`src/**/*.rs` (word-boundary match; `#[cfg(...wasm32...)]`-gated
host-bridge stubs exempted — WASM has no `dlopen`).  A future attempt
to re-add a library symbol to the compiler crate fails this gate.

### Readiness tiers (surveyed 2026-05-23; drains since complete)

- **Tier A — pure-loft (13):** `arguments`, `audience_crystal`,
  `game_protocol`, `gridmesh`, `markdown`, `moros_*`, `shapes`,
  `time`, `world`.  No `#native`; resolved via `probe_sibling_package`.
- **Tier B — clean native, lives in `lib/<X>/native/`:** `graphics`,
  `imaging`, `random`, `server`.  Native code never duplicated in the
  compiler crate.
- **Tier C — was leaking into `src/native.rs`, now drained:**
  `crypto` (6 syms) and `web` (13 of 19) — both moved to their own
  `native/` cdylibs (Phase 1).

## Chunk grouping — a few repos, not 17

One repo per library = 17 repos to track.  Libraries extract in
**chunks**: a small number of multi-package repos, each a family that
versions + releases together under one CI workflow.  **Four chunks is
the cap.**  New libraries join an existing chunk by family fit.

**Library chunks** (registry-published):

| Chunk repo | Packages | Rationale |
|---|---|---|
| `loft-libs-core` | `arguments`, `random`, `crypto`, `time` (+ future stdlib drains: `json` / `html` / `fs`) | Small, stable, no graphics deps — extract first.  `time`'s companion built-in `DateTime` is a language PRIMITIVE in the compiler crate, not library code. |
| `loft-libs-graphics` | `graphics`, `imaging`, `gridmesh`, `shapes` | Graphics stack + `#native` crates; coordinate with [`../02-graphics/`](../02-graphics/) |
| `loft-libs-net` | `server`, `web`, `game_protocol` | HTTP / multiplayer; coordinate with [`../08-server/`](../08-server/) |
| `loft-libs-world` | `world` (Phase-7a-expanded: hex addressing, wall geometry, groups, height; folds in `lib/wall.loft`) | Shared spatial primitives for TTT v5, audience demo, moros, dryopea ([@PLAN46](../../plans/future/46-dryopea/README.md)) |

**Game / application repos** (host game-specific libraries AND the
playable game; registry-publish optional):

- **`moros`** *(existing GitHub project — reuse)*: `moros_editor`,
  `moros_map` (game-only remnant after 7a), `moros_render`,
  `moros_sim`, `moros_ui` + the game executable.  Depends on
  `loft-libs-graphics` + `loft-libs-world`.  Per-library tarballs may
  still publish, but the homepage is the moros repo.
- **`dryopea`** *(new project — [@PLAN46](../../plans/future/46-dryopea/README.md))*:
  dryopea-specific libraries + game, with `loft-libs-world` from the
  registry.

A chunk extracts as a unit (one `git filter-repo` / `subtree split`
of its `lib/*` dirs), intra-chunk deps as path deps, cross-chunk deps
as registry deps.

### Cross-chunk dependency graph

Inter-chunk deps resolve via the registry; intra-chunk deps stay path
deps.  Re-verify at each chunk's Stage-A1 before extracting.

```
loft-libs-core   (no chunk deps — leaves of the graph)
  ↑
  ├── loft-libs-graphics   (arguments from core; shapes / gridmesh internal)
  │     ↑
  │     └── loft-libs-world   (shapes / gridmesh from graphics IF world
  │           ↑                 reuses 2D collision / mesh — verify in 7a)
  ├── loft-libs-net        (crypto / arguments from core)
  ├── moros                depends on graphics + world (+ maybe net)
  └── dryopea              depends on world
```

Phase ordering this imposes: core publishes first (everything depends
on it); graphics + net are independent and can interleave; world (6w)
needs core + Phase 7a complete; moros (7b) needs graphics + world.
**Open verification (Phase 1):** does post-7a `world` need `gridmesh`
from graphics?  Does `moros` need `net` for multiplayer demos?

## Store-allocating cdylib pattern (random's showcase)

Template for any `#native` fn that allocates in the loft store
(returns `Type::Vector`/`Type::Reference`, or takes a struct
`Reference` arg).  Landed with `loft::native_call` (`src/extensions.rs`)
+ `output_native_direct_call` (`src/generation/mod.rs`):

1. The cdylib declares the fn with `LoftStore` as its first
   `extern "C"` arg, returning `LoftRef` (or `bool` for in-place
   writes).
2. `loft.toml::[native.functions]` declares the mapping — no ABI hint;
   codegen infers from the loft-side signature.
3. Codegen emits `enter` + `build_store` + cdylib call +
   `from_loft_ref` for store-allocating returns, and marshals struct
   `Reference` args via `to_loft_ref` (the @P321c fix — `&ref` arg
   store-forwarding, formerly "task #67", is DONE).
4. Showcase property: the same loft program is byte-identical under
   `loft` and `loft --native` (a `rand_seed; rand_indices` ×2
   reproducibility check suffices).

The cdylib's `loft_register!` symbol list is generated from
`[native.functions]` by `native/build.rs` → the shared `loft-ffi-build`
crate; `include!`d at module scope.  Adding a native symbol is two
edits: the manifest row + the `extern "C"` body.  imaging migrated to
this pattern 2026-05-26; crypto/web were the originals.

## CI path for libraries (travels with each chunk)

Keys off `lib/<pkg>/tests/*.loft` + `loft.toml` — nothing
monorepo-specific, so it ports to each extracted chunk:

| Gate | Where | Covers |
|---|---|---|
| Interpreter | `tests/wrap.rs::library_suite` | every `lib/*/tests/*.loft` via `loft test`; skips `LIB_PKGS_SKIP` / `LIB_TESTS_SKIP` |
| Native | `tests/native.rs::native_library_suite` | same via `loft --native test`; skips `LIB_PKGS_NATIVE_SKIP` / `LIB_TESTS_NATIVE_SKIP` ([@P321](../../PROBLEMS.md)) |
| Leak | `tests/wrap.rs` `run_test` gate | unfreed stores at exit; allowlist `SCRIPTS_LEAK_ALLOW` ([@P322](../../PROBLEMS.md)) |
| WASM | `tests/html_wasm.rs::wasm_library_suite` | main()-bearing tests under Node (browser `feature="wasm"`) + wasmtime (`wasm32-wasip2`) when present; skips `LIB_PKGS_WASM_SKIP` (`server` platform-N/A, `imaging` [@P321c](../../PROBLEMS.md), `world` [@P334](../../PROBLEMS.md)) |
| Quick dev loop | `make test-packages` | interpreter-only shell loop (dev-only) |

When a chunk extracts, its repo CI runs the equivalent over the
chunk's packages and the skip-lists travel as the chunk's own
`chunk-skips.toml`.  Prebuilt `.wasm`/`.rlib` artifacts (PACKAGES.md
`prebuilt/<target>/`) are a PKG.REG follow-on, out of scope for the
first extraction round (chunks build from source at install time).

## Per-chunk extraction template

Each chunk extracts in **two stages with a gate between them**: the
external chunk is built, tested, and published on its own (Stage A)
BEFORE any monorepo linking (Stage B).  Within Stage B the swap is
ordered — **link, validate the link, remove, re-validate** — four
separate commits, each its own gate.  Never bundle "remove old + link
new + hope CI proves it."

### Multi-package repo conventions

A repo hosting multiple packages can't assume `loft package` runs at
the root:

1. **Per-package git tags** — `<package>-v<version>` (not bare
   `v<version>`), so sibling libraries shipping overlapping versions
   don't collide.  (Open Q #3, RESOLVED.)
2. **`subpath` field on registry version rows** — relative dir of the
   package inside the repo (default `""`).  `validate.py` honours it
   on the reproducible-build re-check (`cd <subpath>/; loft package`).
   Both land in [PKG_REGISTRY.md § Schema](../../PKG_REGISTRY.md) when
   used; `REGISTRY_SUBMIT.md` gains a "publishing from a chunked repo"
   note.

### Stage A — build the external chunk (no monorepo changes)

1. Verify every package has `loft.toml` + passes both gates in-tree;
   note its `*_NATIVE_SKIP` / leak-allowlist entries (they travel).
2. Create the external repo.
3. Drop in the CI workflow (copy `library-ci.yml` + `chunk-skips.toml`
   from `loft-libs-core`).
4. Push library content preserving history (`git filter-repo` /
   `subtree split`).
5. **Verify chunk CI green on its own** — the "finished and tested"
   gate; do not advance until true.
6. Tag `v0.1.0` per package.
7. Publish to the registry (`loft publish` per package).
8. Smoke-test from outside the monorepo: `loft install <pkg>` into a
   scratch dir + a minimal `use <pkg>;` consumer.

### Stage A → B gate

All true before any monorepo PR: chunk CI green standalone; all
packages published at v0.1.0; Stage-A8 smoke-test resolved each
package from the registry without monorepo paths.

### Stage B — switch the monorepo over (ONE PR, separate commits)

- **B1 Link.** Add `<X> = "0.1.0"` deps to every consumer's
  `loft.toml`; leave `lib/<X>/` in place.
- **B2 Validate the link.** Rename each `lib/<X>/` →
  `lib/_extracting_<X>/` (so `probe_sibling_package` misses it, forcing
  registry fall-through); `make ci` green = functionally equivalent.
  Commit the rename.
- **B3 Remove.** Delete the renamed dirs; remove the chunk's monorepo
  skip-list / leak-allowlist entries.
- **B4 Re-validate.** `make ci` again.  A failure = something depended
  on the in-tree copy → roll back B3, file a chunk-repo issue, do NOT
  patch in the monorepo.
- **B5 Document** in CHANGELOG.md; link the chunk repo if appropriate.
- **B6 Finisher** — add every owned `n_*` symbol to
  `tests/extraction_hygiene.rs::FORBIDDEN_LIBRARY_SYMBOLS` (+ any
  library-only dep to `FORBIDDEN_MAIN_CRATE_DEPS`) so CI locks the
  drain.

### Stage C — ongoing maintenance

Updates land in the external repo; consumers bump the version in
`loft.toml`.  The monorepo carries only the versioned dependency.

## Open questions

1. **Naming.** `loft-<X>` under loft-lang org, or `<X>` org-namespaced?
2. **Version policy.** Per-library independent semver (matches registry idiom).
3. **Tagging.** RESOLVED — `<package>-v<version>` for multi-package repos; bare `v<version>` only for single-package.
4. **Test infrastructure.** RESOLVED — the monorepo CI path (above) is the template; `library-ci.yml` + `chunk-skips.toml` owned by Phase 4 / 6.5.
5. **Back-compat window.** A leaving `lib/<X>/` should keep working via the registry copy for ≥1 release.
6. **Documentation home.** Per-library README migrates to the external repo; decide CLAUDE.md doc-index pointer.
7. **Transitive deps.** Stable libs extract first; the consumer's `loft.toml` updates to the external dep as part of the dependency's extraction commit.
8. **Cross-library breaking changes.** Who tracks an in-monorepo consumer's migration when an extracted library evolves?
9. **World chunk vs graphics chunk.** Separate `loft-libs-world` (cleaner, no OpenGL coupling, but a 5th repo) vs folding into `loft-libs-graphics`.  Resolve before Phase 6w.
10. **Coexisting cell models in `lib/world/`.** Keep sparse Cell/Chunk (TTT/audience) + hex-world (moros/dryopea) as separate types sharing addressing (default), or generalise (likely too invasive for 0.8.x).
11. **`logger.loft` destination.** RESOLVED to option (c) — stays as-is alongside the self-hosting parser (single consumer); revisit if a second consumer emerges.
12. **Cross-chunk dep verification.** Confirm during Phase 1 (see dependency graph): does post-7a `world` need `gridmesh`; does `moros` need `net`?
13. **WASM library CI gate.** IMPLEMENTED (`wasm_library_suite`, 2026-05-25); chunk-repo workflows must include a WASM job (or explicit `skip-wasm` rationale).
14. **Pre-built artifact distribution** (PACKAGES.md `prebuilt/<target>/`). Out of scope for the first round; PKG.REG follow-on (which targets, who runs the matrix build).

## See also

- [README.md](README.md) — the plan: status, next steps, phase summary.
- [PACKAGES.md](../../PACKAGES.md) — package format + registry infrastructure (the prerequisite).
- [PKG_REGISTRY.md](../../PKG_REGISTRY.md) — file-based registry MVP.
