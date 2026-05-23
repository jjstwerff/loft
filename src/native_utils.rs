// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native compilation utilities: rlib management, cache keys, artifact paths.

use std::env;
pub(crate) fn with_trailing_sep(p: &std::path::Path) -> String {
    let mut s = p.to_str().unwrap_or("").to_string();
    if !s.ends_with('/') && !s.ends_with('\\') {
        s.push(std::path::MAIN_SEPARATOR);
    }
    s
}

/// Return the directory that contains `libloft.rlib` for the given target triple.
/// Pass `None` for the native target, `Some("wasm32-wasip2")` for WASM.
/// Returns `None` when the rlib cannot be located.
pub(crate) fn loft_lib_dir_for(target: Option<&str>) -> Option<std::path::PathBuf> {
    let exe_dir = env::current_exe().ok()?.parent()?.to_path_buf();
    // Dev layout: <project>/target/release/loft  or  <project>/target/debug/loft
    // The wasm rlib lives at <project>/target/wasm32-wasip2/release/
    if let Some(triple) = target {
        // Walk up to find a sibling target/<triple>/release directory.
        let mut dir = exe_dir.clone();
        loop {
            let candidate = dir.join("target").join(triple).join("release");
            if candidate.join("libloft.rlib").exists() {
                return Some(candidate);
            }
            // Installed layout: <prefix>/share/loft/<triple>/
            if dir.file_name().is_some_and(|n| n == "bin") {
                let share = dir.parent()?.join("share").join("loft").join(triple);
                if share.join("libloft.rlib").exists() {
                    return Some(share);
                }
            }
            if !dir.pop() {
                break;
            }
        }
        return None;
    }
    // Native: look next to the binary first (dev build in target/release/).
    if exe_dir.join("libloft.rlib").exists() {
        return Some(exe_dir.clone());
    }
    // Cargo places rlibs in target/<profile>/deps/ — check that too.
    let deps = exe_dir.join("deps");
    if deps.join("libloft.rlib").exists() {
        return Some(deps);
    }
    // Installed as <prefix>/bin/loft — look in <prefix>/share/loft/.
    if exe_dir.file_name()? == "bin" {
        let share = exe_dir.parent()?.join("share").join("loft");
        if share.join("libloft.rlib").exists() {
            return Some(share);
        }
    }
    None
}

pub(crate) fn loft_lib_dir() -> Option<std::path::PathBuf> {
    loft_lib_dir_for(None)
}

/// Parse every `target/<profile>/build/*/output` file and extract
/// `cargo:rustc-link-search=native=<path>` directives, returning the
/// list of directories that should be passed to rustc as
/// `-L native=<path>`.
///
/// Cargo passes these flags automatically when building the loft
/// binary itself, but when loft's `--native` mode invokes rustc
/// directly to compile generated user code we must replicate the
/// link environment by hand.  Without these search paths the
/// Windows link step fails with `LNK1181: cannot open input file
/// 'windows.0.48.5.lib'` because the `windows-targets` crate emits a
/// search path pointing into its registry source directory (not
/// `OUT_DIR`).
///
/// `lib_dir` is either `target/<profile>/` or `target/<profile>/deps/`
/// — both resolve to the same parent build root.
pub(crate) fn build_script_native_lib_dirs(lib_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let target_profile = if lib_dir.file_name().is_some_and(|n| n == "deps") {
        match lib_dir.parent() {
            Some(p) => p,
            None => return Vec::new(),
        }
    } else {
        lib_dir
    };
    let build_root = target_profile.join("build");
    let mut seen: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&build_root) else {
        return out;
    };
    for entry in entries.flatten() {
        let output_path = entry.path().join("output");
        let Ok(text) = std::fs::read_to_string(&output_path) else {
            continue;
        };
        for line in text.lines() {
            // Strip optional `cargo::` (1.77+) or `cargo:` (legacy) prefix.
            let body = line
                .strip_prefix("cargo::")
                .or_else(|| line.strip_prefix("cargo:"))
                .unwrap_or(line);
            // Match `rustc-link-search=native=<path>` and the form
            // without an explicit kind (`rustc-link-search=<path>`,
            // which defaults to `all` and includes native).
            let Some(rest) = body.strip_prefix("rustc-link-search=") else {
                continue;
            };
            let path_str = rest.strip_prefix("native=").unwrap_or(rest);
            // Skip framework= / dependency= / crate= kinds — they're
            // unrelated to native lib search.
            if rest.starts_with("framework=")
                || rest.starts_with("dependency=")
                || rest.starts_with("crate=")
            {
                continue;
            }
            let p = std::path::PathBuf::from(path_str);
            if seen.insert(p.clone()) {
                out.push(p);
            }
        }
    }
    out
}

