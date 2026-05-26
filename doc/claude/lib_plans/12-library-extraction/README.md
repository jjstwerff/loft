<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library extraction — `lib/*/` → external repos

Move the `lib/*/` packages out of the main loft repository into
per-family external GitHub **chunks**, each consumable via the
package registry.

This is the **execution arc** for **PKG.EXTRACT** in
[ROADMAP.md](../../ROADMAP.md).  The infrastructure (registry MVP,
lock file, format) is the sibling
[PACKAGES.md § Open work](../../PACKAGES.md#open-work).  The durable
"how it works" reference — inventory, stdlib boundary, chunk
topology, dependency graph, per-chunk template, CI path, open
questions — lives in [REFERENCE.md](REFERENCE.md).  **This README is
status + the forward path only.**

## Status

**ACTIVE.**  Trigger (2026-05-23): @P321c showed the project kept
re-adding library code to the compiler crate; the plan reframed to
**drain what's there** and extract by chunk.

Shipped so far:

- **Decoupling done** (Phases 1–2): crypto + web drained from
  `src/native.rs`; `[native.functions]` manifest is the declarative
  source of truth, `loft-ffi-build` generates the `loft_register!`
  list, `tests/extraction_hygiene.rs` locks the boundary.
- **Registry code complete** (Phase 3, PKG_REGISTRY.md R1–R9):
  `loft package` / `install` / `search` / `info`, lockfile, signing.
  Ecosystem bootstrapped with interim `K_tmp` (YubiKey `K_real` swap
  is a later Scenario-C rotation; do not ship a public release with
  `K_tmp` embedded).
- **`loft-libs-core` SHIPPED** (Phase 4): `arguments`, `random`,
  `crypto` published + installable end-to-end; their `lib/` dirs are
  removed from the monorepo (leftover build/test cruft cleaned
  2026-05-26).
- **`loft-libs-net` SHIPPED** (Phase 6): `web`, `server`,
  `game_protocol` 0.1.0; transitive `loft install` + lockfile merge
  smoke-tested.
- **`loft-libs-graphics` partial** (Phase 5): `shapes` 0.2.0 +
  `gridmesh` 0.1.0 published.  `graphics` + `imaging` remain in the
  monorepo but their native-codegen blocker is **gone** (see below).

Recently corrected (this README was stale before 2026-05-26):

- **Phase 3.5b (path-dep resolution) is DONE**, not TODO —
  `manifest::extract_path_dep` is wired into the resolver
  ([`src/parser/mod.rs:4570`](../../../../src/parser/mod.rs)) with unit
  tests in [`src/manifest.rs`](../../../../src/manifest.rs).
- **The graphics/imaging blocker ("task #67", `&ref`-arg store
  forwarding) is RESOLVED** — @P321c's native fix emits
  `to_loft_ref(...)` at the call site
  ([`src/generation/mod.rs:2402`](../../../../src/generation/mod.rs));
  graphics + imaging both pass `native_library_suite` (not skipped).
  So Phase 5 is no longer codegen-blocked.
- **`imaging` migrated to the manifest pattern** (2026-05-26):
  `[native.functions]` + `build.rs` + generated `loft_register!`;
  redundant `#native` annotations + dead `generated.rs` removed.

## Goal

Every `lib/*/` package (except the monorepo-paired `audience_crystal`)
lives in an external chunk repo, consumable via `loft install`, with
the compiler crate carrying **zero library code** — verified by the
extraction-hygiene gate and identical behaviour across interp / native
/ WASM.

## Next steps — small verifiable increments

Ordered by readiness.  Each is independently verifiable.

1. ~~Migrate `server`/`graphics` to the `[native.functions]` manifest
   pattern.~~  **DONE + superseded** — the manifest was a transitional
   step; the libraries now use the **clean source-scan pattern**
   (bare `#native` in `.loft` → `loft-ffi-build::generate_register_from_loft`
   scans the sources → register list; no manifest).  See
   [REFERENCE.md § Clean libraries](REFERENCE.md#clean-libraries--the-principle).
   `server`/`imaging`/`web` are clean; `graphics` is the documented
   register exception (`CLEAN_REGISTER_EXCEPTIONS`); a parser error
   forbids redundant `#native "n_<fn>"`.  Enforced by the
   `native_libraries_follow_clean_binding_pattern` hygiene gate.
1r. **Phase 6r — re-clean the already-extracted external repos**
   (`loft-libs-core`: crypto/random; `loft-libs-net`: web/server).  They
   were extracted *before* the clean pattern landed, so they still carry
   the old manifest / hand-written `loft_register!` and stripped
   `#native` annotations.  Re-clean = **re-sync the now-clean monorepo
   `lib/<name>/` into each external repo** (clean `.loft` + source-scan
   `build.rs` + drop manifest) and bump their `loft-ffi-build` dep to
   `0.2`.  Two prerequisites: (a) `loft-ffi-build 0.2.0` on crates.io —
   **published** (the source-scan `generate_register_from_loft` ships in
   0.2); (b) **the bare-`#native` parser support must reach `main`** —
   the external CI clones `jjstwerff/loft` *main* to build loft, and bare
   `#native` only parses on the `libraries` branch (commits `909bc9f9` +
   `c1ec5a03`), so the interpreter step errors `Expect native symbol
   string` until `libraries → main` merges.  **Status:** both external
   PRs open + locally verified (`loft-libs-net#1`, `loft-libs-core#1`) —
   cdylibs build against published 0.2, symbol sets byte-identical to the
   old registers, and `scripts/verify_external_libs.sh` (below) greens
   all interpreter tests with a `libraries`-built loft.  Gated on the
   `libraries → main` merge, after which re-running each PR's CI greens
   the interpreter step.  *(The native step is a separate Phase-6.5 gap,
   not fixed by this merge — see below.)*

   **Pre-PR validation — `scripts/verify_external_libs.sh`:** mirrors each
   chunk's `library-ci.yml` (`loft --interpret test` + `loft --native
   test` per package) but builds loft from the **current working tree**
   instead of cloning `main`.  So a loft change the external repos depend
   on (a parser feature, a codegen fix) can be confirmed green *before*
   opening the unblocking `libraries → main` PR.  `--src net=/path`
   validates a local clone (a PR branch) instead of the published repo;
   `--interpret` / `--native` select one step.
2. **Phase 6.5 — green the chunk-repo CI** (`loft-libs-core/graphics/net`
   + registry `pr-validate.yml`): canonical `library-ci.yml` does
   `apt-get install mold` → clone+build loft from source → `loft test`
   + `loft --native test` per package; drop the speculative
   binary-download URL.  *Verify:* each repo's workflow green on push +
   PR; a deliberate one-package-break PR lands red naming the package.
3. **Phase 5 — extract `graphics` + `imaging` into
   `loft-libs-graphics`** (now unblocked).  Follow the
   [per-chunk template](REFERENCE.md#per-chunk-extraction-template).
   *Verify:* chunk CI green standalone + `loft install graphics imaging`
   into a scratch dir; monorepo `make ci` green after the Stage-B swap.
4. **Phase 3.6 — stdlib drain**: Image/Pixel/Format types →
   `lib/imaging/`; `escape_html` → new `lib/html/`; path helpers →
   `02_files.loft` (rename from `02_images.loft`).  *Verify:* `make ci`
   green; `default/*.loft` line count drops (~2,500 → ~2,000).
5. **Phase 7a — split moros into shared `lib/world/` + moros-specific**
   (monorepo-internal; hex addressing, `wall.loft`, `overland.loft`,
   geometry move in).  *Verify:* moros demos render identically;
   every `lib/moros_*/src/*.loft` `use world;` resolves.

After 7a + 6.5: Phase **6w** (extract `loft-libs-world`), then **7b**
(move moros libraries into the existing `moros` project), **7c**
(bootstrap `dryopea` — [@PLAN46](../../plans/future/46-dryopea/README.md)),
**8** (final monorepo cleanup + `audience_crystal` test dir).

## Why a separate plan from PACKAGES.md

PACKAGES.md § Open work = INFRASTRUCTURE (registry, lock file,
format) — one focused arc.  This plan = EXECUTION (which library
extracts when, consumer migration, version-sync) — spans multiple
releases, one chunk per release window.  Standing up each chunk repo
(org perms, branch protection, CI secrets, release tagging) is real
admin work; **chunk-extraction phases are serialised, not batched**,
with ≥1 minor release of soak between consecutive chunks.

## Phase summary

| Phase | Scope | Depends on | Status |
|---|---|---|---|
| 1 | Drain library symbols from `src/native.rs` | — | **DONE** 2026-05-24 |
| 2 | Compile-time native-registry aggregator (`[native.functions]` + `loft-ffi-build`) | 1 | **DONE** 2026-05-24 |
| 3 | PKG.REG + cdylib loader | PACKAGES.md | **code complete** 2026-05-24 (bootstrap via `K_tmp`) |
| 3.5a | Dry-run lib without monorepo consumers (crypto) | 1–2 | **DONE** 2026-05-24 |
| 3.5b | Real path-dep resolution (`extract_path_dep` wired) | 3.5a | **DONE** (`src/parser/mod.rs:4570`) |
| 3.5c | Dry-run libs with consumers | 3.5b | superseded — core/net extracted for real |
| 3.6 | Stdlib drain (Image→imaging, `escape_html`→`html`, path helpers→`02_files`) | 1c | **OPEN** — M (~1 day) |
| 4 | Extract `loft-libs-core` (arguments, random, crypto) | 1–3 + 3.5 | **SHIPPED** 2026-05-24 |
| 5 | Extract `loft-libs-graphics` (graphics, imaging, gridmesh, shapes) | 4 + [`../02-graphics/`](../02-graphics/) | **partial** — shapes/gridmesh shipped; graphics+imaging **now unblocked** |
| 6 | Extract `loft-libs-net` (server, web, game_protocol) | 4 + [`../08-server/`](../08-server/) | **SHIPPED** 2026-05-24 |
| 6.5 | Green CI across every chunk + registry repo (canonical `library-ci.yml`); subsumes parked tasks #61/#62/#63 | 4–6 | **OPEN** — S–M; lands before 6w |
| 6w | Extract `loft-libs-world` (world, Phase-7a-expanded) | 7a + 6.5 | OPEN — M |
| 7a | Split moros: shared spatial primitives → `lib/world/` (monorepo-internal) | 4 | OPEN — M |
| 7b | Move moros libraries into the existing `moros` project | 5 + 6w + 7a | OPEN — MH |
| 7c | Bootstrap `dryopea` project | [@PLAN46](../../plans/future/46-dryopea/README.md) | OPEN — S |
| 8 | Final monorepo cleanup + `audience_crystal` test dir | 7b | OPEN — S |

Phase 7a is monorepo-internal (no user-visible change) and can land
before the registry work; 6w then interleaves with the remaining
chunk extractions once 7a is stable.

## Open phase detail

Shipped phases' build records live in git history + CHANGELOG; the
detail below is for the OPEN phases only.

### Phase 6.5 — green CI across chunks (next infra step)

Phases 4/5/6 shipped working libraries but the per-chunk CI workflows
are red: they `cargo build` loft on an Ubuntu runner without
`apt-get install mold` (the loft repo forces `-fuse-ld=mold`), and the
registry `pr-validate.yml` curls a non-existent binary release.  Land
the canonical `library-ci.yml` (mold install + clone-and-build +
per-package `loft test` / `loft --native test`, Linux-only initially,
loft build cached on `Cargo.lock` SHA), fix `pr-validate.yml` to the
same clone+build pattern, roll into all three chunk repos + the
`library-template` repo, and document the baseline in
`LIBRARY_BLUEPRINT.md`.

### Phase 3.6 — stdlib drain

Shrink `default/*.loft` to genuine universal stdlib.  Moves: Image /
Pixel / Format types → `lib/imaging/src/`; `escape_html` → new
`lib/html/`; path helpers (`dir`/`basename`/`join`/`resolve`/
`path_sep`) → `02_files.loft`; rename `02_images.loft` → `02_files.loft`.
JSON STAYS (the `{x:j}` format specifier + `text as Foo` cast are
shipped language behaviour — pulling JSON out breaks both).  Audit
call sites for new `use imaging;` / `use html;` lines.

### Phase 7a — moros world split (monorepo-internal)

Move the non-moros-specific spatial primitives into `lib/world/`: hex /
chunk types + addressing (from `moros_map`), wall / hex geometry
collision (from `moros_sim/collide.loft`), `lib/wall.loft` (folds in
whole — `DX`/`DY`/`DZ`/`STEP` + placement/edge helpers, load-bearing
for dryopea's build-order walls + rock faces), `lib/overland.loft`
(`OverlandMap` terrain layers), group/height handling.  Palette,
spawn, editor/UI/render stay in `moros_*`.  Preserve the existing
sparse Cell/Chunk shape (TTT v5 + audience demo) alongside the hex
additions (they share addressing — Open Q #10).  Unblocks dryopea
([@PLAN46](../../plans/future/46-dryopea/README.md)).

### Phases 6w / 7b / 7c / 8

Chunk extractions + cleanup; each follows the
[per-chunk template](REFERENCE.md#per-chunk-extraction-template).  6w
needs `world` complete (7a) + green CI (6.5); 7b needs graphics +
world published; 7c is greenfield ([@PLAN46](../../plans/future/46-dryopea/README.md));
8 adds `audience_crystal` package `tests/` and updates
[PACKAGES.md](../../PACKAGES.md) to the monorepo-free state.

## See also

- [REFERENCE.md](REFERENCE.md) — inventory, stdlib boundary, chunk
  topology + dependency graph, store-allocating cdylib pattern, CI
  path, per-chunk extraction template, open questions.
- [PACKAGES.md § Open work](../../PACKAGES.md#open-work) — registry +
  format infrastructure (prerequisite).
- Sibling library plans: [`../02-graphics/`](../02-graphics/),
  [`../08-server/`](../08-server/), [`../21-datetime/`](../21-datetime/);
  game plan [@PLAN46 dryopea](../../plans/future/46-dryopea/README.md).
- [ROADMAP.md](../../ROADMAP.md) — PKG.EXTRACT milestone placement.
