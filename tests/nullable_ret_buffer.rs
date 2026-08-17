// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! loft#938 — the opt-in return buffer for a NULLABLE COLLECTION return.
//!
//! `LOFT_NULLABLE_RETBUF=1` gives `-> vector<T>?` the same hidden `__retbuf` the non-nullable
//! form already gets. It is **off by default** because ONE case is still wrong (below), and it
//! is a store read after its free rather than a leak — which is the very thing this issue is
//! about, so it cannot ship on.
//!
//! This file is the basis for finishing it. It pins three things:
//!
//!  * what the switch FIXES, so a later change cannot quietly lose it (`filed_leak_*`,
//!    `mixed_arms_*`, `native_optional_unify_*`, `forwarding_*`, and the whole matrix in
//!    `tests/probes/938-nullable-collection-return-buffer.loft`),
//!  * that the default path is UNCHANGED, so the opt-in stays opt-in (`default_*`),
//!  * what is still broken, named and runnable via `--ignored` (`known_*`).
//!
//! Run the open one with:
//!
//! ```text
//! cargo test --test nullable_ret_buffer -- --ignored --nocapture
//! ```
//!
//! Two method notes for whoever picks this up.
//!
//! `LOFT_TRACE_RETPROMO=1` FIRST. It prints the candidate AND the verdict: no line at all for
//! a function means a gate UPSTREAM of `classify_ret_promotion`, a `Skip*` verdict means the
//! classifier was asked and said no, and those are different bugs the IR does not distinguish.
//! That is how the top-level pass gate was found after three sessions of reading callee bodies.
//!
//! `LOFT_STRICT_STORES=1` SECOND, and not only when something looks wrong. The remaining case
//! answers every value correctly and leaks nothing; it is only visible as a read of a store
//! that was already freed, which is exactly the silent half of this issue's own boundary
//! table. A green run without it is not evidence.

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

/// One function whose arms disagree — a `null` arm, an arm aliasing a PARAMETER and an arm
/// allocating a FRESH store — is the shape that killed the signature-level
/// allocates-vs-aliases flag. The buffer ABI removes the variance instead of deciding it.
#[test]
fn mixed_arms_deliver_into_the_buffer() {
    let (out, err) = run(MIXED, "--interpret", true, "mixed_on");
    assert!(
        out.contains("c=79 base=3 base0=71"),
        "value or container changed: {out}{err}"
    );
    assert!(!leaked(&err), "the fresh arms leak again\n{err}");
}

/// A FORWARDING tail — `fn fwd(i) -> vector<T>? { a1(i) }` — is what the delivery selector in
/// `block_result` never reached, so it compiled to `return null` on `--native` while the
/// interpreter read the freed slot and only looked right.
const FORWARD: &str = r#"
fn a1(i: integer) -> vector<integer>? { if i == 0 { return null } return [i, i + 10]; }
fn tail(i: integer) -> vector<integer>? { a1(i) }
fn expl(i: integer) -> vector<integer>? { return a1(i); }
fn main() {
  c = 0;
  for i in 0..40 { r = tail(i + 1); c += len(r ?? []); }
  for i in 0..40 { r = expl(i + 1); c += len(r ?? []); }
  n = 0;
  if tail(0) == null { n += 1; }
  if expl(0) == null { n += 1; }
  println("c={c} n={n}");
}
"#;

#[test]
fn forwarding_return_delivers_and_keeps_null() {
    for backend in ["--interpret", "--native"] {
        let tag = format!("forward_{}", backend.trim_start_matches('-'));
        let (out, err) = run(FORWARD, backend, true, &tag);
        assert!(
            out.contains("c=160 n=2"),
            "{backend}: a forwarded nullable collection lost its value or its null: {out}{err}"
        );
        assert!(
            !leaked(&err),
            "{backend}: the forwarded store leaked\n{err}"
        );
    }
}

