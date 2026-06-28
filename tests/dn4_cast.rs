// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN25 DN4 (LOFT_DN4) — `(N-Cast)` / `(N-Cast?)`: a narrowing integer cast whose
// value is not provably in range stops being a silent 8-byte width-tag
// (`400 as u8 == 400`). `as τ` is a compile error (use `as τ?`); `as τ?` is a
// checked cast — value if it fits, else the integer null. Built behind the flag
// (default off) and validated here on BOTH backends before any cutover. Design:
// doc/claude/plans/25-nullable-sequences/implementation-steps.md § Phase 3 / DN4
// and doc/claude/formal/types.md § DN4.

use std::io::Write;
use std::process::Command;

fn write_probe(name: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_dn4");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(name);
    std::fs::File::create(&path)
        .expect("create probe")
        .write_all(src.as_bytes())
        .expect("write probe");
    path
}

const MATRIX: &str = r#"
fn main() {
  // fitting literal `as τ` — accepted, value (no error, no guard)
  assert((5 as u8) == 5, "fit literal as u8");
  // checked cast `as τ?`: in-range literal -> value, out-of-range -> null
  assert((200 as u8?) == 200, "200 as u8? in range");
  assert((300 as u8?) == null, "300 as u8? -> null");
  // dynamic out-of-range -> null; in-range -> value
  a: integer = 300;
  b: integer = 50;
  assert((a as u8?) == null, "dynamic 300 as u8? -> null");
  assert((b as u8?) == 50, "dynamic 50 as u8? -> value");
  // masked value: no range-tracking yet, so the checked form is required
  c: integer = 777;
  assert(((c & 255) as u8?) == (777 & 255), "masked via checked cast");
  // signed boundary (i8 range -128..127)
  assert((-1 as i8?) == -1, "-1 as i8? in range");
  assert((200 as i8?) == null, "200 as i8? -> null");
  println("dn4-matrix ok");
}
"#;

/// The checked-cast value matrix is correct on BOTH backends with DN4 on. (The
/// native sentinel hazard — typing the guard's null branch as the narrow τ would
/// make `i64::MIN as u8 == 0` and lose the null — is the reason this asserts native
/// too.)
#[test]
fn dn4_checked_cast_matrix_both_backends() {
    let path = write_probe("matrix.loft", MATRIX);
    for backend in ["--interpret", "--native"] {
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args([backend])
            .arg(&path)
            .env("LOFT_DN4", "1")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "180")
            .output()
            .expect("spawn loft");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("dn4-matrix ok"),
            "{backend} DN4 matrix failed:\nstdout:{stdout}\nstderr:{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// A narrowing cast to a NON-null target that can't be proven to fit is a compile
/// error (literal and dynamic) — the silent-wrongness fix.
#[test]
fn dn4_rejects_non_fit_nonnull_cast() {
    for (name, src) in [
        ("lit.loft", "fn main(){ x = 300 as u8; println(\"{x}\"); }"),
        (
            "dyn.loft",
            "fn main(){ a: integer = 300; x = a as u8; println(\"{x}\"); }",
        ),
    ] {
        let path = write_probe(name, src);
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .args(["--check"])
            .arg(&path)
            .env("LOFT_DN4", "1")
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("spawn loft");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("narrowing cast"),
            "{name}: expected a narrowing-cast error, got:\n{stderr}"
        );
    }
}

/// With the flag OFF (the default), DN4 is inert — the old width-tag behaviour
/// stands (`300 as u8 == 300`, no error). Guards the opt-in default.
#[test]
fn dn4_off_keeps_old_behaviour() {
    let path = write_probe("off.loft", "fn main(){ x = 300 as u8; println(\"{x}\"); }");
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret"])
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("300"),
        "flag-off DN4 should keep 300 as u8 == 300; got:\nstdout:{stdout}\nstderr:{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
