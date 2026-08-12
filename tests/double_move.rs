// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN139 stage G — the `double-move` lint.
//!
//! @PLN139 made a copy into a container a MOVE: the container owns the value and its death
//! releases it. That closed loft#849 — and turned a shape that used to LEAK into a double
//! close, because `c = mk(); s1 = S { h: c }; s2 = S { h: c }` now hands one resource to two
//! owners and both release it. Rust prevents this with move checking, which loft does not
//! have, so a diagnostic catches it instead.
//!
//! Every cell asserts TWO things: whether the lint fires, and how many times the value is
//! actually released. That pairing is the point. A verdict-only test cannot tell a correct
//! silence from a missed defect, and it is exactly the silent cells that a future widening of
//! the transfer rule would break — so each of them pins the release count that makes its
//! silence correct. The two cells that release twice WITHOUT a warning (`m8`, `m13`) are the
//! lint's documented blind spot, pinned here so it stays a known boundary rather than drifting
//! into an unnoticed one.
//!
//! Binary-level, because the lint runs post-`scopes::check` from `main` (beside the dead-store
//! lint) and only a real invocation reaches it.

use std::process::Command;

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// The shared preamble: a droppable that announces every release, and a container.
const PRELUDE: &str = "\
struct H { id: integer }
fn OpDrop(self: H) { println(\"DROP:{self.id}\"); }
fn mk(id: integer) -> H { return H { id: id }; }
struct S { h: H }
struct Nest { s: S }
";

/// Run one cell and answer `(double_move_warnings, releases)`.
fn cell(name: &str, body: &str) -> (usize, usize) {
    let src = format!("{PRELUDE}\nfn main() {{ {body} }}\n");
    let path = std::env::temp_dir().join(format!("loft_pln139_dm_{name}.loft"));
    std::fs::write(&path, &src).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env_remove("LOFT_NO_DOUBLE_MOVE")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    (
        stderr.matches("double-move").count(),
        stdout.matches("DROP:").count(),
    )
}

/// Assert a cell's verdict and its release count together.
#[track_caller]
fn check(name: &str, body: &str, warnings: usize, releases: usize) {
    let (w, r) = cell(name, body);
    assert_eq!(w, warnings, "{name}: double-move warnings — body: {body}");
    assert_eq!(r, releases, "{name}: releases — body: {body}");
}

// ── the lint FIRES: both hand-offs certainly run ─────────────────────────────

/// Two straight-line hand-offs of one local. The headline shape, and the one @PLN139's
/// cascade converted from a leak into a double close.
#[test]
fn m1_two_fields_from_one_local() {
    check(
        "m1",
        "c = mk(1); s1 = S { h: c }; s2 = S { h: c }; println(\"{s1.h.id}{s2.h.id}\");",
        1,
        2,
    );
}

/// A collection element is an owner on the same terms as a field, so the same local
/// appearing twice in a vector literal is the same defect.
#[test]
fn m6_same_local_into_two_elements() {
    check(
        "m6",
        "c = mk(6); v: vector<H> = [c, c]; println(\"{len(v)}\");",
        1,
        2,
    );
}

/// The two owner KINDS mixed: a field takes it, then an element does. The lint counts
/// hand-offs, not shapes, so this must read the same as two fields.
#[test]
fn m7_field_then_element() {
    check(
        "m7",
        "c = mk(7); s1 = S { h: c }; v: vector<H> = [c]; println(\"{s1.h.id}{len(v)}\");",
        1,
        2,
    );
}

/// Inside ONE arm both hand-offs run whenever the arm does, so an arm is a straight line
/// like any other — this is the cell that keeps the branch rule from over-suppressing.
#[test]
fn m10_two_handoffs_inside_one_arm() {
    check(
        "m10",
        "c = mk(10); p = true; \
         if p { s1 = S { h: c }; s2 = S { h: c }; println(\"{s1.h.id}{s2.h.id}\"); }",
        1,
        2,
    );
}

// ── the lint is SILENT, and the release count proves it should be ────────────

/// One owner. The control every firing cell is read against.
#[test]
fn m2_single_handoff() {
    check(
        "m2",
        "c = mk(2); s1 = S { h: c }; println(\"{s1.h.id}\");",
        0,
        1,
    );
}

/// Opposite arms: whichever way the branch goes the value reaches exactly one owner, so
/// warning here would fail correct code — and a `warning` gates a library's CI.
#[test]
fn m3_opposite_arms() {
    check(
        "m3",
        "c = mk(3); p = true; \
         if p { s1 = S { h: c }; println(\"{s1.h.id}\"); } \
         else { s2 = S { h: c }; println(\"{s2.h.id}\"); }",
        0,
        1,
    );
}

/// Reassigned between the hand-offs, so the two containers hold two DISTINCT resources —
/// two releases of two values, which is correct.
#[test]
fn m4_reassigned_between_handoffs() {
    check(
        "m4",
        "c = mk(4); s1 = S { h: c }; c = mk(40); s2 = S { h: c }; \
         println(\"{s1.h.id}{s2.h.id}\");",
        0,
        2,
    );
}

/// Two sources, one hand-off each — a count kept per variable, not per container.
#[test]
fn m5_two_distinct_sources() {
    check(
        "m5",
        "a = mk(5); b = mk(50); s1 = S { h: a }; s2 = S { h: b }; \
         println(\"{s1.h.id}{s2.h.id}\");",
        0,
        2,
    );
}

/// No droppable anywhere: the transfer predicate asks whether an owner will RELEASE the
/// value, and nothing here does.
#[test]
fn m9_no_droppable_control() {
    check(
        "m9",
        "n = 9; v: vector<integer> = [n, n]; println(\"{len(v)}\");",
        0,
        0,
    );
}

/// Two inline temporaries are two values. Only a variable the author named can be counted,
/// and there is none here.
#[test]
fn m11_distinct_inline_temps() {
    check(
        "m11",
        "s1 = S { h: mk(11) }; s2 = S { h: mk(110) }; println(\"{s1.h.id}{s2.h.id}\");",
        0,
        2,
    );
}

/// Nesting is still ONE owner — the outer container's cascade reaches the inner one, so a
/// chain of containers must not read as a chain of owners.
#[test]
fn m12_nested_container_is_one_owner() {
    check(
        "m12",
        "c = mk(12); n = Nest { s: S { h: c } }; println(\"{n.s.h.id}\");",
        0,
        1,
    );
}

/// A conditional reassignment retires the pending hand-off: on the path that reassigns, the
/// second container takes a different value. Silent is the sound answer — `may` is not
/// `must`, and this tier gates.
#[test]
fn m14_conditional_reassignment_kills_the_pair() {
    check(
        "m14",
        "c = mk(14); s1 = S { h: c }; p = true; if p { c = mk(140); } \
         s2 = S { h: c }; println(\"{s1.h.id}{s2.h.id}\");",
        0,
        2,
    );
}

// ── the documented blind spot: released twice, no warning ────────────────────

/// A loop body is ONE static hand-off that runs N times. Seeing this needs the iteration
/// count, so it is a false NEGATIVE — the safe direction for a tier that gates, and pinned
/// here so it stays a known boundary.
#[test]
fn m8_loop_iteration_is_invisible() {
    check(
        "m8",
        "c = mk(8); for i in 0..2 { s = S { h: c }; println(\"{s.h.id}\"); }",
        0,
        2,
    );
}

/// A second hand-off inside an `if` releases twice only when the branch is taken. Warning
/// would fail the program that does not take it, so this is silent by the same rule as
/// `m3` — and is the other half of the blind spot.
#[test]
fn m13_conditional_second_handoff() {
    check(
        "m13",
        "c = mk(13); s1 = S { h: c }; p = true; \
         if p { s2 = S { h: c }; println(\"{s2.h.id}\"); } println(\"{s1.h.id}\");",
        0,
        2,
    );
}

// ── the opt-out ──────────────────────────────────────────────────────────────

/// `LOFT_NO_DOUBLE_MOVE` silences it, and silencing the diagnostic changes nothing about
/// what the program does — the lint reports, it never rewrites.
#[test]
fn opt_out_silences_without_changing_behaviour() {
    let src = format!(
        "{PRELUDE}\nfn main() {{ c = mk(1); s1 = S {{ h: c }}; s2 = S {{ h: c }}; \
         println(\"{{s1.h.id}}{{s2.h.id}}\"); }}\n"
    );
    let path = std::env::temp_dir().join("loft_pln139_dm_optout.loft");
    std::fs::write(&path, &src).expect("write temp script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_NO_DOUBLE_MOVE", "1")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stderr.contains("double-move"),
        "LOFT_NO_DOUBLE_MOVE must silence the lint, got: {stderr}"
    );
    assert_eq!(
        stdout.matches("DROP:").count(),
        2,
        "the lint reports; it must not change what the program does"
    );
}
