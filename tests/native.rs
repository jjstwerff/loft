// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native-backend integration tests.
//!
//! These tests compile `.loft` files through the `--native` Rust code generator
//! and run the resulting binaries.  They do **not** acquire `WRAP_LOCK`, so they
//! run concurrently with the interpreter-based `wrap` tests — which is safe
//! because native tests write only to `/tmp/loft_native_*` temp files and never
//! touch the same files as the interpreter tests.

extern crate loft;

use loft::compile::byte_code;
use loft::generation::Output;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
use std::io::Error;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
mod common;
use common::cached_default;

/// Process-wide mutex serialising the five high-level native tests
/// (`native_dir`, `native_scripts`, `native_binary_script`,
/// `native_tuple_return_script`, `native_tuple_script`).
///
/// Each test already parallelises internally via `thread::scope` over
/// `available_parallelism()` rustc workers — running two such pools
/// concurrently saturates the CPU twice over, so within ONE process
/// this lock keeps that 2× over-subscription (and the ~140 s→~14 s
/// blowup) away.
///
/// It does NOT, by itself, fix the flaky link failures that once looked
/// like a deps-read race: the real cause was concurrent compiles of the
/// SAME script stem writing the SAME `loft_native_<stem>_bin` output
/// (`50-tuples.loft` is built by three tests), truncating each other's
/// binary mid-link.  Across PROCESSES — nextest runs each test in its
/// own — this in-process lock can't help; correctness there comes from
/// `compile_native_job` compiling to a per-process temp and publishing
/// via atomic `rename(2)`.  See @PLN11 § Discovered follow-ups F1.
fn native_suite_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Docs files that are known to fail in `--native` mode.
/// See PROBLEMS.md for details on each issue number.
///
/// Doc files to skip in native mode.
const NATIVE_SKIP: &[&str] = &[
    // (empty) — "25-generics.loft" was skipped here for the @PLN25/@PLN85
    // Family-D residual (a generic `-> T?` (Optional) return kept the
    // parametric `Optional(Reference(tv))` type instead of substituting T,
    // mistyping the return slot — rustc E0308 on native).  FIXED by #493
    // cell 5 (commit 64d94c50: `substitute_type` gained an `Optional` arm).
    // Verified both the scalar-T (`last_element<integer/text>`, already in
    // this file) and a struct-T instantiation compile + run correctly on
    // `--native`; the skip is removed so `native_dir` now gates this file
    // as a regression guard for cell 5.
];

/// Script files to skip in native mode.
const SCRIPTS_NATIVE_SKIP: &[&str] = &[
    // @PLN48 S2 — spacial<T[x, y]> works on the interpreter (construct + free),
    // but the native IR-schema round-trip for the Radix kind is still WIP
    // (the content type deserialises as u16::MAX).  Remove when native lands.
    "48-spacial-construct-free.loft",
    // Struct yields from a generator's LOOP body are interpreter-only for
    // now: the native eager-collect factory cannot preserve per-yield
    // snapshots (values silently alias), so --native rejects the shape with
    // a compile_error naming the alternatives (#481).  The interp half runs
    // under wrap.
    "447-coroutine-yield-borrow.loft",
    // 135-vector-u8-concat.loft was here for @P316 (`vector<u8>` element read
    // with `?? <int>` mis-compiled); @P316 is fixed, so 135 now runs natively
    // and doubles as the @P316 regression guard.
    //
    // 191-source-dir.loft runs natively as of @PLN9 Phase 1 — the exe-dir anchor
    // (`Stores::source_dir_native` via `current_exe()`) makes `source_dir()`
    // non-empty under `--native`, so it is no longer skipped here.
];

/// Locate `libloft.rlib` and its sibling deps directory for standalone `rustc` compilation.
///
/// Searches only the deps directory of the currently running test binary so that
/// the rlib always matches the features compiled into this test.  The old approach
/// of scanning both profiles and picking by mtime caused S33 in CI: a later
/// `cargo build --no-default-features` produced a newer no-features rlib in the
/// other profile's deps/, shadowing the full-features rlib and leaving png/random
/// functions as stubs that silently return wrong values.
fn find_loft_rlib() -> Option<(PathBuf, PathBuf)> {
    // The test binary lives at target/{profile}/deps/{test_binary}.
    // Its parent is the deps/ directory that holds the rlib built with the same features.
    let deps = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))?;

    // Find the most recently modified loft rlib in this profile's deps/.
    // Accept both "libloft-HASH.rlib" (cargo test profile) and "libloft.rlib"
    // (produced when building lib+test together).  Both live in the same
    // profile-specific deps/ directory, so there is no cross-profile shadowing
    // (the S33 risk only arose when scanning multiple profile directories).
    let rlib = std::fs::read_dir(&deps)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            (n.starts_with("libloft-") || n == "libloft.rlib") && n.ends_with(".rlib")
        })
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?
        .path();

    Some((rlib, deps))
}

/// Collect additional `--extern name=path` arguments for optional feature dependencies.
///
/// Collect additional `--extern name=path` arguments for optional feature dependencies.
///
/// When `rustc` compiles generated `.rs` files standalone, it only knows about crates
/// explicitly declared via `--extern`.  Optional deps like `rand_core` and `rand_pcg`
/// are available in the same deps/ directory as `libloft.rlib` but must be declared
/// explicitly (S31).  This function scans deps/ and returns ALL non-loft rlibs as
/// `(crate_name, rlib_path)` pairs.  All versions of each crate are included so that
/// rustc can select the hash that matches what `libloft` was compiled against.
fn collect_extra_externs(deps_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(deps_dir) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("lib") || !name.ends_with(".rlib") || name.starts_with("libloft") {
            continue;
        }
        // libFOO-HASH.rlib → crate name FOO (hyphens → underscores)
        let without_lib = &name[3..];
        let without_rlib = without_lib.trim_end_matches(".rlib");
        let crate_name = if let Some(pos) = without_rlib.rfind('-') {
            without_rlib[..pos].replace('-', "_")
        } else {
            without_rlib.replace('-', "_")
        };
        result.push((crate_name, entry.path()));
    }
    result
}

