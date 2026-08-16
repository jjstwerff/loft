// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN102 arc-E flip-gate — the E1 diagnostic CODE-SET gate (flip-gate-coverage-gaps.md
// Finding 1). E1 declares the diagnostic CODE (a kebab-slug, rendered
// `error[shift-amount-out-of-range]:`) the FROZEN machine handle — prose stays
// improvable, the code is the contract. The `code!` harness STRIPS the tag
// (`testing.rs::strip_diag_code`) and no golden pinned the set, so a rename/removal was
// SILENT. Two teeth close that:
//   1. every pinned code RENDERS its `[slug]` (a minimal trigger program) → rename /
//      removal / unreachable is red;
//   2. the codes DECLARED in `src/` equal the pinned CODES set → an ADD not reflected
//      here is red (a reviewed diff; a code is add-with-ceremony, never a silent change,
//      and post-flip a rename/removal is a contract break per COMPATIBILITY.md).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// THE GOLDEN: the frozen E1 code set + a minimal program that triggers each. Adding /
/// renaming / removing a code must update this array (a reviewed diff).
const CODES: &[(&str, &str)] = &[
    // @PLN131 — the copy notice is the diagnostic the suggestions work attaches to, so it
    // got the first of this arc's codes. `s` survives the construction, so the copy is
    // avoidable rather than forced.
    (
        "avoidable-copy",
        "struct H { v: vector<integer> }\nfn u(h: H) -> integer { len(h.v) }\n\
         fn main() { s = [1, 2, 3]; h = H { v: s }; print(\"{u(h)} {len(s)}\"); }",
    ),
    // @PLN139 stage G — one `H` handed to two containers. The cascade releases what each
    // container owns, so this closes one resource twice; both hand-offs are straight-line,
    // which is the certainty the lint requires.
    (
        "double-move",
        "struct H { id: integer }\nfn OpDrop(self: H) { print(\"{self.id}\"); }\n\
         struct S { h: H }\n\
         fn main() { c = H { id: 1 }; a = S { h: c }; b = S { h: c }; \
         print(\"{a.h.id}{b.h.id}\"); }",
    ),
    // @PLN107 dead-store lint. `d = s.items` COPIES (C86), so writing `d` cannot reach
    // `s`, and `d` is never read afterwards — the write is lost.
    (
        "lost-write",
        "struct D { items: vector<integer> }\n\
         fn f(s: D) -> integer { d = s.items; d[0] = 9; return len(s.items); }\n\
         fn main() { s = D { items: [1, 2] }; print(\"{f(s)}\"); }",
    ),
    (
        "cast-constant-out-of-range",
        "fn main() { x = 1e30 as integer; print(\"{x}\"); }",
    ),
    // Over `MAX_C_ARITY` (32) C parameters — past what a `#c` binding covers on
    // EITHER backend (@PLN128 arc C; it was 12, interpreter-only).
    (
        "c-binding-not-interpretable",
        "fn big(p0: integer, p1: integer, p2: integer, p3: integer, p4: integer, p5: integer, p6: integer, p7: integer, p8: integer, p9: integer, p10: integer, p11: integer, p12: integer, p13: integer, p14: integer, p15: integer, p16: integer, p17: integer, p18: integer, p19: integer, p20: integer, p21: integer, p22: integer, p23: integer, p24: integer, p25: integer, p26: integer, p27: integer, p28: integer, p29: integer, p30: integer, p31: integer, p32: integer) -> integer; \
         #c \"big\" \"int(int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int,int)\"\n\
         fn main() { print(\"{big(1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33)}\"); }",
    ),
    (
        "coalesce-default-type-mismatch",
        "fn main() { n: integer? = null; print(\"{n ?? \\\"x\\\"}\"); }",
    ),
    ("format-unescaped-brace", "fn main() { print(\"a } b\"); }"),
    (
        "shift-amount-out-of-range",
        "fn main() { x = 1 << 100; print(\"{x}\"); }",
    ),
    // @PLN102 arc C step 3 — the STEER itself: a call to a superseded symbol. Coded late
    // (@PLN131 coverage audit) although the two declaration-side lints below had codes from
    // the start — the one a user actually meets was the one without an identity.
    (
        "superseded-call",
        "fn scaled(v: integer, by: integer) -> integer { v * by }\n\
         fn doubled(v: integer) -> integer { scaled(v, 2) }  #superseded \"scaled\"\n\
         fn main() { print(\"{doubled(21)}\"); }",
    ),
    // @PLN102 arc C — a steer that would ship dangling: the named successor does not exist.
    (
        "superseded-unknown-successor",
        "fn old_way(v: integer) -> integer { v + 1 }  #superseded \"no_such_fn\"\n\
         fn main() { print(\"{old_way(1)}\"); }",
    ),
    // The successor exists, but the superseded body never calls it — the steer ships
    // without its fold.
    (
        "superseded-not-folded",
        "fn new_way(v: integer) -> integer { v + 1 }\n\
         fn old_way(v: integer) -> integer { v + 2 }  #superseded \"new_way\"\n\
         fn main() { print(\"{old_way(1)}\"); }",
    ),
    (
        "text-parse-may-fail",
        "fn main() { x: integer = \"5\" as integer; print(\"{x}\"); }",
    ),
    (
        "redundant-coalesce",
        "struct S { a: integer }\n\
         fn main() { s = S { a: 1 }; print(\"{s.a ?? 0}\"); }",
    ),
    (
        "redundant-default-fallback",
        "struct S { a: integer }\n\
         fn main() { s = S { a: 1 }; print(\"{s.a?}\"); }",
    ),
    (
        "redundant-null-check",
        "struct S { a: integer }\n\
         fn main() { s = S { a: 1 }; if s.a == null { print(\"n\") } }",
    ),
    (
        "redundant-null-negation",
        "struct S { a: integer }\n\
         fn main() { s = S { a: 1 }; if !s.a { print(\"n\") } }",
    ),
    (
        "dead-assignment",
        "fn main() { x = 1; x = 2; print(\"{x}\"); }",
    ),
    ("never-read", "fn main() { x = 1; print(\"hi\"); }"),
    (
        "upper-case-local",
        "fn main() { FOO = 1; print(\"{FOO}\"); }",
    ),
    (
        "unreachable-code",
        "fn f() -> integer { return 1; return 2 }\n\
         fn main() { print(\"{f()}\"); }",
    ),
    (
        "unreachable-match-arm",
        "enum E { A, B }\n\
         fn main() { e: E = A; match e { A => print(\"a\"), A => print(\"a2\"), B => print(\"b\") } }",
    ),
    ("empty-parallel-block", "fn main() { parallel { } }"),
    (
        "text-slice-char-bound",
        "fn main() { s = \"ab\"; print(\"{s[0..len(s)]}\"); }",
    ),
    (
        "text-index-char-bound",
        "fn main() { s = \"ab\"; for i in 0..len(s) { print(\"{s[i]}\") } }",
    ),
    (
        "index-bounds-other-vector",
        "fn main() { a = [1]; b = [2]; for i in 0..len(a) { print(\"{b[i]}\") } }",
    ),
    (
        "function-complexity",
        "fn main() {\n\
           x = 0;\n\
           for a in 0..3 { for b in 0..3 { for c in 0..3 { for d in 0..3 { for e in 0..3 {\n\
             if a > 0 { if b > 0 { if c > 0 { if d > 0 { if e > 0 { x += 1 } } } } }\n\
           } } } } }\n\
           print(\"{x}\");\n\
         }",
    ),
    (
        "too-many-parameters",
        "fn f(a: integer, b: integer, c: integer, d: integer, e: integer, g: integer, h: integer, i: integer) -> integer { a+b+c+d+e+g+h+i }\n\
         fn main() { print(\"{f(1,2,3,4,5,6,7,8)}\"); }",
    ),
    (
        "trailing-boolean-parameters",
        "fn f(a: integer, b: boolean, c: boolean) -> integer { if b { a } else if c { a } else { 0 } }\n\
         fn main() { print(\"{f(1,true,false)}\"); }",
    ),
    (
        "omitted-field-zero",
        "struct S { hover: integer, palette_pick: integer }\n\
         fn main() { s = S { hover: 3 }; print(\"{s.palette_pick}\"); }",
    ),
    (
        "linked-group-double-fill",
        "struct E { k: integer, v: integer }\n\
         struct S { by_k: hash<E[k]>, by_v: sorted<E[v]> }\n\
         fn main() { s = S { by_k: [E { k: 1, v: 10 }], by_v: [E { k: 2, v: 20 }] }; \
         print(\"{len(s.by_k)}\"); }",
    ),
    (
        "needless-reference-parameter",
        "fn f(x: &(integer, integer)) -> integer { x.0 }\n\
         fn main() { v = (1, 2); print(\"{f(&v)}\"); }",
    ),
    (
        "needless-const-parameter",
        "fn f(a: const integer) -> integer { a }\n\
         fn main() { print(\"{f(1)}\"); }",
    ),
    // The `&` must be FIELD-MUTATED: a read-only `&` raises an error at the same position,
    // and the pretty renderer's cascade dedup then suppresses the advice this pins.
    (
        "slow-reference-parameter",
        "struct S { a: integer }\n\
         fn f(s: &S) { s.a = 1 }\n\
         fn main() { v = S { a: 0 }; f(v); print(\"{v.a}\"); }",
    ),
    (
        "not-null-deprecated",
        "struct S { a: integer not null }\n\
         fn main() { s = S { a: 1 }; print(\"{s.a}\"); }",
    ),
    (
        "const-reevaluated",
        "fn mk() -> integer { 7 }\n\
         const K = mk();\n\
         fn main() { print(\"{K}\"); }",
    ),
    (
        "digit-separator-grouping",
        "fn main() { x = 1_00; print(\"{x}\"); }",
    ),
    (
        "empty-braces-not-collection",
        "struct S { v: vector<integer> }\n\
         fn main() { s = S { v: {} }; print(\"{len(s.v)}\"); }",
    ),
    (
        "divide-by-constant-zero",
        "fn main() { print(\"{1 / 0}\"); }",
    ),
    (
        "unary-minus-binds-tighter",
        "fn main() { x = 2; print(\"{-x ** 2}\"); }",
    ),
    (
        "read-size-not-element-multiple",
        "fn main() { f = file(\"loft_trig.bin\"); f#format = LittleEndian; v = f#read(6) as vector<i32>; print(\"{len(v)}\"); }",
    ),
    (
        "file-write-width",
        "fn main() { f = file(\"loft_trig.bin\"); f#format = LittleEndian; f += 1; }",
    ),
    // @PLN131 — the did-you-mean family. Each already computed its replacement and knew
    // where the name sat; they were reachable only as an LSP quickfix until the fix shape
    // gave them `--explain`, `loft fix`, and — the part that matters — VERIFICATION, which
    // is what turns a Levenshtein guess into a measurement.
    (
        "unknown-function",
        "fn helper(v: integer) -> integer { v }\n\
         fn main() { print(\"{helpr(1)}\"); }",
    ),
    (
        "unknown-field",
        "fn main() { s = \"hi\"; print(\"{s.starts_wit(\\\"h\\\")}\"); }",
    ),
    (
        "unknown-variable",
        "fn main() { value = 1; print(\"{valu}\"); }",
    ),
    // Listed in `NO_MINIMAL_TRIGGER` — pinned so the set stays complete; the program is
    // the closest shape and is not run.
    (
        "missing-return-path",
        "fn f(c: boolean) -> integer not null { if c { return 1 } }\n\
         fn main() { print(\"{f(true)}\"); }",
    ),
    (
        "package-contract-drifted",
        "fn main() { print(\"needs an installed package manifest\"); }",
    ),
    (
        "module-name-shadowed",
        "fn main() { print(\"needs a two-package dependency graph\"); }",
    ),
    (
        "persist-bind-through-field",
        "struct Inner { k: integer }\n\
         struct Outer { items: hash<Inner[k]> }\n\
         fn main() { o = Outer { items: [] }; store_persist_bind(o.items, \"loft_trig.store\"); }",
    ),
    (
        "shadowed-by-method",
        "fn main() { print(\"needs the same fn in a LIBRARY — in main this is the C95 error\"); }",
    ),
    // loft#917 — a `?` on a COLLECTION field. The field stores a record id whose zero
    // already means empty, so the `?` cannot read back as null.
    (
        "nullable-collection-field",
        "struct H { xs: vector<integer>? }\n\
         fn main() { h = H { xs: null }; print(\"{h.xs == null}\"); }",
    ),
];