/// The whole boundary matrix, as a runnable program. It lives under `tests/probes/` rather than
/// `tests/scripts/` on purpose: the suite runs `tests/scripts/` on the DEFAULT path, where this
/// program is expected to fail — it is the regression guard for a bug the default still has.
#[test]
fn the_boundary_matrix_passes_on_both_backends() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/probes/938-nullable-collection-return-buffer.loft");
    for backend in ["--interpret", "--native"] {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_TIMEOUT", "300")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_NULLABLE_RETBUF", "1")
            .output()
            .expect("invoke loft");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(all.contains("938 ok"), "{backend}:\n{all}");
        assert!(!leaked(&all), "{backend} leaked:\n{all}");
    }
}

#[test]
fn native_optional_unify_compiles_correctly() {
    let script =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scripts/pln133-optional-unify.loft");
    for backend in ["--interpret", "--native"] {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_TIMEOUT", "300")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_NULLABLE_RETBUF", "1")
            .output()
            .expect("invoke loft");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        // A method returning `vector<T>?` through different routes used to read its element
        // back as 0 where 14 is correct, and only on `--native`.
        assert!(
            all.contains("pln133-optional-unify ok"),
            "{backend} miscompiles a nullable-collection return under the switch\n{all}"
        );
        assert!(!leaked(&all), "{backend} leaked:\n{all}");
    }
}

/// TWO call sites of an enum-dispatching `-> vector<T>?` method, where the `match` forwards to
/// a different implementation per arm. Everything reads correctly and nothing leaks — the only
/// witness is `LOFT_STRICT_STORES=1`, which reports the SECOND site's result being read after a
/// free.
///
/// The mechanism is on the CALLER side and visible in the IR: the callee's return names its
/// `__retbuf` attribute, and at the second site that dep resolves to the FIRST site's variable
/// —
///
/// ```text
/// gd(1):vector<integer>["gd"] = { … t_6AnyVec_take_v(vd(1), 4i32, __ref_3(1)) … }
/// gl(1):vector<integer>["gd"] = { … t_6AnyVec_take_v(vl(1), 6i32, __ref_4(1)) … }
///                     ^^^^ should be __ref_4
/// ```
///
/// so `gl` is typed as a borrow of `gd` and is freed on `gd`'s schedule. It needs BOTH halves —
/// a two-arm dispatch (one arm alone is clean) and two call sites (one site alone is clean) —
/// which is why every smaller probe passes. This is the last thing between the switch and the
/// default.
const TWO_SITE_DISPATCH: &str = r#"
struct VD { step: integer }
struct VL { step: integer }
fn src(i: integer, step: integer) -> vector<integer>? { if i == 0 { return null } return [i, i + step]; }
pub fn take(self: VD, i: integer) -> vector<integer>? { return src(i, self.step); }
pub fn take(self: VL, i: integer) -> vector<integer>? { v = src(i, self.step); return v; }
enum Any { AD { d: VD }, AL { l: VL } }
pub fn take(self: Any, i: integer) -> vector<integer>? {
  match self { AD { d } => d.take(i), AL { l } => l.take(i) }
}
fn main() {
  vd: Any = AD { d: VD { step: 10 } };
  vl: Any = AL { l: VL { step: 20 } };
  gd = vd.take(4) ?? [];
  gl = vl.take(6) ?? [];
  println("gd={gd[1] ?? -1} gl={gl[1] ?? -1}");
}
"#;

#[test]
#[ignore = "loft#938 open: two call sites of a two-arm dispatch share one return dep"]
fn known_two_site_dispatch_reads_a_freed_store() {
    let dir = std::env::temp_dir().join(format!("loft938_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let file = dir.join("two_site.loft");
    std::fs::write(&file, TWO_SITE_DISPATCH).expect("write probe");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_NULLABLE_RETBUF", "1")
        .env("LOFT_STRICT_STORES", "1")
        .output()
        .expect("invoke loft");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The VALUES are right, which is the trap: only the strict-store oracle sees it.
    assert!(all.contains("gd=14 gl=26"), "values changed:\n{all}");
    assert!(
        !all.contains("USE AFTER FREE"),
        "the second site still borrows the first site's buffer\n{all}"
    );
}
