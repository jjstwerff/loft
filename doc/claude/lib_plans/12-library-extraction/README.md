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

## Open phase detail

Shipped phases' build records live in git history + CHANGELOG; the
detail below is for the OPEN phases only.

### Phase 6.5 — green CI across chunks (DONE chunk-side 2026-05-31)

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

**`mmap_storage` gotcha — `cargo build --release --bin loft` is not enough.**
First chunk-CI rollout (`loft-libs-core` PR #2, 2026-05-29) failed
every package's native step with `error[E0463]: can't find crate for
mmap_storage which loft depends on`.  Diagnosis: without explicit
`--lib`, cargo emits `libloft.rlib` only into
`target/release/deps/`, never into the parent `target/release/`.
`loft_lib_dir()` finds the deps-only rlib but the surrounding
`-L dependency=` search path then can't resolve transitive crates.
Fix: `cargo build --release --lib --bin loft`.  Verified by a clean
fresh-clone build locally — `--bin loft` reproduces the CI failure,
`--lib --bin loft` makes it green.  The canonical template
([library-ci.yml.example](library-ci.yml.example)) now carries
the `--lib` flag and a multi-line comment explaining why.

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

### Bringing a chunk to all-green CI — checklist

Distilled from loft-libs-core's path to green (PR #2, 2026-05-30),
reapplied to loft-libs-net (PRs #2 + #3 + tags + releases +
registry #4 + #5, all 2026-05-31), and again to loft-libs-graphics
(PR #1 + gridmesh-v0.1.1 release + registry #6, also 2026-05-31).
The pattern has now been applied **three times** across three
distinct chunk shapes (pure-loft single package, multi-package with
native cdylibs, multi-package pure-loft).  Apply this checklist when
bringing any future chunk to fully-green strict CI; no
chunk-specific divergences have surfaced yet.

**Prerequisite:** the loft compiler bugs surfaced by the FIRST chunk's
warning sweep must be fixed on `jjstwerff/loft:main` before later
chunks attempt the same sweep.  loft-libs-core surfaced @P385 + @P386;
new chunks may surface different latent bugs.

**Steps (in order):**

1. **Copy the canonical [library-ci.yml.example](library-ci.yml.example)**
   to `.github/workflows/library-ci.yml` in the chunk repo.  Replace
   the matrix list with the chunk's actual packages.  Confirm the
   `--lib --bin loft` flag is present (load-bearing — see Phase 6.5
   above for the `mmap_storage` gotcha).

2. **Per-package Phase 6r re-clean** — for each `#native "X"`
   annotation, check if `X` equals the function name (loft's default).
   If yes, drop to bare `#native`.  If no (genuine override), leave
   it alone.  Don't blanket-apply.  Update `loft.toml` to drop the
   `[native.functions]` manifest if all annotations are now bare;
   add a `build.rs` calling
   `loft_ffi_build::generate_register_from_loft("../src")`; bump
   `loft-ffi-build = "0.2"` build-dep; replace any hand-written
   `loft_register!` block with `include!(…/loft_register_gen.rs)`.

3. **Per-package warning sweep** under `LOFT_DENY_WARNINGS=1` — use
   the three idioms documented in `.claude/skills/loft-write/SKILL.md`
   § Warning-clean idioms:
   - `not null` on vector fields safe-to-default-to-`[]`
   - capture-into-local before indexing (skip-pattern 5 needs bare-Var vec)
   - capture-and-null-check (the `x = v[i]; if x != null` hint)

4. **Verify locally first** via `scripts/verify_external_libs.sh`
   in the monorepo, which mirrors the chunk's CI but builds loft from
   the current working tree.  This catches a) compiler bugs that need
   a `libraries → main` PR first, b) source-syntax issues, c) any
   YAML drift from the canonical template.

5. **Land as ONE PR per chunk** (omnibus pattern from loft-libs-core
   PR #2).  Separate PRs for YAML / re-clean / warning sweep are
   interdependent and individually red — bundling them shows the
   cumulative-green outcome and avoids 3 review cycles.

6. **Release sequence (Stage A — green extraction):** bump
   `<pkg>/loft.toml` version → tag `<pkg>-v<version>` at the merge
   SHA → `loft package` (deterministic — same sha256 twice in a
   row) → `gh release create <tag>` with the tarball as the asset →
   open registry PR adding the new version entries to `index.json`
   → wait for all three validator gates (schema lint, tarball
   sha256+size verify, reproducible-build re-check) green → merge
   registry PR.  THIS is "the extraction is green" — the library
   resolves end-to-end via the registry, from anywhere on the
   internet.

7. **Stage B — remove `lib/<pkg>/` from the monorepo.**  ONLY
   after Stage A is green AND the registry PR is merged (which is
   when "the library is in the manifest" — the registry's
   `index.json` is the catalog).  Steps per extracted package:
   - **Audit consumers.**  Grep the monorepo for `use <pkg>;` and
     `path = ".*<pkg>"` references outside the package's own dir
     (every `lib/*/loft.toml`, every `tools/*/loft.toml`,
     `examples/`, `tests/`).  Today (2026-05-31) the residual
     consumers are: `web`/`server` → `tools/audience-demo*` +
     `tools/viewer` + `lib/audience_crystal`; `game_protocol` →
     same set; `gridmesh` → `lib/audience_crystal` + likely
     `lib/moros_*`; `shapes` → `lib/moros_*` candidates (verify).
   - **Migrate each consumer's `loft.toml`** from
     `<pkg> = { path = "../<pkg>" }` to the registry-version form
     `<pkg> = ">=0.1"` (or `>=0.2` for the libraries that bumped
     major-zero minor).  Tools without a `loft.toml` (single-file
     `.loft` scripts under `tools/audience-demo/`) get a minimal
     `loft.toml` declaring the registry dep, or are migrated to
     `loft install` against a workspace-level lockfile if one
     exists for that tool tree.
   - **Verify resolution** by running `make ci` (or the equivalent
     targeted suite for the affected consumers — `cargo test
     --test native_library_suite` covers most monorepo lib
     resolution paths).  Resolution must succeed via the registry,
     not via path-deps to `lib/<pkg>/`.
   - **Delete the directory:** `git rm -r lib/<pkg>/`.  Re-run
     `make ci` — the tree must still be green.  No silent fallback
     to a stashed copy.
   - **Land as a single Stage-B PR** per chunk (or per package if
     one chunk needs multiple migration waves).  The PR title
     pattern: `Stage B: remove lib/<pkg>/ — consumers migrated to
     registry <pkg> <version>`.

**Done when (Stage A):** all matrix jobs green on the chunk's CI
under `LOFT_DENY_WARNINGS=1`, no `.allow_warnings` opt-out files
in the chunk, `scripts/verify_external_libs.sh --src <chunk>=…`
is green against the latest monorepo `lib/<name>/` source, and
the registry PR adding the new version is merged with all three
validator gates green.

**Done when (Stage B):** no `lib/<pkg>/` directory exists in the
monorepo, no `path = "../<pkg>"` references survive in any
`loft.toml`, no monorepo-internal `use <pkg>;` statement resolves
to a path-dep (it must resolve through the registry / lockfile).
`make ci` green.

**Why Stage B is a SEPARATE PR from the omnibus.**  Stage A is
chunk-side work; Stage B is monorepo-side work touching unrelated
consumers.  Bundling them inflates review surface, mixes risks
(monorepo consumer break vs chunk release), and complicates
rollback (if Stage B breaks a consumer, you can revert just Stage
B without unpublishing the registry release).  Each chunk's
Stage B PR is the trigger to bump the row in the phase summary
table from "Stage A SHIPPED" to "SHIPPED (Stage A + B)".

**Release-loop lessons from loft-libs-net v0.1.1 (2026-05-31).**
Applying the loft-libs-core path-to-green to net surfaced two
hidden dependencies that the first chunk didn't expose:

- **Deterministic packaging is a registry prerequisite, not a
  nice-to-have.**  loft-libs-core's first releases happened to
  hash-match by accident (the build environment was identical
  between author and validator).  Net's release would have failed
  gate-3 reproducible-build re-check on every PR because
  `loft package` baked the current `mtime` into both the gzip
  header and every tar entry.  Fix shipped in jjstwerff/loft #234:
  `GzBuilder::new().mtime(0)` + `header.set_mtime(0)` +
  `set_uid(0)` + `set_gid(0)`.  Two consecutive `loft package`
  runs on the same source now produce byte-identical tarballs.
  Verified locally + by the registry validator on PR #5.
- **The registry validator only handled single-package chunk
  repos.**  loft-libs-core's homepage points at
  `loft-libs-core/tree/main/random` and the tag is `random-v0.1.0`;
  the validator's original logic cloned the homepage URL verbatim
  (a `tree/main/random` path) with a `v0.1.0` tag — both wrong for
  multi-package chunks.  Fix shipped in registry #4 (validator
  multi-package homepage handling): parse the URL, cd into the
  subpath, use `<pkg>-v<version>` as the tag.  Without this fix,
  every multi-package chunk release would fail gate-3 even with
  deterministic packaging.

**Order of operations for the third chunk** (`loft-libs-graphics`):
both fixes are already on registry main + jjstwerff/loft main as
of 2026-05-31, so the third chunk only needs the omnibus PR
itself.  Estimated path-to-green: ~1 work-day for the omnibus
+ 1 hour for the release sequence + ~5 min for the registry
PR.  The first two chunks each surfaced 1-2 compiler bugs
during the warning sweep (loft-libs-core: @P385/@P386; net: none
new); budget 1 day per chunk for "fix surfaced compiler bug, land
on monorepo main, rebase chunk PR" rounds.

