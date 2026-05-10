// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! TIC_TAC_TOE v5 — primitive integration tests.
//!
//! v5 introduces the binary-WebSocket surface and (eventually) the
//! session-tagged blob protocol that plan-36's audience-generative-art
//! demo will sit on top of.  Each `t<N>_*` test is a focused
//! validation of one primitive in the v5 layer:
//!
//!   t1 — `send_binary` + `last_opcode` round-trip a 4-byte payload
//!        through opcode 0x02 in both directions
//!   t2 — session-tagged blob header (5 bytes: type + session-u32)   (TODO)
//!   t3 — N-client routing (server tags responses; clients see only
//!        their own + broadcasts)                                     (TODO)
//!   t4 — catch-up replay cache after reconnect                       (TODO)
//!   t5 — sluggish world tick + decay timing harness                  (TODO)
//!
//! Each test pair lives under `lib/game_protocol/examples/v5_*.loft`
//! and is launched as a subprocess to dodge P245 (single-process
//! `parallel{}` + I/O hangs when one arm accepts and another connects
//! to a loopback port).  The harness pattern mirrors
//! `tests/multiplayer_v2.rs` — the helpers below are deliberately
//! the same shape so future ttt-vN suites can copy this file.

#![allow(clippy::too_many_lines)]

use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

// ── Helpers ─────────────────────────────────────────────────────────────

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn examples_dir() -> PathBuf {
    workspace_root().join("lib/game_protocol/examples")
}

/// Pick a free TCP port via a transient bind to `127.0.0.1:0`.  Same
/// idiom as `tests/multiplayer_v2.rs::pick_free_port` (P231).
fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let port = listener.local_addr().expect("local_addr after bind").port();
    drop(listener);
    port
}

/// Wait for the server child to print a marker line on its stdout.
/// Single-client v5 servers (`srv.next()` blocking accept) cannot
/// tolerate the v2-style TCP-connect probe — accepting the probe
/// socket counts as the test's "client", so the probe consumes the
/// server's only `srv.next()` call and the server exits with
/// "accept-failed" before the real client connects.  Reading stdout
/// for "listening port=" is probe-free: the server prints it
/// immediately after `tcp_listen` succeeds, before entering accept.
///
/// Spawns a background reader thread that streams stdout lines into
/// a channel.  Returns the (lines-so-far, residual stdout reader)
/// after the marker is seen, or (lines-so-far, None) on timeout.
fn await_stdout_marker(
    stdout: ChildStdout,
    marker: &str,
    timeout: Duration,
) -> (String, mpsc::Receiver<String>, bool) {
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        let br = BufReader::new(stdout);
        for line in br.lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + timeout;
    let mut accumulated = String::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(line) => {
                accumulated.push_str(&line);
                accumulated.push('\n');
                if line.contains(marker) {
                    return (accumulated, rx, true);
                }
            }
            Err(_) => continue,
        }
    }
    (accumulated, rx, false)
}

