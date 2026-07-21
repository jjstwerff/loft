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
use crate::host::{Program, Value};
use crate::lexer::Position;
use crate::parser::Parser;

/// Load the stdlib into a fresh parser: warm-load the precompiled `Data` bundle
/// when it's available (`startup_cache` — the same bundle the `loft` CLI keeps —
/// ~12× faster than re-parsing `default/`), else cold-parse and save the bundle
/// so the next call is warm.  Both paths are gated on `LOFT_STDLIB_CACHE`; with
/// it unset (the test harness) this degrades to a plain cold parse.  A fresh
/// parser per call is still mandatory — loft registers every definition *per
/// source*, so reusing a warm parser re-registers and conflicts (`"Cannot
/// redefine 'main'"`); the cache short-circuits the stdlib *work*, not the rule.
fn load_stdlib(p: &mut Parser, stdlib_dir: &str) {
    // The stdlib prelude (STD_SOURCE) must be registered before the user buffer,
    // or every stdlib symbol reads as undefined — both paths do that first.
    if !crate::startup_cache::warm_load_stdlib(p, stdlib_dir) {
        let _ = p.parse_dir(stdlib_dir, true, false);
        crate::startup_cache::save_stdlib_cache(p, stdlib_dir);
    }
}

/// Parse `text` as a standalone loft source — with the stdlib in `stdlib_dir`
/// loaded first — and return its diagnostics (positioned, coded; @I75).  The
/// caller resolves `stdlib_dir` (a deployment concern the binary owns, exactly
/// as the `loft` CLI does), so this stays a pure function of its inputs.
pub fn diagnose(text: &str, name: &str, stdlib_dir: &str) -> Diagnostics {
    let mut p = Parser::new();
    load_stdlib(&mut p, stdlib_dir);
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
    load_stdlib(&mut p, stdlib_dir);
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
    /// The resolved user-facing name (`area`, `Point`, `print`).
    pub name: String,
    /// A one-line signature: `fn area(p: Point) -> integer`, `struct Point { … }`,
    /// `const PI: float`.  The clean user-facing type spelling (`type_name_str`).
    pub signature: String,
    /// The `///` doc block immediately above the declaration, in reading order —
    /// empty when the definition has none (or its source can't be read).
    pub doc: Vec<String>,
    /// The resolved definition's source file (as the parser recorded it —
    /// the buffer's parse-name for a local def, else a repo-root-relative path).
    pub def_file: String,
    /// 1-based line of the definition's NAME.
    pub def_line: u32,
    /// 1-based column of the definition's NAME (located in the source, so a jump
    /// lands on the name — not the parser's body-start position).
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
    load_stdlib(&mut p, stdlib_dir);
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
    hover_of_def(data, d, text, name, stdlib_dir)
}

/// Resolve a symbol BY NAME (not by cursor) to every definition that name spells:
/// a free function (`n_<symbol>`), a type / struct / enum / typedef / constant
/// (`<symbol>`), AND every method `Type.<symbol>` — so `lookup("len", …)` returns
/// a free `len` plus `text.len`, `vector.len`, … each with its own signature +
/// doc + location.  The agent-facing counterpart to [`symbol_at`]: you know the
/// NAME, not a `(line, col)`.  Empty when nothing spells it.  Fresh parse per
/// call (same rule as [`diagnose`]); `text` may be empty to search the stdlib alone.
#[must_use]
pub fn lookup(symbol: &str, text: &str, name: &str, stdlib_dir: &str) -> Vec<Hover> {
    let mut p = Parser::new();
    load_stdlib(&mut p, stdlib_dir);
    p.parse_source(text, name, false);
    let data = &p.data;
    let mut out: Vec<Hover> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    let take = |d: u32, out: &mut Vec<Hover>, seen: &mut Vec<u32>| {
        if d == u32::MAX || seen.contains(&d) {
            return;
        }
        let def = data.def(d);
        // Skip machinery: compiler-`synthetic` defs, and the `Dynamic`
        // multi-receiver DISPATCHER (bare `len` → `fn len(text: fn, character:
        // fn, …) -> unknown`) — a degenerate umbrella signature.  The concrete
        // `text.len` / `vector.len` (each a `Function`) carry the real info and
        // come through the method scan below.
        if def.synthetic.is_some() || matches!(def.def_type(), crate::data::DefType::Dynamic) {
            return;
        }
        seen.push(d);
        if let Some(h) = hover_of_def(data, d, text, name, stdlib_dir) {
            out.push(h);
        }
    };
    // Direct: a free function, then a type-like def.
    take(data.def_nr(&format!("n_{symbol}")), &mut out, &mut seen);
    take(data.def_nr(symbol), &mut out, &mut seen);
    // Methods: `t_<LEN><Type>_<symbol>` decode to `Type.<symbol>` — scan and match
    // the trailing segment so `lookup("len")` also surfaces `text.len` etc.
    for d in 0..data.definitions() {
        if let Some((kind, cname)) = crate::api_surface::classify(data, d)
            && kind == "method"
            && cname.rsplit('.').next() == Some(symbol)
        {
            take(d, &mut out, &mut seen);
        }
    }
    out
}

