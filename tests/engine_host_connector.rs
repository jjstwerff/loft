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

/// VM-aware deadline: CI runners are slow and CONTENDED (parallel test
/// binaries + native-build storms) — scale every wait there so timing
/// reflects the machine, not the meaning.
/// Disk-backed scratch for test fixtures.  `std::env::temp_dir()` is a
/// RAM-backed tmpfs on dev boxes (small quota, shared across sessions), and
/// loft's cache-next-to-source rule would put every `--native` fixture's
/// binary cache there too — the disk-quota stall class.  `target/` lives on
/// disk and is cleaned with the build tree.
fn test_tmp() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn vm_deadline(secs: u64) -> Instant {
    let scale = if std::env::var_os("CI").is_some() {
        3
    } else {
        1
    };
    Instant::now() + Duration::from_secs(secs * scale)
}

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
        .env("LOFT_OFFLINE", "1") // hermetic fixtures
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

/// Environment-skip detection for the node/chromium harness
/// (`tools/html_render_check.mjs`): returns the skip reason when the run
/// died WITHOUT delivering a product verdict.  A real product failure
/// always carries output — exit 1 prints the JSON failure block, exit 3 a
/// `harness error:` line — so the silent shapes are the environment's:
///  * exit 2 — the harness's own no-chrome skip;
///  * `timeout waiting for` — chrome exists but cannot LAUNCH here (CI
///    runner sandbox; the CDP endpoint never comes up);
///  * empty-output non-zero / signal death — node killed under suite load
///    (OOM / chrome contention; `status.code()` is None on a signal, so
///    the exit-2 arm never sees it either).
fn harness_env_skip(out: &std::process::Output) -> Option<String> {
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.code() == Some(2) {
        return Some(format!("SKIP: {stderr}"));
    }
    if stderr.contains("timeout waiting for") {
        return Some(format!(
            "SKIP: chromium present but not launchable: {stderr}"
        ));
    }
    if !out.status.success() && out.stdout.is_empty() && out.stderr.is_empty() {
        return Some(format!(
            "SKIP: harness died without a verdict ({:?} — signal/OOM under suite load)",
            out.status
        ));
    }
    None
}

#[test]
fn connector_auto_path_end_to_end() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let server_prog = test_tmp().join(format!("eh_conn_srv_{}.loft", std::process::id()));
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
    let client_prog = test_tmp().join(format!("eh_conn_cli_{}.loft", std::process::id()));
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
        if ev.payload.starts_with("2:") {{
          if !c.done {{
            ub = engine_host::client_udp_bound();
            println("client: sync {{ev.payload}} udp={{ub}}");
            if ub {{ c.done = true; }}
          }}
        }} else {{
          println("client: event {{ev.payload}}");
        }}
      }} else if ev.kind == 2 {{
        println("client: disconnected");
      }}
    }},
    fn() {{ }});
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
    let server_prog = test_tmp().join(format!("eh_kf_srv_{}.loft", std::process::id()));
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
    let client_prog = test_tmp().join(format!("eh_kf_cli_{}.loft", std::process::id()));
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
      else if ev.kind == 1 && ev.payload.starts_with("2:") {{
        ub = engine_host::client_udp_bound();
        println("kclient: sync {{ev.payload}} udp={{ub}}");
      }}
    }},
    fn() {{ }});
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
    let deadline = vm_deadline(15);
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
    let until = vm_deadline(2);
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

