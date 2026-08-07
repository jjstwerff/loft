// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Shared helpers for integration test binaries.
//!
//! Each item is `#[allow(dead_code)]` because this module is pulled
//! into multiple test binaries via `mod common;`, and not every
//! binary uses every helper.  Without the allow, binaries that don't
//! consume a given helper produce a warning that turns into a CI
//! failure under `-D warnings`.

#[allow(dead_code)]
pub mod cross_mode;

/// How much to stretch a test's wall-clock deadline, because the machine is shared.
///
/// A deadline is an UPPER bound: a fast run returns early and pays nothing for a
/// generous budget, while a tight one turns ordinary contention into a failure.  So the
/// question is not "how long should this take" but "am I sharing the box".
///
/// `CI` alone misses the case that actually bites: a LOCAL full-suite run, where dozens
/// of tests share the CPU — measured at 61.6 s against a 60 s budget for a browser test
/// that takes 25 s alone.  `NEXTEST` covers it, since the harness is exactly what runs
/// tests in parallel.  A hand-run test binary keeps the tight budget, so iterating on one
/// test still fails fast.
#[allow(dead_code)]
#[must_use]
pub fn deadline_scale() -> u64 {
    let shared = std::env::var_os("CI").is_some() || std::env::var_os("NEXTEST").is_some();
    if shared { 3 } else { 1 }
}

/// A server-test port, offset by `LOFT_TEST_PORT_OFFSET` (default 0).  The engine-host /
/// wasm-relay tests bind FIXED ports; two suites run at once — e.g. two agents in sibling
/// checkouts (`loft` and `loft2`) — collide on them and flake.  `find_problems.sh` exports a
/// distinct offset per checkout so their port ranges never overlap.  A plain `cargo test` (no
/// offset) keeps the base ports.
#[allow(dead_code)]
pub fn test_port(base: u16) -> u16 {
    let offset = std::env::var("LOFT_TEST_PORT_OFFSET")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    base.saturating_add(offset)
}

use loft::data::Data;
use loft::database::Stores;
use loft::parser::Parser;
use std::path::PathBuf;
use std::sync::OnceLock;

/// On Windows MSVC, the build-script output dirs holding native import libraries
/// (e.g. `windows.0.48.5.lib` from `windows-sys`) must be passed to a hand-driven
/// `rustc` as `-L` paths — cargo adds them via `cargo:rustc-link-search` but a
/// test that links a cdylib by hand does not, so the link fails
/// `LNK1181: cannot open input file …`.  Mirrors `native_lib::native_lib_search_dirs`
/// and the `--native` test runner.  Empty (a no-op) off Windows.
///
/// `rlib` is `target/<profile>/libloft.rlib` or `target/<profile>/deps/libloft-*.rlib`.
#[allow(dead_code)]
#[cfg(not(windows))]
pub fn native_lib_search_dirs(_rlib: &std::path::Path) -> Vec<PathBuf> {
    Vec::new()
}

#[allow(dead_code)]
#[cfg(windows)]
pub fn native_lib_search_dirs(rlib: &std::path::Path) -> Vec<PathBuf> {
    // Walk up to the profile dir (release/ or debug/), then scan `build/<crate>-<hash>/`.
    let Some(profile_dir) = rlib.parent().and_then(|p| {
        if p.file_name().is_some_and(|n| n == "deps") {
            p.parent()
        } else {
            Some(p)
        }
    }) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(profile_dir.join("build")) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let build_entry = entry.path();
        // `out/` and its immediate subdirs (libs generated into OUT_DIR).
        let out = build_entry.join("out");
        if out.is_dir() {
            dirs.push(out.clone());
            if let Ok(subs) = std::fs::read_dir(&out) {
                dirs.extend(
                    subs.filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_dir()),
                );
            }
        }
        // `cargo:rustc-link-search` directives cached in `build/<crate>-<hash>/output`
        // (e.g. `windows_x86_64_msvc` ships its `.lib` inside the registry package).
        if let Ok(content) = std::fs::read_to_string(build_entry.join("output")) {
            for line in content.lines() {
                if let Some(p) = line
                    .strip_prefix("cargo:rustc-link-search=native=")
                    .or_else(|| line.strip_prefix("cargo:rustc-link-search="))
                {
                    let p = PathBuf::from(p);
                    if p.is_dir() && !dirs.contains(&p) {
                        dirs.push(p);
                    }
                }
            }
        }
    }
    dirs
}

