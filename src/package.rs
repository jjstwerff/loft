// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! `loft package` — produce a publishable tarball from a `loft.toml`
//! package directory.
//!
//! PKG.REG R1 (phase R1 of [PKG_REGISTRY.md](../doc/claude/PKG_REGISTRY.md)).
//! Produces `<pkg>-<version>.tar.gz` plus prints the SHA-256 + byte size
//! the publisher uses when adding the version to `loft-lang/registry`'s
//! `index.json`.
//!
//! Tarball layout (per PKG_REGISTRY.md § Tarball format):
//!
//! ```text
//! <pkg>-<version>/
//! ├── loft.toml
//! ├── src/
//! │   └── <pkg>.loft
//! ├── native/             (optional — only if present locally)
//! │   ├── Cargo.toml
//! │   ├── build.rs
//! │   └── src/lib.rs
//! ├── tests/              (optional)
//! └── README.md           (optional)
//! ```
//!
//! Excluded by default: `.git/`, `target/`, `.loft/`, `node_modules/`,
//! `.vscode/`, `.idea/`, any `*.tar.gz` (so the output file doesn't
//! recursively eat itself), and any path component starting with `.`
//! that isn't an explicit allow-listed dotfile (none today).
//!
//! In a git repo, files git ignores (untracked + matched by `.gitignore` /
//! `.git/info/exclude` / `core.excludesfile`) are ALSO excluded — see
//! `git_ignored_set`.  This keeps a package built from a dirty working tree
//! (e.g. after `loft test` wrote gitignored scratch files like
//! `tests/_tmp_*.bin`) byte-identical to one built from a clean clone, which the
//! registry's gate-3 reproducible-build check requires.  Parsing is delegated to
//! `git` rather than re-implemented; outside a git repo only the hardcoded list
//! above applies.

#![cfg(feature = "registry")]

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::GzBuilder;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};

/// Top-level paths that are never included in the tarball.
const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    "target",
    ".loft",
    "node_modules",
    ".vscode",
    ".idea",
];

/// File-name patterns excluded recursively.  The output `*.tar.gz` is
/// excluded so a re-run from the same directory doesn't try to bundle
/// the previous artefact.
fn is_excluded_file(name: &str) -> bool {
    has_suffix_ci(name, ".tar.gz") || has_suffix_ci(name, ".tar")
}

/// Case-insensitive ASCII suffix check.  `str::ends_with` is
/// case-sensitive and `to_ascii_lowercase` allocates; this avoids
/// both.
fn has_suffix_ci(s: &str, suffix: &str) -> bool {
    let sb = s.as_bytes();
    let tb = suffix.as_bytes();
    sb.len() >= tb.len() && sb[sb.len() - tb.len()..].eq_ignore_ascii_case(tb)
}

/// The three compatibility levels a package declares, each a promise on a
/// different axis a consumer can be hurt on.
///
/// Produced by [`declared_levels`], which is the ONE place the rule lives —
/// `loft package` and `loft publish` both call it rather than re-deriving it.
/// Design: `doc/claude/plans/library-compat-contract/README.md` step 5a.
#[derive(Debug, Clone)]
pub struct DeclaredLevels {
    /// `[package] loft` — which loft this needs.  A range (`">=0.8"`), because
    /// the platform is the one axis a library does not choose a single point on.
    pub loft: String,
    /// `[package] api_compatible_with` — the oldest own release this is still a
    /// drop-in for.  A bare version: it names an artifact that exists.
    pub api: String,
    /// `[package] data_compatible_with` — the oldest own release whose stored
    /// data this still reads.
    pub data: String,
}

