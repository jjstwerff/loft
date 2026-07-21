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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::data::{Data, MAIN_SOURCE};
use crate::diagnostics::Diagnostics;
use crate::host::{Program, Value};
use crate::json::Parsed;
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

/// The identifier under a 1-based (`line`, `col`) cursor — the token that
/// find-references / rename resolve.  Public wrapper over `identifier_span_at`.
#[must_use]
pub fn identifier_at(text: &str, line: u32, col: u32) -> Option<String> {
    identifier_span_at(text, line, col).map(|(name, ..)| name)
}

/// The identifier under a 1-based (`line`, `col`) cursor as `(name, start_col,
/// end_col)` — the cols 0-based — or `None` if not on a word.  Right-edge
/// anchoring: a cursor just past a token still selects it.  `prepareRename` uses
/// the span for the highlight range.
#[must_use]
pub fn identifier_span_at(text: &str, line: u32, col: u32) -> Option<(String, u32, u32)> {
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
    Some((chars[start..end].iter().collect(), start as u32, end as u32))
}

fn word_at(text: &str, line: u32, col: u32) -> Option<String> {
    identifier_at(text, line, col)
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

// ── tracker-tag integration (@PLN63 T1/T2) ───────────────────────────────────
// Surface loft's own `@`-tracker system (issues / features / plans) inline,
// reading the ALREADY-generated `index/tags.json` + `index/features.json`
// (`make index`).  Lights up in the loft repo; inert elsewhere (files absent).

/// One tracker tag's information, gathered from the generated index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    /// The tag as written, e.g. `@F7`.
    pub tag: String,
    /// Display kind: `feature` `infra` `problem` `plan` `github` `plan-legacy` `tag`.
    pub kind: &'static str,
    /// The feature/infra title (`@F`/`@I` only), from `features.json`.
    pub title: Option<String>,
    /// A short description — the feature body's first paragraph, else the first
    /// indexed reference's source line.
    pub summary: Option<String>,
    /// The canonical issue URL (`@GH`/`@PLN`/`@F`/`@I`).
    pub url: Option<String>,
    /// How many references the index has for this tag.
    pub references: usize,
}

/// The generated tracker index (`index/tags.json` + `index/features.json`),
/// parsed once and queried per tag.  `load` returns `None` outside the loft repo
/// (the files are absent) — the caller treats that as "no tag info", not an error.
pub struct TagIndex {
    tags: Parsed,
    features: Parsed,
    /// Tags the scanner deemed BROKEN (its `broken` array — `@P`/`@PLAN` that
    /// resolve to no valid PID/plan).  The scanner is the authority (it reads
    /// PROBLEMS.md + the plan dirs + the freeze-banner map); we consume its
    /// verdict rather than re-deriving it.
    broken: Vec<String>,
}

impl TagIndex {
    /// Parse `<index_dir>/tags.json` (required) + `features.json` (optional).
    #[must_use]
    pub fn load(index_dir: &str) -> Option<TagIndex> {
        let tags_txt = std::fs::read_to_string(Path::new(index_dir).join("tags.json")).ok()?;
        let tags = crate::json::parse(&tags_txt).ok()?;
        let features = std::fs::read_to_string(Path::new(index_dir).join("features.json"))
            .ok()
            .and_then(|s| crate::json::parse(&s).ok())
            .unwrap_or(Parsed::Array(Vec::new()));
        // The `broken` array is `[{"tag":"@P999","refs":[…]}, …]`.
        let broken = match pj_get(&tags, "broken") {
            Some(Parsed::Array(items)) => items.iter().filter_map(|it| pj_str(it, "tag")).collect(),
            _ => Vec::new(),
        };
        Some(TagIndex {
            tags,
            features,
            broken,
        })
    }

    /// Whether the scanner flagged `tag` as broken — a `@P`/`@PLAN` reference to
    /// no valid issue/plan.  From the index's `broken` array, so it inherits ALL
    /// the scanner's validation (PROBLEMS.md, plan dirs, the freeze-banner map)
    /// with no re-derivation.  NOTE: a freshly-typed broken tag not yet indexed
    /// won't show until the next `make index` — the index is the source of truth.
    #[must_use]
    pub fn is_broken(&self, tag: &str) -> bool {
        self.broken.iter().any(|t| t == tag)
    }