/// @PLN131 — codes with no MINIMAL trigger, each with why.
///
/// A code here is still coded and still documented; what is missing is a one-file program
/// that fires it, so tooth 1 cannot cover it. Listed rather than silently skipped, because
/// "no trigger" and "no code" look identical in a green run.
const NO_MINIMAL_TRIGGER: &[(&str, &str)] = &[
    (
        // Needs a package whose manifest records an older tested contract — not expressible
        // as a single source file.
        "package-contract-drifted",
        "requires an installed package manifest",
    ),
    (
        // Gated on the DEPRECATED `not null` return spelling, and the fall-through it warns
        // about is already a hard error ("expected integer, got void on return from block"),
        // which fires first. Reachable only if that error is ever relaxed — worth revisiting
        // rather than deleting, since the warning is the friendlier of the two.
        "missing-return-path",
        "pre-empted by a hard error, and gated on a deprecated spelling",
    ),
    (
        // loft#912 — needs TWO packages, each holding a module file of the same basename,
        // with one depending on the other. A single source file cannot express a
        // dependency graph. Covered instead by `tests/module_name_clash.rs`, which builds
        // the two-package tree and asserts both load orders.
        "module-name-shadowed",
        "requires a two-package dependency graph",
    ),
    (
        // loft#940 — fires only for a LIBRARY source (@PLN102 C97 module-scoping). The same
        // definition in a single main file is the C95 hard error "Cannot redefine 'clamp'",
        // which pre-empts it, so no one-file program can reach this warning. Covered instead
        // by `tests/imports.rs`, which has the lib-dir harness.
        "shadowed-by-method",
        "fires only in a library source; in main the C95 error pre-empts it",
    ),
];

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// @PLN131 — codes whose fix is BLOCKED, each with what blocks it.
///
/// **Currently empty: every code says what to write instead.** The list stays because the
/// alternative to a listed exception is a silent one — a code that quietly ships without a
/// fix looks exactly like a code that does not need one.  A row earns its place only when
/// the resolution is KNOWN but cannot be offered soundly, and it carries the reason so the
/// next reader can tell whether the blocker still holds.
///
/// The two rows this held were the `superseded-*` pair, blocked because their concept is
/// `#superseded` itself and the feature catalogue had no entry for it — a fix that links
/// nowhere is not finished.  `@F109` cleared that, exactly as `@F106` had for `move`.
const FIX_BLOCKED: &[(&str, &str)] = &[];

