<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library authoring

End-to-end guide for publishing a loft library to the package
registry.  Walks through scaffolding a fresh library →
developing the API → packaging + publishing → maintaining
(yanking when a vulnerability surfaces).

This consolidates the author-facing surface that lives across
[`PACKAGES.md`](PACKAGES.md), [`PKG_REGISTRY.md`](PKG_REGISTRY.md),
and the `lib_plans/12-library-extraction/` topical files into
a single narrative.  The CLI commands referenced here all live
in the loft binary — no extra tooling install needed.

---

## Quick reference

```
loft new <name>           # scaffold a fresh library skeleton
loft new <name> --native  # also creates native/ cdylib skeleton
loft new <name> --chunk   # also creates .github/workflows/library-ci.yml
loft test                 # exercise tests/
loft package              # build a deterministic tarball
loft publish              # emit registry-PR index.json entry
loft yank <pkg>@<ver>     # emit yank PR blocks (security advisory)
```

The author loop, in five steps:

```
loft new my_lib                       1. scaffold
$EDITOR my_lib/src/my_lib.loft       2. develop
cd my_lib && loft test                3. test
git tag + push + gh release create    4. release source + binary
loft publish                          5. submit to registry
```

**Maintaining the loft-lang family libs** (`loft-libs-*`) — run from a
`loft-lang/loft` checkout, not a single package dir:

```
scripts/registry_maintain.sh   # publish every stale/missing own lib, sign the index, push
scripts/registry-sign.sh       # re-sign the index after a hand-merge (review-then-sign)
scripts/sync-fixtures.sh       # regenerate tests/fixtures/libs/<pkg>/ from the pinned tags
```

These automate the publish + sign + fixture steps (§4–§5) for our own
libraries; the per-package `loft publish` flow below is the mechanism they
wrap (and the path external contributors use — [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md)).

---

## 1. Scaffold a fresh library

```
$ loft new my_lib
Created library `my_lib/`:
  loft.toml
  src/my_lib.loft
  tests/01-smoke.loft
  README.md
  release.sh
```

`loft new` validates the name (lowercase ascii + digits +
underscore), refuses a **reserved namespace name** (`std`, `core` — these resolve to a
built-in namespace, not a package; see [C101](DESIGN_DECISIONS.md)), and refuses to
overwrite an existing dir.

`release.sh` (executable) is a one-command release: it reads name + version
from `loft.toml`, runs the test gate + a deterministic-package check, commits any
version bump, tags `<name>-v<version>`, pushes, packages, and `gh release
create`s.  `./release.sh` releases the current version; `./release.sh 0.2.0`
bumps first.  It refuses to re-cut an existing tag (releases are immutable —
bump instead).  Automates §4a–4b; afterwards an own lib is picked up by
`registry_maintain.sh`, an external lib opens a registry PR.

Flags:

- `--native` — adds `native/Cargo.toml` + `native/build.rs` +
  `native/src/lib.rs` for the Rust cdylib.  Cargo deps already
  point at the registry versions of `loft-ffi` /
  `loft-ffi-build` — the scaffolded native crate works
  immediately on `cargo build --release` post-publish.
- `--chunk` — adds `.github/workflows/library-ci.yml` with the
  canonical CI template (mold + loft source build +
  `loft --interpret --tests tests` + `loft --native --tests
  tests`).  Use when starting a fresh `loft-libs-<family>/`
  chunk repo whose first member is this library.

What the scaffold contains:

- `loft.toml` — `[package]` (name + version 0.1.0 + loft >=0.8)
  + `[library]` (entry = src/<name>.loft) + empty
  `[dependencies]`.
