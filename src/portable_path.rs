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

use std::path::Path;

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
/// position's `file`, the logger's project root — builds and compares plain paths.  A
/// verbatim path never equals or prefix-matches its plain twin (`VerbatimDisk` vs `Disk`
/// components), so every canonicalised path that enters the shared path space sheds the
/// prefix here.  A path that cannot be canonicalised is answered as given rather than
/// dropped.  No-op on Linux/macOS.
#[must_use]
pub fn plain_canonical(path: &Path) -> std::path::PathBuf {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    match abs.to_str() {
        Some(text) => std::path::PathBuf::from(strip_verbatim_disk(text.to_string())),
        None => abs,
    }
}

/// Shed the `\\?\` prefix of a verbatim DISK path (`\\?\D:\…` → `D:\…`), which is the
/// form `canonicalize` produces on Windows.  Only the disk form is stripped: verbatim-UNC
/// (`\\?\UNC\…`) has no plain equivalent and is left as it is.  A path without the prefix
/// is answered unchanged, so this is safe to apply to any spelling.
#[must_use]
pub fn strip_verbatim_disk(path: String) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        rest.to_string()
    } else {
        path
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
    fn a_verbatim_disk_prefix_is_shed_and_nothing_else_is_touched() {
        assert_eq!(
            strip_verbatim_disk(r"\\?\C:\work\a.loft".into()),
            r"C:\work\a.loft"
        );
        assert_eq!(
            strip_verbatim_disk(r"\\?\UNC\srv\share\a.loft".into()),
            r"\\?\UNC\srv\share\a.loft"
        );
        assert_eq!(
            strip_verbatim_disk(r"C:\work\a.loft".into()),
            r"C:\work\a.loft"
        );
        assert_eq!(
            strip_verbatim_disk("/home/u/a.loft".into()),
            "/home/u/a.loft"
        );
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