/// Run `prog` on the interpreter with compact errors (so a typed diagnostic surfaces as
/// its stable `[code]` tag), returning stdout+stderr.
fn compact_output(prog: &str) -> String {
    let path = std::env::temp_dir().join(format!("loft_e1_{}.loft", std::process::id()));
    std::fs::write(&path, prog).unwrap();
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&path)
        .env("LOFT_ERRORS", "compact")
        // @PLN102 case-D's index lint is opt-in, and one pinned code needs it. Harmless
        // elsewhere: it only fires on a loop bounded by another vector's `len`.
        .env("LOFT_LINT_STRICT_INDEX", "1")
        .env("LOFT_TIMEOUT", "60")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Run `prog` with `--explain`, in the PRETTY renderer — the only one that carries fix
/// lines (the compact form is a single line by definition).
fn explain_output(prog: &str) -> String {
    // Per-probe unique name: these tests run concurrently in one process, so a
    // pid-only path has two of them writing and deleting the same file — which fails as
    // "no such file", i.e. as a missing fix rather than as the collision it is.
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "loft_e1_fix_{}_{}.loft",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, prog).unwrap();
    let out = Command::new(loft_bin())
        .args(["--interpret", "--check", "--explain"])
        .arg(&path)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_LINT_STRICT_INDEX", "1")
        .env("LOFT_TIMEOUT", "60")
        .output()
        .expect("failed to invoke loft binary");
    let _ = std::fs::remove_file(&path);
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Every `@FNN` door offered anywhere in `out`.
fn doors(out: &str) -> Vec<u32> {
    let mut v = Vec::new();
    for at in match_positions(out, "· @F") {
        let digits: String = out[at..].chars().take_while(char::is_ascii_digit).collect();
        if let Ok(n) = digits.parse() {
            v.push(n);
        }
    }
    v
}

