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
4. **Test infrastructure.** Each extracted library needs CI
   on its own repo (basic `cargo test` or `loft test`).
   Template GitHub Actions workflow lives where?
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
