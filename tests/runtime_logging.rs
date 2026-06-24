// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-07 phase 4 production-mode tests.
//!
//! Asserts the production-mode log-and-continue contract from
//! [`DESIGN_DECISIONS.md § C66`](../doc/claude/DESIGN_DECISIONS.md#c66--production-loft-programs-never-abort-on-user-attributable-edge-cases-development-may-halt):
//! when the loft binary is invoked with `--production` and a logger
//! is attached, runtime fault sites (panic / assert / div / mod /
//! vec OOB / text OOB / null deref / narrow cast / stack overflow)
//! MUST log the typed event AND let execution continue with the
//! existing silent sentinel — they MUST NOT halt mid-program.
//!
//! Each cell:
//! - writes a loft script to a tempdir
//! - drops a `log.conf` beside it pointing at a known `log.txt` path
//! - invokes `loft --interpret --production <script>`
//! - asserts post-fault stdout reached (program continued past the
//!   fault site)
//! - asserts the log file contains the expected runtime-event entry
//!   shape (`[<kind_label>] <description>`)
//! - asserts the process exits 1 (had_fatal still triggers exit 1
//!   so the host knows something bad happened — the program
//!   completed without halt, then exits at end)
//!
//! Dev-mode behaviour (halt + render) is covered separately by
//! `tests/runtime_errors.rs`.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Set up a tempdir + script + log.conf for a production-mode run.
/// Returns (script_path, log_path).  The log.conf points at the
/// log_path so we can inspect captured entries afterwards.
fn setup_prod_run(name: &str, source: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!(
        "loft_prod_{}_{}_{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create tempdir");
    let script_path = dir.join(format!("{name}.loft"));
    std::fs::write(&script_path, source).expect("write script");
    let log_path = dir.join("log.txt");
    let conf = format!("[log]\nfile = {}\nlevel = info\n", log_path.display());
    std::fs::write(dir.join("log.conf"), conf).expect("write log.conf");
    (script_path, log_path)
}

/// Run a script under `--interpret --production` and return
/// (stdout, stderr, exit-code, captured-log).
fn run_prod(name: &str, source: &str) -> (String, String, Option<i32>, String) {
    let (script_path, log_path) = setup_prod_run(name, source);
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--production")
        .arg(&script_path)
        .current_dir(workspace_root())
        .output()
        .expect("invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(script_path.parent().unwrap());
    (stdout, stderr, out.status.code(), log)
}

/// Production: `panic("msg")` logs Fatal + continues + exits 1
/// (had_fatal).  Pre-panic stdout reaches; post-panic stdout
/// also reaches because the program continues past the panic call.
#[test]
fn prod_user_panic_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  panic(\"boom\");
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("user_panic", source);
    assert_eq!(code, Some(1), "had_fatal should still exit 1");
    assert!(
        stdout.contains("before"),
        "pre-panic stdout missing; got {stdout:?}"
    );
    assert!(
        stdout.contains("after"),
        "production must continue past panic; post-panic stdout missing; got {stdout:?}"
    );
    assert!(
        log.contains("FATAL") && log.contains("[user_panic]") && log.contains("boom"),
        "log missing user_panic Fatal entry; got {log:?}"
    );
}

/// Production: failed `assert(false, "msg")` logs Error + continues + exits 1.
#[test]
fn prod_assertion_failed_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  assert(2 + 2 == 5, \"math is broken\");
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("assertion_failed", source);
    assert_eq!(code, Some(1), "had_fatal should still exit 1");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "production must continue past failed assert; got {stdout:?}"
    );
    assert!(
        log.contains("ERROR")
            && log.contains("[assertion_failed]")
            && log.contains("math is broken"),
        "log missing assertion_failed Error entry; got {log:?}"
    );
}

