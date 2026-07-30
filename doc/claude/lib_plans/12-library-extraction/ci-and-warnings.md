<\!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — CI templates + warning ratchet

Part of [@PLAN12 library extraction](README.md).  Covers
**Phase 6.5** (canonical `library-ci.yml` shipped + applied
to all three chunks), the **Bringing a chunk to all-green
CI checklist** (the per-chunk omnibus pattern + Stage A/B
sequencing), and **Phase 6w-w** (retiring every monorepo
`.allow_warnings` opt-out).  Together: the discipline that
makes every chunk repo green under `LOFT_DENY_WARNINGS=1`
plus the cleanup ratchet for in-monorepo libraries.

---

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
          git clone --depth 1 https://github.com/loft-lang/loft loft-src
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
warning sweep must be fixed on `loft-lang/loft:main` before later
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

4. **Verify locally first** via `scripts/lib_audit.sh --repo <chunk> --local --no-native`
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
in the chunk, `scripts/lib_audit.sh --src <chunk>=…`
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
  header and every tar entry.  Fix shipped in loft-lang/loft #234:
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
both fixes are already on registry main + loft-lang/loft main as
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
Both were fixed in loft-lang/loft #231 (merged 2026-05-30) before
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


### Stage B execution per chunk (proposed 2026-05-31)

The Stage A/B workflow above is documented as steps 6-7 of
the Bringing-a-chunk checklist.  This subsection adds the
per-chunk EXECUTION detail — what specifically lands in each
chunk's Stage B PR, when, and how the consumer migrations are
sequenced.

`loft-libs-core` (Phase 4) shipped Stage A + Stage B together
by 2026-05-28 (the consumer set was small: only monorepo tests
imported `arguments` / `random` / `crypto`, and those were
migrated to `tests/scripts/*.loft` references).  The remaining
two chunks are deferred until 6.6 (auto-install) + 6.12
(loft-dev fixture pattern) ship — otherwise the consumer set
explodes into "every script needs a loft.toml" friction.

#### Phase 6b — `loft-libs-net` Stage B (proposed 2026-05-31)

**Scope.**  Remove `lib/web/`, `lib/server/`, `lib/game_protocol/`
from the monorepo.  Migrate the following consumers:

| Consumer | Migration |
|---|---|
| `tools/audience-demo/*.loft` (4 files: `crystal_render`, `projector`, `crystal_editor`, `crystal_stress`) | Single-file scripts use `use web;` / `use server;` / `use gridmesh;` / `use audience_crystal;`.  After 6.6 ships, auto-install resolves these from the registry on first run.  No loft.toml needed.  Add a minimal `loft.toml` only if pinning is wanted; otherwise script-mode + global lockfile suffices. |
| `tools/audience-demo-50/probe*.loft` (2 files: `probe`, `probe_server`) | Same — auto-install handles it. |
| `tools/viewer/src/main.loft` | Has a `loft.toml` (verify); migrate path-deps to registry-version. |
| `lib/audience_crystal/loft.toml` | Already has `gridmesh = { path = "../gridmesh" }`.  Switch to `gridmesh = ">=0.1"`.  Same for `web` if it appears (audit). |
| `lib/audience_crystal/src/*.loft` + `tests/*.loft` | `use gridmesh;` / `use web;` etc. — resolve through the lockfile after the loft.toml change. |
| `tests/crystal_editor_gold.rs` | Drives `tools/audience-demo/crystal_editor.loft` from repo cwd.  After 6.6 + 6.12, auto-install fires from anywhere; this test passes without changes. |
| `tests/fixtures/libs/` (post-6.12) | Add `web` / `server` / `game_protocol` snapshots so monorepo compiler tests still have library source to compile against. |

**Execution outline:**

1. Verify 6.6 (auto-install) is on `loft-lang/loft:main` and
   working end-to-end (the `LOFT_OFFLINE=1` off-switch and
   announcement output both green).
2. Verify 6.12 (`tests/fixtures/libs/`) has `web` / `server` /
   `game_protocol` snapshots committed; `library_fixture_suite`
   passes.
3. On a feature branch, perform the consumer audit (`grep -rln
   'use web\|use server\|use game_protocol'`).
