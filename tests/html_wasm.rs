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

/// Build a loft program via `--html`, extract the WASM, run it via the
/// Node repro harness with stub imports, and return (stdout, stderr,
/// exit_status).  Returns None when prerequisites are missing.
fn run_html_wasm(name: &str, source: &str) -> Option<(String, String, bool)> {
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

    let tmp = std::env::temp_dir();
    let src = tmp.join(format!("{name}.loft"));
    let html = tmp.join(format!("{name}.html"));
    let wasm = tmp.join(format!("{name}.wasm"));

    std::fs::write(&src, source).expect("write source");

    // Serialise: the loft `--html` driver writes to a fixed
    // `/tmp/loft_html.rs` path, so parallel test invocations would
    // overwrite each other's emitted Rust mid-build.
    let _guard = build_lock().lock().unwrap();
    let status = Command::new(&loft_bin)
        .args([
            "--html",
            html.to_str().unwrap(),
            "--path",
            &format!("{}/", repo_root().display()),
            src.to_str().unwrap(),
        ])
        .status()
        .expect("invoke loft --html");
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