/// Ensure `libloft.rlib` is at least as fresh as the newest `src/*.rs` file.
/// If any source is newer, run `cargo build --lib` to rebuild it.
pub(crate) fn ensure_rlib_fresh() {
    let Some(lib_dir) = loft_lib_dir() else {
        // No rlib found at all — try building from scratch.
        let _ = std::process::Command::new("cargo")
            .args(["build", "--lib"])
            .status();
        return;
    };
    let rlib = lib_dir.join("libloft.rlib");
    let Ok(rlib_mtime) = std::fs::metadata(&rlib).and_then(|m| m.modified()) else {
        return;
    };
    // Walk src/ for the newest .rs file.
    let newest_src = newest_mtime_in("src");
    // Also check default/*.loft — changes there affect codegen output.
    let newest_default = newest_mtime_in("default");
    let newest = newest_src.max(newest_default);
    if newest.is_some_and(|t| t > rlib_mtime) {
        eprintln!("loft: rebuilding libloft.rlib (source is newer)...");
        let _ = std::process::Command::new("cargo")
            .args(["build", "--lib"])
            .status();
    }
}

/// Return the newest modification time of any file under `dir` (recursive).
pub(crate) fn newest_mtime_in(dir: &str) -> Option<std::time::SystemTime> {
    fn walk(path: &std::path::Path, best: &mut Option<std::time::SystemTime>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, best);
            } else if let Ok(m) = p.metadata().and_then(|m| m.modified()) {
                *best = Some(best.map_or(m, |b: std::time::SystemTime| b.max(m)));
            }
        }
    }
    let mut best = None;
    walk(std::path::Path::new(dir), &mut best);
    best
}

/// FNV-1a 64-bit hash for native binary cache keys.
pub(crate) fn fnv64(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Build a cache key from generated Rust source and the rlib identity.
pub(crate) fn native_cache_key(rs_content: &[u8], lib_dir: Option<&std::path::Path>) -> u64 {
    let mut key = fnv64(rs_content);
    if let Some(ld) = lib_dir {
        let rlib = ld.join("libloft.rlib");
        key ^= fnv64(rlib.to_string_lossy().as_bytes());
        if let Ok(mtime) = std::fs::metadata(&rlib).and_then(|m| m.modified()) {
            let d = mtime
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap_or_default();
            key ^= fnv64(&d.as_secs().to_le_bytes());
            key ^= fnv64(&d.subsec_nanos().to_le_bytes());
        }
    }
    key
}

/// Return true if `s` looks like an explicit output path rather than a flag or loft source file.
pub(crate) fn is_output_path(s: &str) -> bool {
    !s.starts_with('-')
        && !std::path::Path::new(s)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("loft"))
}

/// Return (and create) the `.loft/` artifact directory beside `script_path`.
/// Falls back to the current directory's `.loft/` if the parent cannot be determined.
pub(crate) fn loft_artifact_dir(script_path: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(script_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let loft_dir = dir.join(".loft");
    let _ = std::fs::create_dir_all(&loft_dir);
    loft_dir
}

/// Return the default output path for a compiled artifact beside `script_path`.
/// `ext` is the file extension without leading dot (e.g. `"wasm"`, `"rs"`).
pub(crate) fn default_artifact_path(script_path: &str, ext: &str) -> std::path::PathBuf {
    let stem = std::path::Path::new(script_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out");
    loft_artifact_dir(script_path).join(format!("{stem}.{ext}"))
}

pub(crate) fn project_dir() -> String {
    let Ok(prog) = env::current_exe() else {
        return String::new();
    };
    let Some(dir) = prog.parent() else {
        return String::new();
    };
    // Strip target/release or target/debug to get the project root.
    if (dir.ends_with("target/release") || dir.ends_with("target\\release"))
        && let Some(root) = dir.parent().and_then(|p| p.parent())
    {
        return with_trailing_sep(root);
    }
    if (dir.ends_with("target/debug") || dir.ends_with("target\\debug"))
        && let Some(root) = dir.parent().and_then(|p| p.parent())
    {
        return with_trailing_sep(root);
    }
    // Installed binary: binary is in <prefix>/bin/, stdlib in <prefix>/share/loft/.
    if dir.ends_with("bin")
        && let Some(prefix) = dir.parent()
    {
        let share_loft = prefix.join("share").join("loft");
        if share_loft.is_dir() {
            return with_trailing_sep(&share_loft);
        }
        return with_trailing_sep(prefix);
    }
    with_trailing_sep(dir)
}

/// P254 — cache-poisoning defense.  Returns true when a cached
/// native binary at `path` is safe to execute under the current
/// uid.  All platforms reject symlinks (a symlink lets an attacker
/// redirect execution to anything they can name).  On Unix we
/// additionally require:
///
/// - The file owner equals the current effective uid (root-owned
///   files are also rejected when running as a non-root user — a
///   common shared-machine attack drops a SUID binary owned by
///   root).
/// - No group/other permission bits set (mode `& 0o077 == 0`).
///
/// Failed checks return false WITHOUT a side effect; the caller
/// recompiles and overwrites the suspect cache file.  We do not
/// `eprintln!` from the helper so unit tests can assert the
/// boolean cleanly; the recompile path's `eprintln!` covers user
/// visibility.
#[must_use]
pub(crate) fn cache_safe_to_execute(path: &std::path::Path) -> bool {
    // symlink_metadata reports the link itself rather than the
    // target; if the target is owned by the current uid but the
    // link itself isn't, plain metadata() would say "safe" and
    // we'd execute attacker-pointed code.
    let lmd = match std::fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(_) => return false,
    };
    if lmd.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if lmd.uid() != unsafe { libc::geteuid() } {
            return false;
        }
        // Reject group/other permissions — only owner may read,
        // write, or execute.  An attacker with group access
        // could swap the file between our stat and exec; even
        // group-readable files leak compiled output that may
        // contain secrets.
        if lmd.mode() & 0o077 != 0 {
            return false;
        }
        // Reject SUID/SGID — cached binaries should never carry
        // privilege-escalation bits.
        if lmd.mode() & 0o6000 != 0 {
            return false;
        }
    }
    true
}

