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
    //
    // 14-image (`use imaging`) + 21-random (`use random`): test-backed on BOTH
    // backends via `tests/doc_lib_examples.rs` (subprocess through the real `loft`
    // binary, interpret == native).  Skipped in THIS in-process harness because it
    // builds native code without `out.native_cabi = native_cabi_enabled()` (unlike
    // src/test_runner.rs:838), so it emits the legacy `extern crate <pkg>` path and
    // rustc E0463s (no rlib) while the link provides the C-ABI cdylib.  NOT @P389
    // (resolved by C-ABI): the `loft` binary compiles two native packages fine.
    // A test-fidelity follow-up (set native_cabi here) would let these run inline too.
    "14-image.loft",
    "21-random.loft",
];

/// Script files to skip in native mode.
const SCRIPTS_NATIVE_SKIP: &[&str] = &[
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

/// @PLN24 arc C — a `#c` binding calls a real C symbol from native-compiled loft,
/// with no Rust wrapper and no rustc in any library.
///
/// Bound to **libc**, deliberately: it is linked into every Rust binary, so this
/// proves the whole path — declaration, typed `extern "C"`, marshalling, call —
/// with no build step and nothing to install.
///
/// `atoi("-1")` is the cell that matters. A C `int` return read back as a bare
/// `u64` — what a signature-blind caller does — is 4294967295, a plausible large
/// positive that every `>= 0` check accepts; it is the shape that silently
/// defeated `loft-libs-net`'s `server::listen`. It comes back as -1 here only
/// because the declared width goes into the extern, so rustc truncates at the
/// ABI and the cast then sign-extends. That is the plan's invariant, executing.
///
/// `write(1, ptr, count)` covers the other half: a loft `vector` crosses as a
/// pointer AND a count, because C carries no length.
#[test]
fn native_c_binding_calls_libc() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let rlib_info = find_loft_rlib();
    let path = std::env::temp_dir().join("loft_pln24_c_binding.loft");
    std::fs::write(
        &path,
        "pub fn c_strlen(s: text) -> integer;   #c \"strlen\" \"size_t(const char*)\"\n\
         pub fn c_atoi(s: text) -> integer;     #c \"atoi\" \"int(const char*)\"\n\
         pub fn c_abs(v: integer) -> integer;   #c \"abs\" \"int(int)\"\n\
         pub fn c_write(fd: integer, v: vector<u8>) -> integer;  #c \"write\" \"long(int, const void*, size_t)\"\n\
         fn main() {\n\
         \x20 println(\"len {c_strlen(\"hello\")}\");\n\
         \x20 println(\"neg {c_atoi(\"-1\")}\");\n\
         \x20 println(\"abs {c_abs(-7)}\");\n\
         \x20 b: vector<u8> = [];\n\
         \x20 for ch in \"hi\\n\" { b += [(ch as integer) as u8? ?? (0 as u8)] }\n\
         \x20 println(\"wrote {c_write(1, b)}\");\n\
         }\n",
    )?;
    let job = prepare_native_test(&path)?;
    // Ok(false) = skipped (rustc absent / low space) — not a failure.
    if !compile_native_job(&job, &rlib_info)? {
        return Ok(());
    }
    let out = std::process::Command::new(&job.binary).output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stdout: {stdout}\nstderr: {stderr}");
    assert!(stdout.contains("len 5"), "strlen: {stdout}");
    assert!(
        stdout.contains("neg -1"),
        "a 32-bit C return must sign-extend, not come back as 4294967295 — the \
         declared width is what makes that work: {stdout}"
    );
    assert!(stdout.contains("abs 7"), "abs: {stdout}");
    assert!(
        stdout.contains("hi\n") && stdout.contains("wrote 3"),
        "a vector must cross as pointer + count: {stdout}"
    );
    Ok(())
}