/// C80 / E-Uncomp: an UNGUARDED integer `/` by zero is a recoverable
/// calculation fault — it logs a Warn (the "no guard" report) + returns the
/// null sentinel + CONTINUES, mode-independently (exit 0, like OOB below).  A
/// `?? null` / null-check guard would emit the silent `*Nullable` op instead.
#[test]
fn prod_divide_by_zero_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  a = 10;
  b = 0;
  c = a / b;
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("divide_by_zero", source);
    assert_eq!(code, Some(0), "recoverable div-by-zero should exit 0 (C80)");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "must continue past divide-by-zero; got {stdout:?}"
    );
    assert!(
        log.contains("WARN") && log.contains("[divide_by_zero]"),
        "unguarded div-by-zero must report a Warn log; got {log:?}"
    );
}

/// Production: vector positive-index OOB logs Warn + returns null DbRef
/// sentinel + continues.  The post-fault read of `x.something` would
/// see null too, so just verify the post-fault stdout reaches.
#[test]
fn prod_index_out_of_bounds_vector_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  v = [10, 20, 30];
  _x = v[5];
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("index_out_of_bounds_vector", source);
    // @P356: OOB index is a RECOVERABLE fault — logs a Warn and continues
    // without failing the program (exit 0).  Fail-fast is opt-in via
    // `LOFT_DEV_SOFT_HALT` only.
    assert_eq!(code, Some(0), "recoverable OOB should exit 0");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "production must continue past OOB; got {stdout:?}"
    );
    assert!(
        log.contains("WARN")
            && log.contains("[index_out_of_bounds]")
            && log.contains("index 5 out of bounds for length 3"),
        "log missing OOB Warn entry; got {log:?}"
    );
}

/// Production: text positive-index OOB logs Warn + returns char(0) +
/// continues.
#[test]
fn prod_index_out_of_bounds_text_logs_and_continues() {
    let source = "\
fn main() {
  print(\"before\\n\");
  s = \"abc\";
  _c = s[100];
  print(\"after\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("index_out_of_bounds_text", source);
    // @P356: recoverable text OOB logs Warn + continues; exit 0.
    assert_eq!(code, Some(0), "recoverable text OOB should exit 0");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "production must continue past text OOB; got {stdout:?}"
    );
    assert!(
        log.contains("WARN")
            && log.contains("[index_out_of_bounds]")
            && log.contains("index 100 out of bounds for length 3"),
        "log missing text-OOB Warn entry; got {log:?}"
    );
}

/// Phase 4d.1 — `expr ?? fallback` defense suppresses both the
/// runtime log AND the program-end exit code.  Codegen swaps the
/// outer fault-prone op to its `Nullable` peer (which silently
/// returns the sentinel without calling `s.raise`), then `??`
/// discharges the null with the fallback.  No log noise, exit 0.
/// Mirrors the existing C54.G-hybrid behaviour for `a / b ?? 0`.
#[test]
fn prod_vector_oob_with_nullable_rescue_does_not_log() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5] ?? null;
  if x == null { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("vector_oob_nullable_rescue", source);
    assert_eq!(code, Some(0), "?? rescue should exit 0; log={log:?}");
    assert!(
        stdout.contains("rescued"),
        "?? rescue should produce the fallback; got {stdout:?}"
    );
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "?? rescue must NOT emit log entry; got {log:?}"
    );
}

/// Phase 4d.1 — same shape for text indexing.  `s[i] ?? null` on
/// OOB silently produces null.
#[test]
fn prod_text_oob_with_nullable_rescue_does_not_log() {
    let source = "\
fn main() {
  s = \"abc\";
  c = s[100] ?? null;
  if c == null { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("text_oob_nullable_rescue", source);
    assert_eq!(code, Some(0), "?? rescue should exit 0; log={log:?}");
    assert!(stdout.contains("rescued"));
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "?? rescue must NOT emit log entry; got {log:?}"
    );
}

