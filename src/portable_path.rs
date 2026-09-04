// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! One way to render a path so it reads the same on every platform.
//!
//! Three places had grown their own `replace('\\', "/")` — the sandbox's policy
//! matcher, the LSP's `path_to_uri`, and the test-coverage report — and a fourth
//! nearly did.  Each is the same idea, and each got it subtly wrong in the same
//! way: **replacing every backslash is not the same as replacing the separator.**
//! On Unix a backslash is an ordinary filename character, so
//! `replace('\\', "/")` corrupts `weird\name.loft` into a two-segment path that
//! matches the wrong policy, resolves to the wrong URI, or reports a file that
//! does not exist.
//!
//! Replacing [`std::path::MAIN_SEPARATOR`] instead is a no-op on Unix (it swaps
//! `/` for `/`) and does the intended conversion on Windows.  So the corruption
//! cannot happen, and the platform that needs the conversion still gets it.
//!
//! **Use this for paths that are DISPLAYED, matched, or compared as text** — a
//! report a reader copies into an editor, a URI, a policy selector.  Never for a
//! path you are about to open: `std::path` already handles that correctly, and
//! turning it into a string first only loses information.

use std::path::{Path, PathBuf};

/// Render `path` with `/` separators on every platform.
///
/// See the module docs for why this replaces the separator rather than every
/// backslash.
#[must_use]
pub fn portable(path: &Path) -> String {
    portable_str(&path.to_string_lossy())
}