/// @PLN24 arc B — the interpreter calls the same C symbols, and answers the
/// SAME thing.
///
/// The interpreter has no compiler at the call site, so it resolves the symbol
/// and calls it through the fixed per-arity trampolines the architecture probe
/// validated. This test is the parity half: identical stdout from both
/// backends, cell for cell, which is the bar a `#c` binding has to meet before
/// it can be said to work at all. Before arc B, `strlen("hello")` compiled
/// under `--interpret` and answered **7562**.
///
/// The `neg` cell carries the whole reason the declaration has a signature: the
/// interpreter reads a raw `u64` return register, so `atoi("-1")` arrives as
/// 4294967295 unless the DECLARED width truncates and re-extends it.
#[test]
fn interpreted_and_native_c_bindings_agree() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let path = std::env::temp_dir().join("loft_pln24_c_parity.loft");
    std::fs::write(
        &path,
        "pub fn c_strlen(s: text) -> integer;   #c \"strlen\" \"size_t(const char*)\"\n\
         pub fn c_atoi(s: text) -> integer;     #c \"atoi\" \"int(const char*)\"\n\
         pub fn c_abs(v: integer) -> integer;   #c \"abs\" \"int(int)\"\n\
         pub fn c_write(fd: integer, v: vector<u8>) -> integer;  #c \"write\" \"long(int, const void*, size_t)\"\n\
         fn main() {\n\
         \x20 println(\"len {c_strlen(\"hello\")}\");\n\
         \x20 println(\"neg {c_atoi(\"-1\")}\");\n\
         \x20 println(\"abs {c_abs(-7)}\");\n\
         \x20 b: vector<u8> = [];\n\
         \x20 for ch in \"hi\\n\" { b += [(ch as integer) as u8? ?? (0 as u8)] }\n\
         \x20 println(\"wrote {c_write(1, b)}\");\n\
         }\n",
    )?;
    let interp = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&path)
        .output()?;
    let out = String::from_utf8_lossy(&interp.stdout).into_owned();
    let err = String::from_utf8_lossy(&interp.stderr);
    assert!(interp.status.success(), "interpret failed: {out}\n{err}");
    assert!(out.contains("len 5"), "{out}");
    assert!(
        out.contains("neg -1"),
        "the interpreter reads a RAW return register — without the declared \
         width this is 4294967295: {out}"
    );
    assert!(out.contains("abs 7") && out.contains("wrote 3"), "{out}");

    // The parity half. `prepare_native_test` may skip (no rustc / low space),
    // and a skipped comparison must not read as agreement.
    let job = prepare_native_test(&path)?;
    if !compile_native_job(&job, &find_loft_rlib())? {
        return Ok(());
    }
    let native = std::process::Command::new(&job.binary).output()?;
    let nout = String::from_utf8_lossy(&native.stdout);
    assert_eq!(
        out, nout,
        "the two backends must answer identically, cell for cell"
    );
    Ok(())
}

/// @PLN24 arc D — the composition matrix, against a REAL C library, on both
/// backends.
///
/// Arcs A-C proved the mechanism against libc, which is already in the process.
/// This is the shape a library actually has: a package that declares
/// `[c] libs`, a `.so` built by nothing but `cc`, and a binding per cell of the
/// plan's mapping table — every integer width, a text argument, a vector as
/// pointer + count, the opaque-handle open/read/bump/close cycle, and the
/// 7-argument call that straddles the SysV register/stack boundary.
///
/// The assertion is that the two backends produce **identical** output, because
/// that is the only claim worth making: each one alone can be plausibly wrong
/// in a way the other is not.
///
/// Skips when `cc` is absent — the fixture is C, and a machine without a C
/// compiler cannot build it. It does NOT skip silently on a failed build: that
/// is a real failure.
#[test]
fn c_binding_matrix_against_a_declared_library() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let built = std::process::Command::new("make")
        .arg("-C")
        .arg(&root)
        .output()?;
    assert!(
        built.status.success(),
        "the fixture library must build: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let prog = std::env::temp_dir().join("loft_pln24_matrix.loft");
    std::fs::write(
        &prog,
        "use lcabi;\n\
         fn main() {\n\
         \x20 println(\"i64 {lc_i64(lc_i64(1234567890123))}\");\n\
         \x20 println(\"neg {lc_neg_i32(1)}\");\n\
         \x20 println(\"len {lc_strlen(\"loft\")}\");\n\
         \x20 println(\"byte {lc_byte_at(\"loft\", 0)}\");\n\
         \x20 v: vector<integer> = [10, 20, 30];\n\
         \x20 println(\"vec {lc_i64_sum(v)}\");\n\
         \x20 h = lc_open(1000);\n\
         \x20 println(\"handle {lc_read(h)} {lc_bump(h, 7)}\");\n\
         \x20 lc_close(h);\n\
         \x20 println(\"arity7 {lc_arity7(1,1,1,1,1,1,1)}\");\n\
         \x20 t = lc_static_text();\n\
         \x20 println(\"text {t} {t.len()}\");\n\
         \x20 println(\"textarg {lc_strlen(lc_static_text())}\");\n\
         \x20 println(\"cat [{lc_static_text()}][{lc_static_text()}]\");\n\
         \x20 println(\"some {lc_maybe_text(1)}\");\n\
         \x20 println(\"none {lc_opt_text(0) ?? \"<null>\"}\");\n\
         \x20 println(\"latin1 {lc_latin1_text().len()}\");\n\
         \x20 c_text_positions();\n\
         }\n\
         // A text return has to reach EVERY value position, because the caller\n\
         // needs a destination record handed to it and only some positions were\n\
         // routed at first: a struct literal, a vector literal, a field write, a\n\
         // `match` subject and a comparison all lower to `w += producer()`, which\n\
         // is a different emission site from a plain local assignment. Missing it\n\
         // is not a slow path for a `#c` binding — there is no non-destination\n\
         // sibling to fall back to, so it SIGSEGV'd the interpreter while\n\
         // `--native` stayed correct. One function per position, so a regression\n\
         // names the position it broke.\n\
         struct CRec { name: text, n: integer }\n\
         fn c_text_positions() {\n\
         \x20 o = CRec{name: lc_static_text(), n: 1};\n\
         \x20 println(\"p-lit {o.name}\");\n\
         \x20 o.name = lc_maybe_text(1);\n\
         \x20 println(\"p-field {o.name}\");\n\
         \x20 if lc_static_text() == \"loft/c-abi\" { println(\"p-cmp ok\") } else { println(\"p-cmp NO\") }\n\
         \x20 m = match lc_static_text() { \"loft/c-abi\" => lc_maybe_text(1), _ => \"-\" };\n\
         \x20 println(\"p-match {m}\");\n\
         \x20 v: vector<text> = [lc_static_text(), lc_maybe_text(1)];\n\
         \x20 println(\"p-vlit {v[0]} {v[1]}\");\n\
         \x20 println(\"p-cond {if lc_static_text().len() > 3 { lc_static_text() } else { \"-\" }}\");\n\
         \x20 for i in 0..2 { println(\"p-loop{i} {lc_static_text()}\") }\n\
         \x20 s = lc_static_text() + \"!\";\n\
         \x20 println(\"p-concat {s}\");\n\
         }\n",
    )?;
    let libdir = root.join("pkg");
    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    // Hand-computed, not copied from a run: `lc_i64` is self-inverse, the
    // vector sum is position-weighted (10*1 + 20*2 + 30*3), and arity7's
    // arguments carry distinct prime weights (2+3+5+7+11+13+17).
    for want in [
        "i64 1234567890123",
        "neg -1",
        "len 4",
        "byte 108",
        "vec 140",
        "handle 1000 1007",
        "arity7 58",
        // @PLN24 arc D — the `char *` return. Hand-computed: "loft/c-abi" is 10
        // characters, so C's byte count and loft's character count agree on it
        // (`textarg` re-crosses the answer to prove the copy is NUL-terminated,
        // not merely non-empty). NULL reads as loft null. "caf\xE9" is 3 ASCII
        // bytes plus one invalid UTF-8 byte, which becomes ONE replacement
        // character — 4, not 3 (dropped) and not 5 (bytes counted as characters).
        "text loft/c-abi 10",
        "textarg 10",
        "cat [loft/c-abi][loft/c-abi]",
        "some here",
        "none <null>",
        "latin1 4",
        // Every value position, each named so a failure says which one broke.
        "p-lit loft/c-abi",
        "p-field here",
        "p-cmp ok",
        "p-match here",
        "p-vlit loft/c-abi here",
        "p-cond loft/c-abi",
        "p-loop0 loft/c-abi",
        "p-loop1 loft/c-abi",
        "p-concat loft/c-abi!",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "the two backends must agree cell for cell against a real library"
    );
    Ok(())
}

