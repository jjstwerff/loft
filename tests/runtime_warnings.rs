// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-07 phase 4e.2 — undefended fault-site compile-time warning.
//!
//! Binary-level tests: invoke the loft compiler with `LOFT_NO_WARN_RUNTIME`
//! explicitly UNset (the in-process test harness sets it; here we bypass
//! that suppression because the WHOLE POINT is to assert on the
//! warning's presence/absence) and inspect the stderr output.
//!
//! Coverage:
//!   - The warning fires for every undefended fault site
//!     (`undefended_*` cells).
//!   - The warning is silenced by each of the four canonical safe
//!     patterns from the design's "Easy-proof skip list — REQUIRED"
//!     (`skip_*` cells).
//!   - The warning is silenced when 4d.1 / 4d.2 / 4e.1 swap fires
//!     (`defended_*` cells — covered alongside the runtime tests in
//!     `tests/runtime_logging.rs`; rechecked here for the compile-time
//!     half of each defense).
//!   - `LOFT_NO_WARN_RUNTIME=1` silences the warning entirely
//!     (`silenced_by_env` cell).

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Run a loft snippet under `--interpret` with `LOFT_NO_WARN_RUNTIME`
/// explicitly UNset and return (stdout, stderr, exit-status-code).
fn run_with_warnings(name: &str, source: &str) -> (String, String, Option<i32>) {
    let script_path = std::env::temp_dir().join(format!("loft_w42_{name}.loft"));
    std::fs::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

// ── Warning fires on every undefended fault site ────────────────────────────

#[test]
fn undefended_div_by_var_warns() {
    let source = "\
fn main() {
  z = 5;
  x = 10 / z;
  print(\"x={x}\\n\");
}
";
    let (diag, _stderr, _code) = run_with_warnings("undef_div", source);
    assert!(
        diag.contains("warning: integer division may produce null"),
        "expected div warning; got stdout={diag:?}"
    );
}

#[test]
fn undefended_mod_by_var_warns() {
    let source = "\
fn main() {
  z = 5;
  x = 10 % z;
  print(\"x={x}\\n\");
}
";
    let (diag, _stderr, _code) = run_with_warnings("undef_mod", source);
    assert!(
        diag.contains("warning: integer modulus may produce null"),
        "expected mod warning; got stdout={diag:?}"
    );
}

#[test]
fn undefended_vec_index_by_var_warns() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  i = 1;
  x = v[i];
  print(\"x={x}\\n\");
}
";
    let (diag, _stderr, _code) = run_with_warnings("undef_vec", source);
    assert!(
        diag.contains("warning: `v[i]` may produce null"),
        "expected vec OOB warning; got stdout={diag:?}"
    );
}

#[test]
fn undefended_text_index_by_var_warns() {
    let source = "\
fn main() {
  s = \"abc\";
  i = 1;
  c = s[i];
  print(\"c={c}\\n\");
}
";
    let (diag, _stderr, _code) = run_with_warnings("undef_text", source);
    assert!(
        diag.contains("warning: `s[i]` may produce null"),
        "expected text OOB warning; got stdout={diag:?}"
    );
}

// ── Skip pattern 1 — constant non-zero literal divisor ──────────────────────

#[test]
fn skip_constant_nonzero_divisor() {
    let source = "\
fn main() {
  x = 10 / 3;
  y = 10 % 7;
  print(\"x={x} y={y}\\n\");
}
";
    let (diag, _stderr, _code) = run_with_warnings("skip_const_div", source);
    assert!(
        !diag.contains("warning: integer division may produce null"),
        "constant non-zero divisor must NOT warn; got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: integer modulus may produce null"),
        "constant non-zero modulus must NOT warn; got stdout={diag:?}"
    );
}

// ── Skip pattern 2 — constant non-negative literal index ────────────────────

#[test]
fn skip_constant_index() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  s = \"abc\";
  print(\"v[1]={v[1]} s[0]={s[0]}\\n\");
}
";
    let (diag, _stderr, _code) = run_with_warnings("skip_const_idx", source);
    assert!(
        !diag.contains("warning: `v[i]` may produce null"),
        "constant index must NOT warn; got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: `s[i]` may produce null"),
        "constant text index must NOT warn; got stdout={diag:?}"
    );
}

// ── Skip pattern 3 — index is a for-loop iteration variable ─────────────────

#[test]
fn skip_for_loop_iter_var() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  for i in 0..len(v) {
    x = v[i];
    print(\"x={x}\\n\");
  }
}
";
    let (diag, _stderr, _code) = run_with_warnings("skip_for_iter", source);
    assert!(
        !diag.contains("warning: `v[i]` may produce null"),
        "for-loop iter var index must NOT warn; got stdout={diag:?}"
    );
}

