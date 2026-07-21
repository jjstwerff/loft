// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// loft-lsp — the loft Language Server (LSP over JSON-RPC / stdio).
//
// @PLN63 S1 transport + S3 diagnostics.  The read-dispatch loop handles the
// `initialize` / `shutdown` / `exit` lifecycle (S1) and the document lifecycle
// (`didOpen` / `didChange` / `didClose`); on each edit it re-parses the buffer
// with a fresh stdlib-loaded parser (`loft::lsp::diagnose`) and pushes
// `textDocument/publishDiagnostics` so the editor shows squiggles live.  The
// later providers (outline / hover / definition, S4-S6) hang off this same loop.
//
// The compiler coupling lives in the library (`loft::lsp`), so this binary owns
// only the wire protocol + deployment concerns (locating the stdlib).
//
// The JSON on the wire is loft's OWN parser/serializer (`loft::json`), not an
// external crate — the same "own your dependencies" rule as the rest of the tree.
//
// Protocol channel discipline: stdout carries ONLY framed JSON-RPC; anything
// else (logging) must go to stderr, or the transport corrupts.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::Path;

use loft::diagnostics::{DiagEntry, Level};
use loft::json::{self, Parsed};

const SERVER_NAME: &str = "loft-lsp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let mut stdin = io::stdin().lock();
    let stdout = io::stdout();
    let mut shutdown_requested = false;
    // Resolve the stdlib once; re-parsing the buffer per edit reloads it, but
    // the DIRECTORY never moves during a session.
    let stdlib_dir = resolve_stdlib_dir();
    // uri -> current full text.  Sets up S4-S6 (outline/hover/definition need
    // the live buffer); S3 also parses straight from it on each edit.
    let mut documents: HashMap<String, String> = HashMap::new();

    while let Some(body) = read_message(&mut stdin) {
        // A frame that isn't valid JSON is skipped, not fatal — a robust server
        // keeps the session alive rather than dying on one bad message.
        let Ok(msg) = json::parse(&body) else {
            continue;
        };

        let method = obj_str(&msg, "method").unwrap_or_default();
        // Presence of `id` distinguishes a REQUEST (needs a reply) from a
        // NOTIFICATION (must not be replied to).
        let id = obj_get(&msg, "id").cloned();

        match (method.as_str(), id) {
            ("initialize", Some(id)) => send(&stdout, &response(id, initialize_result())),
            ("initialized", None) => {} // notification — no reply
            ("textDocument/didOpen", None) => {
                if let Some((uri, text)) = did_open_params(&msg) {
                    let diags = diagnose_text(&text, &uri, &stdlib_dir);
                    documents.insert(uri.clone(), text);
                    send(&stdout, &publish_diagnostics(&uri, diags));
                }
            }
            ("textDocument/didChange", None) => {
                if let Some((uri, text)) = did_change_params(&msg) {
                    let diags = diagnose_text(&text, &uri, &stdlib_dir);
                    documents.insert(uri.clone(), text);
                    send(&stdout, &publish_diagnostics(&uri, diags));
                }
            }
            ("textDocument/didClose", None) => {
                if let Some(uri) = did_close_uri(&msg) {
                    documents.remove(&uri);
                    // Clear the editor's squiggles for a closed file: publish an
                    // empty list (LSP has no separate "clear" message).
                    send(&stdout, &publish_diagnostics(&uri, Vec::new()));
                }
            }
            ("textDocument/documentSymbol", Some(id)) => {
                // Outline the CURRENTLY-OPEN buffer (never re-read from disk — the
                // editor's copy is the source of truth).  Unknown doc → empty list.
                let symbols = document_symbol_uri(&msg)
                    .and_then(|uri| documents.get(&uri))
                    .map(|text| document_symbols(text, &stdlib_dir))
                    .unwrap_or_default();
                send(&stdout, &response(id, Parsed::Array(symbols)));
            }
            ("textDocument/hover", Some(id)) => {
                let hover = hover_params(&msg)
                    .and_then(|(uri, line, ch)| {
                        let text = documents.get(&uri)?;
                        hover_result(text, &stdlib_dir, line, ch)
                    })
                    .unwrap_or(Parsed::Null); // no symbol under the cursor → null
                send(&stdout, &response(id, hover));
            }
            ("shutdown", Some(id)) => {
                shutdown_requested = true;
                send(&stdout, &response(id, Parsed::Null));
            }
            ("exit", _) => {
                // LSP: exit 0 iff `shutdown` came first, else 1.
                std::process::exit(i32::from(!shutdown_requested));
            }
            (_, Some(id)) => {
                // Unknown REQUEST → JSON-RPC MethodNotFound; a request must always
                // get exactly one reply, even if we don't handle it yet.
                send(&stdout, &error_response(id, -32601, "method not found"));
            }
            (_, None) => {} // unknown notification — ignore
        }
    }
    // stdin closed without `exit` — treat as a clean end of session.
}

