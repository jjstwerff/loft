// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! @PLN13 phase 6 (diagnostics slice) — an unresolved bare call names the
//! package that provides it.
//!
//! Calling a library's free function without a `use` used to end at
//! `Unknown function rand` with no route forward, even though the registry index
//! knows exactly which package exports `rand`.  Bare calls still do not RESOLVE
//! (that is the rest of phase 6, and it has to settle stdlib shadowing first);
//! this only replaces the dead end with the two ways to say what was meant.
//!
//! Every case runs against a FAKE `LOFT_HOME` holding a hand-written index, so
//! the assertions never depend on what the developer happens to have cached.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    workspace_root().join("target/release/loft")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A cached index exporting `rand` (free fn, package `random`), `matches`
/// (a METHOD — must never be offered for a bare call) and `dup` from two
/// packages at once.
const INDEX: &str = r#"{
  "schema_version": 1,
  "updated": "2026-07-23",
  "packages": {
    "random": {
      "name": "random",
      "versions": {
        "0.3.0": {
          "version": "0.3.0",
          "url": "https://example.invalid/random-0.3.0.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1,
          "loft": "2026.7.2",
          "published": "2026-07-23",
          "api": [
            { "sig": "pub fn rand(low: integer, high: integer) -> integer", "doc": "A number." },
            { "sig": "pub fn matches(self: text, pattern: text) -> boolean", "doc": "A method." }
          ]
        }
      }
    },
    "dicer": {
      "name": "dicer",
      "versions": {
        "0.1.0": {
          "version": "0.1.0",
          "url": "https://example.invalid/dicer-0.1.0.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1,
          "loft": "2026.7.2",
          "published": "2026-07-23",
          "api": [
            { "sig": "pub fn dup(v: integer) -> integer", "doc": "Also here." }
          ]
        }
      }
    },
    "duper": {
      "name": "duper",
      "versions": {
        "0.1.0": {
          "version": "0.1.0",
          "url": "https://example.invalid/duper-0.1.0.tar.gz",
          "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
          "size": 1,
          "loft": "2026.7.2",
          "published": "2026-07-23",
          "api": [
            { "sig": "pub fn dup(v: integer) -> integer", "doc": "And here." }
          ]
        }
      }
    }
  }
}"#;

/// Compile `src` against a fake registry home and return the diagnostics.
fn diagnostics_for(tag: &str, src: &str, with_index: bool) -> String {
    let pid = std::process::id();
    let home = std::env::temp_dir().join(format!("loft_fnhint_{tag}_{pid}"));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".loft/registry")).expect("mkdir registry");
    if with_index {
        std::fs::write(home.join(".loft/registry/index.json"), INDEX).expect("write index");
    }
    let prog = home.join("prog.loft");
    std::fs::write(&prog, src).expect("write program");

    let out = Command::new(loft_bin())
        .args(["--interpret", "--errors=compact"])
        .arg(&prog)
        .env("LOFT_HOME", &home)
        .output()
        .expect("invoke loft");
    let text =
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
    let _ = std::fs::remove_dir_all(&home);
    text
}

#[test]
fn unresolved_bare_call_names_the_package_that_provides_it() {
    let d = diagnostics_for("one", "fn main() {\n    v = rand(1, 100);\n}\n", true);
    assert!(
        d.contains("random::rand"),
        "the message should point at the qualified call; got:\n{d}"
    );
    assert!(
        d.contains("use random;"),
        "the message should offer the `use` form too; got:\n{d}"
    );
}

/// A published METHOD is not something a bare call could have meant — and it
/// already resolves without a `use` through the lazy-load triggers — so it must
/// not be offered here.
#[test]
fn a_published_method_is_not_offered_for_a_bare_call() {
    let d = diagnostics_for(
        "method",
        "fn main() {\n    v = matches(\"a\", \"b\");\n}\n",
        true,
    );
    assert!(
        !d.contains("random::matches"),
        "a method must not be suggested as a bare qualified call; got:\n{d}"
    );
}

/// Two packages exporting the same name: name both rather than pick one.
#[test]
fn an_ambiguous_name_lists_every_provider() {
    let d = diagnostics_for("dup", "fn main() {\n    v = dup(2);\n}\n", true);
    assert!(
        d.contains("`dicer`") && d.contains("`duper`"),
        "both providers should be named; got:\n{d}"
    );
}

/// No cached index (a fresh machine, or a registry-less build): the diagnostic
/// degrades to the plain message instead of erroring or stalling.
#[test]
fn no_cached_index_still_reports_the_plain_error() {
    let d = diagnostics_for("noidx", "fn main() {\n    v = rand(1, 100);\n}\n", false);
    assert!(
        d.contains("Unknown function rand"),
        "the plain error must survive; got:\n{d}"
    );
    assert!(
        !d.contains("provides it"),
        "no index means no hint; got:\n{d}"
    );
}
