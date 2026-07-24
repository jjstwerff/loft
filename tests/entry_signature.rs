// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// #618 / #629 — the ENTRY function's call must supply every argument its
// signature declares, on BOTH backends.
//
// `n_main` is emitted from its DECLARATION, and two things widen that
// declaration beyond the plain `fn main()`: a declared `vector<text>` parameter
// (argv), and the HIDDEN return buffer that `ref_return` promotion adds to a
// heap-returning entry.  Each backend got this wrong in its own way:
//
// - the interpreter (`State::execute_argv`) pushed a bare null-sentinel
//   `DbRef` for the hidden buffer.  A vector body writes STRAIGHT into the
//   caller's record (`clear_vector` / `pre_alloc_vector` / `OpFinishRecord`)
//   and never allocates one itself, so every element write addressed
//   `stores[u16::MAX]` and aborted — `index out of bounds: the len is 2 but
//   the index is 65535` (#618);
// - the native generator hardcoded `n_main(&cell)` in all four entry
//   templates, so the same programs did not even compile: rustc `E0061`,
//   a raw cascade rather than a loft diagnostic (#629).
//
// A struct / data-enum entry return was fine on the interpreter throughout —
// its promoted body opens with its own `OpDatabase`, which turns the sentinel
// into a store.  It is kept here as the negative control: a "fix" that
// regressed it would otherwise pass the vector cases.

use std::io::Write;
use std::process::Command;

fn write_probe(name: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_entry_signature");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(name);
    std::fs::File::create(&path)
        .expect("create probe")
        .write_all(src.as_bytes())
        .expect("write probe");
    path
}

/// Run `src` on both backends and assert each prints `expect`.
///
/// The program prints the value it is about to return, so a buffer that is
/// allocated but never filled fails as loudly as one that crashes — "it
/// compiled and exited 0" is not evidence the value survived.
fn both_backends(name: &str, src: &str, expect: &str) {
    let path = write_probe(name, src);
    for backend in ["--interpret", "--native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args([backend])
            .arg(&path)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stdout.contains(expect),
            "{name} on {backend}: expected {expect:?}\nstdout:{stdout}\nstderr:{stderr}"
        );
    }
}

/// #618's core shape: the returned local IS the caller's buffer.
#[test]
fn entry_returns_bare_local_vector_both_backends() {
    both_backends(
        "vec_local.loft",
        "fn main() -> vector<integer> {\n  v = [1, 2, 3];\n  println(\"{v}\");\n  v\n}\n",
        "[1,2,3]",
    );
}

/// One axis per case: element kind, nesting depth, zero cardinality, and an
/// element too wide for 32 bits (the #618 report's "second signature").
#[test]
fn entry_returns_vector_shapes_both_backends() {
    for (name, src, expect) in [
        (
            "vec_text.loft",
            "fn main() -> vector<text> {\n  v = [\"a\", \"b\"];\n  println(\"{v}\");\n  v\n}\n",
            "[\"a\",\"b\"]",
        ),
        (
            "vec_nested.loft",
            "fn main() -> vector<vector<integer>> {\n  v = [[1, 2], [3]];\n  println(\"{v}\");\n  v\n}\n",
            "[[1,2],[3]]",
        ),
        (
            "vec_empty.loft",
            "fn main() -> vector<integer> {\n  v: vector<integer> = [];\n  println(\"{v}\");\n  v\n}\n",
            "[]",
        ),
        (
            "vec_wide.loft",
            "fn main() -> vector<integer> {\n  v = [9000000000, 0];\n  println(\"{v}\");\n  v\n}\n",
            "[9000000000,0]",
        ),
        // Construction path: the value arrives from a CALL, so the entry's
        // buffer receives a callee's result rather than a locally built one.
        (
            "vec_from_call.loft",
            "fn build() -> vector<integer> {\n  w = [4, 5];\n  w\n}\nfn main() -> vector<integer> {\n  v = build();\n  println(\"{v}\");\n  v\n}\n",
            "[4,5]",
        ),
    ] {
        both_backends(name, src, expect);
    }
}

/// Negative control — this path allocates from the sentinel itself and was
/// never broken on the interpreter.  It must stay working.
#[test]
fn entry_returns_struct_both_backends() {
    both_backends(
        "struct_local.loft",
        "struct P { x: integer }\nfn main() -> P {\n  p = P { x: 1 };\n  println(\"{p}\");\n  p\n}\n",
        "{x:1}",
    );
}

/// #629's other half: a DECLARED entry parameter widens `n_main` just as the
/// hidden buffer does, and the native entry template omitted it too.
#[test]
fn entry_declaring_argv_both_backends() {
    let path = write_probe(
        "argv.loft",
        "fn main(args: vector<text>) {\n  println(\"argc={len(args)}\");\n}\n",
    );
    for backend in ["--interpret", "--native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args([backend])
            .arg(&path)
            .args(["one", "two"])
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("argc=2"),
            "argv on {backend}: expected argc=2\nstdout:{stdout}\nstderr:{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// A plain `fn main()` takes no extra argument, so the entry call must be
/// exactly what it always was — the common path stays byte-identical.
#[test]
fn plain_entry_is_unchanged() {
    let path = write_probe("plain.loft", "fn main() {\n  println(\"plain\");\n}\n");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--native"])
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_KEEP_NATIVE_RS", "1")
        .env("LOFT_TIMEOUT", "180")
        .output()
        .expect("spawn loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let rs = stderr
        .lines()
        .find_map(|l| l.split("source preserved at ").nth(1))
        .and_then(|l| l.split_whitespace().next())
        .map(std::path::PathBuf::from)
        .expect("LOFT_KEEP_NATIVE_RS should report the generated path");
    let src = std::fs::read_to_string(&rs).expect("read generated rust");
    assert!(
        src.contains("    n_main(&cell);\n"),
        "a plain entry must still be called with just the cell"
    );
    let _ = std::fs::remove_file(&rs);
}