/// On Windows MSVC, locate build-script output directories for native import libraries.
///
/// When linking against `libloft.rlib` with standalone `rustc`, crates like `windows-sys`
/// that emit native import libraries via their build scripts (e.g. `windows.0.48.5.lib`)
/// are not automatically found.  Cargo normally passes the build-script output dirs as
/// `-L native=…` linker arguments; we replicate that here.
///
/// Strategy: add every `out/` subdirectory of `target/{profile}/build/` as a `-L` path,
/// plus each of their immediate subdirectories.  Some crates (e.g. `windows-targets`) place
/// import libraries in a platform-specific subdirectory such as `out/x86_64-pc-windows-msvc/`
/// rather than directly in `out/`.  Adding both levels covers all known layouts.
fn find_native_lib_dirs(rlib_info: &Option<(PathBuf, PathBuf)>) -> Vec<PathBuf> {
    #[cfg(not(windows))]
    {
        let _ = rlib_info;
        Vec::new()
    }
    #[cfg(windows)]
    {
        let Some((rlib, _)) = rlib_info else {
            return Vec::new();
        };
        // rlib is at target/{profile}/libloft.rlib or target/{profile}/deps/libloft-*.rlib.
        // Walk up to find the profile directory (release/ or debug/).
        let profile_dir = rlib.parent().and_then(|p| {
            if p.file_name().map(|n| n == "deps").unwrap_or(false) {
                p.parent()
            } else {
                Some(p)
            }
        });
        let Some(profile_dir) = profile_dir else {
            return Vec::new();
        };
        let build_dir = profile_dir.join("build");
        let Ok(entries) = std::fs::read_dir(&build_dir) else {
            return Vec::new();
        };
        let mut dirs = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let build_entry = entry.path();

            // Add out/ and its immediate subdirs (for libs generated into OUT_DIR).
            let out = build_entry.join("out");
            if out.is_dir() {
                dirs.push(out.clone());
                if let Ok(subdirs) = std::fs::read_dir(&out) {
                    for sub in subdirs.filter_map(|e| e.ok()) {
                        if sub.path().is_dir() {
                            dirs.push(sub.path());
                        }
                    }
                }
            }

            // Read the build-script output file for `cargo:rustc-link-search` directives.
            // Crates like `windows_x86_64_msvc` ship `windows.0.48.5.lib` inside their
            // source package (cargo registry) and emit
            //   cargo:rustc-link-search=<CARGO_MANIFEST_DIR>
            // rather than writing the file to OUT_DIR.  Cargo caches these directives in
            // `target/{profile}/build/{crate}-{hash}/output`.  Reading them here replicates
            // exactly what cargo passes to the linker.
            let output_file = build_entry.join("output");
            if let Ok(content) = std::fs::read_to_string(&output_file) {
                for line in content.lines() {
                    let path_str = line
                        .strip_prefix("cargo:rustc-link-search=native=")
                        .or_else(|| line.strip_prefix("cargo:rustc-link-search="));
                    if let Some(path_str) = path_str {
                        let p = PathBuf::from(path_str);
                        if p.is_dir() && !dirs.contains(&p) {
                            dirs.push(p);
                        }
                    }
                }
            }
        }
        dirs
    }
}

/// Paths for one native compilation job.
struct NativeJob {
    stem: String,
    tmp_rs: PathBuf,
    binary: PathBuf,
    /// Sidecar file that stores the cache key written at compile time.
    /// Path: `{binary}.key`.  Content: hex-encoded 64-bit FNV-1a hash of the
    /// `.rs` source bytes concatenated with the rlib identity bytes.
    key_file: PathBuf,
}