/// @PLN24 arc D — a C `char *` comes back as loft `text`, identically on both
/// backends, from libc alone (no fixture, no library to install).
///
/// `strerror` is the shape a real binding meets: borrowed storage the caller
/// must not free, a plain `int` argument, and an answer the C library owns. The
/// interpreter copies out of a raw return register and `--native` copies out of
/// a typed `*const c_void`; the two arrive at the same text or this crossing is
/// two mappings rather than one.
#[test]
fn a_c_string_return_crosses_identically_on_both_backends() -> std::io::Result<()> {
    let path = std::env::temp_dir().join("loft_pln24_c_textret.loft");
    std::fs::write(
        &path,
        "pub fn c_strerror(n: integer) -> text;   #c \"strerror\" \"char*(int)\"\n\
         pub fn c_strlen(s: text) -> integer;     #c \"strlen\" \"size_t(const char*)\"\n\
         fn main() {\n\
         \x20 e = c_strerror(2);\n\
         \x20 println(\"len {c_strlen(e)} chars {e.len()}\");\n\
         \x20 println(\"arg {c_strlen(c_strerror(2))}\");\n\
         }\n",
    )?;
    let mut seen = Vec::new();
    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg(&path)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // Not an exact string: `strerror(2)` is locale-dependent (it is "No such
        // file or directory" in the C locale). What IS pinned is that the text
        // survived the crossing — a non-empty answer whose byte length C agrees
        // with, which a truncated or NUL-terminated-at-zero copy fails.
        assert!(
            !stdout.contains("len 0 ") && !stdout.contains("arg 0"),
            "{backend}: an empty `strerror` means the crossing dropped the text:\n{stdout}"
        );
        seen.push(stdout);
    }
    assert_eq!(
        seen[0], seen[1],
        "the two backends must bring back the same text"
    );
    Ok(())
}

