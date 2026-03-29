// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! W1.9 — WASM `compile_and_run()` entry point tests.
//!
//! The Rust-side unit tests verify the virtual FS API directly.
//! The Node.js smoke test is provided as an ignored test with a shell command.

extern crate loft;

/// W1.9 (native): virt_fs populate, get, and clear round-trip.
///
/// This exercises the thread-local VIRT_FS helpers without needing WASM.
#[cfg(feature = "wasm")]
#[test]
fn virt_fs_roundtrip() {
    use loft::wasm::{virt_fs_clear, virt_fs_get, virt_fs_populate};

    virt_fs_populate(&[
        ("main.loft".to_string(), "fn main() {}".to_string()),
        (
            "helper.loft".to_string(),
            "fn greet() -> text { \"hi\" }".to_string(),
        ),
    ]);

    assert_eq!(virt_fs_get("main.loft").as_deref(), Some("fn main() {}"));
    assert_eq!(
        virt_fs_get("helper.loft").as_deref(),
        Some("fn greet() -> text { \"hi\" }")
    );
    assert!(virt_fs_get("missing.loft").is_none());

    virt_fs_clear();
    assert!(virt_fs_get("main.loft").is_none());
}

/// W1.9 (Node.js integration test): runs the full WASM bridge test suite.
///
/// Requires:
///   1. `wasm-pack build --target nodejs --out-dir tests/wasm/pkg \
///          -- --no-default-features --features wasm`
///   2. Node.js in PATH.
///
/// Skips gracefully when either prerequisite is absent.
#[test]
fn wasm_compile_and_run_smoke() {
    // Skip if the WASM package is not built.
    if !std::path::Path::new("tests/wasm/pkg/loft.js").exists() {
        println!("SKIP wasm_compile_and_run_smoke — WASM package not built");
        println!(
            "     Run: wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm"
        );
        return;
    }

    // Skip if Node.js is not in PATH.
    let node_check = std::process::Command::new("node").arg("--version").output();
    if node_check.is_err() {
        println!("SKIP wasm_compile_and_run_smoke — node not in PATH");
        return;
    }

    // Run the bridge test suite.
    let result = std::process::Command::new("node")
        .arg("tests/wasm/bridge.test.mjs")
        .status()
        .expect("failed to launch node");

    assert!(
        result.success(),
        "WASM bridge tests failed (exit {:?}) — run `node tests/wasm/bridge.test.mjs` for details",
        result.code()
    );
}