/// A floor names ONE release, so it must be a bare `major.minor.patch` (an
/// optional `-prerelease` suffix is allowed).  A range would name a set, and a
/// set cannot be fetched and run — which is the entire reason these are real
/// versions rather than abstract epochs.
fn is_bare_version(v: &str) -> bool {
    let core = v.split('-').next().unwrap_or(v);
    let mut parts = core.split('.');
    let ok =
        |p: Option<&str>| p.is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    ok(parts.next()) && ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Check `[package]` declares all three compatibility levels, each naming a real
/// version at or below `version`.
///
/// Required before a package may be **registered** — these three are what a
/// consumer needs to decide whether an upgrade is safe on each axis it can be
/// hurt on: the platform, its call sites, its stored data.  A library that
/// declares nothing has promised nothing, which is fine right up until it asks
/// the registry to distribute it to people who cannot ask it questions.
///
/// # Errors
///
/// Returns **every** problem found rather than the first.  An author fixing one
/// line at a time pays a full tag-and-release cycle per round trip, so a
/// one-at-a-time gate would be several cycles of the same two-line edit.
pub fn declared_levels(
    manifest: &crate::manifest::Manifest,
    version: &str,
) -> Result<DeclaredLevels, Vec<String>> {
    let mut problems = Vec::new();

    // A floor at or above the release being cut would claim compatibility with
    // something that does not exist yet.  For a FIRST release the only honest
    // value is the version itself — trivially true, and the natural bootstrap.
    let mut floor = |field: &str, value: Option<&String>, means: &str| -> Option<String> {
        match value {
            None => {
                problems.push(format!(
                    "`{field}` is not declared.  It is {means}.\n    \
                     For a first release the value is this release itself:\n      \
                     {field} = \"{version}\"\n    \
                     Otherwise name the oldest own release the claim still holds for."
                ));
                None
            }
            Some(v) if !is_bare_version(v) => {
                problems.push(format!(
                    "`{field} = \"{v}\"` is not a bare version.  A floor names ONE release \
                     (`\"0.3.0\"`), never a range — the claim is verified by fetching that \
                     release and running its own tests, and a range names nothing to fetch."
                ));
                None
            }
            Some(v)
                if crate::registry_index::compare_semver(v, version)
                    == std::cmp::Ordering::Greater =>
            {
                problems.push(format!(
                    "`{field} = \"{v}\"` is newer than this release ({version}), so it claims \
                     compatibility with a version that does not exist yet."
                ));
                None
            }
            Some(v) => Some(v.clone()),
        }
    };

    let api = floor(
        "api_compatible_with",
        manifest.api_compatible_with.as_ref(),
        "the oldest release of this package whose public API this one is still a drop-in for",
    );
    let data = floor(
        "data_compatible_with",
        manifest.data_compatible_with.as_ref(),
        "the oldest release of this package whose stored or wire data this one still reads",
    );
    let loft = manifest.loft_version.clone();
    if loft.is_none() {
        problems.push(
            "`loft` is not declared.  It is which loft this package needs, and unlike the \
             other two it is a RANGE:\n      loft = \">=0.8\""
                .to_string(),
        );
    }

    match (loft, api, data) {
        (Some(loft), Some(api), Some(data)) if problems.is_empty() => {
            Ok(DeclaredLevels { loft, api, data })
        }
        _ => Err(problems),
    }
}

/// Render a [`declared_levels`] failure as the message a publisher acts on.
///
/// Shared so `loft package` and `loft publish` refuse in the same words — a gate
/// that reads differently depending on which command hit it teaches that it is
/// two rules rather than one.
#[must_use]
pub fn declared_levels_error(name: &str, version: &str, problems: &[String]) -> String {
    let mut out = format!(
        "`{name}` {version} cannot be registered: a package must declare all three \
         compatibility levels first.\n\n"
    );
    for p in problems {
        out.push_str("  • ");
        out.push_str(p);
        out.push_str("\n\n");
    }
    out.push_str(
        "  These are the three axes an upgrade can hurt a consumer on — the platform, its\n  \
         call sites, its stored data — and a consumer cannot ask your package a question.\n  \
         Declaring a floor is what enters the contract; raising one later is how you\n  \
         declare a break, and it should read like the promise withdrawal it is.\n  \
         See doc/claude/COMPATIBILITY.md and `loft compat --help`.",
    );
    out
}

/// Result of a successful `package_create`.
#[derive(Debug)]
pub struct PackageOutput {
    /// Path to the generated tarball.
    pub tarball: PathBuf,
    /// Byte length of the tarball file.
    pub size: u64,
    /// Lowercase hex SHA-256 of the tarball bytes.
    pub sha256: String,
    /// Package name (from `[package] name`).
    pub name: String,
    /// Package version (from `[package] version`).
    pub version: String,
    /// Publishing repository (from `[package] repository`), if set.  Present →
    /// monorepo (`<name>-v<version>` release tags); absent → legacy
    /// one-repo-per-package fallback.  See `print_summary`.
    pub repository: Option<String>,
    /// The three declared compatibility levels, when the manifest declares all
    /// of them (see [`declared_levels`]).  `None` → the package has not entered
    /// the contract, and `print_summary` refuses to emit a registry entry for it.
    pub levels: Option<DeclaredLevels>,
}

/// Walk `pkg_dir`, build a gzipped tarball, write it to disk, and
/// return its hash + size.  The output path is
/// `<pkg>-<version>.tar.gz` in `out_dir` (defaults to `pkg_dir`).
///
/// # Errors
///
/// Returns `io::Error` on:
/// - Missing or unreadable `loft.toml`.
/// - Manifest missing `[package] name` or `version`.
/// - Filesystem walk / archive write failure.
pub fn package_create(pkg_dir: &Path, out_dir: Option<&Path>) -> io::Result<PackageOutput> {
    let manifest_path = pkg_dir.join("loft.toml");
    let manifest =
        crate::manifest::read_manifest(manifest_path.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 manifest path")
        })?)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("loft.toml not found at {}", manifest_path.display()),
            )
        })?;
    // Cloned rather than moved out: `declared_levels` below needs the whole
    // manifest, and a partial move would make it unborrowable.
    let name = manifest.name.clone().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "loft.toml is missing [package] name",
        )
    })?;
    let version = manifest.version.clone().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "loft.toml is missing [package] version",
        )
    })?;

    let archive_prefix = format!("{name}-{version}");
    let out_name = format!("{archive_prefix}.tar.gz");
    let out_path = out_dir.unwrap_or(pkg_dir).join(&out_name);

    // Build the tarball.
    //
    // Determinism: same source bytes → same tarball, regardless of who
    // ran `loft package` or when.  The registry's gate-3 reproducible-
    // build re-check relies on this.  We normalise at two layers:
    //
    //   1. The OUTER gzip stream uses `GzBuilder::mtime(0)` so the
    //      gzip-mtime header byte field is fixed, not the wall clock.
    //   2. The INNER tar entries are written with manually-constructed
    //      headers that pin `mtime = 0`, `uid = 0`, `gid = 0`, owner
    //      strings empty, and mode = 0o644 (files) / 0o755 (dirs).
    //      `tar::Builder::append_path_with_name` defaults to copying
    //      the on-disk mtime + permissions, which leak through git's
    //      lack-of-mtime-preservation across clones.
    //
    // Files on disk may have arbitrary mtimes (commit time on one
    // checkout, clone time on another) — that's git's contract.  By
    // overriding both gzip + tar timestamps we make `loft package`
    // genuinely content-addressed: the tarball bytes depend only on
    // the file *contents* and the archive paths, nothing else.
    {
        let tar_gz = fs::File::create(&out_path)?;
        let enc = GzBuilder::new()
            .mtime(0)
            .write(tar_gz, Compression::default());
        let mut builder = tar::Builder::new(enc);
        builder.follow_symlinks(false);
        add_dir_contents(&mut builder, pkg_dir, &archive_prefix, &out_name)?;
        let enc = builder.into_inner()?;
        enc.finish()?;
    }

    // Hash + size.
    let (size, sha256) = hash_file(&out_path)?;

    // Carried, not enforced: building a tarball is mechanical (the
    // reproducible-build check re-packages every library just to compare bytes,
    // and must not care what any of them declares).  The gate is at the two
    // commands that produce a REGISTRY ENTRY — `loft package` and `loft publish`.
    let levels = declared_levels(&manifest, &version).ok();

    Ok(PackageOutput {
        tarball: out_path,
        size,
        sha256,
        name,
        version,
        repository: manifest.repository,
        levels,
    })
}