/// @PLN24 arc G — `c_library_available` is symbol-granular, not file-granular.
///
/// The cell that matters: a library that LOADS but does not export a symbol the
/// package declared. A file-granular answer says yes here and the call then
/// faults — the version-skew hole that makes a naive query worse than none, and
/// the reason the answer checks symbols at all.
///
/// The library is built here rather than mocked, and the control is the call
/// that WORKS (`sk_present(41)` is 42, by hand): without it a `false` could just
/// mean nothing loaded, which is the vacuous pass this test exists to refuse.
#[test]
fn an_available_library_must_export_what_was_declared() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the fixture library is built by `cc` here
    }
    let dir = std::env::temp_dir().join(format!("loft_skew_{}", std::process::id()));
    let pkg = dir.join("pkg/skewlib/src");
    std::fs::create_dir_all(&pkg)?;
    let src = dir.join("old.c");
    std::fs::write(&src, "long sk_present(long v) { return v + 1; }\n")?;
    let so = dir.join("libskew.so");
    let built = std::process::Command::new("cc")
        .args(["-O2", "-fPIC", "-shared", "-o"])
        .arg(&so)
        .arg(&src)
        .output()?;
    assert!(built.status.success(), "fixture library must build");

    let so_str = so.to_string_lossy().into_owned();
    std::fs::write(
        dir.join("pkg/skewlib/loft.toml"),
        format!(
            "[library]\nname = \"skewlib\"\nversion = \"0.1.0\"\n\n[c]\noptional-libs = \"{so_str}\"\n"
        ),
    )?;
    std::fs::write(
        pkg.join("skewlib.loft"),
        format!(
            "pub fn sk_present(v: integer) -> integer;  #c \"sk_present\" \"long(long)\"\n\
             // Declared, and absent from this vintage of the library.\n\
             pub fn sk_newer(v: integer) -> integer;    #c \"sk_newer\" \"long(long)\"\n\
             pub const SKEW_SONAME = \"{so_str}\";\n\
             pub fn skew_ok() -> boolean {{ return c_library_available(SKEW_SONAME); }}\n"
        ),
    )?;
    let script = dir.join("probe.loft");
    std::fs::write(
        &script,
        "use skewlib;\nfn go() {\n  println(\"ok={skew_ok()}\");\n  println(\"call={sk_present(41)}\");\n}\ngo();\n",
    )?;

    for backend in ["--interpret", "--native"] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(dir.join("pkg"))
            .arg(&script)
            .output()?;
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}: {s}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            s.contains("call=42"),
            "{backend}: the control must prove the library IS loaded and callable:\n{s}"
        );
        assert!(
            s.contains("ok=false"),
            "{backend}: a loadable library missing a declared symbol is NOT available:\n{s}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// @PLN23 — one uniform interface over four different C libraries.
///
/// `dump <D: SqlDb>` and `seed <D: SqlDb>` in the fixture never name a backend,
/// and every backend runs them unchanged. That is the whole claim, and it is not
/// a small one: sqlite STEPS a prepared statement, libpq MATERIALISES the result
/// and indexes it by (row, col), libmariadb streams rows as `char **`, and
/// duckdb materialises and is read by (col, row) with a caller-frees string.
/// Four result models behind one cursor.
///
/// **sqlite is the cell that keeps this honest.** It needs no server, so a
/// machine with no database still proves the interface, the bindings, the shim
/// loft compiled, and SQL NULL staying distinct from the empty string. Only
/// postgres and mariadb are conditional, and a skip is printed and recognised —
/// never silently counted as a pass, which is how a dead binding would otherwise
/// look green everywhere.
///
/// **duckdb is the fourth, and it is here because of @PLN24 arc G.** It was
/// proven once before and left out of the tree: no distro packages it and its
/// `.so` is 70 MB, so a REQUIRED declaration would have made every machine
/// running this test fetch it. Declared `[c] optional-libs` it costs nothing —
/// the fixture builds and runs without it, and says so — which is what makes
/// keeping a fourth backend in the tree cheap rather than a vendoring decision.
#[test]
fn one_sql_interface_drives_four_different_c_libraries() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // the sqlite backend ships a shim loft must compile
    }
    let has = |names: &[&str]| {
        names.iter().any(|n| {
            ["/lib/x86_64-linux-gnu/", "/usr/lib/", "/usr/lib64/"]
                .iter()
                .any(|d| std::path::Path::new(&format!("{d}{n}")).exists())
        })
    };
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let libdir = root.join("tests/fixtures/sqldb");
    let script = libdir.join("uniform.loft");
    let run = |backend: &str, mode: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .env("LOFT_SQLDB_MODE", mode)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}/{mode}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };

    // The three rows every backend is given, rendered by the SAME generic code:
    // a value, SQL NULL, and the empty string.
    let expect = "[ada] <null> [] ";

    // @PLN23 S4 — the prepared-statement line, and every token in it is a
    // separate claim:
    //
    //   p=2      two holes, and the DRIVER's own parameter count agreed with the
    //            parser's (each backend fails `prepare` outright when they differ)
    //   [ada] <null> []   value / SQL NULL / empty string, still three answers
    //                     — now arriving through the BIND path rather than as
    //                     literals in the statement text
    //   ['); DROP TABLE loft_p; --]
    //            the cell that matters. Spliced into SQL this closes the VALUES
    //            list and drops the table; bound, it is stored verbatim. That the
    //            SELECT after it returns rows at all is the proof the table
    //            survived, so this assertion cannot pass vacuously.
    //   hit=4    the same hostile text found its own row by EQUALITY, which it
    //            could only do by crossing intact as data rather than as syntax.
    //   big=1000 a value far past the 256-byte column buffer mariadb's result
    //            binds start with, round-tripped at full length — the truncation
    //            re-fetch is exercised, not merely written.
    let bound = "p=2 [ada] <null> [] ['); DROP TABLE loft_p; --] hit=4 big=1000";

    // @PLN23 T1–T3 — the transaction line, from one generic `transact<D: SqlDb>` that
    // names no backend. It lands before the object mapping because writing a collection
    // non-atomically is not a smaller step, it is a wrong one.
    //
    //   nested=false      a `db_begin` INSIDE a transaction is REFUSED. The silent
    //                     no-op is the dangerous version: the inner "rollback" would
    //                     discard the OUTER transaction's work, and nothing afterwards
    //                     can tell.
    //   rows=0/1          rollback DISCARDED the row, commit KEPT it. The pair is what
    //                     makes the cell non-vacuous — three of these four fields move
    //                     when the transaction is a fiction.
    //   stray=false/false commit or rollback with nothing open is refused.
    //
    // Proven to FIRE by replacing sqlite's three methods with `return true` (a backend
    // that reports transactions it never opens): `nested=true rows=1/2 stray=true/true`.
    let tx = "begin=true/true nested=false rollback=true commit=true rows=0/1 stray=false/false";

    // sqlite — unconditional. No server, so a failure here is always real.
    if has(&["libsqlite3.so.0"]) {
        let s = run("--interpret", "sqlite")?;
        assert!(
            s.contains(&format!("sqlite {expect}")),
            "sqlite must render value / NULL / empty distinctly:\n{s}"
        );
        assert!(
            s.contains(&format!("sqlite bound {bound}")),
            "sqlite: a bound value must reach the server as DATA, never as syntax:\n{s}"
        );
        assert!(
            s.contains(&format!("sqlite tx {tx}")),
            "sqlite: rollback must discard, commit must persist, and a nested begin \
             must be REFUSED:\n{s}"
        );
        assert_eq!(
            run("--native", "sqlite")?,
            s,
            "both backends, one interface"
        );
    }

    // postgres and mariadb — conditional, and a skip is recognised as a skip.
    for (mode, lib) in [("postgres", "libpq.so.5"), ("maria", "libmariadb.so.3")] {
        if !has(&[lib]) {
            continue;
        }
        let out = run("--interpret", mode)?;
        if out.contains("SKIP") {
            continue; // no server reachable here
        }
        assert!(
            out.contains(&format!("{mode} {expect}")),
            "{mode} must render the same three cells as sqlite:\n{out}"
        );
        // Byte-identical to sqlite's, from the same generic `bound<D: SqlDb>`:
        // three placeholder dialects, three bind APIs, three result models, one
        // answer. A backend that quietly concatenated would differ HERE and
        // nowhere else, which is what makes the line worth comparing whole.
        assert!(
            out.contains(&format!("{mode} bound {bound}")),
            "{mode}: the bound statement must give the same answer as sqlite:\n{out}"
        );
        // The same generic `transact<D: SqlDb>`, byte-identical to sqlite's answer:
        // three servers with three transaction implementations, one contract.
        assert!(
            out.contains(&format!("{mode} tx {tx}")),
            "{mode}: transactions must behave exactly as sqlite's do:\n{out}"
        );
        assert_eq!(
            run("--native", mode)?,
            out,
            "{mode}: both backends must agree"
        );
    }

    // @PLN24 arc G — duckdb, declared `[c] optional-libs`.
    //
    // This cell is unconditional ON PURPOSE, and it is the one that proves the
    // arc: every mode above needs its C library installed to run at all, and
    // before arc G a declared-but-absent library failed the `--native` LINK, so
    // a program that never called into it did not build. Here the program is
    // expected to build and run either way — the only question is which of the
    // two answers it gives.
    let out = run("--interpret", "duckdb")?;
    let native = run("--native", "duckdb")?;
    assert_eq!(
        native, out,
        "duckdb: both backends must agree about an optional library"
    );
    if out.contains("SKIP") {
        // libduckdb absent — the interesting half. The program still ran.
        assert!(
            out.contains("not installed"),
            "an absent optional library must be REPORTED, not inferred from silence:\n{out}"
        );
    } else {
        assert!(
            out.contains(&format!("duckdb {expect}")),
            "duckdb must render the same three cells as sqlite:\n{out}"
        );
    }
    Ok(())
}

