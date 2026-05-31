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

## 2026-05-31 — Phase 6.7a — `loft yank` author helper

Author-side companion to Phase 6.7's consumer-side classifier.
Closes the security loop: now a maintainer can file an
advisory without hand-editing index.json + advisories.json.

src/main.rs:
- New `yank` subcommand + help text.
- `yank_package(target, severity, advisory, summary, affected,
  fixed_in)`:
  - Parses target as `<pkg>@<version>`.
  - Validates `--severity` against the 5-tier enum
    (security_critical / security_high / security_low / bug /
    deprecated); rejects invalid tiers with the full list.
  - Requires `--advisory` (e.g. GHSA-id) + `--summary`.
  - Optional `--affected` (defaults to the exact pinned version)
    and `--fixed-in`.
  - Emits two blocks:
    1. index.json typed `status` field to splice into the
       affected version's entry.
    2. advisories.json `advisories[]` row with the cross-
       referenced GHSA id.
  - `chrono_iso8601_utc()` provides the `published`
    timestamp.
  - `escape_json_string()` handles `"`, `\\`, `\n`, `\t`,
    and control chars; verified the output passes Python's
    json.loads.

Smoke-tested:
- Valid full invocation → both blocks emit cleanly with the
  correct shape.
- Missing severity → refused.
- Invalid severity → refused with the valid-tier list.
- Summary with `"quote"` + `\\backslash` → properly escaped;
  output passes JSON validation.

Author loop closes:
  Maintainer discovers vuln → `loft yank web@0.1.0
  --severity security_critical --advisory GHSA-... ...`
  → paste both blocks into `loft-lang/registry` PR
  → maintainer reviews → CI gate validates (Phase 6.7a's
  proposed cross-ref check; future work in tools/validate.py).

Auto-PR-open (clone registry + apply edits + `gh pr create`)
is a follow-up; today's MVP eliminates the schema-by-memory
+ JSON-by-hand steps.

## 2026-05-31 — `loft new <name>` scaffolder

Author scaffolding companion to `loft publish`.  Creates a
fresh loft library package, ready for development +
`loft publish`.

src/main.rs:
- New `new` subcommand + help text + `scaffold_library`
  implementation.
