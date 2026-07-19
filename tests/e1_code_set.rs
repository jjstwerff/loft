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

/// THE GOLDEN: the frozen E1 code set + a minimal program that triggers each. Adding /
/// renaming / removing a code must update this array (a reviewed diff).
const CODES: &[(&str, &str)] = &[
    (
        "cast-constant-out-of-range",
        "fn main() { x = 1e30 as integer; print(\"{x}\"); }",
    ),
    ("format-unescaped-brace", "fn main() { print(\"a } b\"); }"),
    (
        "shift-amount-out-of-range",
        "fn main() { x = 1 << 100; print(\"{x}\"); }",
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