    /// Everything the index knows about `tag`, or `None` if it is neither
    /// referenced nor a well-formed issue tag (a probable typo → T3 territory).
    #[must_use]
    pub fn lookup(&self, tag: &str) -> Option<TagInfo> {
        let (fam, num) = split_tag(tag)?;
        let refs = pj_get(&self.tags, tag);
        let references = match refs {
            Some(Parsed::Array(a)) => a.len(),
            _ => 0,
        };
        // Deterministic issue URLs per tag family (the repos in CLAUDE.md).
        let (kind, url) = match fam {
            "GH" => (
                "github",
                Some(format!("https://github.com/loft-lang/loft/issues/{num}")),
            ),
            "PLN" => (
                "plan",
                Some(format!("https://github.com/loft-lang/plans/issues/{num}")),
            ),
            "F" => (
                "feature",
                Some(format!(
                    "https://github.com/loft-lang/features/issues/{num}"
                )),
            ),
            "I" => (
                "infra",
                Some(format!(
                    "https://github.com/loft-lang/features/issues/{num}"
                )),
            ),
            "P" => ("problem", None),
            "PLAN" => ("plan-legacy", None),
            _ => ("tag", None),
        };
        let (title, summary) = if fam == "F" || fam == "I" {
            match self.feature(num) {
                Some(f) => (
                    pj_str(f, "title"),
                    pj_str(f, "body").map(|b| first_para(&b)),
                ),
                None => (None, first_ref_context(refs)),
            }
        } else {
            (None, first_ref_context(refs))
        };
        // A local tag (`@P`/`@PLAN`) with no references and no title is unknown.
        if references == 0 && title.is_none() && url.is_none() {
            return None;
        }
        Some(TagInfo {
            tag: tag.to_string(),
            kind,
            title,
            summary,
            url,
            references,
        })
    }

    fn feature(&self, num: i64) -> Option<&Parsed> {
        match &self.features {
            Parsed::Array(items) => items.iter().find(|it| pj_i64(it, "number") == Some(num)),
            _ => None,
        }
    }
}

/// A [`TagInfo`] as a markdown block — the LSP hover body and `loft tag` output.
#[must_use]
pub fn render_tag_markdown(info: &TagInfo) -> String {
    use std::fmt::Write;
    let mut s = format!("**{}** · {}", info.tag, info.kind);
    if let Some(url) = &info.url {
        let _ = write!(s, "  ·  [issue]({url})");
    }
    if let Some(title) = &info.title {
        let _ = write!(s, "\n\n**{title}**");
    }
    if let Some(summary) = &info.summary {
        let _ = write!(s, "\n\n{summary}");
    }
    let plural = if info.references == 1 {
        "reference"
    } else {
        "references"
    };
    let _ = write!(s, "\n\n_{} {plural} indexed_", info.references);
    s
}

/// Split `@F7` → `("F", 7)`; `None` if not a well-formed `@<UPPER>+<digits>` tag.
fn split_tag(tag: &str) -> Option<(&str, i64)> {
    let body = tag.strip_prefix('@')?;
    let split = body.find(|c: char| c.is_ascii_digit())?;
    let (fam, rest) = body.split_at(split);
    if fam.is_empty() || !fam.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let num = digits.parse::<i64>().ok()?;
    Some((fam, num))
}

/// The first real paragraph of a feature body — skip headings / code fences.
fn first_para(body: &str) -> String {
    for para in body.split("\n\n") {
        let t = para.trim();
        if !t.is_empty() && !t.starts_with('#') && !t.starts_with("```") {
            return t.replace('\n', " ");
        }
    }
    String::new()
}

fn first_ref_context(refs: Option<&Parsed>) -> Option<String> {
    match refs {
        Some(Parsed::Array(a)) => a.first().and_then(|r| pj_str(r, "context")).map(|c| {
            let t = c.trim();
            t.chars().take(200).collect()
        }),
        _ => None,
    }
}