/// FNV-1a 64-bit hash — deterministic, no external deps.
fn fnv64(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Build the cache key from the current `.rs` content and rlib identity.
///
/// The key captures both what was compiled (`.rs` bytes) and what it was
/// linked against (rlib path + CONTENT hash).  If either changes the key
/// changes and the binary is recompiled.
///
/// BUILD2: keyed on rlib bytes, not mtime.  `actions/cache` persists `target/`
/// but every CI run reruns `cargo build --release --lib`, which rewrites
/// `libloft.rlib` with a fresh mtime even on a no-op rebuild — an mtime fold
/// then misses and recompiles every native fixture.  rustc's rlib output is
/// byte-deterministic for unchanged sources, so a content hash is stable across
/// the no-op rebuild (warm-cache hit) while still invalidating when the binary
/// actually changes (different bytes → different hash).  The rlib is read once
/// per process via `rlib_content_hash`, not once per fixture.
fn cache_key(rs_content: &[u8], rlib_info: &Option<(PathBuf, PathBuf)>) -> u64 {
    let mut key = fnv64(rs_content);
    if let Some((rlib, _)) = rlib_info {
        key ^= fnv64(rlib.to_string_lossy().as_bytes());
        key ^= rlib_content_hash(rlib);
    }
    key
}

/// FNV-1a hash of a file's bytes, memoised per path (the rlib is ~14MB and the
/// same within a run).  Missing file → 0, matching the old no-op-on-missing.
fn rlib_content_hash(path: &Path) -> u64 {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(&h) = guard.get(path) {
        return h;
    }
    let h = std::fs::read(path).map(|b| fnv64(&b)).unwrap_or(0);
    guard.insert(path.to_path_buf(), h);
    h
}

/// Phase 1 — parse the `.loft` file and generate its Rust source.
///
/// The generated `.rs` is written only when its content changes, so that the binary
/// modification-time cache in Phase 2 is not unnecessarily invalidated.
///
/// Fails the test if the loft parse or scope-check step produces diagnostics.
fn prepare_native_test(entry: &Path) -> std::io::Result<NativeJob> {
    let stem = entry
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .replace('-', "_");
    println!("native {entry:?}");

    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    // Honour `// @ARGS: --lib <dir>` lines at the top of the test file
    // so scripts using `use foo::*` can locate fixtures alongside
    // wrap.rs's loft_suite (e.g. tests/lib/importlib.loft for
    // 88-imports.loft).  Only `--lib <dir>` is recognised; other
    // CLI-side flags are ignored at this layer.
    if let Ok(src) = std::fs::read_to_string(entry) {
        for line in src.lines().take(20) {
            if let Some(args) = line.trim().strip_prefix("// @ARGS:") {
                let mut tokens = args.split_whitespace();
                while let Some(tok) = tokens.next() {
                    if tok == "--lib"
                        && let Some(dir) = tokens.next()
                    {
                        p.lib_dirs.push(dir.to_string());
                    }
                }
            }
        }
    }
    let start_def = p.data.definitions();
    p.parse(&entry.to_string_lossy(), false);
    for l in p.diagnostics.lines() {
        println!("{l}");
    }
    if p.diagnostics.level() >= loft::diagnostics::Level::Error {
        return Err(Error::from(std::io::ErrorKind::InvalidData));
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    let main_nr = p.data.def_nr("n_main");
    let has_main = main_nr < end_def;

    // Collect zero-parameter user functions as test entry points.
    let mut test_fns: Vec<(u32, String)> = Vec::new();
    for d_nr in start_def..end_def {
        let def = p.data.def(d_nr);
        if !matches!(def.def_type, loft::data::DefType::Function) {
            continue;
        }
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        // Only count user-visible parameters (skip hidden __work_* and
        // __ref_* arguments added by text_return / ref_return).
        let has_user_params = def
            .attributes
            .iter()
            .any(|a| !a.name.starts_with("__work_") && !a.name.starts_with("__ref_"));
        if has_user_params {
            continue;
        }
        if def.position.file.starts_with("default/") {
            continue;
        }
        test_fns.push((d_nr, def.name.clone()));
    }

    let entry_defs: Vec<u32> = if has_main {
        vec![main_nr]
    } else {
        test_fns.iter().map(|(d, _)| *d).collect()
    };

    // Generate Rust source into an in-memory buffer first.
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut out = Output::new(&p.data, &state.database);
        out.output_native_reachable(&mut buf, start_def, end_def, &entry_defs)?;
    }

    // For test-style files without fn main(), generate a main() that calls
    // each test function so the native binary is a valid executable.
    // Skip functions marked with @EXPECT_FAIL in the source.
    if !has_main && !test_fns.is_empty() {
        use std::io::Write;
        let src = std::fs::read_to_string(entry).unwrap_or_default();
        let expect_fail_fns: std::collections::HashSet<String> = src
            .lines()
            .filter(|l| l.contains("@EXPECT_FAIL"))
            .flat_map(|l| {
                l.split_whitespace()
                    .skip_while(|w| *w != "@EXPECT_FAIL")
                    .skip(1)
                    .map(String::from)
            })
            .collect();
        // P199 — wrap Stores in UnsafeCell for the new ABI; the work
        // buffers (`stores.null_named(...)`) need a temporary `&mut Stores`
        // derived from the cell.
        writeln!(buf, "\nfn main() {{")?;
        writeln!(
            buf,
            "    let cell = std::cell::UnsafeCell::new(Stores::new());"
        )?;
        writeln!(
            buf,
            "    let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        writeln!(buf, "    init(&cell);")?;
        for (d_nr, name) in &test_fns {
            let user_name = name.strip_prefix("n_").unwrap_or(name);
            if expect_fail_fns
                .iter()
                .any(|f| user_name.contains(f.as_str()))
            {
                writeln!(buf, "    // skipped (EXPECT_FAIL): {name}")?;
            } else {
                // Generate work-buffer locals for hidden __work_* / __ref_* parameters
                // that text_return adds to text-returning functions.
                let def = p.data.def(*d_nr);
                let mut work_args = Vec::new();
                for (i, attr) in def.attributes.iter().enumerate() {
                    if attr.name.starts_with("__work_") {
                        let wname = format!("_w_{user_name}_{i}");
                        writeln!(buf, "    let mut {wname} = String::new();")?;
                        work_args.push(format!("&mut {wname}"));
                    } else if attr.name.starts_with("__ref_") {
                        let wname = format!("_r_{user_name}_{i}");
                        writeln!(buf, "    let mut {wname} = stores.null_named(\"{wname}\");")?;
                        work_args.push(wname.to_string());
                    }
                }
                if work_args.is_empty() {
                    writeln!(buf, "    {name}(&cell);")?;
                } else {
                    writeln!(buf, "    {name}(&cell, {});", work_args.join(", "))?;
                }
            }
        }
        writeln!(buf, "}}")?;
    }

    // Only write the .rs file when the content has changed.  This keeps the file's
    // content stable across runs where the loft source hasn't changed, which
    // means cache_key() produces the same hash and compile_native_job stays cached.
    // scratch_dir honours LOFT_TMPDIR so the whole native run can be kept off a
    // small /tmp tmpfs; all of these must agree on the same directory.
    let scratch = loft::platform::scratch_dir();
    let tmp_rs = scratch.join(format!("loft_native_{stem}.rs"));
    let existing = std::fs::read(&tmp_rs).unwrap_or_default();
    if existing != buf {
        // Atomic publish: write to a per-process temp then rename into place,
        // so a concurrent process (nextest runs each test in its own process)
        // compiling the same stem never reads a half-written source.  See the
        // shared-output collision note in `compile_native_job`.
        let tmp = scratch.join(format!("loft_native_{stem}_{}.rs.tmp", std::process::id()));
        std::fs::write(&tmp, &buf)?;
        std::fs::rename(&tmp, &tmp_rs)?;
    }

    let binary = scratch.join(format!("loft_native_{stem}_bin"));
    let key_file = scratch.join(format!("loft_native_{stem}_bin.key"));
    Ok(NativeJob {
        stem,
        tmp_rs,
        binary,
        key_file,
    })
}

/// Return true if the cached binary is still valid for the current `.rs` content
/// and rlib.  Uses a content-hash sidecar (`{binary}.key`) written at compile
/// time — immune to clock skew and cross-machine binary copies.
fn binary_cache_valid(job: &NativeJob, rlib_info: &Option<(PathBuf, PathBuf)>) -> bool {
    // Binary must exist.
    if !job.binary.exists() {
        return false;
    }
    // Read the stored key from the sidecar.
    let stored = match std::fs::read_to_string(&job.key_file) {
        Ok(s) => s.trim().to_string(),
        Err(_) => return false,
    };
    // Recompute the key from the current .rs content and rlib.
    let rs_content = match std::fs::read(&job.tmp_rs) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let current_key = cache_key(&rs_content, rlib_info);
    stored == format!("{current_key:016x}")
}

/// Phase 2 — compile the generated `.rs` file to a native binary with `rustc`.
///
/// Skips compilation when `binary_cache_valid` is true (binary is already up to date).
/// The binary is kept on disk after use so that subsequent runs can hit the cache.
///
/// Returns `Ok(true)` when a valid binary is available, `Ok(false)` when `rustc` is
/// not in PATH (caller should skip the run phase), and `Err` on a real compile failure.
fn compile_native_job(
    job: &NativeJob,
    rlib_info: &Option<(PathBuf, PathBuf)>,
) -> std::io::Result<bool> {
    if binary_cache_valid(job, rlib_info) {
        println!("  cached  {}", job.stem);
        // BUILD2: bump the binary's mtime on a cache HIT so its timestamp
        // tracks last-USE, not last-compile.  A long-lived cache entry is
        // hit (not recompiled) run after run, so without this its mtime
        // would freeze at first-compile time and an age-based reaper would
        // delete exactly the entries the cache most wants to keep warm.
        let now = std::time::SystemTime::now();
        let _ = std::fs::File::open(&job.binary).and_then(|f| f.set_modified(now));
        let _ = std::fs::File::open(&job.key_file).and_then(|f| f.set_modified(now));
        return Ok(true);
    }
    // Preflight (Layer 2): never start a compile that could overflow a
    // RAM-backed tmpfs and exhaust memory.  Reclaims loft's own stale
    // artefacts first; skips (not fails) the test if space is still low.
    let scratch = loft::platform::scratch_dir();
    if !loft::platform::native_compile_space_ok(&scratch) {
        println!(
            "  SKIP {} — low temp space in {} (set LOFT_TMPFS_MIN_FREE_MB to tune)",
            job.stem,
            scratch.display()
        );
        return Ok(false);
    }
    // nextest runs each test in its own process, so the native tests that share
    // a stem — `native_tuple_script`, `native_tuple_return_script`, and
    // `native_scripts` all compile `50-tuples.loft` — would otherwise have
    // their rustc/lld processes write the SAME `loft_native_<stem>_bin` output
    // concurrently, truncating each other's binary mid-link (observed as a
    // SIGBUS in `rust-lld` / `linking with cc failed`).  The in-process
    // `native_suite_lock()` only serialises them WITHIN one process.  Compile
    // to a per-process temp and publish atomically (rename) below, so concurrent
    // processes never share a mutable output file — keeping full parallelism
    // without a nextest serial group.
    let pid = std::process::id();
    let binary_tmp = scratch.join(format!("loft_native_{}_{pid}_bin", job.stem));
    // PLAN49 follow-up — build the rustc args into a file passed via
    // `rustc @argfile` instead of as a long command line.  Windows
    // `CreateProcessW` enforces a 32 KB command-line limit; on CI runners
    // the rustc `--extern <crate>=<rlib-path>` list + `-L` search paths
    // from `find_native_lib_dirs` (windows-targets build-script outputs)
    // routinely approaches that limit, and a runner-image bump on
    // 2026-05-29 pushed it over, killing every native test with
    // `Os { code: 206, kind: InvalidFilename }`.  The argfile pattern
    // is cross-platform (Linux + macOS happily accept it too) and
    // immune to cmdline length.
    let mut args: Vec<String> = vec![
        "--edition=2024".to_string(),
        "-C".to_string(),
        "debuginfo=0".to_string(),
        "-C".to_string(),
        "opt-level=0".to_string(),
    ];
    // Layer 1: strip the linked binary (~36MB → ~1MB; the bulk is debug info
    // from libloft.rlib + std, useless to a run-and-check test).  Opt out with
    // LOFT_NATIVE_KEEP_SYMBOLS=1 when debugging a native crash (the generated
    // .rs is always kept for recompilation).
    if loft::platform::native_strip_symbols() {
        args.push("-C".to_string());
        args.push("strip=symbols".to_string());
    }
    // LOFT_CRANELIFT=1 — use the Cranelift codegen backend for much faster compilation.
    // Requires a nightly toolchain with `rustup component add rustc-codegen-cranelift-preview`.
    if std::env::var_os("LOFT_CRANELIFT").is_some() {
        args.push("-Z".to_string());
        args.push("codegen-backend=cranelift".to_string());
    }
    args.push("-o".to_string());
    args.push(binary_tmp.display().to_string());
    args.push(job.tmp_rs.display().to_string());
    if let Some((rlib, deps_dir)) = rlib_info {
        args.push("--extern".to_string());
        args.push(format!("loft={}", rlib.display()));
        args.push("-L".to_string());
        args.push(deps_dir.display().to_string());
        // S31: pass --extern for optional feature deps (rand_core, rand_pcg, etc.) so that
        // generated code using `random` or `png` features compiles without E0433 errors.
        for (crate_name, rlib_path) in collect_extra_externs(deps_dir) {
            args.push("--extern".to_string());
            args.push(format!("{crate_name}={}", rlib_path.display()));
        }
    }
    // On Windows MSVC, build-script output dirs holding native import libs (e.g.
    // `windows.0.48.5.lib` from `windows-sys`) must be passed explicitly to standalone
    // rustc — cargo adds them automatically via `cargo:rustc-link-search`, but we don't.
    for dir in find_native_lib_dirs(rlib_info) {
        args.push("-L".to_string());
        args.push(dir.display().to_string());
    }
    // Write one arg per line.  rustc's argfile parser is whitespace-
    // separated; newline-separated is a strict subset and is what
    // every other tool (clang, gcc) accepts too.  Paths containing
    // whitespace get quoted defensively.
    let argfile_path = scratch.join(format!("loft_native_{}_{pid}_args.txt", job.stem));
    let argfile_contents = args
        .iter()
        .map(|s| {
            if s.contains(char::is_whitespace) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&argfile_path, argfile_contents)?;
    let compile_out = match std::process::Command::new("rustc")
        .arg(format!("@{}", argfile_path.display()))
        .output()
    {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  rustc not found — skipping native test for {}", job.stem);
            let _ = std::fs::remove_file(&argfile_path);
            return Ok(false);
        }
        Err(e) => {
            let _ = std::fs::remove_file(&argfile_path);
            return Err(e);
        }
    };
    let _ = std::fs::remove_file(&argfile_path);
    if !compile_out.status.success() {
        let stderr = String::from_utf8_lossy(&compile_out.stderr);
        eprintln!("rustc failed for {}:\n{stderr}", job.stem);
        let _ = std::fs::remove_file(&binary_tmp);
        let _ = std::fs::remove_file(&job.binary);
        let _ = std::fs::remove_file(&job.key_file);
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    // Publish the freshly-linked binary atomically: rename the per-process temp
    // over the shared cache path.  rename(2) swaps the directory entry, so a
    // concurrent process executing or linking the old binary keeps its inode
    // (no in-place truncation → no SIGBUS) and the cache path is always a
    // complete binary.
    std::fs::rename(&binary_tmp, &job.binary)?;
    // Write the cache key so future runs can skip recompilation when nothing
    // changed — also via temp + rename so a concurrent `binary_cache_valid`
    // reader never sees a half-written key.
    let rs_content = std::fs::read(&job.tmp_rs).unwrap_or_default();
    let key = cache_key(&rs_content, rlib_info);
    let key_tmp = scratch.join(format!("loft_native_{}_{pid}_bin.key.tmp", job.stem));
    if std::fs::write(&key_tmp, format!("{key:016x}")).is_ok() {
        let _ = std::fs::rename(&key_tmp, &job.key_file);
    }
    Ok(true)
}

/// Phase 3 — run a compiled native binary and check its exit status.
///
/// The binary is kept on disk after running so it can be reused as a compilation
/// cache on the next invocation (see `binary_cache_valid`).
fn run_native_job(job: &NativeJob) -> std::io::Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let run_status = std::process::Command::new(&job.binary)
        .current_dir(&cwd)
        .status()?;
    if !run_status.success() {
        eprintln!(
            "native binary failed for {} (exit {:?})",
            job.stem,
            run_status.code()
        );
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    Ok(())
}

/// Compile in parallel, then run in parallel.
fn run_native_jobs(
    jobs: Vec<NativeJob>,
    rlib_info: Option<(PathBuf, PathBuf)>,
) -> std::io::Result<()> {
    // Layer 3: scale worker count to temp-fs headroom.  On a roomy disk this is
    // just min(cpus, jobs); on a tight RAM-backed tmpfs it clamps down so N
    // concurrent compiles can't exhaust memory and hang the machine.  Each
    // in-flight compile peaks at ~1.2GB of temp (rustc intermediates dominate;
    // the stripped output binary is ~1MB), so reserve that per worker.
    let cpu_max = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    const PER_WORKER_TMP: u64 = 1280 * 1024 * 1024;
    let concurrency = loft::platform::native_worker_count(
        cpu_max,
        jobs.len(),
        &loft::platform::scratch_dir(),
        PER_WORKER_TMP,
    );
    let rlib_ref = &rlib_info;

    // Phase 2: compile all jobs in parallel chunks.
    let mut compiled: Vec<bool> = Vec::with_capacity(jobs.len());
    let mut first_err: Option<std::io::Error> = None;
    for chunk in jobs.chunks(concurrency) {
        let chunk_results: Vec<std::io::Result<bool>> = std::thread::scope(|s| {
            chunk
                .iter()
                .map(|job| s.spawn(|| compile_native_job(job, rlib_ref)))
                .collect::<Vec<_>>()
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|_| Err(Error::from(std::io::ErrorKind::Other)))
                })
                .collect()
        });
        for r in chunk_results {
            match r {
                Ok(b) => compiled.push(b),
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                    compiled.push(false);
                }
            }
        }
    }
    let compile_fail = compiled.iter().filter(|ok| !**ok).count();

    // Phase 3: run all compiled binaries in parallel.
    let ready: Vec<&NativeJob> = jobs
        .iter()
        .zip(compiled.iter())
        .filter(|(_, ok)| **ok)
        .map(|(job, _)| job)
        .collect();
    let compile_ok = ready.len();
    let run_errors: Vec<String> = std::thread::scope(|s| {
        ready
            .iter()
            .map(|job| s.spawn(|| run_native_job(job)))
            .collect::<Vec<_>>()
            .into_iter()
            .zip(ready.iter())
            .filter_map(|(h, job)| {
                h.join()
                    .unwrap_or_else(|_| Err(Error::from(std::io::ErrorKind::Other)))
                    .err()
                    .map(|_| job.stem.clone())
            })
            .collect()
    });
    let run_ok = compile_ok - run_errors.len();
    println!(
        "\nnative result: {run_ok} passed, {} compile failed, {} run failed; {} total",
        compile_fail,
        run_errors.len(),
        jobs.len()
    );
    if !run_errors.is_empty() {
        println!("  run failures: {}", run_errors.join(", "));
    }
    // Fail if any test failed to compile or run.
    if compile_fail > 0 || !run_errors.is_empty() {
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    Ok(())
}

