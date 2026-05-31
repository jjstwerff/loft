// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Tier-0 lazy auto-`use`: a lightweight pre-scan of a loft source file for
//! `lib::…` references (lib_plans/future/03-lazy-stdlib).
//!
//! The parser loads explicit `use lib;` libraries at the TOP of each pass,
//! before any definition is created — which is the only place a library can be
//! loaded without the two-pass model later complaining "cannot redefine".  A
//! `lib::method` reference, however, is only discovered deep in a function
//! body.  This scan bridges the gap: run once over the file BEFORE the
//! definitions are parsed, collect every `lib::` prefix, and let the use-region
//! load those libraries — so `regex::matches(line)` works with no `use regex;`.
//!
//! Why a hand scan and not the lexer: it must see references inside
//! **format-string interpolations** (`"{regex::matches(line)}"` — the common
//! case), so it scans inside `{…}` within strings.  It deliberately stays
//! conservative: a missed reference just falls back to the normal "unknown"
//! error (the user adds `use`), and a stray hit on a name that is not a real
//! library is filtered out later by the resolver.  Two filters keep it tight:
//!
//!   * **lowercase only** — loft libraries are lowercase; types/enums are
//!     capitalised, so `Color::Red` / `Shape::Circle` are never candidates.
//!   * comments and plain (non-interpolated) string text are skipped.

#[inline]
fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

#[inline]
fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Collect the distinct lowercase `name` prefixes that appear as `name::` in
/// `content`, scanning code and format-string interpolations but skipping line
/// comments and plain string text.  Order-preserving, de-duplicated.
#[must_use]
pub fn scan_qualified_lib_refs(content: &str) -> Vec<String> {
    let b = content.as_bytes();
    let n = b.len();
    let mut refs: Vec<String> = Vec::new();
    let mut i = 0usize;
    // String state: when `in_string`, plain text is skipped; only code inside
    // `{…}` interpolation (brace_depth > 0) is scanned.
    let mut in_string = false;
    let mut brace_depth = 0usize;

    let record = |name: &str, refs: &mut Vec<String>| {
        // lowercase first char → library candidate (types are capitalised)
        if name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            && !refs.iter().any(|r| r == name)
        {
            refs.push(name.to_string());
        }
    };

    while i < n {
        let c = b[i];
        if !in_string {
            // line comment
            if c == b'/' && i + 1 < n && b[i + 1] == b'/' {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if c == b'"' {
                in_string = true;
                brace_depth = 0;
                i += 1;
                continue;
            }
            if is_ident_start(c) {
                let start = i;
                while i < n && is_ident_char(b[i]) {
                    i += 1;
                }
                if i + 1 < n && b[i] == b':' && b[i + 1] == b':' {
                    record(&content[start..i], &mut refs);
                }
                continue;
            }
            i += 1;
        } else {
            match c {
                b'\\' => i += 2, // escape — skip the next byte
                b'{' => {
                    brace_depth += 1;
                    i += 1;
                }
                b'}' => {
                    brace_depth = brace_depth.saturating_sub(1);
                    i += 1;
                }
                b'"' if brace_depth == 0 => {
                    in_string = false;
                    i += 1;
                }
                _ if brace_depth > 0 && is_ident_start(c) => {
                    let start = i;
                    while i < n && is_ident_char(b[i]) {
                        i += 1;
                    }
                    if i + 1 < n && b[i] == b':' && b[i + 1] == b':' {
                        record(&content[start..i], &mut refs);
                    }
                }
                _ => i += 1,
            }
        }
    }
    refs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_plain_code_reference() {
        assert_eq!(
            scan_qualified_lib_refs("fn f() { x = regex::find(\"a\", line) }"),
            vec!["regex".to_string()]
        );
    }

    #[test]
    fn finds_reference_inside_format_string() {
        // The common case: a `lib::` call inside `{…}` interpolation.
        assert_eq!(
            scan_qualified_lib_refs("print(\"hit at {regex::find(p, s)}\\n\")"),
            vec!["regex".to_string()]
        );
    }

    #[test]
    fn skips_plain_string_text_and_comments() {
        // `::` in plain (non-interpolated) text and in a comment are NOT refs.
        assert!(scan_qualified_lib_refs("// see regex::docs for more").is_empty());
        assert!(scan_qualified_lib_refs("s = \"path is a::b not a lib\"").is_empty());
    }

    #[test]
    fn skips_capitalised_enum_variants() {
        // `Enum::Variant` (capitalised) is a type, never a library.
        assert!(scan_qualified_lib_refs("match c { Color::Red => 1, Shape::Box => 2 }").is_empty());
    }

    #[test]
    fn dedups_and_keeps_order() {
        assert_eq!(
            scan_qualified_lib_refs("a::x; b::y; a::z"),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn mixed_code_and_interpolation() {
        let src = "fn f() {\n  if regex::matches(p, s) { print(\"{time::now()}\") }\n}";
        assert_eq!(
            scan_qualified_lib_refs(src),
            vec!["regex".to_string(), "time".to_string()]
        );
    }
}
