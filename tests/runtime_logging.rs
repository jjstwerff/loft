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

/// Production: integer `/` by zero logs Warn + returns null sentinel +
/// continues.  Downstream `?? null` rescue or null-check defends; here
/// we just print the result and confirm the post-fault path runs.
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
    assert_eq!(code, Some(1), "had_fatal should still exit 1");
    assert!(stdout.contains("before"));
    assert!(
        stdout.contains("after"),
        "production must continue past divide-by-zero; got {stdout:?}"
    );
    assert!(
        log.contains("WARN") && log.contains("[divide_by_zero]"),
        "log missing divide_by_zero Warn entry; got {log:?}"
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
    assert_eq!(code, Some(1), "had_fatal should still exit 1");
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
    assert_eq!(code, Some(1), "had_fatal should still exit 1");
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

/// Production: a clean run produces no runtime-event log entries.
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