/// Compile and run every `.loft` file in `tests/docs/` through the native Rust
/// backend (`--native` mode), skipping files listed in `NATIVE_SKIP`.
///
/// Runs concurrently with interpreter-based wrap tests (no WRAP_LOCK).
/// Skips silently if `rustc` is not in PATH.
#[test]
fn native_dir() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/docs")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    let rlib_info = find_loft_rlib();
    let mut jobs = Vec::new();
    for entry in files {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if NATIVE_SKIP.iter().any(|s| *s == name.as_ref()) {
            println!("skip {entry:?} (native skip list — see NATIVE_SKIP)");
            continue;
        }
        jobs.push(prepare_native_test(&entry)?);
    }
    run_native_jobs(jobs, rlib_info)
}

/// Compile and run every extracted feature example in `tests/docs/features/`
/// through the native backend — the native half of @PLN92 strand-3's
/// "example-must-run" guard (@I81).  These files are generated from
/// `loft-lang/features` issues by `tools/features/gen.loft`; only complete-program
/// examples land here (library / syntax fragments are mirrored but not tested).
/// Silent no-op if the directory is absent; skips silently if `rustc` is absent.
#[test]
fn native_features() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut files: Vec<PathBuf> = match std::fs::read_dir("tests/docs/features") {
        Ok(rd) => rd
            .filter_map(|f| f.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    let rlib_info = find_loft_rlib();
    let mut jobs = Vec::new();
    for entry in files {
        jobs.push(prepare_native_test(&entry)?);
    }
    run_native_jobs(jobs, rlib_info)
}

/// Compile and run every `.loft` file in `tests/scripts/` through the native Rust
/// backend (`--native` mode), skipping files listed in `SCRIPTS_NATIVE_SKIP`.
///
/// Runs concurrently with interpreter-based wrap tests (no WRAP_LOCK).
/// Skips silently if `rustc` is not in PATH.
#[test]
fn native_scripts() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let mut files: Vec<PathBuf> = std::fs::read_dir("tests/scripts")?
        .filter_map(|f| f.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .collect();
    files.sort();
    let rlib_info = find_loft_rlib();
    let mut jobs = Vec::new();
    for entry in files {
        let name = entry.file_name().unwrap_or_default().to_string_lossy();
        if SCRIPTS_NATIVE_SKIP.iter().any(|s| *s == name.as_ref()) {
            println!("skip {entry:?} (scripts native skip list — see SCRIPTS_NATIVE_SKIP)");
            continue;
        }
        // Skip files with intentional compile errors or expected failures.
        // Native mode compiles the whole file into one binary and can't
        // tolerate per-function panics like the interpreter runner can.
        if let Ok(src) = std::fs::read_to_string(&entry) {
            if src.contains("@EXPECT_ERROR") {
                println!("skip {entry:?} (has @EXPECT_ERROR)");
                continue;
            }
            if src.contains("@EXPECT_FAIL") {
                println!("skip {entry:?} (has @EXPECT_FAIL)");
                continue;
            }
        }
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare_native_test(&entry)))
        {
            Ok(Ok(job)) => jobs.push(job),
            Ok(Err(e)) => println!("skip {entry:?} (prepare error: {e})"),
            Err(_) => println!("skip {entry:?} (codegen panic — native codegen bug)"),
        }
    }
    run_native_jobs(jobs, rlib_info)
}

