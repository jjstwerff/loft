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
use std::collections::HashSet;
use std::io::Error;
use std::path::{Path, PathBuf};
mod common;
use common::cached_default;

/// Docs files that are known to fail in `--native` mode.
/// See PROBLEMS.md for details on each issue number.
const NATIVE_SKIP: &[&str] = &[
    // Add entries here only for confirmed native-mode failures with a CAVEATS.md reference.
];

/// Script files that are known to fail in `--native` mode.
/// See PROBLEMS.md for issue numbers.
/// Do NOT remove tests from this list by weakening the test — fix the native codegen instead.
const SCRIPTS_NATIVE_SKIP: &[&str] = &[
    // P3: native codegen does not generate loop variables for any/all/count_if.
    "47-predicates.loft",
    // A10: native codegen for field iteration's match arms not yet supported.
    "45-field-iter.loft",
    // T1: native codegen does not support tuple types (interpreter-only).
    "50-tuples.loft",
    // CO1: native codegen does not support coroutines/yield (interpreter-only).
    "51-coroutines.loft",
    // T1: caveats script uses tuple element assign — interpreter-only.
    "46-caveats.loft",
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

    // Find the most recently modified libloft-*.rlib in this profile's deps/.
    let rlib = std::fs::read_dir(&deps)
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("libloft-") && n.ends_with(".rlib")
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
/// linked against (rlib path + modification time).  If either changes the
/// key changes and the binary is recompiled.
fn cache_key(rs_content: &[u8], rlib_info: &Option<(PathBuf, PathBuf)>) -> u64 {
    let mut key = fnv64(rs_content);
    if let Some((rlib, _)) = rlib_info {
        key ^= fnv64(rlib.to_string_lossy().as_bytes());
        if let Ok(mtime) = std::fs::metadata(rlib).and_then(|m| m.modified()) {
            // Mix in the rlib modification time so a recompiled rlib (same path,
            // different binary) also invalidates the cache.
            let nanos = mtime
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let secs = mtime
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            key ^= fnv64(&secs.to_le_bytes());
            key ^= fnv64(&nanos.to_le_bytes());
        }
    }
    key
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
        if !def.attributes.is_empty() {
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
        let mut out = Output {
            data: &p.data,
            stores: &state.database,
            counter: 0,
            indent: 0,
            def_nr: 0,
            declared: HashSet::new(),
            reachable: HashSet::new(),
            loop_stack: Vec::new(),
            next_format_count: 0,
        };
        out.output_native_reachable(&mut buf, start_def, end_def, &entry_defs)?;
    }

    // For test-style files without fn main(), generate a main() that calls
    // each test function so the native binary is a valid executable.
    if !has_main && !test_fns.is_empty() {
        use std::io::Write;
        writeln!(buf, "\nfn main() {{")?;
        writeln!(buf, "    let mut stores = Stores::new();")?;
        writeln!(buf, "    init(&mut stores);")?;
        for (_, name) in &test_fns {
            writeln!(buf, "    {name}(&mut stores);")?;
        }
        writeln!(buf, "}}")?;
    }

    // Only write the .rs file when the content has changed.  This keeps the file's
    // content stable across runs where the loft source hasn't changed, which
    // means cache_key() produces the same hash and compile_native_job stays cached.
    let tmp_rs = std::env::temp_dir().join(format!("loft_native_{stem}.rs"));
    let existing = std::fs::read(&tmp_rs).unwrap_or_default();
    if existing != buf {
        std::fs::write(&tmp_rs, &buf)?;
    }

    let binary = std::env::temp_dir().join(format!("loft_native_{stem}_bin"));
    let key_file = std::env::temp_dir().join(format!("loft_native_{stem}_bin.key"));
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
        return Ok(true);
    }
    let mut cmd = std::process::Command::new("rustc");
    cmd.arg("--edition=2024")
        .arg("-C")
        .arg("debuginfo=0")
        .arg("-C")
        .arg("opt-level=0");
    // LOFT_CRANELIFT=1 — use the Cranelift codegen backend for much faster compilation.
    // Requires a nightly toolchain with `rustup component add rustc-codegen-cranelift-preview`.
    if std::env::var_os("LOFT_CRANELIFT").is_some() {
        cmd.arg("-Z").arg("codegen-backend=cranelift");
    }
    cmd.arg("-o").arg(&job.binary).arg(&job.tmp_rs);
    if let Some((rlib, deps_dir)) = rlib_info {
        cmd.arg("--extern")
            .arg(format!("loft={}", rlib.display()))
            .arg("-L")
            .arg(deps_dir);
        // S31: pass --extern for optional feature deps (rand_core, rand_pcg, etc.) so that
        // generated code using `random` or `png` features compiles without E0433 errors.
        for (crate_name, rlib_path) in collect_extra_externs(deps_dir) {
            cmd.arg("--extern")
                .arg(format!("{crate_name}={}", rlib_path.display()));
        }
    }
    // On Windows MSVC, build-script output dirs holding native import libs (e.g.
    // `windows.0.48.5.lib` from `windows-sys`) must be passed explicitly to standalone
    // rustc — cargo adds them automatically via `cargo:rustc-link-search`, but we don't.
    for dir in find_native_lib_dirs(rlib_info) {
        cmd.arg("-L").arg(dir);
    }
    let compile_out = match cmd.output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  rustc not found — skipping native test for {}", job.stem);
            return Ok(false);
        }
        Err(e) => return Err(e),
    };
    if !compile_out.status.success() {
        let stderr = String::from_utf8_lossy(&compile_out.stderr);
        eprintln!("rustc failed for {}:\n{stderr}", job.stem);
        let _ = std::fs::remove_file(&job.binary);
        let _ = std::fs::remove_file(&job.key_file);
        return Err(Error::from(std::io::ErrorKind::Other));
    }
    // Write the cache key so future runs can skip recompilation when nothing changed.
    let rs_content = std::fs::read(&job.tmp_rs).unwrap_or_default();
    let key = cache_key(&rs_content, rlib_info);
    let _ = std::fs::write(&job.key_file, format!("{key:016x}"));
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
    let concurrency = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(jobs.len().max(1));
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

