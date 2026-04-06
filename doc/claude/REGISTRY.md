
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Package Registry

Design for a file-based package registry that maps library names and versions to
download URLs.  This is Phase 3 of the external library support described in
[EXTERNAL_LIBS.md](EXTERNAL_LIBS.md).

---

## Contents
- [Goals](#goals)
- [Registry File Format](#registry-file-format)
- [Registry File Locations](#registry-file-locations)
- [CLI Interface](#cli-interface)
- [Install Flow](#install-flow)
- [Registry Sync](#registry-sync)
- [Installed Package Check](#installed-package-check)
- [Version Resolution](#version-resolution)
- [Zip Package Layout](#zip-package-layout)
- [Security Considerations](#security-considerations)
- [Implementation](#implementation)
- [Phased Rollout](#phased-rollout)
- [Code Touchpoints](#code-touchpoints)

---

## Goals

1. A developer can run `loft install graphics` and get the library without
   manually downloading or placing files.
2. Different versions of the same library each have their own URL — there is
   no "latest pointer" file that needs to be updated server-side.
3. The registry is a plain text file — it can be hosted on any static file
   server, checked into a git repository, or maintained by hand.
4. The format is human-readable and editable without tooling.
5. No central authority is required.  Users can point to any registry file.

---

## Registry File Format

A registry file is a UTF-8 text file.  Each non-blank, non-comment line
declares one package version:

```
# source: https://raw.githubusercontent.com/jjstwerff/loft-registry/main/registry.txt
# Loft package registry
# Format: <name> <version> <url> [status]
#
# Lines starting with # are comments.  Blank lines are ignored.
# Entries are matched top-to-bottom; the first match wins
# when searching for an exact version.  For "latest", all
# active entries are compared by semver and the highest wins.

graphics 0.1.0 https://example.com/packages/graphics-0.1.0.zip yanked:CVE-2026-001
graphics 0.2.0 https://example.com/packages/graphics-0.2.0.zip
opengl   0.1.0 https://example.com/packages/opengl-0.1.0.zip   deprecated:use-graphics
math     1.0.0 https://example.com/packages/math-1.0.0.zip
math     1.1.0 https://example.com/packages/math-1.1.0.zip
```

### Fields

| Field | Description |
|-------|-------------|
| `name` | Package identifier — must match `[a-z][a-z0-9_]*` |
| `version` | Semver string `MAJOR.MINOR.PATCH` |
| `url` | HTTPS URL to a `.zip` file containing the package |
| `status` | Optional governance field — see below |

### Status field

| Value | Meaning |
|-------|---------|
| *(absent)* | Active — installable without warning |
| `deprecated:<slug>` | Installable but warns; skipped for "latest" if any active version exists |
| `yanked:<slug>` | Not installable; always skipped for "latest"; existing installs unaffected |

The `<slug>` is a short human-readable reason (e.g. `CVE-2026-001`, `outdated`,
`malicious`).  It appears verbatim in diagnostics.

### The `source:` directive

The first `# source: <url>` comment line in the file records where the file
itself was downloaded from.  `loft registry sync` reads this URL to know where
to fetch updates.

```
# source: https://raw.githubusercontent.com/jjstwerff/loft-registry/main/registry.txt
```

The URL points to the personal repository initially.  If the registry migrates
to a GitHub organisation (e.g. `loft-lang/registry`), the `source:` line in
the file is updated and users get the new URL automatically on their next sync —
no interpreter release is needed.

Rules:
- The `source:` line must be the first non-blank line of the file.
- Only one `source:` line is recognised; subsequent ones are plain comments.
- If absent, `loft registry sync` falls back to the `LOFT_REGISTRY_URL`
  environment variable, then the compiled-in default URL.
- The `source:` line is preserved verbatim when `sync` rewrites the file.
- Teams hosting a private registry change only this one line — all other
  registry mechanics (sync, check, install) work identically.

### Constraints

- Fields are separated by one or more ASCII spaces or tabs.
- Trailing whitespace on a line is ignored.
- The URL must start with `https://` or `http://`.
- A name may appear multiple times with different versions.
- Duplicate `(name, version)` pairs: first entry wins (top-to-bottom).
- Yanked entries are never removed — they stay in the file as a permanent
  auditable record with their `yanked:` status.

---

## Registry File Locations

The interpreter searches for a registry file in this order:

1. **`LOFT_REGISTRY` environment variable** — must be an absolute path to a
   local file.  Set this to use a team-internal or project-specific registry.
2. **`~/.loft/registry.txt`** — the user's personal registry, installed by
   the user or by a future `loft registry fetch` command.

If no registry file is found and the user runs `loft install <name>` (not a
local path), the command exits with a clear diagnostic:

```
loft install: no registry file found.
  Create ~/.loft/registry.txt or set LOFT_REGISTRY to a registry file path.
```

### Multiple Registries (future)

A future `loft registry` subcommand could merge multiple sources.  For Phase 3
a single file is sufficient.

---

## CLI Interface

### Installing from registry

```sh
loft install graphics            # install latest version from registry
loft install graphics@0.1.0      # install specific version
```

### Installing from local path (unchanged, Phase 1)

```sh
loft install .                   # install package in current directory
loft install /path/to/mypkg      # install from absolute path
loft install ../sibling          # install from relative path
```

The heuristic for distinguishing registry lookups from local paths:
- Argument starts with `/`, `./`, or `../` → local path.
- Argument contains a path separator (`/`) → local path.
- Otherwise → registry lookup, with optional `@version` suffix.

### Registry subcommands

```sh
loft registry sync              # download latest registry.txt from source URL
loft registry check             # compare installed packages against registry
loft registry list              # show all packages in registry
loft registry list --installed  # show only installed packages
```

### Updated help text

```
  install [target]              install a package to ~/.loft/lib/ for global use
                                install .        — install package in current dir
                                install /p       — install package at /p
                                install name     — download latest from registry
                                install name@v   — download specific version

  registry <subcommand>         manage the local package registry
                                sync             — pull latest registry from source URL
                                check            — report updates, deprecations, yanks
                                list             — browse all packages in registry
                                list --installed — show only installed packages
```

---

## Install Flow

For a registry install (`loft install graphics`):

```
1. Parse "graphics" → name="graphics", version=None
2. Find registry file (LOFT_REGISTRY or ~/.loft/registry.txt)
3. Read and parse registry file
4. find_package(entries, "graphics", None) → pick highest semver entry
5. Download zip from entry.url to a temporary file
6. Extract zip to a temporary directory
7. Locate the package root inside the extracted tree
   (directory containing loft.toml, or the root itself)
8. Call install_package(pkg_root) — existing Phase 1 logic
9. Clean up temporary directory
10. Print: "installed graphics 0.2.0 → ~/.loft/lib/graphics/"
```

For a versioned install (`loft install graphics@0.1.0`):

Steps 1–10 same, except step 4 uses `find_package(entries, "graphics", Some("0.1.0"))`
and step 6 is a hard error if the version is not found.

---

## Registry Sync

`loft registry sync` downloads the authoritative registry file from GitHub (or
a custom source URL) and replaces the local `~/.loft/registry.txt`.

### Sync flow

```
1. Determine source URL:
   a. Read LOFT_REGISTRY_URL env var — if set, use it.
   b. Read local ~/.loft/registry.txt for a "# source: <url>" first line.
   c. Fall back to compiled-in default:
      https://raw.githubusercontent.com/jjstwerff/loft-registry/main/registry.txt

2. Download the URL via HTTPS to a temporary file.

3. Validate the downloaded content:
   - Must be valid UTF-8.
   - Must contain at least one non-comment, non-blank line.
   - Basic format check: each data line must have three whitespace-separated fields.

4. If the download succeeds:
   - Replace ~/.loft/registry.txt with the downloaded content.
   - Print: "registry synced: 14 packages, 28 versions  (2026-04-04)"

5. If the download fails:
   - Leave the existing ~/.loft/registry.txt unchanged.
   - Print error to stderr and exit 1:
     "loft registry sync: download failed: <reason>"
     "  local registry is unchanged."
```

### First-time sync (no local registry)

If `~/.loft/registry.txt` does not exist, `loft registry sync` downloads from
`LOFT_REGISTRY_URL` or the compiled-in default (which tracks wherever the
official registry lives — personal repo or org) and creates the file.  A user
running `loft install` for the first time is directed to run sync first:

```
loft install: no registry file found.
  Run 'loft registry sync' to download the package registry.
  Or set LOFT_REGISTRY to a local registry file path.
```

### Staleness tracking

The file modification time of `~/.loft/registry.txt` is used as the sync
timestamp.  No separate metadata file is needed.

If the local registry is older than **7 days** when `loft registry check` is
run, a warning is printed before the check results:

```
warning: registry was last synced 9 days ago.
  Run 'loft registry sync' to get the latest security information.
```

This warning does not affect the exit code.

### Custom and private registries

Teams can host their own registry file anywhere and point to it:

```sh
export LOFT_REGISTRY=/path/to/company-registry.txt
loft registry sync   # syncs from the source: URL inside that file
```

Or permanently by placing a registry file with a custom `# source:` URL at
`~/.loft/registry.txt`.  The official registry and a custom registry can be
used together only if they are manually merged — a single local file is the
intended model.

---

## Installed Package Check

`loft registry check` scans `~/.loft/lib/` for installed packages, reads each
`loft.toml` for the installed name and version, and compares against the local
registry file.

### Check flow

```
1. Scan ~/.loft/lib/*/loft.toml — collect (name, version) for each installed pkg.
2. Read local registry file.
3. Warn if registry is older than 7 days (does not affect exit code).
4. For each installed package, classify:
   - yanked   — installed version has yanked:<slug> in registry
   - deprecated — installed version has deprecated:<slug> in registry
   - outdated — installed version is active but a higher active version exists
   - current  — installed version is the highest active version
   - unknown  — name not found in registry at all
5. Collect count of registry packages not installed (new packages available).
6. Print report (see below).
7. Exit 0 if no installed packages are yanked; exit 1 if any are yanked.
```

### Output format

```
$ loft registry check
registry: 14 packages, 28 versions  (synced 2 days ago)

installed packages (4):
  graphics  0.1.0  YANKED      CVE-2026-001 — run: loft install graphics
  opengl    0.1.0  deprecated  use-graphics — run: loft install opengl
  math      1.0.0  outdated    → 1.1.0      — run: loft install math
  utils     0.3.0  current

new packages in registry not installed: 10
  run 'loft registry list' to browse

1 security issue — yanked packages must be updated.
```

When all packages are current:

```
$ loft registry check
registry: 14 packages, 28 versions  (synced 2 days ago)

installed packages (4):
  graphics  0.2.0  current
  math      1.1.0  current
  utils     0.3.0  current
  geo       0.5.0  current

all installed packages are up to date.
```

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | No yanked packages installed (updates/deprecations may exist — informational only) |
| 1 | At least one installed package is yanked — action required |

Exit code 1 is intentionally reserved for security-level issues so that CI
pipelines can use `loft registry check` as a gate without triggering on every
available update.

### `loft registry list`

Lists all packages in the registry with their available versions and installed
status:

```
$ loft registry list
name       versions                    installed   status
---------  --------------------------  ----------  --------
geo        0.4.0  0.5.0               0.5.0
graphics   0.1.0  0.2.0               0.1.0       YANKED (0.1.0)
math       1.0.0  1.1.0               1.1.0
opengl     0.1.0                      0.1.0       deprecated
utils      0.3.0  0.4.0  0.5.0        0.3.0       outdated
web        0.1.0                      —
```

`loft registry list --installed` shows only rows where installed is not `—`.

---

## Version Resolution

### Latest version

When no version is specified, all entries whose `name` matches are collected
and compared using semver ordering.  The entry with the highest version is
selected.

Semver comparison: `(major, minor, patch)` tuples compared lexicographically.
This reuses the `version_ge` logic already in `src/manifest.rs`.

### Exact version match

When a version is given (`@0.1.0`), the registry is searched top-to-bottom
for the first entry with matching `(name, version)`.  If not found, the
install fails with:

```
loft install: package 'graphics@0.1.0' not found in registry.
  Available versions: 0.2.0
```

### Already installed

Before downloading, the installer checks `~/.loft/lib/<name>/loft.toml`.
If the installed version matches the selected registry entry, it prints:

```
loft install: graphics 0.2.0 is already installed.
```

and exits without downloading.  Use `--force` to reinstall anyway (future).

---

## Zip Package Layout

The downloaded `.zip` file must contain the package as a directory:

```
graphics-0.2.0/          ← top-level directory (name optional)
  loft.toml
  src/
    graphics.loft
    math.loft
  tests/
    canvas.loft
```

The installer finds the package root by searching for `loft.toml` inside the
extracted tree (depth-first, stopping at the first match).  This tolerates
both flat layout (`loft.toml` at zip root) and the conventional
`name-version/loft.toml` layout produced by GitHub release archives.

If no `loft.toml` is found but a `src/` directory is present at the zip root,
the zip root is treated as the package root (permissive fallback for pure-loft
packages that skip the manifest).

If neither condition is met, the install fails:

```
loft install: could not find package root in downloaded zip.
  Expected loft.toml or src/ directory inside the archive.
```

---

## Security Considerations

### HTTPS only

The installer enforces that URLs start with `https://`.  Plain `http://` URLs
are rejected with a warning unless overridden by a future `--allow-http` flag.

### No signature verification (Phase 3)

Phase 3 does not verify package signatures or checksums.  A future
`loft.toml` field `sha256 = "..."` could hold the expected hash of the
downloaded zip, verified before extraction.  Deferred until the registry
ecosystem is established enough that hash distribution is meaningful.

### Native code trust

Downloaded packages that include native shared libraries (Phase 2 feature)
are fully trusted once installed — `dlopen` gives the plugin full process
access, identical to any other native extension.  The registry is a
distribution mechanism, not a trust boundary.

### Registry file trust

The registry file is a plain text file from the local filesystem.  It does
not execute any code.  A compromised registry file can point to a malicious
zip, but the user controls which registry file is used.

---

## Implementation

### New: `src/registry.rs`

```rust
pub struct RegistryEntry {
    pub name:    String,
    pub version: String,
    pub url:     String,
    /// None = active; Some("yanked:CVE-2026-001") or Some("deprecated:reason")
    pub status:  Option<String>,
}

impl RegistryEntry {
    pub fn is_yanked(&self)     -> bool { self.status.as_deref().unwrap_or("").starts_with("yanked") }
    pub fn is_deprecated(&self) -> bool { self.status.as_deref().unwrap_or("").starts_with("deprecated") }
    pub fn is_active(&self)     -> bool { self.status.is_none() }
    pub fn status_slug(&self)   -> &str { /* part after ':' */ }
}

/// Parse a registry file.  Returns all entries including yanked/deprecated.
/// Also returns the source URL extracted from the "# source: <url>" header.
pub fn read_registry(path: &str) -> (Vec<RegistryEntry>, Option<String>);

/// Find the registry file path (LOFT_REGISTRY env var → ~/.loft/registry.txt).
pub fn registry_path() -> Option<std::path::PathBuf>;

/// Find the source URL: LOFT_REGISTRY_URL env var → source: header in file → compiled-in default.
pub fn source_url(file_source: Option<&str>) -> String;

/// Find the best matching entry for install.
/// version=None → highest semver active entry; version=Some → exact match (any status).
pub fn find_package<'a>(
    entries: &'a [RegistryEntry],
    name:    &str,
    version: Option<&str>,
) -> Option<&'a RegistryEntry>;

/// Scan ~/.loft/lib/ (or given dir) for installed packages.
/// Returns (name, version) for each directory containing a readable loft.toml.
pub fn installed_packages(lib_dir: &std::path::Path) -> Vec<(String, String)>;

pub enum PackageStatus<'a> {
    Yanked     { entry: &'a RegistryEntry },
    Deprecated { entry: &'a RegistryEntry, latest: Option<&'a RegistryEntry> },
    Outdated   { installed: &'a str, latest: &'a RegistryEntry },
    Current,
    Unknown,   // name not in registry
}

/// Compare an installed (name, version) pair against the registry.
pub fn classify<'a>(
    entries: &'a [RegistryEntry],
    name:    &str,
    version: &str,
) -> PackageStatus<'a>;

/// Download the zip at entry.url to a temp file, extract, return package root.
#[cfg(feature = "registry")]
pub fn download_and_extract(
    entry:    &RegistryEntry,
    tmp_base: &std::path::Path,
) -> Result<std::path::PathBuf, String>;

/// Download url into dst_path.  Returns Err with a human-readable message on failure.
#[cfg(feature = "registry")]
pub fn download_file(url: &str, dst: &std::path::Path) -> Result<(), String>;
```

### Cargo.toml additions

```toml
[features]
registry = ["dep:ureq", "dep:zip"]

[dependencies]
ureq = { version = "2", optional = true }
zip  = { version = "2", optional = true }
```

The `registry` feature is included in the `default` feature set so that
`cargo build` produces a `loft` binary with install-from-registry support.
It is excluded from the `wasm` feature set (no network access from WASM).

### `src/main.rs` changes

**`install` subcommand:**
1. After reading the argument, determine whether it is a local path or a
   registry reference (heuristic described above).
2. For registry references: parse the optional `@version` suffix, call
   `registry::registry_path()`, `registry::read_registry()`,
   `registry::find_package()`, then `registry::download_and_extract()`.
3. Pass the extracted package root to the existing `install_package()`.
4. Remove the temporary directory after install completes (or on error).

**`registry` subcommand:**
- `registry sync` — call `registry::source_url()`, `registry::download_file()`,
  validate content, write to `registry_path()`.
- `registry check` — call `registry::installed_packages()`, `registry::read_registry()`,
  `registry::classify()` for each; print report; exit 1 if any yanked.
- `registry list [--installed]` — read registry, scan installed, print table.

### `src/lib.rs` addition

```rust
pub mod registry;
```

### Error handling

All errors during download or extraction are printed to stderr and exit with
code 1 — same pattern as the rest of `main()`.

---

## Phased Rollout

### Phase 3a — Registry lookup and download (0.8.4, Sprint 9)

- `src/registry.rs` — parse registry file, find entry, download + extract zip
- `Cargo.toml` — add `ureq` and `zip` under `registry` feature
- `src/main.rs` — extend `install` subcommand to handle registry names
- `src/lib.rs` — expose `registry` module
- Tests: unit tests in `registry.rs`; integration test `tests/registry.rs`
- Docs: this file; update `EXTERNAL_LIBS.md` Phase 3 section

### Phase 3b — Registry sync and check (0.8.4, Sprint 9)

- `loft registry sync` — download latest registry from `source:` URL
- `loft registry check` — compare installed packages against registry; exit 1 on yanks
- `loft registry list [--installed]` — browse registry with installed status column
- `status` field parsing in `read_registry()` (yanked/deprecated)
- `installed_packages()` scanner, `classify()` function
- Staleness warning when registry is older than 7 days

### Phase 3c — Registry management (future)

- `loft registry search <term>` — filter registry entries by name prefix
- `loft registry add <name> <version> <url>` — append an entry to local file
- Deferred until Phase 3b is in use and the UX is understood.

### Phase 3c — SHA-256 verification (future)

- Optional `loft.toml` field: `zip_sha256 = "abc123..."`
- Or a parallel `.sha256` file next to the `.zip` in the registry
- Verified before extraction
- Deferred until registry ecosystem is established.

---

## Code Touchpoints

| File | Change | Phase |
|------|--------|-------|
| `src/registry.rs` | New: `read_registry`, `find_package`, `download_and_extract` | 3a |
| `src/lib.rs` | Expose `registry` module | 3a |
| `src/main.rs` | Extend `install` for registry names | 3a |
| `Cargo.toml` | Add `registry` feature, `ureq`, `zip` deps | 3a |
| `tests/registry.rs` | Integration tests for 3a | 3a |
| `src/registry.rs` | Add `status` field, `classify`, `installed_packages`, `download_file`, `source_url` | 3b |
| `src/main.rs` | Add `registry sync`, `registry check`, `registry list` subcommands | 3b |
| `tests/registry.rs` | Extend with sync/check/list tests | 3b |
| `doc/claude/REGISTRY.md` | This file | 3a+3b |
| `doc/claude/EXTERNAL_LIBS.md` | Update Phase 3 section | 3a |

---

## See also

- [REGISTRY_GOVERNANCE.md](REGISTRY_GOVERNANCE.md) — submission process, review checklist, yank/deprecation procedures
- [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) — full external library design including Phases 1 and 2
- [PACKAGES.md](PACKAGES.md) — unified package format (interpreter + native + WASM)
- [PLANNING.md](PLANNING.md) — priority backlog