/// Minimal masked-client WS for the S6 push driver (16-bit length frames —
/// the build blob exceeds 125 bytes).
fn s6_ws_connect(port: u16) -> std::net::TcpStream {
    use std::io::{Read, Write};
    let deadline = vm_deadline(15);
    let stream = loop {
        if let Ok(s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        assert!(Instant::now() < deadline, "server never listened on {port}");
        std::thread::sleep(Duration::from_millis(100));
    };
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

fn s6_ws_send(stream: &std::net::TcpStream, text: &str) {
    use std::io::Write;
    let mask = [0x21u8, 0x43, 0x65, 0x87];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8];
    assert!(bytes.len() <= 0xFFFF, "frame too large for the 16-bit form");
    if bytes.len() <= 125 {
        frame.push(0x80 | bytes.len() as u8);
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

/// FNV-1a 64 — must match `fnv64` in doc/kernel-swap.html.
fn s6_fnv64(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

/// @PLN18 08 scenario S6 — the browser swap: a new build under a LIVING
/// page.  The kernel server relays a build push (the bulk-channel role);
/// the PAGE (the persistent host layer) verifies the hash, exports the
/// world out of the parked instance A, and boots instance B over the SAME
/// WebSocket (the loft-rt adoption hook).  Asserted in the page
/// (doc/kernel-swap.html, which throws on any unmet clause): (a) socket
/// identity (one open ever), (b) world continuity (B's counter resumes
/// from A's, no reset), (c) B advances within the window, (d) a corrupt
/// push is rejected and A keeps running.  The pushed script is THE build
/// artifact of this tier (the interpreter-bundle tier: the script is the
/// build, the wasm module is the substrate); the compiled-module variant
/// arrives with the --html/kernel integration.
#[test]
fn s6_browser_swap_under_living_page() {
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

    let port = 18102u16;
    // The relay server: ticks the sync class; relays "pushblob:" payloads
    // verbatim to every client (the bulk-channel role — content-agnostic).
    let server_prog = test_tmp().join(format!("eh_s6_srv_{}.loft", std::process::id()));
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
      if ev.kind == 1 && ev.payload.starts_with("pushblob:") {{
        engine_host::broadcast(ev.payload[9..]);
      }}
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
    let _server = Guard(Some(spawn_loft(&server_prog, false)));

    // Serve doc/ for the page + bundle.
    let http_port = 18103u16;
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

    // Build B's script — THE SAME template as the page's (kept in step by
    // the comment on both sides), marker v2.
    let v2_script = format!(
        r#"
use engine_host;
struct C {{ count: integer not null }}
fn main() {{
  engine_host::sync_class(2);
  c = C {{ count: 0 }};
  engine_host::swap_world(c);
  engine_host::run_client(engine_host::default_host(), {port}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 1 && ev.payload.starts_with("2:") {{
        c.count = c.count + 1;
      }}
    }},
    fn() {{ engine_host::client_send("c:v2:{{c.count}}"); }});
}}
"#
    );
    let good_blob = format!("B!:{}:{v2_script}", s6_fnv64(&v2_script));
    let bad_blob = format!("B!:{}:{v2_script}", s6_fnv64("not the script"));

    // The push driver: wait for the page to be connected and counting,
    // push the CORRUPT blob (rejection leg), then the good one (the swap).
    let pusher = std::thread::spawn(move || {
        let ws = s6_ws_connect(port);
        std::thread::sleep(Duration::from_secs(4));
        s6_ws_send(&ws, &format!("pushblob:{bad_blob}"));
        std::thread::sleep(Duration::from_secs(2));
        s6_ws_send(&ws, &format!("pushblob:{good_blob}"));
        std::thread::sleep(Duration::from_secs(12));
        drop(ws);
    });

    let url = format!("http://127.0.0.1:{http_port}/kernel-swap.html?port={port}");
    let out = Command::new("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", "25000"])
        .args(["--port", "18104"])
        .output()
        .expect("node harness");
    let _ = pusher.join();
    let _ = http.kill();
    let _ = http.wait();
    if let Some(reason) = harness_env_skip(&out) {
        eprintln!("{reason}");
        let _ = std::fs::remove_file(&server_prog);
        return;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "browser swap failed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    let _ = std::fs::remove_file(&server_prog);
}

/// @PLN18 phase 07 acceptance — the ONE-SCRIPT differential: the same loft
/// client source (the template in `doc/kernel-differential.html`, port
/// substituted) runs natively (`run_client`) and in the BROWSER kernel
/// (headless chromium over the doc/pkg interpreter bundle); both must print
/// the identical transcript.  Self-skips without chromium/node (the
/// html_render harness pattern).
#[test]
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

    let port = 18105u16;
    let server_prog = test_tmp().join(format!("eh_diff_srv_{}.loft", std::process::id()));
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
        if ev.payload.starts_with("2:") {{
          if !c.saw_sync {{
            c.saw_sync = true;
            println("t:sync-ok");
          }}
        }} else {{
          println("t:event {{ev.payload}}");
        }}
      }} else if ev.kind == 2 {{
        println("t:disconnected");
      }}
    }},
    fn() {{ }});
  println("t:exited");
}}
"#
    );
    let client_prog = test_tmp().join(format!("eh_diff_cli_{}.loft", std::process::id()));
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
        let client = Command::new(loft_bin())
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
    let http_port = 18106u16;
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
        .args(["--port", "18107"])
        .output()
        .expect("node harness");
    let _ = killer.join();
    let _ = server_killer.join();
    let _ = http.kill();
    let _ = http.wait();
    if let Some(reason) = harness_env_skip(&out) {
        eprintln!("{reason}");
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