// ── diagnostics (S3) ─────────────────────────────────────────────────────────
/// Parse `text` and map its Warning+ diagnostics to LSP `Diagnostic` objects.
/// `uri` is only a label for the parser's internal filename; the wire position
/// comes from the buffer, and the URI on the notification is set by the caller.
fn diagnose_text(text: &str, uri: &str, stdlib_dir: &str) -> Vec<Parsed> {
    let diags = loft::lsp::diagnose(text, uri, stdlib_dir);
    diags
        .entries()
        .iter()
        .filter(|e| e.level >= Level::Warning)
        .map(|e| lsp_diagnostic(e, text))
        .collect()
}

/// One loft `DiagEntry` -> one LSP `Diagnostic`.  loft positions are 1-based
/// (line 1, col 1 = first char); LSP positions are 0-based, so both drop by one.
/// loft records a single point, so the range underlines the identifier at that
/// point (its extent read from `text`) — a visible squiggle under the token,
/// not a zero-width caret.
fn lsp_diagnostic(e: &DiagEntry, text: &str) -> Parsed {
    let line0 = e.line.saturating_sub(1);
    let col0 = e.col.saturating_sub(1);
    let end_col = col0 + token_len_at(text, line0, col0);
    let range = range_of(line0, col0, end_col);
    let mut fields = vec![
        ("range", range),
        ("severity", Parsed::Int(lsp_severity(e.level))),
        ("source", Parsed::Str("loft".into())),
        ("message", Parsed::Str(e.message.clone())),
    ];
    if let Some(code) = e.code {
        fields.push(("code", Parsed::Str(code.into())));
    }
    obj(fields)
}

/// LSP DiagnosticSeverity: Error 1, Warning 2, Information 3, Hint 4.
fn lsp_severity(level: Level) -> i64 {
    match level {
        Level::Fatal | Level::Error => 1,
        Level::Warning => 2,
        Level::Debug => 3,
    }
}