/// Compile and run every `.loft` file in `tests/scripts/` through the native Rust
/// backend (`--native` mode), skipping files listed in `SCRIPTS_NATIVE_SKIP`.
///
/// Runs concurrently with interpreter-based wrap tests (no WRAP_LOCK).
/// Skips silently if `rustc` is not in PATH.
#[test]
fn native_scripts() -> std::io::Result<()> {
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
        // Skip files with intentional compile errors.
        if let Ok(src) = std::fs::read_to_string(&entry)
            && src.contains("@EXPECT_ERROR")
        {
            println!("skip {entry:?} (has @EXPECT_ERROR)");
            continue;
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
#[ignore = "N8a.1/N8a.2: native tuple codegen incomplete — Type::Tuple emits (), TupleGet uses var number, TuplePut is a stub"]
fn native_tuple_script() -> std::io::Result<()> {
    let rlib_info = find_loft_rlib();
    let entry = std::path::Path::new("tests/scripts/50-tuples.loft");
    let job = prepare_native_test(entry)?;
    let compiled = compile_native_job(&job, &rlib_info)?;
    assert!(compiled, "50-tuples.loft failed to compile under --native");
    run_native_job(&job)
}

/// N8a.3: native tuple-returning functions.
///
/// The same 50-tuples.loft script will include a tuple-returning function once
/// N8a.3 is implemented.  This is a placeholder: un-ignored together with
/// native_tuple_script when the updated script passes.
#[test]
#[ignore = "N8a.3: tuple function return not yet added to 50-tuples.loft"]
fn native_tuple_return_script() -> std::io::Result<()> {
    let rlib_info = find_loft_rlib();
    let entry = std::path::Path::new("tests/scripts/50-tuples.loft");
    let job = prepare_native_test(entry)?;
    let compiled = compile_native_job(&job, &rlib_info)?;
    assert!(
        compiled,
        "50-tuples.loft (with tuple return) failed to compile under --native"
    );
    run_native_job(&job)
}