- `src/<name>.loft` — placeholder `pub fn hello() -> text`.
- `tests/01-smoke.loft` — single `test_hello` that asserts the
  placeholder's return value.  Passes immediately, so `loft
  test` is green from the first invocation.
- `README.md` — install + usage snippets.

## 2. Develop the API

Edit `src/<name>.loft`.  Add `[dependencies]` to `loft.toml`
as you need them (`loft install <dep>` to auto-populate the
lockfile).  Add more tests under `tests/`; loft discovers
every `fn test_*()` in `.loft` files there.

Resolution chain for `use X;` in your source:
1. Sidecar `<script>.loft.lock` (when you've `loft pin`'d a
   one-file script).
2. Walk-up `loft.toml` + adjacent `loft.lock` (the project
   mode you get from `loft new` — most common).
3. Cwd `loft.lock` (script-mode fallback).
4. Auto-install from the registry (when nothing else
   resolved).
5. Flat fallbacks.

Run tests:

```
$ loft test                            # interpreter, all tests in tests/
$ loft --native test                   # native codegen
$ LOFT_DENY_WARNINGS=1 loft test       # strict — fails on any warning
```

The `.allow_warnings` opt-out file at the package root lets a
package temporarily ship with warnings; remove it once the
warning count reaches zero.  Chunk CI flips the
`LOFT_DENY_WARNINGS` env to `0` when the opt-out file is
present.

## 3. Pre-release checklist

The mechanical `[auto]` core of the full correctness bar — see
[LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) for the Goal-by-Goal + doc-quality
`[review]` items and the registry `verified` administration.

Before you ship a version:

- [ ] All tests pass under both `loft test` and `loft --native test`.
- [ ] `LOFT_DENY_WARNINGS=1 loft test` is green (or you've
      kept `.allow_warnings` as an opt-out only when the
      package isn't ready yet).
- [ ] `loft.toml` has the new version under `[package] version`.
- [ ] `[package] description` is a real one-line summary (not the `loft new`
      placeholder) — it's the official registry catalog text (`loft search` /
      `loft api --registry`); registry tooling prefers it over the README.
- [ ] README, doc comments on every `pub fn` / `pub struct`.
- [ ] CHANGELOG note for the version (free-form).
- [ ] Local re-package produces a byte-identical sha256 across
      two runs (verifies deterministic packaging):

      ```
      $ loft package && shasum -a 256 *.tar.gz
      $ rm *.tar.gz && loft package && shasum -a 256 *.tar.gz
      # Both hashes must match.
      ```

## 4. Publish

The publish flow has four phases: tag → release source + binary →
emit registry entry → registry PR.

> **Two paths — pick by who owns the library.**
>
> - **A loft-lang family library** (`loft-libs-*`, maintained from a
>   `loft-lang/loft` checkout): don't paste-and-PR by hand.  Tag + release the
>   version (§4a–4b), then run
>   **[`scripts/registry_maintain.sh`](../../scripts/registry_maintain.sh)** — it
>   clones every family repo fresh, publishes each stale/missing version against
>   the live index (`loft publish` under the hood), **signs** the index (the
>   trust-root step — it shows the diff to review first), and pushes.  One
>   reviewed run catches up the whole registry; §4c–4d + §5a below are the
>   mechanism it automates.  Before it signs it re-downloads every tarball and
>   verifies the sha256 — on any mismatch it **refuses to sign** (so a stale
>   release artifact can never reach the signed index; fix that release and
>   re-run).
> - **An external contributor's library**: the manual tag → package → upload →
>   PR flow in §4a–4d (the maintainer signs the index on merge, you sign
>   nothing) — see [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md).
>
> **Never hand-edit `index.json` for an own-lib release.**  `registry_maintain.sh`
> (or `registry-sign.sh` after a hand-merge) regenerates + re-signs it.  A hand
> edit that isn't re-signed leaves the published signature invalid and breaks
> `loft install` / `loft search` for everyone.

### 4a. Tag the version

```
$ git tag <name>-v<version>           # convention: name then dash-v
$ git push origin <name>-v<version>
```

Tag convention is `<name>-v<version>` (e.g. `gridmesh-v0.1.1`).
Required by the registry's gate-3 reproducible-build check —
the validator clones at this tag to re-package.  Set
`repository = "<monorepo>"` in `loft.toml` (e.g. `loft-libs-graphics`)
so `loft package` emits the matching `<name>-v<version>` tag + release
URL automatically (see PACKAGES.md § Manifest).

### 4b. Build + upload the release artifact

```
$ loft package                                          # writes <name>-<ver>.tar.gz
$ gh release create <name>-v<version> <name>-<version>.tar.gz \
    --title "<name> v<version>" \
    --notes "<release notes>"
```

`loft package` produces a deterministic tarball (zero mtimes
in gzip + tar headers).  The sha256 is stable across
machines; the registry's gate-3 re-runs `loft package` from
the tagged source to verify byte-for-byte equality.

**Prebuilt cdylibs build automatically (@PLN21).**  A *native*
library's own CI calls the reusable producer workflow so a
consumer can `use` it with **no Rust toolchain** — and so a
broken host build is caught before merge:

```yaml
# .github/workflows/prebuild.yml
on:
  pull_request:
    paths: ['native/**', 'loft.toml']   # PR → validate it builds on every host
  push:
    tags: ['v*']                        # tag → build + attach to the release
