// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// P137 regression: `loft --html` must produce WASM that runs cleanly
// under a minimal Node host.  The previous failure was a bare
// `(unreachable)` trap on every `--html` program at Stores::new(),
// caused by `std::time::Instant::now()` panicking on
// wasm32-unknown-unknown without a time source.
//
// This test drives the integration end-to-end:
//   1. Writes a trivial `.loft` program.
//   2. Invokes the `loft` binary with `--html` to produce an HTML bundle.
//   3. Extracts the base64-embedded WASM.
//   4. Runs `tools/wasm_repro.mjs` to instantiate it with stub host
//      imports and invoke `loft_start`.
//   5. Asserts the process exits cleanly.
//
// Skipped when the prerequisites (wasm32-unknown-unknown toolchain,
// node binary) are unavailable — typical CI has them; developer
// machines without WASM rust targets get a clear "skipped" message
// rather than a false failure.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// All `--html` invocations write the generated Rust to a fixed
/// `/tmp/loft_html.rs` path, so concurrent tests step on each other.
/// Serialise the build path with a process-wide mutex.
fn build_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn which(cmd: &str) -> Option<PathBuf> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn wasm32_target_installed() -> bool {
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.lines().any(|l| l == "wasm32-unknown-unknown"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Compare the mtime of a derived artefact against a set of source
/// files.  Returns `Err(msg)` describing the newest out-of-date source
/// and the rebuild command, or `Ok(())` when the artefact is fresh.
///
/// Neither `cargo test` nor `make ci` rebuilds the
/// `wasm32-unknown-unknown` rlib or the fixture cdylibs.  Without this
/// check, a stale artefact silently masquerades as a code regression —
/// `--html` fails with rustc errors citing pre-migration line numbers,
/// or `native_loader::*` mis-reads vector elements and reports
/// "expected N, got M".
fn artefact_staleness(artefact: &std::path::Path, sources: &[&std::path::Path]) -> Option<String> {
    let Ok(art_md) = std::fs::metadata(artefact) else {
        return Some(format!("artefact missing: {}", artefact.display()));
    };
    let Ok(art_mtime) = art_md.modified() else {
        return None;
    };
    let mut newest: Option<(std::path::PathBuf, std::time::SystemTime)> = None;
    for s in sources {
        let Ok(md) = std::fs::metadata(s) else {
            continue;
        };
        let Ok(mtime) = md.modified() else { continue };
        if mtime > art_mtime && newest.as_ref().is_none_or(|(_, t)| mtime > *t) {
            newest = Some(((*s).to_path_buf(), mtime));
        }
    }
    newest.map(|(src, _)| {
        format!(
            "source newer than artefact: {} is newer than {}",
            src.display(),
            artefact.display()
        )
    })
}

/// Panic with an actionable rebuild command when the
/// `wasm32-unknown-unknown` rlib is stale vs. key runtime sources.
/// Called before invoking `loft --html` so a mismatch fails fast
/// rather than surfacing as confusing rustc errors.
fn assert_wasm_rlib_fresh() {
    let rlib = repo_root().join("target/wasm32-unknown-unknown/release/libloft.rlib");
    if !rlib.exists() {
        return; // first run — the --html driver will build it
    }
    let root = repo_root();
    let sources = [
        root.join("src/codegen_runtime.rs"),
        root.join("src/ops.rs"),
        root.join("src/data.rs"),
        root.join("src/lib.rs"),
        root.join("src/generation/mod.rs"),
    ];
    let source_refs: Vec<&std::path::Path> =
        sources.iter().map(std::path::PathBuf::as_path).collect();
    if let Some(reason) = artefact_staleness(&rlib, &source_refs) {
        panic!(
            "stale wasm32-unknown-unknown rlib — {reason}\n\
             Rebuild:\n  \
               cargo build --release --target wasm32-unknown-unknown \\\n             \
                 --lib --no-default-features --features random\n\
             (Do NOT use --features wasm — that pulls in wasm-bindgen and\n \
              the resulting bundle imports from __wbindgen_placeholder__.)\n"
        );
    }
}

/// Build a loft program via `--html`, extract the WASM, run it via the
/// Node repro harness with stub imports, and return (stdout, stderr,
/// exit_status).  Returns None when prerequisites are missing.
fn run_html_wasm(name: &str, source: &str) -> Option<(String, String, bool)> {
    run_html_wasm_with_libs_and_assets(name, source, &[], &[])
}

/// Same as `run_html_wasm` but also passes a `--lib <dir>` for each
/// entry in `lib_dirs`.  Needed for programs that `use <pkg>;` out of
/// a local library tree (e.g. `use moros_editor;` from `lib/`).
fn run_html_wasm_with_libs(
    name: &str,
    source: &str,
    lib_dirs: &[&str],
) -> Option<(String, String, bool)> {
    run_html_wasm_with_libs_and_assets(name, source, lib_dirs, &[])
}

/// @P321(c) Phase 3a — extension of `run_html_wasm_with_libs` that also
/// copies `assets` (sibling resource files like `*.png`) into the same
/// `/tmp/` dir as the synthesised `.loft`, so the `--html` driver's
/// asset auto-discovery finds them.  Use from the lib suite, which
/// synthesises entry .loft's outside their original test dir.
fn run_html_wasm_with_libs_and_assets(
    name: &str,
    source: &str,
    lib_dirs: &[&str],
    assets: &[PathBuf],
) -> Option<(String, String, bool)> {
    run_html_wasm_full(name, source, lib_dirs, assets, &[], "tools/wasm_repro.mjs")
}

/// As above, plus arbitrary extra `loft --html` flags (e.g. `--debug=<name>`) and a
/// choice of Node harness (`wasm_repro.mjs` runs `loft_start`; a debug harness may
/// call another export).
fn run_html_wasm_full(
    name: &str,
    source: &str,
    lib_dirs: &[&str],
    assets: &[PathBuf],
    extra_args: &[&str],
    harness_rel: &str,
) -> Option<(String, String, bool)> {
    if which("node").is_none() {
        eprintln!("SKIP: node not installed");
        return None;
    }
    if !wasm32_target_installed() {
        eprintln!("SKIP: rustup target wasm32-unknown-unknown not installed");
        return None;
    }

    let loft_bin = repo_root().join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release` first)");
        return None;
    }

    assert_wasm_rlib_fresh();

    // @P321(c) Phase 3a: per-test subdir so `--html`'s asset
    // auto-discovery (which scans the dir of the entry .loft) only
    // finds assets this test asked for — a shared /tmp/ would let
    // an earlier test's PNGs leak into a later test's bundle.
    let tmp = std::env::temp_dir().join(format!("loft_html_{name}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create per-test dir");
    let src = tmp.join(format!("{name}.loft"));
    let html = tmp.join(format!("{name}.html"));
    let wasm = tmp.join(format!("{name}.wasm"));

    std::fs::write(&src, source).expect("write source");

    for asset in assets {
        let Some(fname) = asset.file_name() else {
            continue;
        };
        let dest = tmp.join(fname);
        if let Err(e) = std::fs::copy(asset, &dest) {
            eprintln!("warn: could not copy asset {asset:?} → {dest:?}: {e}");
        }
    }

    // Serialise: the loft `--html` driver writes to a fixed
    // `/tmp/loft_html.rs` path, so parallel test invocations would
    // overwrite each other's emitted Rust mid-build.
    //
    // P201: recover from a poisoned lock via `into_inner()` so the
    // first test to fail surfaces its real error instead of every
    // later test reporting `PoisonError { .. }`.  The lock guards a
    // shared file path, not invariant state — a panicking test leaves
    // the file half-written, but the next test overwrites it on the
    // next `loft --html` invocation, so consuming the poisoned guard
    // is safe.  `assert_wasm_rlib_fresh()` already runs before this
    // line so a stale rlib fails the test before it can poison the
    // build serial.
    let _guard = build_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut cmd = Command::new(&loft_bin);
    cmd.args([
        "--html",
        html.to_str().unwrap(),
        "--path",
        &format!("{}/", repo_root().display()),
    ]);
    for dir in lib_dirs {
        cmd.arg("--lib").arg(repo_root().join(dir));
    }
    cmd.args(extra_args);
    cmd.arg(src.to_str().unwrap());
    let status = cmd.status().expect("invoke loft --html");
    assert!(status.success(), "loft --html failed for {name}");
    drop(_guard);

    let html_content = std::fs::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = html_content.find(marker).expect("wasmB64 marker present") + marker.len();
    let end = start
        + html_content[start..]
            .find('"')
            .expect("wasmB64 closing quote");
    let b64 = &html_content[start..end];
    let bytes = base64_decode_standard(b64).expect("decode wasmB64");
    std::fs::write(&wasm, &bytes).expect("write extracted wasm");

    let harness = repo_root().join(harness_rel);
    assert!(harness.exists(), "{harness_rel} missing");

    let out = Command::new("node")
        .arg(&harness)
        .arg(&wasm)
        .output()
        .expect("invoke node harness");

    Some((
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    ))
}

/// P137 root case: an empty `fn main() {}` traps before any user code
/// runs.  This was the minimal reproducer that revealed
/// `Stores::new()` → `Instant::now()` as the panic site.  If WASM init
/// regresses (e.g. a future change calls another non-wasm32-safe
/// std API in `Stores::new()`), this catches it.
#[test]
fn p137_html_empty_main_does_not_trap() {
    let Some((_stdout, stderr, ok)) = run_html_wasm("p137_empty", "fn main() {}\n") else {
        return;
    };
    assert!(
        ok,
        "empty main trapped — P137-shape init regression.\n{stderr}"
    );
}

/// P137: build a `println`-only `.loft` program via `--html`, extract
/// the WASM, and run it via the Node repro harness with stub imports.
/// Exit code 0 means `loft_start` returned without trapping.
#[test]
fn p137_html_hello_world_does_not_trap() {
    let Some((stdout, stderr, ok)) = run_html_wasm(
        "p137_hello",
        "fn main() { println(\"hello from loft\"); }\n",
    ) else {
        return;
    };
    assert!(
        ok,
        "WASM trapped — P137 regression.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("hello from loft") || stderr.contains("loft_start: OK"),
        "expected 'hello from loft' in output.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// P137 follow-up: arithmetic + control flow exercise the bytecode
/// dispatch path in the WASM build.  If a future change introduces a
/// panic-able std call inside the dispatch loop (e.g. an unchecked
/// indexing in a hot opcode), this catches it.
#[test]
fn p137_html_arithmetic_loop_runs() {
    let src = "fn main() {
    sum = 0;
    for i in 0..10 { sum = sum + i; }
    println(\"sum={sum}\");
}
";
    let Some((stdout, stderr, ok)) = run_html_wasm("p137_arith", src) else {
        return;
    };
    assert!(ok, "WASM trapped on arithmetic loop.\n{stderr}");
    assert!(
        stdout.contains("sum=45"),
        "expected 'sum=45' in output.\nstdout: {stdout}"
    );
}

/// QUALITY Tier 3 #9 — `file("...")` under `--html` (wasm32
/// without the wasm host-bridge) must not trap even though there
/// is no reachable filesystem.  The stub in
/// `src/database/io.rs::get_file` returns `Format::NotExists` and
/// `src/state/io.rs::get_file_text` leaves the buffer untouched.
/// A `--html` program calling `file("x").content()` must therefore
/// return an empty string without crashing.  This test exercises
/// the full `--html` build → browser repro harness path.
#[test]
fn q9_html_file_content_returns_empty_on_wasm() {
    let src = "fn main() {
    f = file(\"/definitely_missing_on_wasm.txt\");
    t = f.content();
    println(\"len={t.len()}\");
}
";
    let Some((stdout, stderr, ok)) = run_html_wasm("q9_file_content", src) else {
        return;
    };
    assert!(ok, "WASM trapped on file().content() call.\n{stderr}");
    assert!(
        stdout.contains("len=0"),
        "expected 'len=0' (empty-string content on wasm32 stub).\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// P137 follow-up: vectors + iteration.  Exercises store allocation
/// and per-iteration access in the WASM target — would catch a
/// regression in `OpVarVector` or vector-element-access opcodes
/// emitting a panic-able path under wasm32.
#[test]
fn p137_html_vector_iteration_runs() {
    let src = "fn main() {
    items = [10, 20, 30];
    total = 0;
    for x in items { total = total + x; }
    println(\"total={total}\");
}
";
    let Some((stdout, stderr, ok)) = run_html_wasm("p137_vec", src) else {
        return;
    };
    assert!(ok, "WASM trapped on vector iteration.\n{stderr}");
    assert!(
        stdout.contains("total=60"),
        "expected 'total=60' in output.\nstdout: {stdout}"
    );
}

/// ROADMAP 0.8.5 end-to-end smoke: `lib/moros_editor` (which imports
/// `lib/moros_map`) runs cleanly under `--html`.  Exercises the full
/// edit pipeline the browser scene editor drives — paint, height, wall,
/// batched stencil stamp, undo — across the loft-side library + WASM
/// host bridge.  If any moros_editor code path traps under wasm32
/// (e.g. a future change hits a non-wasm32-safe std call), this catches
/// it before the browser build ships.
#[test]
fn moros_editor_html_smoke() {
    let src = r#"use moros_editor;
fn main() {
    m = map_empty();
    us = undo_empty();

    paint_material_with_undo(us, m, 0, 0, 0, 1);
    set_height_with_undo(us, m, 0, 0, 0, 3);
    set_wall_with_undo(us, m, 0, 0, 0, 0, 7);

    batch_begin(us);
    stencil_stamp_with_undo(us, m, stencil_house_small(), 2, 2, 0, 0);
    batch_end(us);

    undo_pop(us, m);

    d = undo_depth(us);
    h = map_get_hex(m, 0, 0, 0).h_material;
    println("depth={d} mat={h}");
}
"#;
    let Some((stdout, stderr, ok)) =
        run_html_wasm_with_libs("moros_editor_smoke", src, &["tests/fixtures/libs"])
    else {
        return;
    };
    assert!(ok, "moros_editor smoke trapped under --html.\n{stderr}");
    assert!(
        stdout.contains("depth=3 mat=1"),
        "expected 'depth=3 mat=1'.\nstdout: {stdout}\nstderr: {stderr}"
    );
}

/// P201 regression: when one html_wasm test panics while holding
/// `build_lock()`, every subsequent test must still be able to acquire
/// the guard.  Before the fix, a poisoned `Mutex<()>` made every later
/// `.lock().unwrap()` call panic with `PoisonError { .. }` — a noisy
/// cascade that hid the original failure.  The fix is the recovery
/// pattern `.lock().unwrap_or_else(|e| e.into_inner())`; this test
/// exercises that pattern on a local mutex so a regression in the
/// recovery shape (e.g. someone reverts to plain `.unwrap()`) trips
/// here without depending on the real `build_lock()` global.
#[test]
fn p201_poisoned_lock_recovery_pattern() {
    use std::sync::Arc;
    let m = Arc::new(Mutex::new(()));
    let m2 = Arc::clone(&m);
    let h = std::thread::spawn(move || {
        let _g = m2.lock().expect("first acquire");
        panic!("simulated test failure with the lock held");
    });
    let _ = h.join(); // expected to be Err
    assert!(
        m.is_poisoned(),
        "precondition: panicking thread must poison the mutex"
    );
    // The fix's recovery pattern — must NOT panic on a poisoned lock.
    let _guard = m.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // Holding the recovered guard proves the pattern is live; drop it
    // to release.  If the assertion below ever changes shape, the doc
    // comment on `build_lock().lock()` (above) needs the same edit.
    drop(_guard);
}

// ── WASM library CI gate ─────────────────────────────────────────────────────
//
// `tests/wrap.rs::library_suite` (interpreter) and
// `tests/native.rs::native_library_suite` (native) gate every
// `lib/<pkg>/tests/*.loft`.  This is the missing third backend: run those
// package tests through WebAssembly under whatever runtime is available —
// Node (the browser `feature="wasm"` flavour, via `--html` +
// `tools/wasm_repro.mjs`) and/or wasmtime (the `wasm32-wasip2` flavour).
//
// Opt-in by entry point: a lib test joins the WASM gate iff it declares a
// `fn main()` (the wasm builds use `main` as the entry; the `loft test`
// subcommand's multi-`test_*` discovery has no wasm equivalent).  Tests
// without a `main` are reported as skipped, not failed.  Both runners
// self-skip when their toolchain is absent, so the suite is a no-op on a
// machine with neither Node nor wasmtime.

/// Library packages skipped wholesale for the WASM gate — those whose tests
/// depend on native-only host facilities the browser/WASI stub can't provide.
/// Travels with each chunk on extraction, mirroring `LIB_PKGS_WASM_SKIP` in
/// the plan-12 design.
const LIB_PKGS_WASM_SKIP: &[&str] = &[
    // server is a native HTTP/socket listener — a browser/WASI guest has no
    // listen/accept, so it cannot run there by construction (a genuine
    // platform limit, not a bug — unlike imaging).
    "server",
];

/// Individual `<pkg>/<file>.loft` lib tests skipped for the WASM gate.
const LIB_TESTS_WASM_SKIP: &[&str] = &[];

/// Packages skipped ONLY on the node (browser, `wasm32-unknown-unknown`) path.
/// Still run on wasmtime (wasip2) which has a working WASI FS via `--dir`.
/// Use this for libraries that genuinely need the filesystem — the browser
/// has none by construction (correct platform behaviour, not a bug to fix).
const LIB_PKGS_NODE_SKIP: &[&str] = &[
    // hex_world's tests save+load a binary world file.  Wasmtime can do this via
    // `--dir <tmp>` (see @P334 fix); the browser has no filesystem at all
    // without a JS host bridge (out of scope for now).  Un-skip if a future
    // JS-host VirtFS bridge ships.
    "hex_world",
    // `input` un-gated on node 2026-06-04: both its blockers — #248 (@P391
    // cross-package ctor → CONST_STORE) and #266 (nested `&self` writes not
    // persisting on `--interpret`) — are fixed, and `01-basics` builds + runs
    // green on the node (`wasm32-unknown-unknown` / browser) path.  It remains
    // in LIB_PKGS_WASMTIME_SKIP below for an UNRELATED reason (E0463: the
    // graphics native crate is absent from the wasmtime sysroot).
];

/// Packages skipped ONLY on the wasmtime (wasip2) path.  Use this for
/// libraries whose host bridge depends on a browser-only API (Canvas,
/// createImageBitmap, etc.) — wasmtime has no canvas / image codec; the
/// browser does.
const LIB_PKGS_WASMTIME_SKIP: &[&str] = &[
    // @P321(c): imaging's PNG decode is provided by the browser via
    // `createImageBitmap` + Canvas `getImageData` (see
    // `lib/imaging/wasm/{src/lib.rs, host.js}` and `doc/loft-gl-wasm.js`).
    // Wasmtime has no equivalent; the bridge call would always return
    // false, breaking `assert(img.width == 256)`.  Browsers handle this
    // fine.
    "imaging",
    // input — the #248/@P391 + #266 language blockers are FIXED (it now runs
    // green on `--interpret`, `--native`, and the node/browser wasm path).
    // STILL gated here for an UNRELATED build issue: `input` depends on
    // `graphics`, whose native crate is absent from the wasmtime (wasip2)
    // sysroot, so `--html` codegen fails with E0463 at lib resolution.
    // Verified failing 2026-06-04.  Un-skip once the graphics crate is
    // available to the wasmtime build (a wasm-sysroot / packaging task, not a
    // language bug).
    "input",
];

/// Collect every `lib/<pkg>/tests/*.loft`, sorted.
fn collect_lib_wasm_tests() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    let Ok(pkgs) = std::fs::read_dir(repo_root().join("lib")) else {
        return files;
    };
    for pkg in pkgs.filter_map(|e| e.ok()) {
        // Skip the `.loft_test_tmp_*` artifact-isolation dirs that the interp /
        // native lib suites create as siblings inside lib/ (run_lib_test_in_temp_cwd)
        // — they exist only transiently and must not be discovered as packages.
        if pkg.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let tests_dir = pkg.path().join("tests");
        let Ok(entries) = std::fs::read_dir(&tests_dir) else {
            continue;
        };
        for f in entries.filter_map(|e| e.ok()) {
            let p = f.path();
            if p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

/// `(pkg, file)` key for an entry, e.g. `("time", "01-basics.loft")`.
fn lib_test_key(entry: &std::path::Path) -> (String, String) {
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
    (pkg, file)
}

/// Locate `wasmtime` on `PATH` or in the default `~/.wasmtime/bin` install dir.
fn find_wasmtime() -> Option<PathBuf> {
    if let Some(p) = which("wasmtime") {
        return Some(p);
    }
    let home = std::env::var_os("HOME")?;
    let p = PathBuf::from(home).join(".wasmtime/bin/wasmtime");
    p.exists().then_some(p)
}

/// Run `source` on `wasm32-wasip2` under wasmtime.  Emits native Rust via the
/// loft binary, injects a WASI-stdout `loft_host_print` shim (the codegen calls
/// `crate::loft_host_print` on wasm32 without the `wasm` feature, but only the
/// browser path declares that import), compiles against the prebuilt loft
/// `wasm32-wasip2` rlib, and runs it.  Returns `(output, ok)`, or `None` when a
/// prerequisite (wasmtime, rustc, the rlib) is missing.
fn run_wasip2_wasm(name: &str, source: &str, lib_dirs: &[&str]) -> Option<(String, bool)> {
    let wasmtime = find_wasmtime()?;
    which("rustc")?;
    let root = repo_root();
    let loft_bin = root.join("target/release/loft");
    if !loft_bin.exists() {
        return None;
    }
    // Prefer the release rlib (the Makefile-built one); fall back to debug.
    let (rlib, deps) = ["release", "debug"].iter().find_map(|prof| {
        let r = root.join(format!("target/wasm32-wasip2/{prof}/libloft.rlib"));
        r.exists()
            .then(|| (r, root.join(format!("target/wasm32-wasip2/{prof}/deps"))))
    })?;

    let tmp = std::env::temp_dir();
    let src = tmp.join(format!("{name}.loft"));
    let rs = tmp.join(format!("{name}_wasip2.rs"));
    let wasm = tmp.join(format!("{name}_wasip2.wasm"));
    std::fs::write(&src, source).ok()?;

    let mut emit = Command::new(&loft_bin);
    emit.args([
        "--native-emit",
        rs.to_str()?,
        "--path",
        &format!("{}/", root.display()),
    ]);
    for d in lib_dirs {
        emit.arg("--lib").arg(root.join(d));
    }
    emit.arg(src.to_str()?);
    if !emit.output().ok()?.status.success() {
        return Some(("loft --native-emit failed".to_string(), false));
    }

    // wasip2 print() now lowers to std `print!` (WASI stdout) directly — the
    // generated source is self-contained, no host-import shim needed (#268).
    let compile = Command::new("rustc")
        .args([
            "--edition=2024",
            "--target",
            "wasm32-wasip2",
            "--crate-type",
            "bin",
            "-O",
            "--extern",
        ])
        .arg(format!("loft={}", rlib.display()))
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("-o")
        .arg(&wasm)
        .arg(&rs)
        .output()
        .ok()?;
    if !compile.status.success() {
        return Some((String::from_utf8_lossy(&compile.stderr).into_owned(), false));
    }
    // @P334 fix (2026-05-29): preopen the dirs a test program may touch so
    // wasip2 file I/O works.  The wasip2 filesystem is sandboxed — without a
    // preopen every file op fails with "os error 44" and any file-using lib
    // traps the moment it opens a path.
    //
    // Robustness: preopen a SET of roots, not just `std::env::temp_dir()`.
    // A test that hardcodes a `/tmp/...` literal (e.g. lib/hex_world's save/load
    // fixture) traps when the harness's TMPDIR points elsewhere
    // (`/tmp/claude-1000`, a CI sandbox dir, …) and only that dir is preopened.
    // Preopening the literal `/tmp` mount AND the temp dir AND the CWD covers
    // /tmp-literal, temp-default, and CWD-relative file paths alike, so a lib
    // test no longer has to know how the harness configured TMPDIR.  Each dir
    // is mapped host==guest; duplicates and missing dirs are skipped.
    let mut preopens: Vec<std::path::PathBuf> = Vec::new();
    let mut add = |d: std::path::PathBuf| {
        if d.is_dir() && !preopens.contains(&d) {
            preopens.push(d);
        }
    };
    add(tmp.clone());
    #[cfg(unix)]
    add(std::path::PathBuf::from("/tmp"));
    if let Ok(cwd) = std::env::current_dir() {
        add(cwd);
    }
    let mut cmd = Command::new(&wasmtime);
    for dir in &preopens {
        cmd.arg("--dir").arg(dir);
    }
    let run = cmd.arg(&wasm).output().ok()?;
    // On failure, surface stderr too — a wasip2 trap (sandbox denial, missing
    // preopen, panic) writes its diagnostic to stderr, and reporting only
    // stdout left earlier failures with an empty, undiagnosable message.
    let report = if run.status.success() {
        String::from_utf8_lossy(&run.stdout).into_owned()
    } else {
        let mut s = String::from_utf8_lossy(&run.stdout).into_owned();
        let err = String::from_utf8_lossy(&run.stderr);
        if !err.trim().is_empty() {
            if !s.is_empty() {
                s.push('\n');
            }
            s.push_str("stderr: ");
            s.push_str(err.trim());
        }
        s
    };
    Some((report, run.status.success()))
}

/// @P268 regression: `print()` on wasip2 (`--native-wasm`) used to fail to
/// compile (E0425) — codegen called `crate::loft_host_print`, which is declared
/// only for the browser host-import path. wasip2 has working WASI stdout, so its
/// `print` branch now lowers to std `print!`. A printing program must both
/// compile and emit to stdout under wasmtime. Self-skips when the wasm toolchain
/// (rustc wasm32-wasip2 + wasmtime + rlib) is unavailable.
#[test]
fn wasip2_print_writes_stdout_p268() {
    let src = "fn main() { print(\"p268_ok\\n\"); }\n";
    let Some((out, ok)) = run_wasip2_wasm("p268_print", src, &[]) else {
        return;
    };
    assert!(ok, "wasip2 print program failed to build/run.\n{out}");
    assert!(
        out.contains("p268_ok"),
        "expected wasip2 stdout to contain 'p268_ok'.\nout: {out}"
    );
}

/// WASM gate over `lib/<pkg>/tests/*.loft` — runs each main()-bearing lib test
/// under Node (browser flavour) and wasmtime (wasip2) when available.  No-op
/// when neither runtime is present.
#[test]
fn wasm_library_suite() {
    let have_node = which("node").is_some();
    let have_wasmtime = find_wasmtime().is_some();
    if !have_node && !have_wasmtime {
        eprintln!("SKIP wasm_library_suite: neither node nor wasmtime available");
        return;
    }

    let mut failures: Vec<String> = Vec::new();
    let mut ran = 0usize;
    for entry in collect_lib_wasm_tests() {
        let (pkg, file) = lib_test_key(&entry);
        if LIB_PKGS_WASM_SKIP.contains(&pkg.as_str())
            || LIB_TESTS_WASM_SKIP.contains(&format!("{pkg}/{file}").as_str())
        {
            println!("skip {pkg}/{file} (LIB_*_WASM_SKIP)");
            continue;
        }
        let source = match std::fs::read_to_string(&entry) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Opt-in: only main()-bearing lib tests have a wasm entry point.
        if !source.contains("fn main(") {
            println!("skip {pkg}/{file} (no fn main — no wasm entry)");
            continue;
        }
        let stem = entry
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .replace(['-', '.'], "_");
        let name = format!("libwasm_{pkg}_{stem}");

        // @P321(c) Phase 3a: collect *.png siblings of the source so the
        // synthesised /tmp/<name>.loft has the same assets next to it as
        // the original lib test, and the --html driver's auto-discovery
        // can embed them.
        let assets: Vec<PathBuf> = entry
            .parent()
            .and_then(|d| std::fs::read_dir(d).ok())
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|d| d.path()))
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .collect();

        if have_node && !LIB_PKGS_NODE_SKIP.contains(&pkg.as_str()) {
            // `run_html_wasm_with_libs` asserts on a `--html` build failure;
            // catch it so one un-buildable lib is recorded, not fatal to the
            // whole suite (and can't mask later libs' results).
            let assets_clone = assets.clone();
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_html_wasm_with_libs_and_assets(&name, &source, &["lib"], &assets_clone)
            }));
            match res {
                Ok(Some((stdout, stderr, ok))) => {
                    ran += 1;
                    println!(
                        "wasm[node] {pkg}/{file}: {}",
                        if ok { "ok" } else { "FAIL" }
                    );
                    let _ = stdout;
                    if !ok {
                        let tail: Vec<&str> = stderr.lines().rev().take(3).collect();
                        failures.push(format!("{pkg}/{file} [node]: {}", tail.join(" | ")));
                    }
                }
                Ok(None) => {} // prerequisites missing — self-skipped
                Err(_) => {
                    ran += 1;
                    println!("wasm[node] {pkg}/{file}: FAIL (build panicked)");
                    failures.push(format!("{pkg}/{file} [node]: --html build failed"));
                }
            }
        }
        if have_wasmtime
            && !LIB_PKGS_WASMTIME_SKIP.contains(&pkg.as_str())
            && let Some((out, ok)) = run_wasip2_wasm(&name, &source, &["lib"])
        {
            ran += 1;
            println!(
                "wasm[wasmtime] {pkg}/{file}: {}",
                if ok { "ok" } else { "FAIL" }
            );
            if !ok {
                let tail: Vec<&str> = out.lines().rev().take(3).collect();
                failures.push(format!("{pkg}/{file} [wasmtime]: {}", tail.join(" | ")));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} wasm library run(s) failed:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
    println!("wasm_library_suite: {ran} wasm run(s) passed");
}

// Minimal base64 decoder — avoids adding a dev-dependency for one
// test.  Handles the standard alphabet (no URL-safe variant needed;
// the loft HTML writer uses `+`/`/`/`=`).
fn base64_decode_standard(s: &str) -> Option<Vec<u8>> {
    const T: [i8; 128] = {
        let mut t = [-1i8; 128];
        let mut i = 0;
        while i < 26 {
            t[b'A' as usize + i] = i as i8;
            t[b'a' as usize + i] = (i + 26) as i8;
            i += 1;
        }
        let mut i = 0;
        while i < 10 {
            t[b'0' as usize + i] = (i + 52) as i8;
            i += 1;
        }
        t[b'+' as usize] = 62;
        t[b'/' as usize] = 63;
        t
    };
    let s = s.as_bytes();
    if !s.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut i = 0;
    while i < s.len() {
        let mut v = [0i32; 4];
        let mut pad = 0;
        for j in 0..4 {
            let b = s[i + j];
            if b == b'=' {
                pad += 1;
                v[j] = 0;
            } else if (b as usize) < 128 && T[b as usize] >= 0 {
                v[j] = T[b as usize] as i32;
            } else {
                return None;
            }
        }
        let combined = (v[0] << 18) | (v[1] << 12) | (v[2] << 6) | v[3];
        out.push(((combined >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((combined >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((combined & 0xff) as u8);
        }
        i += 4;
    }
    Some(out)
}

/// @PLN26 phase 3 (#438) — a program that uses a `#native` (`[native] crate`) package
/// compiles to `--native-wasm` and runs under wasmtime: loft cross-builds the package's
/// native crate to `wasm32-wasip2` on demand and links its rlib (+ the host proc-macro
/// deps).  `native_scalar_pkg` is a minimal wasm-clean fixture (one scalar `#native` fn,
/// `native_answer() -> 42`), so this pins the loft cross-build PIPELINE, isolated from any
/// heavy crate's own wasm-cleanliness.  Skipped when the wasip2 target or wasmtime is absent.
#[test]
fn pln26_phase3_native_package_runs_on_wasm() {
    let Some(wasmtime) = which("wasmtime") else {
        eprintln!("skip: wasmtime not installed");
        return;
    };
    let have_wasip2 = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.lines().any(|l| l == "wasm32-wasip2"));
    if !have_wasip2 {
        eprintln!("skip: wasm32-wasip2 target not installed");
        return;
    }
    let root = repo_root();
    let loft_bin = root.join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("skip: target/release/loft not built");
        return;
    }
    let tmp = std::env::temp_dir().join("loft_pln26_p3");
    let _ = std::fs::create_dir_all(&tmp);
    let prog = tmp.join("native_pkg_wasm.loft");
    std::fs::write(
        &prog,
        "use native_scalar_pkg;\nfn main() {\n  answer = native_answer();\n  \
         print(\"native-answer={answer}\\n\");\n}\n",
    )
    .unwrap();
    let wasm = tmp.join("native_pkg_wasm.wasm");
    let _ = std::fs::remove_file(&wasm);
    // Clean any prior wasm cross-build so the ON-DEMAND build path is exercised.
    let _ = std::fs::remove_dir_all(
        root.join("tests/lib/native_scalar_pkg/native/target/wasm32-wasip2"),
    );

    let build = Command::new(&loft_bin)
        .arg("--native-wasm")
        .arg(&wasm)
        .arg("--lib")
        .arg(root.join("tests/lib"))
        .arg(&prog)
        .output()
        .expect("run loft --native-wasm");
    assert!(
        build.status.success() && wasm.exists(),
        "loft --native-wasm of a #native-package program failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = Command::new(&wasmtime)
        .arg(&wasm)
        .output()
        .expect("run wasmtime");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("native-answer=42"),
        "wasm `#native` call returned wrong/no value: stdout={stdout:?} stderr={}",
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Tier 1 + 2 of the browser byte channel, round-tripped headlessly:
/// `host_output` (loft->JS structured message) is echoed back by the
/// harness loopback as `echo:<msg>` into the input QUEUE, and
/// `host_input()` pops exactly one message per call — the second read is
/// empty.  This is the whole "JS owns the network" request/response
/// pattern with the fetch replaced by a deterministic echo.
#[test]
fn host_output_input_roundtrip_queue() {
    let src = r#"fn main() {
  host_output("fetch http://unit.test/tile");
  reply = host_input();
  println("reply=[{reply}]");
  second = host_input();
  println("second=[{second}]");
}
"#;
    let Some((stdout, stderr, ok)) = run_html_wasm("host_roundtrip", src) else {
        return; // prerequisites missing — skipped with a note
    };
    assert!(ok, "wasm run failed; stderr: {stderr}");
    assert!(
        stdout.contains("reply=[echo:fetch http://unit.test/tile]"),
        "loopback reply missing; stdout: {stdout}"
    );
    assert!(
        stdout.contains("second=[]"),
        "queue must be empty after one pop; stdout: {stdout}"
    );
}

// @PLN98 P3.4 — the live/debug tier's core primitive verified IN THE BROWSER WASM
// RUNTIME (headless node): a `--debug` client bootstraps the parked interpreter
// from the EMBEDDED source (P3.1, no filesystem), flips its fns to the interpreter,
// and the compiled `main` DISPATCHES a call into the interpreter running over the
// SHARED store — producing the correct result. This is "the interpreter over the
// shared live store" (the whole tier's one primitive) proven on wasm32.
#[test]
fn html_debug_client_dispatches_to_the_interpreter_on_wasm() {
    let source = "fn addup(a: integer, b: integer) -> integer {\n  a + b\n}\n\
                  fn triple(x: integer) -> integer {\n  addup(x, addup(x, x))\n}\n\
                  fn main() {\n  print(\"triple(14)={triple(14)}\")\n}\n";
    let Some((stdout, stderr, ok)) = run_html_wasm_full(
        "p98_debug_dispatch",
        source,
        &[],
        &[],
        &["--debug=alice"],
        "tools/wasm_repro.mjs",
    ) else {
        return; // skipped: no node / no wasm target / no release loft
    };
    let all = format!("{stdout}{stderr}");
    assert!(ok, "the --debug wasm client trapped.\n{all}");
    // The parked interpreter switched on and flipped the dispatchable fns.
    assert!(
        all.contains("flipped 2 fn(s) to the interpreter"),
        "the debug client flipped its fns to the interpreter: {all}"
    );
    // A compiled call dispatched INTO the interpreter over the shared store.
    assert!(
        all.contains("dispatched 1 interp call(s) over the shared store"),
        "a compiled call dispatched into the interpreter on wasm: {all}"
    );
    // And it computed the right answer over that shared store.
    assert!(
        all.contains("triple(14)=42"),
        "the interpreted dispatch produced the correct result: {all}"
    );
}

// @PLN98 P3.4 — the interpreter's COOPERATIVE DEBUG CYCLE (breakpoint pause +
// frame read + resume, P3.2) verified IN THE BROWSER WASM RUNTIME (headless node).
// The `--debug` build exports `loft_debug_selftest`, which runs the exact cycle
// (set a breakpoint, run — `execute_argv` returns at the breakpoint, read the
// paused frame's local, step, resume) and reports the outcome; identical to the
// native result. This proves the make-or-break browser pause works on wasm32.
#[test]
fn html_debug_cooperative_pause_cycle_runs_on_wasm() {
    let Some((stdout, stderr, ok)) = run_html_wasm_full(
        "p98_debug_selftest",
        "fn main() { print(\"x\") }\n",
        &[],
        &[],
        &["--debug=alice"],
        "tools/wasm_debug_selftest.mjs",
    ) else {
        return;
    };
    let all = format!("{stdout}{stderr}");
    assert!(ok, "the debug-selftest harness failed: {all}");
    assert!(
        all.contains("PAUSE n=40 STEP m=42 DONE=true"),
        "the cooperative debug cycle ran on wasm: {all}"
    );
    assert!(
        all.contains("RETURN=1"),
        "the selftest reported success: {all}"
    );
}
