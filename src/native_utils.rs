// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

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
    // Native: prefer `deps/` over the uplifted `<profile>/libloft.rlib` —
    // the deps copy is what every binary links; the uplifted copy is only
    // refreshed by an explicit `cargo build --lib` and goes stale-by-content
    // whenever another build universe rewrites deps (#304/#307: post-rebase it
    // lacked the new `loft::rpc` module while generated code referenced it,
    // failing every `--native` compile with E0433).  Keep the ordering aligned
    // with `cache::rlib_candidates` and `native_lib::find_loft_rlib`.
    let deps = exe_dir.join("deps");
    if deps.join("libloft.rlib").exists() {
        return Some(deps);
    }
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

/// The dependency search dir for a [`loft_lib_dir`] result: `lib_dir` itself
/// when it already IS `deps/` (the preferred deps-first resolution, #304/#307),
/// else `lib_dir/deps`.  Appending "deps" unconditionally yields an invalid
/// `…/deps/deps` path that rustc can't search — E0463 "can't find crate" for
/// every transitive dep of libloft (sha2, rand_core, …).
pub(crate) fn deps_dir_of(lib_dir: &std::path::Path) -> std::path::PathBuf {
    if lib_dir.file_name().is_some_and(|n| n == "deps") {
        lib_dir.to_path_buf()
    } else {
        lib_dir.join("deps")
    }
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
    // @P360: this is a dev convenience — rebuild loft's OWN runtime rlib when
    // loft's Rust sources change, for `cargo run`-style use inside the loft
    // repo.  It compares the RELATIVE `src/`/`default/` mtimes, so when loft
    // runs against an external project that merely *has* a `src/` (e.g. a
    // library package's `src/*.loft`), it wrongly fires `cargo build --lib`
    // from a dir with no `Cargo.toml`, printing a confusing
    // "could not find Cargo.toml" error.  Gate the whole thing on a
    // `Cargo.toml` in cwd: `cargo build --lib` needs one there anyway, so if
    // absent we are not in the loft source root — use the shipped rlib as-is.
    if !std::path::Path::new("Cargo.toml").exists() {
        return;
    }
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
pub(crate) fn native_cache_key(
    rs_content: &[u8],
    lib_dir: Option<&std::path::Path>,
    data: Option<&crate::data::Data>,
) -> u64 {
    let mut key = fnv64(rs_content);
    if let Some(ld) = lib_dir {
        let rlib = ld.join("libloft.rlib");
        key ^= fnv64(rlib.to_string_lossy().as_bytes());
        // The main rlib MUST exist when `lib_dir` is Some (`loft_lib_dir` only
        // returns a dir that has it) — `expected: true` so a missing one (a
        // broken resolution) is loud, not a silent 0.
        fold_file_content(&mut key, &rlib, true);
    }
    // @P341: also fold each native PACKAGE rlib's path + content, so rebuilding
    // a library's `#native` crate (`lib/<pkg>/native/...`) invalidates the
    // cached test binary — which links those rlibs via `add_native_extern_flags`.
    // Without this, a cdylib fix is silently masked by a stale cached binary.
    if let Some(d) = data {
        for (crate_name, pkg_dir) in &d.native_packages {
            let rlib_name = format!("lib{}.rlib", crate_name.replace('-', "_"));
            let rlib = std::path::PathBuf::from(pkg_dir)
                .join("native")
                .join("target")
                .join("release")
                .join(&rlib_name);
            key ^= fnv64(rlib.to_string_lossy().as_bytes());
            // A package rlib may legitimately be not-yet-built → `expected:
            // false` (no warning; folds 0, the existing behaviour).
            fold_file_content(&mut key, &rlib, false);
        }
    }
    key
}

/// Fold a file's CONTENT hash into `key` (no-op if the file is missing).
///
/// BUILD2: keyed on bytes, not mtime, so the cache survives across CI runs.
/// `actions/cache` persists `target/` but every CI run reruns `cargo build
/// --release --lib`, which rewrites `libloft.rlib` with a fresh mtime even on a
/// no-op rebuild — an mtime fold then misses every time and recompiles every
/// native fixture from scratch.  rustc's rlib output is byte-deterministic for
/// unchanged sources (verified: a touch-and-rebuild yields an identical
/// sha256), so a content hash is stable across the no-op rebuild → warm-cache
/// hit, while still invalidating when the binary actually changes (different
/// bytes → different hash).  Results are memoised per path so a 14MB rlib is
/// hashed once per process, not once per fixture.
fn fold_file_content(key: &mut u64, path: &std::path::Path, expected: bool) {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<std::path::PathBuf, u64>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let hash = {
        let mut guard = map.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(&h) = guard.get(path) {
            h
        } else {
            let h = match std::fs::read(path) {
                Ok(b) => fnv64(&b),
                // A missing file folds to 0 (a not-yet-built PACKAGE rlib
                // legitimately contributes nothing).  But when the caller
                // declared it SHOULD exist (the main `libloft.rlib` this binary
                // links), missing means the cache key cannot reflect the loft
                // build it links → a stale binary may be reused.  Say so loudly;
                // the 0 is memoised, so this fires at most once per path.
                Err(_) => {
                    if expected {
                        eprintln!(
                            "loft: warning — fingerprint input {} is missing; native \
                             staleness detection is degraded (a stale binary may be reused). \
                             Rebuild loft, or clear ~/.loft/build-cache + the package \
                             native-auto/ dirs.",
                            path.display()
                        );
                    }
                    0
                }
            };
            guard.insert(path.to_path_buf(), h);
            h
        }
    };
    *key ^= hash;
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

/// @P350: parse the import section of a `loft --html` wasm and verify every
/// import module is one of the raw extern bridges the embedded HTML glue
/// provides (`loft_gl` / `loft_io`).  A wasm-bindgen-feature rlib — the
/// "rlib stomp" `make wasm` leaves at
/// `target/wasm32-unknown-unknown/release/libloft.rlib` — imports
/// `__wbindgen_placeholder__` (35+), which the glue cannot satisfy, so the
/// page never instantiates.  Returns `Ok(())` when every import module is
/// acceptable (a wasm with no imports included), or `Err(sorted distinct bad
/// module names)` otherwise.  Mirrors the import check in
/// `tools/check_html_bundle.mjs`, inline so it guards a bare `loft --html`
/// and not only the `make game` path.
///
/// The parser is deliberately conservative: on ANY shape it doesn't
/// understand (bad magic, truncated section, unknown import-descriptor kind)
/// it returns `Ok(())` rather than risk a false abort on a valid bundle — the
/// only path that errors is one where it successfully read import module
/// names and found one that is not `loft_gl`/`loft_io`.
pub(crate) fn html_wasm_import_modules_ok(wasm: &[u8]) -> Result<(), Vec<String>> {
    fn read_uleb(b: &[u8], p: &mut usize) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = *b.get(*p)?;
            *p += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }
    fn read_name<'a>(b: &'a [u8], p: &mut usize) -> Option<&'a [u8]> {
        let len = usize::try_from(read_uleb(b, p)?).ok()?;
        let s = b.get(*p..p.checked_add(len)?)?;
        *p += len;
        Some(s)
    }
    fn skip_limits(b: &[u8], p: &mut usize) -> Option<()> {
        let flag = *b.get(*p)?;
        *p += 1;
        read_uleb(b, p)?; // min
        if flag & 1 == 1 {
            read_uleb(b, p)?; // max
        }
        Some(())
    }
    // Header: magic "\0asm" + version.  Malformed → don't block (the browser
    // would surface a genuinely corrupt module); this parser only judges the
    // import modules of a wasm it can walk.
    if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
        return Ok(());
    }
    let mut p = 8;
    let mut bad: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    while p < wasm.len() {
        let Some(id) = wasm.get(p).copied() else {
            break;
        };
        p += 1;
        let Some(size) = read_uleb(wasm, &mut p) else {
            return Ok(());
        };
        let Ok(size) = usize::try_from(size) else {
            return Ok(());
        };
        let section_start = p;
        let section_end = match section_start.checked_add(size) {
            Some(e) if e <= wasm.len() => e,
            _ => return Ok(()),
        };
        if id == 2 {
            // Import section: u32 count, then `count` (module, field, desc).
            let mut ip = section_start;
            let Some(count) = read_uleb(wasm, &mut ip) else {
                return Ok(());
            };
            for _ in 0..count {
                let Some(module) = read_name(wasm, &mut ip) else {
                    return Ok(());
                };
                let Some(_field) = read_name(wasm, &mut ip) else {
                    return Ok(());
                };
                let Some(kind) = wasm.get(ip).copied() else {
                    return Ok(());
                };
                ip += 1;
                let ok = match kind {
                    0 => read_uleb(wasm, &mut ip).map(|_| ()), // func: typeidx
                    1 => {
                        // table: reftype byte + limits
                        ip += 1;
                        skip_limits(wasm, &mut ip)
                    }
                    2 => skip_limits(wasm, &mut ip), // mem: limits
                    3 => {
                        // global: valtype byte + mut byte
                        ip += 2;
                        Some(())
                    }
                    _ => None, // unknown kind — bail safe
                };
                if ok.is_none() {
                    return Ok(());
                }
                let m = String::from_utf8_lossy(module).into_owned();
                // loft's own host imports (`loft_io` / `loft_gl`) AND library-bridge
                // host imports (e.g. the crypto bridge's `loft_crypto.random_fill`)
                // all use a `loft_` prefix.  A wasm-bindgen stomp instead imports
                // `__wbindgen_*`, so a non-`loft_` module is the red flag this guards.
                if !m.starts_with("loft_") {
                    bad.insert(m);
                }
            }
        }
        p = section_end;
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.into_iter().collect())
    }
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

