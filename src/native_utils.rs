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
