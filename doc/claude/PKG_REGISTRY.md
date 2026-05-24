<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# PKG.REG — file-based registry MVP

Draft 2026-05-24.  This is the design for the **MVP** of the loft
package registry.  Goal: ship `loft install <name>` against a static
file before designing a full server.  The same URL surface migrates
to a server later with no client-side changes.

This doc fleshes out the "(b) central registry server (GitHub Pages +
static index acceptable for MVP)" sub-phase from
[PACKAGES.md § Open work](PACKAGES.md#open-work).

---

## The invariant — end-user experience is identical to a real server

The whole point of choosing a file-based MVP is that **the loft
client never knows the difference**.  Migration from file to server
later is a server-side swap; users do not need to change a flag,
edit a config, or upgrade the loft binary.

Concretely, **EVERY** part of the user-visible surface stays
constant across MVP → server:

| User-visible surface | MVP behaviour | Future-server behaviour |
|---|---|---|
| `loft install crypto` | Downloads tarball, extracts to `~/.loft/registry/`, writes `loft.lock`. | Same. |
| `loft install crypto@0.1.0` | Same — pinned version. | Same. |
| `loft install` (no args) | Reads `loft.toml`, resolves, installs. | Same. |
| `loft.toml` `[dependencies]` syntax | `crypto = "^0.1"` etc. | Same. |
| `loft.lock` format | TOML, see [§ Consumer's lock file](#consumers-lock-file--loftlock). | Same. |
| Sha256 verification | Mandatory. | Same. |
| Cache layout (`~/.loft/registry/<pkg>-<v>/`) | Identical. | Same. |
| Error messages | "package X not found", "version Y conflict", etc. | Same. |
| Search (`loft search foo`) | Client-side grep on cached index. | Server-side filter, faster — but same CLI args + same output shape. |
| `loft info <name>` | Read from cached index. | Read from server endpoint; same output. |
| Offline mode (`--offline`) | Uses cache. | Same. |
| Tarball format | See [§ Tarball format](#tarball-format). | Same. |

Things that change at MIGRATION time and are NOT user-visible:

- Where `registry.json` is hosted (a static file vs a server
  endpoint that emits the same JSON).
- How a publisher uploads a new version (PR against the registry
  repo vs `POST /v1/publish` with an auth token).
- Operational surface (no infra vs `loft-lang.org` server +
  database).

This invariant is load-bearing: the server can be built and
deployed without breaking any installed loft binary in the wild.
Every design decision below preserves it.

---

## Why a file is enough

A registry needs to answer two questions:

1. **"Given a package name + version, where is the tarball?"** —
   index lookup.
2. **"Did I get the tarball I asked for?"** — verification.

That's it.  Server-side features (search, auth, atomic publish,
download counts, yanking) are **non-essential for an early ecosystem**.
Cargo's earliest model was file-based; npm's still has a fundamentally
static GET-by-name shape.

Trade-off matrix:

| Concern | File-based MVP | Full server |
|---|---|---|
| Discovery (`loft search`) | Client-side grep of the index | Server endpoint |
| Auth on publish | PR review on the index repo | Server token auth |
| Atomic publish | git merge serialisation | DB transaction |
| Yanking a version | Edit the index file | API call |
| Download analytics | None | DB rows |
| Hosting cost | $0 (GitHub Pages / raw) | $20+/mo for a tiny server |
| Time to ship | ~1 week | ~1 quarter |
| Migration cost (later) | Server reads/writes same JSON shape | n/a |

The MVP buys us the per-library extraction work (Phase 4-8 in
[`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction/))
without funding a service.

---

## Index format — `registry.json`

A single JSON file, hosted at a stable URL.  Suggested:
`https://loft-lang.org/registry.json` (or
`https://raw.githubusercontent.com/loft-lang/registry/main/index.json`
for the lowest-infra option).

### Decoupled lifecycle — Debian-style "the registry IS a repo"

The registry lives in **its own GitHub repository**, completely
separate from the `loft-lang/loft` compiler repo.  This decouples
two lifecycles that have no reason to be linked:

| Lifecycle | Owner | Cadence |
|---|---|---|
| Loft compiler releases | `loft-lang/loft` (this repo) | Minor every 1-3 months |
| Package index updates | `loft-lang/registry` (separate repo) | Anytime — driven by package author publishes |

Mirrors apt's model: Debian's `Packages.gz` lives on mirror servers,
updated independently from `dpkg`/`apt` binary releases.  Anyone can
host a mirror by cloning the package list; the user picks which
mirror to consult via `sources.list`.  Loft's `LOFT_REGISTRY_URL`
env var (defaults to the canonical
`raw.githubusercontent.com/loft-lang/registry/main/index.json`)
plays the same role.

**Consequences this gets us for free**:

1. **Publishing a new package doesn't require a loft release.**  An
   author tags v0.1.0, opens a PR against `loft-lang/registry`, and
   the package is live for everyone the moment the PR merges.  No
   waiting for the next loft minor.
2. **Mirrors are trivial.**  Anyone with a GitHub account can fork
   `loft-lang/registry`, run their own additions, and point users
   at it via `LOFT_REGISTRY_URL=https://raw.githubusercontent.com/<user>/registry/main/index.json`.
   Useful for corporate / air-gapped deployments.
3. **The loft binary is small.**  It does not bundle a package list
   (which would go stale immediately).  The list is fetched at
   install time.
4. **History is git.**  Every package version add / yank is a
   commit in `loft-lang/registry`.  `git log` IS the audit trail.
   No separate publish-log infrastructure.
5. **CI runs against the registry repo independently** — same
   GitHub Actions style as any other repo.  PR validation
   (schema lint, sha256 verify) is its own workflow file in that
   repo.
6. **Bisecting a regression is `git bisect` on the registry repo**
   — when "package X 0.2.0 broke my build", you can `git bisect`
   `loft-lang/registry` to find the commit that added the bad
   version (and roll back by reverting it).

### Anatomy of the `loft-lang/registry` repo

```
loft-lang/registry/
├── README.md                       (publish workflow + how to PR)
├── index.json                      (the canonical registry — what loft fetches)
├── schema/
│   └── index-v1.json               (JSON schema for the index — used by CI lint)
├── tools/
│   ├── validate.py                 (PR validation: schema lint + sha256 verify)
│   └── add_version.sh              (helper: appends a version row)
└── .github/
    └── workflows/
        └── pr-validate.yml         (CI: runs validate.py on every PR)
```

`index.json` is the only file `loft install` cares about.
Everything else is repo-internal infrastructure for keeping
`index.json` correct.

### Schema

```json
{
  "schema_version": 1,
  "updated": "2026-05-24T08:00:00Z",
  "packages": {
    "<pkg_name>": {
      "description": "Short one-line description (optional).",
      "homepage": "https://github.com/loft-lang/loft-<pkg> (optional)",
      "yanked": ["0.1.2", …],
      "versions": {
        "<semver>": {
          "url": "https://github.com/loft-lang/loft-<pkg>/releases/download/v<semver>/<pkg>-<semver>.tar.gz",
          "sha256": "<64-hex>",
          "size": <bytes>,
          "loft": ">=0.8",
          "deps": {
            "<dep_pkg>": ">=0.1"
          },
          "published": "2026-05-24T08:00:00Z"
        },
        …
      }
    },
    …
  }
}
```

### Field reference

| Field | Required | Notes |
|---|---|---|
| `schema_version` | yes | Integer.  Bump on incompatible schema changes.  Today: `1`. |
| `updated` | yes | ISO-8601 UTC timestamp of the last index modification.  Lets clients detect stale caches. |
| `packages.<name>.description` | no | Display string for `loft search` output. |
| `packages.<name>.homepage` | no | URL to the package's repo / docs. |
| `packages.<name>.yanked` | no | Array of yanked versions.  Yanked versions stay listed so `loft.lock`-pinned consumers don't break, but new installs / version resolution skip them. |
| `packages.<name>.versions.<semver>.url` | yes | Direct download URL for the tarball.  Convention: GitHub release asset. |
| `…sha256` | yes | Lowercase hex of the SHA-256 hash of the tarball bytes.  Verified post-download. |
| `…size` | yes | Byte length.  Bandwidth sanity check + early-abort on giant downloads. |
| `…loft` | yes | Required loft interpreter version.  `>=0.8` syntax mirrors `loft.toml::[package] loft`. |
| `…deps` | no | Inter-package dependencies.  Resolved during install; failures abort before any download. |
| `…published` | yes | ISO-8601 publish timestamp.  Audit trail. |

### Why JSON not TOML

The package manifests are TOML for human authoring.  The registry
index is JSON because it's machine-generated, never hand-edited, and
loft's existing `serde_json` (or `data.rs`'s JSON walker — see
[QUALITY.md § P54](QUALITY.md))
parses it without a TOML dependency on the client side.  Smaller
binary.

### Index size estimate

100 packages × 5 versions × ~400 bytes per version row = ~200 kB.
Fits comfortably in a single fetch.  At 10,000 packages × 20
versions, ~80 MB — still fine for a daily-refresh client cache.
The format scales fine for the lifetime of an MVP.

---

## Tarball format

Each package ships as a gzipped tarball produced by `loft package`
(the publish-side CLI command).  Layout inside the tarball:

```
<pkg>-<version>/
├── loft.toml              (the manifest — verbatim from source)
├── src/                   (loft source files, verbatim)
│   └── <pkg>.loft
├── native/                (optional — only for packages with native code)
│   ├── Cargo.toml
│   ├── build.rs
│   └── src/lib.rs
├── tests/                 (optional — tested by `loft install --tests`)
│   └── *.loft
└── README.md              (optional)
```

**Excluded** (covered by `loft package`'s default ignore list):
- `.git/`, `target/`, `.loft/`
- Anything in `.gitignore` (if present)
- IDE state (`.vscode/`, `.idea/`)

The tarball is the canonical install unit.  Extracted into
`~/.loft/registry/<pkg>-<version>/` on install.

### Sha256 + size verification

After download, the client:
1. Hashes the tarball bytes (SHA-256).
2. Compares to `versions.<v>.sha256` from the index.
3. Compares byte count to `versions.<v>.size`.
4. Either matches → extract; mismatch → abort with a clear error
   and leave the cache untouched.

Hash mismatch is treated as a hard failure (likely indicates a
corrupted download or a tampered mirror).  Size mismatch is the
same family of error.

---

## Local cache layout

`~/.loft/registry/` is the install root:

```
~/.loft/
├── registry/
│   ├── index.json               (cached copy of registry.json)
│   ├── index.json.fetched_at    (ISO-8601 — drives staleness check)
│   ├── crypto-0.1.0/            (extracted tarball)
│   │   ├── loft.toml
│   │   ├── src/
│   │   └── native/
│   ├── crypto-0.1.1/            (multiple versions coexist)
│   └── web-0.1.0/
└── packages.json               (consumer-facing: which versions are linked to which projects — optional, future)
```

**Index refresh policy**:
- TTL: 1 hour by default.
- Re-fetched if `index.json.fetched_at` is older than 1 hour OR if
  `loft install --refresh` is passed OR if a requested package is
  missing from the cached index.
- Atomic write: download to `index.json.tmp`, then rename.  Avoids
  half-written files corrupting future installs.

**Cache GC**: never automatic.  `loft install --gc` (future) prunes
versions no consumer references.  For the MVP, manual.

---

## Consumer's lock file — `loft.lock`

The consumer's project has a `loft.toml` declaring deps:

```toml
[package]
name = "my_app"
version = "0.1.0"

[dependencies]
crypto = ">=0.1"
web = "^0.1.0"
```

`loft install` resolves these against the registry and writes
`loft.lock` alongside `loft.toml`:

```toml
# Auto-generated by loft — do not edit by hand.
# Run `loft install` to update.
schema_version = 1

[[package]]
name = "crypto"
version = "0.1.0"
url = "https://github.com/loft-lang/loft-crypto/releases/download/v0.1.0/crypto-0.1.0.tar.gz"
sha256 = "abc123…"
source = "registry"

[[package]]
name = "web"
version = "0.1.0"
url = "…"
sha256 = "…"
source = "registry"
deps = ["crypto"]
```

Subsequent `loft` runs honour `loft.lock` exactly — same `sha256`
required.  Reproducible across machines / CI / collaborators.

`loft update [<pkg>]` re-resolves (one package, or all) and rewrites
the lock file.  CI uses `loft install --frozen` which refuses to
rewrite the lock and fails if it would.

`source = "registry"` is recorded so future sources
(`source = "git"`, `source = "path"` for local overrides) can
coexist.

---

## `loft install` flow

Driver: `loft install` (no args, in a directory with `loft.toml`)
OR `loft install <name>[@<version>]` (one-off install — adds to
`loft.toml`'s `[dependencies]` and installs).

### Pseudo-flow

```
1. Resolve dependency graph
   1.1 Read ./loft.toml — collect [dependencies]
   1.2 Read ./loft.lock if present — collect pinned versions
   1.3 Refresh index if stale (> 1h since last fetch or --refresh)
   1.4 For each dep:
       - If in loft.lock and the registry still has that version (not yanked),
         use the locked version
       - Else resolve highest non-yanked version satisfying the constraint
   1.5 Recurse into transitive deps from registry.json::deps
   1.6 Diamond resolution: pick highest version satisfying all
       constraints (cargo-style); fail if no version works

2. Plan downloads
   2.1 For each resolved (name, version):
       - If ~/.loft/registry/<name>-<version>/ exists AND its
         loft.toml matches registry data → already installed, skip
       - Else: queue download

3. Download + verify
   3.1 Parallel fetch tarballs (small pool, 4 at a time)
   3.2 For each: hash, compare to registry's sha256, fail loudly
       on mismatch
   3.3 Extract to ~/.loft/registry/<name>-<version>/

4. Write loft.lock
   4.1 Atomic rename: write loft.lock.tmp, rename to loft.lock
   4.2 Includes resolved versions, urls, sha256s, transitive deps

5. Print summary
   "Installed crypto 0.1.0, web 0.1.0 (2 packages)"
   "Resolved from registry.json (cached 14 min ago)"
```

### Error paths

| Condition | Behaviour |
|---|---|
| Registry URL unreachable | Use cached index if not stale; else fail with "registry unreachable, try `--offline` if you have a cache" |
| `--offline` flag | Never fetch; use cache; fail if a requested package isn't cached |
| Package not in registry | Fail with available alternatives (Levenshtein distance ≤ 2 on package names) |
| No version satisfies constraint | Print conflicting constraints with package + dep chain |
| sha256 mismatch | Delete partial download; fail; do NOT retry (could be a real attack) |
| `--frozen` + lock file would change | Fail with diff of intended changes |
| Loft version requirement not met | Fail with "package X requires loft Y, you have Z" |

### CLI surface

| Command | Behaviour |
|---|---|
| `loft install` | Honour loft.toml + loft.lock; install missing packages. |
| `loft install <name>` | Add `<name> = "^<latest>"` to loft.toml, install. |
| `loft install <name>@<version>` | Add `<name> = "<version>"` to loft.toml, install. |
| `loft install --refresh` | Force re-fetch the registry index. |
| `loft install --offline` | Use cache only; fail if anything's missing. |
| `loft install --frozen` | Fail if loft.lock would change.  CI mode. |
| `loft update` | Re-resolve all deps; rewrite loft.lock. |
| `loft update <name>` | Re-resolve only `<name>`. |
| `loft search <query>` | Client-side filter on index `description` + name. |
| `loft info <name>` | Print available versions, latest, homepage. |

`loft install` is idempotent — running it twice does nothing the
second time (cache hit, lock file unchanged).

---

## Publishing flow (MVP)

Manual PR-based.  Acceptable while loft maintainers are reviewing
every publish.

### Author side

1. Tag the release in the per-library repo (e.g.,
   `git tag v0.1.0 && git push --tags`).
2. Run `loft package` (new CLI command — minimal scope):
   - Reads `loft.toml` for name + version.
   - Builds the tarball from the package layout (excludes per
     [§ Tarball format](#tarball-format)).
   - Computes SHA-256 + byte size.
   - Prints:
     ```
     <pkg>-0.1.0.tar.gz       (size: 12.4 kB)
     sha256: abc123…
     ```
3. Attach the tarball to a GitHub release for the tag (`gh release
   create v0.1.0 <pkg>-0.1.0.tar.gz` — or a CI workflow does this
   automatically).
4. Open a PR against `loft-lang/registry` adding the version entry:

   ```diff
    "crypto": {
      "versions": {
   +    "0.1.0": {
   +      "url": "https://github.com/loft-lang/loft-crypto/releases/download/v0.1.0/crypto-0.1.0.tar.gz",
   +      "sha256": "abc123…",
   +      "size": 12698,
   +      "loft": ">=0.8",
   +      "published": "2026-05-24T08:00:00Z"
   +    }
      }
    }
   ```

### Maintainer side

1. CI validates the PR:
   - `schema_version` unchanged.
   - New entries follow the schema (script-lints required fields).
   - `url` is HTTP-accessible (HEAD request succeeds).
   - `sha256` matches the actual file at `url` (downloads + hashes;
     <30s per published tarball at typical sizes).
2. If validation passes, merge.
3. GitHub Pages (or raw.githubusercontent.com) serves the updated
   `registry.json` immediately.

The maintainer pipeline is just GitHub Actions + branch protection
+ a small Python validator script.  No custom service.

### Yanking

PR removes the version from `versions` and adds it to the package's
`yanked` array.  Yanked versions stay listed (so `loft.lock` pins
don't break) but new installs / version resolution skip them.

---

## Migration to a real server (later)

**Hard constraint: the user-visible behaviour must NOT change.** See
[§ The invariant](#the-invariant--end-user-experience-is-identical-to-a-real-server)
above.  Migration is purely an infrastructure swap.

Two paths, both compatible with the MVP's URL surface:

### Path 1 — Server backs the same JSON file

The server reads from a database, serves `GET /registry.json` with
the same shape.  Existing loft clients in the wild keep working
without recompilation or config change.  New features (search
endpoint, publish API, signing) layer on top:

| Endpoint | Purpose |
|---|---|
| `GET /registry.json` | Existing.  All clients hit this. |
| `GET /v1/search?q=` | New.  Server-side index, faster than client-grep at scale. |
| `POST /v1/publish` | New.  Replaces the manual PR; auth via token. |
| `POST /v1/yank` | New.  Replaces yank PRs. |
| `GET /v1/packages/<name>` | New.  Single-package metadata (skip the full index). |

### Path 2 — Stay file-based, add tooling

Some ecosystems (Homebrew, AUR) are file-based forever.  If loft's
ecosystem stays small (~100 packages), the MVP's PR-based publish
flow may never need replacing — just add tooling to automate the
PR process (a GitHub App that auto-opens a PR when a release tag
pushes).

**Decision deferred.**  Path 1 vs Path 2 is a 1.x decision driven
by actual ecosystem growth.  The MVP commits to neither.

---

## Implementation phases (suggested order)

| Phase | Item | Effort | Blocker |
|---|---|---|---|
| **R1** | `loft package` CLI — produce tarball + sha256 from a `loft.toml` package | S | none |
| **R2** | `loft.lock` schema + writer (PKG.7 from PACKAGES.md) | S | none |
| **R3** | Bootstrap empty `registry.json` in `loft-lang/registry` repo | XS | none |
| **R4** | `loft install <name>[@<v>]` — index fetch, resolve, download, verify, extract | M | R1, R2, R3 |
| **R5** | `loft install` (no args) — read project loft.toml, install per lock | S | R4 |
| **R6** | `loft update [<name>]` — re-resolve | S | R4 |
| **R7** | Diamond / transitive resolution | S | R4 |
| **R8** | `loft search`, `loft info` — client-side index queries | XS | R4 |
| **R9** | CI validation script for `loft-lang/registry` PRs | XS | R3 |
| **R10** | First real publish: `lib_plans/12-library-extraction` Phase 4 (`loft-libs-core` chunk) extracts crypto + arguments + random + shapes | M | R1-R9 |

Total MVP scope to "PKG.REG done": **R1 through R9**.  R10 is
the first user of the registry, owned by plan-12.

---

## What this does NOT cover

Out of scope for the file-based MVP — captured here so future
contributors don't redesign each on the spot.

1. **Auth on publish.** Acceptable while maintainers review every
   PR.  Token-based auth lives in Path 1's server.
2. **Package namespaces (e.g. `@user/pkg`)**.  Defer until naming
   conflicts emerge.
3. **Cargo-style features** (`[features]` per dependency).  Loft
   packages don't have feature flags today.
4. **Pre-built native binaries** (PACKAGES.md Open Q #14).
   Tarballs ship loft + Rust source; the consumer's machine builds
   the cdylib via the same `loft-ffi-build`-driven build.rs as the
   monorepo libraries.  Pre-built distribution is a future
   acceleration.
5. **Multi-registry support** (alternative registries / private
   mirrors).  Single registry URL hardcoded in the loft binary.
   Multi-registry is Path 1's job.
6. **Signing (Ed25519 / sigstore).** SHA-256 hash + HTTPS download
   covers integrity for the MVP.  Cryptographic signing is a Path 1
   feature.

---

## Open questions

These need decisions before implementation starts.

1. **Registry URL.**  `loft-lang.org/registry.json` (DNS-controlled,
   loft-lang owned) vs `raw.githubusercontent.com/loft-lang/registry/main/index.json`
   (zero infra, fully GitHub-backed).  Recommendation: start with
   the raw GitHub URL (zero cost, zero ops); add the DNS alias when
   ecosystem maturity justifies it.  Either way the URL is
   overridable via `LOFT_REGISTRY_URL` env var to enable mirrors,
   private registries, and per-CI pinned snapshots.
2. **Loft-version requirements.**  Should the registry index
   include the loft version requirement, or read it from the
   tarball's `loft.toml` post-extract?  Recommendation: include it
   in the index so the client can fail BEFORE downloading an
   incompatible version.
3. **Index cache TTL.**  1 hour as default — too long? too short?
   The registry is small + cheap to refetch; 1 hour seems fine.
   Settable via env var (`LOFT_REGISTRY_TTL=300` for 5 min).
4. **Native package distribution.**  Today the tarball ships the
   Rust `native/` source; the consumer builds.  Future: distribute
   pre-built cdylibs per platform via the same release.  Out of MVP
   scope.
5. **First-class deps.**  Should the index's `deps` field carry
   version constraints, or just package names?  Cargo carries
   constraints (`crypto = ">=0.1"`); loft should mirror for parity.

---

## See also

- [PACKAGES.md](PACKAGES.md) — package format reference; this doc
  is the registry-specific draft of that doc's "Open work" PKG.REG
  bullet.
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction/) —
  consumer of PKG.REG.  Phases 4-8 unblock when PKG.REG ships.
- [STDLIB.md § Logging](STDLIB.md) — `loft install` should log via
  the same machinery; useful for `--verbose` output.