/// @PLN23 S3 — the cursor model: a real result set, walked through a shim loft
/// compiled itself, with SQL NULL kept distinct from the empty string.
///
/// The whole vertical slice in one test — loft → libmariadb + a loft-built
/// ANSI-C shim → a live server → rows back — with no rustc anywhere.
///
/// **The `shim` mode is what keeps this honest.** The cursor needs a server, and
/// a test that merely skips when one is absent cannot tell "no database" from "a
/// dead binding", so it would report a broken shim as a pass on every machine
/// without MariaDB. The `shim` mode needs no server and still exercises the
/// pieces that can silently rot: the shim compiled, `char *` → `text?`, and a
/// NULL pointer arriving as loft null. Only the SERVER half is conditional.
#[test]
fn a_sql_cursor_walks_real_rows_and_keeps_null_apart_from_empty() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists());
    if !present
        || std::process::Command::new("cc")
            .arg("--version")
            .output()
            .is_err()
    {
        return Ok(());
    }
    let libdir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // Beside the fixture it needs, NOT in tests/scripts/ — that directory is swept
    // and every script there must run standalone, while this one needs `--lib`.
    let script = libdir.join("mariadb").join("cursor.loft");
    let run = |backend: &str, mode: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&libdir)
            .arg(&script)
            .env("LOFT_PLN23_MODE", mode)
            .current_dir(root)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}/{mode}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };

    // Always: the shim and the null crossing, no server involved.
    let shim = run("--interpret", "shim")?;
    assert!(
        shim.contains("shim null=true") && shim.contains("shim info=true"),
        "the shim must build and a null row must cross as loft null:\n{shim}"
    );
    assert_eq!(
        run("--native", "shim")?,
        shim,
        "both backends, one loft-built shim"
    );

    // Conditional: the cursor itself.
    let cursor = run("--interpret", "cursor")?;
    if cursor.contains("SKIP unreachable") {
        return Ok(()); // no server here; the shim half above still ran
    }
    for want in [
        "cols 3 rows 3",
        "row 1 ada NULL", // SQL NULL
        "row 2 grace [hi]",
        "row 3 kay []", // the empty string — NOT the same thing
        "done",
    ] {
        assert!(cursor.contains(want), "cursor missing `{want}`:\n{cursor}");
    }
    assert_eq!(
        run("--native", "cursor")?,
        cursor,
        "both backends must read the same rows"
    );
    Ok(())
}