/// The `&str` form, for callers that already hold a path as text — the parser
/// hands file positions around as `String`, so most sandbox/diagnostic paths
/// arrive this way and would otherwise pay a pointless `Path` round-trip.
#[must_use]
pub fn portable_str(path: &str) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        // Unix: already portable, and rewriting would corrupt a legitimate
        // backslash in a filename.
        path.to_string()
    } else {
        path.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

/// Render `path` for a target format in which a backslash is **illegal**, not merely
/// unidiomatic — a `file://` URI, where `\U` is an invalid JSON escape that silently
/// corrupts the LSP message the URI is embedded in (the #640 Windows hang).
///
/// Deliberately NOT [`portable`], and the difference is the whole reason both exist:
/// `portable` renders *this* platform's separator, so on Unix it correctly leaves a
/// backslash alone as the ordinary filename character it is.  Here the input may have
/// come from anywhere — an editor on another platform — and the output must satisfy the
/// FORMAT regardless of who produced it.  A backslash that survives is a corrupt
/// message, which beats preserving an exotic filename.
#[must_use]
pub fn for_uri(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `fs::canonicalize`, rendered in the ONE spelling every other path in the process uses.
///
/// On Windows `canonicalize` answers an extended-length verbatim path (`\\?\D:\…`), while
/// the rest of the pipeline — library `use` resolution, the entry-package skip, a source
/// position's `file`, the logger's project root, the LSP's URIs — builds and compares plain
/// paths.  A verbatim path never equals or prefix-matches its plain twin (`VerbatimDisk`
/// vs `Disk` components), so every canonicalised path that enters the shared path space
/// sheds the prefix here.  A path that cannot be canonicalised is answered as given rather
/// than dropped: an absolute-but-unresolved path is still better than none.  No-op on
/// Linux/macOS.
///
/// This is the only `canonicalize` a caller should reach for; [`try_plain_canonical`] is
/// the fallible twin for a site that must know the path EXISTS.
#[must_use]
pub fn plain_canonical(path: &Path) -> PathBuf {
    try_plain_canonical(path).unwrap_or_else(|| path.to_path_buf())
}

/// [`plain_canonical`] for a site that needs "this path resolves" as a fact — `None` when
/// it does not exist, never a guess.
#[must_use]
pub fn try_plain_canonical(path: &Path) -> Option<PathBuf> {
    let abs = std::fs::canonicalize(path).ok()?;
    Some(match abs.to_str() {
        Some(text) => PathBuf::from(strip_verbatim(text)),
        None => abs,
    })
}

/// [`plain_canonical`] for the many sites that carry a path as text — the parser hands
/// source positions around as `String`, and a display or a comparison key wants one back.
#[must_use]
pub fn plain_canonical_str(path: &str) -> String {
    plain_canonical(Path::new(path))
        .to_string_lossy()
        .into_owned()
}

/// Shed a Windows verbatim prefix: `\\?\D:\…` becomes `D:\…` and `\\?\UNC\srv\share\…`
/// becomes `\\srv\share\…`, the two spellings `canonicalize` produces there and the two
/// plain forms everything else builds.  A path without a prefix is answered unchanged, so
/// this is safe to apply to any spelling.
#[must_use]
pub fn strip_verbatim(path: &str) -> String {
    if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        rest.to_string()
    } else {
        path.to_string()
    }
}

/// Does `file` name a source inside the shipped standard library?
///
/// A stdlib position's `file` is whatever path the loader used: `default/01_code.loft`
/// relative to the repo in a test, an absolute `<install>/share/loft/default/…` from an
/// installed binary, either separator on Windows.  Every reader that treats the stdlib
/// differently — coverage, introspection, the REPL's listing, code generation's logging
/// switch, the entry-point census — asks this ONE question, so the answer cannot vary by
/// how loft was launched.  A user directory literally named `default` is misread as the
/// stdlib; that is the price of a path-shaped answer, and the loader's own record of where
/// it read the stdlib from is the sharper fact if it ever matters.
#[must_use]
pub fn is_stdlib_source(file: &str) -> bool {
    file.starts_with("default/")
        || file.starts_with("default\\")
        || file.contains("/default/")
        || file.contains("\\default\\")
}

/// Is `file` inside `dir`, by path COMPONENTS — so `pkg` does not claim `pkg2/x.loft`, and
/// on Windows a `/` and a `\` spelling of one directory agree.  Both are taken as written;
/// pair with [`plain_canonical`] when either side may be a relative or symlinked spelling.
#[must_use]
pub fn is_under(file: &str, dir: &str) -> bool {
    Path::new(file).starts_with(Path::new(dir))
}

/// [`is_under`] after resolving BOTH sides: does the file `path` on disk live inside the
/// directory `dir` on disk?  `false` when either does not exist — an unresolvable path is
/// inside nothing.
#[must_use]
pub fn is_under_canonical(path: &Path, dir: &Path) -> bool {
    match (try_plain_canonical(path), try_plain_canonical(dir)) {
        (Some(p), Some(d)) => p.starts_with(&d),
        _ => false,
    }
}

/// Do two spellings name the same file on disk?  `false` when either does not exist.
#[must_use]
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (try_plain_canonical(a), try_plain_canonical(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The conversion the Windows leg needs, expressed so it is checkable on both
    /// platforms: a path built from components renders with `/` wherever it runs.
    #[test]
    fn a_relative_path_reads_with_forward_slashes() {
        let p: std::path::PathBuf = ["src", "pos.loft"].iter().collect();
        assert_eq!(portable(&p), "src/pos.loft");
    }

    #[test]
    fn nested_segments_all_convert() {
        let p: std::path::PathBuf = ["a", "b", "c.loft"].iter().collect();
        assert_eq!(portable(&p), "a/b/c.loft");
    }

    /// The bug the hand-rolled form carried.  On Unix a backslash is an ordinary
    /// filename character, so `replace('\\', "/")` invents a separator that was
    /// never there — turning one file into a two-segment path.
    #[cfg(unix)]
    #[test]
    fn a_backslash_in_a_unix_filename_is_not_a_separator() {
        assert_eq!(portable_str("dir/weird\\name.loft"), "dir/weird\\name.loft");
        assert_ne!(portable_str("dir/weird\\name.loft"), "dir/weird/name.loft");
    }

    /// An absolute Windows path keeps its drive prefix — which is why this
    /// replaces the separator in the rendered string rather than joining
    /// `Path::components()`, whose `RootDir` renders as a separator of its own and
    /// would produce `C:/\/a`.
    #[cfg(windows)]
    #[test]
    fn an_absolute_windows_path_keeps_its_prefix() {
        assert_eq!(
            portable(std::path::Path::new(r"C:\a\b.loft")),
            "C:/a/b.loft"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_unix_path_is_unchanged() {
        assert_eq!(portable(std::path::Path::new("/a/b.loft")), "/a/b.loft");
    }

    /// The two are NOT interchangeable, and collapsing them is a real mistake I made:
    /// routing `path_to_uri` through `portable` passed on Windows and broke on Linux,
    /// because there the separator is `/` and the backslash was correctly left alone —
    /// producing a URI carrying `\U`, an invalid JSON escape.  This pins the difference
    /// so the next unification attempt fails here instead of in the LSP transport.
    #[test]
    fn uri_rendering_is_not_platform_rendering() {
        let p = std::path::Path::new(r"C:\a\b.loft");
        assert_eq!(
            for_uri(p),
            "C:/a/b.loft",
            "a URI may never carry a backslash"
        );
        #[cfg(unix)]
        assert_eq!(
            portable(p),
            r"C:\a\b.loft",
            "on Unix those backslashes are filename characters, not separators"
        );
    }

    /// The chokepoint has to STAY one.  Five sites had grown their own
    /// `replace('\\', "/")` — sandbox policy matching (x2), `path_to_uri`, the
    /// coverage report, and a directory listing returned to loft programs — and each
    /// carried the same latent Unix corruption.  Collapsing them is only worth
    /// anything if a sixth cannot quietly appear, so this fails the moment one does.
    #[test]
    fn nobody_hand_rolls_the_conversion_again() {
        let mut offenders = Vec::new();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs")
                    && p.file_name().is_some_and(|f| f != "portable_path.rs")
                    && let Ok(text) = std::fs::read_to_string(&p)
                {
                    for (n, line) in text.lines().enumerate() {
                        // Two shapes, and the second is the one that got away.  The literal
                        // `replace('\\', "/")` is the WRONG conversion; `replace(MAIN_SEPARATOR,
                        // …)` is the RIGHT one written a second time.  Catching only the wrong
                        // shape let a correct copy land in `logger.rs`, which is how a fifth
                        // hand-roll appeared in the module whose whole purpose is to prevent
                        // a fourth.  A duplicate that is correct today still has to be found
                        // and re-fixed when the rule changes.
                        if line.contains(r#"replace('\\', "/")"#)
                            || line.contains("replace(std::path::MAIN_SEPARATOR")
                            || line.contains("replace(MAIN_SEPARATOR")
                        {
                            offenders.push(format!("{}:{}", p.display(), n + 1));
                        }
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "use portable_path::portable / portable_str instead — replacing every \
             backslash corrupts a Unix filename that contains one:\n  {}",
            offenders.join("\n  ")
        );
    }

    #[test]
    fn a_verbatim_prefix_is_shed_and_nothing_else_is_touched() {
        assert_eq!(strip_verbatim(r"\\?\C:\work\a.loft"), r"C:\work\a.loft");
        assert_eq!(
            strip_verbatim(r"\\?\UNC\srv\share\a.loft"),
            r"\\srv\share\a.loft"
        );
        assert_eq!(strip_verbatim(r"C:\work\a.loft"), r"C:\work\a.loft");
        assert_eq!(strip_verbatim("/home/u/a.loft"), "/home/u/a.loft");
    }

    #[test]
    fn the_stdlib_is_recognised_however_it_was_loaded() {
        assert!(is_stdlib_source("default/01_code.loft"));
        assert!(is_stdlib_source(r"default\01_code.loft"));
        assert!(is_stdlib_source(
            "/usr/local/share/loft/default/01_code.loft"
        ));
        assert!(is_stdlib_source(r"C:\loft\default\01_code.loft"));
        assert!(!is_stdlib_source("src/main.loft"));
        assert!(!is_stdlib_source("defaults/x.loft"));
        assert!(!is_stdlib_source(""));
    }

    #[test]
    fn is_under_compares_components_not_characters() {
        assert!(is_under("pkg/src/a.loft", "pkg"));
        assert!(is_under("pkg/src/a.loft", "pkg/src"));
        assert!(!is_under("pkg2/src/a.loft", "pkg"));
        assert!(!is_under("pkg", "pkg/src"));
    }

    #[test]
    fn canonical_comparisons_answer_false_for_what_does_not_exist() {
        let dir = std::env::temp_dir();
        let missing = dir.join("no-such-file-7d2b19.loft");
        assert!(!same_file(&missing, &missing));
        assert!(!is_under_canonical(&missing, &dir));
        assert!(same_file(&dir, &dir));
        assert!(is_under_canonical(&dir, &dir));
        assert_eq!(try_plain_canonical(&missing), None);
    }

    #[test]
    fn plain_canonical_answers_an_existing_path_without_a_verbatim_prefix() {
        let dir = std::env::temp_dir();
        let plain = plain_canonical(&dir);
        assert!(plain.is_absolute(), "{plain:?}");
        assert!(!plain.to_string_lossy().starts_with(r"\\?\"), "{plain:?}");
        // The plain spelling still names the same directory.
        assert_eq!(
            std::fs::canonicalize(&plain).ok(),
            std::fs::canonicalize(&dir).ok()
        );
        // A path that does not exist is answered as given, never dropped.
        let missing = dir.join("no-such-dir-4c1e9a");
        assert_eq!(plain_canonical(&missing), missing);
    }
}
