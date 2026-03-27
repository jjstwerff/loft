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

/// W1.9 (Node.js smoke test — requires wasm-pack + Node.js):
///
/// ```sh
/// wasm-pack build --target nodejs --out-dir tests/wasm/pkg \
///     -- --features wasm --no-default-features
/// node -e "
///   const loft = require('./tests/wasm/pkg/loft_wasm.js');
///   const r = loft.compile_and_run(JSON.stringify([
///       {name:'main.loft',content:'fn main(){println(\"hi\")}'}
///   ]));
///   const out = JSON.parse(r);
///   console.assert(out.success, 'success');
///   console.assert(out.output === 'hi\n', 'output');
///   console.log('W1.9 ok:', out.output.trim());
/// "
/// ```
#[test]
#[ignore = "W1.9: Node.js smoke test — requires wasm-pack + Node.js"]
fn wasm_compile_and_run_smoke() {
    // This test documents the expected Node.js invocation but cannot run here.
    // See the doc comment above for the manual test procedure.
}