/// Count the warnings **loft itself** raised about `script_name` (a `.loft` file name).
///
/// Use this instead of counting every `warning:` line whenever a test spawns the loft binary:
/// a `--native` run relays rustc's whole stderr verbatim (`src/main.rs`, "Relay rustc's own
/// output"), and rustc opens its diagnostics with the same `warning:` header, so a bare count
/// also counts the toolchain's.  That difference is invisible on Linux — where the generated
/// crate compiles clean — and shows up only on another host: on `windows-latest` an MSVC
/// linker warning plus rustc's `warning: N warnings emitted` summary added two phantom
/// warnings and failed a test that passes everywhere else.
///
/// Attribution keys on the location every loft diagnostic carries — `  --> <script>:<line>:<col>`
/// on the line below the header (pretty, the default) or ` at <script>:<line>:<col>` on the
/// header itself (compact, `LOFT_ERRORS=compact`).  rustc's diagnostics point at the generated
/// `loft_native_<pid>.rs`, or carry no location at all, so neither is ever counted.
#[allow(dead_code)]
pub fn loft_warnings(stderr: &str, script_name: &str) -> usize {
    let lines: Vec<&str> = stderr.lines().collect();
    let mut count = 0;
    for (i, line) in lines.iter().enumerate() {
        // Compact: `Warning[code]: <message> at <file>:<line>:<col>` — one line, self-locating.
        if line.starts_with("Warning") && line.contains(script_name) {
            count += 1;
            continue;
        }
        // Pretty: `warning[code]: <message>` followed by the `-->` location line.
        let header = line.starts_with("warning:") || line.starts_with("warning[");
        let points_at_script = lines
            .get(i + 1)
            .is_some_and(|loc| loc.trim_start().starts_with("-->") && loc.contains(script_name));
        if header && points_at_script {
            count += 1;
        }
    }
    count
}

/// Record environmental skips — tests that PASSED-by-skipping for a
/// toolchain/OS reason rather than a code reason — to a side-channel ledger, so
/// they survive nextest's suppression of successful output.
///
/// Reach for this from any test that self-skips on a missing toolchain.  A skip
/// and a pass are indistinguishable in a summary, so without the ledger a green
/// run hides reduced coverage: the regression of whatever the test guards looks
/// exactly like a clean run.  The CI step `Surface environmental test skips`
/// drains the ledger into annotations and a job summary, which is what turns
/// "more tests skip than yesterday" into something visible.
///
/// No-op unless `LOFT_SKIP_LEDGER` (a directory) is set, so local runs are
/// unaffected.  Each call gets its own file — pid-named for the cross-process
/// case (nextest runs one process per test) and counter-suffixed for the
/// same-process one (`cargo test` runs a binary's tests as threads), so a
/// second caller can never truncate the first one's record.
#[allow(dead_code)]
pub fn record_env_skips(suite: &str, reason: &str, skips: &[(String, String)]) {
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let Ok(dir) = std::env::var("LOFT_SKIP_LEDGER") else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::path::Path::new(&dir).join(format!("{suite}-{}-{seq}.tsv", std::process::id()));
    let body: String = skips
        .iter()
        .map(|(entry, detail)| {
            let clean = |s: &str| s.replace(['\t', '\n'], " ");
            format!("{suite}\t{reason}\t{}\t{}\n", clean(entry), clean(detail))
        })
        .collect();
    let _ = std::fs::write(path, body);
}

#[allow(dead_code)]
static DEFAULT_PARSED: OnceLock<(Data, Stores)> = OnceLock::new();

/// Parse the default library once per test binary and cache the result.
/// Each test clones the schema cheaply instead of re-parsing three files.
#[allow(dead_code)]
pub fn cached_default() -> (Data, Stores) {
    let (data, db) = DEFAULT_PARSED.get_or_init(|| {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).unwrap();
        (p.data, p.database)
    });
    (data.clone(), db.clone())
}
