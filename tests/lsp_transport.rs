// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 S0 — protocol test harness for `loft-lsp`.
//
// Drives the loft-lsp binary over stdio with `Content-Length`-framed JSON-RPC
// and asserts its replies, so the transport (S1) and every later feature step is
// CI-tested WITHOUT a live editor.  The harness carries its own positive control:
// `unknown_request_is_an_error_not_a_result` proves it can distinguish a FAILURE
// reply (a JSON-RPC error) from a success — so a green handshake test is
// meaningful, not vacuous.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use loft::json::{self, Parsed};

/// A live loft-lsp subprocess with framed read/write over its stdio.
struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loft-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn loft-lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session {
            child,
            stdin,
            stdout,
        }
    }

    fn send_raw(&mut self, body: &str) {
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, id: i64, method: &str, params: &str) {
        self.send_raw(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
        ));
    }

    fn notify(&mut self, method: &str, params: &str) {
        self.send_raw(&format!(
            r#"{{"jsonrpc":"2.0","method":"{method}","params":{params}}}"#
        ));
    }

    /// Read one framed reply and parse it.
    fn recv(&mut self) -> Parsed {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).unwrap();
            assert!(n > 0, "server closed stdout before replying");
            let header = line.trim_end_matches(['\r', '\n']);
            if header.is_empty() {
                break;
            }
            if let Some(v) = header.strip_prefix("Content-Length:") {
                content_length = v.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0u8; content_length];
        self.stdout.read_exact(&mut buf).unwrap();
        json::parse(&String::from_utf8(buf).unwrap()).expect("reply is valid JSON")
    }
}

fn field<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(e) => e.iter().find(|(k, _, _)| k == key).map(|(_, _, val)| val),
        _ => None,
    }
}

