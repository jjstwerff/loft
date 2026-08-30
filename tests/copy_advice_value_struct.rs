// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! loft#1190 — `advice[avoidable-copy]` is the BORROW worklist, so it is silent where no
//! borrow exists.
//!
//! A `value struct` (@PLN101) is stored INLINE wherever it lives, so writing one into a
//! container writes its bytes into storage the container already has: nothing is allocated,
//! and the rewrite the advice names buys exactly zero.
//!
//! **This is the gate; the `.loft` guard beside it is not.** The defect is a diagnostic that
//! must NOT fire, and neither channel the corpus scores can see that — `check_diagnostics`
//! logs an unexpected warning without failing ("Other warnings are not fatal"), and
//! `make falsify` compares exit / assertions / leak / panic / refusals, none of which move
//! when a notice appears. So the absence is asserted HERE, by counting the notices on stderr,
//! on both backends. `tests/scripts/1190-an-inline-value-copy-has-no-advice.loft` carries the
//! VALUE half — that the suppression did not turn the copy into an alias — which the corpus
//! does score.
//!
//! Why the binary and not the in-process harness: these are end-to-end compile diagnostics on
//! stderr, the same approach `tests/dead_code_lint.rs` takes, and `LOFT_NO_CACHE` is needed
//! because a warm run skips the re-parse that emits them.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// The program under test: three inline-value copies that must be silent, and two copies that
/// must still be advised.
const PROGRAM: &str = r#"
value struct Stamp { ms: integer, tick: integer }
value struct Wrapped { at: Stamp, id: integer }
value struct Heapy { xs: vector<integer>, n: integer }
struct Recorded { ms: integer }

fn main() {
  a = Stamp { ms: 1, tick: 10 };
  v = [a];
  print("{v[0].ms}{a.tick}");

  s = Stamp { ms: 7, tick: 70 };
  w = Wrapped { at: s, id: 3 };
  print("{w.at.ms}{s.tick}");

  n = Wrapped { at: s, id: 4 };
  ns = [n];
  print("{ns[0].at.tick}{n.id}");

  h = Heapy { xs: [1, 2], n: 1 };
  hv = [h];
  print("{len(hv[0].xs)}{h.n}");

  r = Recorded { ms: 1 };
  rv = [r];
  print("{rv[0].ms}{r.ms}\n");
}
"#;

fn run(backend: &str, path: &PathBuf) -> (String, String, Option<i32>) {
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(path)
        .env("LOFT_TIMEOUT", "180")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

fn write_program(dir: &PathBuf) -> PathBuf {
    std::fs::create_dir_all(dir).expect("scratch dir");
    let path = dir.join("value_struct_copy.loft");
    let mut f = std::fs::File::create(&path).expect("write program");
    f.write_all(PROGRAM.as_bytes()).expect("write program");
    path
}

fn assert_backend(backend: &str) {
    let dir = std::env::temp_dir().join(format!(
        "loft_1190_copy_advice_{}",
        backend.trim_matches('-')
    ));
    let path = write_program(&dir);
    let (stdout, diag, code) = run(backend, &path);

    assert_eq!(
        code,
        Some(0),
        "[{backend}] the program must run\n{stdout}\n---\n{diag}"
    );
    assert!(
        stdout.contains("11"),
        "[{backend}] the program must reach its output\n{stdout}"
    );

    // The two that MUST still be advised. `Heapy` is the control that says the marker is not
    // the whole answer: `value struct` says how the value is STORED, not what it OWNS, and a
    // `vector` field beneath one is a real duplicated store. `Recorded` is a plain struct,
    // which lives in a record reached by a `DbRef`, so its copy allocates one even though
    // every field is a scalar.
    for want in ["copy of Heapy", "copy of Recorded"] {
        assert!(
            diag.contains(want),
            "[{backend}] `{want}` must still be advised\n{diag}"
        );
    }

    // The three that must be SILENT — an inline value with no heap part anywhere beneath it,
    // as an element, as a field, and nested two deep.
    for unwanted in ["copy of Stamp", "copy of Wrapped"] {
        assert!(
            !diag.contains(unwanted),
            "[{backend}] `{unwanted}` allocates nothing — there is no borrow to advise\n{diag}"
        );
    }

    // Counted as well as named: a notice that moved to another type, or a second one on the
    // same type, is a regression this file exists to catch.
    let n = diag.matches("advice[avoidable-copy]").count();
    assert_eq!(
        n, 2,
        "[{backend}] want exactly 2 avoidable-copy notices (Heapy, Recorded), got {n}\n{diag}"
    );
}

#[test]
fn inline_value_copies_are_not_advised_interpret() {
    assert_backend("--interpret");
}

#[test]
fn inline_value_copies_are_not_advised_native() {
    assert_backend("--native");
}