/// P254 — companion check for the cache directory itself.
/// Same shape as `cache_safe_to_execute` but applied to the
/// directory: symlink rejected; on Unix, owner-uid match and
/// no group/other bits required.  Used at cache-write time —
/// if the directory exists with weak permissions, we tighten
/// it via `tighten_cache_dir` before writing.
#[must_use]
pub(crate) fn cache_dir_safe(dir: &std::path::Path) -> bool {
    let lmd = match std::fs::symlink_metadata(dir) {
        Ok(md) => md,
        Err(_) => return false,
    };
    if lmd.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if lmd.uid() != unsafe { libc::geteuid() } {
            return false;
        }
        if lmd.mode() & 0o077 != 0 {
            return false;
        }
    }
    true
}

/// P254 — set the cache directory's mode to `0o700` on Unix.
/// No-op on non-Unix (NTFS / ReFS use ACLs that the parent
/// directory already restricts; the symlink check still applies).
/// Called both immediately after `create_dir_all` and before any
/// cache write to repair pre-existing cache directories left over
/// from earlier loft versions.
pub(crate) fn tighten_cache_dir(dir: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let _ = dir;
}

/// P254 — set a freshly written cache binary's mode to `0o700`
/// on Unix (rwx for owner, nothing for group/other).  Called
/// after `std::fs::copy(&binary, &cached_binary)`.  No-op on
/// non-Unix.
pub(crate) fn tighten_cache_binary(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    let _ = path;
}

/// Collect crate names → rlib paths from a deps directory
/// (e.g. `libfoo-<hash>.rlib` → `("foo", "/path/to/libfoo-<hash>.rlib")`).
pub(crate) fn rlibs_in_dir(
    dir: &std::path::Path,
) -> std::collections::HashMap<String, std::path::PathBuf> {
    let mut map = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
            {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(rest) = fname.strip_prefix("lib") {
                    if let Some(dash_pos) = rest.rfind('-') {
                        map.insert(rest[..dash_pos].to_string(), path);
                    }
                }
            }
        }
    }
    map
}

