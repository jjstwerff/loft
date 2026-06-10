// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 — the connector role end-to-end: a loft client (`run_client`)
//! against a loft server (`run`), BOTH with zero transport code.
//!
//! Proves the full auto-path: the kernel negotiates the UDP cookie inside
//! the WS handshake, the connector auto-hellos and keepalives, the server's
//! `broadcast` of a `sync_class` kind reaches the client as datagrams into
//! its conflation slots (`udp=true`), events round-trip over WS, and
//! `run_client` RETURNS when the server goes away.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const PORT: u16 = 18089;

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

fn spawn_loft(prog: &PathBuf, piped: bool) -> Child {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Command::new(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(prog)
        .current_dir(&root)
        .stdout(if piped { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn loft")
}

#[test]
fn connector_auto_path_end_to_end() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let server_prog = std::env::temp_dir().join(format!("eh_conn_srv_{}.loft", std::process::id()));
    std::fs::write(
        &server_prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  w = W {{ tick: 0 }};
  engine_host::run({PORT}, 100000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 1 {{ engine_host::send(ev.cid, "7:{{ev.payload}}"); }}
    }},
    fn() {{
      w.tick = w.tick + 1;
      engine_host::broadcast("2:{{w.tick}}");
    }});
}}
"#
        ),
    )
    .unwrap();
    let client_prog = std::env::temp_dir().join(format!("eh_conn_cli_{}.loft", std::process::id()));
    std::fs::write(
        &client_prog,
        format!(
            r#"
use engine_host;
struct C {{ done: boolean not null }}
fn main() {{
  engine_host::sync_class(2);
  c = C {{ done: false }};
  engine_host::run_client("127.0.0.1", {PORT}, 100000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{
        println("client: connected");
        engine_host::client_send("hi-event");
      }} else if ev.kind == 1 {{
        println("client: event {{ev.payload}}");
      }} else if ev.kind == 2 {{
        println("client: disconnected");
      }}
    }},
    fn() {{
      while engine_host::client_sync_next() {{
        if !c.done {{
          sp = engine_host::client_sync_payload();
          ub = engine_host::client_udp_bound();
          println("client: sync {{sp}} udp={{ub}}");
          if ub {{ c.done = true; }}
        }}
      }}
    }});
  println("client: loop exited");
}}
"#
        ),
    )
    .unwrap();

    let mut server = Guard(Some(spawn_loft(&server_prog, false)));
    std::thread::sleep(Duration::from_millis(800)); // server reaches listen
    let mut client = spawn_loft(&client_prog, true);
    let stdout = client.stdout.take().expect("client stdout piped");
    let _client_guard = Guard(Some(client));

    // Stream the client's lines through a channel so asserts can deadline.
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let wait_for = |rx: &mpsc::Receiver<String>, what: &str, secs: u64| -> String {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "never saw {what:?} on client stdout");
            match rx.recv_timeout(left) {
                Ok(line) if line.contains(what) => return line,
                Ok(_) => continue,
                Err(_) => panic!("never saw {what:?} on client stdout"),
            }
        }
    };

    // 1. The connector connected and its event round-trips over WS.
    wait_for(&rx, "client: connected", 15);
    wait_for(&rx, "client: event 7:hi-event", 10);
    // 2. The fast path goes live with zero transport code on either side:
    //    a sync-class broadcast arrives via the conflation slots over UDP.
    let line = wait_for(&rx, "udp=true", 10);
    assert!(
        line.contains("client: sync 2:"),
        "the udp=true line is a sync beacon: {line:?}"
    );
    // 3. Lifecycle: kill the server; run_client must see the disconnect and
    //    RETURN (unlike the listener's forever-loop).
    if let Some(mut s) = server.0.take() {
        let _ = s.kill();
        let _ = s.wait();
    }
    wait_for(&rx, "client: disconnected", 10);
    wait_for(&rx, "client: loop exited", 10);

    let _ = std::fs::remove_file(&server_prog);
    let _ = std::fs::remove_file(&client_prog);
}

/// Priority keyframes: a sync sample promoted to must-deliver survives a
/// TOTAL datagram blackout.  The client runs with `LOFT_UDP_DROP_NTH=1`
/// (every inbound sync datagram dropped) while bound — so the 100 ms `2:`
/// beacons never arrive — yet the promoted `2:777,…` keyframe lands in the
/// sync slots via its reliable `S:`-framed WS carrier, in the same seq
/// space.  (The positive control — beacons DO arrive without the drop — is
/// `connector_auto_path_end_to_end` above.)
#[test]
fn keyframes_survive_total_datagram_loss() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = 18090u16;
    let server_prog = std::env::temp_dir().join(format!("eh_kf_srv_{}.loft", std::process::id()));
    std::fs::write(
        &server_prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  w = W {{ tick: 0 }};
  engine_host::run({port}, 100000,
    fn(ev: engine_host::Event) {{ }},
    fn() {{
      w.tick = w.tick + 1;
      engine_host::broadcast("2:{{w.tick}}");
      if w.tick - (w.tick / 10) * 10 == 0 {{
        engine_host::keyframe(0, "2:777,{{w.tick}}");
      }}
    }});
}}
"#
        ),
    )
    .unwrap();
    let client_prog = std::env::temp_dir().join(format!("eh_kf_cli_{}.loft", std::process::id()));
    std::fs::write(
        &client_prog,
        format!(
            r#"
use engine_host;
fn main() {{
  engine_host::sync_class(2);
  engine_host::run_client("127.0.0.1", {port}, 100000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{ println("kclient: connected"); }}
    }},
    fn() {{
      while engine_host::client_sync_next() {{
        sp = engine_host::client_sync_payload();
        ub = engine_host::client_udp_bound();
        println("kclient: sync {{sp}} udp={{ub}}");
      }}
    }});
}}
"#
        ),
    )
    .unwrap();

    let _server = Guard(Some(spawn_loft(&server_prog, false)));
    std::thread::sleep(Duration::from_millis(800));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut client = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&client_prog)
        .current_dir(&root)
        .env("LOFT_UDP_DROP_NTH", "1") // total inbound datagram loss
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn client");
    let stdout = client.stdout.take().expect("piped");
    let _client_guard = Guard(Some(client));

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    // Under blackout, every bound-path sync that arrives MUST be a keyframe.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "keyframe never arrived under blackout");
        let Ok(line) = rx.recv_timeout(left) else {
            panic!("keyframe never arrived under blackout");
        };
        if line.contains("udp=true") && line.contains("sync 2:") {
            assert!(
                line.contains("sync 2:777,"),
                "a non-keyframe datagram slipped through the blackout: {line:?}"
            );
            break; // the promoted sample arrived on the reliable carrier
        }
    }

    let _ = std::fs::remove_file(&server_prog);
    let _ = std::fs::remove_file(&client_prog);
}