**Third-chunk result (loft-libs-graphics, 2026-05-31).**
End-to-end took ~30 minutes (chunk PR omnibus + verification +
tag + tarball + GitHub release + registry PR), not the budgeted
~1 work-day, because the third chunk surfaced **no new
chunk-specific work**:
- `shapes` was already at 0.2.0 (had yanked its `graphics` dep in
  an earlier release) and already warning-clean.  No source change.
- `gridmesh` only needed re-syncing from the monorepo's cleaned
  source (the chunk repo was forked before the 2026-05-28 sweep
  that landed `_x` rename + `not null` on `buckets`).  Diff is
  4 lines.
- Tarball reproducible-build re-check passed first try (the
  deterministic-packaging fix from net was sufficient).
The remaining engineering time was a transient GH Actions
runner issue (registry PR #6's first validator run hung 16+ min
on `sudo apt-get install -y mold` — almost certainly an apt
mirror or runner-region issue, not anything we caused).
Cancel-and-rerun produced a healthy run in 1m33s.  **Lesson:**
when `apt-get install mold` doesn't finish in <60s, the runner
is hung — cancel + rerun rather than waiting on the 20-min
timeout.

### Phase 6.6 — auto-install on `use` (proposed 2026-05-31)

**Trigger.**  Stage B for `loft-libs-graphics` (and later
`loft-libs-net`) surfaced a friction point: monorepo consumers
that lack a `loft.toml` (`tools/audience-demo/*.loft`,
`lib/graphics/examples/25-brick-buster.loft`) need `use gridmesh;`
/ `use shapes;` to keep resolving after `lib/<pkg>/` is removed
from the monorepo.  The existing registry-resolution path
(`probe_registry_installed` in `src/parser/mod.rs:4328`) requires
a `loft.lock` in cwd, which in turn requires the user to have
run `loft install <pkg>` first.  That's an extra step every
single-file script needs before it can run.

The Python comparison applies: `python my_script.py` with
`import requests` works if `pip install requests` has been done
once.  The loft equivalent should be: `loft my_script.loft` with
`use gridmesh;` Just Works on first run, with the loft binary
silently doing the `pip install` step on the user's behalf.

**Scope.**  Add an auto-install fallback to the parser's
`use` resolution chain.  When an unresolved `use X;` matches a
name in the signed registry catalog, the parser auto-installs
the latest active version and retries.

**Two modes the user can be in:**

| Mode | Trigger | Behavior |
|---|---|---|
| **Script** | bare `.loft` file, no `loft.toml` in any parent dir | `use X;` auto-installs from registry; one-line `[registry]` announcement on cold cache; silent on cache hit; optional `<script>.loft.lock` sidecar generated by `loft pin <script>` for reproducibility. |
| **Project** | `loft.toml` present in script's dir or any parent up to `/` | Manifest-declared deps install from the registry; `loft.lock` pins exact versions; mirrors Cargo/npm. |

Both modes share `~/.loft/registry/` cache, Ed25519-signed
`index.json`, sha256 per-tarball verification, and `LOFT_OFFLINE=1`
/ `--offline` opt-out.

**Surprise reduction.**  The parser is allowed to fetch ONLY
names that resolve in the signed registry catalog — never
arbitrary URLs.  Every cold install prints three `[registry]`
lines (fetch + verify + extract).  Steady-state (cache hit) is
silent.  Same noise model as Cargo's "Downloading…" /
"Compiling…" output.  The trust surface shrinks to "is the
registry honest?" — same surface as crates.io / PyPI.

**Implementation outline (S, ~1 work-day):**

1. **Parser bridge** (`src/parser/mod.rs::probe_registry_installed`):
   extend the existing fallback to, on lockfile-miss, look up the
   bare name in `loft::registry_index::load_or_fetch_index()` and
   call a new `loft::install::auto_install(name)` if the catalog
   has it.  Return the install path so the existing
   `lib_path_manifest` lookup picks up the source.
2. **`auto_install(name)` in `src/install.rs`**: resolve "latest
   active" version from the loaded index, call the existing
   `install_one()` machinery to fetch + verify + extract.  Update
   the cwd's `loft.lock` if one exists; otherwise write/update
   `~/.loft/global.lock` (script-mode lockfile).
3. **Walk-up project detection**: from the script's directory,
   walk up to `/` looking for `loft.toml`.  Found → project mode
   (lockfile next to that toml).  Not found → script mode (use
   global lockfile).
4. **`loft pin <script>` command**: capture the current
   resolution snapshot into `<script>.loft.lock` next to the
   script.  Subsequent runs prefer the sidecar over global lock.
5. **Announcement output**: `[registry] <pkg> <ver> — fetching
   <url>` then `[registry] sha256 <hex> ✓` then `[registry]
   extracted to ~/.loft/registry/<pkg>-<ver>`.  Cache hit: silent.
   First-time index fetch: `[registry] refreshing index from
   <url>`.
6. **Offline enforcement**: `LOFT_OFFLINE=1` env var or
   `--offline` flag → all network paths refuse with explicit
   "offline mode: pkg X not in cache" messages.
7. **`loft list-installed` command**: enumerate
   `~/.loft/registry/*-*/` directories with their sha256 + size
   from the cached index.  No new state; just a query helper.

**Tests** (in `tests/install.rs` or new `tests/auto_install.rs`):

- Cold cache + online → install fires, paper trail printed,
  package resolves.
- Cold cache + `LOFT_OFFLINE=1` → refused with explicit message
  naming the missing package.
- Warm cache → silent resolution, no network.
- Project mode (loft.toml present, loft.lock present): existing
  behaviour, no auto-install fires for already-locked packages.
- Project mode + missing lock entry for declared dep → auto-install
  fires and merges into lockfile.
- Script mode + sidecar (`script.loft.lock` present) → use
  pinned version, not "latest active".
- Name not in registry catalog → error with suggestion (suggest-
  similar via Levenshtein, mirroring plan-07 phase 5).
- Tampered registry signature → refused (existing
  `registry_signing.rs` path; just verify the auto-install code
  also hits it).

**Open questions (decide during implementation):**

1. **Index refresh policy.**  How stale can the local
   `~/.loft/registry/index.json` get before we refresh?  Options:
   on every cold install (chatty); TTL of 7 days; explicit
   `loft registry sync` only (least chatty but new packages don't
   appear without a manual step).  Recommendation: TTL of 7 days
   with `[registry] refreshing index (X days old)` notice when
   refresh fires.
2. **Script-mode lockfile location.**  `~/.loft/global.lock`
   (one global file, all scripts share) or
   `~/.loft/scripts/<script-path-hash>.lock` (per-script,
   isolated)?  Recommendation: per-script with content-hash key
   — avoids version conflicts across unrelated scripts.
3. **`loft pin` UX.**  Sidecar file next to script (visible,
   committable) or hidden entry in `~/.loft/`?
   Recommendation: sidecar — matches user intuition; `git status`
   surfaces it; user can delete it to unpin.
4. **Multi-dep walks.**  Should auto-install resolve transitive
   deps?  Yes — the install machinery already does this via the
   index's `deps` field; just plug it in.
5. **Range constraints in script-mode.**  Without a `loft.toml`,
   the user has no way to express `gridmesh = ">=0.1.1"`.  Default
   to latest active.  Users who want pinning use `loft pin`.

**Stage B unblock.**  With Phase 6.6 shipped, Stage B for any
chunk becomes: `git rm -r lib/<pkg>/` + `make ci` → first run
prints a few `[registry]` lines as the auto-install fires for
each migrated consumer; subsequent runs silent.  No new
`loft.toml` needed in `tools/audience-demo/` or per-example dirs.

### Phase 6.7 — security advisory channel (proposed 2026-05-31)

**Trigger.**  6.6 ships auto-install, which makes adoption
broader.  Broader adoption + a YEAR-old cached version + a CVE
filed today = users running known-vulnerable code with no
mechanism for the registry to tell them.  The package format
already has a `yanked` field on each entry, but (a) the schema
is untyped (no severity tier), (b) there's no separate
fast-refresh feed (the full `index.json` is large and only
refreshed periodically), and (c) the loft binary doesn't check
the yank list on every invocation.  This phase closes those
gaps.

**Schema bump.**  Each version entry gains an optional typed
`status`:

```json
"0.1.1": {
  ...,
  "status": {
    "kind": "yanked",
    "severity": "security_critical",
    "advisory": "GHSA-xxxx-yyyy-zzzz",
    "summary": "TLS bypass in ws_client_connect"
  }
}
```

Severity tiers, with default loft-binary behaviour:

| Tier | Behavior |
|---|---|
| `security_critical` | **Refuse to build / run.**  Exit non-zero with the advisory URL.  Override: `LOFT_SECURITY_OVERRIDE=<advisory-id>` (env var, audit-trail). |
| `security_high` | **Warn loudly** at start of every run; non-zero exit only under `--strict-security` (CI flag). |
| `security_low` / `bug` | One-line warning per run. |
| `deprecated` | One-line note per day (suppressed by daily-cadence state). |

**Advisory feed — `advisories.json`.**  Sibling to `index.json`
in the registry, signed by the same Ed25519 key.  Schema:

```json
{
  "schema_version": 1,
  "updated": "2026-05-31T12:00:00Z",
  "retention_days": 90,
  "advisories": [
    {
      "id": "GHSA-xxxx-yyyy-zzzz",
      "packages": [{"name": "web", "affected": ">=0.1.0, <0.1.2", "fixed_in": "0.1.2"}],
      "severity": "security_critical",
      "summary": "TLS bypass in ws_client_connect",
      "published": "2026-05-30T08:00:00Z",
      "references": ["https://github.com/loft-lang/loft-libs-net/security/advisories/..."]
    }
  ]
}
```

Two reasons it's separate from `index.json`:

- **Refresh cadence.**  `advisories.json` is small (~kilobytes;
  90-day retention) → cheap to refresh every 24h on the user's
  loft binary.  `index.json` is the full catalog → refresh every
  7d, batched with cold install.