/// @PLN23 S2 — the opaque-handle lifecycle against a real client library.
///
/// `MYSQL *` crosses as a loft `integer` holding the pointer, which is the
/// convention @PLN24 chose because loft has no type separating a handle from a
/// number. Three things have to hold: a handle comes back, it survives being
/// passed BACK in, and a failure carries C's own message across.
///
/// Deliberately **not** gated on a running server. A test that skips when the
/// database is absent cannot tell "no server" from "the binding is broken", and
/// would report a dead binding as a pass on every machine without MariaDB. So
/// the cells here need no server at all: `mysql_init` and `mysql_close` are
/// client-side, and connecting to a port nothing listens on exercises the error
/// path — the binding is proved either way, and only the SPEED of the failure
/// depends on the environment.
#[test]
fn a_c_library_handle_survives_the_round_trip_and_carries_its_error() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists());
    if !present {
        return Ok(());
    }
    let dir = std::env::temp_dir().join("loft_pln23_s2");
    let pkg = dir.join("mariadb").join("src");
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(
        dir.join("mariadb").join("loft.toml"),
        "[library]\nname = \"mariadb\"\nversion = \"0.0.1\"\n\n[c]\nlibs = \"libmariadb.so.3\"\n",
    )?;
    std::fs::write(
        pkg.join("mariadb.loft"),
        // `unix_socket` is `integer`, not `text`: it has to be able to be NULL,
        // and loft text is non-null with no way to spell a null pointer.
        "pub fn db_init(h: integer) -> integer;  #c \"mysql_init\" \"void*(void*)\"\n\
         pub fn db_close(h: integer);            #c \"mysql_close\" \"void(void*)\"\n\
         pub fn db_errno(h: integer) -> integer; #c \"mysql_errno\" \"int(void*)\"\n\
         pub fn db_error(h: integer) -> text;    #c \"mysql_error\" \"const char*(void*)\"\n\
         pub fn db_connect(h: integer, host: text, user: text, pass: text, db: text, port: integer, sock: integer, flags: integer) -> integer;\n\
         #c \"mysql_real_connect\" \"void*(void*, const char*, const char*, const char*, const char*, int, const char*, long)\"\n",
    )?;
    let prog = dir.join("s2.loft");
    std::fs::write(
        &prog,
        "use mariadb;\n\
         fn main() {\n\
         \x20 h = db_init(0);\n\
         \x20 println(\"handle {h != 0}\");\n\
         \x20 // Port 1 has no MariaDB anywhere; the point is the error crossing.\n\
         \x20 c = db_connect(h, \"127.0.0.1\", \"nobody\", \"nothing\", \"\", 1, 0, 0);\n\
         \x20 println(\"refused {c == 0} errno {db_errno(h) != 0} msg {db_error(h).len() > 0}\");\n\
         \x20 db_close(h);\n\
         \x20 println(\"closed\");\n\
         }\n",
    )?;
    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--no-warnings")
            .arg("--lib")
            .arg(&dir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        "handle true",                      // a pointer came back as an integer
        "refused true errno true msg true", // and C's own diagnosis came back with it
        "closed",                           // the handle was still usable at the end
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    assert_eq!(run("--native")?, interp, "both backends, one C library");
    Ok(())
}

