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
const NATIVE_SKIP: &[&str] = &[];

/// Script files that are known to fail in `--native` mode.
/// See PROBLEMS.md for issue numbers.
const SCRIPTS_NATIVE_SKIP: &[&str] = &[];

/// Locate `libloft.rlib` and its sibling deps directory for standalone `rustc` compilation.
///
/// Prefers `target/release/libloft.rlib` (clean single-version deps) over the debug
/// test binary's `deps/` directory (which may have multiple versions of the same crate,
/// causing rustc "multiple candidates" errors).  Falls back to debug if release is absent.
fn find_loft_rlib() -> Option<(PathBuf, PathBuf)> {
    let target_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent()?.parent()?.parent().map(|d| d.to_path_buf()))?;

    let release_rlib = target_dir.join("release").join("libloft.rlib");
    if release_rlib.exists() {
        let deps = target_dir.join("release").join("deps");
        return Some((release_rlib, deps));
    }

    let debug_deps = target_dir.join("debug").join("deps");
    let rlib = std::fs::read_dir(&debug_deps)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| {
            let s = e.file_name();
            let s = s.to_string_lossy();
            s.starts_with("libloft") && s.ends_with(".rlib")
        })
        .map(|e| e.path())?;
    Some((rlib, debug_deps))
}

/// Paths for one native compilation job.
struct NativeJob {
    stem: String,
    tmp_rs: PathBuf,
    binary: PathBuf,
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
    if !p.diagnostics.is_empty() {
        return Err(Error::from(std::io::ErrorKind::InvalidData));
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    let main_nr = p.data.def_nr("n_main");
    let entry_defs: Vec<u32> = if main_nr < end_def {
        vec![main_nr]
    } else {
        (start_def..end_def).collect()
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
        };
        out.output_native_reachable(&mut buf, start_def, end_def, &entry_defs)?;
    }

    // Only write the .rs file when the content has changed.  This keeps the file's
    // modification time stable across runs where the loft source hasn't changed,
    // which allows the binary mtime cache in compile_native_job to remain valid.
    let tmp_rs = std::env::temp_dir().join(format!("loft_native_{stem}.rs"));
    let existing = std::fs::read(&tmp_rs).unwrap_or_default();
    if existing != buf {
        std::fs::write(&tmp_rs, &buf)?;
    }

    let binary = std::env::temp_dir().join(format!("loft_native_{stem}_bin"));
    Ok(NativeJob {
        stem,
        tmp_rs,
        binary,
    })
}

/// Return true if the cached binary is newer than both the `.rs` source and the rlib.
/// When true, `compile_native_job` skips the rustc invocation.
fn binary_cache_valid(job: &NativeJob, rlib_info: &Option<(PathBuf, PathBuf)>) -> bool {
    let bin_mtime = match std::fs::metadata(&job.binary).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return false,
    };
    if let Ok(t) = std::fs::metadata(&job.tmp_rs).and_then(|m| m.modified()) {
        if bin_mtime < t {
            return false;
        }
    }
    if let Some((rlib, _)) = rlib_info {
        if let Ok(t) = std::fs::metadata(rlib).and_then(|m| m.modified()) {
            if bin_mtime < t {
                return false;
            }
        }
    }
    true
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
        return Err(Error::from(std::io::ErrorKind::Other));
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
    if let Some(e) = first_err {
        return Err(e);
    }

    // Phase 3: run all compiled binaries in parallel.
    let ready: Vec<&NativeJob> = jobs
        .iter()
        .zip(compiled.iter())
        .filter(|(_, ok)| **ok)
        .map(|(job, _)| job)
        .collect();
    let run_errors: Vec<std::io::Error> = std::thread::scope(|s| {
        ready
            .iter()
            .map(|job| s.spawn(|| run_native_job(job)))
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err(Error::from(std::io::ErrorKind::Other)))
                    .err()
            })
            .collect()
    });
    if let Some(e) = run_errors.into_iter().next() {
        return Err(e);
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
        jobs.push(prepare_native_test(&entry)?);
    }
    run_native_jobs(jobs, rlib_info)
}
