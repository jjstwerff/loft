// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN13 Step 1 — script-mode DETECTION (no behaviour change yet).
//!
//! A *script* is a `.loft` file with loose top-level statements (bare statements
//! not wrapped in a `fn`, and no explicit `fn main`). Beginner scripts desugar to
//! one run-once `fn main` in a later step; this module only CLASSIFIES a source as
//! script-or-not. It is deliberately not wired into execution — `is_script` is dead
//! code until Step 2 gates the desugar on it.
//!
//! The classification invariant that makes the later desugar safe: **every file the
//! current compiler accepts classifies as NOT a script** — an all-defs library or a
//! program with `fn main`. So the desugar can only change source that is already
//! rejected today, and the whole existing corpus stays untouched. `is_script` is
//! swept over `default/*.loft` + `tests/**` in the test below to prove that.

use crate::parser::Parser;

/// True when `src` is a beginner-style SCRIPT: it has ≥1 loose top-level statement
/// and defines no `fn main`. False for every all-defs or `fn main`-bearing file.
pub fn is_script(src: &str) -> bool {
    let items = split_top_level(src);
    if items.iter().any(|it| is_fn_main(it)) {
        return false;
    }
    items.iter().any(|it| !is_def_item(it))
}

/// @PLN13 Step 2 — the script desugar. If `src` is a beginner script, return the
/// equivalent loft source: its top-level defs kept at top level, and its loose
/// statements collected into ONE `fn main()` that runs them once, in order, sharing
/// state. Returns `None` for a non-script (all-defs / has `fn main`), so the caller
/// leaves those untouched.
///
/// Loose statements are wrapped verbatim; loft requires `;` between block statements,
/// so a multi-statement script must terminate them with `;` until Step 4 makes `;`
/// optional inside a script `main`. (Line numbers shift in the desugared source — an
/// error-position remap is a later step; this is behind the `--script` flag.)
pub fn script_desugar(src: &str) -> Option<String> {
    if !is_script(src) {
        return None;
    }
    let (mut defs, mut body): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for it in split_top_level(src) {
        if is_def_item(it) {
            defs.push(it);
        } else {
            body.push(it);
        }
    }
    let mut out = String::new();
    for d in defs.drain(..) {
        out.push_str(d);
        out.push('\n');
    }
    out.push_str("fn main() {\n");
    for s in body.drain(..) {
        out.push_str(s);
        out.push('\n');
    }
    out.push_str("}\n");
    Some(out)
}

/// Split `src` into its top-level items (source slices), skipping comments and string
/// contents. Each item is a def (with any leading `#` annotations) or a loose
/// statement. An item ends at the first depth-0 `;`, the `}` that closes a top-level
/// body back to depth 0, or — for a `;`-less loose statement — a newline where what
/// has been scanned is a complete statement.
pub fn split_top_level(src: &str) -> Vec<&str> {
    let b = src.as_bytes();
    let n = b.len();
    let mut items = Vec::new();
    // A leading `#!…` shebang line is not loft source (scripts and `fn main` files
    // alike may carry one) — skip it before the first item.
    let mut i = 0;
    if b.starts_with(b"#!") {
        while i < n && b[i] != b'\n' {
            i += 1;
        }
    }
    i = skip_trivia(b, i);
    while i < n {
        let start = i;
        i = scan_item_end(b, i);
        if i <= start {
            break; // no-progress guard (should not happen)
        }
        let slice = src[start..i].trim();
        if !slice.is_empty() {
            items.push(slice);
        }
        i = skip_trivia(b, i);
    }
    items
}

