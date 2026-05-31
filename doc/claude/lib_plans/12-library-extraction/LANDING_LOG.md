<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — Landing log (chronological)

Chronological record of every commit that landed a piece of
@PLAN12.  Append a new section as each phase ships; do NOT
backfill or reorder.  This file is the raw timeline; the
[README](README.md) carries the curated status table; the
topical detail files carry the design narrative.

When plan-12 closes (Phase 6.13), this log becomes the
`finished/12-library-extraction/LANDING_LOG.md` as-is — no
rewrite, no condensation.

---

## 2026-05-24 — Phases 1, 2, 3, 3.5a, 4 — initial extraction wave

Foundational work to land `loft-libs-core` (arguments, random,
crypto) end-to-end:

- Phase 1: drain library symbols from `src/native.rs`.
- Phase 2: compile-time native-registry aggregator
  (`[native.functions]` manifest + `loft-ffi-build`).
- Phase 3: PKG.REG + cdylib loader (code complete, bootstrap
  `K_tmp`).
- Phase 3.5a: dry-run lib without monorepo consumers (crypto).
- Phase 4 Stage A: `loft-libs-core` (arguments, random, crypto)
  shipped to registry.

## 2026-05-24 — Phase 6 Stage A — loft-libs-net

- Extract `loft-libs-net` (server, web, game_protocol) to its
  chunk repo.  0.1.0 published.

## 2026-05-24 — Phase 5 Stage A (partial) — loft-libs-graphics

- shapes 0.2.0 + gridmesh 0.1.0 published.  graphics + imaging
  remain in monorepo (codegen-blocked at the time).

## 2026-05-28 — Phase 4 Stage B — loft-libs-core monorepo cleanup

- `lib/arguments/`, `lib/random/`, `lib/crypto/` removed from
  monorepo.  First chunk to fully close A+B.

## 2026-05-30 — Phase 6.5 first chunk — loft-libs-core canonical CI

- `loft-libs-core` PR #2 applied the canonical
  `library-ci.yml` template (--lib --bin loft, Cargo.lock-keyed
  cache, LOFT_DENY_WARNINGS=1).  Phase 6r per-symbol re-clean
  + warning sweep bundled into the same omnibus.
- Surfaced @P385 + @P386 in jjstwerff/loft (parser + native
  codegen bugs); fixed before the chunk PR could land green.

## 2026-05-31 — Phase 6r/6.5 second chunk — loft-libs-net v0.1.1

- Net Phase 6r + 6.5 omnibus (PR #2): canonical YAML + tcp_*
  bare-#native + warning sweep + new `byte_at.loft` tests.
- Version-bump PR #3 → 3 tags (web/server/game_protocol-v0.1.1)
  → 3 GitHub releases → registry PR #5 (all 3 validator gates
  green).
