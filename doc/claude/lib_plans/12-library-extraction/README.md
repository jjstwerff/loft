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
  `game_protocol` 0.1.0 (initial); transitive `loft install` +
  lockfile merge smoke-tested.  **v0.1.1 patch release shipped
  2026-05-31** (registry PR #5, all three validator gates green
  including the reproducible-build re-check) bundling Phase 6r
  + 6.5 + warning sweep + the new `byte_at.loft` coverage.
  Chain that landed it: jjstwerff/loft #234 (deterministic
  `loft package` — zero mtime in gzip + tar headers) → net #2
  (omnibus) → net #3 (version bump) → 3 tags + 3 GitHub releases →
  registry #4 (validator multi-package homepage fix) → registry #5
  (the version entries).
- **`loft-libs-graphics` partial** (Phase 5): `shapes` 0.2.0 +
  `gridmesh` 0.1.1 published.  Phase 6.5 + warning-sweep omnibus
  shipped 2026-05-31 (chunk PR #1: canonical `library-ci.yml`,
  gridmesh `step_y` `_x` rename + `buckets not null`; registry PR #6
  all three validator gates green).  `graphics` + `imaging` remain
  in the monorepo but their native-codegen blocker is **gone**
  (see below).

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
the compiler crate carrying **zero library code** AND the monorepo
carrying **no `lib/<pkg>/` directory for any extracted package** —
verified by the extraction-hygiene gate, identical behaviour across
interp / native / WASM, and `make ci` green with all consumers
resolving extracted packages through the registry rather than via
`path = "../<pkg>"`.

