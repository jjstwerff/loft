// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN102 C1 commit 1 — `loft api-surface`: the observable public surface as membership
//! + visibility TIER. Proves the closure walk (a non-`pub` type reachable through a `pub`
//! signature is SEALED, not dropped) and the exclusions (a private / unreachable non-`pub`
//! type and a non-`pub` fn are not in the surface).

use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

fn api_surface(src: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "loft_apisurf_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("lib.loft");
    std::fs::write(&file, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("api-surface")
        .arg(&file)
        .output()
        .expect("run loft api-surface");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "api-surface exited non-zero: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Run `loft api-surface --diff <base> <new> [--json]`; return (stdout, exit code).
fn api_diff_cli(base: &str, new: &str, json: bool) -> (String, i32) {
    let dir = std::env::temp_dir().join(format!(
        "loft_apidiff_{}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let fb = dir.join("base.loft");
    let fn_ = dir.join("new.loft");
    std::fs::write(&fb, base).unwrap();
    std::fs::write(&fn_, new).unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("api-surface").arg("--diff").arg(&fb).arg(&fn_);
    if json {
        cmd.arg("--json");
    }
    let out = cmd.output().expect("run loft api-surface --diff");
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn surface_membership_and_tiers() {
    let s = api_surface(
        "struct Widget { x: integer }\n\
         struct Hidden { z: integer }\n\
         pub struct Public { v: integer }\n\
         pub fn make() -> Widget { Widget { x: 5 } }\n\
         pub fn plain(a: integer) -> integer { a + 1 }\n\
         fn helper() -> Hidden { Hidden { z: 0 } }\n",
    );
    // public roots
    assert!(s.contains("make · fn · public"), "make missing:\n{s}");
    assert!(s.contains("plain · fn · public"), "plain missing:\n{s}");
    assert!(
        s.contains("Public · struct · public"),
        "Public missing:\n{s}"
    );
    // the closure: a non-`pub` type returned by a `pub` fn is SEALED, not dropped.
    assert!(
        s.contains("Widget · struct · sealed"),
        "Widget not sealed:\n{s}"
    );
    // exclusions: a private/unreachable non-`pub` type and a non-`pub` fn are NOT surface.
    assert!(!s.contains("Hidden"), "Hidden must be excluded:\n{s}");
    assert!(!s.contains("helper"), "helper must be excluded:\n{s}");
}

#[test]
fn closure_is_transitive() {
    // `build` returns `Outer` (sealed); `Outer` has a field of non-`pub` `Inner` → `Inner`
    // is sealed too. Proves the closure follows struct field types, transitively.
    let s = api_surface(
        "struct Inner { n: integer }\n\
         struct Outer { i: Inner }\n\
         pub fn build() -> Outer { Outer { i: Inner { n: 1 } } }\n",
    );
    assert!(s.contains("build · fn · public"), "build missing:\n{s}");
    assert!(
        s.contains("Outer · struct · sealed"),
        "Outer not sealed:\n{s}"
    );
    assert!(
        s.contains("Inner · struct · sealed"),
        "Inner not sealed (transitive):\n{s}"
    );
}

#[test]
fn signatures_over_every_kind() {
    // Commit 2 — resolved signatures attached, in the clean user-facing type spelling.
    let s = api_surface(
        "struct Widget { x: integer, tag: text }\n\
         enum Shape { Circle { r: integer }, Square { side: integer }, Point }\n\
         pub struct Public { v: integer }\n\
         pub fn make(n: integer, label: text) -> Widget { Widget { x: n, tag: label } }\n\
         pub fn maybe(a: integer) -> Widget? { if a > 0 { Widget { x: a, tag: \"\" } } else { null } }\n\
         pub fn area(s: Shape) -> integer { 0 }\n",
    );
    let has = |line: &str| s.lines().any(|l| l == line);
    // fn: params (name: type) + return; a nullable return renders as `?`.
    assert!(
        has("make · fn · public · (n: integer, label: text) -> Widget"),
        "make sig:\n{s}"
    );
    assert!(
        has("maybe · fn · public · (a: integer) -> Widget?"),
        "nullable return:\n{s}"
    );
    assert!(
        has("area · fn · public · (s: Shape) -> integer"),
        "area sig:\n{s}"
    );
    // struct fields — a public root and a sealed closure member. Fields are sorted by name
    // (commit 3 canonicalisation: named construction → field order is not API), so `tag`
    // precedes `x` regardless of declaration order.
    assert!(
        has("Public · struct · public · { v: integer }"),
        "Public sig:\n{s}"
    );
    assert!(
        has("Widget · struct · sealed · { tag: text, x: integer }"),
        "Widget sig:\n{s}"
    );
    // enum variants, sorted by name, with the synthetic `enum` discriminant tag filtered out.
    assert!(
        has("Shape · enum · sealed · { Circle { r: integer }, Point, Square { side: integer } }"),
        "enum sig:\n{s}"
    );
}

#[test]
fn determinism_corpus() {
    // Commit 3 — the make-or-break. Cosmetically-different-but-identical surfaces MUST
    // produce byte-identical descriptors (a strict check has no escape valve for a false
    // break); a genuinely-different surface MUST differ.
    let same = |a: &str, b: &str, why: &str| {
        assert_eq!(api_surface(a), api_surface(b), "must be identical: {why}");
    };
    let differ = |a: &str, b: &str, why: &str| {
        assert_ne!(api_surface(a), api_surface(b), "must differ: {why}");
    };

    // --- invariances: a cosmetic edit is NOT a diff ---
    same(
        "pub fn f() -> integer { 1 }\nstruct S { a: integer }\npub fn g() -> S { S{a:1} }\n",
        "struct S { a: integer }\npub fn g() -> S { S{a:1} }\npub fn f() -> integer { 1 }\n",
        "reordered top-level defs",
    );
    same(
        "pub struct W { x: integer, tag: text }\n",
        "pub struct W { tag: text, x: integer }\n",
        "reordered struct fields (named construction → not API)",
    );
    same(
        "pub enum E { A { p: integer, q: text }, B }\n",
        "pub enum E { B, A { q: text, p: integer } }\n",
        "reordered enum variants + variant fields",
    );
    same(
        "pub struct W{x:integer}\n",
        "pub struct W {  x : integer  }\n",
        "whitespace / formatting",
    );
    same(
        "type Score = integer;\npub fn f() -> Score { 1 }\n",
        "pub fn f() -> integer { 1 }\n",
        "a transparent alias vs its expansion",
    );

    // --- real changes MUST differ (positive controls — no vacuous determinism) ---
    differ(
        "pub fn f() -> integer { 1 }\n",
        "pub fn f() -> text { \"\" }\n",
        "return type change",
    );
    differ(
        "pub fn f(a: integer, b: text) -> integer { a }\n",
        "pub fn f(b: text, a: integer) -> integer { a }\n",
        "fn param REORDER (positional — a real API change, must NOT be canonicalised away)",
    );
    differ(
        "pub struct W { x: integer }\n",
        "pub struct W { x: text }\n",
        "field type change",
    );
    differ(
        "pub struct W { x: integer }\n",
        "pub struct W { x: integer, y: text }\n",
        "added field",
    );
}

#[test]
fn diff_cli_verdict_and_exit_codes() {
    // Commit 6 — `--diff` wires the surface reader to the diff engine.
    // Superset (added a fn) → exit 0, "drop-in".
    let (out, code) = api_diff_cli(
        "pub fn make() -> integer { 1 }\n",
        "pub fn make() -> integer { 1 }\npub fn extra(a: integer) -> integer { a }\n",
        false,
    );
    assert_eq!(code, 0, "superset exits 0:\n{out}");
    assert!(out.contains("drop-in"), "superset human text:\n{out}");
    // Break (changed return type) → exit 1, names the broken symbol.
    let (out, code) = api_diff_cli(
        "pub fn make() -> integer { 1 }\n",
        "pub fn make() -> text { \"\" }\n",
        false,
    );
    assert_eq!(code, 1, "break exits 1:\n{out}");
    assert!(
        out.contains("BREAK") && out.contains("make"),
        "break human names make:\n{out}"
    );
}

#[test]
fn diff_cli_json() {
    let (out, code) = api_diff_cli(
        "pub fn make() -> integer { 1 }\n",
        "pub fn make() -> text { \"\" }\n",
        true,
    );
    assert_eq!(code, 1);
    assert!(
        out.contains(r#""verdict":"break""#),
        "json break verdict:\n{out}"
    );
    assert!(out.contains("make"), "json names make:\n{out}");
    let (out, code) = api_diff_cli(
        "pub fn make() -> integer { 1 }\n",
        "pub fn make() -> integer { 1 }\npub fn g() -> integer { 2 }\n",
        true,
    );
    assert_eq!(code, 0);
    assert!(
        out.contains(r#""verdict":"superset""#),
        "json superset verdict:\n{out}"
    );
}

// Commit 5 — the @PLN97 LAYOUT axis: a second verdict beside the API axis.
const POINT_V1: &str = "pub struct Point { x: integer, y: integer }\n\
                        pub fn make() -> Point { Point{x:1,y:2} }\n";
const POINT_REORDERED: &str = "pub struct Point { y: integer, x: integer }\n\
                               pub fn make() -> Point { Point{x:1,y:2} }\n";

#[test]
fn layout_axis_field_reorder_is_api_dropin_but_layout_changed() {
    // A field REORDER is a named-construction API drop-in (commit 3 sorts fields), but a store
    // LAYOUT change — the silent DATA break for a persisting consumer that the API axis alone
    // green-lights. It must red (exit 1) on the layout axis and name the reshaped type.
    let (out, code) = api_diff_cli(POINT_V1, POINT_REORDERED, false);
    assert!(out.contains("API: drop-in"), "API drop-in:\n{out}");
    assert!(
        out.contains("Layout: CHANGED") && out.contains("Point"),
        "layout changed names Point:\n{out}"
    );
    assert_eq!(code, 1, "a layout reshape reds:\n{out}");
}

#[test]
fn layout_axis_stable_on_pure_addition() {
    let added = format!("{POINT_V1}pub fn g() -> integer {{ 0 }}\n");
    let (out, code) = api_diff_cli(POINT_V1, &added, false);
    assert!(out.contains("Layout: stable"), "layout stable:\n{out}");
    assert_eq!(code, 0, "both axes clean → exit 0:\n{out}");
}

#[test]
fn diff_json_carries_both_axes() {
    let (out, _) = api_diff_cli(POINT_V1, POINT_REORDERED, true);
    assert!(
        out.contains(r#""api":{"verdict":"superset""#),
        "json api superset:\n{out}"
    );
    assert!(
        out.contains(r#""layout":{"verdict":"changed""#) && out.contains("Point"),
        "json layout changed names Point:\n{out}"
    );
}
