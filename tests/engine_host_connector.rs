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
