// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN16 M5e slice 1 — drive the `--serve` browser server end-to-end over a real socket:
//! HTTP shell, then the WebSocket handshake + the debug protocol (launch → run → output →
//! terminated).  This is the browser's path, exercised without a browser.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn tmp_program(tag: &str, src: &str) -> std::path::PathBuf {
    // Tag-keyed: the two tests run in parallel in one process, so a pid-only name would
    // collide and one's cleanup would delete the other's file mid-`launch`.
    let p = std::env::temp_dir().join(format!("loft_serve_{tag}_{}.loft", std::process::id()));
    std::fs::write(&p, src).expect("write temp program");
    p
}

/// Spawn the server on a test port in a background thread (it loops forever; the thread
/// leaks, which is fine — nextest runs one process per test).  Returns once the port is
/// accepting.
fn start_server(port: u16, file: String) {
    std::thread::spawn(move || {
        let _ = loft::serve::run_serve("default", port, &file);
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start listening on {port}");
}

/// Open a connection and perform the WebSocket upgrade handshake; returns the live stream.
fn ws_connect(port: u16) -> BufReader<TcpStream> {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let req = "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
               Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
               Sec-WebSocket-Version: 13\r\n\r\n";
    (&stream).write_all(req.as_bytes()).unwrap();
    let mut reader = BufReader::new(stream);
    // Read the response head; the server replies 101 with the computed accept key.
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    assert!(
        status.contains("101"),
        "expected 101 switching protocols, got: {status}"
    );
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line.trim_end().is_empty() {
            break;
        }
    }
    reader
}

/// Send a client text frame (masked, as the spec requires for client → server).
fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8]; // FIN + text
    assert!(bytes.len() <= 125, "test payloads are small");
    frame.push(0x80 | bytes.len() as u8); // masked
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

/// Read one server text frame (unmasked), returning its payload as a string.
fn ws_recv(reader: &mut BufReader<TcpStream>) -> String {
    let mut head = [0u8; 2];
    reader.read_exact(&mut head).unwrap();
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            reader.read_exact(&mut b).unwrap();
            u16::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).unwrap();
    String::from_utf8(payload).unwrap()
}

/// Drain frames until one satisfies `pred` (or a bounded count), collecting them.
fn recv_until(
    reader: &mut BufReader<TcpStream>,
    max: usize,
    pred: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut msgs = Vec::new();
    for _ in 0..max {
        let m = ws_recv(reader);
        let stop = pred(&m);
        msgs.push(m);
        if stop {
            break;
        }
    }
    msgs
}

#[test]
fn serve_http_shell_embeds_file_and_run_button() {
    let path = tmp_program("shell", "fn main() { print(\"x\") }\n");
    let port = 18781;
    start_server(port, path.to_string_lossy().into_owned());
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    (&stream)
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut body = String::new();
    let mut s = stream;
    let _ = s.read_to_string(&mut body);
    assert!(body.contains("id=\"run\""), "shell has a Run button");
    assert!(body.contains("ws://"), "shell opens a WebSocket");
    assert!(
        body.contains(&path.to_string_lossy().into_owned()),
        "shell embeds the file path for launch"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_launch_run_streams_output() {
    let path = tmp_program(
        "run",
        "fn greet(n: integer) -> integer { n * 2 }\n\
         fn main() {\n  a = greet(21);\n  print(\"hi a={a}\")\n}\n",
    );
    let file = path.to_string_lossy().into_owned();
    let port = 18782;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);

    // launch the file → {ok:true}
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
    );
    let launch = ws_recv(&mut ws);
    assert!(
        launch.contains("\"id\":1,\"ok\":true"),
        "launch ok: {launch}"
    );

    // run → the program's print streams as an `output` event, then `terminated`
    ws_send(ws.get_ref(), "{\"id\":2,\"req\":\"run\"}");
    let msgs = recv_until(&mut ws, 8, |m| m.contains("\"event\":\"terminated\""));
    let all = msgs.join("\n");
    assert!(
        all.contains("\"category\":\"stdout\",\"text\":\"hi a=42\""),
        "program output streamed over the websocket: {all}"
    );
    assert!(
        all.contains("\"event\":\"terminated\""),
        "run terminates: {all}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_compile_streams_diagnostics() {
    // @PLN16 M5e slice 2 — compile over the websocket → a structured `diagnostics` event.
    let path = tmp_program("diag", "fn main() {\n  X = 5;\n  print(\"{X}\")\n}\n");
    let file = path.to_string_lossy().into_owned();
    let port = 18783;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"compile\",\"file\":\"{file}\"}}"),
    );
    let msgs = recv_until(&mut ws, 4, |m| m.contains("\"event\":\"diagnostics\""));
    let all = msgs.join("\n");
    assert!(
        all.contains("\"event\":\"diagnostics\"") && all.contains("\"level\":\"warning\""),
        "a warning diagnostic streamed over the websocket: {all}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_repl_eval_top_level() {
    // @PLN16 M5e — the REPL panel's context-aware eval over the websocket: a top-level
    // expression prints its value, a definition persists + is callable, an error returns
    // a <repl> diagnostics event.
    let path = tmp_program("repl", "fn main() { print(\"x\") }\n");
    let file = path.to_string_lossy().into_owned();
    let port = 18784;
    start_server(port, file);
    let mut ws = ws_connect(port);
    // an expression → value printed
    ws_send(
        ws.get_ref(),
        "{\"id\":1,\"req\":\"replEval\",\"input\":\"2 + 3\"}",
    );
    let r1 = ws_recv(&mut ws);
    assert!(
        r1.contains("\"context\":\"top\"") && r1.contains("\"output\":\"5\\n\""),
        "2+3 → 5: {r1}"
    );
    // define then call
    ws_send(
        ws.get_ref(),
        "{\"id\":2,\"req\":\"replEval\",\"input\":\"fn dbl(n: integer) -> integer { n * 2 }\"}",
    );
    let _ = ws_recv(&mut ws);
    ws_send(
        ws.get_ref(),
        "{\"id\":3,\"req\":\"replEval\",\"input\":\"dbl(21)\"}",
    );
    let r3 = ws_recv(&mut ws);
    assert!(r3.contains("\"output\":\"42\\n\""), "dbl(21) → 42: {r3}");
    // incomplete → more
    ws_send(
        ws.get_ref(),
        "{\"id\":4,\"req\":\"replEval\",\"input\":\"fn open() -> integer {\"}",
    );
    let r4 = ws_recv(&mut ws);
    assert!(r4.contains("\"more\":true"), "incomplete → more: {r4}");
    // error → a <repl> diagnostics event
    ws_send(
        ws.get_ref(),
        "{\"id\":5,\"req\":\"replEval\",\"input\":\"nope + 1\"}",
    );
    let msgs = recv_until(&mut ws, 3, |m| m.contains("diagnostics"));
    assert!(
        msgs.join("\n").contains("\"file\":\"<repl>\"")
            && msgs.join("\n").contains("\"level\":\"error\""),
        "error → <repl> diagnostics: {:?}",
        msgs
    );
    let _ = std::fs::remove_file(&path);
}