fn field_str(v: &Parsed, key: &str) -> Option<String> {
    match field(v, key) {
        Some(Parsed::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn field_arr<'a>(v: &'a Parsed, key: &str) -> Option<&'a Vec<Parsed>> {
    match field(v, key) {
        Some(Parsed::Array(a)) => Some(a),
        _ => None,
    }
}

/// Build a `didOpen`/`didChange`-shaped params string, with `program` correctly
/// JSON-escaped via loft's own serializer (so embedded `\n`/`"` survive).
fn open_params(uri: &str, program: &str) -> String {
    let text = json::to_json_string(&Parsed::Str(program.to_string()));
    format!(r#"{{"textDocument":{{"uri":"{uri}","languageId":"loft","version":1,"text":{text}}}}}"#)
}

fn change_params(uri: &str, program: &str) -> String {
    let text = json::to_json_string(&Parsed::Str(program.to_string()));
    format!(
        r#"{{"textDocument":{{"uri":"{uri}","version":2}},"contentChanges":[{{"text":{text}}}]}}"#
    )
}

// S3 — the diagnostics push path.  Open a buffer with an error, assert the
// server pushes `publishDiagnostics` with the fault at the right RANGE; then
// edit it clean and assert the squiggle clears (an empty diagnostics list).
#[test]
fn diagnostics_publish_on_open_then_clear_on_fix() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let _ = s.recv();
    s.notify("initialized", "{}");

    let uri = "file:///buf.loft";
    // `nope` is undefined; it sits on line 2 (1-based), col 3 — so LSP 0-based
    // range start is line 1, character 2, and the token is 4 chars wide.
    s.notify(
        "textDocument/didOpen",
        &open_params(uri, "fn main() {\n  nope(3)\n}\n"),
    );
    let note = s.recv();
    assert_eq!(
        field_str(&note, "method").as_deref(),
        Some("textDocument/publishDiagnostics"),
        "server pushes a publishDiagnostics notification on open"
    );
    let params = field(&note, "params").expect("notification carries params");
    assert_eq!(
        field_str(params, "uri").as_deref(),
        Some(uri),
        "echoes the document uri"
    );
    let diags = field_arr(params, "diagnostics").expect("diagnostics is an array");
    assert_eq!(diags.len(), 1, "exactly one error, got {diags:?}");

    let d = &diags[0];
    assert_eq!(
        field(d, "severity").and_then(Parsed::as_i64),
        Some(1),
        "an Error maps to LSP severity 1"
    );
    assert!(
        field_str(d, "message").unwrap_or_default().contains("nope"),
        "message names the offending symbol: {d:?}"
    );
    let start = field(field(d, "range").unwrap(), "start").unwrap();
    assert_eq!(
        (
            field(start, "line").and_then(Parsed::as_i64),
            field(start, "character").and_then(Parsed::as_i64)
        ),
        (Some(1), Some(2)),
        "0-based range start sits on `nope` (line 1, char 2): {d:?}"
    );
    let end = field(field(d, "range").unwrap(), "end").unwrap();
    assert_eq!(
        field(end, "character").and_then(Parsed::as_i64),
        Some(6),
        "range underlines the whole 4-char token `nope`: {d:?}"
    );

    // Edit the buffer to a valid program — the squiggle must clear.
    s.notify(
        "textDocument/didChange",
        &change_params(uri, "fn main() {\n  print(\"hi\")\n}\n"),
    );
    let cleared = s.recv();
    let params = field(&cleared, "params").expect("clear carries params");
    assert!(
        field_arr(params, "diagnostics").is_some_and(Vec::is_empty),
        "a valid buffer publishes an empty diagnostics list (clears squiggles): {cleared:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// S4 — the outline path.  Open a buffer, request `documentSymbol`, assert the
// server replies with the top-level defs (kind + name in source order) and a
// selectionRange that lands on the NAME.
#[test]
fn document_symbol_lists_the_outline() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        matches!(
            field(caps, "documentSymbolProvider"),
            Some(Parsed::Bool(true))
        ),
        "advertises documentSymbolProvider: {init:?}"
    );
    s.notify("initialized", "{}");

    let uri = "file:///o.loft";
    let prog = "struct Point {\n  x: integer,\n}\nfn main() {\n  print(\"hi\")\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv(); // consume the publishDiagnostics push from didOpen

    s.request(
        2,
        "textDocument/documentSymbol",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}}}}"#),
    );
    let reply = s.recv();
    assert_eq!(field(&reply, "id").and_then(Parsed::as_i64), Some(2));
    let syms = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("documentSymbol result is an array, got {other:?}"),
    };
    let got: Vec<(i64, String)> = syms
        .iter()
        .map(|x| {
            (
                field(x, "kind").and_then(Parsed::as_i64).unwrap(),
                field_str(x, "name").unwrap(),
            )
        })
        .collect();
    // SymbolKind: Struct 23, Function 12 — in source order.
    assert_eq!(
        got,
        vec![(23, "Point".to_string()), (12, "main".to_string())],
        "outline lists `struct Point` then `fn main`: {got:?}"
    );

    // The struct's selectionRange underlines the NAME `Point` (line 0, chars 7..12),
    // not the parser's body-start position.
    let sel = field(&syms[0], "selectionRange").unwrap();
    let start = field(sel, "start").unwrap();
    let end = field(sel, "end").unwrap();
    assert_eq!(
        (
            field(start, "line").and_then(Parsed::as_i64),
            field(start, "character").and_then(Parsed::as_i64),
            field(end, "character").and_then(Parsed::as_i64),
        ),
        (Some(0), Some(7), Some(12)),
        "selectionRange covers `Point`: {:?}",
        &syms[0]
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

#[test]
fn initialize_handshake_and_clean_shutdown() {
    let mut s = Session::start();

    s.request(1, "initialize", "{}");
    let reply = s.recv();
    assert_eq!(
        field(&reply, "id").and_then(Parsed::as_i64),
        Some(1),
        "id must echo the request"
    );
    let result = field(&reply, "result").expect("initialize returns a result, not an error");
    assert!(
        field(result, "capabilities").is_some(),
        "advertises capabilities"
    );
    let info = field(result, "serverInfo").expect("advertises serverInfo");
    assert_eq!(field_str(info, "name").as_deref(), Some("loft-lsp"));

    s.notify("initialized", "{}"); // notification — must draw no reply

    s.request(2, "shutdown", "null");
    let sd = s.recv();
    assert_eq!(field(&sd, "id").and_then(Parsed::as_i64), Some(2));
    assert!(
        matches!(field(&sd, "result"), Some(Parsed::Null)),
        "shutdown result is null"
    );

    s.notify("exit", "null");
    let status = s.child.wait().expect("wait for loft-lsp");
    assert_eq!(status.code(), Some(0), "exit after shutdown must be code 0");
}

// POSITIVE CONTROL — the harness must be able to SEE a failure path, or a green
// handshake proves nothing.  An unhandled request must come back as a JSON-RPC
// error (never a bogus success), and the harness must read it as such.
#[test]
fn unknown_request_is_an_error_not_a_result() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let _ = s.recv();

    s.request(7, "textDocument/definition", "{}"); // not wired until S6
    let reply = s.recv();
    assert_eq!(field(&reply, "id").and_then(Parsed::as_i64), Some(7));
    assert!(
        field(&reply, "result").is_none(),
        "an unhandled request must NOT return a result"
    );
    let err = field(&reply, "error").expect("an unhandled request returns an error object");
    assert_eq!(
        field(err, "code").and_then(Parsed::as_i64),
        Some(-32601),
        "MethodNotFound"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}