/// The character length of the identifier starting at (`line0`, `col0`) in
/// `text` — so the squiggle underlines the whole token.  Non-identifier or
/// past-end positions get length 1 (a single-character caret, never zero-width).
fn token_len_at(text: &str, line0: u32, col0: u32) -> u32 {
    let Some(line) = text.lines().nth(line0 as usize) else {
        return 1;
    };
    let chars: Vec<char> = line.chars().collect();
    let start = col0 as usize;
    if start >= chars.len() {
        return 1;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    if !is_word(chars[start]) {
        return 1;
    }
    let len = chars[start..].iter().take_while(|&&c| is_word(c)).count();
    len.max(1) as u32
}

fn position(line: u32, character: u32) -> Parsed {
    obj(vec![
        ("line", Parsed::Int(i64::from(line))),
        ("character", Parsed::Int(i64::from(character))),
    ])
}

/// An LSP `Range` on one line from `start_char` to `end_char` (all 0-based).
fn range_of(line: u32, start_char: u32, end_char: u32) -> Parsed {
    obj(vec![
        ("start", position(line, start_char)),
        ("end", position(line, end_char)),
    ])
}

fn publish_diagnostics(uri: &str, diagnostics: Vec<Parsed>) -> Parsed {
    let params = obj(vec![
        ("uri", Parsed::Str(uri.into())),
        ("diagnostics", Parsed::Array(diagnostics)),
    ]);
    notification("textDocument/publishDiagnostics", params)
}

// ── outline / document symbols (S4) ──────────────────────────────────────────
/// Outline the buffer `text`: map each top-level `loft::lsp::Symbol` to an LSP
/// `DocumentSymbol`.  The parser labels the source with a fixed name (it only
/// tags the internal filename; positions come from the buffer itself).
fn document_symbols(text: &str, stdlib_dir: &str) -> Vec<Parsed> {
    loft::lsp::outline(text, "buf.loft", stdlib_dir)
        .iter()
        .map(|s| lsp_document_symbol(s, text))
        .collect()
}

/// One `Symbol` -> one LSP `DocumentSymbol`.  `range` and `selectionRange` both
/// point at the NAME on its declaration line: the parser records a def's
/// position at the body start (past the name), so — to make the Outline jump to
/// the name — the name is located in the source line (`name_range`), with the
/// recorded position as a fallback.
fn lsp_document_symbol(sym: &loft::lsp::Symbol, text: &str) -> Parsed {
    let range = name_range(text, sym);
    obj(vec![
        ("name", Parsed::Str(sym.name.clone())),
        ("kind", Parsed::Int(symbol_kind(sym.kind))),
        ("range", range.clone()),
        ("selectionRange", range),
    ])
}

/// LSP `SymbolKind` for a `classify` label.  Unknown labels fall back to
/// Function (12) rather than dropping the symbol.
fn symbol_kind(kind: &str) -> i64 {
    match kind {
        "struct" => 23,
        "enum" => 10,
        "method" => 6,
        "constant" => 14,
        "interface" => 11,
        "operator" => 25,
        "typedef" => 5, // no dedicated typedef kind; Class is the conventional fallback
        _ => 12,        // "fn" and anything unrecognized
    }
}

/// A `Range` covering the symbol's NAME on its declaration line.  The display
/// name may be a method `Type.method`; the source token is the last dotted
/// segment, so search for that.  Falls back to the parser's recorded position
/// (never zero-width) when the name isn't found on the line.
fn name_range(text: &str, sym: &loft::lsp::Symbol) -> Parsed {
    let line0 = sym.line.saturating_sub(1);
    let needle = sym.name.rsplit('.').next().unwrap_or(&sym.name);
    if let Some(line) = text.lines().nth(line0 as usize)
        && let Some(byte_idx) = line.find(needle)
    {
        let start = line[..byte_idx].chars().count() as u32;
        let end = start + needle.chars().count() as u32;
        return range_of(line0, start, end);
    }
    let col0 = sym.col.saturating_sub(1);
    range_of(line0, col0, col0 + 1)
}

fn document_symbol_uri(msg: &Parsed) -> Option<String> {
    obj_str(obj_get(obj_get(msg, "params")?, "textDocument")?, "uri")
}

// ── hover (S5) ───────────────────────────────────────────────────────────────
/// Resolve the symbol under the cursor and render it as an LSP `Hover`, or `None`
/// when nothing resolves (the caller sends `null`).  LSP positions are 0-based;
/// `symbol_at` is loft-native 1-based, so both bump by one at this boundary.
fn hover_result(text: &str, stdlib_dir: &str, line0: u32, char0: u32) -> Option<Parsed> {
    let h = loft::lsp::symbol_at(text, "buf.loft", stdlib_dir, line0 + 1, char0 + 1)?;
    let contents = markup(&hover_markdown(&h));
    Some(obj(vec![("contents", contents)]))
}

/// The hover body: the signature in a `loft` code fence, then the `///` doc.
fn hover_markdown(h: &loft::lsp::Hover) -> String {
    let mut s = format!("```loft\n{}\n```", h.signature);
    if !h.doc.is_empty() {
        s.push_str("\n\n");
        s.push_str(&h.doc.join("\n"));
    }
    s
}

/// An LSP `MarkupContent` (markdown).
fn markup(value: &str) -> Parsed {
    obj(vec![
        ("kind", Parsed::Str("markdown".into())),
        ("value", Parsed::Str(value.into())),
    ])
}

/// `params.textDocument.uri` + `params.position.{line, character}` (0-based).
fn hover_params(msg: &Parsed) -> Option<(String, u32, u32)> {
    let params = obj_get(msg, "params")?;
    let uri = obj_str(obj_get(params, "textDocument")?, "uri")?;
    let pos = obj_get(params, "position")?;
    let line = u32::try_from(obj_get(pos, "line")?.as_i64()?).ok()?;
    let ch = u32::try_from(obj_get(pos, "character")?.as_i64()?).ok()?;
    Some((uri, line, ch))
}

// ── document-lifecycle param extraction ──────────────────────────────────────
/// `didOpen`: `params.textDocument.{uri, text}`.
fn did_open_params(msg: &Parsed) -> Option<(String, String)> {
    let doc = obj_get(obj_get(msg, "params")?, "textDocument")?;
    Some((obj_str(doc, "uri")?, obj_str(doc, "text")?))
}

/// `didChange` under full-document sync: `params.textDocument.uri` +
/// `params.contentChanges[last].text` (full sync sends whole-file replacements;
/// take the last change, which carries the final text).
fn did_change_params(msg: &Parsed) -> Option<(String, String)> {
    let params = obj_get(msg, "params")?;
    let uri = obj_str(obj_get(params, "textDocument")?, "uri")?;
    let changes = match obj_get(params, "contentChanges")? {
        Parsed::Array(v) => v,
        _ => return None,
    };
    let text = obj_str(changes.last()?, "text")?;
    Some((uri, text))
}

/// `didClose`: `params.textDocument.uri`.
fn did_close_uri(msg: &Parsed) -> Option<String> {
    obj_str(obj_get(obj_get(msg, "params")?, "textDocument")?, "uri")
}

// ── stdlib resolution (deployment) ───────────────────────────────────────────
/// Locate the stdlib `default/` directory the way the `loft` CLI does: relative
/// to this binary for a release/installed layout, else the source tree.  Checked
/// most-specific first; falls back to a CWD-relative `default` (repo-root runs).
fn resolve_stdlib_dir() -> String {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let candidates = [
        exe_dir.join("../../default"), // dev: target/{release,debug}/loft-lsp -> repo/default
        exe_dir.join("../share/loft/default"), // installed: <prefix>/bin -> <prefix>/share/loft
        exe_dir.join("../default"),    // release layout with default beside the binary dir
    ];
    for c in candidates {
        if c.is_dir() {
            return c.to_string_lossy().into_owned();
        }
    }
    "default".to_string()
}

// ── framing ─────────────────────────────────────────────────────────────────
// Read one `Content-Length: N\r\n <headers> \r\n<N bytes>` message; the JSON
// body, or `None` at EOF.
fn read_message(stdin: &mut impl BufRead) -> Option<String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if stdin.read_line(&mut line).ok()? == 0 {
            return None; // EOF
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break; // blank line ends the header block
        }
        if let Some(v) = header.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
        // Content-Type and any other header is ignored.
    }
    let mut buf = vec![0u8; content_length?];
    stdin.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn send(stdout: &io::Stdout, msg: &Parsed) {
    let body = json::to_json_string(msg);
    let mut out = stdout.lock();
    // Content-Length is the BYTE length of the UTF-8 body.
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

// ── json helpers over loft::json::Parsed ─────────────────────────────────────
fn obj_get<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(entries) => entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, val)| val),
        _ => None,
    }
}

