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
    // Over `MAX_C_ARITY` (12) C parameters — the interpreter's caller cannot reach it.
    (
        "c-binding-not-interpretable",
        "fn big(a: integer, b: integer, c: integer, d: integer, e: integer, f: integer, \
         g: integer, h: integer, i: integer, j: integer, k: integer, l: integer, \
         m: integer) -> integer; \
         #c \"big\" \"int(int,int,int,int,int,int,int,int,int,int,int,int,int)\"\n\
         fn main() { print(\"{big(1,2,3,4,5,6,7,8,9,10,11,12,13)}\"); }",
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
#[test]
fn every_pinned_code_offers_a_fix() {
    for (code, prog) in CODES {
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
#[test]
fn every_offered_door_resolves_to_a_catalogue_entry() {
    let snapshot =
        std::fs::read_to_string(root().join("index/features.json")).expect("features snapshot");
    for (code, prog) in CODES {
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
#[test]
fn every_e1_code_renders_its_slug() {
    for (code, prog) in CODES {
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