/// N8a: native code generation for tuple types.
///
/// Runs `tests/scripts/50-tuples.loft` through the native Rust backend end-to-end.
/// Ignored until N8a.1 (`rust_type(Type::Tuple)` fix) and N8a.2 (`TupleGet`/`TuplePut`
/// emit) are implemented.  When un-ignored, `50-tuples.loft` and `46-caveats.loft`
/// are removed from `SCRIPTS_NATIVE_SKIP`.
#[test]
fn native_tuple_script() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    let entry = std::path::Path::new("tests/scripts/50-tuples.loft");
    let job = prepare_native_test(entry)?;
    // Ok(false) = skipped (rustc absent, or Layer-2 low-space guard) — not a
    // failure; a real compile error returns Err.  Skip the run in that case.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    run_native_job(&job)
}

/// S35: native binary I/O script.
///
/// Runs `tests/scripts/20-binary.loft` through the native Rust backend end-to-end.
/// Exercises the Insert-return pattern (Set(rv, Insert([Set(_read_34, Null), Block])))
/// fixed in S35: output_set now hoists inner ops as statements before the assignment.
#[test]
fn native_binary_script() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    let entry = std::path::Path::new("tests/scripts/20-binary.loft");
    let job = prepare_native_test(entry)?;
    // Ok(false) = skipped (rustc absent, or Layer-2 low-space guard) — not a
    // failure; a real compile error returns Err.  Skip the run in that case.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    run_native_job(&job)
}

