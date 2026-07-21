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
