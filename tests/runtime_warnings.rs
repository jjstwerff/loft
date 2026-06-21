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
    let (_stdout, diag, _code) = run_with_warnings("undef_div", source);
    assert!(
        diag.contains("warning: division may produce null"),
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
    let (_stdout, diag, _code) = run_with_warnings("undef_mod", source);
    assert!(
        diag.contains("warning: modulus may produce null"),
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
    let (_stdout, diag, _code) = run_with_warnings("undef_vec", source);
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
    let (_stdout, diag, _code) = run_with_warnings("undef_text", source);
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
    let (_stdout, diag, _code) = run_with_warnings("skip_const_div", source);
    assert!(
        !diag.contains("warning: division may produce null"),
        "constant non-zero divisor must NOT warn; got stdout={diag:?}"
    );
    assert!(
        !diag.contains("warning: modulus may produce null"),
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
    let (_stdout, diag, _code) = run_with_warnings("skip_const_idx", source);
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
    let (_stdout, diag, _code) = run_with_warnings("skip_for_iter", source);
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
    let (_stdout, diag, _code) = run_with_warnings("def_nullable", source);
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
    let (_stdout, diag, _code) = run_with_warnings("def_null_check", source);
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
    let (_stdout, diag, _code) = run_with_warnings("def_fmt", source);
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

// ── Plan-07 phase 4f.12 — stack overflow becomes typed RuntimeError ─────────

/// Infinite recursion produces a typed `StackOverflow` error
/// (rendered with file:line:col and the call chain) instead of
/// an opaque Rust panic.  Production mode logs and continues
/// per C66; dev mode halts and renders.
#[test]
fn f4f_stack_overflow_raises_typed_error() {
    let source = "\
fn recurse(n: integer) -> integer {
  return recurse(n + 1);
}

fn main() {
  x = recurse(0);
  print(\"x={x}\\n\");
}
";
    let (_stdout, stderr, code) = run_with_warnings("f4f_stack", source);
    assert_eq!(code, Some(1), "stack overflow must halt with exit 1");
    // Pretty-renderer + call chain go to stderr per main.rs's
    // `eprint!` / `eprintln!` calls.
    assert!(
        stderr.contains("call stack overflow"),
        "expected typed StackOverflow error; got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("in fn recurse() ← called from"),
        "expected call-chain header pointing at recurse(); got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("more frames"),
        "expected truncation summary; got stderr={stderr:?}"
    );
}

// ── Plan-07 phase 4g — soft-halt + call-chain rendering ─────────────────────

/// `--dev-soft-halt` continues past the first fault and reports
/// every fault site to stderr instead of halting on the first.
#[test]
fn g4g_soft_halt_continues_past_faults() {
    let source = "\
fn main() {
  z = 0;
  bad1 = 1 / z;
  bad2 = 2 / z;
  print(\"done\\n\");
}
";
    let script_path = std::env::temp_dir().join("loft_w42_g4g_soft.loft");
    std::fs::write(&script_path, source).expect("write");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--dev-soft-halt")
        .arg(&script_path)
        .current_dir(workspace_root())
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .output()
        .expect("invoke loft");
    let _ = std::fs::remove_file(&script_path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Both faults rendered to stderr.
    assert!(
        stderr.contains("soft-halt: divide by zero")
            && stderr.matches("soft-halt: divide by zero").count() == 2,
        "expected 2 soft-halt fault lines on stderr; got {stderr:?}"
    );
    // Script ran past faults to print "done".
    assert!(
        stdout.contains("done"),
        "soft-halt must continue execution; got stdout={stdout:?}"
    );
    // Exit non-zero because had_fatal was set.
    assert_eq!(
        out.status.code(),
        Some(1),
        "soft-halt with faults must exit 1"
    );
}

/// Without `--dev-soft-halt`, the first fault halts execution.
#[test]
fn g4g_default_halts_on_first_fault() {
    let source = "\
fn main() {
  z = 0;
  bad1 = 1 / z;
  bad2 = 2 / z;
  print(\"done\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("g4g_no_soft", source);
    assert_eq!(code, Some(1));
    assert!(
        !stdout.contains("done"),
        "default mode must halt on first fault; got stdout={stdout:?}"
    );
}

/// Faults inside a call chain render the chain after the typed
/// error, innermost first.
#[test]
fn g4g_call_chain_rendered() {
    let source = "\
fn divide_safely(a: integer, b: integer) -> integer {
  return a / b;
}

fn calculate_damage(base: integer, armor: integer) -> integer {
  reduced = divide_safely(base * 100, armor);
  return reduced;
}

fn main() {
  enemy_armor = 0;
  dmg = calculate_damage(50, enemy_armor);
  print(\"damage: {dmg}\\n\");
}
";
    let (_stdout, stderr, code) = run_with_warnings("g4g_chain", source);
    assert_eq!(code, Some(1), "must halt at fault");
    // Innermost frame named first, then chevron + outer frames.
    assert!(
        stderr.contains("in fn divide_safely() ← called from"),
        "expected innermost-first chain header; got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("fn calculate_damage()"),
        "expected outer frame; got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("fn main()"),
        "expected top frame; got stderr={stderr:?}"
    );
}

// ── Plan-07 phase 4f — float / single div / mod by zero raises ──────────────

/// Undefended float div by zero raises and halts in dev mode.
#[test]
fn f4f_float_div_by_zero_raises() {
    let source = "\
fn main() {
  z = 0.0;
  bad = 1.0 / z;
  print(\"bad={bad}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("4f_div_float", source);
    assert_eq!(code, Some(1), "must halt with exit 1; got code={code:?}");
    assert!(
        !stdout.contains("bad="),
        "post-fault stdout must NOT print; got stdout={stdout:?}"
    );
}

/// Defended float div by `?? default` does NOT raise — IEEE result
/// (Inf for `1.0 / 0.0`) is returned.
#[test]
fn f4f_float_div_with_nullable_does_not_raise() {
    let source = "\
fn main() {
  z = 0.0;
  good = (1.0 / z) ?? 99.0;
  print(\"good={good}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("4f_div_float_def", source);
    assert_eq!(code, Some(0), "?? defense must not halt");
    // 1.0 / 0.0 = Inf which is NOT null, so `??` doesn't fire.
    assert!(
        stdout.contains("good=inf"),
        "expected IEEE Inf; got stdout={stdout:?}"
    );
}

/// Defended `0.0 / 0.0` (NaN, IS null for floats) `?? default` returns default.
#[test]
fn f4f_float_nan_with_nullable_returns_default() {
    let source = "\
fn main() {
  z = 0.0;
  good = (0.0 / z) ?? 42.0;
  print(\"good={good}\\n\");
}
";
    let (stdout, _stderr, code) = run_with_warnings("4f_nan_float_def", source);
    assert_eq!(code, Some(0));
    // 0.0 / 0.0 = NaN which IS null, so `??` fires.
    assert!(
        stdout.contains("good=42"),
        "expected `??` default 42; got stdout={stdout:?}"
    );
}

/// Float mod by zero raises like div.
#[test]
fn f4f_float_mod_by_zero_raises() {
    let source = "\
fn main() {
  z = 0.0;
  bad = 5.0 % z;
  print(\"bad={bad}\\n\");
}
";
    let (_stdout, _stderr, code) = run_with_warnings("4f_mod_float", source);
    assert_eq!(code, Some(1));
}

/// Single-precision div by zero raises.
#[test]
fn f4f_single_div_by_zero_raises() {
    let source = "\
fn main() {
  z = 0.0f;
  bad = 1.0f / z;
  print(\"bad={bad}\\n\");
}
";
    let (_stdout, _stderr, code) = run_with_warnings("4f_div_single", source);
    assert_eq!(code, Some(1));
}

// ── Plan-07 phase 4h — `not null` field-reminder hint ──────────────────

/// A struct field read 10+ times with no `??` defense triggers the
/// `not null` hint at the struct declaration.
#[test]
fn hint_4h_high_read_count_suggests_not_null() {
    let source = "\
struct Player {
  id: integer,
  name: text
}

fn use_player(p: Player) -> integer {
  s = p.id + p.id + p.id + p.id;
  s += p.id + p.id + p.id + p.id;
  s += p.id + p.id + p.id + p.id;
  return s;
}

fn main() {
  p = Player { id: 1, name: \"alice\" };
  print(\"score={use_player(p)}\\n\");
}
";
    let (_stdout, _stderr, code) = run_with_warnings("hint4h_high", source);
    let diag = String::from_utf8_lossy(
        &Command::new(loft_bin())
            .arg("--interpret")
            .arg({
                let p = std::env::temp_dir().join("loft_w42_hint4h_high.loft");
                std::fs::write(&p, source).expect("write");
                p
            })
            .current_dir(workspace_root())
            .env_remove("LOFT_NO_HINT_NOT_NULL")
            .env_remove("LOFT_NO_WARN_RUNTIME")
            .output()
            .expect("invoke loft")
            .stderr,
    )
    .into_owned();
    assert_eq!(code, Some(0), "must not halt; got code={code:?}");
    assert!(
        diag.contains("field `Player.id`") && diag.contains("consider marking it `not null`"),
        "expected not-null hint for Player.id; got diag={diag:?}"
    );
}

/// A `not null` field with many reads must NOT trigger the hint.
#[test]
fn hint_4h_already_not_null_quiet() {
    let source = "\
struct Player {
  id: integer not null,
  name: text
}

fn use_player(p: Player) -> integer {
  s = p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id;
  return s;
}

fn main() {
  p = Player { id: 1, name: \"alice\" };
  print(\"score={use_player(p)}\\n\");
}
";
    let p = std::env::temp_dir().join("loft_w42_hint4h_already.loft");
    std::fs::write(&p, source).expect("write");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&p)
        .current_dir(workspace_root())
        .env_remove("LOFT_NO_HINT_NOT_NULL")
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .output()
        .expect("invoke loft");
    let _ = std::fs::remove_file(&p);
    let diag = String::from_utf8_lossy(&out.stdout);
    assert!(
        !diag.contains("consider marking it `not null`"),
        "already-not-null field must not get hint; got diag={diag:?}"
    );
}

/// A field read with `?? default` defense (even once) must silence
/// the hint regardless of read count.
#[test]
fn hint_4h_defended_with_nullable_quiet() {
    let source = "\
struct Player {
  id: integer,
  name: text
}

fn use_player(p: Player) -> integer {
  s = (p.id ?? 0);
  s += p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id;
  return s;
}

fn main() {
  p = Player { id: 1, name: \"alice\" };
  print(\"score={use_player(p)}\\n\");
}
";
    let p = std::env::temp_dir().join("loft_w42_hint4h_defended.loft");
    std::fs::write(&p, source).expect("write");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&p)
        .current_dir(workspace_root())
        .env_remove("LOFT_NO_HINT_NOT_NULL")
        .env_remove("LOFT_NO_WARN_RUNTIME")
        .output()
        .expect("invoke loft");
    let _ = std::fs::remove_file(&p);
    let diag = String::from_utf8_lossy(&out.stdout);
    assert!(
        !diag.contains("consider marking it `not null`"),
        "?? defended field must not get hint; got diag={diag:?}"
    );
}

/// `LOFT_NO_HINT_NOT_NULL=1` env var silences the hint entirely.
#[test]
fn hint_4h_env_silences() {
    let source = "\
struct Player {
  id: integer,
  name: text
}

fn use_player(p: Player) -> integer {
  s = p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id + p.id;
  return s;
}

fn main() {
  p = Player { id: 1, name: \"alice\" };
  print(\"score={use_player(p)}\\n\");
}
";
    let p = std::env::temp_dir().join("loft_w42_hint4h_env.loft");
    std::fs::write(&p, source).expect("write");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&p)
        .current_dir(workspace_root())
        .env("LOFT_NO_HINT_NOT_NULL", "1")
        .env("LOFT_NO_WARN_RUNTIME", "1")
        .output()
        .expect("invoke loft");
    let _ = std::fs::remove_file(&p);
    let diag = String::from_utf8_lossy(&out.stdout);
    assert!(
        !diag.contains("consider marking it `not null`"),
        "LOFT_NO_HINT_NOT_NULL=1 must silence hint; got diag={diag:?}"
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

// ── Skip pattern 4 — loop-bounded arithmetic index ──────────────────────────
//
// Generalises skip-pattern 3 (a bare loop var) to integer arithmetic over
// loop counters — `m[i * 4 + j]` where i, j are active for-loop vars is as
// trustworthy as a bare `m[i]`.  Surfaced by the graphics matrix kernels
// (lib/graphics/src/math.loft::mat4_mul).
#[test]
fn skip_loop_bounded_arithmetic_index() {
    let source = "\
fn main() {
  m = [for k in 0..16 { k * 1.0 }];
  sum = 0.0;
  for i in 0..4 {
    for j in 0..4 {
      sum += m[i * 4 + j];
    }
  }
  print(\"sum={sum}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_loop_arith", source);
    assert!(
        !diag.contains("warning: `v[i]` may produce null"),
        "loop-bounded arithmetic index must NOT warn; got stderr={diag:?}"
    );
    // An index mixing in a non-loop value (a parameter / field) still warns.
    let source2 = "\
fn at(v: vector<integer>, base: integer) -> integer {
  total = 0;
  for k in 0..3 { total += v[base + k]; }
  total
}
fn main() { print(\"{at([1,2,3,4], 1)}\\n\"); }
";
    let (_o2, diag2, _c2) = run_with_warnings("skip_loop_arith_neg", source2);
    assert!(
        diag2.contains("warning: `v[i]` may produce null"),
        "index mixing a non-loop value (base) must still warn; got stderr={diag2:?}"
    );
}

// ── `??` after a fault-prone op is a real defense, not "redundant" ──────────
//
// Indexing a `not null` vector, or dividing by a `not null` field, can still
// yield null (out-of-bounds / divide-by-zero).  So `v[i] ?? d` and
// `a / s.field ?? d` must NOT be flagged "Redundant null coalescing" even
// though the vector / field operand carries `not null`.  Without the fix the
// vector-index case and the division case hit contradictory checks (the OOB /
// div warning said "defend me", the redundant check said "the `??` is
// useless").  Surfaced by lib/graphics/src/math.loft + graphics.loft.
#[test]
fn coalesce_after_fault_op_not_redundant() {
    let source = "\
struct M { m: vector<float> not null }
struct S { stride: integer not null }
fn pick(x: const M, i: integer) -> float { x.m[i] ?? 0.0 }
fn cnt(s: const S, n: integer) -> integer { (n / s.stride) ?? 0 }
fn main() {
  a = pick(M { m: [1.0, 2.0] }, 0);
  b = cnt(S { stride: 2 }, 10);
  print(\"a={a} b={b}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("coalesce_fault_op", source);
    assert!(
        !diag.contains("Redundant null coalescing"),
        "`??` after an index / division must NOT be flagged redundant; got stderr={diag:?}"
    );
}

// ── @P368b — a non-zero NAMED-CONSTANT divisor must not warn ─────────────────
// `const K = 2.0; x / K` used to fire the divide-by-zero warning (the skip-
// pattern only saw literal divisors).  Const propagation now folds the named
// constant to its literal value, so a non-zero named const divisor is
// skip-pattern-1 — no warning.  A genuine variable divisor still warns.
#[test]
fn skip_named_const_nonzero_divisor() {
    let source = "\
const K = 2.0;
const N = 4;
fn main() {
  x = 10.0; y = 20;
  a = x / K;
  b = y / N;
  print(\"a={a} b={b}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_named_const_div", source);
    assert!(
        !diag.contains("division may produce null"),
        "@P368b: non-zero named-const divisor must NOT warn; got stderr={diag:?}"
    );
    // a genuine variable divisor still warns.
    let source2 = "\
fn main() {
  x = 10.0;
  c = ticks();
  d = x / c;
  print(\"d={d}\\n\");
}
";
    let (_o2, diag2, _c2) = run_with_warnings("named_const_div_neg", source2);
    assert!(
        diag2.contains("division may produce null"),
        "a variable divisor must still warn; got stderr={diag2:?}"
    );
}

// ── Skip pattern 5 — guarded STRUCT FIELD index ─────────────────────────────
// `if i < len(self.v) { self.v[i] }` must be proven safe exactly like the
// bare-local form.  Since a struct vector-field read COPIES (@PLN85 #415),
// value-correct code indexes the field directly rather than a captured alias.

#[test]
fn skip_field_guarded_vec_index() {
    let source = "\
struct S { v: vector<text> not null }
fn rd(self: S, i: integer) -> text {
  if i < len(self.v) { self.v[i] } else { \"\" }
}
fn main() {
  s = S { v: [\"a\", \"b\"] };
  print(\"{rd(s, 0)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_field_guard", source);
    assert!(
        !diag.contains("may produce null on out-of-bounds"),
        "a `if i < len(self.v) {{ self.v[i] }}` guard must NOT warn; got stderr={diag:?}"
    );
}

#[test]
fn skip_field_guarded_captured_len() {
    let source = "\
struct S { v: vector<text> not null }
fn rd(self: S, i: integer) -> text {
  n = len(self.v);
  if i < n { self.v[i] } else { \"\" }
}
fn main() {
  s = S { v: [\"a\"] };
  print(\"{rd(s, 0)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("skip_field_caplen", source);
    assert!(
        !diag.contains("may produce null on out-of-bounds"),
        "captured `n = len(self.v); if i < n {{ self.v[i] }}` must NOT warn; got stderr={diag:?}"
    );
}

#[test]
fn undefended_field_vec_index_warns() {
    let source = "\
struct S { v: vector<text> not null }
fn rd(self: S, i: integer) -> boolean {
  x = self.v[i];
  x != null
}
fn main() {
  s = S { v: [\"a\"] };
  print(\"{rd(s, 0)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("undef_field", source);
    assert!(
        diag.contains("may produce null on out-of-bounds"),
        "an UNguarded `self.v[i]` must still warn; got stderr={diag:?}"
    );
}

#[test]
fn wrong_field_guard_still_warns() {
    // Guard proves `i < len(self.v)` but the index is on a DIFFERENT field
    // `self.w` — the VecKey differs, so the warning must still fire.
    let source = "\
struct S { v: vector<text> not null, w: vector<text> not null }
fn rd(self: S, i: integer) -> text {
  if i < len(self.v) { self.w[i] } else { \"\" }
}
fn main() {
  s = S { v: [\"a\"], w: [\"b\"] };
  print(\"{rd(s, 0)}\\n\");
}
";
    let (_stdout, diag, _code) = run_with_warnings("wrong_field_guard", source);
    assert!(
        diag.contains("may produce null on out-of-bounds"),
        "a guard on a DIFFERENT field must still warn; got stderr={diag:?}"
    );
}

// ── @PLN46 W2 — `#null_safe` param annotation (skip pattern 6) ───────────────

/// A fault-prone expression passed DIRECTLY to a `#null_safe` function is not
/// flagged — the callee contracts to handle the possible null.
#[test]
fn skip_null_safe_arg() {
    let source = "\
fn handles_null(c: character) -> boolean { c == 'a' }
#null_safe
fn scan(line: text, j: integer) -> boolean { handles_null(line[j]) }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w2_null_safe", source);
    assert!(
        !diag.contains("may produce null"),
        "an index passed to a #null_safe param must NOT warn; got stderr={diag:?}"
    );
}

/// The SAME call shape on an UNannotated function still warns — the annotation is
/// the signal, never "the helper happens to avoid the unsafe op".
#[test]
fn null_safe_only_skips_annotated() {
    let source = "\
fn no_annotation(c: character) -> boolean { c == 'a' }
fn scan(line: text, j: integer) -> boolean { no_annotation(line[j]) }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w2_unannotated", source);
    assert!(
        diag.contains("may produce null"),
        "an index passed to an UNannotated param must still warn; got stderr={diag:?}"
    );
}

/// PRECISION: the suppression is direct-argument only — a fault op buried inside a
/// NON-null-safe nested call still warns, even when the OUTER call is `#null_safe`
/// (the inner callee gets the raw null, so the prompt must stand).
#[test]
fn null_safe_does_not_leak_through_nested_call() {
    let source = "\
fn outer(b: boolean) -> boolean { b }
#null_safe
fn raw(c: character) -> boolean { c == 'a' }
fn scan(line: text, j: integer) -> boolean { outer(raw(line[j])) }
fn main() -> integer { 0 }
";
    let (_stdout, diag, _code) = run_with_warnings("w2_nested", source);
    assert!(
        diag.contains("may produce null"),
        "an index inside a nested NON-null-safe call must still warn; got stderr={diag:?}"
    );
}
