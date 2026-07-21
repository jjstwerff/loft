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
    // Opt this server into the stdlib startup cache: the parse accessors then
    // warm-load the precompiled stdlib `Data` bundle (~12× faster than
    // re-parsing `default/` on every edit) instead of cold-parsing each request.
    // Only default it on — an explicit override (the var set to empty = off) is
    // respected.  SAFETY: set before any thread is spawned (single-threaded
    // startup), the one point Rust 2024 requires for `set_var`.
    if std::env::var_os("LOFT_STDLIB_CACHE").is_none() {
        unsafe { std::env::set_var("LOFT_STDLIB_CACHE", "1") };
    }

    let mut stdin = io::stdin().lock();
    let stdout = io::stdout();
    let mut shutdown_requested = false;
    // Resolve the stdlib once; re-parsing the buffer per edit reloads it, but
    // the DIRECTORY never moves during a session.
    let stdlib_dir = resolve_stdlib_dir();
    // uri -> current full text.  Sets up S4-S6 (outline/hover/definition need
    // the live buffer); S3 also parses straight from it on each edit.
    let mut documents: HashMap<String, String> = HashMap::new();
    // The source formatter, compiled LAZILY on the first formatting request
    // (compiling it loads the stdlib + the formatter program — heavy — and many
    // sessions never format).  `formatter_tried` distinguishes "not yet built"
    // from "built and failed" so a failed build isn't retried every request.
    let mut formatter: Option<loft::lsp::Formatter> = None;
    let mut formatter_tried = false;
    // The workspace root (from `initialize`) and the tracker index under it —
    // `index/tags.json` + `features.json` (`make index`).  Lazy + cached; absent
    // outside the loft repo, so the tag features are simply inert there.
    let mut workspace_root: Option<String> = None;
    let mut tag_index: Option<loft::lsp::TagIndex> = None;
    let mut tag_index_tried = false;
    // The workspace reverse index (identifier occurrences across the tree),
    // built lazily on the first find-references request; open buffers are
    // overlaid at query time so unsaved edits are reflected.
    let mut workspace_index: Option<loft::lsp::WorkspaceIndex> = None;
    let mut workspace_index_tried = false;

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
            ("initialize", Some(id)) => {
                workspace_root = initialize_root(&msg);
                send(&stdout, &response(id, initialize_result()));
            }
            ("initialized", None) => {} // notification — no reply
            ("textDocument/didOpen", None) => {
                if let Some((uri, text)) = did_open_params(&msg) {
                    let mut diags = diagnose_text(&text, &uri, &stdlib_dir);
                    ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_tried);
                    diags.extend(tag_diagnostics(&text, tag_index.as_ref())); // T3
                    documents.insert(uri.clone(), text);
                    send(&stdout, &publish_diagnostics(&uri, diags));
                }
            }
            ("textDocument/didChange", None) => {
                if let Some((uri, text)) = did_change_params(&msg) {
                    let mut diags = diagnose_text(&text, &uri, &stdlib_dir);
                    ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_tried);
                    diags.extend(tag_diagnostics(&text, tag_index.as_ref())); // T3
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
                let symbols = text_document_uri(&msg)
                    .and_then(|uri| documents.get(&uri))
                    .map(|text| document_symbols(text, &stdlib_dir))
                    .unwrap_or_default();
                send(&stdout, &response(id, Parsed::Array(symbols)));
            }
            ("textDocument/codeAction", Some(id)) => {
                // Step B: turn each diagnostic that carries a structured
                // `suggestion` into a "Change to `X`" quick-fix (a WorkspaceEdit).
                let actions = code_actions(&msg);
                send(&stdout, &response(id, Parsed::Array(actions)));
            }
            ("textDocument/semanticTokens/full", Some(id)) => {
                // Step D: classify the buffer's identifier tokens, delta-encoded.
                let data = text_document_uri(&msg)
                    .and_then(|uri| documents.get(&uri))
                    .map(|text| {
                        encode_semantic_tokens(&loft::lsp::semantic_tokens(
                            text,
                            "buf.loft",
                            &stdlib_dir,
                        ))
                    })
                    .unwrap_or_default();
                send(
                    &stdout,
                    &response(id, obj(vec![("data", Parsed::Array(data))])),
                );
            }
            ("textDocument/inlayHint", Some(id)) => {
                // S7 (@PLN115): inferred-type hints at assignment-local declarations,
                // positioned via the resolution index (the substrate E was blocked on).
                let hints = text_document_uri(&msg)
                    .and_then(|uri| documents.get(&uri))
                    .map(|text| {
                        loft::lsp::inlay_hints(text, "buf.loft", &stdlib_dir)
                            .iter()
                            .map(inlay_hint_json)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                send(&stdout, &response(id, Parsed::Array(hints)));
            }
            ("textDocument/completion", Some(id)) => {
                // Step C: members after `expr.`, else in-scope names + keywords.
                let items = text_document_position(&msg)
                    .and_then(|(uri, line, ch)| {
                        let text = documents.get(&uri)?;
                        Some(
                            loft::lsp::complete(text, "buf.loft", &stdlib_dir, line + 1, ch + 1)
                                .iter()
                                .map(completion_item)
                                .collect::<Vec<_>>(),
                        )
                    })
                    .unwrap_or_default();
                send(&stdout, &response(id, Parsed::Array(items)));
            }
            ("textDocument/formatting", Some(id)) => {
                // Run the same formatter the `loft fmt` CLI uses on the open buffer;
                // reply with ONE whole-document edit (or none when already tidy).
                let edits =
                    match text_document_uri(&msg).and_then(|uri| documents.get(&uri).cloned()) {
                        Some(text) => {
                            if !formatter_tried {
                                formatter_tried = true;
                                formatter = loft::lsp::Formatter::new(&stdlib_dir);
                            }
                            match formatter.as_mut().and_then(|f| f.format(&text)) {
                                // A no-op edit is noise; only send one when it changes the text.
                                Some(formatted) if formatted != text => {
                                    vec![full_document_edit(&text, &formatted)]
                                }
                                _ => Vec::new(),
                            }
                        }
                        None => Vec::new(),
                    };
                send(&stdout, &response(id, Parsed::Array(edits)));
            }
            ("textDocument/hover", Some(id)) => {
                // Clone the buffer out so the `documents` borrow is dropped before
                // the mutable `tag_index` access below.
                let at = text_document_position(&msg)
                    .and_then(|(uri, line, ch)| Some((documents.get(&uri)?.clone(), line, ch)));
                let hover = match at {
                    Some((text, line, ch)) => {
                        // T1: a tracker tag under the cursor wins over symbol hover.
                        let tag_hit = loft::lsp::tags_in(&text)
                            .into_iter()
                            .find(|(_, l, s, e)| *l == line + 1 && *s <= ch && ch < *e);
                        let tag_hover = tag_hit.and_then(|(tag, l, s, e)| {
                            ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_tried);
                            let info = tag_index.as_ref()?.lookup(&tag)?;
                            let contents = markup(&loft::lsp::render_tag_markdown(&info));
                            Some(obj(vec![
                                ("contents", contents),
                                ("range", range_of(l - 1, s, e)),
                            ]))
                        });
                        tag_hover
                            .or_else(|| hover_result(&text, &stdlib_dir, line, ch))
                            .unwrap_or(Parsed::Null)
                    }
                    None => Parsed::Null,
                };
                send(&stdout, &response(id, hover));
            }
            ("textDocument/references", Some(id)) => {
                // Clone the buffer out first so the `documents` borrow is dropped.
                let req = text_document_position(&msg).and_then(|(uri, line, ch)| {
                    let text = documents.get(&uri)?.clone();
                    Some((uri, text, line, ch))
                });
                let locations = match req {
                    Some((uri, text, line, ch)) => {
                        let file = uri_to_path(&uri);
                        // S4 (@PLN115): an assignment-local resolves to its EXACT binding
                        // occurrences via the resolution index — a same-named field access
                        // (`p.x` vs local `x`) is excluded, which the name-scan can't do.
                        // Params / loop binders / globals return None → the F-v1 path.
                        if let Some(refs) =
                            loft::lsp::local_binding_refs(&text, &stdlib_dir, &file, line + 1, ch + 1)
                        {
                            let len = loft::lsp::identifier_at(&text, line + 1, ch + 1)
                                .map_or(0, |n| n.chars().count() as u32);
                            refs.iter().map(|r| location_of(r, len)).collect()
                        } else if let Some(refs) = loft::lsp::method_refs(
                            &text,
                            line + 1,
                            ch + 1,
                            workspace_root.as_deref().unwrap_or(""),
                            &open_overlays(&documents),
                            &stdlib_dir,
                        ) {
                            // @PLN115: a METHOD resolves to its exact receiver-typed
                            // occurrences across the workspace (`text.len`, not every
                            // `len`) — keyed on the stable mangled method name.
                            let len = loft::lsp::identifier_at(&text, line + 1, ch + 1)
                                .map_or(0, |n| n.chars().count() as u32);
                            refs.iter().map(|r| location_of(r, len)).collect()
                        } else {
                            match loft::lsp::identifier_at(&text, line + 1, ch + 1) {
                                Some(name) => {
                                    ensure_workspace_index(
                                        &workspace_root,
                                        &mut workspace_index,
                                        &mut workspace_index_tried,
                                    );
                                    // F: a LOCAL scopes to its function; a global stays workspace-wide.
                                    let scope = loft::lsp::reference_scope(
                                        &text,
                                        &name,
                                        &stdlib_dir,
                                        line + 1,
                                    );
                                    references_of(
                                        &name,
                                        workspace_index.as_ref(),
                                        &documents,
                                        &scope,
                                        &file,
                                    )
                                }
                                None => Vec::new(),
                            }
                        }
                    }
                    None => Vec::new(),
                };
                send(&stdout, &response(id, Parsed::Array(locations)));
            }
            ("textDocument/prepareRename", Some(id)) => {
                // The identifier's range + placeholder IF renamable (not stdlib),
                // else null so the editor won't offer a rename box.
                let result = text_document_position(&msg)
                    .and_then(|(uri, line, ch)| {
                        let text = documents.get(&uri)?;
                        let (name, s, e) =
                            loft::lsp::prepare_rename(text, line + 1, ch + 1, &stdlib_dir)?;
                        Some(obj(vec![
                            ("range", range_of(line, s, e)),
                            ("placeholder", Parsed::Str(name)),
                        ]))
                    })
                    .unwrap_or(Parsed::Null);
                send(&stdout, &response(id, result));
            }
            ("textDocument/rename", Some(id)) => {
                ensure_workspace_index(
                    &workspace_root,
                    &mut workspace_index,
                    &mut workspace_index_tried,
                );
                match rename_edit(&msg, &documents, workspace_index.as_ref(), &stdlib_dir) {
                    Ok(edit) => send(&stdout, &response(id, edit)),
                    // A refused/invalid rename → a JSON-RPC error the editor surfaces.
                    Err(reason) => send(&stdout, &error_response(id, -32803, &reason)),
                }
            }
            ("textDocument/documentLink", Some(id)) => {
                // T2: every tracker tag with an issue URL becomes a clickable link.
                let links = text_document_uri(&msg)
                    .and_then(|uri| documents.get(&uri).cloned())
                    .map(|text| {
                        ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_tried);
                        document_links(&text, tag_index.as_ref())
                    })
                    .unwrap_or_default();
                send(&stdout, &response(id, Parsed::Array(links)));
            }
            ("textDocument/definition", Some(id)) => {
                let location = text_document_position(&msg)
                    .and_then(|(uri, line, ch)| {
                        let text = documents.get(&uri)?;
                        // @PLN115: index-first (resolves locals / methods / fields
                        // by position), then the name-based fallback.
                        let h = loft::lsp::resolve_at(text, &stdlib_dir, line + 1, ch + 1)
                            .or_else(|| {
                                loft::lsp::symbol_at(text, "buf.loft", &stdlib_dir, line + 1, ch + 1)
                            })?;
                        Some(definition_location(&uri, &h, &stdlib_dir))
                    })
                    .unwrap_or(Parsed::Null); // unresolved → null
                send(&stdout, &response(id, location));
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
    // Round-trip the structured suggestion on the diagnostic's `data` — the
    // editor echoes it back in a `codeAction` request, so the quick-fix needs no
    // re-parse (step A→B).
    if let Some(suggestion) = &e.suggestion {
        fields.push((
            "data",
            obj(vec![("suggestion", Parsed::Str(suggestion.clone()))]),
        ));
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

/// An LSP `InlayHint` JSON object (kind 1 = Type) for a resolved hint.  The `line`
/// and `col` are 1-based; LSP positions are 0-based.
fn inlay_hint_json(h: &loft::lsp::InlayHint) -> Parsed {
    obj(vec![
        (
            "position",
            position(h.line.saturating_sub(1), h.col.saturating_sub(1)),
        ),
        ("label", Parsed::Str(h.label.clone())),
        ("kind", Parsed::Int(1)),
    ])
}

/// Delta-encode classified tokens into the LSP semantic-tokens flat int array:
/// five ints per token `[Δline, Δcol, len, tokenType, tokenModifiers]`, each
/// position relative to the previous token.  Tokens must arrive sorted.
fn encode_semantic_tokens(tokens: &[loft::lsp::SemanticToken]) -> Vec<Parsed> {
    let mut out = Vec::with_capacity(tokens.len() * 5);
    let (mut prev_line, mut prev_col) = (0u32, 0u32);
    for t in tokens {
        let line0 = t.line.saturating_sub(1);
        let col0 = t.col.saturating_sub(1);
        let delta_line = line0.saturating_sub(prev_line);
        let delta_col = if delta_line == 0 {
            col0.saturating_sub(prev_col)
        } else {
            col0
        };
        out.push(Parsed::Int(i64::from(delta_line)));
        out.push(Parsed::Int(i64::from(delta_col)));
        out.push(Parsed::Int(i64::from(t.len)));
        out.push(Parsed::Int(i64::from(t.kind)));
        out.push(Parsed::Int(0)); // no modifiers
        prev_line = line0;
        prev_col = col0;
    }
    out
}

/// One `loft::lsp::Completion` → an LSP `CompletionItem`.
fn completion_item(c: &loft::lsp::Completion) -> Parsed {
    obj(vec![
        ("label", Parsed::Str(c.label.clone())),
        ("kind", Parsed::Int(i64::from(c.kind))),
        ("detail", Parsed::Str(c.detail.clone())),
    ])
}

// ── codeAction (quick-fixes from diagnostic suggestions, step B) ─────────────
/// For each diagnostic the editor sends back that carries a `data.suggestion`
/// (round-tripped from step A), a `CodeAction` quick-fix whose `WorkspaceEdit`
/// replaces the diagnostic's range with the suggestion.  loft already COMPUTED
/// the fix ("did you mean 'X'") — this just applies it.
fn code_actions(msg: &Parsed) -> Vec<Parsed> {
    let Some(params) = obj_get(msg, "params") else {
        return Vec::new();
    };
    let Some(uri) = obj_get(params, "textDocument").and_then(|d| obj_str(d, "uri")) else {
        return Vec::new();
    };
    let diags = match obj_get(params, "context").and_then(|c| obj_get(c, "diagnostics")) {
        Some(Parsed::Array(a)) => a,
        _ => return Vec::new(),
    };
    diags
        .iter()
        .filter_map(|d| {
            let suggestion = obj_str(obj_get(d, "data")?, "suggestion")?;
            let range = obj_get(d, "range")?.clone();
            let edit = obj(vec![
                ("range", range),
                ("newText", Parsed::Str(suggestion.clone())),
            ]);
            // `{ uri: [TextEdit] }` — uri is a runtime String, so build the object directly.
            let changes = Parsed::Object(vec![(uri.clone(), 0, Parsed::Array(vec![edit]))]);
            Some(obj(vec![
                ("title", Parsed::Str(format!("Change to `{suggestion}`"))),
                ("kind", Parsed::Str("quickfix".into())),
                ("diagnostics", Parsed::Array(vec![d.clone()])),
                ("isPreferred", Parsed::Bool(true)),
                ("edit", obj(vec![("changes", changes)])),
            ]))
        })
        .collect()
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

/// `params.textDocument.uri` — shared by documentSymbol + formatting.
fn text_document_uri(msg: &Parsed) -> Option<String> {
    obj_str(obj_get(obj_get(msg, "params")?, "textDocument")?, "uri")
}

// ── formatting ───────────────────────────────────────────────────────────────
/// A single LSP `TextEdit` replacing the WHOLE document with `formatted`.  The
/// range spans from the start to the true end of `original` (past the last
/// character), so it covers the buffer regardless of a trailing newline.
fn full_document_edit(original: &str, formatted: &str) -> Parsed {
    let (end_line, end_char) = doc_end(original);
    let range = obj(vec![
        ("start", position(0, 0)),
        ("end", position(end_line, end_char)),
    ]);
    obj(vec![
        ("range", range),
        ("newText", Parsed::Str(formatted.into())),
    ])
}

/// The 0-based position just past the last character of `text` (its end).
fn doc_end(text: &str) -> (u32, u32) {
    let mut line = 0u32;
    let mut col = 0u32;
    for ch in text.chars() {
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

// ── hover (S5) ───────────────────────────────────────────────────────────────
/// Resolve the symbol under the cursor and render it as an LSP `Hover`, or `None`
/// when nothing resolves (the caller sends `null`).  LSP positions are 0-based;
/// `symbol_at` is loft-native 1-based, so both bump by one at this boundary.
fn hover_result(text: &str, stdlib_dir: &str, line0: u32, char0: u32) -> Option<Parsed> {
    // @PLN115: the resolution index resolves LOCALS / METHODS / FIELDS position-
    // precisely; fall back to the name-based lookup on a definition's own name.
    let h = loft::lsp::resolve_at(text, stdlib_dir, line0 + 1, char0 + 1)
        .or_else(|| loft::lsp::symbol_at(text, "buf.loft", stdlib_dir, line0 + 1, char0 + 1))?;
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

/// An LSP `TextDocumentPositionParams`: `params.textDocument.uri` +
/// `params.position.{line, character}` (0-based).  Shared by hover + definition.
fn text_document_position(msg: &Parsed) -> Option<(String, u32, u32)> {
    let params = obj_get(msg, "params")?;
    let uri = obj_str(obj_get(params, "textDocument")?, "uri")?;
    let pos = obj_get(params, "position")?;
    let line = u32::try_from(obj_get(pos, "line")?.as_i64()?).ok()?;
    let ch = u32::try_from(obj_get(pos, "character")?.as_i64()?).ok()?;
    Some((uri, line, ch))
}

// ── go-to-definition (S6) ────────────────────────────────────────────────────
/// An LSP `Location` for the resolved definition.  A LOCAL def lives in the open
/// document, so its uri is the request's; a stdlib / library def lives in a file
/// on disk, so its uri is a `file://` path there.  The range underlines the name.
fn definition_location(request_uri: &str, h: &loft::lsp::Hover, stdlib_dir: &str) -> Parsed {
    let uri = if h.def_file == "buf.loft" {
        request_uri.to_string()
    } else {
        stdlib_file_uri(stdlib_dir, &h.def_file)
    };
    let line0 = h.def_line.saturating_sub(1);
    let col0 = h.def_col.saturating_sub(1);
    let name_len = h.name.rsplit('.').next().unwrap_or(&h.name).chars().count() as u32;
    obj(vec![
        ("uri", Parsed::Str(uri)),
        ("range", range_of(line0, col0, col0 + name_len)),
    ])
}

/// `file://` URI for a stdlib/library source file, `pos.file` being repo-root
/// relative and the stdlib root the parent of `stdlib_dir` (`…/default`).
fn stdlib_file_uri(stdlib_dir: &str, rel_file: &str) -> String {
    let path = Path::new(stdlib_dir)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(rel_file);
    let abs = std::fs::canonicalize(&path).unwrap_or(path);
    format!("file://{}", abs.display())
}

// ── tracker-tag integration (T1 hover / T2 documentLink) ─────────────────────
/// The workspace root path from `initialize` params — `rootUri` (a `file://`
/// uri) or the legacy `rootPath`.  Used to locate `<root>/index/`.
fn initialize_root(msg: &Parsed) -> Option<String> {
    let params = obj_get(msg, "params")?;
    if let Some(uri) = obj_str(params, "rootUri") {
        return Some(uri.strip_prefix("file://").unwrap_or(&uri).to_string());
    }
    obj_str(params, "rootPath")
}

// ── find-references (workspace reverse index) ────────────────────────────────
/// Build the workspace reverse index once (from `workspace_root`), caching it.
fn ensure_workspace_index(
    root: &Option<String>,
    index: &mut Option<loft::lsp::WorkspaceIndex>,
    tried: &mut bool,
) {
    if *tried {
        return;
    }
    *tried = true;
    if let Some(r) = root {
        *index = Some(loft::lsp::WorkspaceIndex::build(r));
    }
}

/// LSP `Location[]` for every reference to `name`, with open buffers overlaid so
/// unsaved edits count, then narrowed by `scope` (a local → its function only).
/// Empty when there is no workspace index.
fn references_of(
    name: &str,
    index: Option<&loft::lsp::WorkspaceIndex>,
    documents: &HashMap<String, String>,
    scope: &loft::lsp::RefScope,
    current_file: &str,
) -> Vec<Parsed> {
    let Some(wi) = index else {
        return Vec::new();
    };
    let overlays: Vec<(String, String)> = documents
        .iter()
        .map(|(uri, text)| (uri_to_path(uri), text.clone()))
        .collect();
    let len = name.chars().count() as u32;
    let all = wi.references_overlaid(name, &overlays);
    loft::lsp::scoped_refs(all, scope, current_file)
        .iter()
        .map(|r| location_of(r, len))
        .collect()
}

/// An LSP `Location` JSON object for a reference `r`, highlighting `len` chars.
fn location_of(r: &loft::lsp::Reference, len: u32) -> Parsed {
    let line0 = r.line.saturating_sub(1);
    let col0 = r.col.saturating_sub(1);
    obj(vec![
        ("uri", Parsed::Str(format!("file://{}", r.file))),
        ("range", range_of(line0, col0, col0 + len)),
    ])
}

/// A `file://` document uri → its filesystem path.
fn uri_to_path(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

/// The open documents as `(path, content)` overlays — the unsaved editor view that
/// replaces the on-disk copy during a workspace query.
fn open_overlays(documents: &HashMap<String, String>) -> Vec<(String, String)> {
    documents
        .iter()
        .map(|(uri, text)| (uri_to_path(uri), text.clone()))
        .collect()
}

/// Plan a rename and return an LSP `WorkspaceEdit`, or an error message (the
/// server sends it as a JSON-RPC error, which the editor shows to the user).
fn rename_edit(
    msg: &Parsed,
    documents: &HashMap<String, String>,
    index: Option<&loft::lsp::WorkspaceIndex>,
    stdlib_dir: &str,
) -> Result<Parsed, String> {
    let (uri, line, ch) = text_document_position(msg).ok_or("bad rename params")?;
    let new_name = obj_get(msg, "params")
        .and_then(|p| obj_str(p, "newName"))
        .ok_or("missing newName")?;
    let text = documents.get(&uri).ok_or("document is not open")?;
    let old = loft::lsp::identifier_at(text, line + 1, ch + 1)
        .ok_or("the cursor is not on an identifier")?;
    // S4 (@PLN115): an assignment-local renames its EXACT binding occurrences (a
    // same-named field / method / global excluded) — no workspace index needed, as
    // a local lives in this buffer alone.  Params / loop binders / globals fall
    // through to the workspace name-based plan below.
    let file = uri_to_path(&uri);
    if let Some(refs) = loft::lsp::local_binding_refs(text, stdlib_dir, &file, line + 1, ch + 1) {
        if !loft::lsp::is_valid_identifier(&new_name) {
            return Err(format!("`{new_name}` is not a valid identifier"));
        }
        if new_name == old {
            return Ok(Parsed::Null);
        }
        return Ok(build_workspace_edit(
            &refs,
            old.chars().count() as u32,
            &new_name,
        ));
    }
    let wi = index.ok_or("no workspace to rename across")?;
    let overlays: Vec<(String, String)> = documents
        .iter()
        .map(|(u, t)| (uri_to_path(u), t.clone()))
        .collect();
    let refs = loft::lsp::plan_rename(&old, &new_name, wi, &overlays, stdlib_dir)?;
    // F: a LOCAL rename touches only its own function; a global stays workspace-wide.
    let scope = loft::lsp::reference_scope(text, &old, stdlib_dir, line + 1);
    let refs = loft::lsp::scoped_refs(refs, &scope, &file);
    if refs.is_empty() {
        return Err(format!("no references to `{old}` in scope"));
    }
    Ok(build_workspace_edit(
        &refs,
        old.chars().count() as u32,
        &new_name,
    ))
}

/// An LSP `WorkspaceEdit` — a `{uri: [TextEdit]}` map — replacing each reference's
/// old-name range with `new`.
fn build_workspace_edit(refs: &[loft::lsp::Reference], old_len: u32, new: &str) -> Parsed {
    let mut by_uri: Vec<(String, Vec<Parsed>)> = Vec::new();
    for r in refs {
        let uri = format!("file://{}", r.file);
        let col0 = r.col.saturating_sub(1);
        let edit = obj(vec![
            (
                "range",
                range_of(r.line.saturating_sub(1), col0, col0 + old_len),
            ),
            ("newText", Parsed::Str(new.to_string())),
        ]);
        if let Some((_, edits)) = by_uri.iter_mut().find(|(u, _)| u == &uri) {
            edits.push(edit);
        } else {
            by_uri.push((uri, vec![edit]));
        }
    }
    let changes = Parsed::Object(
        by_uri
            .into_iter()
            .map(|(u, e)| (u, 0, Parsed::Array(e)))
            .collect(),
    );
    obj(vec![("changes", changes)])
}

/// Load the tracker index once (from `<root>/index/`), caching the result.
/// `tried` distinguishes "not yet loaded" from "loaded and absent" so a missing
/// index isn't re-read every request.
fn ensure_tag_index(
    root: &Option<String>,
    index: &mut Option<loft::lsp::TagIndex>,
    tried: &mut bool,
) {
    if *tried {
        return;
    }
    *tried = true;
    if let Some(r) = root {
        *index = loft::lsp::TagIndex::load(&format!("{r}/index"));
    }
}

/// T3 — Warnings for buffer tags the scanner flagged as broken (a `@P`/`@PLAN`
/// reference to no valid issue/plan, per the index's `broken` array).  Empty when
/// the index is absent.  Folded into the diagnostics push on open/change.
fn tag_diagnostics(text: &str, index: Option<&loft::lsp::TagIndex>) -> Vec<Parsed> {
    let Some(idx) = index else {
        return Vec::new();
    };
    loft::lsp::tags_in(text)
        .into_iter()
        .filter(|(tag, ..)| idx.is_broken(tag))
        .map(|(tag, line, s, e)| {
            obj(vec![
                ("range", range_of(line - 1, s, e)),
                ("severity", Parsed::Int(2)), // Warning
                ("source", Parsed::Str("loft-tag".into())),
                (
                    "message",
                    Parsed::Str(format!(
                        "broken tracker reference {tag} — no matching issue/plan (per the index)"
                    )),
                ),
            ])
        })
        .collect()
}

/// Every tracker tag with an issue URL, as an LSP `DocumentLink`.  Empty when the
/// index is absent (outside the loft repo) — tags then carry no links.
fn document_links(text: &str, index: Option<&loft::lsp::TagIndex>) -> Vec<Parsed> {
    let Some(idx) = index else {
        return Vec::new();
    };
    loft::lsp::tags_in(text)
        .into_iter()
        .filter_map(|(tag, line, s, e)| {
            let url = idx.lookup(&tag)?.url?;
            Some(obj(vec![
                ("range", range_of(line - 1, s, e)),
                ("target", Parsed::Str(url)),
            ]))
        })
        .collect()
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
        ("definitionProvider", Parsed::Bool(true)),
        ("referencesProvider", Parsed::Bool(true)),
        ("codeActionProvider", Parsed::Bool(true)),
        ("inlayHintProvider", Parsed::Bool(true)),
        (
            "completionProvider",
            obj(vec![(
                "triggerCharacters",
                Parsed::Array(vec![Parsed::Str(".".into())]),
            )]),
        ),
        (
            "semanticTokensProvider",
            obj(vec![
                (
                    "legend",
                    obj(vec![
                        (
                            "tokenTypes",
                            Parsed::Array(
                                loft::lsp::semantic_token_types()
                                    .iter()
                                    .map(|t| Parsed::Str((*t).into()))
                                    .collect(),
                            ),
                        ),
                        ("tokenModifiers", Parsed::Array(Vec::new())),
                    ]),
                ),
                ("full", Parsed::Bool(true)),
            ]),
        ),
        (
            "renameProvider",
            obj(vec![("prepareProvider", Parsed::Bool(true))]),
        ),
        ("documentFormattingProvider", Parsed::Bool(true)),
        (
            "documentLinkProvider",
            obj(vec![("resolveProvider", Parsed::Bool(false))]),
        ),
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
