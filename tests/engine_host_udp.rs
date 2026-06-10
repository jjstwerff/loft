// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 phase 05a — the state-sync UDP channel end-to-end:
//!
//! 1. the kernel issues a cookie; the loft side transports it over WS
//!    (cookie issuance = mechanics, transport = meaning);
//! 2. before the hello, `sync_send` falls back to WS frames;
//! 3. `H:<cookie>` binds the source addr (acked `A:<cid>`), after which the
//!    SAME `sync_send` call rides UDP;
//! 4. inbound datagrams conflate to newest per sender — a burst inside one
//!    tick yields exactly the newest payload, and a stale seq is discarded;
//! 5. a silent path times out and sends revert to WS transparently.

use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const PORT: u16 = 18088;
/// Long ticks make the conflation window deterministic.
const TICK_US: u32 = 200_000;

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
        assert!(Instant::now() < deadline, "kernel never listened on {port}");
        std::thread::sleep(Duration::from_millis(100));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let req = "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
               Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
               Sec-WebSocket-Version: 13\r\n\r\n";
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

fn ws_recv(stream: &TcpStream) -> String {
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).unwrap();
    let len = match hdr[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).unwrap();
            u16::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).unwrap();
    String::from_utf8(payload).unwrap()
}

/// Receive one datagram as text, or `None` on timeout.
fn udp_recv(sock: &UdpSocket) -> Option<String> {
    let mut buf = [0u8; 2048];
    match sock.recv(&mut buf) {
        Ok(n) => Some(String::from_utf8_lossy(&buf[..n]).into_owned()),
        Err(_) => None,
    }
}

/// Strip the kernel's outbound `S:<seq>:` stamp; panics on a malformed frame.
fn unstamp(dgram: &str) -> (i64, String) {
    let rest = dgram.strip_prefix("S:").expect("S: stamp");
    let (seq, payload) = rest.split_once(':').expect("seq separator");
    (seq.parse().expect("numeric seq"), payload.to_string())
}

#[test]
fn udp_sync_channel_end_to_end() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let prog = std::env::temp_dir().join(format!("eh_udp_{}.loft", std::process::id()));
    std::fs::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  w = W {{ tick: 0 }};
  engine_host::run({PORT}, {TICK_US},
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{
        ck = engine_host::udp_cookie(ev.cid);
        engine_host::send(ev.cid, "7:{{ck}}");
      }}
    }},
    fn() {{
      w.tick = w.tick + 1;
      while engine_host::sync_next() {{
        sc = engine_host::sync_cid();
        sp = engine_host::sync_payload();
        engine_host::sync_send(sc, "echo:{{sp}}");
      }}
      engine_host::sync_send(0, "beacon:{{w.tick}}");
    }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let child = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));

    // 1. Connect; the first WS frame carries the cookie (meaning-transported).
    let ws = ws_connect(PORT);
    let cookie_frame = ws_recv(&ws);
    let cookie = cookie_frame
        .strip_prefix("7:")
        .expect("cookie frame")
        .to_string();
    assert_eq!(cookie.len(), 16, "16-hex-char cookie: {cookie:?}");

    // 2. Pre-hello: sync_send falls back to WS — beacons arrive as plain
    //    WS text frames (no S: stamp).
    let b = ws_recv(&ws);
    assert!(b.starts_with("beacon:"), "WS fallback beacon, got {b:?}");

    // 3. Hello: bind the UDP path; the kernel acks.  Datagrams can drop even
    //    on loopback under load — retry the hello until the ack.
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.connect(("127.0.0.1", PORT)).unwrap();
    udp.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut first_udp_beacon = None;
    loop {
        udp.send(format!("H:{cookie}").as_bytes()).unwrap();
        match udp_recv(&udp) {
            Some(d) if d == "A:0" => break,
            Some(d) if d.starts_with("S:") => {
                // Already bound on an earlier hello; the ack raced a beacon.
                first_udp_beacon = Some(d);
                break;
            }
            _ => assert!(Instant::now() < deadline, "hello never acked"),
        }
    }

    // 4. Post-hello: the SAME sync_send now rides UDP — beacons arrive as
    //    seq-stamped datagrams.
    let dgram = match first_udp_beacon {
        Some(d) => d,
        None => loop {
            match udp_recv(&udp) {
                Some(d) => break d,
                None => assert!(Instant::now() < deadline, "no UDP beacon"),
            }
        },
    };
    let (seq, payload) = unstamp(&dgram);
    assert!(seq >= 1, "outbound seq stamped: {dgram:?}");
    assert!(payload.starts_with("beacon:"), "UDP beacon: {dgram:?}");

    // 5. Conflation: a same-tick burst (10, then 12, then stale 11) must
    //    yield exactly ONE echo — the newest (12).  Sync right after a beacon
    //    so the burst lands inside one 200 ms tick window.
    let _ = udp_recv(&udp); // align roughly to a tick boundary
    udp.send(b"S:10:posA").unwrap();
    udp.send(b"S:12:posC").unwrap();
    udp.send(b"S:11:posB").unwrap();
    let mut echoes = Vec::new();
    let until = Instant::now() + Duration::from_millis(600); // ~3 ticks
    while Instant::now() < until {
        if let Some(d) = udp_recv(&udp) {
            let (_, p) = unstamp(&d);
            if let Some(e) = p.strip_prefix("echo:") {
                echoes.push(e.to_string());
            }
        }
    }
    assert_eq!(
        echoes,
        vec!["posC".to_string()],
        "conflate-to-newest: one echo, the highest seq"
    );

    // 6. Stale discard: seq 5 < 12 is never applied — no echo follows.
    udp.send(b"S:5:staleZ").unwrap();
    let until = Instant::now() + Duration::from_millis(600);
    while Instant::now() < until {
        if let Some(d) = udp_recv(&udp) {
            let (_, p) = unstamp(&d);
            assert!(
                !p.contains("staleZ"),
                "stale seq must be discarded, got {p:?}"
            );
        }
    }

    // 7. Timeout: stop sending datagrams; after UDP_TIMEOUT_US (3 s) the path
    //    unbinds and beacons revert to WS.  Skip the WS frames queued from
    //    step 2's era by requiring a beacon FRESHER than the last UDP one.
    let mut last_udp_tick = 0i64;
    let until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < until {
        if let Some(d) = udp_recv(&udp) {
            let (_, p) = unstamp(&d);
            if let Some(t) = p.strip_prefix("beacon:") {
                last_udp_tick = t.parse().unwrap_or(0);
            }
        }
    }
    ws.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let f = ws_recv(&ws);
        if let Some(t) = f.strip_prefix("beacon:")
            && t.parse::<i64>().unwrap_or(0) > last_udp_tick
        {
            break; // fresh beacon on WS: the fallback re-engaged
        }
        assert!(Instant::now() < deadline, "beacons never reverted to WS");
    }

    let _ = std::fs::remove_file(&prog);
}