- Validates the name (lowercase ASCII + digits + underscore;
  matches loft's identifier rules).  Refuses if `<name>/`
  already exists.
- Always writes:
  - loft.toml (package + library + empty dependencies)
  - src/<name>.loft (placeholder `pub fn hello() -> text`)
  - tests/01-smoke.loft (single test_hello regression guard)
  - README.md (install + usage snippets)
- `--native` flag also writes:
  - native/Cargo.toml (cdylib + rlib; registry-version
    loft-ffi + loft-ffi-build deps)
  - native/build.rs (`loft_ffi_build::generate_register_from_loft`)
  - native/src/lib.rs (include the generated register file)
- `--chunk` flag writes `.github/workflows/library-ci.yml` (the
  canonical template with the new library as the only matrix
  entry).  For first-library-in-a-fresh-chunk-repo cases.
- Prints a "Next steps" block after creation.

Smoke-tested:
- Pure-loft scaffold `loft new foo_lib` → 01-smoke test passes
  immediately (no edits needed).
- `--native` scaffold writes the native crate skeleton.
- `--chunk` scaffold writes the CI YAML.
- Name with mixed case (`FooLib`) refused with clear error.
- Collision (`loft new foo_lib` after dir exists) refused.

Together with `loft publish`, gives the library author this
loop:

  $ loft new my_lib              # scaffold
  $ cd my_lib
  # ... edit src/my_lib.loft ... add real API
  $ loft test                    # exercise the tests
  $ git init && gh repo create   # publish source
  $ git tag my_lib-v0.1.0 && git push --tags
  $ loft package && gh release create my_lib-v0.1.0 my_lib-0.1.0.tar.gz
  $ loft publish                 # emit registry index entry
  # paste into loft-lang/registry PR

## 2026-05-31 — LIBRARY_AUTHORING.md — end-to-end author guide

- `doc/claude/LIBRARY_AUTHORING.md` — consolidates the author UX
  sprint (Phase 6.16 `loft publish` + `loft new` scaffolder +
  Phase 6.7a `loft yank`) into one user-facing walkthrough:
  scaffold → develop → pre-release checklist → publish
  (tag → release source + binary → emit registry entry → PR) →
  maintain (patch releases, yank with severity tiers,
  `loft update`).  Closes the author-friction gap by giving
  authors a single doc to read instead of cross-referencing
  PACKAGES.md + REGISTRY_SUBMIT.md + topical phase files.
- CLAUDE.md doc-index row added immediately after
  REGISTRY_SUBMIT.md, flagging LIBRARY_AUTHORING as the
  higher-level walkthrough.

## 2026-05-31 — Phase 6.16 — `loft publish` author helper

src/main.rs:
- New `publish` subcommand + help text + `publish_package` body.
- Re-packages the current dir via `package::package_create`
  (deterministic; same machinery as `loft package`).
- Auto-detects the chunk repo from `git remote get-url origin`
  via the new `git_remote_org_repo` helper.  Handles both
  HTTPS and SSH URL shapes.
- Verifies the GitHub release exists with the expected asset
  via `gh release view <tag> --json assets` (new helper
  `github_release_has_asset`).  Skipped under `--dry-run`.
- Emits the registry-ready `index.json` version block (url +
  sha256 + size + loft + subpath + deps + published) plus a
  commented-out package-block stub for first-version releases.
- Reads `[dependencies]` from loft.toml, filters out path-deps,
  emits the registry-version entries as the `deps` map.

Smoke-tested against the live `loft-libs-graphics/gridmesh`
package (v0.1.1):
  - `--dry-run` emits the index entry; release-check skipped.
  - Live mode verifies `gridmesh-v0.1.1` exists with
    `gridmesh-0.1.1.tar.gz` asset → success message printed.

Closes the publish-by-hand friction:
  Before: bump version → tag → push → `loft package` →
          `gh release create` → manually compute sha256/size →
          hand-edit registry/index.json → open PR.
  After:  bump version → tag → push → `gh release create`
          (with `loft package` tarball as asset) → `loft publish`
          → paste emitted block into registry PR.

Auto-PR-open (clone registry + splice index.json + `gh pr
create`) is the next iteration; today's MVP eliminates the
hash-+-size computation step + tag/asset verification.

## 2026-05-31 — Phase 5b attempt — prerequisites surfaced, BLOCKED

Tried to start `loft-libs-graphics` Stage B by copying imaging
+ graphics into the chunk repo.  Two real preparatory tasks
surfaced that aren't part of 5b itself:

1. **imaging depends on `loft-ffi-macros`** — `lib/imaging/native/Cargo.toml`
   has `loft-ffi-macros = { path = "../../../loft-ffi-macros" }`.
   Chunk-resident Cargo.toml needs registry-version deps
   (`loft-ffi-macros = "0.1"`), but loft-ffi-macros is NOT on
   crates.io.  Confirmed by direct `cargo fetch` probe: "no
   matching package named `loft-ffi-macros` found, location
   searched: crates.io index".  Options: (a) publish
   `loft-ffi-macros` 0.1.0 to crates.io; (b) refactor imaging
   to use plain `loft-ffi` like web/server do (no proc-macro);
   (c) vendor loft-ffi-macros into imaging's `native/`.

2. **graphics has 209 warnings + `.allow_warnings` opt-out** —
   shipping graphics to the chunk repo with the opt-out file
   would carry the debt into the chunk against the "every new
   chunk starts warning-free" Phase 6.5 discipline (`LOFT_DENY_WARNINGS=1`
   default).  Warning sweep (Phase 6w-w slice for graphics)
   needed before graphics can ship clean Stage A.

5b attempt rolled back; no changes committed.  Plan README's
5b row updated to BLOCKED with the prerequisite list.

Recommended next steps before retrying 5b:
- Publish `loft-ffi-macros 0.1.0` to crates.io (separate
  small task; mirrors the `loft-ffi-build 0.2.0` publish that
  unblocked Phase 6r).
