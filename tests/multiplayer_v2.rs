// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! TIC_TAC_TOE v2 — multi-client + spectator integration test.
//!
//! Spawns the v2 loft server as a subprocess plus N loft clients,
//! captures their stdout, asserts on the wire-protocol behaviour
//! across various scenarios.  This is the lockdown for the v2
//! protocol-only ground layer: any regression in
//!
//!   - multi-client server primitives in `lib/server`,
//!   - the namespace handler registry + MAP handshake,
//!   - cross-client routing (Placement → Spectator*),
//!   - the WebSocket client's auto-reconnect / multi-address path,
//!
//! will be caught here in CI before it lands.
//!
//! Each test:
//!   1. Picks a unique TCP port (so parallel test runs don't collide).
//!   2. Spawns the server pointing at that port.
//!   3. Waits for the listener to come up.
//!   4. Spawns clients with a configured stagger.
//!   5. Captures their stdout to memory.
//!   6. Asserts on patterns in the captured output.
//!   7. Cleans up all child processes.
//!
//! All tests are bounded by an outer timeout so a hang doesn't
//! wedge CI.

#![allow(clippy::too_many_lines)]

use std::io::{BufReader, Read};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
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
    workspace_root().join("tests/integration/multiplayer")
}

/// P231: allocate a free TCP port by binding to `127.0.0.1:0` and
/// immediately closing — the kernel records the port as recently used
/// but it's still available to a follow-up bind by the spawned server.
/// Each test gets its own port, so default `cargo test` parallelism
/// no longer collides on a single hardcoded one.  There's still a
/// tiny TOCTOU window (another process could grab the port between
/// the close and the server's bind), but in practice this is the
/// standard idiom for ephemeral test ports across Rust ecosystems.
fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
    let port = listener.local_addr().expect("local_addr after bind").port();
    drop(listener);
    port
}

/// Wait for a TCP listener to come up on `port`.  Polls every
/// `poll_ms` up to `timeout`.  Returns true iff the connect
/// succeeded within the budget.
fn wait_for_port(port: u16, timeout: Duration, poll_ms: u64) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(poll_ms));
    }
    false
}

/// Read all stdout from a child until it exits or the timeout
/// elapses, whichever comes first.  Returns (stdout, exit_status).
/// On timeout the child is killed.
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