/// @PLN131 ship step 5 — every pinned code says what to write instead.
///
/// The suggestion is the deliverable half of a diagnostic, so a code that only says what is
/// wrong is half-built.  The exceptions are listed rather than tolerated: `FIX_BLOCKED`
/// names each one and what blocks it, which is what stops "no fix yet" from quietly
/// becoming "no fix ever".
// @speed 2.2
#[test]
fn every_pinned_code_offers_a_fix() {
    for (code, prog) in CODES {
        // A code with no minimal trigger renders nothing, so there is nothing to read a
        // fix or a door out of. Skipped by NAME, listed with its reason.
        if NO_MINIMAL_TRIGGER.iter().any(|(c, _)| c == code) {
            continue;
        }
        let blocked = FIX_BLOCKED.iter().find(|(c, _)| c == code);
        let out = explain_output(prog);
        let has_fix = out.contains("  fix  ");
        match blocked {
            Some((_, why)) => assert!(
                !has_fix,
                "`{code}` is listed in FIX_BLOCKED ({why}) but now offers a fix — drop its \
                 row from that list.\ngot:\n{out}"
            ),
            None => assert!(
                has_fix,
                "`{code}` renders no fix line under `--explain`. A diagnostic says what is \
                 wrong; a fix says what to write instead, and that is the half a reader \
                 acts on. Attach one with `fix_last`, or add a row to FIX_BLOCKED saying \
                 what blocks it.\nprog: {prog}\ngot:\n{out}"
            ),
        }
    }
}