jobs:
  prebuild:
    uses: loft-lang/loft/.github/workflows/prebuild-native.yml@main
    with:
      publish: ${{ startsWith(github.ref, 'refs/tags/') }}
```

On a PR it builds the cdylib per host triple (a validation
gate); on the tag it attaches each `lib<stem>.<ext>` to the
release and prints the `binaries[<triple>]` entry (with its
`loft_ffi_fp`) to paste into the registry PR below.  Prebuilts
are optional and per-platform: ship the common targets, others
fall back to a source build on first use.

### 4c. Emit the registry entry

From the package dir:

```
$ loft publish
# Paste this entry into `loft-lang/registry/index.json` under
# `"packages": { "<name>": { "versions": { ... } } }`:

"0.1.0": {
  "url": "https://github.com/<org>/<chunk>/releases/download/<name>-v0.1.0/<name>-0.1.0.tar.gz",
  "sha256": "<sha>",
  "size": <bytes>,
  "loft": ">=0.8",
  "subpath": "<name>",
  "deps": { ... from your loft.toml ... },
  "published": "<now ISO-8601>"
}

[publish] verified release <name>-v0.1.0 exists with asset <name>-0.1.0.tar.gz
[publish] next step: open registry PR with the entry above
```

`loft publish` auto-detects the chunk repo from `git remote
get-url origin`, verifies the GitHub release exists with the
expected asset, re-packages locally to compute the sha256 +
size (no chance of hash mismatch), and emits the
index.json-ready entry.

Flags:

- `--dry-run` — skip the GitHub-release verification (use
  when developing this loop locally).

### 4d. Open the registry PR

Manually clone `loft-lang/registry`, paste the emitted block
into `index.json`, and open a PR:

```
$ git clone git@github.com:loft-lang/registry.git
$ cd registry
$ git checkout -b add-<name>-<version>
$ $EDITOR index.json   # paste the emitted entry
$ git commit -am "Add <name> <version>"
$ gh pr create --title "Add <name> <version>" --body "<rationale>"
```

The registry's CI runs three gates:

1. **Schema lint** — `index.json` validates against the
   schema.
2. **Tarball verify** — sha256 + size of the published
   tarball match what your PR claims.
3. **Reproducible-build re-check** — clones your source at
   the tag, runs `loft package`, compares the resulting
   sha256.  Catches "the GitHub release tarball is stale" +
   "the git tag was force-pushed" + supply-chain swaps.

Maintainer reviews + merges.  The new version is then live;
`loft install <name>` works.

If you're publishing a NEW package (no prior versions),
also add the package block above the versions:

```json
"<name>": {
  "description": "<one-liner>",
  "homepage": "https://github.com/<org>/<chunk>/tree/main/<name>",
  "categories": ["<category>"],
  "yanked": [],
  "versions": { ... your version block ... }
}
```

`loft publish` emits a commented-out stub for this — copy +
customize.

## 5. Maintain

### 5a. Patch releases

For a `0.1.0` → `0.1.1` patch:

1. Bump `version` in `loft.toml`.
2. Commit + tag `<name>-v0.1.1`.
3. `loft package` + `gh release create <name>-v0.1.1 ...`.
4. Publish to the registry:
   - **Own (loft-lang family) lib:** run
     [`scripts/registry_maintain.sh`](../../scripts/registry_maintain.sh) from a
     `loft-lang/loft` checkout — it sees the new version as "stale", publishes it,
     signs, and pushes (review the diff at the prompt).  No manual PR.
   - **External lib:** `loft publish` + registry PR with the new block (under the
     same package, in `versions`).

The registry keeps all versions; old releases stay reachable
unless explicitly yanked.

### 5b. Yank a vulnerable version

When a CVE surfaces against a published version:

1. **Publish the fix first.**  Don't yank before the fixed
   version is available; that strands consumers.
2. Run `loft yank`:

   ```
   $ loft yank web@0.1.0 \
       --severity security_critical \
       --advisory GHSA-xxxx-yyyy-zzzz \
       --summary "TLS bypass in ws_client_connect" \
       --affected ">=0.1.0, <0.1.1" \
       --fixed-in "0.1.1"
   ```

3. The CLI emits two blocks:
   - **Edit 1**: typed `status` field for `index.json`'s
     affected version entry.
   - **Edit 2**: cross-referenced row for
     `advisories.json`'s `advisories[]` array.

4. Apply both edits to your local registry checkout, commit,
   open the PR:

   ```
   $ git checkout -b yank-web-0.1.0
   $ $EDITOR index.json advisories.json
   $ git commit -am 'yank: web 0.1.0 (GHSA-xxxx-yyyy-zzzz)'
   $ gh pr create --title 'yank: web 0.1.0' \
       --body "<advisory rationale + reference URLs>"
   ```

Severity tiers (effects on the loft binary's runtime check):

| Tier | Behaviour |
|---|---|
| `security_critical` | Loud error block at start of every run; user proceeds anyway (default).  Opt-in refusal via `LOFT_STRICT_SECURITY=1` / `--strict-security` (CI gates). |
| `security_high` | Loud warning, always proceeds. |
| `security_low` / `bug` | One-line warning per run. |
| `deprecated` | One-line note. |

The default-warn policy mirrors `cargo audit` — security fixes
can introduce breaking changes, and the user often needs to
run their cached vulnerable code while porting.

### 5c. Update the dep set

When a downstream consumer is on an old version that's been
patched:

```
$ loft update                 # refresh lockfile to latest in-range
$ loft update <pkg>           # scoped to one package
$ loft update --check         # CI gate: exit 1 if updates available
```

`loft update` auto-skips yanked versions via the same
`find_best_version` filter that picks installs.

### 5d. Re-sync the loft monorepo fixture (in-tree-tested libs only)

This step is **only** for libraries that the loft compiler's own
test-suite exercises through a pinned source mirror under
[`loft-lang/loft`](https://github.com/loft-lang/loft)'s
`tests/fixtures/libs/<pkg>/` — the dogfood libraries (`arguments`,
`graphics`, `gridmesh`, `shapes`, `imaging`, `game_protocol`, `web`,
`hex_world`, `time`).  A pure registry-only library has no fixture; skip
to step 5c's registry PR and you're done.

The fixture is a **deliberate snapshot, not auto-latest** — so a
library change that affects the compiler tests is a reviewable commit in
the loft repo, never silent drift.  After the new tag exists in the
chunk repo (5a step 2):

1. In `loft-lang/loft`, bump the tag in `scripts/sync-fixtures.sh`'s
   `PINNED_REFS` table for your package
   (`graphics  graphics-v0.1.0` → `graphics  graphics-v0.1.1`).
2. Refresh the snapshot:
   ```
   $ scripts/sync-fixtures.sh            # clones the tag, copies <pkg>/ into the fixture
   ```
3. Run the suites the fixture feeds — at minimum `cargo test --release
   --test wrap` (interpreter) and any package-specific gold tests
   (e.g. `graphics_gold`) — to confirm the new snapshot still passes.
4. Commit the `PINNED_REFS` bump **and** the `tests/fixtures/libs/<pkg>/`
   diff together as one reviewable commit (per the branch policy: on a
   feature branch, PR to `main`).
5. The CI invariant `scripts/sync-fixtures.sh --check` (exit 1 on
   fixture-vs-tag drift) now passes for that package.

Why the fixture and not a registry install: zero network during `cargo
test`, reproducible across machines + CI, and it survives the eventual
removal of the in-monorepo `lib/<pkg>/` source.  Full rationale +
`PINNED_REFS` semantics live in the `scripts/sync-fixtures.sh` header.

### 5e. Fix a library bug — the clean dev-checkout flow

A library's source lives **only** in its chunk repo; loft consumes a pinned,
read-only **snapshot** under `tests/fixtures/libs/<pkg>/` (§ 5d).  So a library
fix never happens in the loft tree, and never by editing the fixture directly
(that's drift — `scripts/sync-fixtures.sh --check` fails).  This holds even when
a *language* change breaks the fixture: when @PLN22 removed flat-list
`use lib::a, b;`, `game_protocol`'s fixture stopped compiling — the right fix was
to re-release the lib (`game_protocol-v0.1.2`, grouped `use`) and bump the pin,
**not** to hand-patch the fixture to compile (that hides the drift and ships a
library whose own tests no longer build on current loft).  Work in the chunk
repo, **out of the loft tree**, so no stale artifacts accrue in loft:

1. **Issue home.** File / find the bug in the **chunk repo's** tracker
   (`loft-lang/loft-libs-<chunk>`), per
   [ISSUE_TRACKING.md § Convention](ISSUE_TRACKING.md) — so `Fixes #N` is
   same-repo and the `fixed-pending-merge` lifecycle works.  (A bug mis-filed in
   `loft-lang/loft` whose fix is library code gets re-homed there.)
