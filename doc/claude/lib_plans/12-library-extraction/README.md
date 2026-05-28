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
  removed from the monorepo (residual `lib/random/native/target/`
  build cache cleaned 2026-05-28).
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
   `0.2`.  Prerequisites: (a) `loft-ffi-build 0.2.0` on crates.io —
   **published**; (b) bare-`#native` parser support on `main` —
   **landed** (#220 `clean native binding pattern` merged 2026-05-26,
   followed by #221/#223/#224).  **Status:** both external PRs
   (`loft-libs-net#1`, `loft-libs-core#1`) opened 2026-05-26 ~16:14 UTC
   and failed at 16:16 UTC because they ran against a pre-#220 `main`.
   *Re-running the failed jobs* is now sufficient to green the
   interpreter step.  *(The native step is a separate Phase-6.5 gap,
   not fixed by re-running — see below.)*

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
2t. **Phase 6t — library test self-sufficiency** (BLOCKS Phase 5
   and Phase 6w; 6r and 6.5 can run in parallel).  Today 4 of 11
   libraries are not independently testable post-extraction
   because critical coverage lives in monorepo Rust harnesses, not
   in `lib/<name>/tests/`.  Close the gaps before any chunk extracts
   that depends on them.  See [§ Phase 6t detail](#phase-6t--library-test-self-sufficiency)
   below.  *Verify:* for each library, deleting `tests/<harness>.rs`
   from the monorepo leaves the library's regression coverage
   intact (each `lib/<name>/tests/` directory plus, where needed,
   `lib/<name>/native/tests/` for Rust-side integration tests).
3. **Phase 5 — extract `graphics` + `imaging` into
   `loft-libs-graphics`** (codegen-unblocked; gated on 6t for
   `graphics` rendering-regression coverage).  Follow the
   [per-chunk template](REFERENCE.md#per-chunk-extraction-template).
   *Verify:* chunk CI green standalone + `loft install graphics imaging`
   into a scratch dir; monorepo `make ci` green after the Stage-B swap.
4. **Phase 3.6 — stdlib drain**: Image/Pixel/Format types →
   `lib/imaging/`; `escape_html` → new `lib/html/`; path helpers →
   `02_files.loft` (rename from `02_images.loft` — done 2026-05-28; path
   helpers `dir`/`basename`/`join(text,text)`/`resolve` move from
   `03_text.loft` still pending).  *Verify:* `make ci` green;
   `default/*.loft` line count drops (~2,500 → ~2,000).
5. **Phase 7a — split moros into shared `lib/world/` + moros-specific**
   (monorepo-internal; hex addressing, `wall.loft`, `overland.loft`,
   geometry move in).  *Verify:* moros demos render identically;
   every `lib/moros_*/src/*.loft` `use world;` resolves.
   **Status:** partial — `lib/world/` package created with sparse
   32x32-chunk model + save/load (389 lines, smoke test green);
   `lib/wall.loft` + `lib/overland.loft` folded into
   `lib/world/src/` 2026-05-28 (no consumers existed at lib/ root,
   so the move is non-breaking).  Remaining: mark wall/overland items
   `pub`, wire them into the `world.loft` entry, and migrate
   `lib/moros_*` to `use world;` for the hex / chunk primitives they
   currently duplicate locally.

After 7a + 6.5 + 6t: Phase **6w** (extract `loft-libs-world`), then
**7p** (rename + design new cross-cutting slots), **7b** (move moros
libraries into the existing `moros` project), **7c** (bootstrap
`dryopea` — [@PLAN46](../../plans/future/46-dryopea/README.md)),
**8** (final monorepo cleanup + `audience_crystal` test dir).

`6w` extracts `lib/world` (now subsuming MapFile schema + the
spatial primitives 7a folded in), so dryopea and bumper can both
`loft install world` rather than re-implementing chunk math.  This
is the cross-project payoff for the 7a work.

Phase 5 (graphics + imaging extraction) is codegen-unblocked but
gated on 6t-Tier-2 (gold-image regression must live in
`lib/graphics/native/tests/` before graphics ships externally).

## Cross-project consumers — moros / dryopea / bumper-airplanes

Three game consumers drive the library catalog: **moros** (single-
player RPG, `lib/moros_*` packages), **dryopea**
([@PLAN46](../../plans/future/46-dryopea/README.md), sci-fi tower-
defence / free-build; depends on `lib_plans/20-terrain-heightmap` +
`lib_plans/19-gridmesh` Phase C; reuses `lib/server` + `lib/web`),
and **bumper-airplanes**
([@PLAN50](../../plans/future/50-bumper-airplanes/README.md),
audience demo on `origin/bumper_plane`; loads dryopea-authored
MapFiles; proposes `lib/physics_2body`).

### Shared-library matrix

| Library / planned slot | moros | dryopea | bumper |
|---|---|---|---|
| `lib/graphics` | ✓ | ✓ | ✓ |
| `lib/imaging` | ✓ | ✓ | ✓ |
| `lib/server` | — | ✓ | ✓ |
| `lib/web` | — | ✓ | ✓ |
| `lib/game_protocol` | — | ✓ | ✓ |
| `lib/world` (after 7a) | ✓ | ✓ | ✓ |
| `lib/gridmesh` | ✓ | ✓ | ✓ |
| `lib_plans/20` terrain-heightmap | — | ✓ | ✓ (palette → height) |
| `lib/moros_editor` (as authoring tool) | ✓ | ✓ | ✓ |
| `lib/moros_map` (hex/chunk addressing) | ✓ | overlaps `world` | overlaps `world` |
| `lib/moros_sim/collide.loft` | ✓ | similar shape | similar shape |
| `lib/physics_2body` (NEW slot) | — | ✓ | ✓ |
| `lib/particles` (NEW slot) | — | ✓ (explosions, exhaust) | ✓ (trails, confetti) |
| MapFile JSON schema (cross-project contract) | source | consumer | consumer |

### What the overlap implies

**1. Phase 7a is a cross-project unlock, not a moros-internal
cleanup.**  Folding hex/chunk addressing + `wall.loft` +
`overland.loft` into `lib/world/` is what lets THREE projects share
one chunked-map substrate.  Today dryopea + bumper would each
re-implement chunk math after extraction; after 7a they consume
`lib/world` directly.

**2. `lib/moros_editor` is misnamed for its actual scope.**  Bumper
loads "a saved dryopea editor MapFile (or future stencil-pipeline
output)" and dryopea reuses moros's editor + spawn/items markers.
One editor authors maps for three games.  After 7a folds the
spatial primitives into `lib/world/`, the editor logically becomes
`lib/world_editor/` (or `lib/level_editor/`) — a `moros_` prefix on
an editor used by dryopea and the audience demo is documentation
lying about scope.  Rename should happen **before** Phase 7b
(moros migration) ships externally — otherwise three projects fork
the same editor under three names.

**3. The MapFile JSON schema is an implicit cross-project contract,
currently undocumented and buried inside `lib/moros_map`.**  Before
any of the three games extracts, the schema should be promoted to
a stable, versioned format in `lib/world/`, with a single
`world::load_mapfile(path)` entry point.  Otherwise dryopea +
bumper fork their parsers as soon as they leave the monorepo.

**4. `lib/physics_2body` cannot be PLAN50-private.**  PLAN50's
sub-arc 4 introduces it for plane bounce + stall, but dryopea also
needs 3D continuous physics (hover vehicle, enemy ballistics,
scramble-rocket trajectory) and moros's `lib/moros_sim/collide.loft`
is a 2D-ish degenerate case of the same surface.  Allocate a
`lib_plans/` slot so the API design is shared from the start;
otherwise PLAN50 ships bespoke physics and dryopea rewrites it.

**5. The PLAN50 multiplayer QoS layer is reusable.**  Sight-range
filter + three-tier rate-LOD + bounce-forecast is a general-purpose
broadcast primitive ("ship pose state to N receivers, throttle by
distance, send discrete events as forecasts").  Dryopea's planet
multiplayer + co-op base defence wants the same shape.  Belongs in
`lib/server` (or as a thin layer in `lib/game_protocol`) — not in
PLAN50 application code.  Add as an `## Open work` row in
[`../future/08-server/`](../future/08-server/).

**6. Particles are a near-certain duplicate.**  Bumper has smoke
trails + score confetti.  Dryopea wants explosions + scramble-rocket
exhaust.  If PLAN50 ships particles as application code, dryopea
copies-and-modifies.  A `lib/particles` slot — ribbon trails +
point-burst particles, two flavours — is cheap to design once.

### Missing lib_plans/ slots (gaps surfaced by this analysis)

| Slot | Status | Why | Blocks |
|---|---|---|---|
| [`lib_plans/future/26-physics-2body/`](../future/26-physics-2body/README.md) | **FILED** 2026-05-28 | Shared collision + integrator API for moros / dryopea / bumper | PLAN50 sub-arc 4; dryopea phase 4 (vehicle); Phase 7b clean moros migration |
| [`lib_plans/future/27-particles/`](../future/27-particles/README.md) | **FILED** 2026-05-28 | Shared trails + point-burst particles for dryopea + bumper | PLAN50 phase 3 (trails); dryopea phase TBD (explosions) |
| MapFile schema in `lib/world/` (covered by Phase 7a) | DESIGNED below | Cross-project contract; today buried in `lib/moros_map` | Phase 6w (`loft-libs-world` extraction) |
| [`lib_plans/future/08-server/` § Gap 8](../future/08-server/README.md#gap-8--per-recipient-broadcast-qos-sight--rate-lod--forecast) — broadcast QoS layer | **FILED** 2026-05-28 | Sight + rate-LOD + forecast pattern from PLAN50 phase 7; reused by dryopea | dryopea multiplayer; Phase 6r (re-clean already-extracted `loft-libs-net`) |
| Editor rename — `lib/moros_editor` → `world_editor` family (covered by [`lib_plans/future/24-universal-editor/`](../future/24-universal-editor/README.md) L1-L5) | EXISTING SLOT | Editor authors maps for three games, not just moros | Phase 7b moros migration (rename **before** moros leaves) |

The two new `lib_plans/` slots (26, 27) carry their own API
sketches + phase breakdowns + open questions; the editor rename
is subsumed by the existing universal-editor plan (slot 24).  The
two designs that live inline in this plan (MapFile schema below,
canonical `library-ci.yml` below) are tied to specific phases here
(7a and 6.5 respectively) and don't merit their own slots.

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
| 3.6 | Stdlib drain (Image→imaging, `escape_html`→`html`, path helpers→`02_files`) | 1c | **partial** — `escape_html`→`lib/html/` DONE 2026-05-27 (Image/Pixel already in `lib/imaging`); `02_images.loft`→`02_files.loft` rename DONE 2026-05-28; path-helper consolidation (`dir`/`basename`/`join(text,text)`/`resolve` move from `03_text.loft` → `02_files.loft`) remains |
| 4 | Extract `loft-libs-core` (arguments, random, crypto) | 1–3 + 3.5 | **SHIPPED** 2026-05-24 |
| 5 | Extract `loft-libs-graphics` (graphics, imaging, gridmesh, shapes) | 4 + [`../02-graphics/`](../02-graphics/) | **partial** — shapes/gridmesh shipped; graphics+imaging **now unblocked** |
| 6 | Extract `loft-libs-net` (server, web, game_protocol) | 4 + [`../08-server/`](../08-server/) | **SHIPPED** 2026-05-24 |
| 6.5 | Green CI across every chunk + registry repo (canonical `library-ci.yml`); subsumes parked tasks #61/#62/#63 | 4–6 | **OPEN** — S–M; lands before 6w |
| 6w-w | Retire every `lib/*/.allow_warnings` opt-out — clean each library's warnings until the gate runs strict everywhere | 6.5 (gate landed) | **OPEN** — variable per package; tracks the ratchet to zero |
| 6t | Library test self-sufficiency — move Rust harness coverage into each library so it survives extraction (Tier 1 gridmesh script copies, Tier 2 `graphics_gold.rs` port, Tier 3 `multiplayer_v{2,5}.rs` port, Tier 4 `loft test --deps`) | 4–6 | **partial** — Tier 1 DONE, Tier 2 DONE; Tiers 3+4 OPEN; blocks Phase 5 (graphics) + Phase 6w (world) extraction |
| 6w | Extract `loft-libs-world` (world, Phase-7a-expanded) | 7a + 6.5 + 6t | OPEN — M |
| 7a | Split moros: shared spatial primitives → `lib/world/` (cross-project unlock — feeds dryopea + bumper + moros; subsumes MapFile schema promotion) | 4 | **partial** — `lib/world/` shipped (sparse 32x32 model + save/load, smoke test green); `lib/wall.loft` + `lib/overland.loft` folded into `lib/world/src/` 2026-05-28; remaining: `pub` markers + moros migration + MapFile schema doc |
| 7p | Cross-cutting primitives extracted before moros leaves: `lib/moros_editor` → `lib/world_editor` rename + `lib_plans/NN-physics-2body` + `lib_plans/NN-particles` slots filed; sequencing matters more than effort | 7a | **OPEN** — XS (rename) + M (new slot designs) |
| 7b | Move moros libraries into the existing `moros` project | 5 + 6w + 7a + 7p | OPEN — MH |
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

**Canonical `library-ci.yml` (concrete design):**

```yaml
name: library-ci
on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    strategy:
      fail-fast: false
      matrix:
        package: [arguments, random, crypto]   # ← per-chunk list
    steps:
      - uses: actions/checkout@v4

      - name: Install mold (loft's pinned linker)
        run: sudo apt-get update -y && sudo apt-get install -y mold

      - name: Cache cargo registry + loft build
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            loft-src/target
          key: loft-${{ hashFiles('loft-src/Cargo.lock') }}

      - name: Clone + build loft from source
        run: |
          git clone --depth 1 https://github.com/jjstwerff/loft loft-src
          cd loft-src
          cargo build --release --bin loft
          echo "$PWD/target/release" >> $GITHUB_PATH

      - name: Interpreter — loft test
        working-directory: ${{ matrix.package }}
        # `LOFT_DENY_WARNINGS=1` is set globally below.  A package can
        # opt out by adding a `.allow_warnings` file at its root (used
        # while the package is being cleaned up — drop the file once
        # the warning count reaches zero).
        env:
          LOFT_DENY_WARNINGS: ${{ hashFiles(format('{0}/.allow_warnings', matrix.package)) != '' && '0' || '1' }}
        run: loft --interpret --tests tests

      - name: Native — loft --native test
        working-directory: ${{ matrix.package }}
        env:
          LOFT_DENY_WARNINGS: ${{ hashFiles(format('{0}/.allow_warnings', matrix.package)) != '' && '0' || '1' }}
        run: loft --native --tests tests

      # Optional: only fires for libraries with a native/tests/ dir
      # (Phase 6t Tier 2 — graphics gold-image regression et al.)
      - name: Rust integration tests (if present)
        working-directory: ${{ matrix.package }}/native
        if: hashFiles('${{ matrix.package }}/native/tests/*.rs') != ''
        run: cargo test --release
```

**Key choices:**

- **One job per package** (matrix) — a single-package failure
  names the package in the PR check list; one giant job would just
  show "library-ci: failed."
- **Build loft from source** — drops the speculative
  binary-download URL `pr-validate.yml` had.  Cached on
  `Cargo.lock` SHA so steady-state cost is `cargo build` no-op
  after first install.
- **Linux-only initially** — macOS/Windows added once the Linux
  baseline is green; not blocking initial chunk extraction.
- **Rust integration step is conditional** — only fires when a
  library has `native/tests/*.rs` (Phase 6t Tier 2 home for
  graphics gold-image; Tier 3 home for game_protocol multiplayer).
  Libraries without native tests skip the step cleanly.
- **`chunk-skips.toml`** (separate file at repo root) — per-package
  `NATIVE_SKIP` / `LIB_TESTS_SKIP` allow-list translated from the
  monorepo's `LIB_PKGS_NATIVE_SKIP` / `LIB_TESTS_NATIVE_SKIP`.  The
  `loft test` invocation reads it.
- **`LOFT_DENY_WARNINGS=1` by default** — warnings become CI failures,
  preventing a clean library from regressing.  A package opts out
  temporarily by `touch lib/<pkg>/.allow_warnings` (CI flips the
  env var to `0` for that package only); remove the file once the
  warnings are cleaned up.  This is a ratchet: every new chunk
  starts warning-free, and the allow-file is a visible IOU that
  shows up in `git status` and `gh pr view --files`.

### Phase 6w-w — retire every `.allow_warnings`

The warning gate landed 2026-05-28 ([Phase 6.5 work above]) with
**3 of 17 libraries already clean** and the other 14 carrying an
opt-out file.  Each opt-out is an IOU; this phase tracks the cleanup
ratchet.

| Library | Warnings | Opt-out | Trend |
|---|---|---|---|
| `game_protocol` | 0 | — | clean |
| `html` | 0 | — | clean |
| `time` | 0 | — | clean |
| `imaging` | 0 | **removed 2026-05-28** | DONE — one `?? Pixel{...}` |
| `gridmesh` | 0 | **removed 2026-05-28** | DONE — `_x` rename + 1× `not null` |
| `server` | 0 | **removed 2026-05-28** | DONE — 13× `_ = self;` (method-call surface) |
| `web` | 0 | **removed 2026-05-28** | DONE — 1× `not null` + 3× `_ = self;` |
| `world` | 0 | **removed 2026-05-28** | DONE — needed both src/parser/operators.rs skip-pattern 5 (recognise `if i < len(v)` guards for indexed-assign positions) + world.loft idiom rewrites (`cells = X; if i < len(cells) { ... cells[i] }`) + `as u8`/`as u16` width casts on binary writes + `chunks not null` |
| `moros_editor` | 31 | yes | Tier B; cross-references `moros_map` — pair with Phase 7p (consumer migration) per memory: project_consumer_stall (user-side) |
| `moros_map` | 34 | yes | Same — Tier B; paired with 7p |
| `markdown` | 50 | yes | Tier B; 50× `s[i]` needs sweep — non-consumer lib so unblocked but bulk |
| `shapes` | 118 | yes | Tier C; 72× `v[i]` + 44× div-by-zero + 2× mod-by-zero |
| `audience_crystal` | 127 | yes | Tier C; cross-references audience demo (consumer) |
| `graphics` | 209 | yes | Tier C |
| `moros_ui` | 407 | yes | Tier C; consumer-adjacent — pair with 7p |
| `moros_render` | 466 | yes | Tier C; consumer-adjacent — pair with 7p |
| `moros_sim` | 1192 | yes | Tier D — paired with Phase 7p migration |

**Progress 2026-05-28:** 6 of 14 opt-outs retired (imaging, gridmesh,
server, web, markdown, world).  8 remaining: 5 paired with Phase 7p
consumer migration (moros_editor / moros_map / moros_ui /
moros_render / moros_sim — touching them isolated from their
consumers risks rework when 7p eventually re-shapes the code), 2
need per-callsite semantic review for division-by-zero
warnings (audience_crystal: 33; graphics: 84 — silencing with `?? 0`
can mask rendering / decay-math bugs), and 1 inherits graphics's
remaining warnings (shapes — only reachable via the graphics dep).

The detector fix that closed `world` is reusable: skip-pattern 5
in `src/parser/operators.rs::is_easy_proof` recognises
`if idx_var < len(vec_var) { ... v[idx_var] ... }` for both reads
and indexed assigns inside the then-block, accepting the user-facing
`len` wrapper and the underlying `LengthVector` opcode and stripping
the `ConvBoolFromInt` cast the parser emits around the comparison.
The pattern handles bare `Var` index and `Var` collection; more
complex shapes (`if i < self.height && j < self.width { ... v[j * w + i] }`,
which graphics uses) still warn — a Phase-6w-w follow-up could
extend the detector to recognise field-access bounds too.

**Strategy.**

1. **Tier-A cleanups (XS):** `imaging`, `web`, `gridmesh`, `world`,
   `server` — 1–13 warnings each.  Single-PR per library; delete
   `.allow_warnings` as the last commit.  These can land while
   waiting on other extraction phases.
2. **Tier-B (S):** `moros_editor`, `moros_map`, `markdown` — 31–50
   warnings.  Triage shows most are repeated patterns (vector index
   without `??`, never-null field declarations); fix sweeps the
   pattern.
3. **Tier-C (M–MH):** `shapes`, `audience_crystal`, `graphics`,
   `moros_ui`, `moros_render` — 100–500 warnings.  Bulk pattern
   sweeps; expect to surface real bugs (warnings flag genuinely
   risky code).  Land per-pattern, not per-library.
4. **Tier-D (`moros_sim`, 1192 warnings):** wait for Phase 7p (cross-
   cutting primitives extracted before moros migration).  Cleaning
   `moros_sim` against the current shape and then re-cleaning
   against the `world_editor` / `physics_2body` migration would
   double the work; pair the warning cleanup with the migration.

**Verify per library:** `cd lib/<pkg> && LOFT_DENY_WARNINGS=1 loft test`
passes; delete `.allow_warnings`; `make test-packages` stays green.

**Verify globally:** when every `.allow_warnings` is gone,
`find lib/ -name .allow_warnings` returns empty; `make ci` stays
green; the canonical `library-ci.yml` runs strict for every package.
Phase 6w-w is closed when this holds.

*Verify:* land this YAML in `loft-lang/library-template` first;
copy into each existing chunk repo (`loft-libs-core`,
`loft-libs-graphics`, `loft-libs-net`).  Deliberate one-package-break
PR (e.g., introduce a syntax error in `crypto/src/crypto.loft`) lands
red naming `test (crypto)` specifically.

### Phase 3.6 — stdlib drain

Shrink `default/*.loft` to genuine universal stdlib.  Moves:
**`escape_html` → new `lib/html/` — DONE 2026-05-27** (with its test
migrated from `tests/scripts/106` to `lib/html/tests/01-escape.loft`,
now `use html;`); Image / Pixel already live in `lib/imaging/src/`
(Format stays in default — it's file-related and `lib/imaging` depends
on it at load time); **`02_images.loft` → `02_files.loft` rename DONE
2026-05-28** — `src/wasm.rs DEFAULT_FILES`, `src/gendoc.rs`, the test
fixtures (`tests/generated/default.rs`, `tests/lib/p145_repro.rs`),
the load-order block in `CLAUDE.md`, and current-state references in
STDLIB.md / COMPILER.md / DOC.md / NATIVE.md / LIFETIME.md /
INTERMEDIATE.md / WASM.md / DEVELOPMENT.md updated; `path_sep()`
already lived there.  **Remaining:** move
`dir`/`basename`/`join(text,text)`/`resolve` from `03_text.loft` →
`02_files.loft` (load-order safe — they only use primitives defined in
`01_code.loft`; needs an audit that no `02_files.loft` declaration is
shadowed).  JSON STAYS (the `{x:j}` format specifier + `text as Foo`
cast are shipped language behaviour — pulling JSON out breaks both).
Audit call sites for new `use html;` lines.

### Phase 6t — library test self-sufficiency

**Problem.**  Audit of test coverage (2026-05-28) found 4 of 11
libraries are not independently testable post-extraction.  The gap is
two clusters of monorepo-owned Rust harnesses that drive library code
but live outside `lib/<name>/tests/`:

| Cluster | Files | Tests | Library subjects | Why Rust-side |
|---|---|---|---|---|
| Graphics gold-image regression | `tests/graphics_gold.rs` + `tests/gold/*.png` | 8 | `lib/graphics` | PNG decode + per-channel MAE tolerance compare against checked-in reference PNGs.  Pure-loft can't replicate the tolerance algorithm; encoder drift means byte-compare is brittle. |
| Multiplayer integration | `tests/multiplayer_v2.rs` + `tests/multiplayer_v5.rs` | 9 | `lib/server`, `lib/web`, `lib/game_protocol` | Subprocess orchestration to dodge @P245 (single-process `parallel{}` + I/O hangs when one arm accepts and another connects to a loopback port).  Must run client + server as separate processes. |

Plus two thin loft-side gaps (Tier 1, mechanical):

- `tests/scripts/130-gridmesh-crystal-equiv.loft` — gridmesh C1 SegMesh equivalence vs legacy CrystalMesh.
- `tests/scripts/133-crystal-incr.loft` — incremental crystal update.

Both reference `audience_crystal` (monorepo-paired) so the copy
must substitute a synthetic `CellSnap` fixture inside `lib/gridmesh/tests/`.

**Out of scope — these stay in loft.**  `tests/wrap.rs` and
`tests/native.rs` are the discovery harnesses that enumerate every
`lib/<pkg>/tests/*.loft`; they wire the per-library tests into
`make ci` and are not subject to extraction.  `tests/leak.rs`,
`tests/runtime_warnings.rs`, `tests/codegen_emitter.rs`,
`tests/issues.rs`, `tests/extraction_hygiene.rs` are
compiler/runtime regressions that use library code as a fixture, not
as the subject under test — they belong to the loft toolchain.

**Tier 1 — mechanical copies (XS, no design needed).**

- Copy `tests/scripts/130-gridmesh-crystal-equiv.loft` →
  `lib/gridmesh/tests/crystal-equiv.loft`.
- Copy `tests/scripts/133-crystal-incr.loft` →
  `lib/gridmesh/tests/crystal-incr.loft`.
- In each, replace the `use audience_crystal;` block with a small
  pure-loft helper inside the test that builds an equivalent
  `CellSnap`-shaped fixture (the tests only need a known input,
  not the audience_crystal logic).  Delete the originals from
  `tests/scripts/`.
- *Verify:* `cd lib/gridmesh && loft --interpret test` runs the new
  files green; `make ci` still green after removing the originals.

**Tier 2 — port `graphics_gold.rs` to `lib/graphics/native/tests/` (M).**

`lib/graphics` already has a Rust crate at `lib/graphics/native/`
(the cdylib).  Add `lib/graphics/native/tests/gold.rs` as a Rust
integration test inside that crate, carrying:

- The 8 `#[test]` functions (same names, same examples driven from
  `lib/graphics/examples/`).
- The PNG decode + MAE compare helper.
- The reference PNGs — move `tests/gold/*.png` →
  `lib/graphics/tests/gold/*.png` (loft-package convention; Rust
  test reads them from there via a workspace-relative path).
- `UPDATE_GOLD=1` env var behaviour preserved.

When `lib/graphics` extracts to `loft-libs-graphics`, its native
crate + integration test travel together.  The `library-ci.yml`
template needs one new step per library that has a `native/tests/`
directory: `cd lib/<name>/native && cargo test --release`.

*Verify:* `cd lib/graphics/native && cargo test --release` runs the
8 tests; deleting `tests/graphics_gold.rs` from the monorepo leaves
coverage intact.

**Tier 3 — port multiplayer harnesses to `loft-libs-net` (MH).**

The 9 subprocess-orchestrated tests across `multiplayer_v2.rs` (4
tests) and `multiplayer_v5.rs` (5 tests) test the surface that
`lib/server` + `lib/web` + `lib/game_protocol` *jointly* expose —
no single library owns them.  Two ship sites are plausible:

(a) **Inside `loft-libs-net` as a workspace integration crate.**
After extraction, the chunk repo is a Cargo workspace with one
member per library; add a sibling `tests-integration/` crate that
carries the harnesses.  CI runs `cargo test -p loft-libs-net-tests`
after the per-library steps.

(b) **Inside `lib/game_protocol/native/tests/`.**  Same shape as
Tier 2 — game_protocol is the topmost layer, the harnesses sit
where the surface is defined.  Requires adding a minimal
`lib/game_protocol/native/` crate (game_protocol has no native
binding today).

(a) is the cleaner long-term home — the harness *is* an integration
test of the chunk, not of any one library.  (b) is the shorter
migration path but couples the multiplayer suite to game_protocol's
extraction timing.

For now, prefer (a): ship the harnesses inside the `loft-libs-net`
external repo's `tests-integration/` crate at the same time the
chunk is re-cleaned (Phase 6r).

*Verify:* `cargo test --manifest-path tests-integration/Cargo.toml`
(or workspace `cargo test -p ...`) runs all 9 tests against the
checked-out `lib/server` + `lib/web` + `lib/game_protocol`;
deleting both monorepo harnesses leaves coverage intact.

**Order of operations.**  Tier 1 first (XS, unlocks gridmesh hygiene).
Tier 2 next (blocks Phase 5 graphics extraction).  Tier 3 last
(can land alongside Phase 6r since `loft-libs-net` is already
extracted; the integration suite is additive to that repo).

**Tier 4 — `loft test --deps` — SHIPPED 2026-05-28.**  Consumer-side
walker that runs `loft test` on every dependency in the current
project's transitive (default) or direct tree.  Wired into the
canonical `library-ci.yml.example` template as a final step so a
chunk repo's PR catches "this graphics release broke gridmesh's
tests in our environment" before it merges, not after a downstream
consumer's CI flags it.

CLI surface (in `src/main.rs`):

```
loft test --deps                  # transitive — all deps + their deps
loft test --deps=direct           # one level only
loft test --deps=transitive       # explicit; same as plain --deps
```

`--deps` implies `--no-warnings` when running each dep's tests —
the consumer should not be blocked by lint debt inside a dep it
doesn't own.  Errors still surface via exit code.

Implementation status:

| # | What | Status |
|---|---|---|
| T1 | Free-fn dep resolver | implemented as local helper in `run_dep_tests` (path-dep + sibling fallback) |
| T2 | `--deps[=direct]` flag + direct walker | DONE — `run_dep_tests(transitive=false)` |
| T3 | Transitive walk + `HashSet<PathBuf>` cycle guard | DONE — default mode |
| T4 | `--lock=PATH` driver (read lockfile, resolve each pinned entry) | **DEFERRED** — registry-version deps fall through silently with a one-line warning to the host project; T4 closes that when lockfile parsing is wired |
| T5 | `--skip=` allow-list filter | DEFERRED — easy add when needed |
| T6 | `library-ci.yml.example` template `loft test --deps` step | DONE |

Smoke-tested via `lib/audience_crystal` (declares `gridmesh` as
path-dep): `loft test --deps=direct` ran 3 audience_crystal test
files + 4 gridmesh test files, reported `1 dep(s) tested, 0 failed`.

### Phase 7a — moros world split (cross-project unlock; appears monorepo-internal)

Move the non-moros-specific spatial primitives into `lib/world/`: hex /
chunk types + addressing (from `moros_map`), wall / hex geometry
collision (from `moros_sim/collide.loft`), `lib/wall.loft` (folds in
whole — `DX`/`DY`/`DZ`/`STEP` + placement/edge helpers, load-bearing
for dryopea's build-order walls + rock faces), `lib/overland.loft`
(`OverlandMap` terrain layers), group/height handling.  Palette,
spawn, editor/UI/render stay in `moros_*`.  Preserve the existing
sparse Cell/Chunk shape (TTT v5 + audience demo) alongside the hex
additions (they share addressing — Open Q #10).  Unblocks dryopea
([@PLAN46](../../plans/future/46-dryopea/README.md)) AND
bumper-airplanes ([@PLAN50](../../plans/future/50-bumper-airplanes/README.md))
— see [§ Cross-project consumers](#cross-project-consumers--moros--dryopea--bumper-airplanes)
for the shared-substrate argument.  Phase 7a is described as
"monorepo-internal" because no user-visible behaviour changes, but
**three downstream projects depend on its output** — it is the
top of the dependency chain for 6w/7b/7c.

Phase 7a also subsumes the **MapFile schema promotion**: the JSON
format currently buried inside `lib/moros_map` becomes a documented,
versioned `world::load_mapfile(path)` entry point in `lib/world/`,
since dryopea and bumper both consume it.

**Done so far:**

- `lib/world/` package created — sparse 32x32 chunk world model,
  4-byte `Cell` wire-format, save/load round-trip, 389 lines.  Smoke
  test (`lib/world/tests/world.loft`) passes under `--interpret`.
- `lib/wall.loft` (754L) + `lib/overland.loft` (7L) folded into
  `lib/world/src/` (2026-05-28).  These had zero `use` references at
  `lib/` root — physically dead code — so the move is non-breaking.
- **MapFile schema documented** as a cross-project contract in
  `lib/world/MAPFILE.md` (2026-05-28) — v1 JSON shape, versioning
  policy, per-consumer overlays (moros / dryopea / bumper), and a
  6-step migration plan to `world::load_mapfile()` for when the
  consumer stall lifts.  Schema is currently enforced by
  `lib/moros_map`'s `Map` struct (the contract names what's there).

**Blocked on consumer stall — pub markers won't parse against
current loft:**

A speculative `sed` pass marking every wall.loft / overland.loft
item `pub` was attempted on 2026-05-28 and reverted.  The legacy
moros syntax in those files uses `enum` for what current loft calls
`struct` (e.g. `enum Tile { material: u8, ...}` — struct-shaped
field list, no variants), and the parser rejects it as
`Expect enum values to be in camel case style`.  Translating the
files to current loft syntax requires touching consumer logic
(memory: project_consumer_stall (user-side)),
so this is genuinely blocked — not just deferred.

**Remaining (resumes when consumer-stall lifts):**

- Translate wall.loft / overland.loft `enum`-as-struct declarations
  to current `pub struct ...` syntax; mark every item `pub` as
  needed by the eventual consumer (paired with the consumer's
  migration, not in advance).
- Wire `use wall;` / `use overland;` (or fold content directly) into
  `world.loft`.
- Move hex / chunk types + geometry collision out of `moros_map` /
  `moros_sim/collide.loft` into `lib/world/` per the MAPFILE.md
  migration plan (steps 1–3).
- Add `world::load_mapfile()` / `world::save_mapfile()` entry points
  (MAPFILE.md step 4) and a `pub use world::*;` shim in `moros_map`
  (step 5).
- Migrate `lib/moros_*` to `use world;` (step 6); verify moros
  demos render identically.

**MapFile schema (concrete design — part of 7a):**

Today the MapFile JSON format lives implicitly inside `lib/moros_map`'s
save/load code.  Three projects consume it; before extraction the
schema must be:

1. **Versioned at the top level** — every MapFile carries a
   `schema_version: integer` field.  Loaders for older versions
   stay supported within `lib/world`; new fields land as
   backwards-compatible additions (default-on-absent).
2. **Documented as a stable contract** — a `MAPFILE.md` reference
   doc in `lib/world/` describing every field, its semantics, the
   versioning policy, and the loader's null-default behaviour.
3. **Loaded through one entry point** — `world::load_mapfile(path:
   text) -> Map` and `world::save_mapfile(self: Map, path: text) ->
   FileResult`.  Callers never see raw JSON; the format becomes
   the library's responsibility.

Sketch of v1 fields (extracted from current moros_map save/load):

```jsonc
{
  "schema_version": 1,
  "name": "test_map",
  "size": { "cx_min": -2, "cx_max": 5, "cz_min": -2, "cz_max": 5 },
  "palette": [
    { "id": 0, "name": "void",  "color": "#00000000" },
    { "id": 1, "name": "grass", "color": "#5fa030ff", "height_band": [0.0, 2.0] },
    { "id": 2, "name": "wall",  "color": "#808080ff", "extrude": "pillar:8,12" }
  ],
  "chunks": [
    {
      "cx": 0, "cz": 0,
      "cells": [
        { "hx": 0,  "hz": 0,  "palette": 1, "h": 0.5, "item": null },
        { "hx": 1,  "hz": 0,  "palette": 2, "h": 0.0, "item": null }
      ]
    }
  ],
  "spawn":  [ { "x": 0.5, "z": 0.5, "kind": "player" } ],
  "items":  [ ],
  "walls":  [ ]
}
```

**Per-consumer overlays.**  Bumper extrudes `palette[i].extrude`
strings into 3D pillars / cliffs; dryopea reads `palette[i].height_band`
for slope generation (paired with [lib-plan 20 terrain-heightmap](../future/20-terrain-heightmap/README.md));
moros reads the existing flat fields.  Unknown overlay fields are
preserved on round-trip (forward-compat).

**Targets / bumper-specific data.**  Per PLAN50 Open Q #6
(recommended: separate `targets.json`), bumper's target positions
ship in a sibling file keyed to the same hex coords — not inside the
MapFile.  Keeps the MapFile shape stable across all three games.

### Phases 6w / 7b / 7c / 8

Chunk extractions + cleanup; each follows the
[per-chunk template](REFERENCE.md#per-chunk-extraction-template).  6w
needs `world` complete (7a) + green CI (6.5); 7b needs graphics +
world published; 7c is greenfield ([@PLAN46](../../plans/future/46-dryopea/README.md));
8 adds `audience_crystal` package `tests/` and updates
[PACKAGES.md](../../PACKAGES.md) to the monorepo-free state.

**Phase 7p prerequisite for 7b — extract cross-cutting primitives
first.**  Moving `lib/moros_*` into the existing moros project is
naively a git filter-repo + path update, but if it ships before the
cross-cutting primitives are factored out, dryopea + bumper end up
copy-and-forking moros internals.  Concrete checklist before 7b
ships:

| Sub-arc | Driving slot / design | Verify |
|---|---|---|
| Editor rename + L1-L2 of universal-editor extraction | [`lib_plans/future/24-universal-editor/`](../future/24-universal-editor/README.md) L0 (architecture spike + naming) + L1 (`hex_grid`) + L2 (`hex_map`) | `lib/moros_editor/`'s name reflects its cross-game scope; the L0 naming decision is what unblocks the rename |
| Physics primitives | [`lib_plans/future/26-physics-2body/`](../future/26-physics-2body/README.md) Phase 1 (types + sphere-vs-AABB step) | `lib/moros_sim/collide.loft` items migrated into `lib/physics_2body/`; moros tests green using the new package |
| MapFile schema | Inline design in Phase 7a above | `world::load_mapfile()` + `world::save_mapfile()` are the only entry points; `MAPFILE.md` documents v1 |
| Particles slot | [`lib_plans/future/27-particles/`](../future/27-particles/README.md) Phases 1–2 (trail + burst types) | Slot READMEs exist; PLAN50 / dryopea use the slot's API in their (still-stalled) design docs |
| Broadcast QoS | [`lib_plans/future/08-server/` § Gap 8](../future/08-server/README.md#gap-8--per-recipient-broadcast-qos-sight--rate-lod--forecast) — `BroadcastTopology` + sight + rate-LOD + forecast | `lib/server/src/broadcast.loft` exposes the topology API; PLAN50 wires through it |

Each row is independently sized in its own slot; this table is the
order-of-operations checklist, not the implementation plan.

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