/// @PLN131 — every door a fix opens onto is a real catalogue entry.
///
/// A door onto nothing is worse than no door, and a fix names its concept precisely so a
/// reader who wants the *why* has somewhere to go.  Checking every offered door (rather
/// than one pinned `@F`) means a renumbered or deleted feature breaks the build on the day
/// it happens, whichever fix pointed at it.
// @speed 2.2
#[test]
fn every_offered_door_resolves_to_a_catalogue_entry() {
    let snapshot =
        std::fs::read_to_string(root().join("index/features.json")).expect("features snapshot");
    for (code, prog) in CODES {
        // A code with no minimal trigger renders nothing, so there is nothing to read a
        // fix or a door out of. Skipped by NAME, listed with its reason.
        if NO_MINIMAL_TRIGGER.iter().any(|(c, _)| c == code) {
            continue;
        }
        let found_doors = doors(&explain_output(prog));
        for n in &found_doors {
            let listed = snapshot.contains(&format!("\"number\": {n}"))
                || snapshot.contains(&format!("\"number\":{n}"));
            assert!(
                listed,
                "`{code}` offers a fix whose door is `@F{n}`, which is not in the feature \
                 catalogue — a door onto nothing is worse than no door."
            );
        }
        // Per code, not summed across them: a total lets one code's three doors cover
        // another's zero, which is exactly the gap this is meant to catch.
        if !FIX_BLOCKED.iter().any(|(c, _)| c == code) {
            assert!(
                !found_doors.is_empty(),
                "`{code}` offers a fix that names no door. The concept is the handle a \
                 reader searches for; a fix without one has taken the teaching half away."
            );
        }
    }
}

/// Tooth 1 — every pinned code renders its `[slug]` tag.
// @speed 2.5
#[test]
fn every_e1_code_renders_its_slug() {
    for (code, prog) in CODES {
        if NO_MINIMAL_TRIGGER.iter().any(|(c, _)| c == code) {
            continue;
        }
        let out = compact_output(prog);
        assert!(
            out.contains(&format!("[{code}]")),
            "E1 code `{code}` did not render its tag — renamed / removed / unreachable?\n\
             the code is the FROZEN machine handle (@PLN102 E1).\nprog: {prog}\ngot:\n{out}"
        );
    }
}