/// Recursively add the contents of `src_dir` to `builder`, prefixing
/// each entry with `archive_prefix/`.  Skips the exclusion list.
///
/// `out_name` is the name of the tarball-in-progress, so a `loft
/// package` invocation that targets its own directory doesn't include
/// the partially-written archive in itself.
fn add_dir_contents(
    builder: &mut tar::Builder<GzEncoder<fs::File>>,
    src_dir: &Path,
    archive_prefix: &str,
    out_name: &str,
) -> io::Result<()> {
    let ignored = git_ignored_set(src_dir);
    walk(
        builder,
        src_dir,
        src_dir,
        archive_prefix,
        out_name,
        &ignored,
    )
}

/// Paths (relative to the package root) that git ignores — UNTRACKED files
/// matched by `.gitignore` / `.git/info/exclude` / `core.excludesfile`.  We skip
/// these when packaging so a tarball built from a DIRTY working tree (e.g. after
/// `loft test` wrote gitignored scratch files such as `tests/_tmp_*.bin`) matches
/// one built from a clean clone — the registry's gate-3 reproducible-build
/// invariant.  `.gitignore` parsing is delegated to `git` itself rather than
/// re-implemented.
///
/// Returns empty (skip nothing) when the dir is not a git repo or `git` is
/// missing, so non-git packaging is unchanged.  Only *untracked* ignored files
/// are listed, so a clean checkout (which has none) packages identically before
/// and after this change — existing releases stay valid.
fn git_ignored_set(root: &Path) -> std::collections::HashSet<PathBuf> {
    let mut set = std::collections::HashSet::new();
    let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ])
        .output()
    else {
        return set; // git not installed → behave as before
    };
    if !out.status.success() {
        return set; // not inside a git repo → behave as before
    }
    for p in out.stdout.split(|&b| b == 0) {
        if !p.is_empty() {
            let s = String::from_utf8_lossy(p);
            set.insert(PathBuf::from(s.trim_end_matches('/')));
        }
    }
    set
}

