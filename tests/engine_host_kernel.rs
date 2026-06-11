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

/// @PLN18 08 scenario S3 — edit the interpreted fn; the MIXED build keeps
/// running.  The flip target is interpreted from startup (`LOFT_FLIP_FNS`);
/// the test then EDITS the source file under the serving kernel: a good
/// edit (+1 -> +100) applies live — observed as the world counter's STEP
/// changing while the counter itself stays monotone (continuity: no
/// restart, no reset); a broken edit (a real parse ERROR) keeps the +100
/// body serving; a signature change is rejected (call sites embed frame
/// sizes).  The timeline is differential: the interpreted leg (the original
/// tier-0 reload path) passes the SAME milestones.  Application is
/// asynchronous (dispatch-path poll vs the interp loop's op counter), so
/// the legs assert step timelines + reload milestones, not raw vectors;
/// the stderr file is the synchronization point for the REJECTED edits.
#[test]
fn s3_live_edit_under_native_baseline() {
    let native = run_s3_scenario(18097, false);
    let interp = run_s3_scenario(18098, true);
    assert!(
        native.stderr.contains("live-dispatch: n_bump_events"),
        "the native leg must dispatch through the interpreter:\n{}",
        native.stderr
    );
    for (leg, run) in [("native", &native), ("interp", &interp)] {
        assert!(
            run.stderr.contains("'bump_events' v1 live"),
            "{leg}: the good edit must reload:\n{}",
            run.stderr
        );
        assert!(
            run.stderr.contains("kept its old body"),
            "{leg}: the broken edit must be refused:\n{}",
            run.stderr
        );
        assert!(
            run.stderr.contains("changed its signature"),
            "{leg}: the signature change must be refused:\n{}",
            run.stderr
        );
    }
}

