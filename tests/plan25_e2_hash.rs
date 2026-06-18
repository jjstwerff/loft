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
//! Both backends: the `--native` codegen of a `hash<__nullable<S>>` is fixed
//! (rung 4 — the synth enum's `Some`/`Null` variant structures are built
//! up-front so their type-ids precede the consuming struct, matching the order
//! the native emitter replays; plus `__nullable<S>`-as-bool coerces to its
//! present-check). `--native` runs are skipped when `rustc` is unavailable.
//!
//! `LOFT_NO_CACHE=1` is mandatory: the warm program cache keys on source, NOT
//! on `LOFT_E2_SYNTH`, so a cached gate-off bundle would mask the rewrite.

use std::path::Path;
use std::process::Command;

/// Run the loft binary on `prog`, gate-on, cache-off, with extra `args`.
fn run(args: &[&str], prog: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .args(args)
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

fn rustc_available() -> bool {
    Command::new("rustc").arg("--version").output().is_ok()
}

/// Assert `want` appears in stdout on the interpreter AND (when rustc is
/// present) on `--native` — the keyed-collection-over-nullable path must agree
/// across backends.
fn assert_both(prog: &Path, want: &str) {
    let (ok, out) = run(&["--interpret"], prog);
    assert!(ok, "interpret run failed; stdout={out:?}");
    assert!(
        out.contains(want),
        "interpret missing {want:?}; got {out:?}"
    );
    if rustc_available() {
        let (ok, out) = run(&["--native"], prog);
        assert!(ok, "native run failed; stdout={out:?}");
        assert!(out.contains(want), "native missing {want:?}; got {out:?}");
    }
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
    // len preserved; key extraction finds the shared Some-wrapped records by
    // the `t` field through the Some payload; field access unwraps to S.
    assert_both(&probe("counting", SRC), "len=3 three=3 one=1 missing=true");
}

const SRC_ANON_FORLOOP: &str = "\
struct Count { t: text, v: integer }
struct Counting { entries: vector<Count>, lookup: hash<Count[t]> }
fn fill(c: Counting) {
  c.entries = [ {t:\"One\",v:1}, {t:\"Two\",v:2}, {t:\"Three\",v:3}, {t:\"Four\",v:4} ]
}
fn main() {
  c = Counting { };
  fill(c);
  add = 0;
  for item in c.entries { add += item.v; }
  print(\"add={add} three={c.lookup[\\\"Three\\\"].v}\\n\");
}
";

#[test]
fn anon_literal_into_hash_field_and_forloop_over_shared_array() {
    // Rung 5: anon `{ … }` literals into a hash field (the element resolves
    // against the `Some` variant, not the enum). For-loop deref: iterating the
    // struct-field nullable vector — stored as a LINKED array of ref slots that
    // shares records with the hash — must DEREF each element (OpVectorRefNullable)
    // before reading `item.v`, not read the rec-id slot as the record inline.
    assert_both(&probe("anon_forloop", SRC_ANON_FORLOOP), "add=10 three=3");
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
    assert_both(&path, "missing=true");
}
