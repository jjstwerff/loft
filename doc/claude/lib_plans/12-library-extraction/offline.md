<\!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — offline support

Part of [@PLAN12 library extraction](README.md).  Covers
**Phase 6.11** (`loft bundle export`/`import` + `LOFT_REGISTRY_URL=file://`
for air-gapped, regulated-environment, classroom-lab, edge,
and locked-down-firewall deployments) and **Phase 6.12** (the
loft-developer offline test loop — `tests/fixtures/libs/`
bundled-fixture pattern that survives Stage B's `lib/<pkg>/`
removal).

Companion docs:
- [security.md](security.md) — stale-advisory thresholds work
  in tandem with bundle freshness for regulated environments.
- [registry-resolution.md](registry-resolution.md) — bundle
  import populates the same `~/.loft/registry/` cache the
  auto-install path uses.

---

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