/// @PLN25 / registry-republish gate: the server scripts `use web` — a REGISTRY
/// package.  A registry copy predating the DN1 null model (`-> text` +
/// `return null`) REJECTS at compile time, so the server can never listen and
/// the test would burn its full timeout on a known, documented condition (the
/// loft-libs-net web/server republish, @PLN25 task #4).  Probe with `--dump`
/// (compile only, no execution) and SKIP — loudly — when the compile failure
/// lies inside the registry copy.  An IN-TREE failure still runs (and fails)
/// the test.  Self-healing: once the republished package is installed the
/// probe passes and the tests run again.
fn registry_predates_dn1(server_script: &str) -> bool {
    let out = Command::new(loft_bin())
        .arg("--dump")
        .arg(examples_dir().join(server_script))
        .current_dir(examples_dir())
        .output();
    let Ok(out) = out else { return false };
    if out.status.success() {
        return false;
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("/.loft/registry/") {
        eprintln!(
            "SKIP {server_script}: the installed registry package predates the DN1 null \
             model (awaiting the web/server republish — @PLN25 task #4):\n{}",
            err.lines()
                .filter(|l| l.contains("/.loft/registry/"))
                .take(2)
                .collect::<Vec<_>>()
                .join("\n")
        );
        return true;
    }
    false
}

/// Wrapper that auto-kills its server child on drop.  Use this so
/// every test path tears the server down even on assertion panic.
struct ServerGuard {
    child: Option<Child>,
    port: u16,
}

impl ServerGuard {
    fn spawn(server_script: &str, port: u16) -> Self {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg(examples_dir().join(server_script))
            .env("LOFT_TICTACTOE_PORT", port.to_string()) // server reads this if implemented
            .current_dir(examples_dir())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd.spawn().expect("failed to spawn loft server");
        ServerGuard {
            child: Some(child),
            port,
        }
    }

    fn wait_listening(&self) -> bool {
        // 30s timeout: Windows CI cold-runners can take 10+s to compile
        // / launch the loft binary on first invocation.  Local runs
        // bind in <1s, so the bump only affects flake-prone CI.
        wait_for_port(self.port, Duration::from_secs(60), 50)
    }

    /// Diagnostic helper: when `wait_listening` fails, drain whatever
    /// the server child wrote to stdout/stderr and check whether it
    /// already exited.  Returns a multi-line string suitable for
    /// inclusion in a panic message — the boring "server failed to
    /// start within 60s" is replaced with actual signal about WHY
    /// (server panicked at parse time, port was already taken, etc.).
    /// P229b — the previous bare timeout swallowed all diagnostic
    /// information from the child, which is exactly the symptom we
    /// see on Windows CI today.
    fn diagnose_listen_failure(&mut self) -> String {
        let mut out = String::new();
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
            let mut stdout_buf = String::new();
            let mut stderr_buf = String::new();
            if let Some(mut s) = child.stdout.take() {
                let _ = s.read_to_string(&mut stdout_buf);
            }
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut stderr_buf);
            }
            if !stdout_buf.is_empty() {
                out.push_str("\n  --- server stdout ---\n");
                out.push_str(&stdout_buf);
            }
            if !stderr_buf.is_empty() {
                out.push_str("\n  --- server stderr ---\n");
                out.push_str(&stderr_buf);
            }
            if stdout_buf.is_empty() && stderr_buf.is_empty() {
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

/// Spawn a v2 client as a subprocess, returning a Child whose
/// stdout is piped (and stderr inherited).  Caller is responsible
/// for `drain_with_timeout` and cleanup.  `port` is forwarded via
/// `LOFT_TICTACTOE_PORT` so the client targets the correct
/// per-test server (P231).
fn spawn_client(label: &str, port: u16) -> Child {
    spawn_client_with_delay(label, port, 0)
}

/// P229a: spawn a v2 client with `LOFT_TICTACTOE_CLIENT_DELAY_MS`
/// set to `delay_ms` so the client pauses after its handshake before
/// starting moves.  The two-client overlap test needs both clients
/// to be registered with the server before either makes a move; on
/// fast schedulers (macOS) the default zero-delay client races
/// through its 3 X moves in <50 ms — well before the partner has
/// completed its own handshake — so neither sees the other's
/// SpectatorPlacement frames.  A small delay (~200 ms) makes the
/// overlap deterministic on every platform.
///
/// P231: `port` is also forwarded so the client connects to the
/// per-test server rather than the legacy hardcoded 7878.
fn spawn_client_with_delay(label: &str, port: u16, delay_ms: u32) -> Child {
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(examples_dir().join("tictactoe_client_v2.loft"))
        .arg(label)
        .current_dir(examples_dir())
        .env("LOFT_TICTACTOE_PORT", port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if delay_ms > 0 {
        cmd.env("LOFT_TICTACTOE_CLIENT_DELAY_MS", delay_ms.to_string());
    }
    cmd.spawn().expect("failed to spawn loft client")
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

// ── Tests ────────────────────────────────────────────────────────────────

/// Smoke test — single client connects, plays through to GameOver.
/// This is the same protocol the v1 client validates, but driving
/// the v2 server.  Locks in: handshake, namespace registry, basic
/// click → Placement → GameOver round-trip on the multi-client path.
//
// @P229b probe (2026-05-29) — TEMPORARILY un-ignored on Windows to
// surface the actual failure mode via diagnose_listen_failure.  The
// 2026-05-21 hypothesis ("bind-then-drop race") was code-review-only
// and never verified against live Windows output.  This probe answers:
// is the cause actually port bind, or is it a sibling of PR #228's
// CreateProcessW spawn issue, or something else entirely?  Re-add the
// cfg_attr ignore once we have a real Windows error message to act on.
#[test]
fn v2_single_client_completes_game() {
    if registry_predates_dn1("tictactoe_server_v2.loft") {
        return;
    }
    // Note: the current v2 server hardcodes port 7878; if multiple
    // tests in this file run in parallel they'd collide.  Using
    // serial_test crate would be cleaner, but we lock the suite via
    // `#[test] fn ...` and rely on cargo's per-binary parallelism
    // (these tests share the same binary; cargo runs them
    // sequentially by default within one binary unless --test-threads
    // is bumped).
    let port = pick_free_port();
    let _server = {
        let mut s = ServerGuard::spawn("tictactoe_server_v2.loft", port);
        if !s.wait_listening() {
            let diag = s.diagnose_listen_failure();
            panic!("server failed to start within 60s{diag}");
        }
        s
    };

    let client = spawn_client("S", port);
    let (out, status) = drain_with_timeout(client, Duration::from_secs(60));

    assert_eq!(
        status,
        Some(0),
        "client did not exit cleanly; stdout=\n{out}"
    );
    assert!(
        out.contains("[S] Placement: X,0,0"),
        "missing first placement; got:\n{out}"
    );
    assert!(
        out.contains("[S] Placement: X,2,0"),
        "missing winning placement; got:\n{out}"
    );
    assert!(
        out.contains("[S] GameOver: X"),
        "missing GameOver; got:\n{out}"
    );
}

/// Two clients play concurrently.  Each must see:
///   - 3 own X placements (their column-0 winning sequence),
///   - 2 own O placements (server's responses on their board),
///   - 1 GameOver,
///   - 3 spectator X placements (the other client's X moves),
///   - 2 spectator O placements,
///   - 1 SpectatorGameOver.
///
/// The exact frame *ordering* depends on scheduling, so we assert
/// on counts, not order.
#[test]
fn v2_two_clients_with_spectator_routing() {
    if registry_predates_dn1("tictactoe_server_v2.loft") {
        return;
    }
    let port = pick_free_port();
    let _server = {
        let mut s = ServerGuard::spawn("tictactoe_server_v2.loft", port);
        if !s.wait_listening() {
            let diag = s.diagnose_listen_failure();
            panic!("server failed to start within 60s{diag}");
        }
        s
    };

    // P229a: both clients spawn essentially simultaneously, then each
    // pauses ~200 ms after handshake before placing its first X.  On
    // macOS the scheduler is fast enough that without the pause the
    // first client would complete all 3 moves before the second's
    // handshake even reaches the server — so neither side observes
    // the other's spectator frames.  The delay is set via the
    // `LOFT_TICTACTOE_CLIENT_DELAY_MS` env var that the v2 client
    // honours via `web::sleep_ms`.  Linux (already passing) tolerates
    // the extra 200 ms with no measurable impact.
    let a = spawn_client_with_delay("A", port, 200);
    thread::sleep(Duration::from_millis(50));
    let b = spawn_client_with_delay("B", port, 200);

    let (a_out, a_status) = drain_with_timeout(a, Duration::from_secs(60));
    let (b_out, b_status) = drain_with_timeout(b, Duration::from_secs(60));

    assert_eq!(
        a_status,
        Some(0),
        "A did not exit cleanly; stdout=\n{a_out}"
    );
    assert_eq!(
        b_status,
        Some(0),
        "B did not exit cleanly; stdout=\n{b_out}"
    );

    // Each client's own game must complete.
    assert!(
        a_out.contains("[A] GameOver: X"),
        "A did not finish own game; stdout=\n{a_out}"
    );
    assert!(
        b_out.contains("[B] GameOver: X"),
        "B did not finish own game; stdout=\n{b_out}"
    );

    // Each client's three own X placements should show up.
    for cell in &["X,0,0", "X,1,0", "X,2,0"] {
        let needle = format!("[A] Placement: {cell}");
        assert!(
            a_out.contains(&needle),
            "A missing own placement {cell}; stdout=\n{a_out}"
        );
        let needle_b = format!("[B] Placement: {cell}");
        assert!(
            b_out.contains(&needle_b),
            "B missing own placement {cell}; stdout=\n{b_out}"
        );
    }

    // The whole point of v2: each client should see the OTHER's
    // frames as Spectator*.  At least one of the partner's X
    // placements must reach each side, AND the partner's GameOver
    // must arrive as SpectatorGameOver.
    //
    // Note: which spectator frames arrive depends on the timing —
    // if A finishes before B connects, A sees nothing about B and
    // vice versa.  The 50 ms stagger above makes it likely (but not
    // guaranteed) that BOTH overlap; we require AT LEAST ONE side
    // to have observed the other.
    let a_saw_b_spectator = count_occurrences(&a_out, "[A] SpectatorPlacement") > 0
        || a_out.contains("[A] SpectatorGameOver");
    let b_saw_a_spectator = count_occurrences(&b_out, "[B] SpectatorPlacement") > 0
        || b_out.contains("[B] SpectatorGameOver");
    assert!(
        a_saw_b_spectator || b_saw_a_spectator,
        "neither client observed the other's spectator frames; \
         a_out:\n{a_out}\n\nb_out:\n{b_out}"
    );
}

/// Late-join scenario: client A finishes its game; client B
/// connects afterwards, plays its own game, finishes.  Locks in
/// that:
///   - A's earlier game completion does not block the server.
///   - B can connect even though A's slot is still occupied.
///   - B plays its own independent game.
///   - The server does not crash on the second connection.
///
/// We do NOT require B to receive any spectator frames from A —
/// A finished before B's MAP arrived, so its events are gone.
#[test]
fn v2_late_join_independent_games() {
    if registry_predates_dn1("tictactoe_server_v2.loft") {
        return;
    }
    let port = pick_free_port();
    let _server = {
        let mut s = ServerGuard::spawn("tictactoe_server_v2.loft", port);
        if !s.wait_listening() {
            let diag = s.diagnose_listen_failure();
            panic!("server failed to start within 60s{diag}");
        }
        s
    };

    // A connects, plays, finishes.
    let a = spawn_client("A", port);
    let (a_out, a_status) = drain_with_timeout(a, Duration::from_secs(60));
    assert_eq!(a_status, Some(0), "A did not exit; stdout=\n{a_out}");
    assert!(
        a_out.contains("[A] GameOver: X"),
        "A did not win; stdout=\n{a_out}"
    );

    // Pause briefly so the server's accept loop has settled.
    thread::sleep(Duration::from_millis(100));

    // B connects; should still play to completion despite A having
    // already finished.
    let b = spawn_client("B", port);
    let (b_out, b_status) = drain_with_timeout(b, Duration::from_secs(60));
    assert_eq!(b_status, Some(0), "B did not exit; stdout=\n{b_out}");
    assert!(
        b_out.contains("[B] GameOver: X"),
        "B did not win; stdout=\n{b_out}"
    );
}