/// Whether the host-native backend links `#native` packages by C-ABI (their
/// cdylib `.so`) instead of as Rust rlibs — see NATIVE.md § Resolution: separate
/// the API id from the Rust part.  Both the codegen (`Output::native_cabi`) and
/// the linker flags below read this, so they always agree on a given host.
///
/// @PLN26 phase 4 — the C-ABI path (import-library linking + DLL staged beside
/// the binary on Windows, an RPATH'd `.so` elsewhere) is now the default on
/// EVERY host.  Windows was the last holdout; its arm was verified green in the
/// focused CI (`win-cdylib.yml` job `win-cdylib-cabi`, `LOFT_NATIVE_CABI=1`,
/// `native_crate_package_links_and_runs_via_cabi` PASS on `windows-latest`)
/// before this flip.  `LOFT_NATIVE_CABI=0` remains an escape hatch that forces
/// the legacy rlib link on any host should the C-ABI path regress; `=1` is now a
/// no-op (it already matches the default).
#[must_use]
pub(crate) fn native_cabi_enabled() -> bool {
    // The C-ABI path is the default on every host; only `LOFT_NATIVE_CABI=0`
    // opts back into the legacy rlib link (the escape hatch).
    !matches!(std::env::var("LOFT_NATIVE_CABI").ok().as_deref(), Some("0"))
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
        // Host-native C-ABI link (NATIVE.md § Resolution: separate the API id
        // from the Rust part).  Link the package's cdylib `.so` by C-ABI instead
        // of its rlib via `--extern`: the `.so` seals the package's whole Rust
        // crate graph (its own `loft_ffi` copy included), so no `-L dependency=`
        // or per-crate pinning is needed and the shared-dep `StableCrateId`
        // collision class is gone by construction.  Native target only — wasm
        // cross-compiles the package to an rlib (the branch below).
        if target.is_none() && native_cabi_enabled() {
            // @PLN26 phase 0.4 — resolve the `.so` exactly as the interpreter does
            // (`resolve_native_lib`): a host-triple prebuilt (ABI-gated on
            // `loft_ffi_fingerprint`) wins over a source build, a missing declared
            // system lib is terminal, else auto-build (freshness keyed on the
            // loft-ffi ABI, so the `.so` survives loft rebuilds).  The shared
            // resolver keeps native-compile and interpret on the SAME `.so` and
            // adds the prebuilt + missing-syslib handling the hand-rolled path lacked.
            // @PLN26 phase 0.4b — the cdylib is named after the `[library] native`
            // stem, which can differ from the crate name; read it from the manifest
            // (the SAME stem the interpreter resolves with) and fall back to the
            // crate name only when absent.  This keeps native-compile and interpret
            // on one `.so` even when `[lib] name` ≠ `crate_name`.
            let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
                .and_then(|m| m.native)
                .unwrap_or_else(|| crate_name.replace('-', "_"));
            let Some(so) = crate::extensions::resolve_native_lib(pkg_dir, &stem) else {
                // Unresolvable (missing system lib / build failed) — the resolver
                // already printed an actionable message.  Skip; the link then fails
                // loudly on the undefined symbol rather than silently mis-linking.
                continue;
            };
            let so_path = std::path::PathBuf::from(&so);
            if let Some(so_dir) = so_path.parent() {
                // `-l dylib=<name>` derived from the RESOLVED file (strip the `lib`
                // prefix + extension) so a prebuilt or non-`lib<stem>` cdylib links.
                let libname = so_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map_or(stem.as_str(), |s| s.strip_prefix("lib").unwrap_or(s));
                cmd.arg("-L").arg(format!("native={}", so_dir.display()));
                cmd.arg("-l").arg(format!("dylib={libname}"));
                if cfg!(windows) {
                    // @PLN26 phase 4 — Windows links a DLL through its IMPORT
                    // LIBRARY, and there is NO RPATH: the MSVC linker rejects
                    // `-Wl,-rpath`, and the loader finds the DLL beside the `.exe`
                    // / on `PATH` — so the DLL is staged beside the binary at run
                    // time (`stage_native_dlls`), the Windows form of the
                    // `$ORIGIN` rpath used below.
                    //
                    // Naming bridge: a Rust cdylib's import lib is `<stem>.dll.lib`,
                    // but `-l dylib=<stem>` makes MSVC link.exe open `<stem>.lib`
                    // (verified: `LNK1181: cannot open input file
                    // 'loft_native_scalar.lib'`).  Copy `<stem>.dll.lib` →
                    // `<stem>.lib` beside it so the `-l dylib=` above resolves —
                    // both are import libs for the same DLL, identical content.
                    let dll_lib = so_dir.join(format!("{libname}.dll.lib"));
                    let plain_lib = so_dir.join(format!("{libname}.lib"));
                    if dll_lib.exists() && !plain_lib.exists() {
                        let _ = std::fs::copy(&dll_lib, &plain_lib);
                    }
                    // Disallow-the-unverifiable-loudly: if NEITHER import-lib name
                    // is present the link would die on an opaque `LNK1181`, so name
                    // it rather than mis-link.
                    if !plain_lib.exists() && !dll_lib.exists() {
                        eprintln!(
                            "loft: native package `{crate_name}` cdylib at {} has no import \
                             library (`{libname}.dll.lib` / `{libname}.lib`) — Windows links a \
                             DLL through its import lib, not the DLL directly (@PLN26 phase 4).  \
                             Rebuild the package's cdylib with a toolchain that emits one.",
                            so_dir.display()
                        );
                    }
                } else {
                    // @PLN26 phase 0.1 — two RPATH entries: the build/prebuilt dir
                    // (run-from-build-tree: tests, dev) AND `$ORIGIN` (an installed
                    // binary that ships the `.so` beside it — `make install` copies
                    // it next to the binary).  `$ORIGIN` is passed literally; the
                    // dynamic loader expands it at run time.  Windows has no RPATH
                    // (the arm above); it stages the DLL beside the binary instead.
                    cmd.arg(format!("-Clink-arg=-Wl,-rpath,{}", so_dir.display()));
                    cmd.arg("-Clink-arg=-Wl,-rpath,$ORIGIN");
                }
            }
            continue;
        }
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
            // @PLAN12 Phase 6b — native target via the shared helper:
            // chunk-resident installs (~/.loft/registry/<pkg>-<ver>/)
            // get the redirected target at ~/.loft/build-cache/<pkg>-<ver>/;
            // monorepo lib/<pkg>/native/ keeps in-tree target/.  Must
            // match what `extensions::auto_build_native` writes so the
            // rlib is found at link time.
            crate::extensions::native_target_root(std::path::Path::new(pkg_dir))
                .join("release")
                .join(&rlib_name)
        };
        // @P359: on a clean checkout the package-under-test's rlib may not
        // exist yet at link time — parse-time `auto_build_native` only fires
        // for *dependency* packages (resolved via `use`), not for the package
        // being tested directly.  CI runs a single fresh `loft --native test`,
        // so without this the first (only) run links nothing and rustc errors
        // E0463 "can't find crate".  Build it on demand here.  Native target
        // only; WASM relies on prebuilt rlibs (no host cargo build).
        // @PLN11 Arc N / N0 — rebuild on a missing rlib OR a stale one (a package
        // rlib built by a DIFFERENT loft build, e.g. left by `make
        // rebuild-native-cdylibs` before loft's ABI changed).  Without the
        // fingerprint check the stale rlib is linked as-is → the "generated
        // rust-code error".  `auto_build_native` re-checks + re-stamps the sidecar.
        // #274 — key on the SAME `native_artifact_cache_key` that `auto_build_native`
        // stamps with (loft-ffi ABI + RUSTFLAGS), so this link-time gate agrees with
        // the build-time stamp: a flag-divergent rlib (whose shared `libloading` would
        // collide with loft's copy) reads as stale here and is rebuilt.
        let profile_dir = rlib_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let stale = !loft::cache::native_artifact_fingerprint_matches(
            profile_dir,
            loft::cache::native_artifact_cache_key(),
        );
        if target.is_none() && (!rlib_path.exists() || stale) {
            let stem = crate_name.replace('-', "_");
            let _ = crate::extensions::auto_build_native(pkg_dir, &stem);
        }
        // @PLN26 phase 3 — a wasm target needs the package's rlib (wasm links statically;
        // the C-ABI `.so` path is host-only).  When neither a shipped `prebuilt/` rlib nor a
        // prior cross-build is present, CROSS-BUILD the package's native crate to the wasm
        // target on demand — `auto_build_native_target`, the wasm sibling of
        // `auto_build_native`, lands the rlib at the in-tree path `rlib_path` reads below.
        // Only if that can't produce the rlib (no toolchain/target, or the crate is not
        // wasm-clean) do we emit a clear signal instead of dying on a bare E0463.
        if let Some(tgt) = target {
            // Cross-build on demand UNLESS a `prebuilt/<t>/` rlib is shipped (trusted as
            // sent).  `auto_build_native_target` reuses an ABI-fresh in-tree rlib and
            // rebuilds a stale/missing one, so calling it every wasm compile is cheap (a
            // fingerprint read) and keeps a stale wasm rlib from being linked after a
            // loft-ffi ABI change — the wasm analogue of the host `stale` gate above.
            let prebuilt = std::path::PathBuf::from(pkg_dir)
                .join("prebuilt")
                .join(tgt)
                .join(&rlib_name);
            if !prebuilt.exists() {
                let stem = crate_name.replace('-', "_");
                crate::extensions::auto_build_native_target(pkg_dir, &stem, tgt);
            }
            if !rlib_path.exists() {
                eprintln!(
                    "loft: native package `{crate_name}` has no {tgt} build (no prebuilt, and \
                     cross-build unavailable or the crate is not wasm-clean) — ship a prebuilt \
                     rlib for the package, or run with --interpret."
                );
            }
        }
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
            // @PLN26 phase 3 — a wasm cross-build keeps the package's PROC-MACRO deps
            // (e.g. `loft-ffi-macros`, used by its `#[loft_native]` bridge) as HOST
            // artifacts under `native/target/release/deps` (cargo builds proc-macros for
            // the host even with `--target`).  The wasm `<target>/release/deps` dir above
            // holds only the wasm rlibs, so `extern crate <pkg>` fails to resolve the
            // proc-macro (E0463) without this.  rustc filters by target, so the host
            // loft-ffi rlib that also lives here is ignored in favour of the wasm one.
            if target.is_some() {
                let host_deps = std::path::PathBuf::from(pkg_dir)
                    .join("native")
                    .join("target")
                    .join("release")
                    .join("deps");
                if host_deps.is_dir() {
                    cmd.arg("-L")
                        .arg(format!("dependency={}", host_deps.display()));
                }
            }
            // @P229 (G2): harvest build-script `rustc-link-search` dirs from
            // THIS native package's own target tree, not just the top-level
            // one.  A diamond-dep can pull a second `windows_x86_64_msvc`
            // version (e.g. graphics → winit/glutin pulls 0.52.6) built under
            // `<pkg>/native/target/release/build`, whose `windows.0.52.0.lib`
            // the top-level harvest at the call site never sees — so its
            // `/LIBPATH` is missing and the link fails `LNK1181: cannot open
            // input file 'windows.0.52.0.lib'`.  `rlib_path.parent()` is the
            // package's `<profile>` dir, exactly what the helper expects.
            if let Some(profile_dir) = rlib_path.parent() {
                for nd in build_script_native_lib_dirs(profile_dir) {
                    cmd.arg("-L").arg(format!("native={}", nd.display()));
                }
            }
        }
    }
}