fn obj_str(v: &Parsed, key: &str) -> Option<String> {
    match obj_get(v, key) {
        Some(Parsed::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Build a JSON object from `(key, value)` pairs.  The `usize` in each entry is
/// the source byte offset `loft::json` records for diagnostics; on the emit path
/// it is unused, so `0` is fine.
fn obj(entries: Vec<(&str, Parsed)>) -> Parsed {
    Parsed::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), 0, v))
            .collect(),
    )
}

// ── message builders ─────────────────────────────────────────────────────────
fn response(id: Parsed, result: Parsed) -> Parsed {
    obj(vec![
        ("jsonrpc", Parsed::Str("2.0".into())),
        ("id", id),
        ("result", result),
    ])
}

fn error_response(id: Parsed, code: i64, message: &str) -> Parsed {
    let err = obj(vec![
        ("code", Parsed::Int(code)),
        ("message", Parsed::Str(message.into())),
    ]);
    obj(vec![
        ("jsonrpc", Parsed::Str("2.0".into())),
        ("id", id),
        ("error", err),
    ])
}

/// A JSON-RPC notification (no `id` — the server pushes it, the client never
/// replies).  `publishDiagnostics` is the S3 use.
fn notification(method: &str, params: Parsed) -> Parsed {
    obj(vec![
        ("jsonrpc", Parsed::Str("2.0".into())),
        ("method", Parsed::Str(method.into())),
        ("params", params),
    ])
}

fn initialize_result() -> Parsed {
    // `textDocumentSync` as an object: `openClose` so the client sends
    // didOpen/didClose, `change: 1` for full-document sync on each edit — the S3
    // diagnostics contract.  Later providers (hover, definition, …) add their
    // own capability flags as they land, so an editor never asks for what isn't wired.
    let sync = obj(vec![
        ("openClose", Parsed::Bool(true)),
        ("change", Parsed::Int(1)),
    ]);
    let capabilities = obj(vec![
        ("textDocumentSync", sync),
        ("documentSymbolProvider", Parsed::Bool(true)),
        ("hoverProvider", Parsed::Bool(true)),
    ]);
    let server_info = obj(vec![
        ("name", Parsed::Str(SERVER_NAME.into())),
        ("version", Parsed::Str(SERVER_VERSION.into())),
    ]);
    obj(vec![
        ("capabilities", capabilities),
        ("serverInfo", server_info),
    ])
}