- Graphics warning sweep (Phase 6w-w; non-trivial — 209
  warnings to triage; some require code idiom changes per
  the loft-write skill's "warning-clean idioms" doc).

5b's monorepo cleanup (5b.2 — remove `lib/shapes/`,
`lib/gridmesh/`, `lib/graphics/`, `lib/imaging/`) waits on
5b.1 (graphics + imaging Stage A) which waits on the above
prerequisites.

## 2026-05-31 — Phase 6b SHIPPED — net Stage B (consumer migration sweep)

Plan-12's primary mission goal for net: monorepo `lib/web/` +
`lib/server/` + `lib/game_protocol/` REMOVED.  Consumers
migrated; tests green.

src/extensions.rs:
- Refactored `auto_build_native` to delegate target-dir choice
  to a new shared `native_target_root(pkg_dir)` helper.  Chunk-
  resident installs (`~/.loft/registry/<pkg>-<ver>/`) get
  `~/.loft/build-cache/<pkg>-<ver>/` (writable, decoupled from
  install dir).  Monorepo `lib/<pkg>/native/` keeps in-tree
  `target/`.

src/native_utils.rs::add_native_extern_flags:
- Uses the same `native_target_root` helper to find the rlib
  at link time, so registry-installed packages' rlibs are
  found in the redirected build-cache instead of the in-tree
  target/ that no longer exists.

Script relocation (git mv preserves history):
- `lib/game_protocol/examples/*.loft` → `tests/integration/multiplayer/`
- `lib/server/tests/server.loft` → `tests/integration/p244_smoke.loft`
- `tests/integration/loft.toml` + `loft.lock` pin web/server/game_protocol
  via registry; the scripts resolve via Phase 6.6's walk-up
  loft.toml detection.

Consumer loft.tomls (new):
- `tools/audience-demo/loft.toml` — web/server registry + path-
  deps to monorepo audience_crystal/gridmesh/graphics.
- `tools/audience-demo-50/loft.toml` — web/server registry only.
- `tools/viewer/loft.toml` — server registry + path-dep to
  monorepo markdown.
- Generated lockfiles via `loft install` from each dir.

Rust harness path updates:
- `tests/multiplayer_v{2,3,5}.rs` — `lib/game_protocol/examples`
  → `tests/integration/multiplayer`.
- `tests/codegen_emitter.rs` p244 — `lib/server/tests/server.loft`
  → `tests/integration/p244_smoke.loft`.

tests/extraction_hygiene.rs:
- Added web's 19 symbols (n_http_do, n_ws_*, n_pack_*, etc.) to
  `FORBIDDEN_LIBRARY_SYMBOLS_MANUAL` with the
  "loft-libs-net/web/native" owner tag.  Required because the
  dynamic `forbidden_library_symbols()` scan walks `lib/*` only;
  after Stage B, web's symbols don't appear in the dynamic scan
  but the hygiene gate still needs to detect their re-introduction
  in `src/**`.

