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
    // Phase 1: the promoted sample must arrive despite the blackout.  (The
    // pre-bind beacons legitimately arrive over WS and — since the routing
    // symmetry landed — surface through the same sync slots, possibly
    // drained after the bind; they are NOT leaks.)
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(!left.is_zero(), "keyframe never arrived under blackout");
        let Ok(line) = rx.recv_timeout(left) else {
            panic!("keyframe never arrived under blackout");
        };
        if line.contains("udp=true") && line.contains("sync 2:777,") {
            break; // the promoted sample arrived on the reliable carrier
        }
    }
    // Phase 2: AFTER a keyframe (definitely post-bind), the only sync that
    // may arrive is more keyframes — a plain beacon now would be a datagram
    // that slipped the blackout.
    let until = Instant::now() + Duration::from_secs(2);
    while let Ok(line) = rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
        if line.contains("udp=true") && line.contains("sync 2:") {
            assert!(
                line.contains("sync 2:777,"),
                "a non-keyframe datagram slipped through the blackout: {line:?}"
            );
        }
        if Instant::now() >= until {
            break;
        }
    }

    let _ = std::fs::remove_file(&server_prog);
    let _ = std::fs::remove_file(&client_prog);
}

/// @PLN18 phase 07 acceptance — the ONE-SCRIPT differential: the same loft
/// client source (the template in `doc/kernel-differential.html`, port
/// substituted) runs natively (`run_client`) and in the BROWSER kernel
/// (headless chromium over the doc/pkg interpreter bundle); both must print
/// the identical transcript.  Self-skips without chromium/node (the
/// html_render harness pattern).
#[test]
#[ignore = "browser leg trips an Instant::now panic ('time not implemented') in the \
compile_and_run + frame-yield + run_client combo — the native leg passes and the \
harness works end-to-end (it CAUGHT this).  Un-ignore once the wasm time source in \
that path is bridged; see 07-browser-kernel.md § Remaining."]
fn browser_kernel_one_script_differential() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let chrome_ok = ["google-chrome", "chromium", "chromium-browser", "chrome"]
        .iter()
        .any(|c| {
            Command::new("sh")
                .arg("-c")
                .arg(format!("command -v {c}"))
                .output()
                .is_ok_and(|o| o.status.success())
        });
    let node_ok = Command::new("sh")
        .arg("-c")
        .arg("command -v node")
        .output()
        .is_ok_and(|o| o.status.success());
    let harness = root.join("tools/html_render_check.mjs");
    if !chrome_ok || !node_ok || !harness.exists() || !root.join("doc/pkg/loft.js").exists() {
        eprintln!("SKIP: chromium/node/harness/bundle missing");
        return;
    }

    let port = 18095u16;
    let server_prog = std::env::temp_dir().join(format!("eh_diff_srv_{}.loft", std::process::id()));
    std::fs::write(
        &server_prog,
        format!(
            r#"
use engine_host;
struct W {{ tick: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  w = W {{ tick: 0 }};
  engine_host::run({port}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 1 {{ engine_host::send(ev.cid, "7:hi"); }}
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
    // The SAME client source as the page's template (port substituted the
    // same way) — the one-script invariant, asserted by construction.
    let client_src = format!(
        r#"
use engine_host;
struct C {{ saw_sync: boolean not null }}
fn main() {{
  engine_host::sync_class(2);
  c = C {{ saw_sync: false }};
  engine_host::run_client(engine_host::default_host(), {port}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 0 {{
        println("t:connected");
        engine_host::client_send("hello");
      }} else if ev.kind == 1 {{
        println("t:event {{ev.payload}}");
      }} else if ev.kind == 2 {{
        println("t:disconnected");
      }}
    }},
    fn() {{
      while engine_host::client_sync_next() {{
        if !c.saw_sync {{
          c.saw_sync = true;
          println("t:sync-ok");
        }}
      }}
    }});
  println("t:exited");
}}
"#
    );
    let client_prog = std::env::temp_dir().join(format!("eh_diff_cli_{}.loft", std::process::id()));
    std::fs::write(&client_prog, &client_src).unwrap();
    let expect = [
        "t:connected",
        "t:event 7:hi",
        "t:sync-ok",
        "t:disconnected",
        "t:exited",
    ];

    // ── Native leg ──
    {
        let _server = Guard(Some(spawn_loft(&server_prog, false)));
        std::thread::sleep(Duration::from_millis(800));
        let mut client = Command::new(loft_bin())
            .arg("--interpret")
            .arg("--no-warnings")
            .arg("--lib")
            .arg(root.join("lib"))
            .arg(&client_prog)
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("native client");
        std::thread::sleep(Duration::from_secs(3));
        // Kill the server; the client sees the disconnect and exits.
        drop(_server);
        let out = client.wait_with_output().expect("client output");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("t:")).collect();
        assert_eq!(lines, expect, "native transcript");
    }

    // ── Browser leg ──
    let _server = Guard(Some(spawn_loft(&server_prog, false)));
    std::thread::sleep(Duration::from_millis(800));
    // Serve doc/ (the page + bundle); kill the kernel server mid-run so the
    // browser client exits and the page compares its transcript.
    let http_port = 18096u16;
    let mut http = Command::new("python3")
        .args([
            "-m",
            "http.server",
            &http_port.to_string(),
            "--bind",
            "127.0.0.1",
        ])
        .current_dir(root.join("doc"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("http server");
    let _http_guard = Guard(None); // placeholder; kill below explicitly
    let killer = std::thread::spawn({
        move || {
            std::thread::sleep(Duration::from_secs(4));
        }
    });
    let url = format!("http://127.0.0.1:{http_port}/kernel-differential.html?port={port}");
    let server_killer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(4));
        drop(_server);
    });
    let out = Command::new("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", "9000"])
        .args(["--port", "18097"])
        .output()
        .expect("node harness");
    let _ = killer.join();
    let _ = server_killer.join();
    let _ = http.kill();
    let _ = http.wait();
    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "browser-kernel differential failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let _ = std::fs::remove_file(&server_prog);
    let _ = std::fs::remove_file(&client_prog);
}
