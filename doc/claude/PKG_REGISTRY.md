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

**Revision 3 (2026-05-24)** — switched index signing from CI-based
(GitHub Actions + `REGISTRY_SIGNING_KEY_BASE64` secret) to
**maintainer-laptop-based**.  Same crypto (Ed25519); private key
now never leaves hardware the maintainer physically controls.
`loft-keygen sign` / `loft-keygen verify` subcommands added.
Removed `sign-and-commit.yml` + `sign-index.py` from the CI
template directory.  Rationale in [§ Why laptop signing, not CI](#why-laptop-signing-not-ci).

**Revision 2 (2026-05-24)** — Debian-comparison pass added.
Promoted index signing from "Path 1 server feature" to "MVP R3.5"
(real security gap closed).  Added schema slots for `conflicts` /
`replaces` / `provides` / `binaries` / `prerelease` / `categories`
(Debian-inspired, reserved fields, resolver-side support deferred
to keep MVP scope tight).  Explicit "not adopted" table records
items deliberately rejected (pre/postinst scripts, debconf,
alternatives, triggers, epoch versioning, `main`/`contrib`/`non-free`)
so future PRs aren't re-litigated.  See
[§ Comparison to Debian](#comparison-to-debian--what-we-adopted-what-we-didnt).

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
      "categories": ["crypto", "stdlib-augment"],
      "yanked": ["0.1.2", …],
      "versions": {
        "<semver>": {
          "url": "https://github.com/loft-lang/loft-libs-<domain>/releases/download/<pkg>-v<semver>/<pkg>-<semver>.tar.gz",
          "sha256": "<64-hex>",
          "size": <bytes>,
          "loft": ">=0.8",
          "subpath": "<pkg>",
          "deps": {
            "<dep_pkg>": ">=0.1"
          },
          "conflicts": [],
          "replaces": [],
          "provides": [],
          "binaries": {},
          "prerelease": false,
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
| `packages.<name>.categories` | no | Array of category tags for discovery.  Free-form; `loft search --category crypto` filters on them.  Inspired by Debian's `Section:` field and cargo's `categories = […]`.  Empty by default; future curation can establish a canonical set. |
| `packages.<name>.yanked` | no | Array of yanked versions.  Yanked versions stay listed so `loft.lock`-pinned consumers don't break, but new installs / version resolution skip them. |
| `packages.<name>.versions.<semver>.url` | yes | Direct download URL for the tarball.  Convention: GitHub release asset. |
| `…sha256` | yes | Lowercase hex of the SHA-256 hash of the tarball bytes.  Verified post-download. |
| `…size` | yes | Byte length.  Bandwidth sanity check + early-abort on giant downloads. |
| `…loft` | yes | Required loft interpreter version.  `>=0.8` syntax mirrors `loft.toml::[package] loft`. |
| `…subpath` | for monorepos | Package directory within the release repo (e.g. `crypto` inside `loft-libs-core`).  The loft-lang libraries are **domain monorepos** (`loft-libs-core`/`-net`/`-graphics`/`-game`/`-world`/`-assets`/`-docs`) tagged `<pkg>-v<version>`, so `subpath` tells the installer where the package lives in the unpacked tarball.  Omit for a one-repo-per-package layout. |
| `…deps` | no | Inter-package dependencies.  Resolved during install; failures abort before any download. |
| `…conflicts` | no | **(Schema slot — resolver support deferred.)** Array of package names + version constraints that cannot coexist with this version in the same dependency graph.  Inspired by Debian's `Conflicts:`.  Reserved field so the schema doesn't need a bump when the resolver gains support. |
| `…replaces` | no | **(Schema slot — resolver support deferred.)** Array of packages this version takes over from (rename / fork takeover).  Inspired by Debian's `Replaces:`. |
| `…provides` | no | **(Schema slot — resolver support deferred.)** Array of virtual capability names this version supplies.  Lets a different package satisfy the same `deps` constraint — e.g. `crypto-bcrypt` and `crypto-argon2` both provide `password-hash`.  Inspired by Debian's `Provides:`. |
| `…binaries` | no | **(Schema slot — pre-built distribution deferred.)** Map of `<target-triple>` → `{url, sha256}` pointing at pre-built cdylibs.  When present and a triple matches, `loft install` skips the local `cargo build` step.  When absent, consumer builds from the `native/` source in the tarball.  Inspired by Debian's per-arch `.deb` files. |
| `…prerelease` | no | Boolean.  `true` for beta / rc versions.  `loft install <pkg>` (no version) skips prereleases by default; `loft install <pkg>@<v>` honours an explicit pin; `loft install <pkg>@beta` resolves the latest prerelease.  Inspired by Debian's `testing` / `unstable` release pockets. |
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

`~/.loft/` holds everything `loft install` writes to disk on the
consumer's machine.  Layout:

```
~/.loft/
├── trust-root/
│   └── registry-signing-key.bin       (private key — maintainer-only,
│                                       absent on consumer machines)
├── registry/
│   └── <registry-url-hash>/
│       ├── index.json                 (cached registry index)
│       ├── index.json.sig             (signature, verified on every load)
│       ├── index.json.fetched_at      (ISO-8601 — staleness clock)
│       └── index.json.etag            (HTTP conditional-GET hint)
├── packages/
│   └── <package>/
│       └── <version>/                 (extracted tarball, ready to compile)
│           ├── loft.toml
│           ├── src/
│           └── native/
└── cache/
    └── <sha256>.tar.gz                (raw downloaded tarballs — CAS-keyed
                                        so duplicate SHAs across packages /
                                        registries de-duplicate naturally)
```

**Why this shape**:

- **One `<registry-url-hash>/` subdir per registry URL.**  Lets the
  client run against `loft-lang/registry` (default) plus mirrors
  (`LOFT_REGISTRY_URL=...`) without their indexes colliding.  Hash is
  short (8 hex chars of SHA-256 of the URL).
- **`packages/<name>/<version>/` is the install address.**  When loft
  resolves `crypto = "0.1.0"`, it looks for
  `~/.loft/packages/crypto/0.1.0/loft.toml`.  Multiple versions
  coexist by design — project A pinned to 0.1.0 and project B pinned
  to 0.2.0 both work without re-download.
- **`cache/<sha256>.tar.gz` is content-addressable.**  The same
  bytes are stored once even if multiple registry entries (or
  mirrors) reference them.  Also gives the trust-root re-validation
  chain a stable target — the SHA-256 in the lockfile points at the
  same bytes years later.
- **Atomic install**: download to `cache/<sha>.tar.gz.tmp`, hash-verify,
  rename to final.  Extract to `packages/<name>/<version>.tmp/`,
  rename to final.  No half-written state survives a crash.

**Index refresh policy** (per registry URL):

- TTL: 1 hour by default.
- Re-fetched when `index.json.fetched_at` is older than 1 hour, OR
  when `loft install --refresh` is passed, OR when a requested
  package is missing from the cached index.
- Conditional GET via the cached `index.json.etag` — if the
  registry hasn't changed, the response is 304 and no bytes
  transfer.
- Signature verified on every load, even cache-hit paths.

**Cache lifecycle — no automatic cleanup, conservative by design**:

- **Old versions stay** when newer versions install.  Lockfile
  reproducibility requires this: project A pins `crypto = 0.1.0`,
  project B pins `crypto = 0.2.0`, both coexist forever in
  `packages/crypto/`.  Auto-deleting on newer install would
  silently break A's offline rebuild.
- **Re-download is allowed**, not forced.  If `cache/<sha>.tar.gz`
  is deleted but `packages/<name>/<version>/` survives, the
  extracted tree is still usable; cache file is re-fetched only
  when needed for verification.

**Manual cleanup commands** (MVP scope marked):

| Command | Effect | Status |
|---|---|---|
| `loft cache list` | Show what's in `~/.loft/{packages,cache}/` with sizes | MVP |
| `loft cache clean` | Wipe `~/.loft/{registry,packages,cache}/` (nuclear) | MVP |
| `loft cache prune` | Drop entries not referenced by any `loft.lock` in `$PWD` (recursive scan) | v0.2 |
| `loft cache prune --scan <dir>` | Same with explicit roots | v0.2 |

Why prune lands later: lockfile-aware GC is easy to get wrong (a
lockfile in a stale clone shouldn't pin a tarball forever).  Ship
the nuclear option first; iterate on the conservative one once
real-world feedback is in.

**Disk cost rough estimate**:

A typical package = 30 kB tarball + ~150 kB extracted.  100
packages with ~3 versions each = ~50 MB total.  Negligible on a
modern machine; visible-but-fine on a constrained dev VM.

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

**Author-facing guide**: [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md)
walks through the same flow step-by-step with troubleshooting
for common failure modes (sha256 mismatch, reproducible-build
mismatch).  The section below is the design/reference
description; that doc is what you'd hand to a contributor.

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
   +    "0.2.1": {
   +      "url": "https://github.com/loft-lang/loft-libs-core/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz",
   +      "sha256": "abc123…",
   +      "size": 12698,
   +      "loft": ">=0.8",
   +      "subpath": "crypto",
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

## Index signing — `index.json.sig`

Closes a real security gap in the file-based design.  Without
signing, the chain of trust is HTTPS → `index.json` → tarball
sha256 — but **nothing protects the index itself**.  An attacker
who compromises the registry repo or MITM's the HTTPS connection
can serve a modified `index.json` pointing at malicious tarballs
with matching (recomputed) sha256s.

Borrowed from Debian's `Release.gpg` / `InRelease`.

### How it works

1. Loft maintainers hold a single trust-root signing key (Ed25519,
   stored on a trusted laptop + hardware-token backups, NOT in any
   service's secret manager).
2. The loft binary embeds the **public key** at compile time
   (`src/registry_keys.rs`, ~50 bytes).  Multiple keys can be
   embedded for rotation or for multi-maintainer setups; clients
   accept signatures from any listed key.
3. **Signing happens locally on the maintainer's laptop**, NOT in
   CI.  Per accepted PR against `loft-lang/registry`:
   - Maintainer reviews + checks out the PR branch.
   - Maintainer runs `loft-keygen sign --in index.json --key
     <private>.bin --out index.json.sig` to produce a raw
     64-byte Ed25519 signature.
   - Maintainer commits `index.json.sig` and merges the PR.
4. Clients fetch both files.  Verify the signature over the bytes
   of `index.json`.  Refuse to use an unsigned/invalid-sig index
   unless `--allow-unsigned` is passed.

### Why laptop signing, not CI

Two reasons the maintainer signs locally instead of in GitHub
Actions:

- **No third-party trust dependency.**  CI signing would require
  storing the private key as a `REGISTRY_SIGNING_KEY_BASE64`
  GitHub secret — adding GitHub's secret-management to the trust
  chain.  Local signing keeps the key on hardware the maintainer
  physically controls.
- **Human in the loop.**  A maintainer reviewing + signing the
  merged commit IS the audit trail.  CI signing would sign
  whatever lands in `main` — no human gate.

Cost: ~30 seconds per merge.  For an early-stage ecosystem with
weekly publishes that's negligible.  When the ecosystem
outgrows laptop signing, the architecture migrates to Path 1
(real server) and signing follows.

### Two-stage bootstrap — interim `K_tmp` → permanent `K_real`

The registry MVP needs a single Ed25519 secret to sign
`index.json`.  Where that secret *lives* is independent of the
registry going live.  Running the full publish → install →
verification flow against a throwaway key (`K_tmp`) before the
permanent hardware-backed key (`K_real`) is ready is a deliberate
pattern, not a shortcut:

- **Interim (`K_tmp`)**: generate on the trusted dev laptop, no
  hardware backup, no off-site copies.  Use it to validate the
  full pipeline (loft-lang/registry online, first PR + sign +
  merge cycle, `loft install` against the live URL).  **Do not
  ship a public loft release with `K_tmp` embedded** — only the
  maintainer's local loft trusts it; blast radius is one
  machine.
- **Final (`K_real`)**: full 3-2-1 storage per
  [REGISTRY_BOOTSTRAP.md § Step 1.5](REGISTRY_BOOTSTRAP.md#step-15--store-the-private-key-for-the-long-haul).
  First public loft release embeds `K_real` in
  `TRUSTED_PUBLIC_KEYS`.

**Going from interim → final** is mechanically identical to the
"Compromised key" path: generate `K_real`, embed it in
`TRUSTED_PUBLIC_KEYS` removing `K_tmp`, re-sign every signed
index/asset with `K_real`, ship the public loft release.  The
recovery runbook for this is
[REGISTRY_RECOVERY.md § Scenario C](REGISTRY_RECOVERY.md#scenario-c).

Running this transition as a *planned* rotation has two
benefits:

- The end-to-end registry mechanic gets validated against a
  realistic load (real GitHub repo, real release assets, real
  install) before the trust root is consequential.
- The recovery procedure that's currently a runbook gets a real
  dry-run before you need it in anger.

### Multi-maintainer support

`TRUSTED_PUBLIC_KEYS` is `&[[u8; 32]]` — a slice.  Adding a
co-maintainer's public key alongside the primary key is a
one-line edit + a loft minor release.  Each maintainer signs on
their own laptop with their own private key.  Clients verify
against any embedded key.  No shared secrets; no privileged
"signing role" to compromise; bus-factor mitigated by having
multiple keys live in parallel.

For the bootstrap, one key is fine — the multi-key path is the
extension point when a co-maintainer joins.

### Key rotation

Add the new public key to a freshly-tagged loft binary release.
Sign the index with BOTH old + new keys for a transition window
(say, 3 months).  Once enough of the ecosystem has upgraded, drop
old-key signatures.  Compromised key: bump loft minor, embed only
the new key, distrust the old key via `~/.loft/distrusted_keys`
override.

### Why Ed25519, not GPG

GPG is a deep rabbit hole (web of trust, key servers, expiry,
revocation).  Ed25519 is one signature, one public key, ~120
lines of `ed25519-dalek` to verify.  Same security guarantee for
the threat model loft cares about (integrity of the index from a
single trusted root).

### Implementation phases — slotted into the R-list

* **R3.5** (between R3 bootstrap and R4 install) — add the
  `loft-keygen sign` + `loft-keygen verify` subcommands; embed
  the public key in the loft binary's `TRUSTED_PUBLIC_KEYS`.
  Required for R4 to verify on install.
* **R10.5** (after R10 first publish) — first real key rotation
  drill to validate the rotation path before key compromise
  happens for real.

---

## Reproducible-build verification at the registry

Borrowed from Debian's reproducible-builds initiative.  R1's
`loft package` already produces deterministic tarballs (same
source dir → same sha256 across runs).  Promote this from
"nice property" to "registry-side enforcement":

* When a PR adds a new version entry to `index.json`:
  1. The CI workflow checks out the package's tagged source
     (`git clone --depth=1 --branch v<version> <homepage>`).
  2. Runs `loft package` on the checkout.
  3. Compares the produced sha256 to the PR's claimed sha256.
  4. Rejects the PR on mismatch.

This catches:
- Honest mistakes (publisher manually edited the tarball,
  forgot to repackage).
- Malicious publishers (PR claims hash X but the published
  GitHub-release tarball has hash Y — only the consumer
  hash-check catches this today; the CI gate catches it at
  PR time).

Caveat: doesn't catch supply-chain attacks on the upstream
source itself (compromised git tags, etc.).  That's a different
trust problem (sigstore / cosign / `attestations.json`); out of
MVP scope.  Reproducible-build verification is the cheap layer
that handles the common honest-mistake / opportunistic-attack
class.

Lives in `R9` (registry-PR validator) — schema-wise no change;
it's a CI workflow addition.

---

## Comparison to Debian — what we adopted, what we didn't

Surveyed Debian/apt's ecosystem for prior art.  Decisions:

### Adopted in the MVP (schema-level)

| Debian concept | Loft equivalent | Where |
|---|---|---|
| `Release.gpg` / `InRelease` (signed index) | `index.json.sig` (Ed25519) | [§ Index signing](#index-signing--indexjsonsig) — phase R3.5 |
| `Section:` field (categorisation) | `packages.<name>.categories` | Schema field, free-form tags |
| Reproducible-build pledge | `loft package` deterministic sha256 + CI re-verify | R1 + R9 |
| Source-build install (consumer compiles native) | Default behaviour; pre-built optional via `binaries` field | Schema slot |
| `apt-get update` (separate index refresh) | Cached index with 1h TTL + `--refresh` flag | [§ Local cache layout](#local-cache-layout) |
| Mirrors via `sources.list` | `LOFT_REGISTRY_URL` env var | [§ Decoupled lifecycle](#decoupled-lifecycle--debian-style-the-registry-is-a-repo) |

### Adopted as schema slots — resolver support deferred

These get reserved fields NOW so the schema doesn't need a bump
when implementation lands.  No client-side resolver changes
required for MVP.

| Debian concept | Loft schema slot | When to implement |
|---|---|---|
| `Conflicts:` | `versions.<v>.conflicts: []` | When two real packages can't coexist. |
| `Replaces:` | `versions.<v>.replaces: []` | When a package is renamed / forked. |
| `Provides:` (virtual packages) | `versions.<v>.provides: []` | When alternative implementations of the same capability appear. |
| Per-arch `.deb` files | `versions.<v>.binaries: {<triple>: …}` | When local `cargo build` becomes the install bottleneck. |
| `testing` / `unstable` release pockets | `versions.<v>.prerelease: bool` | When a package author wants a beta channel. |

### Not adopted — incompatible with loft's model

| Debian concept | Why we don't want it |
|---|---|
| Pre/post install scripts (`preinst`, `postinst`) | Security disaster: arbitrary code execution at install time.  Loft packages stay as data + Rust source + loft source — no install scripts.  Compilation of the Rust crate is the *only* code path that runs (and it's `rustc`, not the package's own scripts). |
| `debconf` (interactive config) | Wrong UX for a programming-language ecosystem; install must be scriptable + non-interactive. |
| `dpkg-divert` / `update-alternatives` | Solves file-conflict resolution for system-wide installs.  Doesn't apply — each loft package lives in its own `~/.loft/registry/<pkg>-<version>/` directory; no global file conflicts possible. |
| Triggers (one package reacts to another's install/remove) | Overkill for a programming-language ecosystem.  Real use case has yet to emerge. |
| Epoch versioning (`1:2.3.4-5`) | Debian needed this because some upstreams reset their version numbers.  Semver covers loft's case; no epoch required. |
| `main` / `contrib` / `non-free` section split (license tiers) | Becomes relevant when packages with restrictive licenses appear.  Current ecosystem is uniformly LGPL/MIT/Apache; a single tier is fine.  `categories` field above can carry a `non-free` tag if/when needed. |
| Source vs binary package split (`.dsc` + tarball vs `.deb`) | Our tarball bundles both — source IS the install unit.  When pre-built distribution lands via the `binaries` schema slot, it'll be an OPTIONAL acceleration, not a separate package type. |

This split is **load-bearing**: future PRs that propose any of the
"not adopted" items should be redirected here.  The rationale is
recorded so the decision doesn't get re-litigated.

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
| `GET /index.json.sig` | Existing.  Ed25519 signature; clients verify with embedded public key.  Server signs on every update with the same maintainer key the MVP used. |
| `GET /v1/search?q=` | New.  Server-side index, faster than client-grep at scale. |
| `POST /v1/publish` | New.  Replaces the manual PR; auth via token.  Server still produces a signed `index.json` after each publish; signing key lives in the server's HSM. |
| `POST /v1/yank` | New.  Replaces yank PRs. |
| `GET /v1/packages/<name>` | New.  Single-package metadata (skip the full index). |
| `GET /v1/attestations/<pkg>-<v>` | Future.  Per-tarball publisher attestations (sigstore-style) — finer-grained than the single trust-root signing the MVP ships. |

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
| **R1** | `loft package` CLI — produce tarball + sha256 from a `loft.toml` package | S | **DONE 2026-05-24** — `src/package.rs` + `loft package` subcommand.  4 unit tests + smoke-tested on `lib/crypto` (5.6 kB) and `lib/web` (21.5 kB), sha256 stable across runs. |
| **R2** | `loft.lock` reader/writer | S | **DONE 2026-05-24** — `src/lockfile.rs` (NOT feature-gated; lockfile is read on every loft invocation).  Atomic-rename writer, schema_version pin, 11 unit tests. |
| **R3** | Registry repo bootstrap docs | XS | **DONE 2026-05-24** — `doc/claude/REGISTRY_BOOTSTRAP.md` runbook + `doc/claude/registry_sample.json` template.  The actual GitHub repo creation is a maintainer-driven manual op. |
| **R3.5** | Index signing (Ed25519) | S | **DONE 2026-05-24; trust root BOOTSTRAPPED 2026-06-14 (PR #371)** — `src/registry_keys.rs` `TRUSTED_PUBLIC_KEYS` now holds **3 independent keys** (1 software laptop + 2 on-card YubiKeys; [REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md)), `src/registry_signing.rs` `verify_index` (4 tests). Live index signed; `scripts/registry-sign.sh` is the review-then-sign path. Activates on the next loft release. |
| **R4** | `loft install <name>[@<v>]` — index fetch, sig verify, resolve, download, extract | M | **DONE 2026-05-24** — `src/registry_index.rs` (schema + parser + version constraint resolver + HTTPS fetcher + tarball extractor, 12 tests) + `src/install.rs` (orchestrator, 3 tests).  CLI flags wired: `--refresh`, `--offline`, `--prerelease`, `--allow-unsigned`, `--require-signature`.  Falls back to legacy text-format registry when `LOFT_LEGACY_REGISTRY` is set (preserves existing tooling). |
| **R5** | `loft install` (no args; reads project loft.toml) | S | **DONE 2026-05-24** (subsumed by R4 — `install_one` consults project `loft.toml`'s `[dependencies]` via the resolver, writes `loft.lock` atomically). |
| **R6** | `loft update [<name>]` | S | **DEFERRED** — re-runs the R4 flow with `--refresh`; needs a one-line subcommand to invalidate the lockfile pin for `<name>` before resolution.  Trivial extension once the registry is live; not blocking other phases. |
| **R7** | Diamond / transitive resolution | S | **DONE 2026-05-24** — `install::resolve_recursive` walks `deps` from each resolved version.  Diamond conflict detection (re-check the new constraint against the existing pin) is the next refinement; today the resolver picks the FIRST satisfying version per name. |
| **R8** | `loft search`, `loft info` | XS | **DONE 2026-05-24** — `loft search [query]` (case-insensitive match on name + description + categories) and `loft info <name>` (homepage, categories, latest, deps, version table with yanked/prerelease tags). |
| **R9** | Registry CI validator | S | **DONE 2026-05-24** — template scripts at `doc/claude/registry_ci_template/` (validate.py, pr-validate.yml, README, registry_README.md).  Drop into `loft-lang/registry` once bootstrapped.  Signing happens locally on the maintainer laptop via `loft-keygen sign`, NOT in CI — see [§ Why laptop signing, not CI](#why-laptop-signing-not-ci). |
| **R2** | `loft.lock` schema + writer (PKG.7 from PACKAGES.md) | S | none |
| **R3** | Bootstrap empty `registry.json` in `loft-lang/registry` repo | XS | none |
| **R3.5** | Index signing (`index.json.sig`) + public-key embed in loft binary.  Borrowed from Debian's `Release.gpg`.  See [§ Index signing](#index-signing--indexjsonsig). | S | R3 |
| **R4** | `loft install <name>[@<v>]` — index fetch, **signature verify**, resolve, download, verify tarball sha256, extract | M | R1, R2, R3, R3.5 |
| **R5** | `loft install` (no args) — read project loft.toml, install per lock | S | R4 |
| **R6** | `loft update [<name>]` — re-resolve | S | R4 |
| **R7** | Diamond / transitive resolution | S | R4 |
| **R8** | `loft search`, `loft info` — client-side index queries | XS | R4 |
| **R9** | CI validation script for `loft-lang/registry` PRs — schema lint + sha256 verify + **reproducible-build re-check** (rebuild from source, compare sha256) | S | R1, R3 |
| **R10** | First real publish: `lib_plans/12-library-extraction` Phase 4 (`loft-libs-core` chunk) extracts crypto + arguments + random + shapes | M | R1-R9 |

Total MVP scope to "PKG.REG done": **R1 through R9** (including
the new R3.5 signing phase).  R10 is the first user of the
registry, owned by plan-12.

| **R10.5** | First real key-rotation drill — exercise the rotation path while no real compromise is happening. | XS | R3.5, R10 |

### Status: MVP code complete 2026-05-24

R1-R9 (excluding R6 `loft update`, deferred until live registry
exists) all DONE in the client binary.  The remaining work is
ECOSYSTEM bootstrap:

1. Maintainer runs [REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md)
   — generate Ed25519 keypair offline, embed pubkey in
   `src/registry_keys.rs`, create `loft-lang/registry` repo, ship
   CI templates from
   [registry_ci_template/](registry_ci_template/), seed empty
   `index.json`.
2. Loft minor release with the embedded trust root.
3. First publish (R10) — typically `crypto` from
   [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction/)
   Phase 4.
4. R10.5 key-rotation drill before any real compromise.

### Coverage check — the registry must not drift behind the repos

`scripts/check_registry_coverage.sh` (loft repo) compares every
`loft-lang/loft-libs-*` library's `loft.toml` version against the
published `index.json` and warns on **missing** (no entry at all) and
**stale** (repo version newer than the newest published one) libraries.
It runs per-PR as the advisory CI job "library registry coverage"; run
it locally with `--strict` (exit 1 on findings) as a pre-publish gate.
The fix for any finding is the publish flow in
[REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md).  Libraries still inside the
loft repo's `lib/` are out of scope — they are unextracted by design
(PKG.EXTRACT).

### Maintainer fast-path — one run, one OK, one signature

`scripts/registry_maintain.sh` turns the findings into a single
sitting.  One run gathers the combined worklist — **own libs** to
publish (the coverage findings), **foreign submission PRs** on
`loft-lang/registry` with their validation-CI verdict, and **foreign
upstream drift** (an author's repo ahead of the registry,
informational).  After one confirmation it merges the green PRs,
re-filters the worklist against the post-merge index (a PR may have
covered a finding), then for each remaining own lib runs
`loft package` → creates the tag + GitHub release when absent →
`loft publish` → merges the emitted entry into `index.json`.  The
maintainer signs once (`loft-keygen sign`; key via `--key` /
`LOFT_REGISTRY_KEY`) and the script commits, pushes, and re-runs the
coverage check to confirm a clean state.  Without a key it stops after
staging and prints the two remaining commands — the signature never
moves off the maintainer's hardware (§ Why laptop signing, not CI).
`--dry-run` shows the worklist and changes nothing.

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
6. ~~**Signing (Ed25519 / sigstore).** SHA-256 hash + HTTPS download
   covers integrity for the MVP.  Cryptographic signing is a Path 1
   feature.~~ **REVISED 2026-05-24** — index signing IS in the MVP
   via R3.5 (Ed25519 over `index.json`, single trust-root key).
   Per-tarball signing (sigstore / cosign / per-publisher keys) is
   still a Path 1 feature; the file-based MVP's trust model is "the
   index is signed by loft maintainers; that index attests to every
   tarball's sha256."

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