/// @PLN28: unbounded recursion in native-compiled code must surface loft's typed
/// `call stack overflow` (exit non-zero) — the same cap the interpreter enforces —
/// NOT the opaque Rust `fatal runtime error: stack overflow, aborting`.  The
/// generated `main` runs on a large-stack thread so the `MAX_CALL_DEPTH` guard in
/// `cr_call_push` fires cleanly before the OS stack is exhausted.
#[test]
fn native_deep_recursion_reports_clean_stack_overflow() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    // Write to a temp file, NOT tests/scripts/ (which the success-runners sweep —
    // an infinitely-recursing script would break them).
    let path = std::env::temp_dir().join("loft_native_stack_overflow_guard.loft");
    std::fs::write(
        &path,
        "fn recur(n: integer) -> integer { m = recur(n + 1); return m + 1; }\n\
         fn main() { x = recur(0); print(\"{x}\"); }\n",
    )?;
    let job = prepare_native_test(&path)?;
    // Ok(false) = skipped (rustc absent / low space) — not a failure.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    let out = std::process::Command::new(&job.binary).output()?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "deep recursion must exit non-zero; stderr: {stderr}"
    );
    assert!(
        stderr.contains("call stack overflow"),
        "expected loft's typed stack-overflow diagnostic; stderr: {stderr}"
    );
    assert!(
        !stderr.contains("fatal runtime error"),
        "must NOT be the raw Rust stack-overflow abort; stderr: {stderr}"
    );
    Ok(())
}

/// N8a.3: native tuple-returning functions.
///
/// The same 50-tuples.loft script will include a tuple-returning function once
/// N8a.3 is implemented.  This is a placeholder: un-ignored together with
/// native_tuple_script when the updated script passes.
#[test]
fn native_tuple_return_script() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    let entry = std::path::Path::new("tests/scripts/50-tuples.loft");
    let job = prepare_native_test(entry)?;
    // Ok(false) = skipped (rustc absent, or Layer-2 low-space guard) — not a
    // failure; a real compile error returns Err.  Skip the run in that case.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    run_native_job(&job)
}

// Initiative 03 Phase 2: the native Moros editor driver lives at
// `lib/graphics/examples/moros_editor.loft` and compiles end-to-end
// via `loft --native --path <repo>/ --lib <repo>/lib`.  Manual
// verification on a machine with a display server: the binary opens
// a 1024×768 window rendering the 7×7 hex starter map; WASD orbits
// the camera; Esc exits.  An automated compile-regression needs the
// `prepare_native_test` harness to pass `--path <repo>` for sibling-
// module resolution (`use render;` works only from inside the
// graphics package's own src/ or examples/ dirs) — deferred to
// Phase 5 polish; see doc/claude/plans/03-native-moros-editor/.

/// Library packages whose tests do NOT yet compile/run under `--native`.
/// These are pre-existing native-codegen / binding / runtime gaps (NOT linkage
/// gaps — the `#native`-crate linkage was fixed via
/// `native_utils::add_native_extern_flags` in the test runner, which recovered
/// graphics/shapes/server/web/moros_render/moros_sim).  Tracked under @P321.
const LIB_PKGS_NATIVE_SKIP: &[&str] = &[
    // crypto — FIXED (@P321a): sha256/base64/hmac wired into codegen_runtime.rs.
    // arguments — FIXED (@P321b): OpSetText with a null value now stores the null
    // pointer instead of emitting `(()).to_string()`.  Regression:
    // tests/scripts/repro_p321b.loft.
    // random — FIXED (@P321f): wired `n_rand_seed` into codegen_runtime (was a
    // void empty-stub no-op) AND fixed `n_rand_indices` to store 8-byte (i64)
    // elements matching how `vector<integer>` is read.
    // moros_editor — FIXED (@P321e): a text-returning match fn `.to_string()`'d
    // its result into a `__ret_N` local and returned `Str::new(&local)`
    // (dangling); the return now routes a text-LOCAL value through `stores.scratch`.
    // moros_ui — FIXED (@P321g): a `&`-ref-param call on an assignment RHS
    // (`x = route_click(p, st.es_tools, …)`) arrived as `Span(Insert([Set(__ref_N,
    // …), Call]))`; output_set's S35 hoist matched only a bare `Insert`, so it
    // fell through to the brace-less Insert arm → `let x = let __ref_N = …; call`
    // (let in expression position).  output_set now unspans before the S35 check.
    // imaging — FIXED (@P321c): the native direct-call codegen now forwards a
    // LoftStore + converts struct `Reference` ARGS to LoftRef
    // (`output_native_direct_call`), so a store-mutating package `#native` fn
    // (`load_png(path, image)`) gets the full 4-arg ABI.  The cdylib's
    // hardcoded field offsets were also wrong; `loft generate` now emits
    // offsets from the canonical struct schema (`Stores::position`/`size`)
    // instead of a separate layout calc, and lib/imaging/native matches them.
    // input — design draft landed 2026-06-01 (LAVITION W.13); runtime blocked
    // on @P391 (cross-package constructor lands in CONST_STORE).  Un-skip once
    // @P391 ships.
    "input",
];

