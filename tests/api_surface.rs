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