fn s3_fixture(port: u16, bump_decl: &str, bump_stmt: &str) -> String {
    format!(
        r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
{bump_decl} {{
  {bump_stmt}
  w.events
}}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      engine_host::broadcast("got:{{ev.payload}}#{{n}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
    )
}

/// One ping round trip; returns the counter and asserts it advanced
/// (monotone = the world survived whatever just happened).
fn s3_ping(ws: &TcpStream, last: &mut i64) -> i64 {
    ws_send(ws, "p");
    let r = ws_recv(ws);
    let n: i64 = r
        .rsplit('#')
        .next()
        .and_then(|x| x.parse().ok())
        .unwrap_or_else(|| panic!("unparseable reply {r:?}"));
    assert!(n > *last, "world counter went backwards: {} -> {n}", *last);
    let step = n - *last;
    *last = n;
    step
}

/// Ping until the counter's step matches `want` (the edit has applied).
fn s3_await_step(ws: &TcpStream, last: &mut i64, want: i64) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if s3_ping(ws, last) == want {
            return;
        }
        assert!(Instant::now() < deadline, "step never became {want}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Block until the kernel's stderr contains `needle` — the deterministic
/// sync point for an edit whose only observable is a refusal line.
/// Keeps PINGING while it waits: in the mixed build the reload poll runs
/// inside the dispatch path, so an edit is only examined when the flipped
/// fn is actually called (lazy by construction — an S3 finding; the pings
/// drive it, and stay step-correct on the old body, proving refusal en
/// route).
fn s3_await_stderr(ws: &TcpStream, last: &mut i64, path: &std::path::Path, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        s3_ping(ws, last);
        if std::fs::read_to_string(path)
            .unwrap_or_default()
            .contains(needle)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "stderr never contained {needle:?}"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn run_s3_scenario(port: u16, interpret: bool) -> S2Run {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return S2Run {
            replies: Vec::new(),
            stderr: String::new(),
        };
    }
    let decl_v0 = "fn bump_events(w: W) -> integer";
    let prog = std::env::temp_dir().join(format!("eh_s3_{port}_{}.loft", std::process::id()));
    std::fs::write(&prog, s3_fixture(port, decl_v0, "w.events = w.events + 1;")).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = std::env::temp_dir().join(format!("eh_s3_{port}_{}.err", std::process::id()));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_LIVE_RELOAD", "1");
    if interpret {
        cmd.arg("--interpret");
    } else {
        cmd.env("LOFT_LIVE_FLIP", "1")
            .env("LOFT_FLIP_FNS", "bump_events")
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
    let mut last = 0i64;
    assert_eq!(s3_ping(&ws, &mut last), 1, "original body steps by 1");
    // Good edit: the step becomes +100; the world counter continues.
    std::fs::write(
        &prog,
        s3_fixture(port, decl_v0, "w.events = w.events + 100;"),
    )
    .unwrap();
    s3_await_step(&ws, &mut last, 100);
    // Broken edit: a real parse ERROR (the lenient parser downgrades a
    // missing operand to warning + null, so use an unknown name).  Sync on
    // the refusal line, then prove the +100 body still serves.
    std::fs::write(
        &prog,
        s3_fixture(port, decl_v0, "w.events = w.events + nosuchvar;"),
    )
    .unwrap();
    s3_await_stderr(&ws, &mut last, &err_path, "kept its old body");
    assert_eq!(
        s3_ping(&ws, &mut last),
        100,
        "broken edit must not change the body"
    );
    // Signature change: rejected; the +100 body still serves.
    std::fs::write(
        &prog,
        s3_fixture(
            port,
            "fn bump_events(w: W, extra: integer) -> integer",
            "w.events = w.events + extra;",
        ),
    )
    .unwrap();
    s3_await_stderr(&ws, &mut last, &err_path, "changed its signature");
    assert_eq!(
        s3_ping(&ws, &mut last),
        100,
        "sig change must not change the body"
    );
    drop(_guard); // kill now so the stderr file is complete
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
    S2Run {
        replies: vec![last.to_string()],
        stderr,
    }
}

/// One full connect+upgrade ATTEMPT; `None` on any failure.  During the
/// swap's dual-bind overlap a dial can land on the DYING process's listener
/// queue and drop mid-upgrade — seats (and this harness) retry the whole
/// attempt, which converges on the new build within the gap bound.
fn ws_try_connect(port: u16) -> Option<TcpStream> {
    let stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let req = "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
               Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
               Sec-WebSocket-Version: 13\r\n\r\n";
    (&stream).write_all(req.as_bytes()).ok()?;
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).ok()?;
        head.push(b[0]);
    }
    String::from_utf8_lossy(&head)
        .contains("101")
        .then_some(stream)
}

/// Tolerant frame read for swap-gap detection: `None` = the connection
/// closed (the handover) or timed out.
fn ws_try_recv(stream: &TcpStream) -> Option<String> {
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).ok()?;
    let len = match hdr[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).ok()?;
            u16::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    if hdr[0] & 0x0F == 0x08 {
        return None; // close frame
    }
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).ok()?;
    String::from_utf8(payload).ok()
}