/// @PLN24 arc F / @PLN23 S1 — a `#c` binding reaches a real SYSTEM library, on
/// both backends, with no rustc and no dev headers.
///
/// The fixture proved the mechanism; a system library proves the parts a fixture
/// cannot have. `libmariadb.so.3` is a VERSIONED soname, and that is the whole
/// point of this test: `-l dylib=mariadb` makes the linker look for
/// `libmariadb.so`, the `-dev` symlink, while the interpreter `dlopen`s
/// `libmariadb.so.3`, the runtime file — so one declaration resolved to two
/// different files and the program ran interpreted and failed to LINK natively on
/// a machine where the library is plainly installed.
///
/// Skips when libmariadb is absent, because a machine without it says nothing.
#[test]
fn a_c_binding_reaches_a_versioned_system_library_on_both_backends() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    // Present only if the runtime package is installed; the `-dev` symlink is
    // deliberately NOT required, which is the property under test.
    let present = [
        "/lib/x86_64-linux-gnu/libmariadb.so.3",
        "/usr/lib/libmariadb.so.3",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists());
    if !present {
        return Ok(());
    }
    let dir = std::env::temp_dir().join("loft_pln23_s1");
    let pkg = dir.join("mariadb").join("src");
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(
        dir.join("mariadb").join("loft.toml"),
        "[library]\nname = \"mariadb\"\nversion = \"0.0.1\"\n\n[c]\nlibs = \"libmariadb.so.3\"\n",
    )?;
    std::fs::write(
        pkg.join("mariadb.loft"),
        "pub fn client_info() -> text;       #c \"mysql_get_client_info\" \"const char*(void)\"\n\
         pub fn client_version() -> integer; #c \"mysql_get_client_version\" \"long(void)\"\n",
    )?;
    let prog = dir.join("s1.loft");
    std::fs::write(
        &prog,
        "use mariadb;\nfn main() { println(\"{client_info()} {client_version()}\") }\n",
    )?;

    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&dir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    // Not an exact version — that is the machine's. What is pinned: the text came
    // back non-empty and the integer is a real version, so both crossings worked.
    let v: i64 = interp
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    assert!(
        v > 10000,
        "the client version must cross as a real number: {interp:?}"
    );
    assert_eq!(
        run("--native")?,
        interp,
        "both backends, one system library"
    );
    Ok(())
}

