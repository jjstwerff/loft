// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#938 — the opt-in return buffer for a NULLABLE COLLECTION return.
//!
//! `LOFT_NULLABLE_RETBUF=1` gives `-> vector<T>?` the same hidden `__retbuf` the non-nullable
//! form already gets. It is **off by default** because two cases are still wrong (below), and
//! one of them is a `--native` wrong ANSWER rather than a leak.
//!
//! This file is the basis for finishing it. It pins three things:
//!
//!  * what the switch FIXES, so a later change cannot quietly lose it (`filed_leak_*`),
//!  * that the default path is UNCHANGED, so the opt-in stays opt-in (`default_*`),
//!  * what is still broken, named and runnable via `--ignored` (`known_*`).
//!
//! Run the open ones with:
//!
//! ```text
//! cargo test --test nullable_ret_buffer -- --ignored --nocapture
//! ```
//!
//! Method note for whoever picks this up: reach for `LOFT_TRACE_RETPROMO=1` FIRST. No output
//! for a function means a gate UPSTREAM of `classify_ret_promotion`, which is a different bug
//! from the classifier deciding wrong — and not a distinction the IR shows. That is how the
//! top-level pass gate was found after three sessions of reading callee bodies.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    let mut p = std::env::current_exe().expect("test binary path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("loft")
}

/// Run `src` on `backend`, with the switch on or off. Returns `(stdout, stderr)`.
///
/// `tag` names the probe's own file. Tests in one binary share a PID, so keying the scratch
/// path on that alone let them clobber each other's source — a failure that reproduced only
/// in the batch and passed when the test was run alone.
fn run(src: &str, backend: &str, retbuf: bool, tag: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!("loft938_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join(format!("{tag}.loft"));
    std::fs::write(&file, src).expect("write probe");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(&file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1");
    if retbuf {
        cmd.env("LOFT_NULLABLE_RETBUF", "1");
    }
    let out = cmd.output().expect("invoke loft");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn leaked(stderr: &str) -> bool {
    stderr.contains("not freed")
}

/// The filed shape: an unbound `f(i) != null` in a loop. The result is never bound, so the
/// comparison captures it in a work-ref whose free is emitted ONCE at scope exit while each
/// turn overwrites the slot. 39 of 40 stores are orphaned, and the count scales with the
/// iteration count — the unbounded shape of loft#688.
const FILED: &str = r#"
fn gives(n: integer) -> vector<integer>? { if n != 0 { [n] } else { null } }
fn main() {
  c = 0;
  for i in 0..40 { if gives(i + 1) != null { c += 1; } }
  println("c={c}");
}
"#;

/// A `-> vector<T>?` returning a BORROWED view of a parameter. The constraint on any fix: the
/// caller's collection must survive, so this must not become an over-free once the leak above
/// is closed. It is the row that says "free unconditionally" is not the answer.
const BORROWED: &str = r#"
fn view(v: vector<integer>, n: integer) -> vector<integer>? { if n != 0 { v } else { null } }
fn main() {
  base = [71, 82, 93];
  c = 0;
  for i in 0..40 { if view(base, i + 1) != null { c += 1; } }
  println("c={c} base={len(base)} base0={base[0]}");
}
"#;

/// One function whose arms disagree: a `null` arm, an arm aliasing a PARAMETER, and an arm
/// allocating a FRESH store. The null arm forces the `__ret_N` merge before promotion sees
/// the arms, so none of them is a return tail and none delivers into the buffer.
const MIXED: &str = r#"
fn pick(v: vector<integer>, n: integer) -> vector<integer>? {
  if n == 0 { null } else if n == 1 { v } else { [n, n + 1] }
}
fn main() {
  base = [71, 82, 93];
  c = 0;
  for i in 0..40 { r = pick(base, i); c += len(r ?? []); }
  println("c={c} base={len(base)} base0={base[0]}");
}
"#;

#[test]
fn filed_leak_is_fixed_with_the_switch_on() {
    let (out, err) = run(FILED, "--interpret", true, "filed_on");
    assert!(out.contains("c=40"), "value changed: {out}{err}");
    assert!(
        !leaked(&err),
        "loft#938's filed leak is back with LOFT_NULLABLE_RETBUF=1\n{err}"
    );
}

#[test]
fn borrowed_view_is_not_over_freed_with_the_switch_on() {
    let (out, err) = run(BORROWED, "--interpret", true, "borrowed_on");
    assert!(
        out.contains("c=40 base=3 base0=71"),
        "the caller's collection did not survive — a free became an over-free: {out}{err}"
    );
    assert!(!leaked(&err), "borrowed view leaked instead\n{err}");
}

#[test]
fn default_path_is_unchanged() {
    // The switch is opt-in, so with it OFF the filed leak is still there. Pinning it keeps
    // the two paths honestly separated: if this ever passes, the fix has been defaulted on
    // and the `known_*` cases below must be green first.
    let (out, err) = run(FILED, "--interpret", false, "filed_off");
    assert!(
        out.contains("c=40"),
        "value changed with the switch off: {out}"
    );
    assert!(
        leaked(&err),
        "the filed leak is gone with LOFT_NULLABLE_RETBUF unset — if that is deliberate, \
         default the switch on and delete this test"
    );
}

#[test]
#[ignore = "loft#938 open: a mixed null/alias/fresh function leaks its fresh arms"]
fn known_mixed_arms_still_leak() {
    let (out, err) = run(MIXED, "--interpret", true, "mixed_on");
    assert!(
        out.contains("c=79 base=3 base0=71"),
        "value or container changed: {out}{err}"
    );
    assert!(
        !leaked(&err),
        "the fresh arms still leak — the arms merge into `__ret_N` before promotion sees \
         them, so none is a return tail and none delivers into the buffer\n{err}"
    );
}

#[test]
#[ignore = "loft#938 open: pln133-optional-unify miscompiles on --native with the switch on"]
fn known_native_optional_unify_miscompiles() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/pln133-optional-unify.loft");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--native")
        .arg(&script)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NULLABLE_RETBUF", "1");
    let out = cmd.output().expect("invoke loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // A method returning `vector<T>?` through different routes reads its element back as 0
    // where 14 is correct — and only on `--native`; the interpreter passes. A silent wrong
    // answer on one backend is why this switch is not defaulted on.
    assert!(
        all.contains("pln133-optional-unify ok"),
        "native miscompiles a nullable-collection return under the switch\n{all}"
    );
}
