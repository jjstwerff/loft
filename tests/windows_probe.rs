// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 / WINDOWS.md — FOCUSED Windows probes for the engine-host kernel
//! (run by `.github/workflows/windows-probe.yml` on the `windows-probe`
//! branch; a no-op crate everywhere else).  Each probe answers ONE question
//! the unix-gated lifecycle suite leaves open on Windows; findings graduate
//! into WINDOWS.md and, where green, into un-gating the real tests.
//!
//! Probe 1 — does the kernel SERVE on Windows?  (std-bind fallback path:
//!           listen, ws upgrade, event echo, tick.)
//! Probe 2 — same-port UDP beside the listener (the 05a auto-path's bind).
//! Probe 3 — THE OVERLAP QUESTION: can a second listener bind the same port
//!           while the first still listens (the S5 swap handover rides
//!           SO_REUSEPORT on unix; Windows SO_REUSEADDR semantics differ)?
//!           This probe REPORTS, it does not assert a wish.
#![cfg(windows)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use loft::compile;
use loft::database::Stores;
use loft::keys::DbRef;
use loft::parser::Parser;
use loft::state::State;

/// Drive the kernel IN-PROCESS (no spawned loft binary — fast on a cold
/// runner): parse a minimal server, byte-code it, run its main on a thread.
fn spawn_inprocess_kernel(port: u16) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let src = format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      w.events = w.events + 1;
      engine_host::send(ev.cid, "got:{{ev.payload}}#{{w.events}}t{{w.ticks}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        );
        let mut parser = Parser::new();
        parser.parse_dir("default", true, false).expect("stdlib");
        parser.lib_dirs = vec!["lib".to_string()];
        parser.parse_str(&src, "<win-probe>", false);
        assert!(
            parser.diagnostics.level() < loft::diagnostics::Level::Error,
            "probe server must parse clean: {:?}",
            parser.diagnostics.lines()
        );
        loft::scopes::check(&mut parser.data);
        let mut data = parser.data;
        let mut state = State::new(parser.database);
        compile::byte_code(&mut state, &mut data);
        state.execute_argv("main", &data, &[]);
    })
}

fn ws_connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(30);
    let stream = loop {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        assert!(Instant::now() < deadline, "kernel never listened on {port}");
        std::thread::sleep(Duration::from_millis(200));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let req = "GET /ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
               Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
    (&stream).write_all(req.as_bytes()).unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    assert!(
        String::from_utf8_lossy(&head).contains("101"),
        "upgrade accepted"
    );
    stream
}

fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [0x21u8, 0x43, 0x65, 0x87];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8];
    assert!(bytes.len() <= 125);
    frame.push(0x80 | bytes.len() as u8);
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

fn ws_recv(stream: &TcpStream) -> String {
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).unwrap();
    let len = (hdr[1] & 0x7F) as usize;
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).unwrap();
    String::from_utf8(payload).unwrap()
}

/// Probe 1+2: the kernel serves on Windows — listen (std fallback), upgrade,
/// event echo with world state, drift-free ticks, and the same-port UDP bind
/// (its failure is a warning by design; this asserts the WS path regardless).
#[test]
fn probe_kernel_serves_on_windows() {
    let port = 18200u16;
    let _kernel = spawn_inprocess_kernel(port);
    let ws = ws_connect(port);
    ws_send(&ws, "a");
    let r1 = ws_recv(&ws);
    assert!(r1.starts_with("got:a#1t"), "first echo: {r1}");
    std::thread::sleep(Duration::from_millis(120));
    ws_send(&ws, "b");
    let r2 = ws_recv(&ws);
    assert!(r2.starts_with("got:b#2t"), "world advanced: {r2}");
    // Ticks moved between the two echoes (the 10 ms drift-free tick).
    let t1: i64 = r1.rsplit('t').next().unwrap().parse().unwrap();
    let t2: i64 = r2.rsplit('t').next().unwrap().parse().unwrap();
    assert!(t2 > t1, "ticks advance on Windows: {t1} -> {t2}");
    println!("PROBE kernel-serves: OK (echo + world + ticks)");
}

/// Probe 3 — the overlap question, REPORTED not wished: while a listener
/// holds a port, what does a second std bind on the same port do on
/// Windows?  (unix: fails without SO_REUSEPORT; the S5 swap counts on the
/// overlap.)  The answer decides the Windows swap design: overlap-capable
/// (port the unix shape) or sequential-rebind (close-then-bind + rollback
/// rebind).
#[test]
fn probe_same_port_second_bind_semantics() {
    let first = std::net::TcpListener::bind(("0.0.0.0", 18201)).expect("first bind");
    let second = std::net::TcpListener::bind(("0.0.0.0", 18201));
    println!(
        "PROBE overlap: second std bind while first listens -> {:?}",
        second.as_ref().map(|_| "BOUND").map_err(|e| e.kind())
    );
    // Whatever the semantics, the FIRST listener must still accept.
    let probe = TcpStream::connect(("127.0.0.1", 18201));
    println!(
        "PROBE overlap: connect during the experiment -> {:?}",
        probe.as_ref().map(|_| "CONNECTED").map_err(|e| e.kind())
    );
    drop(second);
    drop(first);
}