- **Audit shape.**  Retained advisories are append-only; old
  entries don't churn when a new package version ships.  Easier
  to mirror, monitor, and audit independently of the active
  catalog.

**Loft-binary check.**  On every invocation that resolves a
package (cached install OR fresh auto-install):

1. Compute (package_name, version) tuples for each loaded
   library.
2. Load `~/.loft/registry/advisories.json` (refresh if >24h
   old AND online).
3. For each tuple, check the affected range against advisory
   entries; classify by severity.
4. Apply the severity table above.

**Output examples.**

```
# security_critical — fail
$ loft my_script.loft
error: gridmesh 0.1.1 was yanked for a security vulnerability
  advisory: GHSA-xxxx-yyyy-zzzz
  summary:  TLS bypass in ws_client_connect
  fix:      gridmesh >=0.1.2 (run `loft install gridmesh@0.1.2`)
  override (audit-trail required): LOFT_SECURITY_OVERRIDE=GHSA-xxxx-yyyy-zzzz

# security_high — warn loud
$ loft my_script.loft
warning: web 0.1.0 has a known security issue
  advisory: GHSA-aaaa-bbbb-cccc
  summary:  Memory disclosure in HTTP parser
  fix:      web >=0.1.2 (run `loft install web@0.1.2`)
hello world

# bug (yanked but non-security)
$ loft my_script.loft
warning: gridmesh 0.1.0 was yanked (bug)
  fix: gridmesh >=0.1.1
hello world
```

**Verification timing — when hashes get checked.**

Five orthogonal moments.  The design goal: **steady-state
script runs pay nothing for verification.**  Loft is being
optimised for "many runs of small scripts" (cold-start work in
CS.C1/C2/C3); per-invocation hashing would noise the wrong
axis.  Verification binds to compile-cache invalidation
instead.

| # | Moment | What's verified | Default | Off-switch |
|---|---|---|---|---|
| 1 | **Install / auto-install** | sha256 (matches `index.json`) + Ed25519 sig on tarball | always | — (cannot disable) |
| 2′ | **At compile (cache miss)** | every library in the dep graph being compiled: cached install's sha256 matches `index.json`.  Amortised into compile time — when bytecode is being regenerated anyway, hashing N libraries adds milliseconds to an operation already costing hundreds. | on | `LOFT_NO_BUILD_VERIFY=1` |
| 3 | **Advisory feed refresh** | every 24h: re-fetch `advisories.json`, verify sig | on | `LOFT_OFFLINE=1` (uses cache) |
| 4 | **Per-invocation advisory check** | each loaded (name, version) tuple compared against cached advisories — µs in-memory lookup once feed is loaded | on | none — advisories always checked when feed cached |
| 5 | **`loft audit`** | exhaustive: re-hash every cached package + advisory match for every entry in cache and current lockfile.  Ignores all caches/markers — the explicit deep-scan path. | manual | — |

**Steady-state cost** (warm bytecode cache, no source / lockfile
changes): only moment 4.  µs per run.  Effectively free.

**What triggers a recompile (and thus moment 2′ firing):**

- Source mtime drift on any `.loft` file in the dep graph.
- Lockfile changed (auto-install fired or `loft update` ran).
- Compiler version changed (different `loft --version`).
- Target changed (`--interpret` ↔ `--native` ↔ `--html`).
- **NEW** — cached install's mtime drifted since last compile.
  Catches post-install tamper of `~/.loft/registry/<pkg>-<ver>/`.
  Cost: one `stat` per loaded library on compile-cache-hit
  path (microseconds).

That last invalidation rule is the closes-the-gap addition.
Without it, a modify-cached-library-and-restore-its-mtime attack
sails through cache hits forever; with it, ANY mtime change on
the cached install triggers a recompile and the recompile path
re-hashes (moment 2′) → mismatch caught.

**Threat model honesty.**  The retired moment "per-invocation
library hash" was only catching a narrow attack model
(modify-cached-library-WITHOUT-touching-mtime) that already
assumes the attacker has write access to `~/.loft/registry/`.
At that access level, they could equally replace the loft
binary, modify shell rc to set `LOFT_NO_BUILD_VERIFY=1`, or
tamper `~/.loft/installed.toml`.  Per-invocation hashing was
paying ~5-30ms per run for ~1% of the threat surface; that
tradeoff is wrong for a "many small runs" target.
`loft audit` (moment 5) remains the explicit escape hatch for
users who want to re-hash everything on demand.

Stdlib coverage is implicit: `default/*.loft` lives inside the
loft binary via `include_str!`, so verifying the loft binary's
bytes (lib-plan 30 § Phase 30.4 — stat-on-startup with
hash-on-drift) verifies the embedded stdlib by transitivity.
No separate stdlib-file hash check needed.

**Implementation outline (~1-2 work-days):**

1. **Registry schema bump.**  `tools/validate.py` in
   `loft-lang/registry` accepts the new typed `status` field;
   keep the old free-form string accepted in input but normalise
   on emit.  Add `advisories.json` + `advisories.json.sig` to
   the gate-1 schema lint.
2. **Advisory feed maintenance.**  Document the workflow in
   `REGISTRY_SUBMIT.md`: when yanking a version for security,
   author submits a PR adding both the per-version `status` AND
   an `advisories.json` entry referencing the GHSA.  CI verifies
   the cross-reference.
3. **Loft binary — advisory loader.**  New
   `src/registry_advisories.rs`: load + verify signature +
   cache `advisories.json` with 24h TTL (verification moment 3).
   Honours `LOFT_OFFLINE=1` (use cache; error if cache empty).
4. **Loft binary — compile-time library hash** (verification
   moment 2′).  Hook in the bytecode-cache miss path: when the
   compiler is about to regenerate bytecode for a script + its
   dep graph, hash each cached library's on-disk bytes and
   compare to the entry in `index.json`.  Mismatch → refuse to
   compile.  Cache-hit path: skip hashing entirely; the cached
   bytecode already encoded the verified state.  ALSO add a
   new cache-invalidation rule: cache-hit becomes a cache-miss
   if any loaded library's on-disk mtime drifted since the
   cache was written.  Off-switch: `LOFT_NO_BUILD_VERIFY=1`
   (intended for fully-offline development against
   known-trusted local builds).  The bytecode cache key
   itself encodes the verification state, so a successful
   verify is implicitly cached alongside the compile output.
5. **Loft binary — per-invocation advisory check**
   (verification moment 4).  After a (name, version) tuple
   lands, classify against the cached advisories.  Defer
   fail/warn emission until `main`'s pre-execute point so we
   never warn for the same package twice in one run.
6. **Override mechanism.**  `LOFT_SECURITY_OVERRIDE=<id>` env
   var allows running with a `security_critical` yanked
   version, but emits a stderr audit line: `[security] override
   applied: GHSA-xxxx-yyyy-zzzz (gridmesh 0.1.1)`.  Used for
   incident response: if the user is the one INVESTIGATING the
   CVE, they need to run the vulnerable version locally.
7. **`loft audit` command** (verification moment 5).  Explicit
   exhaustive query — scans the current lockfile (project mode)
   or the global cache (script mode), re-hashes every package
   against `index.json` (ignoring `.sha256.verified` markers),
   re-checks every tuple against advisories, and reports every
   discrepancy + every affected version without running
   anything.  Exit code reflects worst severity found.

**Tests** (in `tests/registry_advisories.rs`):

