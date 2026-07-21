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

use crate::data::MAIN_SOURCE;
use crate::diagnostics::Diagnostics;
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
