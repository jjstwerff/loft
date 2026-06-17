// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN25 E2 / A3 — a `hash<S[k]>` whose element shares records with a sibling
//! `vector<S>` (the "two views, one record set" pattern). Gate-on, both views
//! are rewritten to the synthetic `__nullable<S>` enum, so the key fields live
//! in the `Some` payload, not at the enum's top level.
//!
//! These guard the key-resolution + lookup rungs that are FIXED on the
//! interpreter:
//!  - rung 1: `key_owner` resolves key field numbers/positions through `Some`
//!    (`Stores::hash`, `create_key`, `determine_keys`, `field_content`);
//!  - rung 2: `get_keys` (the fifth `key_owner` site) returns the key TYPES so
//!    `read_key` pops the right bytes — otherwise the stack misaligns and the
//!    hash-container ref reads as junk (`hash::find` panics on a bad store_nr);
//!  - rung 3: `index_type` keeps the synth-`__nullable<S>` lookup result as
//!    `Type::Enum(_, true)` (like a vector element) so `lookup[k].field`
//!    auto-unwraps through `Some` instead of erroring `Unknown field`.
//!
//! Interpreter-only for now: the `--native` codegen of a `hash<__nullable<S>>`
//! is rung 4, still open (see the plan README), so do NOT add `--native` here
//! until it lands.
//!
//! `LOFT_NO_CACHE=1` is mandatory: the warm program cache keys on source, NOT
//! on `LOFT_E2_SYNTH`, so a cached gate-off bundle would mask the rewrite.

use std::path::Path;
use std::process::Command;

/// Run the loft binary on `prog`, interpreter backend, gate-on, cache-off.
fn run_interp(prog: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(prog)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_E2_SYNTH", "1")
        .output()
        .expect("spawn loft binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn probe(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("loft_e2_hash_probe");
    std::fs::create_dir_all(&dir).expect("probe dir");
    let path = dir.join(format!("{name}.loft"));
    std::fs::write(&path, body).expect("write probe");
    path
}

const SRC: &str = "\
struct Count { t: text, v: integer }
struct Counting { entries: vector<Count>, lookup: hash<Count[t]> }
fn fill(c: Counting) {
  c.entries = [ Count{t: \"One\", v: 1}, Count{t: \"Two\", v: 2}, Count{t: \"Three\", v: 3} ];
}
fn main() {
  c = Counting { };
  fill(c);
  print(\"len={len(c.entries)} \");
  print(\"three={c.lookup[\\\"Three\\\"].v} one={c.lookup[\\\"One\\\"].v} \");
  print(\"missing={c.lookup[\\\"Five\\\"] == null}\\n\");
}
";

#[test]
fn hash_over_nullable_vector_lookup_and_field_access() {
    let path = probe("counting", SRC);
    let (ok, out) = run_interp(&path);
    assert!(ok, "interpret run failed; stdout={out:?}");
    // len preserved; key extraction finds the shared Some-wrapped records by
    // the `t` field through the Some payload; field access unwraps to S.
    assert!(
        out.contains("len=3 three=3 one=1 missing=true"),
        "hash lookup over nullable vector wrong; got {out:?}"
    );
}

#[test]
fn lone_nullable_hash_field_constructs_and_misses_cleanly() {
    // A lone hash field on a fresh struct (no insert): the missing-key lookup
    // must return null, not panic on a misread container ref (rung 2).
    let path = probe(
        "lone",
        "struct Count { t: text, v: integer }\n\
         struct Box { lookup: hash<Count[t]> }\n\
         fn main() {\n  \
           c = Box { };\n  \
           print(\"missing={c.lookup[\\\"x\\\"] == null}\\n\");\n\
         }\n",
    );
    let (ok, out) = run_interp(&path);
    assert!(ok, "interpret run failed; stdout={out:?}");
    assert!(
        out.contains("missing=true"),
        "lone nullable hash field lookup wrong; got {out:?}"
    );
}