/// PKG.4/PKG.5: add `--extern` flags to a rustc command for native package rlibs.
/// When `target` is `Some("wasm32-wasip2")`, looks for WASM rlibs in `prebuilt/wasm32-wasip2/`;
/// otherwise looks for native rlibs in `native/target/release/`.
///
/// Uses `-L dependency=` for the native package's deps so deep transitive deps
/// resolve. For any crate that also appears in loft's own deps, adds an explicit
/// `--extern name=<loft's copy>` so rustc uses a single copy, avoiding
/// StableCrateId collisions.
///
/// Shared by the standalone native compile (`main.rs`), the WASM compile, and
/// the native test runner (`test_runner.rs`) so all three link a package's
/// `#native` crate identically (LibCI native library gate).
pub(crate) fn add_native_extern_flags(
    cmd: &mut std::process::Command,
    data: &crate::data::Data,
    target: Option<&str>,
    loft_deps_dir: Option<&std::path::Path>,
) {
    let loft_rlibs = loft_deps_dir.map(rlibs_in_dir).unwrap_or_default();

    for (crate_name, pkg_dir) in &data.native_packages {
        // Look for the compiled rlib in the package's native crate output.
        let rlib_name = format!("lib{}.rlib", crate_name.replace('-', "_"));
        // P244-windows fix #2 (2026-05-12): use single-segment joins,
        // not `.join("native/target/release")` with embedded slashes.
        // When `pkg_dir` is a Windows extended-length path (`\\?\D:\…`),
        // a multi-segment join string with `/` separators inside the
        // verbatim namespace doesn't normalize and the resulting path
        // doesn't match real on-disk files.  Each `.join("X")` with a
        // single component is normalized correctly by `Path` semantics.
        let rlib_path = if let Some(tgt) = target {
            // WASM: check prebuilt first, then native/target/<target>/release/
            let prebuilt = std::path::PathBuf::from(pkg_dir)
                .join("prebuilt")
                .join(tgt)
                .join(&rlib_name);
            if prebuilt.exists() {
                prebuilt
            } else {
                std::path::PathBuf::from(pkg_dir)
                    .join("native")
                    .join("target")
                    .join(tgt)
                    .join("release")
                    .join(&rlib_name)
            }
        } else {
            // Native: check native/target/release/
            std::path::PathBuf::from(pkg_dir)
                .join("native")
                .join("target")
                .join("release")
                .join(&rlib_name)
        };
        if rlib_path.exists() {
            let extern_name = crate_name.replace('-', "_");
            cmd.arg("--extern")
                .arg(format!("{}={}", extern_name, rlib_path.display()));
            // Add the native crate's deps directory so transitive deps (GL, glutin, etc.)
            // resolve. Use `dependency` search scope so these crates are only found as
            // transitive deps of the native crate, not as direct deps.
            let deps_dir = rlib_path.parent().unwrap().join("deps");
            if deps_dir.is_dir() {
                cmd.arg("-L")
                    .arg(format!("dependency={}", deps_dir.display()));
                // Pin any crate that also exists in loft's deps to loft's copy,
                // preventing StableCrateId collisions from duplicate rlibs.
                if !loft_rlibs.is_empty() {
                    let pkg_crates = rlibs_in_dir(&deps_dir);
                    for (dep_name, loft_path) in &loft_rlibs {
                        if pkg_crates.contains_key(dep_name) {
                            cmd.arg("--extern").arg(format!(
                                "{}={}",
                                dep_name,
                                loft_path.display()
                            ));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod p254_cache_safety {
    use super::*;

    #[test]
    fn nonexistent_cache_is_unsafe() {
        let p = std::env::temp_dir().join("loft_p254_does_not_exist_xyz_12345");
        let _ = std::fs::remove_file(&p);
        assert!(!cache_safe_to_execute(&p));
    }

    #[test]
    fn freshly_written_owner_only_cache_is_safe() {
        let p = std::env::temp_dir().join(format!(
            "loft_p254_safe_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
        tighten_cache_binary(&p);
        assert!(cache_safe_to_execute(&p), "owner-only file should be safe");
        let _ = std::fs::remove_file(&p);
    }

    #[cfg(unix)]
    #[test]
    fn group_writable_cache_is_unsafe() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!(
            "loft_p254_groupwrite_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"x").unwrap();
        // 0o766 has group write and other rwx — attacker-modifiable.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o766)).unwrap();
        assert!(!cache_safe_to_execute(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cache_is_unsafe() {
        let target = std::env::temp_dir().join(format!(
            "loft_p254_symtarget_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let link = std::env::temp_dir().join(format!(
            "loft_p254_symlink_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_file(&link);
        std::fs::write(&target, b"x").unwrap();
        tighten_cache_binary(&target);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        // Even though the symlink TARGET would pass, the link
        // itself routes through `symlink_metadata` and is rejected.
        assert!(!cache_safe_to_execute(&link));
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);
    }

    #[cfg(unix)]
    #[test]
    fn suid_cache_is_unsafe() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!(
            "loft_p254_suid_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"x").unwrap();
        // 0o4700 — owner rwx + setuid bit.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o4700)).unwrap();
        assert!(!cache_safe_to_execute(&p));
        let _ = std::fs::remove_file(&p);
    }

    /// Lightweight nanosecond suffix to keep test temp paths
    /// from colliding when `cargo test` runs the cases in
    /// parallel without `--test-threads=1`.
    fn chrono_ish_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    }
}
