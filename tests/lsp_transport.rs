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