2. **Checkout — out of tree.** Clone the chunk repo to a dedicated dev dir
   *outside* the loft working tree (e.g. `~/loft-dev/<chunk>`), never into
   `loft/lib/<pkg>/`.  The pre-extraction `lib/<pkg>/` layout is being removed;
   any leftover skeleton there (build cruft, no source, no `.git`) is **stale and
   should be deleted** — it only pollutes loft's `git status` and creates "is the
   source here?" ambiguity (the trap that hid `graphics/native/src/text.rs`
   during the @P340 / `@GH252` follow-up — the real source was in the fixture +
   chunk repo, never in `lib/graphics/`).
3. **Fix + test.** Edit the package source in the checkout; run the library's own
   suite there, or test it against loft with `--lib ~/loft-dev/<chunk>`.  The
   checkout shadows nothing in loft's tree and builds in its **own** `target/`.
4. **Tag + push.** Commit with `Fixes #N` (chunk-repo issue), tag
   `<pkg>-vX.Y.Z`, push.  The chunk repo's own apply/strip workflows label then
   close the issue on merge.
5. **Re-sync the loft fixture (§ 5d).** Bump `PINNED_REFS` + run
   `sync-fixtures.sh`, and commit the `tests/fixtures/libs/<pkg>/` diff in loft as
   **one reviewable commit** — separate from the issue close.  loft now tracks the
   fixed snapshot.
