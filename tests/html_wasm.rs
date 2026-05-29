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

    let harness = repo_root().join("tools/wasm_repro.mjs");
    assert!(harness.exists(), "tools/wasm_repro.mjs missing");

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
    let Some((stdout, stderr, ok)) = run_html_wasm_with_libs("moros_editor_smoke", src, &["lib"])
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
    // @P321c (OPEN): imaging's store-MUTATING `#native` load_png/save_png has
    // no working codegen marshalling — the `--html` build emits broken Rust
    // (`n_load_png` defined multiple times + E0308).  The native gate skips it
    // for the same @P321c; this WASM gate confirms the browser path is broken
    // too.  NOTE: imaging SHOULD run in a browser — but the right fix is to
    // route load_png/save_png to a JS host bridge over the browser's own image
    // codec (`createImageBitmap` / Canvas `getImageData` / `toBlob`), NOT to
    // bundle a Rust PNG stack into wasm (same "borrow the platform" design as
    // the time lib's `js_sys::Date`).  Un-skip when @P321c lands that route.
    "imaging",
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
    // world's tests save+load a binary world file.  Wasmtime can do this via
    // `--dir <tmp>` (see @P334 fix); the browser has no filesystem at all
    // without a JS host bridge (out of scope for now).  Un-skip if a future
    // JS-host VirtFS bridge ships.
    "world",
];

/// Collect every `lib/<pkg>/tests/*.loft`, sorted.
fn collect_lib_wasm_tests() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    let Ok(pkgs) = std::fs::read_dir(repo_root().join("lib")) else {
        return files;
    };
    for pkg in pkgs.filter_map(|e| e.ok()) {
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

    // Inject the WASI-stdout print bridge that the wasip2 codegen expects.
    let code = std::fs::read_to_string(&rs).ok()?;
    let shim = "#[unsafe(no_mangle)] pub extern \"C\" fn loft_host_print(ptr: *const u8, len: usize) \
        { let s = unsafe { std::slice::from_raw_parts(ptr, len) }; use std::io::Write; \
        let _ = std::io::stdout().write_all(s); }\n";
    let patched = code.replacen(
        "extern crate loft;",
        &format!("extern crate loft;\n{shim}"),
        1,
    );
    std::fs::write(&rs, &patched).ok()?;

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
    // @P334 fix (2026-05-29): preopen `--dir <tmp>` so wasip2 file I/O works.
    // The wasip2 filesystem is sandboxed — without a preopen every file op
    // fails with "os error 44" and any file-using lib traps the moment it
    // tries to open a path.  `<tmp>` is the loft `std::env::temp_dir()`
    // default, which is where `lib/world` saves/loads its test fixture; libs
    // that write elsewhere either honour env-overrides (LOFT_HOME, etc.) or
    // use CWD-relative paths.
    let run = Command::new(&wasmtime)
        .arg("--dir")
        .arg(&tmp)
        .arg(&wasm)
        .output()
        .ok()?;
    Some((
        String::from_utf8_lossy(&run.stdout).into_owned(),
        run.status.success(),
    ))
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
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("png"))
            })
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
        if have_wasmtime && let Some((out, ok)) = run_wasip2_wasm(&name, &source, &["lib"]) {
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
