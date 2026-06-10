// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 phase 02 — tier 0 live reload, end-to-end: editing a named fn of a
//! RUNNING kernel server swaps it live (fn-ref dispatch + patched call
//! sites), with world state preserved across the swap.  The matrix:
//!
//! 1. clean edit → new behavior, counter keeps counting (no restart);
//! 2. broken edit → the OLD body keeps serving (stale meaning beats a dead
//!    loop), the loop survives;
//! 3. the fix-up edit → swaps again (v1 → v2 of the temp-def chain);
//! 4. a signature change → rejected; the last good body keeps serving.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PORT: u16 = 18093;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft")
}

struct Guard(Option<Child>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn ws_connect(port: u16) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(15);
    let stream = loop {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        assert!(Instant::now() < deadline, "server never listened");
        std::thread::sleep(Duration::from_millis(100));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
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
    assert!(String::from_utf8_lossy(&head).contains("101"));
    stream
}

fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [9u8, 8, 7, 6];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8, 0x80 | bytes.len() as u8];
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

/// Poll until the echo's prefix matches `want`; returns the counter suffix.
fn await_prefix(ws: &TcpStream, want: &str) -> i64 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        ws_send(ws, "ping");
        let r = ws_recv(ws);
        if let Some(rest) = r.strip_prefix(want) {
            return rest
                .split('#')
                .nth(1)
                .and_then(|n| n.parse().ok())
                .unwrap_or(-1);
        }
        assert!(
            Instant::now() < deadline,
            "echo never gained prefix {want:?} (last: {r:?})"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

const BODY_A: &str = "\"A:{p}#{n}\"";

fn program(body: &str, sig: &str) -> String {
    format!(
        r#"
use engine_host;

fn reply({sig}) -> text {{
  {body}
}}

struct W {{ n: integer not null }}

fn main() {{
  w = W {{ n: 0 }};
  engine_host::run({PORT}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 1 {{
        w.n = w.n + 1;
        engine_host::send(ev.cid, reply(ev.payload, w.n));
      }}
    }},
    fn() {{ }});
}}
"#
    )
}

#[test]
fn live_reload_swaps_a_running_fn() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let dir = std::env::temp_dir().join(format!("eh_reload_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prog = dir.join("srv.loft");
    let sig = "p: text, n: integer";
    std::fs::write(&prog, program(BODY_A, sig)).unwrap();

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let child = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .env("LOFT_LIVE_RELOAD", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let _guard = Guard(Some(child));

    let ws = ws_connect(PORT);

    // Baseline.
    let n0 = await_prefix(&ws, "A:ping");
    assert!(n0 >= 1);

    // 1. Clean edit → live swap, counter continuity (the world survived).
    std::fs::write(&prog, program("\"B:{p}#{n}\"", sig)).unwrap();
    let n1 = await_prefix(&ws, "B:ping");
    assert!(
        n1 > n0,
        "counter must continue across the swap ({n0} -> {n1})"
    );

    // 2. Broken edit → the old body keeps serving; the loop survives.
    std::fs::write(&prog, program("\"C:{p}#{n}\" +", sig)).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let n2 = await_prefix(&ws, "B:ping");
    assert!(
        n2 > n1,
        "still serving the last good body after a broken edit"
    );

    // 3. The fix-up → swaps to the corrected body.
    std::fs::write(&prog, program("\"C:{p}#{n}\"", sig)).unwrap();
    let n3 = await_prefix(&ws, "C:ping");
    assert!(n3 > n2);

    // 4. Signature change → rejected; the last good body keeps serving.
    std::fs::write(
        &prog,
        program("\"D:{p}#{n}\"", "p: text, n: integer, x: integer"),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let n4 = await_prefix(&ws, "C:ping");
    assert!(n4 > n3, "signature changes never half-apply");

    let _ = std::fs::remove_dir_all(&dir);
}
