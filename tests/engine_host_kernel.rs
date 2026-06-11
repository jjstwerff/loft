// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 phase 01 — the kernel end-to-end: a loft program on
//! `engine_host::run` (the Rust-mechanics loop), driven by a real WebSocket
//! client.  Proves: connect event → handler closure (captures mutating) →
//! broadcast; the drift-free tick fires; multiple round trips on one
//! connection.  State is a STRUCT world (per #314: a bare scalar captured by a
//! reader closure + a writer closure crashes; struct-held state is the correct
//! idiom and works).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft")
}

/// Kills the whole PROCESS GROUP: a `--native` run spawns the compiled
/// binary as a grandchild of the loft driver — killing only the child
/// orphans the actual server (probe-caught: a rerun connected to the
/// previous test's orphan and read its counter).
struct Guard(Option<Child>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            unsafe {
                libc::killpg(c.id() as i32, libc::SIGKILL);
            }
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
    // Read the 101 head to its blank line (unbuffered).
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.contains("101"), "upgrade accepted: {head}");
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

#[test]
fn kernel_event_broadcast_and_ticks() {
    run_kernel_scenario(18087, true);
}

/// @PLN18 08 scenario S1 — the COMPILED baseline serves the same game:
/// the identical fixture, built `--native` (the kernel natives' typed twins
/// in `codegen_runtime`), must produce the identical transcript.  The first
/// run pays the rustc build (cached by content hash afterwards).
#[test]
fn s1_native_baseline_matches_interpreted() {
    run_kernel_scenario(18094, false);
}

/// @PLN18 08 scenario S2 — the debugger pushes one fn to the interpreter,
/// LIVE, under a serving kernel.  The flip target (`bump_events(w: W) ->
/// integer`) mutates the shared world; the event counter must count
/// monotonically across compiled -> interp -> compiled (state continuity is
/// the whole claim).  The flip is driven by a CONTROL INPUT (a WS message ->
/// `live_flip`), and is observable ONLY via the `LOFT_DISPATCH_DEBUG`
/// sentinel — the meaning transcript is byte-identical to the interpreted
/// run of the same fixture (the differential leg).
#[test]
fn s2_live_flip_under_serving_kernel() {
    let native = run_s2_scenario(18095, false);
    let interp = run_s2_scenario(18096, true);
    assert_eq!(
        native.replies, interp.replies,
        "S2 differential: a tier flip must be meaning-invisible"
    );
    assert!(
        native.stderr.contains("live-dispatch: n_bump_events"),
        "the native leg must really dispatch through the interpreter:\n{}",
        native.stderr
    );
    assert!(
        !interp.stderr.contains("live-dispatch"),
        "the interpreted leg has no live dispatch"
    );
}

struct S2Run {
    replies: Vec<String>,
    stderr: String,
}

fn run_s2_scenario(port: u16, interpret: bool) -> S2Run {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return S2Run {
            replies: Vec::new(),
            stderr: String::new(),
        };
    }
    let prog = std::env::temp_dir().join(format!("eh_s2_{port}_{}.loft", std::process::id()));
    std::fs::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn bump_events(w: W) -> integer {{
  w.events = w.events + 1;
  w.events
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      if ev.payload == "flip" {{
        engine_host::live_flip("bump_events", true);
        engine_host::send(ev.cid, "flip-ack#{{n}}");
        return;
      }}
      if ev.payload == "unflip" {{
        engine_host::live_flip("bump_events", false);
        engine_host::send(ev.cid, "unflip-ack#{{n}}");
        return;
      }}
      engine_host::broadcast("got:{{ev.payload}}#{{n}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = std::env::temp_dir().join(format!("eh_s2_{port}_{}.err", std::process::id()));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    if interpret {
        cmd.arg("--interpret");
    } else {
        cmd.env("LOFT_LIVE_FLIP", "1")
            .env("LOFT_DISPATCH_DEBUG", "1");
    }
    cmd.process_group(0); // own group, so Guard can kill driver + grandchild
    let child = cmd
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .spawn()
        .expect("spawn kernel");
    let _guard = Guard(Some(child));

    let ws = ws_connect(port);
    let mut replies = Vec::new();
    // compiled -> flip -> interp -> unflip -> compiled; the counter must
    // cross every tier boundary without a gap or repeat (1..=5).
    for msg in ["a", "flip", "b", "unflip", "c"] {
        ws_send(&ws, msg);
        replies.push(ws_recv(&ws));
    }
    assert_eq!(
        replies,
        vec![
            "got:a#1",
            "flip-ack#2",
            "got:b#3",
            "unflip-ack#4",
            "got:c#5"
        ],
        "the world's counter must be continuous across tier flips ({})",
        if interpret { "interp" } else { "native+flip" }
    );
    drop(_guard); // kill now so the stderr file is complete
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
    S2Run { replies, stderr }
}

fn run_kernel_scenario(port: u16, interpret: bool) {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let prog = std::env::temp_dir().join(format!("eh_kernel_{port}_{}.loft", std::process::id()));
    std::fs::write(
        &prog,
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      w.events = w.events + 1;
      if ev.payload == "stats" {{
        engine_host::send(ev.cid, "stats:events={{w.events}},ticks_pos={{w.ticks > 0}}");
        return;
      }}
      engine_host::broadcast("got:{{ev.payload}}#{{w.events}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        ),
    )
    .unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut cmd = Command::new(loft_bin());
    if interpret {
        cmd.arg("--interpret");
    }
    cmd.process_group(0); // own group, so Guard can kill driver + grandchild
    let child = cmd
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

    let ws = ws_connect(port);
    // Two round trips: handler closure capture (events counter) must advance.
    ws_send(&ws, "hello");
    assert_eq!(ws_recv(&ws), "got:hello#1");
    ws_send(&ws, "again");
    assert_eq!(ws_recv(&ws), "got:again#2");
    // Give the 10ms tick a beat, then ask for stats: ticks must be counting.
    std::thread::sleep(Duration::from_millis(100));
    ws_send(&ws, "stats");
    let stats = ws_recv(&ws);
    assert_eq!(
        stats, "stats:events=3,ticks_pos=true",
        "captures + drift-free ticks live: {stats}"
    );
    let _ = std::fs::remove_file(&prog);
}