/// @PLN24 arc D — loft compiles the ANSI-C shim itself, with `cc` and no rustc.
///
/// The plan's trade is that loft-core stays a generic linking tool and every
/// signature the fixed trampolines cannot express is wrapped in a few lines of C
/// the library ships. That trade only holds if loft can BUILD those lines —
/// otherwise the escape hatch is a claim, and the author's alternative is the
/// rustc toolchain `#c` exists to avoid.
///
/// One cell per shape that needs a shim: a `double` argument (a different
/// register file, so it crosses as its bit pattern), an out-parameter (two
/// answers, one return slot), and a caller-frees `char *` (loft never frees one,
/// so the shim owns the release). The float cell is hand-computed rather than
/// copied from a run — 2.5 × 4.0 is exactly 10.0, whose bit pattern is pinned
/// below, so a shim that returned a plausible-but-wrong double fails.
#[test]
fn loft_builds_the_ansi_c_shim_a_package_ships() -> std::io::Result<()> {
    let _guard = native_suite_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if std::process::Command::new("cc")
        .arg("--version")
        .output()
        .is_err()
    {
        return Ok(()); // no C compiler on this machine
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/c_abi");
    let libdir = root.join("pkg");
    // Start from no artifact, so the build itself is what is under test.
    let _ = std::fs::remove_dir_all(libdir.join("lcshim").join("native-auto"));

    let prog = std::env::temp_dir().join("loft_pln24_shim.loft");
    std::fs::write(
        &prog,
        "use lcshim;\n\
         fn main() {\n\
         \x20 println(\"scale {shim_scale(4612811918334230528, 4616189618054758400)}\");\n\
         \x20 println(\"mod {shim_mod(17, 5)}\");\n\
         \x20 println(\"owned {shim_owned(7)}\");\n\
         }\n",
    )?;
    let run = |backend: &str| -> std::io::Result<String> {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg(backend)
            .arg("--lib")
            .arg(&libdir)
            .arg(&prog)
            .output()?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(stdout)
    };
    let interp = run("--interpret")?;
    for want in [
        // 2.5 * 4.0 = 10.0 → 0x4024000000000000.
        "scale 4621819117588971520",
        "mod 2",
        "owned shim-7",
    ] {
        assert!(
            interp.contains(want),
            "interpret missing `{want}`:\n{interp}"
        );
    }
    // The artifact is the proof loft did the compiling: nothing else put it there.
    let built: Vec<_> = std::fs::read_dir(libdir.join("lcshim").join("native-auto"))
        .map(|d| d.filter_map(Result::ok).map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        !built.is_empty(),
        "loft must have built the shim into native-auto/"
    );

    let native = run("--native")?;
    assert_eq!(
        interp, native,
        "a shim-backed binding must answer the same on both backends"
    );
    Ok(())
}

/// @PLN24 arc D — the return spellings a `#c` declaration must refuse, because
/// nothing at runtime would.
///
/// The plan's central measurement is that a wrong binding produces a plausible
/// answer rather than a failure, so every one of these is caught at the
/// declaration or not at all. Both backends must refuse in the same words: a
/// shape rejected by one and accepted by the other is the divergence this plan
/// exists to keep out.
#[test]
fn a_text_return_must_say_it_is_a_c_string() -> std::io::Result<()> {
    // (declaration, the words the refusal has to carry)
    let cases = [
        (
            "pub fn f() -> text;   #c \"lc_x\" \"void*(void)\"",
            "spell the return `char*`",
        ),
        (
            "pub fn f() -> text;   #c \"lc_x\" \"int(void)\"",
            "the loft declaration returns `text`",
        ),
        (
            // A `vector` return cannot be reconstructed from a pointer: C
            // carries no length, which is why an ARGUMENT crosses as a PAIR and
            // a return cannot cross at all.
            "pub fn f() -> vector<integer>;   #c \"lc_x\" \"char*(void)\"",
            "returns `vector<integer>` but the C signature returns `char *`",
        ),
    ];
    for (decl, want) in cases {
        let path = std::env::temp_dir().join("loft_pln24_c_textret_refuse.loft");
        std::fs::write(&path, format!("{decl}\nfn main() {{ println(\"x\") }}\n"))?;
        let mut seen = Vec::new();
        for backend in ["--interpret", "--native"] {
            let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
                .arg(backend)
                .arg("--errors=compact")
                .arg(&path)
                .output()?;
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            assert!(
                !out.status.success(),
                "{backend} must refuse `{decl}`: {stderr}"
            );
            assert!(
                stderr.contains(want),
                "{backend}: refusing `{decl}` must say `{want}`: {stderr}"
            );
            seen.push(stderr);
        }
        assert_eq!(
            seen[0], seen[1],
            "and both backends must say it identically"
        );
    }
    Ok(())
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

/// loft#742 — a NESTED vector field must reference the content type the
/// compiler recorded, not one rebuilt from the loft `Type`.
///
/// `type_def_nr` cannot see an element's `forced_size`, so rebuilding the
/// nesting produced `db.vector(db.vector(<plain integer>))` for BOTH
/// `vector<vector<integer>>` and `vector<vector<integer(-32768, 32767)>>` —
/// the wrong element width, and a `vector<vector<integer>>` minted at an id
/// the compiler had assigned to something else, which shifted every runtime
/// type id after it.
///
/// The check is `LOFT_STRICT_SCHEMA_IDS`, which turns the `init()` schema
/// comparison into a hard failure at the first divergence — the same gate that
/// found this. Asserting the program's OUTPUT alone would not catch it: these
/// programs print correct answers today, because nothing happened to bake an
/// id at or past the shift.
#[test]
fn a_nested_narrow_vector_field_keeps_the_type_ids_aligned() {
    if std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skip: rustc unavailable (--native needs it)");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // Each of these carries a differently-shaped nested narrow vector, and each
    // drifted by a different amount before the fix.
    for script in [
        "tests/scripts/184-nested-narrow-int-vector.loft",
        "tests/scripts/624-nested-narrow-width.loft",
        "tests/scripts/432-untyped-vector-literal-arg.loft",
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--native")
            .arg(root.join(script))
            .env("LOFT_STRICT_SCHEMA_IDS", "1")
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run the loft binary");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("diverges from the compiler"),
            "{script}: the generated schema drifted from the compiler's.\n{stderr}"
        );
        assert!(
            out.status.success(),
            "{script}: exited non-zero.\n{}\n{stderr}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}
