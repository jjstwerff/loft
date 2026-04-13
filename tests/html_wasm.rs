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

/// P137: build a `println`-only `.loft` program via `--html`, extract
/// the WASM, and run it via the Node repro harness with stub imports.
/// Exit code 0 means `loft_start` returned without trapping.
#[test]
fn p137_html_hello_world_does_not_trap() {
    if which("node").is_none() {
        eprintln!("SKIP: node not installed");
        return;
    }
    if !wasm32_target_installed() {
        eprintln!("SKIP: rustup target wasm32-unknown-unknown not installed");
        return;
    }

    let tmp = std::env::temp_dir();
    let src = tmp.join("p137_hello.loft");
    let html = tmp.join("p137_hello.html");
    let wasm = tmp.join("p137_hello.wasm");

    std::fs::write(&src, "fn main() { println(\"hello from loft\"); }\n").expect("write source");

    // Build the --html bundle.  Use the release binary at
    // $CARGO_MANIFEST_DIR/target/release/loft — cargo test builds
    // tests against that binary by convention.
    let loft_bin = repo_root().join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release` first)");
        return;
    }

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
    assert!(status.success(), "loft --html failed");

    // Extract the base64 wasm blob from the HTML.
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

    // Run the Node harness.
    let harness = repo_root().join("tools/wasm_repro.mjs");
    assert!(harness.exists(), "tools/wasm_repro.mjs missing");

    let out = Command::new("node")
        .arg(&harness)
        .arg(&wasm)
        .output()
        .expect("invoke node harness");

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        out.status.success(),
        "WASM trapped — P137 regression.\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        stdout.contains("hello from loft") || stderr.contains("loft_start: OK"),
        "expected 'hello from loft' in output.\nstdout: {stdout}\nstderr: {stderr}"
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
    if s.len() % 4 != 0 {
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