- Discovered + fixed two infrastructure prerequisites:
  - `loft package` deterministic mtime (jjstwerff/loft #234).
  - Registry validator multi-package homepage parsing
    (loft-lang/registry #4).
- Filed @P389: cross-package `--native` link failure on Linux
  CI when a single `loft --native test` binary links two
  chunk-resident cdylibs + their transitive deps.

## 2026-05-31 — Phase 6.5 third chunk — loft-libs-graphics gridmesh 0.1.1

- Graphics Phase 6.5 omnibus (PR #1): canonical YAML +
  gridmesh re-sync from cleaned monorepo source (`_x` rename +
  `buckets not null`).
- gridmesh-v0.1.1 → registry PR #6 (validator gates green
  after a transient mold-install hang in the first run).
- Path-to-green for the third chunk: ~30 minutes (vs the ~1
  work-day budgeted), because the third chunk surfaced no new
  chunk-specific work — confirms the pattern is stable.

## 2026-05-31 — Phase 6.6 — auto-install on `use`

Four commits on the `libraries7` branch:

- **`449d8eb`** — auto-install on `use`.  Parser auto-installs
  from registry on cold cache when an unresolved `use X;`
  matches a name in the signed index.  Off-switches:
  `LOFT_OFFLINE=1`, `LOFT_NO_AUTO_INSTALL=1`.
- **`5702633`** — `loft pin <script>` sidecar.  Writes
  `<script>.loft.lock` next to a single-file script so
  subsequent runs use the pinned versions regardless of cwd.
- **`d0fb2f9`** — walk-up `loft.toml` detection.  Project-mode
  resolution: from the script's directory, walks up to `/`
  looking for `loft.toml`; uses that dir's lockfile + writes
  there.  Cwd becomes irrelevant.
- **`d949379`** — `loft list-installed` query helper.
  Enumerates `~/.loft/registry/<pkg>-<ver>/` with sha256 + size
  + index status.

## 2026-05-31 — Phase 6.7 — security advisory channel (loft-binary side)

Four chunks + a default-policy revision:

- **`7b79b7b`** — `src/registry_advisories.rs` data model +
  classifier (`parse_advisories` + `classify`).  Severity enum
  (security_critical / security_high / security_low / bug /
  deprecated) + 8 unit tests.
- **`b4479a3`** — TTL-aware loader + signature verification.
  24h TTL constant; `load_or_fetch` with offline-respecting
  cache + soft-fail for unhosted feed (HTTP 404).
- **`37bf154`** — `loft audit` CLI.  Walks
  `~/.loft/registry/` + classifies each install vs the feed;
  exit code reflects worst severity (1/2/3 + 4 for feed-load
  errors).
- **`46ffbba`** — runtime classifier hook in
  `parser/mod.rs::check_advisory`.  Per-process dedupe via
  `advisory_checked` set; lazy `OnceLock` feed loader (cache-
  only, never refetches mid-parse).
- **`6f9e94e`** — policy revision: critical default switched
  from "refuse to run" to "loud warning, run anyway".  Strict
  mode is opt-in via `LOFT_STRICT_SECURITY=1` (env) or
  `--strict-security` (CLI).  Reason: security fixes can
  introduce breaking changes; the user often needs to run
  their cached vulnerable version while porting.

## 2026-05-31 — Phase 6.8 — `loft update` command

- **`12387a3`** — `loft update [<pkg>] [--dry-run] [--check]`.
  Walks the project's loft.lock (walk-up from cwd to find it),
  cross-references each entry against the registry index +
  the project's `loft.toml` range, installs the highest active
  in-range version.  Yanked versions automatically skipped via
  `find_best_version`.  CI gate: `--check` exits 1 on any
  available update.

## 2026-05-31 — Phase 6.11 — offline bundle support

- **`d736c10`** — `loft bundle export` / `loft bundle import`
  + `LOFT_REGISTRY_URL=file://` resolution.  Export collects
  index + advisories + tarballs + manifest into a directory;
  import verifies sha256 per tarball + extracts to
  `~/.loft/registry/`; `http_get_bytes` gained a `file://`
  branch so local mirrors work as drop-in registry URLs.
  Air-gap deployment loop verified end-to-end with an
  isolated `LOFT_HOME` simulating the air-gap target.

## 2026-05-31 — Phase 6b attempt — rolled back

Attempted the "in-monorepo move-then-delete" refactor (Option D
from the 6b sequencing discussion): move
`lib/game_protocol/examples/*.loft` →
`tests/integration/multiplayer/`, move `lib/server/tests/server.loft`
→ `tests/integration/p244_smoke.loft`, add a
`tests/integration/loft.toml` + lockfile pinning net packages,
update `multiplayer_v{2,3,5}.rs` + `codegen_emitter.rs` p244
paths.

The move + Rust harness updates + lockfile generation all worked.
Multiplayer test started executing against registry-resolved
net packages.  TWO friction points surfaced:

1. **Sandbox / cargo write permissions** — `auto_build_native`
   wants to write to `~/.loft/registry/<pkg>-<ver>/native/target/`
   to build the cdylib on demand.  Restricted environments
   (sandbox, CI runners with locked-down user dirs) refuse the
   write with "Operation not permitted" and the build silently
   fails; the parser then panics at first call to a native fn
   with "native function not loaded — call extensions::load_all()
   first".
2. **`ring` macOS SDK path** — building `web`'s native crate
   from the registry copy invokes `ring`'s C build; the env
   cargo gets handed when invoked from `~/.loft/registry/`
   misses the macOS SDK's `TargetConditionals.h` lookup
   (works fine from the monorepo's `lib/web/native/` workspace
   context because the workspace's `.cargo/config.toml` +
   parent SDK env are inherited).

Both fire ONLY when the native crate is at the registry path,
not when it's at `lib/<pkg>/native/`.  Real blocker for 6b
shipping "just works" — `loft install web` + `use web;` from
a fresh cache needs to reliably build web's cdylib.

Rolled back the move; tree restored; multiplayer test green
again with the original layout.  6b status flipped to BLOCKED;
plan-12 README updated with the friction notes.

Next step: harden `auto_build_native` against shared-cache /
restricted-write environments + investigate the `ring`
macOS-SDK path issue (likely a `cargo build` env var
inheritance problem when invoked outside a workspace).  Lands
as a separate plan-12 sub-phase OR a fix into `extensions.rs`
+ rerun 6b.

## 2026-05-31 — Phase 6.15 — library catalog generator

- `scripts/gen_library_catalog.py` — Python script that reads
  `index.json` (live HTTPS / file:// / local path) and emits
  `doc/library-catalog.md`: a categorised table of every
  published library with one-liner description + latest active
  version + link to homepage.
- `--check` mode for CI drift detection (exit 1 if the
  committed file differs from the regenerated output).
- Verified against the live registry + the mock-registry
  fixture from Phase 6.12.
- 8 active packages across 6 categories at generation time
  (cli/crypto/geometry/graphics/math/net).
- Registry-side CI wire-up + polished HTML view at
  `loft-lang.org/libraries` are follow-ups.

## 2026-05-31 — Phase 6.14 — library docs pipeline (CLI + CI template)

- `loft doc <path>` CLI was already wired as PKG.8 — reads
  `<pkg>/loft.toml` + `<pkg>/src/*.loft` (+ optional
  `<pkg>/docs/*.loft`) and emits HTML to `<pkg>/doc/`.
  Verified end-to-end against the gridmesh fixture
  (`index.html` + `api-general.html`).
- `library-ci.yml.example` gained two new steps:
  - "Generate per-package docs" — always runs; collects
    `<pkg>/doc/` as a CI artifact.
  - "Publish docs to gh-pages (release only)" — fires on
    `<pkg>-v*` tags; pushes to `<chunk>.github.io/<pkg>/<ver>/`.
- Per-chunk rollout (applying the new YAML to each chunk repo)
  lands incrementally.  Cross-package link generation +
  `<pkg>/latest/` symlinks are follow-ups.

## 2026-05-31 — Phase 6.13 — landing-log seed

- `LANDING_LOG.md` (this file) created and back-populated with
  every shipped phase through Phase 6.12.  Phase 6.13 now
  IN PROGRESS — incremental append per phase.  Permanent-doc
  harvest + user-facing onboarding remain closure-time work.

## 2026-05-31 — Phase 6.12 — loft-dev offline test loop (scaffolding)

- **`bc32723`** — `scripts/sync-fixtures.sh` +
  `tests/fixtures/libs/` (arguments, shapes, gridmesh,
  game_protocol — pure-loft only) + `tests/fixtures/mock-registry/`
  (synthetic index.json + advisories.json) + 4 mock_registry
  tests + 2 doc-hygiene gates (lib_fixtures_have_loft_toml +
  mock_registry_fixtures_are_valid).  Native-cdylib fixtures
  defer to their respective Stage B work.

---

## Pending phases (in roughly the order they'd land)

These will get an entry here when they ship:

- Phase 5b — `loft-libs-graphics` Stage B (extract graphics +
  imaging Stage A first; then remove monorepo lib dirs).
- Phase 6b — `loft-libs-net` Stage B (remove monorepo
  lib/{web,server,game_protocol}/).
- Phase 6.7a — author-side yank workflow (`loft yank` CLI +
  validator cross-ref gate).
- Phase 6.16 — `loft publish` CLI.
- Phase 6w-w — retire every `.allow_warnings`.
- Phase 6t Tier 3 — multiplayer harness port.
- Phase 6t Tier 5 — remaining coverage gaps.
- Phase 6w — extract `loft-libs-world`.
- Phase 7a/p/b/c — moros split + cross-project consumers.
- Phase 8 — final monorepo cleanup.
- Phase 6.13 — documentation harvest + close-out (the
  closure ritual itself; this file becomes the finished plan's
  landing log).

Append new sections above this list as phases ship; remove
items from this list as they land.