/// Whitespace, `//` line comments, and `/* */` block comments (the corpus uses all
/// three). Returns the offset of the next significant byte.
fn skip_trivia(b: &[u8], mut i: usize) -> usize {
    let n = b.len();
    loop {
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < n && b[i] == b'/' && b[i + 1] == b'/' {
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < n && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        return i;
    }
}

/// Scan one top-level item from `start`, tracking bracket depth and skipping string
/// contents (both `"…"` and `` `…` `` raw strings) and comments, and return the offset
/// just past its end: the first depth-0 `;`, the `}` that closes a top-level body back
/// to depth 0, or EOF.
///
/// There is deliberately no newline boundary: a `;`/brace-less shape (a brace-less fn
/// body `fn f() -> text\n  return x`, or — in a real script — consecutive loose
/// statements) is merged into one item. That is fine for DETECTION — a merged item is
/// still classified correctly (a def stays a def; a loose statement stays loose).
/// Precise statement boundaries are a Step-2 concern (the desugar).
fn scan_item_end(b: &[u8], start: usize) -> usize {
    let n = b.len();
    let mut i = start;
    let mut depth: i32 = 0;
    while i < n {
        match b[i] {
            b'"' => {
                i += 1;
                while i < n && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
                continue;
            }
            b'`' => {
                // a backtick raw string (loft's multi-line string — shader sources
                // in `graphics/render.loft` embed GLSL braces that must NOT count).
                i += 1;
                while i < n && b[i] != b'`' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            b'/' if i + 1 < n && b[i + 1] == b'/' => {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if i + 1 < n && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' => depth -= 1,
            b'}' => {
                depth -= 1;
                if depth <= 0 {
                    return i + 1; // a top-level body closed
                }
            }
            b';' if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    n
}

/// Skip leading `#annotation` / `#directive`s and the trivia (whitespace + comments)
/// around them, returning the def keyword underneath. Annotations take three forms:
/// bare (`#pure`, `#cwd`), a `"…"` payload (`#rust"impl"`), or a `(…)` payload
/// (`#impure(host_io)`); native stdlib fns carry a POST-fix `#rust"…"` that lands at
/// the head of the NEXT item (`#rust"prev" // comment  fn next() -> T;`), so the
/// comment between must be skipped too.
fn strip_leading_annotations(item: &str) -> &str {
    let b = item.as_bytes();
    let n = b.len();
    let mut i = skip_trivia(b, 0);
    while i < n && b[i] == b'#' {
        i += 1;
        while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
            i += 1;
        }
        while i < n && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        match b.get(i) {
            Some(b'"') => {
                i += 1;
                while i < n && b[i] != b'"' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            Some(b'(') => {
                let mut d = 0i32;
                while i < n {
                    match b[i] {
                        b'(' => d += 1,
                        b')' => {
                            d -= 1;
                            i += 1;
                            if d == 0 {
                                break;
                            }
                            continue;
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i = skip_trivia(b, i);
    }
    item[i..].trim_start()
}

/// A top-level item is a DEF (a def keyword, possibly annotation-prefixed), a bare
/// directive (a lone `#cwd`), or an implicit top-level CONSTANT
/// (`UPPER = …` / `UPPER: T = …` — loft accepts these without a `const` keyword, which
/// is why a lowercase `x = 5` is instead a script statement). Anything else is loose.
fn is_def_item(item: &str) -> bool {
    let core = strip_leading_annotations(item);
    core.is_empty() || Parser::starts_top_level_def(core) || is_top_level_const(core)
}

/// `UPPER_IDENT = …` or `UPPER_IDENT: T = …` — loft's implicit uppercase-named
/// top-level constant. A LOWER-case leading name is NOT a const (it is a script
/// statement or a compile error), which is exactly the script-vs-not distinction.
fn is_top_level_const(core: &str) -> bool {
    let mut ci = core.char_indices();
    let Some((_, first)) = ci.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    let mut end = first.len_utf8();
    for (idx, c) in ci {
        if c.is_alphanumeric() || c == '_' {
            end = idx + c.len_utf8();
        } else {
            break;
        }
    }
    let after = core[end..].trim_start();
    after.starts_with('=') || after.starts_with(':')
}

/// True when the item is `fn main` (after stripping annotations): `fn` then the name
/// `main` at a word boundary.
fn is_fn_main(item: &str) -> bool {
    let core = strip_leading_annotations(item);
    let Some(rest) = core.strip_prefix("fn") else {
        return false;
    };
    let rest = rest.trim_start();
    rest.strip_prefix("main")
        .is_some_and(|r| !r.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

#[cfg(test)]
mod tests {
    use super::{is_script, split_top_level};

    // ── injected-fault controls ──────────────────────────────────────────────
    #[test]
    fn loose_statements_are_a_script() {
        assert!(is_script("print(\"hi\")\n"));
        assert!(is_script("x = 5\nprint(\"x={x}\")\n"));
    }
    #[test]
    fn all_defs_is_not_a_script() {
        assert!(!is_script("fn helper() { print(\"hi\") }\n"));
        assert!(!is_script("struct P { x: integer }\nenum E { A, B }\n"));
    }
    #[test]
    fn explicit_main_is_not_a_script() {
        // even mixed with a loose-looking line, an explicit main opts out.
        assert!(!is_script("fn main() { print(\"hi\") }\n"));
    }
    #[test]
    fn annotated_and_commented_defs_are_not_scripts() {
        assert!(!is_script("#pure\nfn f() -> integer { 1 }\n"));
        assert!(!is_script("#cwd\nfn main() { print(\"hi\") }\n"));
        assert!(!is_script("// a comment\n/* block */\nfn f() {}\n"));
        // a def whose body/string contains statement-like text must not fool it.
        assert!(!is_script("fn f() { let s = \"x = 5\"; print(s) }\n"));
    }
    #[test]
    fn implicit_uppercase_constants_are_not_scripts() {
        // the hex_world / graphics corpus shape: top-level `UPPER = …` (+ typed, + arrays).
        assert!(!is_script("DX = 20.0;\nDY = -34.6;\n"));
        assert!(!is_script("STEP = [\n0, 4,\n2, 4,\n];\n"));
        assert!(!is_script(
            "MAX: integer = 10;\nfn f() -> integer { MAX }\n"
        ));
        // a LOWER-case top-level assignment IS a script statement (the distinction).
        assert!(is_script("total = 0\ntotal = total + 1\n"));
    }
    #[test]
    fn backtick_strings_and_braceless_bodies_are_not_scripts() {
        // graphics/render.loft: a const holding a backtick shader with GLSL braces.
        assert!(!is_script("SHADOW = `\nvoid main() {{ x = 1; }}\n`;\n"));
        // native_crate_pkg.loft: a brace-less fn body.
        assert!(!is_script("pub fn hi() -> text\n    return \"hi\"\n"));
    }
    // ── the desugar (Step 2) ─────────────────────────────────────────────────
    #[test]
    fn desugar_none_for_non_scripts() {
        assert_eq!(super::script_desugar("fn main() { print(\"hi\") }\n"), None);
        assert_eq!(
            super::script_desugar("fn f() {}\nstruct S { x: integer }\n"),
            None
        );
    }
    #[test]
    fn desugar_all_loose_into_one_main() {
        let out = super::script_desugar("print(\"a\");\nprint(\"b\");\n").unwrap();
        assert_eq!(out, "fn main() {\nprint(\"a\");\nprint(\"b\");\n}\n");
    }
    #[test]
    fn desugar_hoists_defs_and_keeps_statement_order() {
        // a def BETWEEN two loose statements is hoisted to top level; the loose
        // statements keep their order in `main`.
        let out =
            super::script_desugar("print(\"a\");\nfn helper() { 1 }\nprint(\"b\");\n").unwrap();
        assert_eq!(
            out,
            "fn helper() { 1 }\nfn main() {\nprint(\"a\");\nprint(\"b\");\n}\n"
        );
    }

    #[test]
    fn shebang_is_skipped() {
        // a shebang program keeps its `fn main` verdict; a shebang script is a script.
        assert!(!is_script(
            "#!/usr/bin/env loft\nfn main() { print(\"hi\") }\n"
        ));
        assert!(is_script("#!/usr/bin/env loft\nprint(\"hi\")\n"));
    }
    #[test]
    fn split_counts_top_level_items() {
        let items = split_top_level("fn a() {}\nfn b() {}\nstruct S { x: integer }\n");
        assert_eq!(items.len(), 3, "items: {items:?}");
    }

    // ── the corpus sweep: EVERY file the compiler accepts must be NOT-a-script ──
    #[test]
    fn no_corpus_file_classifies_as_script() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut misclassified = Vec::new();
        for dir in ["default", "tests"] {
            let mut stack = vec![root.join(dir)];
            while let Some(d) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&d) else {
                    continue;
                };
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|x| x == "loft") {
                        // The invariant is about files the compiler ACCEPTS. Skip the
                        // deliberately-invalid fixtures: the error-message corpus and
                        // any `@EXPECT_ERROR` / `@EXPECT_FAIL` test.
                        if p.components().any(|c| c.as_os_str() == "error_messages") {
                            continue;
                        }
                        if let Ok(src) = std::fs::read_to_string(&p) {
                            // `@EXPECT_ERROR`/`@EXPECT_FAIL` = deliberately invalid;
                            // `@SCRIPT` = an intentional `--script`-only fixture (not
                            // accepted by the plain compiler, so outside the invariant).
                            if src.contains("@EXPECT_ERROR")
                                || src.contains("@EXPECT_FAIL")
                                || src.contains("@SCRIPT")
                            {
                                continue;
                            }
                            if is_script(&src) {
                                misclassified.push(p.display().to_string());
                            }
                        }
                    }
                }
            }
        }
        assert!(
            misclassified.is_empty(),
            "these accepted-by-the-compiler files wrongly classified as scripts:\n{}",
            misclassified.join("\n")
        );
    }
}
