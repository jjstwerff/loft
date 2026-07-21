// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// loft-lsp — the loft Language Server (LSP over JSON-RPC / stdio).
//
// @PLN63 S0/S1 — TRANSPORT SKELETON ONLY.  No compiler integration yet: this
// handles the LSP message framing + the `initialize` / `shutdown` / `exit`
// lifecycle so an editor can connect, hand-shake, and disconnect cleanly.  The
// feature providers (diagnostics S2/S3, outline/hover/definition S4-S6) hang off
// this same read-dispatch loop as their steps land.
//
// The JSON on the wire is loft's OWN parser/serializer (`loft::json`), not an
// external crate — the same "own your dependencies" rule as the rest of the tree.
//
// Protocol channel discipline: stdout carries ONLY framed JSON-RPC; anything
// else (logging) must go to stderr, or the transport corrupts.

use std::io::{self, BufRead, Write};

use loft::json::{self, Parsed};

const SERVER_NAME: &str = "loft-lsp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let mut stdin = io::stdin().lock();
    let stdout = io::stdout();
    let mut shutdown_requested = false;

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

fn initialize_result() -> Parsed {
    // S1 advertises only the transport contract: full-document text sync.  Each
    // feature provider (hoverProvider, diagnosticProvider, …) is added to these
    // capabilities as its step lands, so an editor never asks for what isn't wired.
    let capabilities = obj(vec![("textDocumentSync", Parsed::Int(1))]);
    let server_info = obj(vec![
        ("name", Parsed::Str(SERVER_NAME.into())),
        ("version", Parsed::Str(SERVER_VERSION.into())),
    ]);
    obj(vec![
        ("capabilities", capabilities),
        ("serverInfo", server_info),
    ])
}