/// @PLN18 08 scenario S5 — the native swap: a new process under a running
/// world.  One test drives the WHOLE heart pipeline: a compiled kernel
/// serves with a fn flipped to the interpreter (S2) → rollback legs (a
/// missing artifact is refused; a build that dies before serving rolls
/// back while the world keeps counting) → a live source edit (S3's
/// subject) → background rebuild (S4) → swap: the world counter crosses
/// the cutover EXACTLY continuous, the tick stamp never resets, the WS
/// seat's gap is bounded (measured — probe gate 5's seat-reconnect
/// verdict), and the flipped fn runs COMPILED in the new build (dispatch
/// reset: zero post-swap interp dispatches).
/// Kill any process running an `eh_s5` cache binary — OUR OWN fixture's
/// stem, exactly anchored.  The swap child outlives its parent chain by
/// DESIGN (it is the new server), so the process-group Guard alone cannot
/// be relied on across runs; with SO_REUSEPORT a stale orphan silently
/// SHARES the test port and poisons every dial (probe-caught).
fn s5_kill_stale(stem: &str) {
    if let Ok(out) = Command::new("pgrep").arg("-f").arg(stem).output() {
        for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            if let Ok(pid) = pid.parse::<i32>() {
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }
}

struct S5Hygiene(&'static str);
impl Drop for S5Hygiene {
    fn drop(&mut self) {
        s5_kill_stale(self.0);
    }
}

#[test]
fn s5_native_swap_under_running_world() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    const STEM: &str = "/.loft/cache/eh_s5_18100-";
    s5_kill_stale(STEM); // a stale orphan from a prior run shares the port
    let _hygiene = S5Hygiene(STEM); // and OUR swap child must die at exit
    std::thread::sleep(Duration::from_millis(200));
    let port = 18100u16;
    let fixture = |ver: &str| {
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
  resumed = engine_host::swap_world(w);
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      n = bump_events(w);
      if ev.payload == "rebuild" {{
        engine_host::send(ev.cid, "rebuild:{{engine_host::rebuild_start()}}");
        return;
      }}
      if ev.payload == "status" {{
        engine_host::send(ev.cid, "status:{{engine_host::rebuild_status()}}");
        return;
      }}
      if ev.payload == "swap" {{
        engine_host::send(ev.cid, "swap:{{engine_host::swap_start(engine_host::rebuild_artifact())}}");
        return;
      }}
      if ev.payload == "badpath" {{
        engine_host::send(ev.cid, "badpath:{{engine_host::swap_start("/nonexistent/binary")}}");
        return;
      }}
      if ev.payload == "badswap" {{
        engine_host::send(ev.cid, "badswap:{{engine_host::swap_start("/bin/false")}}");
        return;
      }}
      engine_host::send(ev.cid, "{ver}:{{ev.payload}}#{{n}}t{{w.ticks}}");
    }},
    fn() {{ w.ticks = w.ticks + 1; }});
}}
"#
        )
    };
    let prog = std::env::temp_dir().join(format!("eh_s5_{port}.loft"));
    std::fs::write(&prog, fixture("v1")).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = std::env::temp_dir().join(format!("eh_s5_{port}.err"));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.env("LOFT_LIVE_FLIP", "1")
        .env("LOFT_FLIP_FNS", "bump_events")
        .env("LOFT_DISPATCH_DEBUG", "1");
    cmd.process_group(0);
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

    let stderr_now = || std::fs::read_to_string(&err_path).unwrap_or_default();
    let mut count = 0i64; // every event bumps the world counter
    let ws = ws_connect(port);
    let ask = |ws: &TcpStream, msg: &str, count: &mut i64| -> String {
        ws_send(ws, msg);
        *count += 1;
        ws_recv(ws)
    };
    // The flipped baseline serves (S2 still alive under this fixture).
    let r = ask(&ws, "a", &mut count);
    assert_eq!(
        r,
        format!("v1:a#{count}t")
            .rsplit_once('t')
            .unwrap()
            .0
            .to_string()
            + "t"
            + r.rsplit_once('t').unwrap().1,
        "shape"
    );
    assert!(r.starts_with(&format!("v1:a#{count}t")), "baseline: {r}");
    assert!(stderr_now().contains("live-dispatch: n_bump_events"));

    // Rollback leg 1: a missing artifact is refused outright (no freeze).
    assert_eq!(ask(&ws, "badpath", &mut count), "badpath:false");
    // Rollback leg 2: a build that dies before serving.  The freeze defers
    // the next replies; they drain after the rollback (recv timeout rides
    // it).  The world must keep counting and the old build keep serving.
    assert_eq!(ask(&ws, "badswap", &mut count), "badswap:true");
    let r = ask(&ws, "b", &mut count);
    assert!(
        r.starts_with(&format!("v1:b#{count}t")),
        "post-rollback: {r}"
    );
    assert!(
        stderr_now().contains("rolled back"),
        "the dead build must roll back:\n{}",
        stderr_now()
    );

    // S3+S4: live edit, then background rebuild to ready.
    std::fs::write(&prog, fixture("v2")).unwrap();
    assert_eq!(ask(&ws, "rebuild", &mut count), "rebuild:true");
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let st = ask(&ws, "status", &mut count);
        if st == "status:2" {
            break;
        }
        assert!(Instant::now() < deadline, "rebuild never ready");
        std::thread::sleep(Duration::from_millis(300));
    }

    // THE SWAP.  The ack arrives before the freeze; the world counter and
    // tick stamp in the LAST pre-swap reply are the continuity anchors.
    let r = ask(&ws, "c", &mut count);
    assert!(r.starts_with(&format!("v1:c#{count}t")), "pre-swap: {r}");
    let t_pre: i64 = r.rsplit_once('t').unwrap().1.parse().unwrap();
    assert_eq!(ask(&ws, "swap", &mut count), "swap:true");

    // Wait for the handover: the old process closes this connection.
    ws.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let gap_start = Instant::now();
    while ws_try_recv(&ws).is_some() {}
    // Reconnect into the NEW build (seat-reconnect semantics; measure it).
    // Full-attempt retry: a dial in the dual-bind overlap can land on the
    // dying process and drop mid-upgrade.
    let reconnect_deadline = Instant::now() + Duration::from_secs(10);
    let new_ws = loop {
        if let Some(ws) = ws_try_connect(port) {
            break ws;
        }
        if Instant::now() >= reconnect_deadline {
            // Diagnostics: who exists, who listens, what the server said.
            let ps = Command::new("pgrep").args(["-af", "eh_s5_18100"]).output();
            let ss = Command::new("ss").args(["-tlnp"]).output();
            panic!(
                "never reconnected into the new build\n--- pgrep:\n{}\n--- ss -tlnp:\n{}\n--- server stderr tail:\n{}",
                String::from_utf8_lossy(&ps.map(|o| o.stdout).unwrap_or_default()),
                String::from_utf8_lossy(&ss.map(|o| o.stdout).unwrap_or_default())
                    .lines()
                    .filter(|l| l.contains("18100"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                stderr_now()
                    .lines()
                    .rev()
                    .take(8)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let gap = gap_start.elapsed();
    eprintln!("S5 swap gap (close -> serving reconnect): {gap:?}");
    assert!(
        gap < Duration::from_secs(3),
        "the swap gap must stay under the keepalive timeout: {gap:?}"
    );

    // Continuity: the new build serves v2 meaning over the SAME world —
    // counter exactly +1 (nothing lost, nothing duplicated), ticks never
    // reset.
    let marker = stderr_now();
    let r = ask(&new_ws, "d", &mut count);
    assert!(
        r.starts_with(&format!("v2:d#{count}t")),
        "the new build must serve the new meaning over the OLD world: {r}"
    );
    let t_post: i64 = r.rsplit_once('t').unwrap().1.parse().unwrap();
    assert!(
        t_post >= t_pre,
        "tick stamp must cross the swap monotonically ({t_pre} -> {t_post})"
    );
    assert!(
        marker.contains("loft-swap: world restored from"),
        "the new build must restore the snapshot:\n{marker}"
    );
    // Dispatch reset: bump_events is COMPILED in the new build — no interp
    // dispatches after the handover marker.
    let full = stderr_now();
    let after = full.split("handing over").nth(1).unwrap_or("");
    let _ = ask(&new_ws, "e", &mut count); // drive one more dispatch window
    assert!(
        !after.contains("live-dispatch:"),
        "the swap must reset the dispatch tier:\n{after}"
    );
    drop(_guard);
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
}

/// @PLN18 08 scenario S4 — the background rebuild: the serve host compiles
/// the full project (a real rustc run) while the OLD build keeps serving.
/// Asserts the design's three clauses:
/// (a) the artifact lands and corresponds to the source — a repeat request
///     on unchanged source is an instant cache hit on the SAME path;
/// (b) build isolation — the tick counter keeps advancing in every poll
///     interval while rustc runs (probe gate 4);
/// (c) an edit DURING the build invalidates it — "requeued" on stderr, and
///     the final artifact is the settled source's build.
/// The during-build edit sets the source back to the spawn content; the
/// cache keeps ONE binary per source stem, so the unique-content build
/// evicts it and the requeued build is a second real rustc — both builds
/// are measurement windows for (b).
#[test]
fn s4_background_rebuild_under_serving_kernel() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let port = 18099u16;
    // Tick step: unique per run -> the rebuild is ALWAYS a cache miss (a
    // real rustc window for (b)/(c)); behavior-equal (ticks just advance).
    let unique_step = (std::process::id() % 1000) + 2;
    let fixture = |tick_step: u32| {
        format!(
            r#"
use engine_host;
struct W {{ events: integer not null, ticks: integer not null }}
fn main() {{
  w = W {{ events: 0, ticks: 0 }};
  engine_host::run({port}, 10000,
    fn(ev: engine_host::Event) {{
      if ev.kind != 1 {{ return; }}
      if ev.payload == "rebuild" {{
        engine_host::send(ev.cid, "rebuild:{{engine_host::rebuild_start()}}");
        return;
      }}
      if ev.payload == "status" {{
        engine_host::send(ev.cid, "status:{{engine_host::rebuild_status()}}");
        return;
      }}
      if ev.payload == "artifact" {{
        engine_host::send(ev.cid, "artifact:{{engine_host::rebuild_artifact()}}");
        return;
      }}
      engine_host::send(ev.cid, "ticks:{{w.ticks}}");
    }},
    fn() {{ w.ticks = w.ticks + {tick_step}; }});
}}
"#
        )
    };
    let prog = std::env::temp_dir().join(format!("eh_s4_{port}.loft"));
    std::fs::write(&prog, fixture(1)).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let err_path = std::env::temp_dir().join(format!("eh_s4_{port}.err"));
    let err_file = std::fs::File::create(&err_path).unwrap();
    let mut cmd = Command::new(loft_bin());
    cmd.process_group(0);
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
    let ask = |msg: &str| -> String {
        ws_send(&ws, msg);
        ws_recv(&ws)
    };
    let ticks = |reply: String| -> i64 {
        reply
            .strip_prefix("ticks:")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("unparseable {reply:?}"))
    };

    // ── Phase 1 (b): a clean unique-content rebuild — a real rustc window
    // with no interference; every poll interval must show tick progress.
    // (The running v0 binary is unaffected: reload is OFF.)
    std::fs::write(&prog, fixture(unique_step)).unwrap();
    assert_eq!(ask("rebuild"), "rebuild:true");
    let mut last_ticks = ticks(ask("t"));
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let now_ticks = ticks(ask("t"));
        assert!(
            now_ticks > last_ticks,
            "tick stalled during the background build (build isolation broken)"
        );
        last_ticks = now_ticks;
        let st = ask("status");
        if st == "status:2" {
            break;
        }
        assert!(
            st == "status:1",
            "rebuild must stay building/ready, got {st}"
        );
        assert!(Instant::now() < deadline, "rebuild never became ready");
    }

    // ── Phase 2 (c): an edit DURING a build invalidates it.  The snapshot
    // is taken at request time, so wherever the edit lands relative to the
    // child's own file read, completion sees drift -> requeue.  The settled
    // content (v0) is already cached -> the requeued build converges fast.
    std::fs::write(&prog, fixture(unique_step + 1)).unwrap();
    assert_eq!(ask("rebuild"), "rebuild:true");
    std::fs::write(&prog, fixture(1)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let st = ask("status");
        if st == "status:2" {
            break;
        }
        assert!(
            st == "status:1",
            "requeue path must stay building, got {st}"
        );
        assert!(Instant::now() < deadline, "requeued rebuild never ready");
        std::thread::sleep(Duration::from_millis(300));
    }
    let stderr = std::fs::read_to_string(&err_path).unwrap_or_default();
    assert!(
        stderr.contains("rebuild stale (source changed during build) — requeued"),
        "the mid-build edit must requeue:\n{stderr}"
    );
    // (a) the artifact exists and is the settled source's build.
    let artifact = ask("artifact");
    let path = artifact.strip_prefix("artifact:").unwrap().to_string();
    assert!(!path.is_empty(), "ready artifact must have a path");
    assert!(
        std::path::Path::new(&path).exists(),
        "artifact must exist: {path}"
    );
    // (a) repeat request on unchanged source: instant cache hit, SAME path.
    assert_eq!(ask("rebuild"), "rebuild:true");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let st = ask("status");
        if st == "status:2" {
            break;
        }
        assert!(Instant::now() < deadline, "cache-hit rebuild never ready");
        std::thread::sleep(Duration::from_millis(250));
    }
    assert_eq!(
        ask("artifact"),
        format!("artifact:{path}"),
        "unchanged source must yield the SAME artifact (hash-stable)"
    );
    drop(_guard);
    let _ = std::fs::remove_file(&prog);
    let _ = std::fs::remove_file(&err_path);
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