/// Tracker tags in one line as `(tag, start_col, end_col)` — 0-based char cols.
/// A tag is `@` + uppercase letters + digits, with an optional single lowercase
/// suffix (`@P259a`).  Deep segment forms (`@PLAN22-2d`) resolve to their base.
fn tags_in_line(line: &str) -> Vec<(String, usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        let alpha0 = j;
        while j < chars.len() && chars[j].is_ascii_uppercase() {
            j += 1;
        }
        let digit0 = j;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j > digit0 && digit0 > alpha0 {
            // A single trailing lowercase (e.g. `@P259a`) when word-bounded.
            if j < chars.len()
                && chars[j].is_ascii_lowercase()
                && (j + 1 >= chars.len() || !chars[j + 1].is_ascii_alphanumeric())
            {
                j += 1;
            }
            out.push((chars[start..j].iter().collect(), start, j));
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The tracker tag under a 1-based (`line`, `col`) cursor, or `None`.
#[must_use]
pub fn tag_at(text: &str, line: u32, col: u32) -> Option<String> {
    let line_str = text.lines().nth(line.saturating_sub(1) as usize)?;
    let c = col.saturating_sub(1) as usize;
    tags_in_line(line_str)
        .into_iter()
        .find(|(_, s, e)| *s <= c && c < *e)
        .map(|(t, _, _)| t)
}

/// Every tag occurrence in `text` as `(tag, line, start_col, end_col)` — 1-based
/// line, 0-based cols — for `documentLink`.
#[must_use]
pub fn tags_in(text: &str) -> Vec<(String, u32, u32, u32)> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (tag, s, e) in tags_in_line(line) {
            out.push((tag, i as u32 + 1, s as u32, e as u32));
        }
    }
    out
}

// ── Parsed accessors (over loft::json) ───────────────────────────────────────
fn pj_get<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(e) => e.iter().find(|(k, _, _)| k == key).map(|(_, _, val)| val),
        _ => None,
    }
}

fn pj_str(v: &Parsed, key: &str) -> Option<String> {
    match pj_get(v, key) {
        Some(Parsed::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn pj_i64(v: &Parsed, key: &str) -> Option<i64> {
    pj_get(v, key).and_then(Parsed::as_i64)
}

// ── workspace reverse index (@PLN63 — find-references / rename) ───────────────
// A workspace-wide reverse index: every identifier's occurrences across the
// `.loft` files under a root, keyed by name.  Built by LEXING each file (loft's
// own lexer), so comments / strings / keywords are excluded and `x.len` yields a
// `len` identifier — a precise-but-unscoped finder (name-keyed, not type-resolved:
// `references("len")` returns every `len` token across all types).

/// One occurrence of an identifier in the workspace — a use site or a definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Absolute file path (canonicalized).
    pub file: String,
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub col: u32,
}

/// A reverse index of identifier occurrences across a workspace's `.loft` files.
pub struct WorkspaceIndex {
    by_name: HashMap<String, Vec<Reference>>,
}

impl WorkspaceIndex {
    /// Build by lexing every `.loft` file under `root` (skipping build / VCS /
    /// dependency dirs).  Reflects the SAVED files; the caller overlays open,
    /// edited buffers at query time (`identifier_refs`).
    #[must_use]
    pub fn build(root: &str) -> WorkspaceIndex {
        let mut by_name: HashMap<String, Vec<Reference>> = HashMap::new();
        for path in loft_files(Path::new(root)) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let file = canonical(&path);
            for (name, r) in scan_identifiers(&text, &file) {
                by_name.entry(name).or_default().push(r);
            }
        }
        WorkspaceIndex { by_name }
    }

    /// Every recorded occurrence of `name` (the SAVED-file view).
    #[must_use]
    pub fn references(&self, name: &str) -> &[Reference] {
        self.by_name.get(name).map_or(&[][..], Vec::as_slice)
    }

    /// References to `name` with open buffers overlaid: for each open document,
    /// its live refs replace its stale disk refs (matched by canonical path), so
    /// find-references reflects unsaved edits.  `overlays` is `(file_path, text)`
    /// per open document.  Sorted by (file, line, col).
    #[must_use]
    pub fn references_overlaid(&self, name: &str, overlays: &[(String, String)]) -> Vec<Reference> {
        let open: Vec<(String, &str)> = overlays
            .iter()
            .map(|(p, t)| (canonical(Path::new(p)), t.as_str()))
            .collect();
        let mut refs: Vec<Reference> = self
            .references(name)
            .iter()
            .filter(|r| !open.iter().any(|(p, _)| *p == r.file))
            .cloned()
            .collect();
        for (path, text) in &open {
            refs.extend(identifier_refs(text, path, name));
        }
        refs.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.col.cmp(&b.col))
        });
        refs
    }
}

/// Every occurrence of the identifier `name` in `text`, tagged with `file`.  A
/// fresh lex — the overlay for an open, edited buffer (its live refs replace the
/// stale disk ones).
#[must_use]
pub fn identifier_refs(text: &str, file: &str, name: &str) -> Vec<Reference> {
    scan_identifiers(text, file)
        .into_iter()
        .filter(|(n, _)| n == name)
        .map(|(_, r)| r)
        .collect()
}