/// Tooth 2 — the codes DECLARED in `src/` equal the pinned CODES set.
#[test]
fn source_declared_codes_match_the_pinned_set() {
    let declared = scan_source_codes();
    let pinned: BTreeSet<String> = CODES.iter().map(|(c, _)| (*c).to_string()).collect();
    assert_eq!(
        declared, pinned,
        "\nE1 code set drifted between src/ and the pinned CODES list in this file.\n\
         A code is the FROZEN machine handle (@PLN102 E1). On an intentional change: update \
         CODES (+ its trigger). Post-flip an add is a reviewed diff and a rename/removal is \
         a contract break.\n"
    );
}

/// Extract every kebab-slug code literal from the two diagnostic-emit forms across `src/`:
///  · `code = "X"` — the `diagnostic!(… code = "X" …)` macro arm (literal follows directly);
///  · `*_coded(Level::_, "X", …)` — the lexer's `err_coded`/`diagnostic_coded` helpers
///    (the code is the first kebab string literal after the `(`).
fn scan_source_codes() -> BTreeSet<String> {
    let mut files = Vec::new();
    collect_rs(&root().join("src"), &mut files);
    let mut out = BTreeSet::new();
    for f in files {
        let s = std::fs::read_to_string(&f).unwrap_or_default();
        // form 1 — `code = "X"`
        for at in match_positions(&s, "code = \"") {
            if let Some(lit) = read_to_quote(&s[at..])
                && is_kebab_code(&lit)
            {
                out.insert(lit);
            }
        }
        // form 2 — `*_coded( … "X" …`  (first kebab literal within a bounded window)
        for at in match_positions(&s, "_coded(") {
            let win = &s[at..(at + 200).min(s.len())];
            if let Some(lit) = first_kebab_literal(win) {
                out.insert(lit);
            }
        }
    }
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_rs(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Byte offsets just PAST each occurrence of `needle` in `hay`.
fn match_positions(hay: &str, needle: &str) -> Vec<usize> {
    let mut v = Vec::new();
    let mut from = 0;
    while let Some(i) = hay[from..].find(needle) {
        let end = from + i + needle.len();
        v.push(end);
        from = end;
    }
    v
}

/// `s` begins immediately after an opening `"`; return the literal up to the next `"`.
fn read_to_quote(s: &str) -> Option<String> {
    s.find('"').map(|q| s[..q].to_string())
}

/// The first kebab-code string literal appearing in `win`.
fn first_kebab_literal(win: &str) -> Option<String> {
    let mut rest = win;
    while let Some(q) = rest.find('"') {
        let after = &rest[q + 1..];
        if let Some(lit) = read_to_quote(after) {
            if is_kebab_code(&lit) {
                return Some(lit);
            }
            rest = &after[lit.len()..];
        } else {
            break;
        }
    }
    None
}

/// A kebab code = lowercase letters/digits with at least one hyphen, starting with a letter.
fn is_kebab_code(s: &str) -> bool {
    s.contains('-')
        && s.bytes().next().is_some_and(|c| c.is_ascii_lowercase())
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

/// @PLN131 — every pinned code has a row in `doc/claude/DIAGNOSTICS.md`.
///
/// A code exists so a reader can look it up; one with nothing to grep to is a dead door,
/// and the index has to land WITH the code rather than after it, or the gap is invisible
/// until someone hits it. This lives beside `CODES` deliberately: the pinned set is already
/// the one home for "which codes exist", so the doc check reads from it instead of running a
/// second scan of `src/` that can disagree with the first.
#[test]
fn every_pinned_code_is_documented() {
    let index = std::fs::read_to_string(root().join("doc/claude/DIAGNOSTICS.md"))
        .expect("doc/claude/DIAGNOSTICS.md");
    let missing: Vec<&str> = CODES
        .iter()
        .map(|(code, _)| *code)
        .filter(|code| !index.contains(&format!("`{code}`")))
        .collect();
    assert!(
        missing.is_empty(),
        "code(s) with no row in doc/claude/DIAGNOSTICS.md: {}\n\
         A code is the handle a reader searches for — add the row in the same change.",
        missing.join(", ")
    );
}