fn drain_with_timeout(mut child: Child, timeout: Duration) -> (String, Option<i32>) {
    let mut stdout_text = String::new();
    let stdout = child.stdout.take().expect("child stdout was piped");
    let reader_handle = thread::spawn(move || {
        let mut buf = String::new();
        let mut br = BufReader::new(stdout);
        let _ = br.read_to_string(&mut buf);
        buf
    });

    let deadline = Instant::now() + timeout;
    let mut exit_status: Option<i32> = None;
    while Instant::now() < deadline {
        if let Ok(Some(s)) = child.try_wait() {
            exit_status = s.code();
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    if exit_status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Ok(s) = reader_handle.join() {
        stdout_text = s;
    }
    (stdout_text, exit_status)
}

/// Auto-kills its server child on drop.  Avoids leaking a listener
/// when an assertion panics.
struct ServerGuard {
    child: Option<Child>,
    port: u16,
    /// Lines of stdout accumulated up to (and including) the
    /// "listening port=" marker — preserved so a downstream failure
    /// can include the early server output in its diagnostic.
    early_stdout: String,
    /// Receiver for the rest of the server's stdout.  Drained at the
    /// end of the test by `drain_remaining_stdout`.
    stdout_rx: Option<mpsc::Receiver<String>>,
}

impl ServerGuard {
    fn spawn(server_script: &str, port: u16) -> Self {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg(examples_dir().join(server_script))
            .env("LOFT_TICTACTOE_PORT", port.to_string())
            .current_dir(examples_dir())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd.spawn().expect("failed to spawn loft v5 server");
        ServerGuard {
            child: Some(child),
            port,
            early_stdout: String::new(),
            stdout_rx: None,
        }
    }

    /// Wait until the server prints its "listening port=" marker.
    /// Probe-free — see `await_stdout_marker` for why.  On success,
    /// stashes the early-stdout lines + retains the residual reader
    /// so the rest of the run can be drained later.
    fn wait_listening(&mut self) -> bool {
        let stdout = match self.child.as_mut().and_then(|c| c.stdout.take()) {
            Some(s) => s,
            None => return false,
        };
        let marker = format!("listening port={}", self.port);
        let (early, rx, ok) = await_stdout_marker(stdout, &marker, Duration::from_secs(60));
        self.early_stdout = early;
        self.stdout_rx = Some(rx);
        ok
    }

    /// Drain whatever the server has emitted since `wait_listening`
    /// returned, plus the early lines.  Cheap when the test passed;
    /// useful as a panic message when something failed downstream.
    fn snapshot_stdout(&mut self) -> String {
        let mut s = self.early_stdout.clone();
        if let Some(rx) = &self.stdout_rx {
            // Drain non-blocking: anything already on the channel.
            while let Ok(line) = rx.try_recv() {
                s.push_str(&line);
                s.push('\n');
            }
        }
        s
    }

    fn diagnose_listen_failure(&mut self) -> String {
        let mut out = String::new();
        if !self.early_stdout.is_empty() {
            out.push_str("\n  --- server stdout (pre-marker) ---\n");
            out.push_str(&self.early_stdout);
        }
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    out.push_str(&format!(
                        "\n  server child already exited: status={status:?}"
                    ));
                }
                Ok(None) => {
                    out.push_str("\n  server child still running but not listening");
                }
                Err(e) => {
                    out.push_str(&format!("\n  failed to query child status: {e}"));
                }
            }
            let mut stderr_buf = String::new();
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut stderr_buf);
            }
            if !stderr_buf.is_empty() {
                out.push_str("\n  --- server stderr ---\n");
                out.push_str(&stderr_buf);
            }
            if self.early_stdout.is_empty() && stderr_buf.is_empty() {
                out.push_str("\n  (server produced no stdout/stderr)");
            }
        } else {
            out.push_str("\n  (no server child captured)");
        }
        out
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn spawn_client(client_script: &str, port: u16) -> Child {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(examples_dir().join(client_script))
        .current_dir(examples_dir())
        .env("LOFT_TICTACTOE_PORT", port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    cmd.spawn().expect("failed to spawn loft v5 client")
}

/// Convenience: spawn server + wait for its "listening port=N"
/// marker, panic with a useful diagnostic if it doesn't appear.
fn spawn_listening_server(script: &str, port: u16, label: &str) -> ServerGuard {
    let mut s = ServerGuard::spawn(script, port);
    if !s.wait_listening() {
        let diag = s.diagnose_listen_failure();
        panic!("{label} server failed to start within 60s{diag}");
    }
    s
}

// ── t1: 4-byte binary frame round-trip ──────────────────────────────────

/// Validates that `send_binary` + `last_opcode` compose end-to-end
/// across `lib/server` (single-client path) and `lib/web`
/// (`WsHandler`).  The 4-byte payload is the smallest non-trivial
/// frame; the cell layout that plan-36 will use (u8 color + u8
/// height + u16 age) is exactly 4 bytes.
#[cfg_attr(
    target_os = "windows",
    ignore = "P229b: server cannot bind on Windows CI; investigate via diagnose_listen_failure"
)]
#[test]
fn v5_t1_binary_round_trip() {
    let port = pick_free_port();
    let mut server = spawn_listening_server("v5_t1_binary_server.loft", port, "v5-t1");

    let client = spawn_client("v5_t1_binary_client.loft", port);
    let (out, status) = drain_with_timeout(client, Duration::from_secs(30));

    let server_out = server.snapshot_stdout();
    assert_eq!(
        status,
        Some(0),
        "v5-t1 client did not exit cleanly; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    assert!(
        out.contains("v5-t1c: ok"),
        "v5-t1 client did not reach the success line; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    // Lock in that the BINARY opcode actually went out — opcode 1 (text)
    // would be a regression to the legacy path masquerading as a pass.
    assert!(
        out.contains("v5-t1c: recv opcode=2"),
        "v5-t1 client received non-binary opcode; client stdout=\n{out}"
    );
}

// ── t2: session-tagged binary blobs ────────────────────────────────────

/// Server sends 5 binary blobs with session id 7, then 1 with
/// session id 8.  Client groups by session id (flushing on every
/// session-change) and reports "applied session=N blobs=M" for each.
/// Validates:
///   - lib/web pack_u8 / pack_u32_le / pack_u16_le / pack_take build
///     the wire bytes correctly (NUL bytes preserved past the
///     character-null-sentinel limitation of `text` interpolation)
///   - lib/web byte_at decodes individual bytes from the inbound
///     binary text payload
///   - the new-session-id transition triggers a flush on the client
///   - no blobs are coalesced or dropped (5 + 1, not 4 + 1 or 5 + 0)
#[cfg_attr(
    target_os = "windows",
    ignore = "P229b: server cannot bind on Windows CI"
)]
#[test]
fn v5_t2_session_blob_grouping() {
    let port = pick_free_port();
    let mut server = spawn_listening_server("v5_t2_session_blobs_server.loft", port, "v5-t2");

    let client = spawn_client("v5_t2_session_blobs_client.loft", port);
    let (out, status) = drain_with_timeout(client, Duration::from_secs(30));

    let server_out = server.snapshot_stdout();
    assert_eq!(
        status,
        Some(0),
        "v5-t2 client did not exit cleanly; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    assert!(
        out.contains("v5-t2c: ok"),
        "v5-t2 client did not reach the success line; client stdout=\n{out}\n--- server stdout:\n{server_out}"
    );
    assert!(
        out.contains("v5-t2c: applied session=7 blobs=5"),
        "v5-t2 client did not flush session 7 with 5 blobs; client stdout=\n{out}"
    );
    assert!(
        out.contains("v5-t2c: applied session=8 blobs=1"),
        "v5-t2 client did not flush session 8 with 1 blob; client stdout=\n{out}"
    );
}
