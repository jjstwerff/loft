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
    run_html_wasm_with_libs(name, source, &[])
}

/// Same as `run_html_wasm` but also passes a `--lib <dir>` for each
/// entry in `lib_dirs`.  Needed for programs that `use <pkg>;` out of
/// a local library tree (e.g. `use moros_editor;` from `lib/`).
fn run_html_wasm_with_libs(
    name: &str,
    source: &str,
    lib_dirs: &[&str],
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

    let tmp = std::env::temp_dir();
    let src = tmp.join(format!("{name}.loft"));
    let html = tmp.join(format!("{name}.html"));
    let wasm = tmp.join(format!("{name}.wasm"));

    std::fs::write(&src, source).expect("write source");

    // Serialise: the loft `--html` driver writes to a fixed
    // `/tmp/loft_html.rs` path, so parallel test invocations would
    // overwrite each other's emitted Rust mid-build.
    let _guard = build_lock().lock().unwrap();
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
    let Some((stdout, stderr, ok)) =
        run_html_wasm_with_libs("moros_editor_smoke", src, &["lib"])
    else {
        return;
    };
    assert!(ok, "moros_editor smoke trapped under --html.\n{stderr}");
    assert!(
        stdout.contains("depth=3 mat=1"),
        "expected 'depth=3 mat=1'.\nstdout: {stdout}\nstderr: {stderr}"
    );
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