/// Phase 4d.1 — struct-ref vector `vp[i] ?? null` (uses
/// `OpVectorRef`, not `OpGetVector`) — confirms the dispatch covers
/// both index opcodes.
#[test]
fn prod_struct_ref_vector_oob_with_nullable_rescue_does_not_log() {
    let source = "\
struct P { v: integer }
fn main() {
  vp = [P{v:1}, P{v:2}];
  q = vp[5] ?? null;
  if q == null { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("struct_ref_oob_nullable_rescue", source);
    assert_eq!(code, Some(0), "?? rescue should exit 0; log={log:?}");
    assert!(stdout.contains("rescued"));
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "?? rescue must NOT emit log entry; got {log:?}"
    );
}

/// Phase 4d.2 — bare `if x != null { … }` after `x = v[i]`
/// detected as a defensive check; codegen swaps `OpGetVector` to
/// its `Nullable` peer at compile time so neither log nor halt
/// fires.  Mode-independent (works in production AND dev) because
/// the swap happens BEFORE the runtime mode branch.
#[test]
fn prod_vector_oob_with_bare_null_check_does_not_log() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5];
  if x != null { print(\"hit\\n\"); }
  else { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("vector_oob_bare_null_check", source);
    assert_eq!(code, Some(0), "bare null-check should exit 0; log={log:?}");
    assert!(
        stdout.contains("rescued"),
        "defensive null-check should fire; got {stdout:?}"
    );
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "defended site must NOT emit log entry; got {log:?}"
    );
}

/// Phase 4d.2 — same shape but using bare truthy `if x { … }`
/// (loft's truthy check is `false` for null DbRef, so this also
/// counts as a defensive check).
#[test]
fn prod_vector_oob_with_bare_truthy_check_does_not_log() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  x = v[5];
  if x { print(\"hit\\n\"); }
  else { print(\"rescued\\n\"); }
}
";
    let (stdout, _stderr, code, log) = run_prod("vector_oob_bare_truthy_check", source);
    assert_eq!(
        code,
        Some(0),
        "bare truthy-check should exit 0; log={log:?}"
    );
    assert!(
        stdout.contains("rescued"),
        "defensive truthy-check should fire; got {stdout:?}"
    );
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "defended site must NOT emit log entry; got {log:?}"
    );
}

/// Production: a clean run produces no runtime-event log entries.
/// Phase 4e.1 — format-string interpolation MUST NEVER halt, log, or
/// warn even when the interpolated expression is undefended.  Format
/// strings are the developer's debugging surface (per C66 + chat
/// 2026-05-11): the `println("{x}")` you reach for to inspect a bug
/// must not itself become the next bug.  Every fault-prone op inside
/// `{...}` is auto-swapped to its Nullable peer at parse time.
#[test]
fn prod_format_string_div_by_zero_does_not_log() {
    let source = "\
fn main() {
  z = 0;
  print(\"a={1 / z}\\n\");
  print(\"b={2 % z}\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("fmt_div_zero", source);
    assert_eq!(code, Some(0), "format-string div should not halt");
    assert!(stdout.contains("a=null"), "got {stdout:?}");
    assert!(stdout.contains("b=null"), "got {stdout:?}");
    assert!(
        !log.contains("[divide_by_zero]"),
        "format-string div MUST NOT log; got {log:?}"
    );
}

/// Phase 4e.1 — format-string vector OOB must not halt or log.
#[test]
fn prod_format_string_vector_oob_does_not_log() {
    let source = "\
fn main() {
  v = [10, 20, 30];
  print(\"v[999]={v[999]}\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("fmt_vec_oob", source);
    assert_eq!(code, Some(0), "format-string OOB should not halt");
    assert!(stdout.contains("v[999]=null"), "got {stdout:?}");
    assert!(
        !log.contains("[index_out_of_bounds]"),
        "format-string OOB MUST NOT log; got {log:?}"
    );
}

/// Guards against the production code emitting spurious log entries
/// for normal program execution.
#[test]
fn prod_clean_run_logs_nothing_runtime() {
    let source = "\
fn main() {
  print(\"hello\\n\");
}
";
    let (stdout, _stderr, code, log) = run_prod("clean_run", source);
    assert_eq!(code, Some(0), "clean run should exit 0");
    assert!(stdout.contains("hello"));
    // No runtime-event entries should appear.  (The log file may not
    // even exist if nothing ever logged.)
    assert!(
        !log.contains("[user_panic]")
            && !log.contains("[assertion_failed]")
            && !log.contains("[divide_by_zero]")
            && !log.contains("[index_out_of_bounds]"),
        "clean run produced spurious runtime-event log; got {log:?}"
    );
}