6. **Teardown.** `rm -rf ~/loft-dev/<chunk>` and `~/.loft/build-cache/<pkg>-*`.
   The loft tree is pristine; no stale artifacts remain.

The principle is the project's anti-stale-artifact rule (GOALS.md § "the method
mirrors the goals"): one source of truth for where the code lives (the chunk
repo), isolated + disposable build artifacts (out-of-tree checkout, own
`target/`), and a clean teardown — so the shared medium (the build) can't drift
out of sync and lie.

## Reference

| Topic | Source |
|---|---|
| Package format ([package] / [library] / [dependencies] / [native] / [wasm.bridge]) | [PACKAGES.md](PACKAGES.md) |
| Registry index.json + advisories.json schema | [PKG_REGISTRY.md](PKG_REGISTRY.md) |
| Air-gap deployment workflow | [`lib_plans/12-library-extraction/offline.md`](lib_plans/12-library-extraction/offline.md) |
| Security advisory channel (consumer side) | [`lib_plans/12-library-extraction/security.md`](lib_plans/12-library-extraction/security.md) |
| Canonical `library-ci.yml` template | [`lib_plans/12-library-extraction/library-ci.yml.example`](lib_plans/12-library-extraction/library-ci.yml.example) |
| Cross-package consumer matrix (moros / dryopea / bumper) | [`lib_plans/12-library-extraction/README.md` § Cross-project consumers](lib_plans/12-library-extraction/README.md#cross-project-consumers--moros--dryopea--bumper-airplanes) |
| Monorepo test-fixture re-sync (dogfood libs — step 5d) | [`scripts/sync-fixtures.sh`](../../scripts/sync-fixtures.sh) header (`PINNED_REFS`, `--check`) |

## Troubleshooting

**`loft publish` says "release `<tag>` not found"** — you
haven't created the GitHub release yet, OR the asset name in
the release doesn't match `<name>-<version>.tar.gz`.  Re-run
`loft package` to ensure the filename is canonical; then
`gh release create <tag> <tarball>`.

**Registry CI's gate-3 fails with sha256 mismatch** — the
tarball on the GitHub release doesn't byte-for-byte match
what `loft package` produces from the tagged source tree.
Common causes:
- You ran `loft package` against a working tree with
  uncommitted changes, then committed + tagged afterwards.
  Re-run `loft package` from a clean checkout at the tag.
- You uploaded a manually-edited tarball.  Don't.

**`use <pkg>;` resolves to the wrong version** — check the
resolution chain via `loft list-installed` + the closest
`loft.lock`.  Sidecar `<script>.loft.lock` takes precedence
over walk-up `loft.lock`.  `LOFT_OFFLINE=1` blocks
auto-install.

**Native crate fails to build from `~/.loft/registry/.../native/`**
— the cargo build redirects to `~/.loft/build-cache/<pkg>-<ver>/`
(per Phase 6b's `auto_build_native` redirect).  Check that
`~/.loft/build-cache/` is writable + that `loft-ffi` /
`loft-ffi-build` are reachable on crates.io (or via your
configured `[source]` mirror).
