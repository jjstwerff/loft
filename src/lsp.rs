// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 — the library-side support surface for `loft-lsp` (the Language
// Server binary in `src/bin/loft-lsp.rs`).  The binary owns the wire protocol;
// this module owns the loft-compiler calls, so the compiler coupling lives in
// the library and stays testable without spawning a server (`tests/lsp_*`).
//
// Feature providers land here step by step: S3 diagnostics (this file), then
// S4 outline / S5 hover / S6 go-to-definition reuse the same fresh-parse.

use std::path::Path;

use crate::data::{Data, MAIN_SOURCE};
use crate::diagnostics::Diagnostics;
use crate::lexer::Position;
use crate::parser::Parser;

/// Parse `text` as a standalone loft source — with the stdlib in `stdlib_dir`
/// loaded first — and return its diagnostics (positioned, coded; @I75).
///
/// A **fresh** parser per call is mandatory, not an optimization gap: loft
/// registers every definition *per source* (that is how files read each other
/// on `use`), so a second parse on the same parser re-registers and conflicts
/// (`"Cannot redefine 'main'"`).  Re-parsing the stdlib each call is the ~80 ms
/// cost of that rule — within the per-edit LSP budget.  The caller resolves
/// `stdlib_dir` (a deployment concern the binary owns, exactly as the `loft`
/// CLI does), so this stays a pure function of its inputs.
pub fn diagnose(text: &str, name: &str, stdlib_dir: &str) -> Diagnostics {
    let mut p = Parser::new();
    // Load order matters: the stdlib prelude (STD_SOURCE) must be registered
    // before the user buffer, or every stdlib symbol reads as undefined.
    let _ = p.parse_dir(stdlib_dir, true, false);
    p.parse_source(text, name, false);
    std::mem::take(&mut p.diagnostics)
}

/// One top-level definition in a buffer, for the editor Outline / breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// User-facing name, internal encodings stripped: `main`, `text.len`, `Point`.
    pub name: String,
    /// Display kind: `fn` `method` `operator` `struct` `enum` `typedef` `constant`
    /// `interface` — the `api_surface::classify` label set.
    pub kind: &'static str,
    /// 1-based source line of the definition.
    pub line: u32,
    /// 1-based source column of the definition (`Position::pos`).
    pub col: u32,
}

/// The top-level definitions the buffer `text` declares (the user source,
/// `MAIN_SOURCE`) — name, kind, and position — ordered by source position.
/// Drives `textDocument/documentSymbol`.
///
/// Fresh parse per call, the same rule as [`diagnose`].  Only the user buffer's
/// own definitions are returned: stdlib defs live at `STD_SOURCE`, and
/// compiler-`synthetic` defs (e.g. `__nullable<S>`) are excluded — an outline
/// shows what the user wrote, not what the compiler manufactured.  Kind and the
/// decoded name come from the shared [`crate::api_surface::classify`], so an
/// enum VARIANT (part of its enum's shape) is folded out, not listed top-level.
pub fn outline(text: &str, name: &str, stdlib_dir: &str) -> Vec<Symbol> {
    let mut p = Parser::new();
    let _ = p.parse_dir(stdlib_dir, true, false);
    p.parse_source(text, name, false);
    let data = &p.data;
    let mut symbols: Vec<Symbol> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            def.source == MAIN_SOURCE && def.synthetic.is_none()
        })
        .filter_map(|d| {
            let (kind, name) = crate::api_surface::classify(data, d)?;
            let pos = &data.def(d).position;
            Some(Symbol {
                name,
                kind,
                line: pos.line,
                col: pos.pos,
            })
        })
        .collect();
    symbols.sort_by_key(|s| (s.line, s.col));
    symbols
}

/// Hover information for the symbol under a cursor: its signature, its `///` doc,
/// and where it is defined (the location also drives S6 go-to-definition).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// A one-line signature: `fn area(p: Point) -> integer`, `struct Point { … }`,
    /// `const PI: float`.  The clean user-facing type spelling (`type_name_str`).
    pub signature: String,
    /// The `///` doc block immediately above the declaration, in reading order —
    /// empty when the definition has none (or its source can't be read).
    pub doc: Vec<String>,
    /// The resolved definition's source file (as the parser recorded it).
    pub def_file: String,
    /// 1-based line of the definition.
    pub def_line: u32,
    /// 1-based column of the definition.
    pub def_col: u32,
}

