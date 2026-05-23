<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library extraction — `lib/*/` → external repos

Move the `lib/*/` packages currently inside the main loft
repository out into per-family external GitHub repositories,
each consumable via the package registry.

This is the **execution arc** for **PKG.EXTRACT** in
[ROADMAP.md](../../../ROADMAP.md).  The infrastructure work
(package registry MVP, lock file, format extensions) lives
in the sibling plan
[PACKAGES.md § Open work](../../../PACKAGES.md#open-work).  This plan picks up
once the infrastructure ships.

## Status

**Blocked** on PKG.REG (central registry MVP) landing in
[PACKAGES.md § Open work](../../../PACKAGES.md#open-work).  Until `loft install
<name>` works against a registry, there's no consumption
path for an extracted library.

When unblocked: per-library extraction proceeds on its own
validated schedule.  Some libraries may extract early
(stable, low-churn); others stay in the monorepo until
their API matures.

## Why a separate plan from PACKAGES.md infrastructure work

PACKAGES.md § Open work = INFRASTRUCTURE (registry, lock
file, format).  This plan = EXECUTION (which library
extracts when, how to migrate downstream consumers,
version-sync policy).

Different lifecycles:
- PACKAGES.md infrastructure work targets one focused arc
  (likely 0.8.6).
- This plan spans multiple releases — each library extracted
  on its own validated schedule.

Different acceptance criteria:
- Infrastructure work: "`loft install <name>` works,
  `loft.lock` honored, signing verifies."
- This plan: per-library — "lib/<X>/ removed from monorepo,
  `loft install <X>` from external repo produces identical
  behaviour, downstream consumers (other libraries, test
  scripts, examples) migrated."

Different audiences:
- PACKAGES.md readers care about how the registry / format
  works.
- This plan readers care about whether their favorite
  library is going to break or move.

## Current inventory — extraction candidates

The `lib/*/` packages with `loft.toml` (i.e., already using
the package format and ready in principle to extract once
the registry exists):

| Library | Notes | Likely extraction priority |
|---|---|---|
| `lib/arguments/` | CLI argument parsing | Early — small, stable, low-churn |
| `lib/crypto/` | Cryptographic primitives | Early — bounded scope |
| `lib/random/` | RNG | Early — bounded scope |
| `lib/shapes/` | Geometric primitives | Early |
| `lib/imaging/` | Image manipulation | Mid — used by moros_*, validate dependency chain |
| `lib/graphics/` | OpenGL / 2D drawing | Mid — large, used by demos; coordinate with [`../02-graphics/`](../02-graphics/) plan |
| `lib/server/` | HTTP server | Mid — coordinate with [`../08-server/`](../08-server/) plan; depends on game-loop additions |
| `lib/web/` | Web utilities | Mid |
| `lib/game_protocol/` | Multiplayer protocol | Mid — coordinate with EVENT_LOOP / multiplayer-editor / tic-tac-toe plans |
| `lib/moros_editor/` | Moros editor | Late — large, mid-development |
| `lib/moros_map/` | Moros map data | Late — paired with editor |
| `lib/moros_render/` | Moros rendering | Late — paired with editor |
| `lib/moros_sim/` | Moros simulation | Late — paired with editor |
| `lib/moros_ui/` | Moros UI | Late — paired with editor |

Single-file `.loft` modules at `lib/*.loft` (`code.loft`,
`docs.loft`, `lexer.loft`, `logger.loft`, `overland.loft`,
`parser.loft`, `testlib.loft`, `wall.loft`) are NOT package-
format adopters yet.  Either keep as in-tree single-file
modules (no extraction), or migrate them to package format
first (own decision per file).

## Chunk grouping — a few repos, not 17

One GitHub repo per library = 17 repos to track, release, and CI separately.
To avoid that sprawl the libraries extract in **chunks**: a small number of
multi-package repos, each holding a related FAMILY that versions + releases
together under one CI workflow.  Each chunk is a workspace of `loft.toml`
packages (published to the registry per-package, maintained as one repo).

Proposed chunks (refine before the first extraction):

| Chunk repo | Packages | Rationale |
|---|---|---|
| `loft-libs-core` | `arguments`, `random`, `crypto`, `shapes` | Small, stable, no graphics deps — extract first |
| `loft-libs-graphics` | `graphics`, `imaging`, `gridmesh` | Graphics stack + `#native` crates; coordinate with [`../02-graphics/`](../02-graphics/) |
| `loft-libs-net` | `server`, `web`, `game_protocol` | HTTP / multiplayer; coordinate with [`../08-server/`](../08-server/) |
| `loft-moros` | `moros_editor`, `moros_map`, `moros_render`, `moros_sim`, `moros_ui` | The moros RPG stack; depends on the graphics chunk; extract last (mid-development) |

`lib/audience_crystal/` (the projector's crystal mesh-gen prototype that
`gridmesh` was extracted from) is NOT an extraction candidate — it stays
in-monorepo with the audience demo.  It also has no package `tests/` dir, so it
is the one packaged lib NOT covered by the library gates below; its behaviour is
gated via the core `tests/scripts/130|133|135` cross-mode equivalence tests.
Adding package tests (so it joins the gate) is the small hardening follow-up.

A chunk extracts as a unit (one `git filter-repo` / `subtree split` of its
`lib/*` dirs), keeping intra-chunk deps as path deps and cross-chunk deps (e.g.
moros → graphics) as registry deps.  The
[extraction template](#per-library-extraction-template) applies per-chunk
(step 6 deletes the whole chunk's `lib/*` dirs in one PR).

## CI path for libraries (built 2026-05-23) — travels with each chunk

The libraries now have a self-contained CI path in the monorepo that ports
unchanged to each extracted chunk (it keys off `lib/<pkg>/tests/*.loft` +
`loft.toml`, nothing monorepo-specific):

| Gate | Where | Covers |
|---|---|---|
| Interpreter | `tests/wrap.rs::library_suite` | every `lib/*/tests/*.loft` via `loft test` (subprocess, package-resolved); skips via `lib_test_skipped` (`LIB_PKGS_SKIP` / `LIB_TESTS_SKIP`) |
| Native | `tests/native.rs::native_library_suite` | the same via `loft --native test` (compiles each to native Rust, linking the package's `#native` crate); skips `LIB_PKGS_NATIVE_SKIP` / `LIB_TESTS_NATIVE_SKIP` (native-codegen gaps, [@P321](../../../PROBLEMS.md)) |
| Leak | `tests/wrap.rs` `run_test` gate | unfreed stores at program exit fail; allowlist `SCRIPTS_LEAK_ALLOW` ([@P322](../../../PROBLEMS.md)) |
| Quick dev loop | `make test-packages` | interpreter-only shell loop over every package test (dev-only, in `ci-full`; the cargo suites above are the gates) |

When a chunk extracts, its repo CI runs the equivalent (`loft test` +
`loft --native test` over the chunk's packages) and the skip-lists travel with
the code as the chunk's own `*_NATIVE_SKIP` / leak allowlist.  This is the
"clear CI path for itself" the libraries needed before living outside the
monorepo (Open question #4 below).

## Per-library extraction template

Each extraction is its own focused commit (or small commit
chain).  Template:

1. Verify the library has `loft.toml` and passes its own
   tests in-tree (`cd lib/<X> && cargo test` or `loft
   tests/...`).
2. Create the external GitHub repo with the same name
   (`loft-<X>` convention, or just `<X>` under the loft-lang
   org).
3. Push library content to the external repo, preserving
   git history via `git filter-repo` or `git subtree split`.
4. Tag a v0.1.0 release in the external repo.
5. Publish to the package registry: `cd <external-repo> &&
   loft publish`.
6. Remove `lib/<X>/` from the monorepo in a single PR:
   - Add `loft.toml` dependency on `<X> = "0.1.0"` in every
     monorepo consumer.
   - Update consumer `use` statements (typically no change
     if package name matches).
   - Run full test suite to confirm `loft install <X>` +
     existing import path produces identical behaviour.
   - Delete `lib/<X>/`.
7. Document the extraction in CHANGELOG.md.
8. Subsequent updates land in the external repo; consumers
   bump version in their `loft.toml`.

## Open questions

These need decisions before the first extraction starts.
Listed here so future-you doesn't have to re-discover them.

1. **Naming convention.** `loft-<X>` (under loft-lang org)
   or `<X>` (org-namespaced)?  Affects `loft install` UX.
2. **Version policy.** Per-library independent semver, or
   monorepo-style coordinated bumps?  Independent semver
   matches package-registry idiom.
3. **Tagging.** Are external repo tags `v0.1.0` style or
   `0.1.0` (matching loft.toml syntax)?  Rust uses `v`
   prefix; npm uses bare.
4. **Test infrastructure.** RESOLVED (2026-05-23) — the
   monorepo CI path (see [§ CI path for libraries](#ci-path-for-libraries-built-2026-05-23--travels-with-each-chunk))
   is the template: a chunk repo runs `loft test` (interp) +
   `loft --native test` (native) over its packages, carrying
   its own `*_NATIVE_SKIP` / leak allowlist.  Still TODO: the
   reusable GitHub Actions workflow YAML (build loft, then run
   the two gates) — write once, copy per chunk.
5. **Backwards-compatibility window.** When `lib/<X>/`
   leaves the monorepo, existing `use lib_<X>` (or whatever
   the current import shape is) should KEEP WORKING via
   the registry-installed copy for at least one release.
6. **Documentation home.** Per-library README.md migrates
   to the external repo.  CLAUDE.md doc index entry stays?
   Update to point at the external repo URL?
7. **Transitive deps.** If `lib/moros_render/` depends on
   `lib/graphics/`, does graphics extract first, or do they
   extract together?  Likely answer: stable libs extract
   first (graphics before moros_render), but moros_render's
   monorepo `loft.toml` updates to depend on the external
   graphics package as part of the graphics-extraction
   commit, not moros_render's later one.
8. **Cross-library breaking changes.** When an extracted
   library evolves and an in-monorepo consumer needs
   updates, who tracks the migration?

## See also

- [PACKAGES.md § Open work](../../../PACKAGES.md#open-work) — package registry +
  format infrastructure (PREREQUISITE)
- [`../../../PACKAGES.md`](../../../PACKAGES.md) — package
  format reference
- Sibling library plans whose libraries appear in the
  inventory above:
  - [`../02-graphics/`](../02-graphics/) — graphics library
  - [`../05-game-infra/`](../05-game-infra/) — game infra
  - [`../08-server/`](../08-server/) — server library
  - [`../10-game-client/`](../10-game-client/) — game client
- [`../../../ROADMAP.md`](../../../ROADMAP.md) — milestone
  placement (PKG.EXTRACT scheduled 1.1+)