// ── 4d.1 / 4d.2 / 4e.1 defenses silence the compile-time warning too ────────

#[test]
fn defended_nullable_rescue_quiet() {
    let source = "\
fn main() {
  z = 5;
  v = [10, 20, 30];
  i = 1;
  a = (10 / z) ?? 0;
  b = v[i] ?? 0;
  print(\"a={a} b={b}\\n\");
}
";
    let (diag, _stderr, _code) = run_with_warnings("def_nullable", source);
    assert!(
        !diag.contains("warning: integer division"),
        "?? rescue must silence div warning; got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: `v[i]`"),
        "?? rescue must silence vec warning; got stdout={diag:?}"
    );
}

#[test]
fn defended_bare_null_check_quiet() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  i = 1;
  x = v[i];
  if x != null { print(\"got\\n\"); }
}
";
    let (diag, _stderr, _code) = run_with_warnings("def_null_check", source);
    assert!(
        !diag.contains("warning: `v[i]`"),
        "if x != null must silence vec warning; got stdout={diag:?}"
    );
}

#[test]
fn defended_format_string_quiet() {
    let source = "\
fn main() {
  z = 5;
  v = [10, 20, 30];
  i = 1;
  print(\"div={10 / z} vec={v[i]}\\n\");
}
";
    let (diag, _stderr, _code) = run_with_warnings("def_fmt", source);
    assert!(
        !diag.contains("warning: integer division"),
        "format-string div must silence warning (4e.1); got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: `v[i]`"),
        "format-string vec must silence warning (4e.1); got stdout={diag:?}"
    );
}

// ── Env-var silencing knob ──────────────────────────────────────────────────

#[test]
fn silenced_by_env() {
    let source = "\
fn main() {
  z = 5;
  x = 10 / z;
  print(\"x={x}\\n\");
}
";
    let script_path = std::env::temp_dir().join("loft_w42_silenced.loft");
    std::fs::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .env("LOFT_NO_WARN_RUNTIME", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);
    let diag = String::from_utf8_lossy(&out.stdout);
    assert!(
        !diag.contains("warning: integer division"),
        "LOFT_NO_WARN_RUNTIME=1 must silence the warning; got stdout={diag:?}"
    );
}

// ── Plan-07 phase 4e.3 — distinct null tokens in format-string output ──

/// Format-string interpolation of `1 / z` (z=0) renders `null(/0)`
/// — distinguishes fault-produced null from bare-value null.
#[test]
fn fmt43_div_by_zero_renders_null_div() {
    let source = "\
fn main() {
  z = 0;
  print(\"a={1 / z}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("fmt43_div", source);
    assert_eq!(code, Some(0), "format-string suppression must not halt");
    assert!(
        stdout.contains("a=null(/0)"),
        "expected `null(/0)` suffix; got stdout={stdout:?}"
    );
}

#[test]
fn fmt43_mod_by_zero_renders_null_mod() {
    let source = "\
fn main() {
  z = 0;
  print(\"a={5 % z}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("fmt43_mod", source);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("a=null(%0)"),
        "expected `null(%0)` suffix; got stdout={stdout:?}"
    );
}

#[test]
fn fmt43_vec_oob_renders_null_oob() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  print(\"a={v[999]}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("fmt43_vec", source);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("a=null(oob)"),
        "expected `null(oob)` suffix; got stdout={stdout:?}"
    );
}

#[test]
fn fmt43_genuine_null_renders_bare_null() {
    let source = "\
fn main() {
  z = null as integer;
  print(\"a={z}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("fmt43_bare", source);
    assert_eq!(code, Some(0));
    assert!(
        stdout.contains("a=null"),
        "expected bare null; got stdout={stdout:?}"
    );
    assert!(
        !stdout.contains("a=null("),
        "genuine null must not get a fault suffix; got stdout={stdout:?}"
    );
}

#[test]
fn fmt43_loft_format_bare_null_env_silences_suffix() {
    let source = "\
fn main() {
  z = 0;
  print(\"a={1 / z}\\n\");
}
";
    let script_path = std::env::temp_dir().join("loft_w42_fmt43_env.loft");
    std::fs::write(&script_path, source).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&script_path)
        .current_dir(workspace_root())
        .env("LOFT_FORMAT_BARE_NULL", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&script_path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("a=null"),
        "bare null still rendered; got stdout={stdout:?}"
    );
    assert!(
        !stdout.contains("a=null("),
        "LOFT_FORMAT_BARE_NULL=1 must silence the suffix; got stdout={stdout:?}"
    );
}