/// Resolve the identifier under the cursor at (`line`, `col`) — both 1-based,
/// loft-native (the binary converts from LSP's 0-based) — to its definition and
/// return a [`Hover`]: signature + `///` doc + the definition's location.
///
/// Name-based resolution: the identifier is looked up as a free function
/// (`n_<word>`) then as a type / struct / enum / typedef / constant (`<word>`),
/// each falling back to the stdlib source — so hovering `print` resolves.
/// Methods (`t_<LEN><Type>_…`, which need the receiver type) and local variables
/// are NOT resolved here; that needs the position-aware index (a later step).
/// Fresh parse per call, the same rule as [`diagnose`].
#[must_use]
pub fn symbol_at(text: &str, name: &str, stdlib_dir: &str, line: u32, col: u32) -> Option<Hover> {
    let word = word_at(text, line, col)?;
    let mut p = Parser::new();
    let _ = p.parse_dir(stdlib_dir, true, false);
    p.parse_source(text, name, false);
    let data = &p.data;
    // A free function first, then a type-like def; `def_nr` checks the user source
    // then falls back to the stdlib (STD_SOURCE), so both user and stdlib resolve.
    let mut d = data.def_nr(&format!("n_{word}"));
    if d == u32::MAX {
        d = data.def_nr(&word);
    }
    if d == u32::MAX {
        return None;
    }
    let (kind, cname) = crate::api_surface::classify(data, d)?;
    let pos = data.def(d).position.clone();
    Some(Hover {
        signature: render_signature(data, d, kind, &cname),
        doc: doc_above(text, name, stdlib_dir, &pos),
        def_file: pos.file,
        def_line: pos.line,
        def_col: pos.pos,
    })
}

/// `<keyword> <name><body>` — the signature body from `api_surface::signature_of`
/// joined with the right spacing (fn/const bodies open with `(`/`:`, no space; the
/// rest open with `{`/`=`, one space).
fn render_signature(data: &Data, d: u32, kind: &str, name: &str) -> String {
    let body = crate::api_surface::signature_of(data, d, kind);
    let kw = match kind {
        "fn" | "method" => "fn",
        "struct" => "struct",
        "enum" => "enum",
        "typedef" => "type",
        "constant" => "const",
        "interface" => "interface",
        other => other, // "operator" and any future label render as-is
    };
    match kind {
        "fn" | "method" | "operator" | "constant" => format!("{kw} {name}{body}"),
        _ => format!("{kw} {name} {body}"),
    }
}

/// The contiguous `///` doc block directly above the declaration at `pos`, in
/// reading order.  Reads from the open buffer when `pos` points into it, else
/// from the definition's source file on disk (stdlib / imported libs), resolved
/// relative to the stdlib root.  Empty on no doc or an unreadable source.
fn doc_above(buf: &str, buf_name: &str, stdlib_dir: &str, pos: &Position) -> Vec<String> {
    let disk;
    let src: &str = if pos.file == buf_name {
        buf
    } else {
        // `pos.file` is repo-root-relative (e.g. `default/01_code.loft`); the
        // stdlib root is the parent of `stdlib_dir` (`…/default`).
        let root = Path::new(stdlib_dir)
            .parent()
            .map_or_else(|| Path::new("").to_path_buf(), Path::to_path_buf);
        match std::fs::read_to_string(root.join(&pos.file)) {
            Ok(s) => {
                disk = s;
                disk.as_str()
            }
            Err(_) => return Vec::new(),
        }
    };
    let lines: Vec<&str> = src.lines().collect();
    let mut doc: Vec<String> = Vec::new();
    // `pos.line` is 1-based; the line above the declaration is index `line - 2`.
    let mut i = pos.line as isize - 2;
    while i >= 0 {
        let trimmed = lines[i as usize].trim_start();
        let Some(rest) = trimmed.strip_prefix("///") else {
            break;
        };
        doc.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        i -= 1;
    }
    doc.reverse();
    doc
}

/// The identifier token under a 1-based (`line`, `col`) cursor in `text`, or
/// `None` if the cursor is not on a word.  A cursor at a token's right edge still
/// anchors on it.
fn word_at(text: &str, line: u32, col: u32) -> Option<String> {
    let line_str = text.lines().nth(line.saturating_sub(1) as usize)?;
    let chars: Vec<char> = line_str.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut anchor = (col.saturating_sub(1) as usize).min(chars.len() - 1);
    if !is_word(chars[anchor]) {
        if anchor == 0 || !is_word(chars[anchor - 1]) {
            return None;
        }
        anchor -= 1;
    }
    let mut start = anchor;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = anchor + 1;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    Some(chars[start..end].iter().collect())
}