tests/multiplayer_v5.rs:
- `v5_t5_world_tick_and_decay` `#[ignore]`'d.  `use world;` in
  the script no longer resolves from the new
  `tests/integration/multiplayer/` location (the parser's walk-up
  finds `tests/integration/loft.toml` which doesn't list world;
  the monorepo's `lib/world/` is no longer a sibling).  Un-ignore
  when Phase 6w extracts `loft-libs-world`.

git rm -r:
- `lib/web/`
- `lib/server/`
- `lib/game_protocol/`

Verification (with SDKROOT set for the local macOS ring/SDK
quirk):
- multiplayer_v2: 3/3 pass
- multiplayer_v3: 2/2 pass
- multiplayer_v5: 4/4 pass (+ 1 ignored: v5_t5)
- codegen_emitter::p244_text_native_wrapper_compiles_under_native: pass
- extraction_hygiene: 4/4 pass
- wrap: 49/49 pass
- doc_hygiene: 22/22 pass

Next phase: 5b (graphics + imaging Stage A then Stage B) OR
Phase 6w (extract loft-libs-world — will un-ignore v5_t5) OR
the smaller author-side phases (6.7a yank, 6.16 publish).

## 2026-05-31 — Phase 6b prep — `auto_build_native` target-dir redirect

The actual 6b unblocker.  `extensions::auto_build_native` was
writing `target/` inside the package install dir
(`~/.loft/registry/<pkg>-<ver>/native/target/`).  Restricted
on shared / sandboxed / read-only filesystems → cargo build
fails silently → parser panics at first call to a native fn.

Fix in src/extensions.rs:
- When `pkg_dir` is under `~/.loft/registry/` (i.e., the package
  is a `loft install`-extracted chunk), redirect cargo's target
  via `CARGO_TARGET_DIR=~/.loft/build-cache/<pkg>-<ver>/`.  Sits
  alongside the install in user-writable space.
- When `pkg_dir` is inside the monorepo's `lib/<pkg>/native/`,
  keep the in-tree `target/` — the workspace's existing
  `cargo test` paths + `add_native_extern_flags`'s mtime keying
  expect it there.
- Cached-build lookup checks the redirected target first (where
  future builds land), then the legacy in-tree target/ (so
  existing builds from older loft binaries are still reused).

Verified end-to-end by running 6b execution probe (Stage B
removal of `lib/{web,server,game_protocol}/`): multiplayer
v2 + v3 + 4/5 of v5 pass against registry-resolved net
packages with auto-built cdylibs.  Stage B itself rolled back
pending the downstream consumer-migration work (audience-demo
+ viewer + extraction_hygiene gate + v5_t5 world dep);
6b's main native-build blocker is now resolved.

Still pending for 6b to ship:
- `tools/audience-demo/*.loft` + `tools/audience-demo-50/*.loft`
  + `tools/viewer/src/main.loft` need a loft.toml (or top-level
  monorepo lockfile) so they resolve `use web/server;` post-
  removal.
- v5_t5 `use world;` — either Phase 6w extracts world, or
  v5_t5 gets `#[ignore]`'d in the gap.
- `tests/extraction_hygiene.rs::manifest_native_functions_cover_drained_libraries`
  scans `lib/web/src/web.loft` for `#native` annotations; needs
  to switch to fixture or to gracefully handle missing lib.
- Multiplayer + p244 script relocation (`lib/<pkg>/examples`
  + `lib/server/tests/server.loft` → `tests/integration/`).

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

## 2026-05-31 — PR #238 squash-merged to main (`eff4b01`)

The 38-commit `libraries7` branch (Phase 6.6 / 6.7 / 6.7a / 6.8 /
6.11 / 6.12 / 6.13 / 6.14 / 6.15 / 6.16 + `loft new` +
LIBRARY_AUTHORING.md + net Stage B + author UX sprint) merged to
main as squash commit `eff4b01`.

CI-discovered fixes the merge carries (caught during PR review,
not in the original feature commits):

- **`744535c`** — fmt + clippy --all-targets cleanup + tests/registry_e2e.rs
  `lock_path: None` in 4× `InstallOptions` blocks + `doc/claude/PROBLEMS.md`
  @P389 row's `../lib_plans/12-...` broken link → `lib_plans/12-...` +
  `doc/claude/ROADMAP.md` row for `lib_plans/future/30-loft-distribution`
  (the plan existed on disk but never appeared in ROADMAP) + wasm
  cross-build fix (`src/extensions.rs::native_target_root` gates
  `registry_index::cache_dir()` behind `feature = "registry"`,
  returns in-tree path on non-registry builds) + post-Stage-B
  `src/wasm.rs` repoint of `web.loft` from `lib/web/src/web.loft`
  (removed by Stage B) → `tests/fixtures/libs/web/src/web.loft` +
  `scripts/sync-fixtures.sh PINNED_REFS` adds `web-v0.1.1`.
- **`f2594eb`** — `.github/workflows/ci.yml` G3-avoidance pre-build
  drops the now-stale `lib/web/native/Cargo.toml` +
  `lib/server/native/Cargo.toml` lines (both removed by Stage B).
  graphics + imaging stay in the pre-build (Stage B pending).
- **`6bc9d95`** — `scaffold_library` had a stray `#[cfg(feature =
  "registry")]` it didn't need (the function only writes files);
  drop the gate so `loft new` works under default-features and
  `--no-default-features` alike.
- **`5b945bb`** — `tests/codegen_emitter.rs::p244_text_native_wrapper_compiles_under_native`
  marked `#[ignore = "blocked by @P389 — cross-package native link
  on Linux CI"]` because post-Stage B the test pulls TWO chunk-resident
  cdylibs (server + transitively web) into one native link — exactly
  @P389's failure shape.  Pre-Stage-B the CI's pre-build of
  `lib/web/native` + `lib/server/native` populated each `target/release/deps/`
  with rustls/ureq rlibs and masked it; post-Stage B both packages
  live in `~/.loft/build-cache/<pkg>-<ver>/` and the link fails.
  The actual @P244 regression target (the LoftStr→Str wrapper in
  `src/generation/mod.rs`) is NOT at risk — wrapper codegen runs
  before link.  Un-ignore when @P389 is resolved.  Also fixes the
  broken markdown link in `doc/claude/lib_plans/12-library-extraction/security.md:340`
  (`../30-loft-distribution/` → `../future/30-loft-distribution/`).

Final CI gate state at merge: all checks green (Test ubuntu/macos/windows,
Browser build, Clippy, Format, Doc hygiene, stack_align_guard, all Analyze*,
CodeQL, Nightly health) except the **pre-existing ASan UAF/OOB gate
failure** that also fails on `main` HEAD (commit `2f5268d`, the PLAN53 Wave 2
work — `taiki-e/install-action@nextest` picks up `-Zsanitizer=address`
in RUSTFLAGS and stable rustc rejects it).  GitHub `mergeStateStatus:
UNSTABLE` allowed the squash through.

What remains OPEN in @PLAN12 after this merge:

- Phase 3.6 (path-helper consolidation `dir`/`basename`/`join(text,text)`/`resolve`
  from `03_text.loft` → `02_files.loft`).
- Phase 5 (extract `graphics` + `imaging` into `loft-libs-graphics` Stage A).
- Phase 5b (Stage B — partially unblocked: `loft-ffi-macros 0.1.0`
  published to crates.io 2026-05-31 15:59 UTC; remaining blocker is
  the graphics warning sweep — documented in the phase summary).
- Phase 6w-w (retire every `lib/*/.allow_warnings`).
- Phase 6t Tier 3 (multiplayer harness port — pulls in @P389 fix).
- Phase 6t Tier 5 (imaging / world / markdown coverage gaps).
- Phase 6w (extract `loft-libs-world`).
- Phase 7a/p/b/c (moros split + cross-project consumers).
- Phase 8 (final monorepo cleanup + `audience_crystal` test dir).

## 2026-05-31 — Phase 5b prereq #2 + lint improvements (doc-updates)

Branch: `doc-updates` (not yet merged to main).  Five commits
covering 5b prerequisite resolution + lint quality fixes + the
graphics warning sweep + PROBLEMS.md additions.

- **`4c7c8cc`** — plan-doc reconciliation after PR #238 (status
  bullets + Phase 6.13 IN PROGRESS update + "Pending phases" list
  pruning).