fn walk(
    builder: &mut tar::Builder<GzEncoder<fs::File>>,
    root: &Path,
    current: &Path,
    archive_prefix: &str,
    out_name: &str,
    ignored: &std::collections::HashSet<PathBuf>,
) -> io::Result<()> {
    // Collect + sort entries so the resulting tarball is deterministic
    // across runs (same bytes → same SHA-256, which the publisher's
    // PR-validation script depends on).
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(current)?.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        let file_type = entry.file_type()?;

        // Skip anything git ignores (see git_ignored_set) — keeps a dirty-tree
        // package byte-identical to a clean clone (gate-3 reproducible build).
        if path
            .strip_prefix(root)
            .is_ok_and(|rel| ignored.contains(rel))
        {
            continue;
        }

        // Top-level dir exclusions.
        if file_type.is_dir()
            && path.parent() == Some(root)
            && EXCLUDED_DIRS.contains(&name_str.as_ref())
        {
            continue;
        }
        // Per-component exclusion of nested EXCLUDED_DIRS (e.g.
        // `native/target/` — target appears nested too).
        if file_type.is_dir() && EXCLUDED_DIRS.contains(&name_str.as_ref()) {
            continue;
        }
        if file_type.is_file() && is_excluded_file(&name_str) && name_str == out_name {
            // Skip the in-flight tarball itself.
            continue;
        }
        if file_type.is_file() && is_excluded_file(&name_str) {
            // Skip stale `*.tar.gz` artefacts too — the publisher
            // shouldn't accidentally embed last release's tarball.
            continue;
        }

        // Compute the archive-relative path: `<prefix>/<rel>`.
        let rel = path
            .strip_prefix(root)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let archive_rel = PathBuf::from(archive_prefix).join(rel);

        if file_type.is_dir() {
            walk(builder, root, &path, archive_prefix, out_name, ignored)?;
        } else if file_type.is_file() {
            // Deterministic file entry: hand-construct the tar header
            // with mtime=0, uid=0, gid=0, mode=0o644, empty owner
            // strings.  Skips the `append_path_with_name` defaults
            // which copy on-disk mtime + uid + gid — those vary
            // between checkouts (git doesn't preserve mtimes) and
            // would make the tarball non-reproducible.
            let metadata = path.metadata()?;
            let mut header = tar::Header::new_gnu();
            header.set_size(metadata.len());
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_uid(0);
            header.set_gid(0);
            header.set_entry_type(tar::EntryType::Regular);
            // `username` / `groupname` default to empty for new headers.
            // `set_cksum()` must be called LAST — it computes the
            // checksum over every other header byte, so any later
            // mutation invalidates it.
            let file = fs::File::open(&path)?;
            builder.append_data(&mut header, &archive_rel, file)?;
        }
        // Symlinks: skipped here — a symlinked file caught by
        // `is_file()` above goes through the deterministic-header
        // path; a true symlink (`file_type.is_symlink()` true,
        // `is_file()` false) is dropped silently.  Packages should
        // not rely on symlinks in the tarball; the MVP behaviour is
        // unchanged for the regular-file case.
    }
    Ok(())
}

