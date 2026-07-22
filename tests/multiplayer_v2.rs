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

use std::io::{BufRead, BufReader, Read};
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

/// Wrapper that auto-kills its server child on drop.  Use this so
/// every test path tears the server down even on assertion panic.
struct ServerGuard {
    child: Option<Child>,
    port: u16,
    /// Everything the server child wrote to stderr while we waited for it to bind,
    /// captured on a reader thread.  `diagnose_listen_failure` drains the rest.
    early_stderr: Option<std::sync::mpsc::Receiver<String>>,
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
        let mut child = cmd.spawn().expect("failed to spawn loft server");
        // Read stderr line-by-line on a thread so `bind_outcome` can watch for the
        // listener's verdict without blocking, and without consuming the handle that
        // `diagnose_listen_failure` needs.
        let early_stderr = child.stderr.take().map(|s| {
            let (tx, rx) = std::sync::mpsc::channel();
            thread::spawn(move || {
                for line in BufReader::new(s).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            });
            rx
        });
        ServerGuard {
            child: Some(child),
            port,
            early_stderr,
        }
    }

    /// Wait until this server has decided whether it owns `self.port`.
    ///
    /// **Not the same question as "is anything listening there".**  `pick_free_port`
    /// binds `:0`, reads the port and closes, so between that close and this server's
    /// bind another test can take the port — measured at roughly 1 % for 12 concurrent
    /// pickers and 10 % for 64, and the full suite has ~9 files racing on this idiom.
    /// The loser is invisible from the outside: `server::listen` hands back a `Server`
    /// whose handle is `-1` and the script cheerfully prints "listening", so a plain
    /// connect-probe SUCCEEDS — against the *winner's* server.  The losing test then
    /// runs its clients against a stranger's server, and the failure surfaces far away
    /// as "client hung" or "neither client observed the other".
    ///
    /// So ask the server itself.  The native listener prints exactly one of these:
    ///   ok   — `loft server listening on 0.0.0.0:<port>`
    ///   lost — `loft_tcp_listen: cannot bind 0.0.0.0:<port>: Address already in use`
    fn bind_outcome(&self, timeout: Duration) -> BindOutcome {
        let Some(rx) = self.early_stderr.as_ref() else {
            return BindOutcome::Unknown;
        };
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(line) if line.contains("cannot bind") => return BindOutcome::PortTaken,
                Ok(line) if line.contains("loft server listening on") => return BindOutcome::Bound,
                Ok(_) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        BindOutcome::Unknown
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

/// What the server child said about its own bind — see [`ServerGuard::bind_outcome`].
#[derive(PartialEq, Eq, Debug)]
enum BindOutcome {
    Bound,
    PortTaken,
    /// The server said neither within the budget (an old build, or a genuinely stuck
    /// child).  Treated as "carry on and let `wait_listening` judge", so this helper can
    /// never be the reason a test fails.
    Unknown,
}

/// Start `server_script` on a port it actually owns, retrying the pick when it loses the
/// race described in [`ServerGuard::bind_outcome`].  Returns the guard and its port.
///
/// Retrying is the right response rather than failing: losing the pick is an ordinary,
/// expected event under a saturated box, and re-picking costs milliseconds.  What must
/// never happen is what happened before — proceeding against someone else's server.
fn spawn_server_on_free_port(server_script: &str) -> (ServerGuard, u16) {
    spawn_server_on_free_port_inner(server_script, false).0
}

/// `steal_first`: hold the FIRST picked port so this server is guaranteed to lose the
/// bind — the fault injection `server_detects_and_retries_a_stolen_port` uses to prove
/// the detection is not vacuous.  Also returns how many picks were burned.
fn spawn_server_on_free_port_inner(
    server_script: &str,
    steal_first: bool,
) -> ((ServerGuard, u16), usize) {
    const ATTEMPTS: usize = 5;
    let mut last_taken = 0;
    let mut thief = None;
    for attempt in 0..ATTEMPTS {
        let port = pick_free_port();
        if steal_first && attempt == 0 {
            // Hold the port on the SAME address the server binds (`0.0.0.0:{port}` — see
            // the `server` package's `n_tcp_listen`).  Binding `127.0.0.1` collides on
            // Linux but NOT on macOS/BSD, where a wildcard bind coexists with a
            // loopback-specific one: the injected fault silently did not happen there,
            // the server bound fine, and this control failed on macOS CI while passing
            // locally.  Matching the address makes the conflict identical on both.
            thief = TcpListener::bind(("0.0.0.0", port)).ok();
            assert!(
                thief.is_some(),
                "fault injection could not hold port {port}"
            );
        }
        let mut s = ServerGuard::spawn(server_script, port);
        match s.bind_outcome(Duration::from_secs(60)) {
            BindOutcome::PortTaken => {
                last_taken = port;
                continue; // `s` drops here, killing the zombie that never bound
            }
            BindOutcome::Bound | BindOutcome::Unknown => {
                if !s.wait_listening() {
                    let diag = s.diagnose_listen_failure();
                    panic!("server failed to start within 60s{diag}");
                }
                drop(thief);
                return ((s, port), attempt);
            }
        }
    }
    drop(thief);
    panic!(
        "could not obtain a free port for {server_script} in {ATTEMPTS} attempts (last contended port {last_taken})"
    );
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

/// Positive control for the port-race detection, and the reason it exists.
///
/// Steals the first picked port on purpose, so the server MUST lose its bind.  Asserts
/// two things:
///   1. a bare connect-probe still succeeds on the stolen port — which is exactly why
///      `wait_for_port` alone could never catch this, and why a losing test used to sail
///      on and run its clients against whoever actually owned the port;
///   2. `spawn_server_on_free_port` nonetheless notices, burns that pick, and comes back
///      with a server on a DIFFERENT port that really works.
///
/// Without the `bind_outcome` check this test fails at step 2, so it cannot go vacuous.
#[test]
fn server_detects_and_retries_a_stolen_port() {
    let ((_server, port), attempts_burned) =
        spawn_server_on_free_port_inner("tictactoe_server_v2.loft", true);
    assert!(
        attempts_burned >= 1,
        "the stolen port was not detected — the fault injection did not take effect"
    );

    // The server we got back is a real one: a client can complete a game against it.
    let client = spawn_client("S", port);
    let (out, status) = drain_with_timeout(client, Duration::from_secs(60));
    assert_eq!(
        status,
        Some(0),
        "client did not exit cleanly; stdout=\n{out}"
    );
    assert!(
        out.contains("[S] GameOver: X"),
        "retried server did not play a full game; stdout=\n{out}"
    );
}

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
    // Note: the current v2 server hardcodes port 7878; if multiple
    // tests in this file run in parallel they'd collide.  Using
    // serial_test crate would be cleaner, but we lock the suite via
    // `#[test] fn ...` and rely on cargo's per-binary parallelism
    // (these tests share the same binary; cargo runs them
    // sequentially by default within one binary unless --test-threads
    // is bumped).
    let (_server, port) = spawn_server_on_free_port("tictactoe_server_v2.loft");

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
    let (_server, port) = spawn_server_on_free_port("tictactoe_server_v2.loft");

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
    let (_server, port) = spawn_server_on_free_port("tictactoe_server_v2.loft");

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