**Two-stage per-chunk workflow** (codified 2026-05-31, see [Phase
6.5 § Bringing a chunk to all-green CI — checklist](#phase-65--green-ci-across-chunks-done-chunk-side-2026-05-31)):

- **Stage A — green extraction:** chunk PR (canonical CI + 6r
  re-clean + warning sweep + tests) → tag + tarball + GitHub
  release → registry PR (all three validator gates green) →
  registry PR merged.  The library is now in the registry catalog
  ("in the manifest") and `loft install <pkg>` works end-to-end.
- **Stage B — monorepo cleanup:** consumers migrated from
  `path = "../<pkg>"` to the registry-version dep
  (`<pkg> = ">=0.1"`); `lib/<pkg>/` deleted; `make ci` green.
  Lands as a SEPARATE PR from Stage A.

Stage A without Stage B is "the library is published but the
monorepo still depends on its in-tree copy" — a real release for
external users, but not the end state.  loft-libs-core completed
both stages; loft-libs-net + loft-libs-graphics are Stage-A-only
as of 2026-05-31.

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
   (`loft-libs-core`: random; `loft-libs-net`: web/server).  They
   were extracted *before* the clean pattern landed, so they still carry
   the old manifest / hand-written `loft_register!` and stripped
   `#native` annotations.  Re-clean = **re-sync the now-clean monorepo
   `lib/<name>/` into each external repo** (clean `.loft` + source-scan
   `build.rs` + drop manifest) and bump their `loft-ffi-build` dep to
   `0.2`.  Prerequisites: (a) `loft-ffi-build 0.2.0` on crates.io —
   **published**; (b) bare-`#native` parser support on `main` —
   **landed** (#220 `clean native binding pattern` merged 2026-05-26,
   followed by #221/#223/#224).  **Status:** `loft-libs-core` re-clean
   landed via PR #2 (2026-05-30) — bundled with the YAML refresh and
   the arguments warning sweep.  `loft-libs-net`'s re-clean **landed
   2026-05-31** in net PR #2 (omnibus: canonical YAML + per-symbol
   6r re-clean — 9 `tcp_*` sites stripped to bare `#native`, 16 `ws_*`
   sites kept because the loft fn name genuinely differs from the
   native symbol; warning sweep — 16 `_ = self;` insertions across
   web/server; new `web/tests/byte_at.loft` — 5 tests).  Shipped to
   the registry as `web/server/game_protocol 0.1.1`; gate-3
   reproducible-build re-check green on all three.

   **Per-symbol decision rule** (learned from loft-libs-core 2026-05-30):
   the re-clean **only applies where the `#native "n_<fn>"` string is
   redundant** — i.e. where the symbol equals the function name (loft's
   default).  Where the symbol genuinely differs from the function name
   (`fn sha256_native … #native "n_sha256"` — the loft fn is `sha256_native`
   so it wraps the native `n_sha256`), the explicit string is the
   correct form and must stay.  Crypto fell into this second bucket
   and was left alone; random fell into the first and was re-cleaned.
   When opening a Phase 6r PR, audit each `#native "X"` annotation
   individually — don't sweep blindly.

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
| 4 | Extract `loft-libs-core` (arguments, random, crypto) | 1–3 + 3.5 | **SHIPPED (A+B)** — Stage A 2026-05-24; Stage B (monorepo `lib/{arguments,random,crypto}/` removed) by 2026-05-28 |
| 5 | Extract `loft-libs-graphics` (graphics, imaging, gridmesh, shapes) | 4 + [`../02-graphics/`](../02-graphics/) | **partial — Stage A only** — shapes 0.2.0, gridmesh 0.1.1 shipped to registry; **monorepo `lib/shapes/` + `lib/gridmesh/` still present** (Stage B not run; consumers in `lib/moros_*` + `lib/audience_crystal` still path-resolve them).  graphics + imaging not yet extracted (codegen-unblocked + Tier-2-unblocked, awaits Stage A start) |
| 6 | Extract `loft-libs-net` (server, web, game_protocol) | 4 + [`../08-server/`](../08-server/) | **partial — Stage A only** — 0.1.0 (2026-05-24) + 0.1.1 (2026-05-31) shipped to registry; **monorepo `lib/web/` + `lib/server/` + `lib/game_protocol/` still present** (Stage B not run; consumers in `tools/audience-demo*`, `tools/viewer`, `lib/audience_crystal` still path-resolve them) |
| 6.5 | Green CI across every chunk + registry repo (canonical `library-ci.yml`); subsumes parked tasks #61/#62/#63 | 4–6 | **DONE (chunk side)** — all three chunks on canonical YAML + at least one patch release published: `loft-libs-core` (PR #2 2026-05-30); `loft-libs-net` v0.1.1 (PRs #2 + #3 + registry #5, 2026-05-31); `loft-libs-graphics` gridmesh 0.1.1 (PR #1 + registry #6, 2026-05-31).  Remaining: registry `pr-validate.yml` already aligned (registry #4 multi-package homepage fix merged 2026-05-31); macOS / Windows matrix expansion is the only chunk-CI delta still on the to-do list (currently Linux-only by design until a Linux baseline soaks) |
| 6.6 | Auto-install on `use` — parser auto-installs from registry when an unresolved `use X;` matches a name in the signed index; one-line announcement on cold cache, silent on cache hit; works WITHOUT a `loft.toml` (script mode) and WITH one (project mode); offline opt-out via `LOFT_OFFLINE=1`/`--offline` | 3 (registry MVP), 6.5 | **OPEN — S (~1 day)** — designed 2026-05-31; unblocks Stage B for any chunk with single-file-script consumers (net's `tools/audience-demo`, graphics's `lib/graphics/examples/25-brick-buster.loft`); see [§ Phase 6.6 detail](#phase-66--auto-install-on-use-proposed-2026-05-31) below |
| 6.7 | Security advisory channel — typed `status` field on registry entries (severity tier + advisory ID + summary), separate `advisories.json` feed with 24h TTL, loft binary checks installed versions against the feed on every invocation and refuses-or-warns by severity | 6.6 (auto-install — yanked-fix paths invoke the same machinery) | **OPEN — S (~1-2 days)** — designed 2026-05-31; ships the trust signal that lets the registry recall a vulnerable version effectively; see [§ Phase 6.7 detail](#phase-67--security-advisory-channel-proposed-2026-05-31) below |
| 6.8 | `loft update` command — `loft update` refreshes lockfile to latest active versions of all declared deps; `loft update <pkg>` targets one package; project-mode explicit upgrade path that pairs with the 6.7 yank channel | 6.6 (lockfile primitives), 6.7 (advisory feed for "is the new version safe to pick") | **OPEN — S (~1 day)** — designed 2026-05-31; closes the loop "registry says you're outdated → user has a one-command fix"; see [§ Phase 6.8 detail](#phase-68--loft-update-command-proposed-2026-05-31) below |
| 6.11 | Offline bundle support — `loft bundle export <pkgs> <outdir>` + `loft bundle import <indir>` + `LOFT_REGISTRY_URL=file://` resolution; stale-advisory thresholds (`LOFT_ADVISORY_MAX_AGE`, `LOFT_ADVISORY_STALE_REFUSE`); makes air-gapped / regulated-environment / classroom-lab deployments first-class | 6.6, 6.7 | **OPEN — S (~1-2 days)** — designed 2026-05-31; companion to 6.7 advisory channel for environments that need controlled-schedule registry refresh instead of on-demand; see [§ Phase 6.11 detail](#phase-611--offline-bundle-support-proposed-2026-05-31) below |
| 6.12 | Loft-developer offline test loop — `tests/fixtures/libs/` + `scripts/sync-fixtures.sh`; bundled-fixture pattern that survives Stage B's `lib/<pkg>/` removal; eliminates "loft contributor needs internet to run tests" failure mode; mock-registry fixture for testing registry-resolution code paths | Stage B for each chunk (the gap this closes only appears once `lib/<pkg>/` is removed) | **OPEN — S (~1 day)** — designed 2026-05-31; defensive: lands BEFORE Stage B aggressive removal so the loft contributor experience doesn't regress; see [§ Phase 6.12 detail](#phase-612--loft-developer-offline-test-loop-proposed-2026-05-31) below |
| 6.13 | Documentation harvest + close-out — extract design content from this plan into permanent reference docs (PACKAGES.md / PKG_REGISTRY.md / new authoring docs); create user-facing onboarding docs (INSTALL.md / SECURITY.md / PUBLISHING.md / USING_LIBRARIES.md / library catalog); retire stale in-monorepo `lib/<pkg>/` references; CLAUDE.md table surgery; reference audit sweep; split plan into `README.md` (retrospective) + `LANDING_LOG.md`; move to `lib_plans/finished/12-library-extraction/`.  The closure ritual that prevents the plan from "just stopping" with valuable design content trapped in a finished doc no one reads | All previous 6.x phases shipped (6.5 + 6.6 + 6.7 + 6.8 + 6.11 + 6.12) + Stage B done for all three chunks (core ✓ already, net + graphics pending) | **OPEN — M (~3-5 days total, spread across the plan's tail)** — designed 2026-05-31; harvesting happens AS each 6.x phase ships (don't accumulate in the plan); see [§ Phase 6.13 detail](#phase-613--documentation-harvest--close-out-proposed-2026-05-31) below |
| 6w-w | Retire every `lib/*/.allow_warnings` opt-out — clean each library's warnings until the gate runs strict everywhere | 6.5 (gate landed) | **OPEN** — variable per package; tracks the ratchet to zero |
| 6t | Library test self-sufficiency — Tier 1 gridmesh script copies, Tier 2 `graphics_gold.rs` port, Tier 3 `multiplayer_v{2,3,5}.rs` port, Tier 4 `loft test --deps`, Tier 5 (NEW) coverage gaps with no Rust-harness home (`imaging` / `world` / `markdown`) | 4–6 | **partial** — Tiers 1+2+4 DONE; Tier 3 OPEN; Tier 5 OPEN; blocks Phase 5 (`imaging`), Phase 6r re-clean (Tier 3), Phase 6w (`world`) |
| 6w | Extract `loft-libs-world` (world, Phase-7a-expanded) | 7a + 6.5 + 6t | OPEN — M |
| 7a | Split moros: shared spatial primitives → `lib/world/` (cross-project unlock — feeds dryopea + bumper + moros; subsumes MapFile schema promotion) | 4 | **partial** — `lib/world/` shipped (sparse 32x32 model + save/load, smoke test green); `lib/wall.loft` + `lib/overland.loft` folded into `lib/world/src/` 2026-05-28; remaining: `pub` markers + moros migration + MapFile schema doc |
| 7p | Cross-cutting primitives extracted before moros leaves: `lib/moros_editor` → `lib/world_editor` rename + `lib_plans/NN-physics-2body` + `lib_plans/NN-particles` slots filed; sequencing matters more than effort | 7a | **OPEN** — XS (rename) + M (new slot designs) |
| 7b | Move moros libraries into the existing `moros` project | 5 + 6w + 7a + 7p | OPEN — MH |
| 7c | Bootstrap `dryopea` project | [@PLAN46](../../plans/future/46-dryopea/README.md) | OPEN — S |
| 8 | Final monorepo cleanup + `audience_crystal` test dir | 7b | OPEN — S |

Phase 7a is monorepo-internal (no user-visible change) and can land
before the registry work; 6w then interleaves with the remaining
chunk extractions once 7a is stable.


## Phase detail

Open-phase design content is split by topic across companion
files (this README is status + phase summary + cross-project
context only).

| File | Phases covered | Topic |
|---|---|---|
| [ci-and-warnings.md](ci-and-warnings.md) | 6.5, Bringing-a-chunk checklist, 6w-w | Canonical `library-ci.yml`, per-chunk omnibus pattern, Stage A/B sequencing, `.allow_warnings` ratchet |
| [registry-resolution.md](registry-resolution.md) | 6.6, 6.8 | Auto-install on `use` (Python-style for scripts; Cargo-style for projects); `loft update` command |
| [security.md](security.md) | 6.7 | `advisories.json` signed feed, typed severity tiers, classifier fail/warn behaviour, verify-on-recompile timing |
| [offline.md](offline.md) | 6.11, 6.12 | `loft bundle export/import`, `LOFT_REGISTRY_URL=file://`, stale-advisory thresholds, loft-developer fixture pattern |
| [closure.md](closure.md) | 6.13 | Documentation harvest + close-out ritual (the plan's own closure) |
| [stdlib-drain.md](stdlib-drain.md) | 3.6 | Scope hygiene + CVE-surface lever; what stays embedded permanently |
| [test-coverage.md](test-coverage.md) | 6t (Tiers 1-5) | Per-library test self-sufficiency; multiplayer harness port; @P389 cross-package-link blocker |
| [moros-split.md](moros-split.md) | 7a, 7p, 6w, 7b, 7c, 8 | Cross-project consumer thread: shared `lib/world/`, physics/particles slots, moros + dryopea project moves, final monorepo cleanup |

Shipped phases' build records live in git history + CHANGELOG;
the docs above are for OPEN phases only.

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