/// Build a [`Hover`] for a resolved definition `d`: signature (via
/// `api_surface::signature_of`) + the `///` doc and name-precise location read
/// from the def's own source (the buffer for a local def, the file on disk for
/// stdlib/library ones).  Shared by [`symbol_at`] and [`lookup`].
fn hover_of_def(data: &Data, d: u32, text: &str, name: &str, stdlib_dir: &str) -> Option<Hover> {
    let (kind, cname) = crate::api_surface::classify(data, d)?;
    let pos = data.def(d).position.clone();
    // Read the definition's own source ONCE — for the `///` doc AND to locate the
    // name (the parser records `pos` at the body start, past the name).
    let src = read_def_source(text, name, stdlib_dir, &pos);
    let doc = src
        .as_deref()
        .map_or_else(Vec::new, |s| doc_block_above(s, pos.line));
    let def_col = src
        .as_deref()
        .and_then(|s| name_col_on_line(s, pos.line, &cname))
        .unwrap_or(pos.pos);
    Some(Hover {
        signature: render_signature(data, d, kind, &cname),
        name: cname,
        doc,
        def_file: collapse_slashes(&pos.file),
        def_line: pos.line,
        def_col,
    })
}

/// Collapse repeated `/` in a path.  The stdlib startup-cache can bake a `//`
/// into a def's recorded file (a trailing-separator base dir joined with a
/// name), and a doubled slash in a `file:line` reference reads as unpolished.
fn collapse_slashes(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut prev_slash = false;
    for c in p.chars() {
        if c == '/' && prev_slash {
            continue;
        }
        prev_slash = c == '/';
        out.push(c);
    }
    out
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

/// The definition's own source text: the open buffer when `pos` points into it,
/// else the file on disk (stdlib / imported libs).  `pos.file` is repo-root
/// relative (e.g. `default/01_code.loft`); the stdlib root is the parent of
/// `stdlib_dir` (`…/default`).  `None` when the source can't be read.
fn read_def_source(buf: &str, buf_name: &str, stdlib_dir: &str, pos: &Position) -> Option<String> {
    if pos.file == buf_name {
        return Some(buf.to_string());
    }
    let root = Path::new(stdlib_dir)
        .parent()
        .map_or_else(|| Path::new("").to_path_buf(), Path::to_path_buf);
    std::fs::read_to_string(root.join(&pos.file)).ok()
}

/// The contiguous `///` doc block directly above the declaration on `decl_line`
/// (1-based), in reading order.  Empty when there is none.
fn doc_block_above(src: &str, decl_line: u32) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut doc: Vec<String> = Vec::new();
    // The line above the declaration is index `decl_line - 2`.
    let mut i = decl_line as isize - 2;
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

/// The 1-based column of `name` on `decl_line` (1-based) in `src`, so a jump
/// lands on the name rather than the parser's body-start position.  A method
/// name (`Type.method`) is searched by its last segment.  `None` if not found
/// on that line (e.g. a multi-line signature) — the caller keeps `pos.pos`.
fn name_col_on_line(src: &str, decl_line: u32, name: &str) -> Option<u32> {
    let needle = name.rsplit('.').next().unwrap_or(name);
    let line = src.lines().nth(decl_line.saturating_sub(1) as usize)?;
    let byte_idx = line.find(needle)?;
    Some(line[..byte_idx].chars().count() as u32 + 1)
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

/// The loft source formatter (`tools/fmt/whole.loft`) compiled to a runnable
/// program — the SAME formatter the `loft fmt` CLI runs, so the LSP and the CLI
/// produce identical output.  Compiling loads the stdlib + the formatter program
/// and is heavy (~hundreds of ms), so build ONE and reuse it across requests.
pub struct Formatter {
    prog: Program,
}

impl Formatter {
    /// Compile the formatter with the stdlib in `stdlib_dir`.  `None` if it can't
    /// be built (e.g. the stdlib can't be found or loaded).
    #[must_use]
    pub fn new(stdlib_dir: &str) -> Option<Formatter> {
        // Relative to THIS file (`src/lsp.rs`); the same `include_str!` target the
        // CLI's `run_fmt_command` uses, so there is one formatter source of truth.
        const FMT_SRC: &str = include_str!("../tools/fmt/whole.loft");
        Program::from_source_with_stdlib(FMT_SRC, stdlib_dir)
            .ok()
            .map(|prog| Formatter { prog })
    }

    /// Format `text`, or `None` if the formatter itself errors (never on a
    /// no-op — a well-formatted buffer returns itself unchanged).
    pub fn format(&mut self, text: &str) -> Option<String> {
        self.prog
            .call("format", &[Value::Text(text.to_string())])
            .ok()?
            .into_text()
            .ok()
    }
}