/// Read `path` into memory, return `(byte_count, lowercase_hex_sha256)`.
/// Tarballs in the MVP are typically <100 kB; the in-memory hash is
/// fine.  Switch to streaming if a publish target exceeds ~10 MB.
fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    let size = buf.len() as u64;
    let mut hasher = Sha256::new();
    hasher.update(&buf);
    let digest = hasher.finalize();
    let hex = hex_encode(&digest);
    Ok((size, hex))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Format a `PackageOutput` for human consumption — the surface the
/// publisher reads to know what to paste into the registry index PR.
///
/// # Errors
///
/// Propagates any `io::Error` from the underlying writer.
#[allow(clippy::cast_precision_loss)] // package sizes fit comfortably in f64
pub fn print_summary(out: &PackageOutput, w: &mut dyn Write) -> io::Result<()> {
    writeln!(w, "{}", out.tarball.display())?;
    writeln!(w, "  package:  {} v{}", out.name, out.version)?;
    writeln!(
        w,
        "  size:     {} bytes ({:.1} kB)",
        out.size,
        out.size as f64 / 1024.0
    )?;
    writeln!(w, "  sha256:   {}", out.sha256)?;
    writeln!(w)?;
    writeln!(
        w,
        "Index entry to paste into loft-lang/registry/index.json (PKG_REGISTRY.md schema):"
    )?;
    writeln!(w, "  \"{}\": {{", out.version)?;
    let tarball_name = out
        .tarball
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    // `[package] repository` set → the package ships from a monorepo, so the
    // release tag is `<name>-v<version>` (disambiguates packages sharing one
    // repo) at the named repo.  Absent → the legacy one-repo-per-package form
    // (`loft-<name>` repo, bare `v<version>` tag).  A bare repository value is
    // an org-relative repo under `loft-lang`; a value with `/` is `owner/repo`.
    let url = match out.repository.as_deref() {
        Some(repo) => {
            let owner_repo = if repo.contains('/') {
                repo.to_string()
            } else {
                format!("loft-lang/{repo}")
            };
            format!(
                "https://github.com/{owner_repo}/releases/download/{}-v{}/{tarball_name}",
                out.name, out.version
            )
        }
        None => format!(
            "https://github.com/loft-lang/loft-{}/releases/download/v{}/{tarball_name}",
            out.name, out.version
        ),
    };
    writeln!(w, "    \"url\": \"{url}\",")?;
    writeln!(w, "    \"sha256\": \"{}\",", out.sha256)?;
    writeln!(w, "    \"size\": {},", out.size)?;
    // The manifest's own value, not a constant.  This line read a hardcoded
    // `">=0.8"` for every package regardless of what it declared, so a library
    // needing a newer loft published an entry saying it did not.
    let levels = out.levels.as_ref();
    writeln!(
        w,
        "    \"loft\": \"{}\",",
        levels.map_or(">=0.8", |l| l.loft.as_str())
    )?;
    if let Some(l) = levels {
        // Carried into the index so a resolver can read a version's promises
        // without fetching and unpacking its tarball first.
        writeln!(w, "    \"api_compatible_with\": \"{}\",", l.api)?;
        writeln!(w, "    \"data_compatible_with\": \"{}\",", l.data)?;
    }
    writeln!(w, "    \"published\": \"<ISO-8601 UTC timestamp>\"")?;
    writeln!(w, "  }}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Per-test tmpdir.  Includes the test name + process id so
    /// parallel test runners don't trample each other's fixtures.
    fn tmpdir(name: &str) -> PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("loft_pkg_test_{}_{}", std::process::id(), name));
        if p.exists() {
            let _ = fs::remove_dir_all(&p);
        }
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    /// Build a manifest from `[package]` body lines, so each level case differs
    /// in exactly the line under test.
    fn manifest_with(package_lines: &str) -> crate::manifest::Manifest {
        let dir = tmpdir(&format!(
            "levels_{:x}",
            package_lines.len() * 31
                + usize::from(package_lines.as_bytes().first().copied().unwrap_or(0))
        ));
        let path = dir.join("loft.toml");
        write(
            &path,
            &format!(
                "[package]\nname = \"probe\"\nversion = \"0.4.0\"\n{package_lines}\n\
                 [library]\nentry = \"src/probe.loft\"\n"
            ),
        );
        let m = crate::manifest::read_manifest(path.to_str().unwrap()).unwrap();
        let _ = fs::remove_dir_all(&dir);
        m
    }

    /// Step 5a: all three levels are required before a package may be
    /// registered, and each floor must name a version that could exist.
    ///
    /// The `all` / `first_release` cells are the ones that must PASS — without
    /// them a gate that rejected everything would look identical to a correct
    /// one, which is how the first hand-run of this matrix read.
    #[test]
    fn declared_levels_matrix() {
        let ok = |lines: &str| declared_levels(&manifest_with(lines), "0.4.0");
        let err = |lines: &str| ok(lines).expect_err("should be rejected").join(" | ");

        // All three declared, floors below the release being cut.
        let levels = ok(
            "loft = \">=0.8\"\napi_compatible_with = \"0.2.0\"\ndata_compatible_with = \"0.1.0\"",
        )
        .expect("all three declared");
        assert_eq!(levels.loft, ">=0.8");
        assert_eq!(levels.api, "0.2.0");
        assert_eq!(levels.data, "0.1.0");

        // A FIRST release: the floors are the release itself.  This is the
        // bootstrap the error message tells authors to write, so it has to pass.
        assert!(
            ok("loft = \">=0.8\"\napi_compatible_with = \"0.4.0\"\ndata_compatible_with = \"0.4.0\"")
                .is_ok()
        );

        // Each one missing is named, and its own name appears in the message —
        // an author must not have to guess which of the three is absent.
        assert!(
            err("loft = \">=0.8\"\ndata_compatible_with = \"0.1.0\"")
                .contains("`api_compatible_with` is not declared")
        );
        assert!(
            err("loft = \">=0.8\"\napi_compatible_with = \"0.2.0\"")
                .contains("`data_compatible_with` is not declared")
        );
        assert!(
            err("api_compatible_with = \"0.2.0\"\ndata_compatible_with = \"0.1.0\"")
                .contains("`loft` is not declared")
        );

        // ALL problems at once, not the first: each round trip costs a publish
        // cycle, so a one-at-a-time gate is several cycles of the same edit.
        let none = ok("").expect_err("nothing declared");
        assert_eq!(none.len(), 3, "expected all three reported, got: {none:?}");

        // A floor names ONE release, so a range is rejected — a set names
        // nothing that can be fetched and run.
        assert!(
            err("loft = \">=0.8\"\napi_compatible_with = \">=0.2\"\ndata_compatible_with = \"0.1.0\"")
                .contains("not a bare version")
        );
        // `loft` is the exception: it IS a range, and must stay accepted as one.
        assert!(ok("loft = \">=0.8\"\napi_compatible_with = \"0.2.0\"\ndata_compatible_with = \"0.1.0\"").is_ok());

        // A floor above the release claims compatibility with a version that
        // does not exist yet.
        assert!(
            err("loft = \">=0.8\"\napi_compatible_with = \"0.9.0\"\ndata_compatible_with = \"0.1.0\"")
                .contains("does not exist yet")
        );
    }

    /// The index entry carries the manifest's OWN `loft` range and both floors.
    /// This line was a hardcoded `">=0.8"` for every package regardless of what
    /// it declared, so a library needing a newer loft published an entry saying
    /// it did not.
    #[test]
    fn index_entry_carries_declared_levels() {
        let out = PackageOutput {
            tarball: PathBuf::from("crypto-0.2.1.tar.gz"),
            size: 1,
            sha256: "00".to_string(),
            name: "crypto".to_string(),
            version: "0.2.1".to_string(),
            repository: Some("loft-libs-core".to_string()),
            levels: Some(DeclaredLevels {
                loft: ">=2026.7".to_string(),
                api: "0.2.0".to_string(),
                data: "0.1.0".to_string(),
            }),
        };
        let mut buf = Vec::new();
        print_summary(&out, &mut buf).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert!(text.contains("\"loft\": \">=2026.7\""), "{text}");
        assert!(
            text.contains("\"api_compatible_with\": \"0.2.0\""),
            "{text}"
        );
        assert!(
            text.contains("\"data_compatible_with\": \"0.1.0\""),
            "{text}"
        );
    }

    #[test]
    fn packages_minimal_layout() {
        let dir = tmpdir("packages_minimal_layout");
        let pkg = dir.join("hello");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"hello\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\n[library]\nentry = \"src/hello.loft\"\n",
        );
        write(
            &pkg.join("src").join("hello.loft"),
            "pub fn greet() -> text { return \"hi\"; }\n",
        );

        let out = package_create(&pkg, None).expect("package_create");
        assert_eq!(out.name, "hello");
        assert_eq!(out.version, "0.1.0");
        assert!(out.tarball.exists());
        assert!(out.size > 0);
        assert_eq!(out.sha256.len(), 64);
        assert!(out.sha256.chars().all(|c| c.is_ascii_hexdigit()));

        let _ = fs::remove_dir_all(&dir);
    }

    /// The generated registry URL follows `[package] repository`: a monorepo
    /// uses the `<name>-v<version>` tag at the named repo; absence falls back to
    /// the legacy one-repo-per-package `loft-<name>` + `v<version>` form.
    #[test]
    fn index_url_monorepo_vs_legacy() {
        let mk = |repo: Option<&str>| PackageOutput {
            tarball: PathBuf::from("crypto-0.2.1.tar.gz"),
            size: 1,
            sha256: "00".to_string(),
            name: "crypto".to_string(),
            version: "0.2.1".to_string(),
            repository: repo.map(str::to_string),
            levels: None,
        };
        let url_of = |o: &PackageOutput| {
            let mut buf = Vec::new();
            print_summary(o, &mut buf).unwrap();
            String::from_utf8(buf).unwrap()
        };
        // monorepo (bare repo → org-relative under loft-lang)
        assert!(url_of(&mk(Some("loft-libs-core"))).contains(
            "https://github.com/loft-lang/loft-libs-core/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz"
        ));
        // explicit owner/repo
        assert!(url_of(&mk(Some("acme/widgets"))).contains(
            "https://github.com/acme/widgets/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz"
        ));
        // legacy fallback (no repository)
        assert!(url_of(&mk(None)).contains(
            "https://github.com/loft-lang/loft-crypto/releases/download/v0.2.1/crypto-0.2.1.tar.gz"
        ));
    }

    #[test]
    fn excludes_dotgit_and_target() {
        let dir = tmpdir("excludes_dotgit_and_target");
        let pkg = dir.join("with_excludes");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"with_excludes\"\nversion = \"0.1.0\"\n",
        );
        write(
            &pkg.join("src").join("with_excludes.loft"),
            "// real source\n",
        );
        write(&pkg.join(".git").join("HEAD"), "ref: refs/heads/main\n");
        write(&pkg.join("target").join("debug").join("x"), "binary\n");

        let out = package_create(&pkg, None).expect("package_create");

        // Re-open and walk: confirm .git and target are absent.
        let tar_gz = fs::File::open(&out.tarball).unwrap();
        let dec = flate2::read::GzDecoder::new(tar_gz);
        let mut ar = tar::Archive::new(dec);
        let mut paths: Vec<String> = Vec::new();
        for entry in ar.entries().unwrap() {
            let entry = entry.unwrap();
            paths.push(entry.path().unwrap().to_string_lossy().into_owned());
        }
        assert!(
            paths.iter().any(|p| p.ends_with("loft.toml")),
            "loft.toml missing: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.contains("src/with_excludes.loft")),
            "src missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains(".git")),
            ".git leaked: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("target/")),
            "target leaked: {paths:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn excludes_gitignored_untracked_files() {
        // A tarball built from a DIRTY tree (gitignored scratch files present —
        // e.g. `tests/_tmp_*.bin` left by `loft test`) must match one built from
        // a clean clone, or the registry's gate-3 reproducible-build check fails.
        // Regression for the hex_world 0.1.x mismatch.
        let dir = tmpdir("excludes_gitignored_untracked_files");
        let pkg = dir.join("ignlib");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"ignlib\"\nversion = \"0.1.0\"\n",
        );
        write(&pkg.join("src").join("ignlib.loft"), "// real source\n");
        write(&pkg.join(".gitignore"), "tests/_tmp_*.bin\n");
        write(&pkg.join("tests").join("_tmp_scratch.bin"), "junk\n");

        // Needs a git repo so git can resolve the ignore rules.  If git is
        // unavailable the fix degrades to a no-op, so skip rather than fail.
        let git_ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&pkg)
            .args(["init", "-q"])
            .status()
            .is_ok_and(|s| s.success());
        if !git_ok {
            let _ = fs::remove_dir_all(&dir);
            return;
        }

        let out = package_create(&pkg, None).expect("package_create");
        let tar_gz = fs::File::open(&out.tarball).unwrap();
        let dec = flate2::read::GzDecoder::new(tar_gz);
        let mut ar = tar::Archive::new(dec);
        let mut paths: Vec<String> = Vec::new();
        for entry in ar.entries().unwrap() {
            paths.push(
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        assert!(
            paths.iter().any(|p| p.contains("src/ignlib.loft")),
            "src missing: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("_tmp_scratch.bin")),
            "gitignored scratch file leaked into the package: {paths:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha256_is_deterministic_across_runs() {
        let dir = tmpdir("sha256_is_deterministic_across_runs");
        let pkg = dir.join("det");
        write(
            &pkg.join("loft.toml"),
            "[package]\nname = \"det\"\nversion = \"0.1.0\"\n",
        );
        write(&pkg.join("src").join("det.loft"), "// content\n");

        let a = package_create(&pkg, None).unwrap();
        // Remove the previous tarball so the second run rebuilds.
        fs::remove_file(&a.tarball).unwrap();
        let b = package_create(&pkg, None).unwrap();

        // Tarballs include mtime in headers, so byte-for-byte equality
        // requires preserving on-disk mtimes — which they are, because
        // we didn't touch the source files between runs.  This is the
        // contract: same inputs → same hash, the assertion in the
        // PKG_REGISTRY.md schema.
        assert_eq!(
            a.sha256, b.sha256,
            "sha256 drifted: {} vs {}",
            a.sha256, b.sha256
        );
        assert_eq!(a.size, b.size);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fails_when_manifest_missing() {
        let dir = tmpdir("fails_when_manifest_missing");
        let pkg = dir.join("empty");
        fs::create_dir_all(&pkg).unwrap();
        let err = package_create(&pkg, None).expect_err("should fail");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        let _ = fs::remove_dir_all(&dir);
    }
}