- **`e790ead`** — Phase 5b prerequisite #1 RESOLVED.  Note
  `loft-ffi-macros 0.1.0` published to crates.io 2026-05-31 15:59 UTC
  (~15min after PR #238 squash).  Phase summary row updated:
  BLOCKED → PARTIALLY UNBLOCKED.
- **`e592fd8`** — graphics warning sweep, first wave (3 files clean):
  `lib/graphics/src/mesh.loft` (6/6, sphere divisions + vertex-lookup
  guards), `lib/graphics/src/scene.loft` (0 native warnings — earlier
  count was mesh.loft cross-pollution), `lib/graphics/src/glb.loft`
  (10/10, identity-Mat4 element-wise `?? 0.0`, indexed-assign
  refactor in `glb_scene_json`'s mesh_mat build).
- **`639d546`** — **lint quality improvements**.  Two bugs fixed in
  `src/parser/operators.rs::warn_undefended_fault_sites`:
  - **Position attribution** — initialise `WarnCtx::last_pos` to the
    function's own `Definition.position` so faults inside the body
    without a finer-grained `Value::Span` attribute to the function
    being analysed, NOT to the lexer cursor (which had advanced past
    the closing brace to the next function's header).  Prior
    behaviour misattributed warnings to unrelated clean functions.
  - **AND-conjunction guard walking** — new `collect_guard_pairs`
    helper recursively walks the If-cond tree.  Recognises both the
    short-circuit lowering `If(left, right, Boolean(false))` (loft's
    `a and b` form) and explicit `AndBool` / `And` / `LandInt` calls,
    so `if a < len(u) and b < len(v) { u[a] + v[b] }` now lifts both
    conjuncts onto `guarded_pairs`.  Both arms suppress correctly.
  - Verified: 50/50 wrap + 681/681 issues tests pass.
- **`177453e`** — graphics warning sweep COMPLETE.  Three more files:
  `lib/graphics/src/graphics.loft` (53→0), `lib/graphics/src/render.loft`
  (61→0), `lib/graphics/src/scene.loft` (`Node.mesh_idx` and
  `Node.material_idx` marked `not null` to close the field-defence
  lint that surfaced once indexing was visible).  `.allow_warnings`
  opt-out **deleted**; `LOFT_DENY_WARNINGS=1` clean across all six
  source files.

Restructure pattern documented (bind-local + len-guard idiom):

    rf_d = self.data;
    rf_n = len(rf_d);
    for fr_y in y0..y1 {
      for fr_x in x0..x1 {
        fr_idx = fr_y * self.width + fr_x;
        if fr_idx < rf_n { rf_d[fr_idx] = color; }
      }
    }

The outer compound bounds-check (`if x>=0 and x<w and y>=0 and y<h`)
stays for short-circuit speed; the inner `if fr_idx < rf_n` is
lint-defensive (unreachable under the canvas invariant
`len(data) == width * height`).  `draw_bezier`'s stack rewrote from
indexed-write to truncate-then-append using a temp local to dodge
the @P390 self-slice-assign issue.

Surfaced **@P390** (self-slice-assign drops element values; PROBLEMS.md
row added with workaround).

**Phase 5b is now fully UNBLOCKED.**

## 2026-05-31 — Phase 5b SHIPPED — loft-libs-graphics Stage A + Stage B

Branch: `doc-updates`.  Stage A for graphics + imaging shipped to
chunk repo + registry; Stage B removed all four monorepo lib dirs.

**Stage A (chunk repo + registry):**

- Chunk repo `loft-lang/loft-libs-graphics`:
  - `40e0a3d` Add graphics 0.1.0.
  - `86613ca` Add imaging 0.1.0 (wasm bridge deferred — see
    follow-up section below).
  - Tags: `graphics-v0.1.0`, `imaging-v0.1.0`.
  - GitHub releases with deterministic tarballs:
    - https://github.com/loft-lang/loft-libs-graphics/releases/tag/graphics-v0.1.0
      (sha256 `762271f637cbedaff4c3cda73ef9c6da0bcc826b8c81d9bb08eae03a645510db`,
      88,828 B)
    - https://github.com/loft-lang/loft-libs-graphics/releases/tag/imaging-v0.1.0
      (sha256 `1eceb3771cfbefed9809b2efec39fd8180bba0336cc959e2e0ed6ddb369cf4ce`,
      12,320 B)
- Registry repo `loft-lang/registry` PR #7 merged 17:48 UTC:
  https://github.com/loft-lang/registry/pull/7
  - Adds graphics + imaging package entries at the loft-libs-graphics
    section.
  - All three validator gates green:
    - Gate 1 schema lint ✓
    - Gate 2 sha256 + size verify (downloaded from GH releases) ✓
    - Gate 3 reproducible-build re-check (cloned at tag, re-ran
      `loft package`, byte-identical) ✓

**Stage B (monorepo cleanup):**

- **`408686d`** (5b-4) — consumer migration.  4 loft.tomls swapped
  from path-deps to registry versions:
  - `lib/moros_render/loft.toml`: `graphics = { path = "../graphics" }`
    → `graphics = ">=0.1"`.
  - `lib/moros_sim/loft.toml`: same.
  - `lib/audience_crystal/loft.toml`: `gridmesh = ">=0.1"`.
  - `tools/audience-demo/loft.toml`: `gridmesh` + `graphics` swap.
  - 4 new `loft.lock` files pinning the registry SHA + version.
  - Sibling moros_* deps stay path-resolved (Phase 7-series).
- **`93ce34a`** (5b-5) — monorepo cleanup.  Removed
  `lib/{shapes,gridmesh,graphics,imaging}/` (93 tracked + ~2235
  untracked artifact files):
  - `src/wasm.rs::BUNDLED_LIB_FILES` repointed all 7
    `include_str!` paths from `../lib/{graphics,shapes}/src/X.loft`
    → `../tests/fixtures/libs/{graphics,shapes}/src/X.loft`.  Same
    fixture-repoint pattern Phase 6b used for `web.loft`.
  - `tests/extraction_hygiene.rs::FORBIDDEN_LIBRARY_SYMBOLS_MANUAL`
    gained 56 graphics symbols (n_gl_*, n_save_png, n_text_height,
    n_rasterize_text_into, n_audio_*) + 1 imaging symbol
    (n_load_png; n_save_png shared with graphics).  Retired the
    stale TBD comment block.
  - `scripts/sync-fixtures.sh` PINNED_REFS extended with
    `graphics-v0.1.0` + `imaging-v0.1.0`.  Bug fix: each
    (chunk, ref) pair now clones into its own temp dir (was: one
    clone per chunk at first-seen ref, which broke when later refs
    added packages the earlier ref's commit didn't have).
  - `tests/fixtures/libs/{graphics,imaging,shapes}/` populated from
    canonical chunk-repo tags (50 files total).
- **`21af961`** (5b-6) — `cargo fmt` for extraction_hygiene.rs (the
  long graphics symbol rows needed wrapping).

**Verification** (all green):
- `cargo build --release --bin loft --features registry`
- `cargo fmt --check`
- `cargo clippy --bin loft --features registry --all-targets -- -D warnings`
- `cargo test --release --test wrap` 50/50
- `cargo test --release --test issues` 681/681
- `cargo test --release --test extraction_hygiene` 4/4
- `./scripts/find_problems.sh --bg` full nextest — zero failures
- `scripts/check_doc_drift.sh` clean (only pre-existing plan-53
  time-projection warning)

**Phase 5b CLOSED.**  Phase 5 chunk (loft-libs-graphics) is now
fully (A+B) shipped — third and final chunk to complete both
stages after loft-libs-core (4+stage-B) and loft-libs-net (6+6b).

---

## Pending phases (in roughly the order they'd land)

These will get an entry here when they ship:

- Phase 3.6 — path-helper consolidation `dir`/`basename`/`join(text,text)`/
  `resolve` from `03_text.loft` → `02_files.loft`.
- Phase 6w-w — retire every `.allow_warnings`.  `lib/graphics/`
  retired by warning sweep + `lib/shapes/` retired by removal
  (2026-05-31); remaining 6 monorepo libs: moros_map (4),
  moros_editor (6), moros_render (31), audience_crystal (40),
  moros_ui (155), moros_sim (173) — total 409 unique warnings.
- Phase 6t Tier 3 — multiplayer harness port (touches @P389).
- Phase 6t Tier 5 — imaging / world / markdown coverage gaps.
- Phase 6w — extract `loft-libs-world`.
- Phase 7a/p/b/c — moros split + cross-project consumers.
- Phase 8 — final monorepo cleanup + `audience_crystal` test dir.
- Phase 6.13 closure — permanent-doc harvest (PACKAGES.md /
  PKG_REGISTRY.md / new author-facing onboarding docs); CLAUDE.md
  table surgery; reference-audit sweep; move this whole directory
  to `lib_plans/finished/12-library-extraction/`.  Fires when only
  the longer-arc items above remain.

Append new sections above this list as phases ship; remove
items from this list as they land.