/// Lex `text` and collect every non-keyword identifier as `(name, Reference)`.
fn scan_identifiers(text: &str, file: &str) -> Vec<(String, Reference)> {
    use crate::lexer::{LexItem, Lexer};
    let mut lexer = Lexer::default();
    lexer.parse_string(text, file);
    let mut out = Vec::new();
    // `restart()` (inside `parse_string`) already advanced to the first token,
    // so peek FIRST, then `cont()`.
    loop {
        let r = lexer.peek();
        match r.has {
            LexItem::None => break,
            LexItem::Identifier(name) if !crate::lexer::is_keyword(&name) => {
                out.push((
                    name,
                    Reference {
                        file: file.to_string(),
                        line: r.position.line,
                        col: r.position.pos,
                    },
                ));
            }
            _ => {}
        }
        lexer.cont();
    }
    out
}

/// Canonical absolute path string (falls back to the lossy path on error).
fn canonical(p: &Path) -> String {
    std::fs::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Every `.loft` file under `root`, skipping build / VCS / dependency dirs.
fn loft_files(root: &Path) -> Vec<PathBuf> {
    const SKIP: &[&str] = &[
        "target",
        ".git",
        ".loft",
        "node_modules",
        ".claude",
        ".cache",
    ];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !SKIP.contains(&name.as_ref()) && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if name.ends_with(".loft") {
                out.push(path);
            }
        }
    }
    out
}

// ── rename (on top of the reverse index) ─────────────────────────────────────

/// Whether `name` is a syntactically valid loft identifier (and not a keyword) —
/// the guard on a rename's NEW name.
#[must_use]
pub fn is_valid_identifier(name: &str) -> bool {
    let mut cs = name.chars();
    if !matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') {
        return false;
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !crate::lexer::is_keyword(name)
}

/// A stdlib source path (under a `default/` dir) — files a rename must never edit.
fn is_stdlib_path(file: &str) -> bool {
    file.contains("/default/") || file.ends_with("/default")
}

/// For `prepareRename`: the identifier's span at the cursor IF it is renamable —
/// on an identifier that is NOT a standard-library symbol.  `None` otherwise, so
/// the editor won't offer to rename a stdlib name.
#[must_use]
pub fn prepare_rename(
    text: &str,
    line: u32,
    col: u32,
    stdlib_dir: &str,
) -> Option<(String, u32, u32)> {
    let span = identifier_span_at(text, line, col)?;
    if !lookup(&span.0, "", "query.loft", stdlib_dir).is_empty() {
        return None; // a stdlib symbol — not renamable
    }
    Some(span)
}

/// Plan a workspace-wide rename of `old` → `new`: the reference locations to
/// rewrite, or an error message explaining why the rename is refused.
///
/// Guards (SAFE by default): the new name must be a valid identifier; the old
/// name must not resolve to a **standard-library** symbol (`print`, `len`, …);
/// and no reference may live in a stdlib file — renaming those would corrupt the
/// shared library.  NOTE: lexical / name-keyed (like find-references), so it best
/// fits GLOBAL user symbols whose name is unique across the workspace; a local
/// variable or a name reused across scopes is renamed everywhere it appears.
///
/// # Errors
/// Returns a human-readable message when the rename is refused: the new name is
/// not a valid identifier, `old` is a stdlib symbol or is referenced by stdlib
/// files, or there are no references to `old`.
pub fn plan_rename(
    old: &str,
    new: &str,
    index: &WorkspaceIndex,
    overlays: &[(String, String)],
    stdlib_dir: &str,
) -> Result<Vec<Reference>, String> {
    if !is_valid_identifier(new) {
        return Err(format!("`{new}` is not a valid identifier"));
    }
    if new == old {
        return Ok(Vec::new());
    }
    if !lookup(old, "", "query.loft", stdlib_dir).is_empty() {
        return Err(format!(
            "`{old}` is a standard-library symbol — refusing to rename it"
        ));
    }
    let refs = index.references_overlaid(old, overlays);
    if refs.iter().any(|r| is_stdlib_path(&r.file)) {
        return Err(format!(
            "`{old}` is used by the standard library — refusing to rename it"
        ));
    }
    if refs.is_empty() {
        return Err(format!("no references to `{old}` in the workspace"));
    }
    Ok(refs)
}
