// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 2c/2d — the deliver WHOLE-SLICE falsifier: a loft value delivered from a
// `--html`-built wasm is reconstructed IN JS, driven only by the layout descriptor, and equals
// the value the program delivered. This closes the parity gate interpret == native == --html:
// `deliver_parity.rs` pins interpret == native (the loopback bytes); this pins that the generic
// JS reader (doc/loft-deliver.js, the twin of `read_via_descriptor`) reproduces the same value in
// the browser target.
//
// Self-skips when the wasm toolchain is unavailable (node / wasm32 target / release binary) —
// same policy as tests/html_wasm.rs.

use std::path::PathBuf;
use std::process::Command;

fn which(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wasm32_installed() -> bool {
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-unknown-unknown"))
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Build `source` via `loft --html`, extract the embedded wasm, run it through the deliver node
/// harness, and return its stdout — or `None` if the toolchain self-skips.
fn run_deliver(name: &str, source: &str) -> Option<String> {
    if !which("node") {
        eprintln!("SKIP: node not installed");
        return None;
    }
    if !wasm32_installed() {
        eprintln!("SKIP: wasm32-unknown-unknown target not installed");
        return None;
    }
    let loft_bin = repo_root().join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release`)");
        return None;
    }

    let tmp = std::env::temp_dir().join(format!("loft_deliver_{name}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create per-test dir");
    let src = tmp.join(format!("{name}.loft"));
    let html = tmp.join(format!("{name}.html"));
    let wasm = tmp.join(format!("{name}.wasm"));
    std::fs::write(&src, source).expect("write source");

    let status = Command::new(&loft_bin)
        .args([
            "--html",
            html.to_str().unwrap(),
            "--path",
            &format!("{}/", repo_root().display()),
        ])
        .arg(src.to_str().unwrap())
        .status()
        .expect("invoke loft --html");
    assert!(status.success(), "loft --html failed for {name}");

    // Extract the embedded wasm (`const wasmB64="…"`), exactly as tests/html_wasm.rs does.
    let page = std::fs::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = page.find(marker).expect("wasmB64 marker") + marker.len();
    let end = start + page[start..].find('"').expect("wasmB64 closing quote");
    std::fs::write(&wasm, loft::base64::decode(&page[start..end])).expect("write wasm");

    let harness = repo_root().join("tools/deliver_repro.mjs");
    assert!(harness.exists(), "tools/deliver_repro.mjs missing");
    let out = Command::new("node")
        .arg(&harness)
        .arg(&wasm)
        .output()
        .expect("invoke node deliver harness");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "deliver harness failed for {name}\nstdout:{stdout}\nstderr:{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(stdout)
}

#[test]
fn deliver_reconstructs_nested_value_in_js() {
    // Exercises every node kind in the serializable subset the reader mirrors: text (interned),
    // a nested record, an i64 scalar, an f64 scalar, a scalar vector (the fast lane), and a bool.
    let src = r#"
struct Inner { a: integer, b: float }
struct Outer { name: text, inner: Inner, nums: vector<integer>, ok: boolean }
fn main() {
  o = Outer { name: "hi", inner: Inner { a: 7, b: 1.5 }, nums: [10, 20, 30], ok: true };
  deliver(1, o);
}
"#;
    let Some(stdout) = run_deliver("nested", src) else {
        return; // toolchain self-skip
    };
    // The JS reader must reconstruct the exact value the program delivered. Pin the tag + value,
    // NOT the internal type-id (which shifts with the stdlib).
    let want_value =
        "\"value\":{\"name\":\"hi\",\"inner\":{\"a\":7,\"b\":1.5},\"nums\":[10,20,30],\"ok\":true}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value) && stdout.contains("\"tag\":1"),
        "reconstructed value mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}
