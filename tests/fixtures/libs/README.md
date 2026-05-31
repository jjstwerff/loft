<\!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<\!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# `tests/fixtures/libs/` — bundled snapshots of extracted libraries

@PLAN12 Phase 6.12.  Each `<pkg>/` subdirectory is a snapshot of
an extracted library at a pinned chunk-repo tag.  Snapshots are
the source of truth for loft compiler tests that need library code
to compile / parse against.

## Why fixtures, not registry-cached installs

- **Zero network during `cargo test`** once committed.  Critical for
  CI runners with locked-down outbound networking, for the loft
  contributor on a plane, for fresh laptops straight from
  `git clone`.
- **Reproducible across machines.**  Every contributor's compiler
  tests run against the same library bytes.
- **Survives Stage B's `lib/<pkg>/` removal.**  After Stage B for a
  given chunk, monorepo `lib/<pkg>/` is gone; tests that previously
  read from there would break without this fixture.
- **Pinned tags committed.**  A library version change is a
  deliberate, reviewable commit (refresh + diff), not silent drift.

## Refresh ritual

Update `scripts/sync-fixtures.sh`'s `PINNED_REFS` table when a
chunk ships a new version the compiler tests should track, then:

```
scripts/sync-fixtures.sh           # refresh all fixtures
scripts/sync-fixtures.sh --check   # CI gate: drift detection
```

CI doc-hygiene runs the `--check` mode.  Drift → fail.

## Current population

Pure-loft packages only (2026-05-31).  Native-cdylib packages
(crypto / web / server / imaging) keep their canonical source in
the monorepo `lib/<pkg>/` until Stage B for their chunk runs;
THEN their snapshots land here.

Run `scripts/sync-fixtures.sh` once to populate from the
canonical chunk repos.

## Layout

```
tests/fixtures/libs/
├── README.md                    (this file)
├── arguments/                   (from loft-libs-core)
├── random/                      (from loft-libs-core)
├── shapes/                      (from loft-libs-graphics)
├── gridmesh/                    (from loft-libs-graphics)
└── game_protocol/               (from loft-libs-net)
```

## Companion: `tests/fixtures/mock-registry/`

A minimal `index.json` + `advisories.json` fixture for testing
the registry-resolution code paths offline.  See its README.