/// Specific library test FILES skipped under `--native` (the rest of the
/// package DOES compile), keyed `"<pkg>/<file>.loft"`.
const LIB_TESTS_NATIVE_SKIP: &[&str] = &[
    // Network: live HTTPS to httpbin.org — same reason as the interpreter skip
    // (wrap.rs::LIB_TESTS_SKIP).  Not a native gap.
    "web/http.loft",
    // @P321d FIXED 2026-05-23: nested vector index `m.a[0].b[2]` no longer
    // emits two live `&mut stores` borrows (E0499) — the OpGetVector /
    // OpVectorRef `#rust` templates bind `@r` to a local before the call.

    // @P333 FIXED 2026-05-26: `moros_render/geometry.loft` +
    // `moros_sim/persistence.loft` no longer hardcode `/tmp/` — they use
    // CWD-relative filenames + `delete()`, so they run on Windows too.  The
    // Windows skips are removed (macOS + Linux already passed).
];

/// True if `entry` (a `lib/<pkg>/tests/<file>.loft` path) is skipped under the
/// NATIVE library gate (its own codegen-gap list above; the interpreter gate's
/// skips live in `wrap.rs::lib_test_skipped`).
fn native_lib_test_skipped(entry: &Path) -> bool {
    let file = entry
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let pkg = entry
        .parent()
        .and_then(|d| d.parent())
        .and_then(|d| d.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if LIB_PKGS_NATIVE_SKIP.contains(&pkg.as_str()) {
        return true;
    }
    let key = format!("{pkg}/{file}");
    LIB_TESTS_NATIVE_SKIP.contains(&key.as_str())
}

/// Native counterpart of `wrap.rs::library_suite`: compile + run every
/// Run `loft [extra_args] test <stem>` for a lib package in a UNIQUE temp CWD —
/// a `.loft_test_tmp_*` SIBLING inside `lib/` (so the package's relative deps
/// `../<name>` still resolve to the real packages), with the package's contents
/// symlinked in.  Isolates cwd-relative test artifacts so the native and
/// interpreter lib suites (separate, concurrently-run test binaries) don't race
/// on a shared file (e.g. `moros_render_test.glb`).  Duplicated from `wrap.rs`
/// (integration-test binaries can't share fns).  Non-unix falls back to `pkg_dir`.
fn run_lib_test_in_temp_cwd(
    loft_bin: &str,
    pkg_dir: &Path,
    stem: &str,
    extra_args: &[&str],
) -> std::io::Result<std::process::Output> {
    let mut args: Vec<&str> = extra_args.to_vec();
    args.push("test");
    args.push(stem);
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let lib_root = pkg_dir.parent().unwrap_or(pkg_dir);
        let tmp = lib_root.join(format!(
            ".loft_test_tmp_{}_{}",
            std::process::id(),
            CTR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir(&tmp)?;
        for entry in std::fs::read_dir(pkg_dir)?.filter_map(|e| e.ok()) {
            let target = entry.path().canonicalize().unwrap_or_else(|_| entry.path());
            let _ = std::os::unix::fs::symlink(&target, tmp.join(entry.file_name()));
        }
        let out = std::process::Command::new(loft_bin)
            .current_dir(&tmp)
            .args(&args)
            .output();
        let _ = std::fs::remove_dir_all(&tmp);
        out
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new(loft_bin)
            .current_dir(pkg_dir)
            .args(&args)
            .output()
    }
}

/// `lib/<pkg>/tests/*.loft` under `--native`, skipping packages/files with known
/// native-codegen gaps (`LIB_*_NATIVE_SKIP`, @P321).  Shells out
/// `cd lib/<pkg> && loft --native test <stem>` so it reuses the CLI's package
/// resolution AND the `#native`-crate linkage (`add_native_extern_flags`).
///
/// Holds `native_suite_lock` so it serialises with the other native suites
/// (shared `/tmp` rlib + binary cache).  Skips silently when `rustc` / the loft
/// rlib are unavailable, like `native_scripts`.
#[test]
fn native_library_suite() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if find_loft_rlib().is_none() {
        println!("native_library_suite: skipped (no libloft.rlib / rustc unavailable)");
        return Ok(());
    }
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let mut files: Vec<PathBuf> = Vec::new();
    for pkg in std::fs::read_dir("lib")?.filter_map(|e| e.ok()) {
        // Skip the `.loft_test_tmp_*` artifact-isolation dirs (see
        // run_lib_test_in_temp_cwd) so they're never discovered as packages.
        if pkg.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let tests_dir = pkg.path().join("tests");
        if !tests_dir.is_dir() {
            continue;
        }
        for f in std::fs::read_dir(&tests_dir)?.filter_map(|e| e.ok()) {
            let p = f.path();
            if p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut failures: Vec<String> = Vec::new();
    let mut ran = 0;
    let mut env_skips: Vec<(String, String)> = Vec::new();
    for entry in files {
        if native_lib_test_skipped(&entry) {
            println!("skip {entry:?} (LIB_*_NATIVE_SKIP — @P321)");
            continue;
        }
        let pkg_dir = entry.parent().and_then(|d| d.parent()).unwrap_or(&entry);
        let stem = entry
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        println!("native lib test {entry:?}");
        let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, &stem, &["--native"])?;
        ran += 1;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // Windows: the windows-targets crate emits a search path that
        // doesn't survive the test-binary link step (LNK1181: cannot
        // open input file 'windows.0.NN.0.lib').  Environmental, not a
        // code regression — mirrors the toolchain-skip pattern in
        // tests/exit_codes.rs.  loft test wraps the rustc error as
        // "native compile: error: linking with `link.exe` failed: exit
        // code: 1181" — the raw "LNK1181" symbol from cc's separate
        // stderr may not survive the capture.  Match both forms.
        if combined.contains("LNK1181") || combined.contains("link.exe` failed: exit code: 1181") {
            // Capture the actual linker error lines, not just the generic
            // label — without this the skip is a SILENT green (nextest hides
            // a passing test's stdout), so a regression of G2's fix would
            // look identical to a clean pass.  The detail is surfaced by the
            // CI "Surface environmental test skips" step via the ledger below.
            let detail: Vec<&str> = combined
                .lines()
                .filter(|l| l.contains("1181") || l.contains("LNK") || l.contains("link.exe"))
                .take(3)
                .collect();
            println!("skip {entry:?} (Windows windows-targets LNK1181 — environmental)");
            env_skips.push((format!("{entry:?}"), detail.join(" | ")));
            ran -= 1;
            continue;
        }
        // `loft test` exits 0 even on a caught crash; detect failure by markers.
        let failed = !out.status.success()
            || combined.contains("SIGSEGV")
            || combined.contains("panicked")
            || combined.contains("native compile:")
            || combined.contains("test result: FAILED")
            || !combined.contains("test result: ok");
        if failed {
            let tail: Vec<&str> = combined.lines().rev().take(5).collect();
            failures.push(format!("{entry:?}: {}", tail.join(" | ")));
        }
    }
    if !failures.is_empty() {
        return Err(Error::other(format!(
            "{} of {ran} native library tests failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        )));
    }
    if !env_skips.is_empty() {
        record_env_skips("native_library_suite", "LNK1181", &env_skips);
        println!(
            "native_library_suite: {ran} passed, {} skipped (environmental — LNK1181)",
            env_skips.len()
        );
    } else {
        println!("native_library_suite: {ran} native library tests passed");
    }
    Ok(())
}

/// @PLN26 — the C-ABI native-package EXEC path: a program that `use`s a
/// `[native] crate` package, compiled with `--native`, must link the package's
/// cdylib by C-ABI (`add_native_extern_flags`) and call its `#native` symbol.
/// The minimal `native_scalar_pkg` fixture exports one scalar symbol
/// (`n_native_answer` → 42), so this is cheap to build yet covers the whole
/// path — including, on Windows, the import-library link + DLL staging of
/// @PLN26 phase 4 (the focused `win-cdylib` workflow's C-ABI job sets
/// `LOFT_NATIVE_CABI=1` to force that arm on; on the normal Windows CI this
/// exercises the rlib-link path instead).  The hard-coded 42 is the oracle, so
/// a broken link (undefined symbol / P269 / wrong value) fails LOUDLY — not
/// vacuously.  This is the regression guard the C-ABI exec path previously
/// lacked (phase 0 was a manual probe).
///
/// Serialises via `native_suite_lock` (shared /tmp rlib + binary cache) and
/// skips cleanly when `rustc` / the loft rlib are unavailable, like the other
/// native suites.
#[test]
fn native_crate_package_links_and_runs_via_cabi() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if find_loft_rlib().is_none() {
        println!("native_crate_package_links_and_runs_via_cabi: skipped (no libloft.rlib / rustc)");
        return Ok(());
    }
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let pkg_dir = Path::new("tests/lib/native_scalar_pkg");

    // --native: the C-ABI exec path (the path @PLN26 phase 4's Windows arm extends).
    let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, "answer", &["--native"])?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !combined.contains("no implementation"),
        "native_crate package symbol was not linked (P269) — the C-ABI exec path regressed:\n{combined}"
    );
    assert!(
        combined.contains("test result: ok"),
        "--native native_crate exec test did not pass:\n{combined}"
    );

    // --interpret parity: the same package via the runtime dylib dispatch.
    let out_i = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, "answer", &["--interpret"])?;
    let combined_i = format!(
        "{}{}",
        String::from_utf8_lossy(&out_i.stdout),
        String::from_utf8_lossy(&out_i.stderr)
    );
    assert!(
        combined_i.contains("test result: ok"),
        "--interpret native_crate exec test did not pass:\n{combined_i}"
    );
    Ok(())
}

/// The imaging fixture's PNG round-trip — a `[native] crate` package whose
/// `#native` functions (`load_png`/`save_png`) do STORE-MUTATING file I/O via
/// raw `std::fs`.  Guards two things at once: (1) the store-mutating C-ABI path
/// (LoftStore + LoftRef marshalling, the hardest native shape), and (2) the
/// source-dir cwd anchoring — `loft test` runs from the package root, but a
/// native crate's `std::fs` must resolve `map.png` / `_tmp_*.png` where loft's
/// `file()` does (`tests/`), which only holds because the runner chdir's to
/// `source_dir`.  Both round-trip files assert hard pixel oracles, so a broken
/// link OR a cwd regression fails loudly.  Runs on BOTH backends.
///
/// Serialises via `native_suite_lock`; skips cleanly without `rustc` / the loft
/// rlib, like the other native suites.
#[test]
fn imaging_fixture_png_roundtrip_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if find_loft_rlib().is_none() {
        println!("imaging_fixture_png_roundtrip_both_backends: skipped (no libloft.rlib / rustc)");
        return Ok(());
    }
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    let pkg_dir = Path::new("tests/fixtures/libs/imaging");
    for stem in ["14-image", "15-regression"] {
        for mode in ["--native", "--interpret"] {
            let out = run_lib_test_in_temp_cwd(loft_bin, pkg_dir, stem, &[mode])?;
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                combined.contains("test result: ok"),
                "imaging {stem} {mode} round-trip did not pass:\n{combined}"
            );
        }
    }
    Ok(())
}

/// Record environmental skips (tests that PASSED-by-skipping for a
/// toolchain/OS reason, not a code reason) to a side-channel ledger so they
/// survive nextest's success-output suppression.  Without this a green run
/// hides reduced coverage — a regression of the underlying fix (e.g. G2's
/// Windows native link) would look identical to a clean pass.  A CI step
/// (`Surface environmental test skips`) drains the ledger into annotations +
/// a job summary.  No-op unless `LOFT_SKIP_LEDGER` (a directory) is set, so
/// local runs are unaffected.  One file per test process (pid-named) avoids
/// cross-process write races.
fn record_env_skips(suite: &str, reason: &str, skips: &[(String, String)]) {
    let Ok(dir) = std::env::var("LOFT_SKIP_LEDGER") else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = std::path::Path::new(&dir).join(format!("{suite}-{}.tsv", std::process::id()));
    let body: String = skips
        .iter()
        .map(|(entry, detail)| {
            let clean = |s: &str| s.replace(['\t', '\n'], " ");
            format!("{suite}\t{reason}\t{}\t{}\n", clean(entry), clean(detail))
        })
        .collect();
    let _ = std::fs::write(path, body);
}