- Advisory matches version → fail/warn per severity.
- Advisory doesn't match → silent.
- Cached advisories.json absent + offline → fall through with
  diagnostic warning (don't refuse, but tell user "advisory
  feed unavailable; could not check security status").
- Cached advisories.json present + offline → use cache.
- Tampered signature → refuse to use the feed; surface the
  error.
- Override env var → run + audit-log to stderr.
- `loft audit` against a fixture lockfile with multiple severities
  → exit code matches worst.

Plus, for verification timing (moments 2′ + 5 above):

- **Cold-cache compile** (bytecode cache miss): hashes fire,
  match → compile proceeds; mismatch → compile refused with
  expected vs actual sha256.
- **Warm-cache run** (bytecode cache hit, no mtime drift):
  no hashing, no I/O beyond `stat`; steady-state cost is µs.
- **Mtime drift invalidation**: modify a cached library's
  bytes + touch the file → next run sees mtime drift,
  invalidates bytecode cache, re-hashes (moment 2′) → mismatch
  caught.
- **Mtime drift WITHOUT content change** (e.g. `touch` after
  a benign rebuild): cache invalidates, re-hash succeeds,
  compile proceeds normally — slightly slower but correct.
- **`LOFT_NO_BUILD_VERIFY=1`** → moment 2′ skipped at compile
  time; explicit stderr note `[verify] build verify disabled
  (LOFT_NO_BUILD_VERIFY)`.  Used for offline dev against
  locally-trusted libraries.
- **`loft audit` mismatch detection**: tamper a cached
  install (with OR without mtime restoration) → audit reports
  the bad sha256; exit code reflects worst severity.  This
  catches the `modify-then-restore-mtime` attack that bypasses
  moment 2′'s mtime trigger.

**Open questions:**

1. **Override audit storage.**  Just stderr, or also a file
   (`~/.loft/security_overrides.log`)?  Recommendation: stderr
   only — file logging adds a maintenance burden and we already
   write to stderr; users / CI who care can capture it.
2. **Range syntax for `affected`.**  Cargo's semver, Python's
   PEP 440, or a simple form?  Recommendation: pin to the
   existing loft.toml range syntax (`>=X, <Y`) for consistency.
3. **Multi-advisory aggregation.**  If 3 advisories hit one
   package, do we print all 3 or only the most-severe?
   Recommendation: print all; one line each.  Users investigating
   want the full picture.
4. **Retention beyond 90 days.**  Should advisories for
   long-ago-yanked versions stay in the feed indefinitely, or
   move to a separate "archive" file?  Recommendation: 90-day
   active + archive in `advisories-archive.json` for queries
   targeting old versions.

**Why this isn't deferred to a future plan.**  PLAN12 is the
*adoption* arc; security advisory is the trust signal that
makes wider adoption defensible.  Without 6.7, every published
loft library is one CVE away from a manual disclosure
campaign.  WITH 6.7, the registry recall mechanism is
mechanical and audit-friendly.

**`"package": "loft"` is a valid advisory entry.**  The same
schema covers the loft binary itself: a CVE in the parser, in
native codegen, in the runtime, or in any `default/*.loft`
stdlib file that didn't drain (per Phase 3.6) becomes an
advisory entry with `"package": "loft"`, version range, and
fix-in version.  The classifier in step 4 uses the SAME logic
for binary and library tuples — only the lookup key changes.
This is what permanently covers the non-drainable stdlib
floor (operators, base types, control flow, format strings,
core collection ops, bootstrap I/O) — the parts of the stdlib
3.6 deliberately leaves embedded.  Practical example:

```
$ loft my_script.loft
error: loft 0.8.4 was yanked for a security vulnerability
  advisory: GHSA-zzzz-yyyy-xxxx
  summary:  Format string evaluator allows arbitrary read in user-controlled payloads
  fix:      loft >=0.8.5 (run `loft self-update`)
```

The `loft self-update` referenced in the fix line is shipped
by [`lib_plans/30-loft-distribution/`](../30-loft-distribution/README.md)
— Phase 6.7 produces the advisory, Phase 30 provides the
mechanical fix path.  Both halves are required to make the
trust chain useful for the binary; 6.7 alone surfaces the
problem, 30 alone has no signal to act on.

### Phase 6.8 — `loft update` command (proposed 2026-05-31)

**Trigger.**  6.7 surfaces "you have an outdated version" but
the user's next move is awkward: edit `loft.toml` to bump the
version range, then re-run `loft install <pkg>@<new>`, then
maybe edit the lockfile.  Project-mode lacks an explicit
"update everything to the latest version that satisfies my
range constraints" command.  6.6 ships the install primitives;
6.7 ships the yank signal; 6.8 is the explicit user-driven
update flow that closes the loop.

**Scope.**

```
loft update                  # refresh every dep in loft.lock to latest active
                             # that satisfies its loft.toml range
loft update <pkg>            # refresh one package
loft update --major          # allow major-version bumps that step outside
                             # the current range (writes new range to loft.toml)
loft update --dry-run        # report what would change without writing
loft update --check          # exit non-zero if any updates are available
                             # (CI gate)
```

**Behaviour matrix.**

| Invocation | Reads | Writes | Network |
|---|---|---|---|
| `loft update` (project mode) | loft.toml, loft.lock | loft.lock | one HTTP per affected pkg |
| `loft update` (script mode) | `~/.loft/global.lock` (or sidecar) | same | same |
| `loft update <pkg>` | as above, scoped to one pkg | same | one HTTP |
| `loft update --major` | as above | loft.toml (range bump) + loft.lock | same |
| `loft update --dry-run` | as above | nothing | catalog refresh only |
| `loft update --check` | as above | nothing | catalog refresh only |

**Range-bump semantics.**  Without `--major`, `loft update`
respects the existing `loft.toml` range:

- `gridmesh = ">=0.1"`  →  picks the highest active 0.x.y
- `gridmesh = ">=0.1, <0.2"`  →  picks the highest active 0.1.y
- `gridmesh = "0.1.1"`  →  no change (range is exact)

With `--major`, the range itself can be bumped — useful when a
library publishes 1.0 and the user wants to follow.  Always
prints what changed.

**Implementation outline (~1 work-day):**

1. **Range parser.**  Already exists at `loft-ffi-build`'s
   manifest resolver; reuse for `loft update`.
2. **`loft update` driver.**  Walk lockfile entries; for each,
   call `loft::install::install_one(name, latest_in_range)`;
   write updated lockfile.  Emit a per-package diff: `gridmesh
   0.1.1 → 0.1.2 (security fix GHSA-xxxx)`.
3. **`--major` mode.**  Allowed-to-bump range write requires
   editing `loft.toml`.  Use the manifest read/write helper
   from `src/manifest.rs`; preserve comments and ordering
   (PKG_REGISTRY.md § Manifest editing).
4. **`--check` mode.**  Exit codes: 0 if all up-to-date, 1 if
   updates available, 2 if YANKED versions are present (calls
   into 6.7's classifier).  Used as a CI gate by chunk repos
   that want "fail PRs that introduce stale deps."
5. **Integration with 6.7.**  When 6.7's classifier reports a
   `security_*` yank, `loft update <pkg>` automatically picks
   a version that ISN'T yanked, even if the current range
   would otherwise include the yanked version.  Behavior is
   "fix the security issue first; respect the range constraint
   second."  Audit line: `[update] gridmesh 0.1.1 → 0.1.2
   (skipping yanked 0.1.1)`.

**Tests** (in `tests/update.rs`):

- Project mode + range satisfied by newer version → updates,
  rewrites loft.lock.
- Project mode + range satisfied only by current version →
  no change, prints "already latest in range".
- Script mode + sidecar → updates sidecar.
- Script mode + global lock → updates global lock.
- `--dry-run` → prints diff, doesn't write.
- `--check` + updates available → exit 1.
- `--check` + yanked version → exit 2.
- `--major` → rewrites loft.toml range; preserves surrounding
  formatting.
- Integration with 6.7: yanked version in current range →
  skipped + audit line.

**Open questions:**

1. **Lockfile diff format.**  Per-pkg one-liner ("`gridmesh
   0.1.1 → 0.1.2`") or unified-diff over the lockfile?
   Recommendation: per-pkg lines for human reading; a
   `--format=json` flag for tooling.
2. **Pre-update hook for breaking changes.**  Should `loft
   update` consult a `[breaking_changes]` field on the version
   entry (from registry) and print a heads-up before
   updating?  Recommendation: file as 6.9 if/when the registry
   schema supports it; out of scope here.
3. **Concurrent invocation safety.**  Two `loft update` runs
   in parallel against the same loft.lock — file lock or
   last-writer-wins?  Recommendation: simple file lock on
   `loft.lock.lck`; concurrent invocation is rare.

**Why this is small.**  Most of the machinery already exists:
- `install_one` (6.6) does the fetch + extract.
- Lockfile read/write (`src/lockfile.rs`) is done.
- Range parser is in `loft-ffi-build`.
- 6.7's classifier provides the yank skip logic.

6.8 is the user-facing glue + a thin `--major` mode.  One day
of focused work.

### Phase 6.11 — offline bundle support (proposed 2026-05-31)

**Trigger.**  6.6 + 6.7 designs assume periodic network access
(auto-install on first encounter; 24h advisory refresh).  Some
real loft-adoption environments don't have that:

- **Air-gapped servers** — defence / banking / medical
  infrastructure where outbound network is policy-forbidden.
- **Regulated environments** — registry refresh must happen
  on a controlled, audited schedule, not on-demand by user
  invocation.
- **Classroom / training labs** — no per-machine internet
  access; one bundled snapshot served from a local share.
- **Edge devices** — intermittent connectivity; the device
  should function during outages.
- **Build farms behind strict firewalls** — outbound HTTPS
  may be locked down to specific allow-lists.

Without explicit offline support, every loft script that uses
a registry library on these machines is broken.

**Scope.**  Add three pieces:

1. **Bundle export tool** — `loft bundle export` writes a
   self-contained directory with the registry artifacts a
   target machine needs.
2. **Bundle import tool** — `loft bundle import` populates
   `~/.loft/registry/` from a bundle directory, verifying
   signatures as it goes.
3. **`LOFT_REGISTRY_URL=file://...` resolution** — the loft
   binary treats a local file:// URL the same as the canonical
   HTTPS URL, walking the same signature + sha256 chain.

**Bundle export — concrete shape:**

```
$ loft bundle export --packages web,server,gridmesh --output /tmp/bundle
[bundle] including web @ latest active (0.1.1)
[bundle] including web dependencies: (none)
[bundle] including server @ latest active (0.1.1)
[bundle] including server dependencies: web @ 0.1.1 (already bundled)
[bundle] including gridmesh @ latest active (0.1.1)
[bundle] copying index.json + sig
[bundle] copying advisories.json + sig (current as of 2026-05-31T12:00:00Z)
[bundle] bundle size: 87 KB
[bundle] /tmp/bundle/ ready
```

Layout:

```
/tmp/bundle/
├── index.json                      ← full catalog (may be filtered)
├── index.json.sig
├── advisories.json                 ← current at export time
├── advisories.json.sig
├── packages/
│   ├── web-0.1.1.tar.gz
│   ├── web-0.1.1.tar.gz.sig
│   ├── server-0.1.1.tar.gz
│   ├── server-0.1.1.tar.gz.sig
│   ├── gridmesh-0.1.1.tar.gz
│   └── gridmesh-0.1.1.tar.gz.sig
└── manifest.json                   ← bundle metadata (export time, included pkgs)
```

Flags:

- `--packages <list>` — explicit list of packages to include
  (with transitive deps auto-resolved).
- `--all` — include every package in `index.json`.
- `--platforms <list>` — include binary entries for these
  platforms (interacts with lib-plan 30).  Default: skip
  binary entries.
- `--include-toolchain` — bundle the loft binary itself
  (lib-plan 30 § Phase 30.2 toolchain entries).
- `--registry-url <url>` — bundle from a non-default registry
  (e.g. company-internal mirror).
- `--filter-index` — strip `index.json` to only the included
  packages (smaller bundle; loses ability to install untracked
  packages on the target).  Default: keep full index.

**Bundle import — concrete shape:**

```
$ loft bundle import /tmp/bundle
[bundle] verifying index.json signature ... ok
[bundle] verifying advisories.json signature ... ok (current 2026-05-31T12:00:00Z)
[bundle] verifying web 0.1.1 sha256 ... ok
[bundle] verifying server 0.1.1 sha256 ... ok
[bundle] verifying gridmesh 0.1.1 sha256 ... ok
[bundle] extracting to ~/.loft/registry/
[bundle] 3 packages imported
```

Import always overlays (never replaces) so multiple imports
accumulate.

**`LOFT_REGISTRY_URL=file://...`:**

```bash
# Option A: point at an unbundled mirror dir
export LOFT_REGISTRY_URL=file:///mnt/loft-mirror/index.json

# Option B: point at an imported bundle's index
export LOFT_REGISTRY_URL=file:///tmp/bundle/index.json
```

The loft binary's registry-resolution paths treat this URL
the same as the canonical HTTPS URL: parse index.json, verify
signature, look up packages, fetch tarballs from the URLs
inside the index (which may themselves be `file://` for a
fully self-contained mirror).

**Stale-advisory thresholds:**

```bash
LOFT_ADVISORY_MAX_AGE=30          # Print stale warning if feed older (default: 30 days)
LOFT_ADVISORY_STALE_REFUSE=1      # Refuse to run if feed too stale (default: off)
LOFT_ADVISORY_STALE_THRESHOLD=90  # Days past which "stale refuse" fires (default: 90)
```

On the air-gapped machine, the operator picks a refresh
cadence (weekly? monthly?) and bundles+imports on that
schedule.  The thresholds give a graceful warning ramp →
optional hard stop for regulated environments.

**Implementation outline (S, ~1-2 work-days):**

1. **`loft bundle export`** — new subcommand in `src/main.rs`.
   Reuses `loft::install::resolve_deps` from 6.6 for transitive
   resolution.  Reuses `loft::registry_index::fetch_signed` for
   the index + sig.  Streams tarballs from the HTTP source to
   local files; never extracts.
2. **`loft bundle import`** — verify each artifact in turn,
   extract tarballs into `~/.loft/registry/<pkg>-<ver>/`.
   Refuses on first signature/hash mismatch (no partial
   imports).
3. **`file://` URL support** — extend
   `loft::registry_index::fetch_signed` to read `file://`
   URLs in addition to `https://`.  The signature chain is
   identical; only the transport changes.  Add `file://`
   security note: a writable mirror dir is a privileged
   attack surface (anyone who can write to it can inject a
   package that the SIG-verified-against-different-key gate
   catches, but trust depends on the operator's mount
   permissions).
4. **Stale-advisory threshold logic** — in
   `src/registry_advisories.rs`: load advisory feed →
   compute `now - feed.updated_at` → warn at MAX_AGE → refuse
   at STALE_THRESHOLD if STALE_REFUSE=1.
5. **`manifest.json` in bundle** — bookkeeping: when the
   bundle was made, against which registry URL, with which
   loft binary version (advisory in case the operator wants
   to also ship the matching binary alongside).

**Tests** (in `tests/bundle.rs`):

- Export + reimport on a clean target → all packages resolve.
- Export with `--packages X,Y` → only X, Y, and deps included.
- Tampered tarball in bundle → import refuses.
- Tampered sig in bundle → import refuses.
- Bundle older than STALE_THRESHOLD + STALE_REFUSE=1 →
  invocation refused; stderr names the threshold.
- `LOFT_REGISTRY_URL=file://` against a mirror dir → resolves
  identically to HTTPS.
- Export + transport over `scp` (simulated by a `cp` to a
  fresh tmpdir) + import → no info loss.

**Open questions:**

1. **Bundle sig over the whole bundle, or just per-artifact?**
   Each artifact is independently signed by the existing
   registry chain.  Adding a "bundle envelope" sig adds
   another verification step but no new trust (the contents
   are already signed).  Recommendation: skip the envelope;
   per-artifact sigs are sufficient.  `manifest.json` is
   advisory metadata, not security-critical.
2. **`--filter-index` vs full index.**  A filtered index is
   smaller but breaks "I want to add another package later
   without re-bundling."  Recommendation: full index is
   default; `--filter-index` is opt-in for tiny bundles.
3. **Bundle format stability.**  Should we version the
   bundle format (`manifest.json.schema_version = 1`)?  Yes
   — cheap insurance for future bundle-tool evolution.

**Why this is small.**  6.6's resolver + 6.7's signature
verification machinery already cover most of the work.
Bundle export is "walk the resolver + copy files instead of
extracting"; bundle import is "verify-then-extract for each
file"; `file://` is one match arm in the URL fetcher.  The
stale-advisory thresholds are 20 lines in
`registry_advisories.rs`.

### Phase 6.12 — loft-developer offline test loop (proposed 2026-05-31)

**Trigger.**  Stage B (per-chunk `lib/<pkg>/` removal) creates
a real friction point for loft contributors: the monorepo's
test suite needs library code to compile against (parser
tests, codegen tests, library_suite), but after Stage B that
library code lives in external chunk repos.  Without explicit
support, every loft contributor needs network access on first
checkout to populate `~/.loft/registry/` for the tests to
pass.

**Goal:** routine `cargo test` against a fresh loft checkout
needs **zero network access** — the loft contributor can work
on the train, on a plane, on a CI runner without outbound
network, on a fresh laptop straight from `git clone`.

**Approach: bundled fixtures, not registry-cached installs.**

- A new `tests/fixtures/libs/<pkg>/` tree carries a snapshot
  of each extracted library's source (small enough to commit;
  typically <100KB per lib).
- The fixtures are the source of truth for compiler tests,
  not the registry installs.
- A `scripts/sync-fixtures.sh` script regenerates fixtures by
  cloning the chunk repos at a pinned tag and copying the
  source — run by the maintainer when chunks ship a new
  version that the loft compiler should test against.
- A doc-hygiene CI gate verifies fixtures match published
  versions (catches "maintainer forgot to sync").

**Why fixtures, not a registry cache:**

| Approach | Pros | Cons |
|---|---|---|
| **Bundled fixtures** (recommended) | Zero internet for `cargo test`; reproducible across machines; tests pin a specific version of each lib | Drift between published version and fixture if maintainer forgets to sync (CI gate prevents) |
| Registry-cached installs | No fixture maintenance; tests follow latest published version | Internet on first checkout; per-machine state; "cleaned ~/.loft" breaks tests |
| Live chunk-repo clones | Always-current | Per-developer setup ritual; CI clones increase test runtime |

Bundled fixtures match the "loft developer dependencies live
in the repo" expectation and survive Stage B cleanly.

**Layout:**

```
tests/fixtures/libs/
├── arguments/
│   ├── loft.toml
│   ├── src/arguments.loft
│   └── tests/
├── crypto/
│   ├── loft.toml
│   ├── src/crypto.loft
│   ├── native/  ← also fixture; thin Rust cdylib
│   └── tests/
├── gridmesh/
│   ├── loft.toml
│   ├── src/gridmesh.loft
│   └── tests/
└── ...
```

A `tests/fixtures/libs/README.md` documents:
- These are NOT canonical sources — they're snapshots.
- Source of truth lives in the chunk repos
  (`loft-libs-core`, `loft-libs-net`, `loft-libs-graphics`).
- Refresh ritual: `scripts/sync-fixtures.sh`.
- Pinning: each fixture's `loft.toml` carries an explicit
  version that matches the chunk-repo tag the snapshot was
  taken from.

**`scripts/sync-fixtures.sh` outline:**

```bash
#!/usr/bin/env bash
# scripts/sync-fixtures.sh - refresh tests/fixtures/libs/
# from the canonical chunk repos.  Run when a chunk ships
# a new version that loft tests should track.

set -euo pipefail

REPOS=(
  "loft-libs-core: arguments random crypto"
  "loft-libs-net:  web server game_protocol"
  "loft-libs-graphics: shapes gridmesh"
)

for line in "${REPOS[@]}"; do
  repo="${line%%:*}"
  pkgs="${line#*: }"
  tmpdir=$(mktemp -d)
  git clone --depth 1 "git@github.com:loft-lang/$repo.git" "$tmpdir"
  for pkg in $pkgs; do
    rm -rf "tests/fixtures/libs/$pkg"
    cp -r "$tmpdir/$pkg" "tests/fixtures/libs/$pkg"
  done
  rm -rf "$tmpdir"
done

git status tests/fixtures/libs/
echo "Inspect the diff; commit if intentional."
```

**Test infrastructure changes:**

- `tests/wrap.rs::collect_library_tests` already iterates
  `lib/<pkg>/tests/*.loft`.  After Stage B's removal, that
  list shrinks.  Add a sibling
  `collect_fixture_library_tests` that iterates
  `tests/fixtures/libs/<pkg>/tests/*.loft`.
- New `library_fixture_suite` test (mirror of
  `library_suite`) that drives the fixture libraries.
- Parser/codegen tests that today read `lib/<pkg>/src/*.loft`
  switch to `tests/fixtures/libs/<pkg>/src/*.loft`.
- Path-deps in `tests/fixtures/libs/<pkg>/loft.toml` resolve
  via the existing sibling-fallback (looking at
  `tests/fixtures/libs/<sibling>/`).

**Mock-registry fixture** (companion to the libs fixtures):

`tests/fixtures/mock-registry/` carries a fake `index.json`
+ `advisories.json` + `packages/` for testing
registry-resolution paths (6.6's auto-install, 6.7's
advisory check, 6.8's `loft update`, 6.11's bundle import).
Used by `tests/auto_install.rs` etc. via
`LOFT_REGISTRY_URL=file://./tests/fixtures/mock-registry/`.

**Implementation outline (S, ~1 work-day):**

1. **`scripts/sync-fixtures.sh`** — write the script;
   regenerate `tests/fixtures/libs/` for the current state of
   the three chunk repos.
2. **Test harness updates** — switch parser/codegen tests
   that read `lib/<pkg>/` to read `tests/fixtures/libs/<pkg>/`.
3. **`library_fixture_suite`** — add the mirror suite in
   `tests/wrap.rs`.  Initially runs ALONGSIDE `library_suite`
   (both pass while `lib/<pkg>/` exists); after Stage B for
   each chunk, the corresponding `lib/<pkg>/` rows disappear
   from `library_suite` automatically.
4. **`tests/fixtures/mock-registry/`** — minimal mock with
   2-3 fixture packages + signed feed.  Signed by a test key
   committed to the repo (`tests/fixtures/test-key.pub` +
   `.sec`); test runs override the production key via env.
5. **Doc-hygiene CI gate** — new test in
   `tests/doc_hygiene.rs` that runs
   `scripts/sync-fixtures.sh` in `--check` mode (no
   mutation) and fails if the fixtures drift from the
   canonical chunk repos.  Bypassable by intentional pinning
   when the loft tests intentionally test against an older
   library version.

**Tests** (in `tests/fixtures_hygiene.rs`):

- `tests/fixtures/libs/<pkg>/loft.toml` version matches the
  pinned version in `scripts/sync-fixtures.sh`.
- `library_fixture_suite` passes (loft tests against fixtures
  green).
- `LOFT_REGISTRY_URL=file://./tests/fixtures/mock-registry/`
  resolves `gridmesh@0.1.1` (or whichever the mock includes).
- After `cargo clean` + `rm -rf ~/.loft`, `cargo test` still
  passes (no network needed).

**Open questions:**

1. **Sync gate strictness.**  Should "fixture drift" fail CI
   hard, or just warn?  Recommendation: hard fail; if the
   maintainer intentionally pins an older version, they pin
   the chunk-repo tag in `sync-fixtures.sh` and the gate
   compares against THAT tag, not the chunk-repo head.
2. **Fixture native code.**  Crypto / web / server / imaging
   have `native/` Rust cdylibs.  Those build artifacts are
   per-platform; do we commit them or rebuild on first test
   run?  Recommendation: rebuild — the `auto_build_native`
   path already handles this; committed binaries are a
   different mess.
3. **Pin policy for sync-fixtures.sh.**  Always pin to the
   latest published version, or pin to a specific tag
   committed in the script?  Recommendation: pinned commit
   SHAs in the script.  Forces an explicit "update fixtures"
   ritual when chunks ship new versions.

**Why this lands BEFORE Stage B aggressive removal.**  Stage
B's removal of `lib/<pkg>/` (per chunk) would break loft's
own test suite unless 6.12 has shipped first.  Sequencing
this is critical:

```
For each chunk:
  Stage A (publish externally + green registry PR)  ← already shipped for core/net/graphics-partial
  6.12 sync the relevant fixtures                   ← NEW pre-step
  6.12 verify cargo test passes against fixtures    ← NEW pre-step
  Stage B (remove monorepo lib/<pkg>/)              ← only NOW safe
```

Without 6.12, Stage B inflicts a "loft contributors need
internet" regression on every monorepo checkout.  With 6.12,
the regression is bounded to "maintainers need to sync
fixtures when chunks ship new versions" — a planned,
auditable cadence.

### Phase 6.13 — documentation harvest + close-out (proposed 2026-05-31)

**Trigger.**  Plan-12 is unusually doc-heavy.  By the time
6.5 + 6.6 + 6.7 + 6.8 + 6.11 + 6.12 + Stage B + lib-plan 30
have all landed, this README is ~2500+ lines of design
content, decision records, and lessons learned.  Closing the
plan by simply moving the file to `lib_plans/finished/`
strands all of that — finished plans are read for archaeology,
not as ongoing reference.  Without explicit doc-harvest work,
"where's the canonical authoring guide?" gets the wrong
answer "open this 2026 plan."

**Goal.**  When plan-12 closes:

- Every piece of durable design content lives in a PERMANENT
  reference doc (PACKAGES.md, PKG_REGISTRY.md, or a
  purpose-named new doc), not in the finished plan.
- The finished plan is a compressed retrospective +
  chronological landing log.  ~500 lines, not ~2500.
- User-facing onboarding for "install loft", "use libraries",
  "publish a library", "security model", "library catalog"
  exists as top-level docs (not buried inside Claude-internal
  references).
- No stale in-monorepo `lib/<pkg>/` references survive in any
  doc.
- `CLAUDE.md`'s doc index reflects the new layout; reading-
  by-goal paths route through current docs, not finished
  plans.

**Scope.**  Six categories of work, each enumerated in detail
in [§ Evaluation — doc state after plan-12 lands](#suggested-closure-sequence-the-work-to-actually-do)
above.  Recapped here as concrete deliverables:

| Category | Output | Lines |
|---|---|---|
| **Migration from plan-12** | Move design content from this plan to permanent docs.  Phase 6.5 template → PACKAGES.md § Library CI; Phase 6r per-symbol rule → PACKAGES.md or `LIBRARY_AUTHORING.md`; Phase 6.6 auto-install → PACKAGES.md § Auto-install; Phase 6.7 advisory schema → PKG_REGISTRY.md § Security advisories; Phase 6.8 `loft update` → `CLI.md`; Phase 6.11 offline → `OFFLINE.md`; Phase 6.12 dev-loop → DEVELOPMENT.md § Test fixtures; verify-on-recompile tables → PACKAGES.md § Verification + lib-plan-30. | varies |
| **New Claude-internal docs** | `LIBRARY_AUTHORING.md` (end-to-end "publish a library" guide); `OFFLINE.md` (air-gap + bundle workflow + loft-dev offline loop). | ~400 + ~250 |
| **New user-facing docs (repo root)** | `INSTALL.md` (install.sh + OS packages + self-update); `SECURITY.md` (trust model + vuln disclosure); `PUBLISHING.md` (author's view); `USING_LIBRARIES.md` (consumer's view of `use` + manifest + lockfile + CLI). | ~150 + ~200 + ~300 + ~250 |
| **Library catalog generator** | Script that pulls `index.json` from the registry and writes a markdown catalog page; CI auto-update; published at `loft-lang.org/libraries` or `doc/library-catalog.md`. | ~50 (script) + dynamic page |
| **CLAUDE.md table surgery** | Add new docs to index; retire obsolete reading-by-goal rows ("Implement `loft install`" → done; "Build the `server` library" → it lives in chunk repo now); update reading-by-goal paths so "Add a feature to the compiler" doesn't route through plan-12. | ~30 row changes |
| **Reference audit + sweep** | `grep -rln "PLAN12\|plan-12\|12-library-extraction" doc/` — rewrite every survivor to point at the new permanent doc OR cite the finished-plan retrospective.  No reference points at an open phase. | per-file edits across ~20 docs |
| **Plan-12 closure** | Split `lib_plans/12-library-extraction/README.md` into `README.md` (compressed retrospective, ~500 lines) + `LANDING_LOG.md` (chronological per-phase landing record, ~500 lines).  `git mv` to `lib_plans/finished/12-library-extraction/`. | retrospective rewrite + log compile |

**Harvest cadence — DON'T accumulate.**

The critical rule: harvest each 6.x phase's design content
INTO the permanent doc AT THE TIME THE PHASE SHIPS, not at
plan close.  Otherwise 6.13 becomes a 2-week migration
sprint where one person tries to remember why each design
decision was made.

```
Phase 6.6 ships → extract auto-install design into PACKAGES.md § Auto-install
Phase 6.7 ships → extract advisory schema into PKG_REGISTRY.md § Security advisories
Phase 6.8 ships → extract `loft update` UX into CLI.md
Phase 6.11 ships → create OFFLINE.md from the section
Phase 6.12 ships → extract fixture pattern into DEVELOPMENT.md § Test fixtures
...
Phase 6.13 close → just the user-facing docs + cleanup + plan split
```

At 6.13 time, the plan README is already ~half-migrated.
What's LEFT in the plan is just:
- Retrospective narrative (kept; that's the closure record).
- Stage A / Stage B per-chunk landing log (kept; chronological).
- Implementation lessons not yet folded elsewhere (rare; fold them).

**Implementation outline (M, ~3-5 work-days):**

1. **Per-phase harvest (continuous, ~half day per shipping
   phase)** — when a phase merges, extract its `### Phase 6.x detail`
   section's permanent content into the target reference doc.
   Leave a compact landing-record stub in the plan ("Shipped
   2026-Q? in commit `<sha>`; design now at PACKAGES.md § Foo").
2. **User-facing doc creation (sprint, ~2 days)** —
   `INSTALL.md`, `SECURITY.md`, `PUBLISHING.md`,
   `USING_LIBRARIES.md`.  Each ~150-300 lines, mostly
   reorganisation + tone shift from Claude-internal to
   user-facing.
3. **Library catalog generator (~half day)** — `scripts/gen_library_catalog.py`
   that pulls `index.json` and writes `doc/library-catalog.md`.
   Wire into CI to auto-update on registry change.
4. **CLAUDE.md surgery (~half day)** — add new docs to index,
   retire obsolete rows, rewrite reading-by-goal paths that
   still route through plan-12.
5. **Reference audit (~half day)** — `grep -rln` for the
   plan / phase identifiers; rewrite each surviving reference
   to point at the permanent doc.
6. **Plan-12 split + move (~half day)** — compress the
   plan's narrative into a retrospective README; extract
   per-phase landing chronology into LANDING_LOG.md; `git mv`
   to `lib_plans/finished/12-library-extraction/`.

**Verification (the gate that closes the plan):**

A `tests/doc_hygiene.rs` test plus a manual checklist:

```rust
#[test]
fn plan12_no_open_phase_references() {
    // After plan-12 closes, every reference to PLAN12 / plan-12 / 12-library-extraction
    // in doc/ must point at either:
    //   (a) lib_plans/finished/12-library-extraction/README.md (retrospective), OR
    //   (b) lib_plans/finished/12-library-extraction/LANDING_LOG.md (chronology)
    // NEVER at "Phase X" inside an open plan.

    // Walk doc/, grep for the identifiers, classify each survivor.
    // Fail if any "Phase 6.X" or "§ Phase X" reference survives outside the finished plan.
}
```

Manual checklist (the "done when" recital):

- [ ] `make ci` green (existing gates plus the new plan12_no_open_phase_references).
- [ ] All shipping 6.x phases have a one-line "Shipped {date}: see {permanent-doc}" entry in the plan.
- [ ] `INSTALL.md`, `SECURITY.md`, `PUBLISHING.md`,
      `USING_LIBRARIES.md`, `LIBRARY_AUTHORING.md`,
      `OFFLINE.md` exist and pass doc_hygiene.
- [ ] `doc/library-catalog.md` generates cleanly from `index.json`.
- [ ] `CLAUDE.md` doc-index table reflects new docs; obsolete
      reading-by-goal rows retired.
- [ ] `grep -rln "PLAN12" doc/` returns only references to the
      finished plan + LANDING_LOG (no "Phase X" pointers
      survive).
- [ ] Plan moved to `lib_plans/finished/12-library-extraction/`;
      readme compressed to retrospective shape (~500 lines).
- [ ] LANDING_LOG.md present with per-phase chronology.
- [ ] User who's never seen the plan can reach "how do I
      publish a library?" from `CLAUDE.md` in ≤2 clicks.

**Why this is its own phase, not "just close-out work."**

Closure is real work.  Other recently-closed plans
(`plans/finished/22-mutable-closures/`,
`plans/finished/52-value-block-borrow-cleanup/`,
`plans/finished/44-hash-semantics/`) demonstrate that doc
discipline at close determines whether the finished plan
serves as a useful artifact or becomes archaeology.  Plan-12
is unusually large; its closure work deserves explicit phase
status so it doesn't get treated as "the cleanup task someone
will get to."

**Open questions:**

1. **Catalog format.**  HTML page on `loft-lang.org`, or
   markdown in the repo, or both?  Recommendation: markdown
   in repo (commitable, no hosting dep) + auto-rendered HTML
   on loft-lang.org as a polished view.
2. **Retrospective compression target.**  Keep all design
   detail, or just decisions + outcomes?  Recommendation:
   decisions + outcomes + the few "what we learned" lessons
   that surface design pitfalls future projects should know
   about.  Implementation detail moves to permanent docs.
3. **Should 6.13 also fold lib-plan 30's design?**  No —
   lib-plan 30 is its own slot, with its own lifecycle.  Its
   doc harvest is lib-plan-30's responsibility when it
   closes.
4. **Versioned doc snapshots.**  Should the doc state be
   tagged with each minor release ("this is how things
   worked in v0.9.0")?  Out of scope for 6.13;
   CHANGELOG.md already serves this purpose at the user
   level.

### Phase 6w-w — retire every `.allow_warnings`

The warning gate landed 2026-05-28 ([Phase 6.5 work above]) with
**3 of 17 libraries already clean** and the other 14 carrying an
opt-out file.  Each opt-out is an IOU; this phase tracks the cleanup
ratchet.

**Strict-warnings is also a compiler-fuzzer.**  Lesson from
loft-libs-core's `arguments` sweep (2026-05-29): the warning sweep
surfaced **TWO previously-latent compiler bugs** that had never been
hit in practice — @P385 (parser type-inference asymmetry on
`if cond { v[i] ?? null } else { null }` returning text) and @P386
(native codegen `Str/&str` mismatch for text-nullable returns).
Both were fixed in jjstwerff/loft #231 (merged 2026-05-30) before
the arguments PR could land green.  **Plan for this pattern on each
chunk:** budget time during the first strict sweep to file + fix
1-2 compiler bugs that surface.  The bugs were always there; the
strict warnings just give us the test cases that find them.

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
| `markdown` | 0 | **removed 2026-05-28** | DONE (already cleaned in the 2026-05-28 sweep; line was stale until the 2026-05-29 audit) |
| `shapes` | 118 (transitive) | yes | Tier C; **the warnings actually originate in `lib/graphics/src/`** — shapes imports graphics, so its strict-warnings count tracks graphics's count directly.  Closing graphics also closes shapes. |
| `audience_crystal` | 120 | yes | Tier C; cross-references audience demo (consumer).  Count was 127 on 2026-05-28; partial cleanup landed since. |
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

**Dual purpose** (re-articulated 2026-05-31, after the Phase 6.7
security-channel design surfaced the asymmetry between drainable
and embedded stdlib):

1. **Scope hygiene** — move surfaces that are NOT language
   primitives out of `default/*.loft` into purpose-named
   libraries (HTML escaping doesn't belong in the language
   core; image types don't either).
2. **CVE-surface lever** — every surface that leaves the
   embedded stdlib becomes patchable on the library release
   cadence instead of the binary release cadence.  Faster
   security fixes for drainable territory.  The non-drainable
   floor (operators / base types / control flow / format
   strings / core collection ops / minimum bootstrap I/O) stays
   in the binary permanently and is covered by Phase 6.7's
   advisory channel for `"package": "loft"` entries.

**The drain does NOT shrink to zero.**  Phase 6.7 covers the
permanent floor.  3.6 is "move what should never have been in
the floor"; it isn't a strategy to externalise the entire
stdlib.

Done so far: **`escape_html` → new `lib/html/` — DONE 2026-05-27**
(with its test migrated from `tests/scripts/106` to
`lib/html/tests/01-escape.loft`, now `use html;`); Image / Pixel
already live in `lib/imaging/src/` (Format stays in default — it's
file-related and `lib/imaging` depends on it at load time);
**`02_images.loft` → `02_files.loft` rename DONE 2026-05-28** —
`src/wasm.rs DEFAULT_FILES`, `src/gendoc.rs`, the test fixtures
(`tests/generated/default.rs`, `tests/lib/p145_repro.rs`), the
load-order block in `CLAUDE.md`, and current-state references in
STDLIB.md / COMPILER.md / DOC.md / NATIVE.md / LIFETIME.md /
INTERMEDIATE.md / WASM.md / DEVELOPMENT.md updated; `path_sep()`
already lived there.

**Remaining (active):**

- Move `dir`/`basename`/`join(text,text)`/`resolve` from
  `03_text.loft` → `02_files.loft` (load-order safe — they only
  use primitives defined in `01_code.loft`; needs an audit that
  no `02_files.loft` declaration is shadowed).
- Audit call sites for new `use html;` lines.
- Future candidates as they mature: regex, JSON, CSV, base64,
  date/time helpers — each becomes a library package the same
  way `lib/html` did.  Schedule by maturity, not by clock.

**STAYS in stdlib permanently** (covered by Phase 6.7 advisories,
NOT by 3.6 drain):
- Operators, base type definitions, control flow primitives.
- Format strings (the `{x}` / `{x:j}` interpolation surface is
  shipped language behaviour).
- Core collection ops (`push`, `len`, hash insert/remove).
- The `null` sentinel and `??` operator.
- The bootstrap I/O surface needed by `01_code.loft`.
- JSON `{x:j}` format specifier + `text as Foo` cast (these
  ARE language behaviour, not library API — pulling JSON out
  breaks both).

### Phase 6t — library test self-sufficiency

**Problem.**  Audit of test coverage (2026-05-28) found 4 of 11
libraries are not independently testable post-extraction.  The gap is
two clusters of monorepo-owned Rust harnesses that drive library code
but live outside `lib/<name>/tests/`:

| Cluster | Files | Tests | Library subjects | Why Rust-side |
|---|---|---|---|---|
| Graphics gold-image regression | `tests/graphics_gold.rs` + `tests/gold/*.png` | 8 | `lib/graphics` | PNG decode + per-channel MAE tolerance compare against checked-in reference PNGs.  Pure-loft can't replicate the tolerance algorithm; encoder drift means byte-compare is brittle. |
| Multiplayer integration | `tests/multiplayer_v{2,3,5}.rs` | 10 (v2: 3, v3: 2, v5: 5) | `lib/server`, `lib/web`, `lib/game_protocol` | Subprocess orchestration to dodge @P245 (single-process `parallel{}` + I/O hangs when one arm accepts and another connects to a loopback port).  Must run client + server as separate processes. |

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

**Tier 1 — mechanical copies (XS, no design needed). DONE.**

Outcome (verified 2026-05-29): the two `tests/scripts/13X-...loft`
files were folded into `lib/gridmesh/tests/segmesh.loft` (the
crystal-equivalence + incremental-update assertions live alongside
the segmesh's own tests rather than as separate files).  The
`use audience_crystal;` block was replaced with a synthetic
`CellSnap`-shaped fixture as specified.  The originals are gone
from `tests/scripts/`.  `cd lib/gridmesh && loft test` reports
20 passed across 4 files.

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

The 10 subprocess-orchestrated tests across `multiplayer_v2.rs`
(3 tests), `multiplayer_v3.rs` (2 tests), and `multiplayer_v5.rs`
(5 tests) test the surface that `lib/server` + `lib/web` +
`lib/game_protocol` *jointly* expose — no single library owns
them.  (Inventory verified 2026-05-29; earlier drafts of this plan
named only v2 + v5 and undercounted v2's test count.)  Two ship
sites are plausible:

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
(or workspace `cargo test -p ...`) runs all 10 tests against the
checked-out `lib/server` + `lib/web` + `lib/game_protocol`;
deleting all three monorepo harnesses
(`tests/multiplayer_v{2,3,5}.rs`) leaves coverage intact.

**Order of operations.**  Tier 1 first (XS, unlocks gridmesh hygiene).
Tier 2 next (blocks Phase 5 graphics extraction).  Tier 3 last
(can land alongside Phase 6r since `loft-libs-net` is already
extracted; the integration suite is additive to that repo).

**Tier 3 blocker — cross-package `--native` link on Linux CI**
(surfaced 2026-05-31 during the `loft-libs-net` 6r/6.5 sweep, PR #2).
The omnibus first tried to lift the HTTP round-trip + WebSocket
echo tests from `lib/game_protocol/examples/` into
`server/tests/`.  Each test uses **both** `use server;` and
`use web;` in the same loft program — server to listen on a port,
web's http_get / ws_handler to drive a client arm via
`parallel { server_arm; client_arm }`.  Builds and runs locally
on macOS; fails on `ubuntu-latest` CI at the `rustc` link step
("linking with `cc` failed: exit status: 1") when both cdylibs
plus their transitive deps (ureq + rustls + ring from web,
TCP sockets from server) are pulled into one generated binary.
A server-only smoke (`listen` + `close`, no `web::`) passes —
the gate is specifically "two `#native` cdylibs from sibling
packages composed into one `loft --native test` binary."
**Filed** as @P389 in PROBLEMS.md.  Tests dropped from PR #2
(commit `c27198b`) and game_protocol-style two-process
multiplayer harnesses remain the path forward (Tier 3 above).
The gap reinforces option (a) for Tier 3 ship-site choice —
the workspace integration crate sidesteps the single-binary
limit by running clients and servers as separate processes.

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

**Tier 5 — coverage gaps that never had a Rust home (S each, NEW).**

Validation run 2026-05-29 (every monorepo library exercised under
both `loft test` and `loft --native test`) surfaced four libraries
with **inadequate regression depth** that the original Phase-6t
framing missed.  Unlike Tiers 2–3, these gaps are *not* about
migrating coverage out of a Rust harness — the coverage **never
existed**.  Closing them is the work needed to ship extracted chunk
repos with real tests instead of smoke probes.

| Library | What `lib/<name>/tests/` carries today | Coverage gap | Blocks chunk |
|---|---|---|---|
| `imaging` | `tests/14-image.loft` doc-example + `tests/15-regression.loft` (9 tests, **DONE 2026-05-29**): `Pixel.value()` packing, save/load round-trip (4×4 + 8×3 non-square + 5×5 solid + 2×2 extremes), `(x,y) → y*w+x` addressing, `save_png` failure modes (0×0 image, nonexistent dir).  10 tests total green on both gates with `LOFT_DENY_WARNINGS=1`. | — | ~~Phase 5 (`loft-libs-graphics`)~~ unblocked |
| `world` | `tests/world.loft` smoke + `tests/02-persist.loft` (15 tests, **DONE 2026-05-29**): `chunk_idx_32`/`hex_idx_32` for positive AND negative inputs, `cell_count` (empty, after-set, overwrite, clear), `neighbour_count` (isolated + 6-axial-neighbours), `world_save`/`world_load` round-trip (empty, single-cell, many-cells-across-chunks, tick-preserved-through-`tick_and_decay`, negative-coords), `world_load` failure modes (missing file → 0, wrong magic → 0, wrong version → 0).  16 tests total, both gates green, `LOFT_DENY_WARNINGS=1` clean.  The MapFile JSON schema entry points (`world::load_mapfile` / `save_mapfile`) are still future work; covered when the schema migrates from `lib/moros_map`. | — | ~~Phase 6w (`loft-libs-world`)~~ unblocked for binary-format chunk extraction; MapFile schema landing is the only remaining 7a-step-4 work for full coverage |
| `server` | `tests/server.loft` — one `srv = listen(); srv.close()` smoke | Real surface (HTTP / WebSocket / TLS / session) only exercised by `multiplayer_v{2,3,5}.rs` (Tier 3).  Once Tier 3 lands in `loft-libs-net/tests-integration/`, server is covered transitively; `lib/server/tests/` itself remains a smoke (acceptable) | Phase 6 re-clean (6r) — **waived if Tier 3 lands first** |
| `markdown` | `tests/01-render.loft` — `fn main()` driver with `must_contain` / `must_not_contain` / `must_eq` helpers and 79 grouped assertions across ~25 feature areas (html_escape, slugify, rewrite_link, ATX/setext headings, paragraphs, bold/italic/strike/code, smart underscore, nesting, backslash escapes, images, links + titles, autolinks, hr, blockquote merging + separation, fenced code, lists UL/OL/continuation, task lists, tables + alignment, HTML-comment stripping, CRLF, UTF-8, raw-HTML escaping, tracker-tag autolinks, image URL rewriting, `extract_headings`).  Re-audit **2026-05-29**: this IS the ≥30-test coverage Tier 5 was supposed to add — the original Tier 5 framing was based on counting `fn test_*` (= 0) and missed the `fn main()` driver style. | — | ~~Markdown extraction (post-6w)~~ already covered; only a cosmetic refactor-to-`fn test_*`-discovery would remain |

*Target per library:* `cd lib/<name> && loft test` reports
≥10 test functions passing for `imaging` / `world` / `markdown`.
`server` is explicitly waived because Tier 3 covers it transitively.

*Order of operations.*  `imaging` first (blocks Phase 5 — the
soonest extraction that needs it; **DONE 2026-05-29** —
`lib/imaging/tests/15-regression.loft`, 9 tests + the doc-example
= 10 total green on both gates).  `world` next (pairs with the
MAPFILE entry-point landing in Phase 7a; co-blocks 6w).
`markdown` last (independent; no extraction blocker until
markdown ships externally).

*Why this Tier was missed originally:* the 2026-05-28 audit asked
"which Rust harnesses own library coverage?" — a *migration*
question.  It did not ask "which libraries lack adequate coverage
anywhere?" — a *creation* question.  Tier 5 closes the second one.

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

**Remaining work — UNBLOCKED today (re-evaluated 2026-05-29):**

A 2026-05-28 `sed` pass marking every wall.loft / overland.loft
item `pub` was reverted because the legacy moros syntax uses `enum`
for what current loft calls `struct` (`enum Tile { material: u8,
... }` — struct-shaped field list, no variants), and the parser
rejects it as `Expect enum values to be in camel case style`.  At
the time this was framed as consumer-blocked.  Re-validation 2026-
05-29 shows the framing was wrong: `grep -rn 'use wall\|use
world::wall' lib/` returns **zero** — wall.loft and overland.loft
have **no consumers anywhere in lib/**, so the translation breaks
nothing.

**Scope correction (2026-05-29):** a translation probe found
wall.loft carries *more* legacy-syntax issues than the
`enum`-as-struct claim:

- One `enum Tile { … }` to translate (line 165 — struct-shaped).
- *Two* duplicate struct names: `WallPoint` declared at lines 250
  AND 379 (different field shapes); `Line` declared at lines 262
  AND 393.  Loft rejects the duplicates — at least one of each
  pair must be renamed (design decision: which is canonical, or
  do both survive under distinct names).
- `assert!(...)` Rust-macro syntax at line 211 etc. — translate to
  `assert(...)`.
- `if flipped ^(steps < 0)` at line 317 — `^` operator semantics
  (XOR vs cast-shape) need a per-call check.
- Likely more once these clear and the parser proceeds further.

So the translation is **mechanical at the per-edit level** but
**design-shaped at the file level** (the duplicate struct rename
is a naming decision, not a transliteration).  Since wall.loft
has zero consumers, the safest path is: rename duplicates by their
*usage neighbourhood* (e.g. `WallPoint` near `Drawing` becomes
`DrawingWallPoint`), drop unused dead code, and validate by
parsing.  Estimated effort: **S–M** (half a day, vs the README's
prior "mechanical / XS" framing).  Treated as deferred-by-scope
until either Phase 7b moros migration unsticks (and the consumer
specifies which structs it needs) or a separate clean-up sub-phase
is filed.

The remaining mechanical / additive work below is still doable
today without any external-consumer coordination:

- Translate `enum`-as-struct declarations in wall.loft / overland.loft
  to current `struct` syntax (mechanical syntax migration; zero
  consumers to break).
- Mark items `pub` after translation.
- Wire `use wall;` / `use overland;` (or fold content directly) into
  `world.loft`.
- Move hex / chunk types + geometry collision **additively** from
  `moros_map` / `moros_sim/collide.loft` into `lib/world/` per
  MAPFILE.md migration plan steps 1–3 (leave the moros_map originals
  intact — non-breaking; consumers keep using their own types until
  ready).
- Add `world::load_mapfile()` / `world::save_mapfile()` entry points
  (MAPFILE.md step 4) — additive public API, no consumer surface.
- Add `pub use world::*;` shim in `moros_map` (MAPFILE.md step 5) —
  compatibility shim, moros_map's API to its callers unchanged.

Together these are five of MAPFILE.md's six migration steps and
constitute **the bulk of Phase 7a**.

**Remaining work — genuinely consumer-blocked (one item):**

- Migrate `lib/moros_*` to `use world;` (MAPFILE.md step 6) — this
  *replaces* `moros_map`'s internal `Hex` / `Chunk` types with
  `world::*` and changes consumer call sites.  Paired with the
  external moros project's migration; not done in advance.

The consumer stall affects step 6 only.  Steps 1–5 are unblocked.
**6w (`loft-libs-world` chunk extraction) depends on steps 1–5 but
NOT on step 6** — the chunk can ship at `world 0.1.0` carrying the
additive types + load/save API while moros continues to use its
own internal copies, then upgrade to `world 0.2.0` when the
consumer stall lifts.

**Coverage prerequisite (links to Phase 6t Tier 5):** `lib/world`
currently has only the smoke test.  Before 6w extracts, Tier 5
adds save/load round-trip + MapFile schema tests against
`world::load_mapfile` / `world::save_mapfile` (added in step 4
above) + sparse-write boundary tests.  Coverage growth pairs
naturally with the API growth.

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