/// @PLN26 phase 4 — stage every native-package DLL beside a just-built Windows
/// binary so it loads at run time.  Windows has no RPATH, so the loader finds a
/// linked DLL beside the `.exe` / on `PATH`; copying it next to the binary is the
/// Windows form of the `$ORIGIN` rpath the ELF link embeds (`add_native_extern_flags`).
/// No-op off Windows and when the C-ABI path is disabled (the rlib path links the
/// package statically, so there is nothing to stage).  Resolves the DLL with the
/// same `resolve_native_lib` the link used, so run-time and link-time agree on the
/// file.  Best-effort: a failed copy leaves the loader's normal search to find it.
pub(crate) fn stage_native_dlls(exe_dir: &std::path::Path, data: &crate::data::Data) {
    if !cfg!(windows) || !native_cabi_enabled() {
        return;
    }
    for (crate_name, pkg_dir) in &data.native_packages {
        let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
            .and_then(|m| m.native)
            .unwrap_or_else(|| crate_name.replace('-', "_"));
        if let Some(so) = crate::extensions::resolve_native_lib(pkg_dir, &stem) {
            let so_path = std::path::PathBuf::from(&so);
            if let Some(name) = so_path.file_name() {
                let dest = exe_dir.join(name);
                if dest != so_path {
                    let _ = std::fs::copy(&so_path, &dest);
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

#[cfg(test)]
mod p350_html_wasm_import_check {
    use super::*;

    /// Build a minimal valid wasm (header + one import section) whose single
    /// import has the given module name and is a function import (kind 0).
    fn wasm_with_import_module(module: &str) -> Vec<u8> {
        // import entry: <ulen module><module><ulen field><field><kind=0><typeidx=0>
        let field = "f";
        let mut entry = Vec::new();
        entry.push(module.len() as u8);
        entry.extend_from_slice(module.as_bytes());
        entry.push(field.len() as u8);
        entry.extend_from_slice(field.as_bytes());
        entry.push(0x00); // kind: func
        entry.push(0x00); // typeidx 0
        // import section body: <count=1><entry>
        let mut body = vec![0x01u8];
        body.extend_from_slice(&entry);
        // section: <id=2><usize len><body>
        let mut wasm = b"\0asm".to_vec();
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1
        wasm.push(0x02); // import section id
        wasm.push(body.len() as u8); // section size (small enough for one byte uleb)
        wasm.extend_from_slice(&body);
        wasm
    }

    #[test]
    fn accepts_loft_gl_and_loft_io() {
        assert!(html_wasm_import_modules_ok(&wasm_with_import_module("loft_gl")).is_ok());
        assert!(html_wasm_import_modules_ok(&wasm_with_import_module("loft_io")).is_ok());
    }

    #[test]
    fn rejects_wbindgen_stomp() {
        let bad = html_wasm_import_modules_ok(&wasm_with_import_module("__wbindgen_placeholder__"))
            .expect_err("wbindgen import module must be rejected");
        assert_eq!(bad, vec!["__wbindgen_placeholder__".to_string()]);
    }

    #[test]
    fn no_imports_is_ok() {
        // Bare header, no sections — a wasm with zero imports is acceptable.
        let mut wasm = b"\0asm".to_vec();
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        assert!(html_wasm_import_modules_ok(&wasm).is_ok());
    }

    #[test]
    fn malformed_does_not_false_abort() {
        // Bad magic / truncated input must be conservatively accepted (Ok),
        // never a false rejection of a bundle this parser can't read.
        assert!(html_wasm_import_modules_ok(b"not a wasm").is_ok());
        assert!(html_wasm_import_modules_ok(&[]).is_ok());
        assert!(html_wasm_import_modules_ok(b"\0asm\x01\x00\x00\x00\x02\xff").is_ok());
    }
}