4. Migrate each consumer's `loft.toml` (path-dep →
   registry-version).  For scripts without a `loft.toml`, no
   change needed (auto-install fires).
5. Run `make ci` — all consumers should resolve via auto-install
   + lockfile; cache populates on first run.
6. `git rm -r lib/web/ lib/server/ lib/game_protocol/`.
7. Re-run `make ci` — tree still green.
8. Update `tests/wrap.rs::collect_library_tests` if hardcoded
   per-package skip lists reference these now-removed packages.
9. Update plan-12 phase summary table: row 6 → "SHIPPED (A+B)".
10. PR title: `Stage B: remove lib/{web,server,game_protocol}/ — consumers migrated to net 0.1.1`.

**Verify:** `find lib/ -maxdepth 1 -name 'web' -o -name 'server' -o -name 'game_protocol'` returns empty.  `grep -rln 'path = "../\(web\|server\|game_protocol\)"' lib/ tools/` returns empty.  `make ci` green.

#### Phase 5b — `loft-libs-graphics` Stage B (proposed 2026-05-31)

**Scope.**  Two-step phase because Phase 5 itself is only
half-shipped: shapes + gridmesh are Stage A; graphics +
imaging haven't been extracted yet.

**5b.1 — extract graphics + imaging Stage A** (the remaining
half of Phase 5):

1. Both are codegen-unblocked (per @P321c fix) and
   Tier-2-unblocked (`tests/graphics_gold.rs` infrastructure
   ready to port via Phase 6t Tier 2 — DONE per phase-summary).
2. Sync `lib/graphics/` + `lib/imaging/` source into
   `loft-lang/loft-libs-graphics/`.  Apply the Bringing-a-chunk
   checklist (canonical YAML — already there since gridmesh
   shipped; per-symbol 6r re-clean if any `#native` annotations
   need it; warning sweep using the three idioms).
3. Open omnibus PR; verify all 4 packages green
   (shapes/gridmesh already; graphics + imaging joining).
4. Tag `graphics-v0.1.0` + `imaging-v0.1.0`; package; release;
   registry PR adding the two new entries.

**5b.2 — Stage B for the full chunk:**

Same execution outline as 6b above, applied to consumers of
`shapes` / `gridmesh` / `graphics` / `imaging`:

| Consumer | Migration |
|---|---|
| `lib/graphics/examples/25-brick-buster.loft` | `use shapes;` — auto-install resolves it.  Brick-buster is Makefile-driven, not strict CI; runs on demand. |
| `lib/audience_crystal/loft.toml` | Already covered in 6b. |
| `tools/audience-demo/*.loft` | `use gridmesh;` — also covered in 6b's auto-install case. |
| `lib/moros_render/loft.toml` + `lib/moros_sim/loft.toml` | Both have `graphics = { path = "../graphics" }`.  Switch to `graphics = ">=0.1"`. |
| `tests/graphics_gold.rs` | Drives examples that `use graphics;`; after Stage A, resolution goes through the registry via 6.6.  Verify on a clean `~/.loft/` checkout. |
| `tests/fixtures/libs/` | Add `shapes` / `gridmesh` / `graphics` / `imaging` snapshots. |

**Execution:** mirrors 6b (consumer audit → migrate
loft.tomls → `make ci` → `git rm -r lib/{shapes,gridmesh,graphics,imaging}/`
→ `make ci` again → phase-table update → Stage B PR).

**Verify:** as 6b, plus the brick-buster example builds + runs
via the Makefile target after `lib/shapes/` is gone (cache must
be populated; air-gap users follow the bundle workflow).

**Sequencing.**  5b.1 (extract graphics + imaging) can land
INDEPENDENTLY of 5b.2 (Stage B cleanup) — they're separable.
Reasonable order:

1. 5b.1 first (publish graphics + imaging 0.1.0 to registry).
   At this point `loft-libs-graphics` has all four packages
   shipped Stage A.
2. Wait for 6.6 + 6.12 to ship (auto-install + fixture
   pattern).
3. 5b.2 then (remove monorepo copies).
4. 6b can run in parallel with 5b.2 — they touch different
   monorepo directories.
