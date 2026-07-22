// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `host_input()` — the program-input primitive (mirror of `print`).  Reads all
//! input as one text: stdin on `--interpret` / `--native` (and WASI), the JS
//! host on `--html`.  This guards the interpret == native byte-parity and the
//! empty-input case.  The `--html` leg (JS `globalThis.loftInput` → the same
//! bytes) is proven with the Node harness in `doc/claude/WEB_APPS.md`; it is not
//! run here because it needs the wasm toolchain + Node.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

const FIXTURE: &str = r#"fn main() {
  input = host_input();
  println("echo=[{input}] len={input.len()}");
}
"#;

/// Run the fixture on `backend`, feeding `stdin`, return captured stdout.  Each
/// call gets its own temp dir so `--native`'s `.loft/` build artifacts and
/// parallel test threads never collide.
///
/// On anything other than a clean exit with output, the returned string is a
/// DIAGNOSTIC rather than the empty string.  This used to discard stderr and return
/// `""`, so when the `--native` leg failed under full-suite load the assertion read
/// `left: "echo=[café] len=4\n", right: ""` — a symptom with its cause thrown away,
/// leaving nothing to act on but "it flaked".  A failing build or a killed child now
/// says so, in the panic message, at the moment it happens.
fn run(backend: &str, stdin: &str) -> String {
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("loft_host_input_{}_{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("hi.loft");
    std::fs::write(&path, FIXTURE).expect("write fixture");
    let mut child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args([backend, path.to_str().unwrap()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loft");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() && !stdout.is_empty() {
        return stdout;
    }
    // Carry the cause into the assertion instead of returning a bare "".
    let stderr = String::from_utf8_lossy(&out.stderr);
    format!(
        "<{backend} produced no usable output: status={:?}, stdout={stdout:?}, stderr={}>",
        out.status,
        if stderr.trim().is_empty() {
            "(empty)".to_string()
        } else {
            format!("{:?}", stderr.trim())
        }
    )
}

/// The whole point: the same source reading `host_input()` produces byte-for-byte
/// identical output on the interpreter and the native backend.
#[test]
fn host_input_interpret_equals_native() {
    let interp = run("--interpret", "hello loft");
    let native = run("--native", "hello loft");
    assert_eq!(
        interp, "echo=[hello loft] len=10\n",
        "interpret: {interp:?}"
    );
    assert_eq!(interp, native, "interpret vs native diverged");
}

/// No input → an empty (not null) text, length 0 — on both backends.
#[test]
fn host_input_empty() {
    assert_eq!(run("--interpret", ""), "echo=[] len=0\n");
    assert_eq!(run("--native", ""), "echo=[] len=0\n");
}

/// `host_output(msg)` — the outbound mirror.  On native/WASI it writes one
/// line per message to STDERR (the machine channel, scriptable by the
/// invoking process), never to stdout — and byte-identically on both
/// backends.  (The `--html` leg — globalThis.loftOutput — rides the same
/// Node/browser harness as the input leg.)
#[test]
fn host_output_goes_to_stderr_on_both_backends() {
    let fixture = r#"fn main() {
  host_output("req 1 GET http://x/");
  host_output("req 2 GET http://y/");
  println("done");
}
"#;
    for backend in ["--interpret", "--native"] {
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("loft_host_output_{}_{id}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("ho.loft");
        std::fs::write(&path, fixture).expect("write fixture");
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args([backend, path.to_str().unwrap()])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("run loft");
        let _ = std::fs::remove_dir_all(&dir);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stdout,
            "done
",
            "{backend} stdout: {stdout:?}"
        );
        assert!(
            stderr.contains(
                "req 1 GET http://x/
"
            ) && stderr.contains(
                "req 2 GET http://y/
"
            ),
            "{backend} stderr: {stderr:?}"
        );
    }
}

/// Bytes pass through verbatim, including a UTF-8 multi-byte char (len is bytes).
#[test]
fn host_input_utf8_passthrough() {
    // "café" = 4 characters over 5 UTF-8 bytes (é is 2).  The engine moves opaque
    // bytes; loft decodes them and `len` counts characters (@PLN110).
    let interp = run("--interpret", "café");
    assert_eq!(interp, "echo=[café] len=4\n", "interpret: {interp:?}");
    assert_eq!(
        interp,
        run("--native", "café"),
        "interpret vs native diverged"
    );
}
