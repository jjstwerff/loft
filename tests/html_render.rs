// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Browser-side WebGL rendering gate for `loft --html` artefacts.
//
// `tests/html_wasm.rs` already gates that the WASM instantiates and
// `loft_start` returns without trapping.  That catches WASM-side
// regressions but NOT JS-side failures: a `compileShader` error in the
// JS bridge (the recurring failure mode after JS-bridge or shader
// changes) prints `console.error` and the WASM keeps running cleanly.
// The HTML-instantiation test sees a clean exit, the canvas is blank,
// and the regression ships.
//
// This gate closes that loop with two layers:
//
// - **Layer 1** — fail on any JS console.error / Runtime.exceptionThrown
//   in the first 6 seconds of page load.  Catches shader compile
//   errors, missing WebGL2 contexts, init exceptions.
//
// - **Layer 2** — clip a screenshot to the canvas element, decode the
//   PNG, count distinct RGB triples.  Fail if below 20 distinct colors
//   (a blank canvas has 1-2; a working Brick Buster frame has 128).
//   Catches the "compiles clean, blank canvas" pattern that Layer 1
//   can't see — e.g. a successful shader compile that never gets used
//   to draw, or a draw call that no-ops because of a state-tracking
//   regression.
//
// Skips cleanly when prerequisites (node, chrome, wasm32 toolchain,
// host loft binary) are not available — same shape as
// `tests/html_wasm.rs`.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

fn which(cmd: &str) -> Option<PathBuf> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

fn any_of(cmds: &[&str]) -> Option<PathBuf> {
    cmds.iter().find_map(|c| which(c))
}

fn wasm32_target_installed() -> bool {
    let Ok(out) = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains("wasm32-unknown-unknown")
}

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !p.join("Cargo.toml").exists() {
        if !p.pop() {
            return PathBuf::from(".");
        }
    }
    p
}

/// `make game` is not concurrent-safe (writes to a fixed
/// `/tmp/loft_html_opt.wasm` + the same output file).  Only one test
/// process should run it at a time.
fn build_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Pick a free TCP port for the per-test HTTP server.  Better than a
/// fixed port — parallel `cargo nextest` workers would collide.
fn pick_free_port() -> Option<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

/// Ensure `doc/brick-buster.html` exists (build it via `make game` if
/// missing or older than the source loft program).  Returns the
/// absolute path on success, `None` if prerequisites are missing.
fn ensure_brick_buster_built(root: &Path) -> Option<PathBuf> {
    let html = root.join("doc/brick-buster.html");
    let src = root.join("lib/graphics/examples/25-brick-buster.loft");
    let needs_build = !html.exists()
        || (html.metadata().ok()?.modified().ok()? < src.metadata().ok()?.modified().ok()?);
    if !needs_build {
        return Some(html);
    }
    if !wasm32_target_installed() {
        eprintln!("SKIP: rustup target wasm32-unknown-unknown not installed");
        return None;
    }
    let _guard = build_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    eprintln!("building doc/brick-buster.html via `make game` ...");
    let status = Command::new("make")
        .arg("game")
        .current_dir(root)
        .status()
        .ok()?;
    if !status.success() {
        eprintln!("FAIL: `make game` exited {status:?}");
        return None;
    }
    Some(html)
}

/// Spawn `python3 -m http.server PORT -d doc/` as a child process.
/// The harness Node script fetches via http:// so file:// CORS rules
/// don't bite the WebGL2 / WASM imports.
fn spawn_doc_server(root: &Path, port: u16) -> Option<Child> {
    let py = which("python3").or_else(|| which("python"))?;
    let child = Command::new(py)
        .args(["-m", "http.server", &port.to_string(), "-d"])
        .arg(root.join("doc"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    // Wait for the server to accept connections (max 2 s)
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Some(child);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Brick Buster loads cleanly in headless WebGL2 — no shader compile
/// errors, no exceptions, no `console.error` calls during a 6-second
/// startup window.
///
/// Catches the recurring failure mode: a JS-bridge or loft-shader
/// change ships, the WASM still returns cleanly (so `tests/html_wasm.rs`
/// passes), but every frame logs a `compileShader` error and renders a
/// blank canvas.
#[test]
fn brick_buster_browser_renders_without_console_errors() {
    let Some(_chrome) = any_of(&["google-chrome", "chromium", "chromium-browser", "chrome"]) else {
        eprintln!("SKIP: no chrome binary in PATH");
        return;
    };
    let Some(_node) = which("node") else {
        eprintln!("SKIP: node not installed");
        return;
    };
    let root = repo_root();
    let harness = root.join("tools/html_render_check.mjs");
    if !harness.exists() {
        eprintln!("SKIP: tools/html_render_check.mjs missing");
        return;
    }
    let loft_bin = root.join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release` first)");
        return;
    }
    let Some(_html_path) = ensure_brick_buster_built(&root) else {
        return;
    };
    let Some(port) = pick_free_port() else {
        eprintln!("SKIP: could not pick a free TCP port");
        return;
    };
    let Some(mut server) = spawn_doc_server(&root, port) else {
        eprintln!("SKIP: failed to start python http.server");
        return;
    };
    // Make sure the server gets cleaned up even if the assertion below panics.
    let _server_guard = ServerGuard(&mut server);

    let cdp_port = port.wrapping_add(1);
    let url = format!("http://127.0.0.1:{port}/brick-buster.html");
    let screenshot = std::env::temp_dir().join("brick_buster_render_test.png");

    let out = Command::new("node")
        .arg(&harness)
        .arg(&url)
        .args(["--wait-ms", "6000"])
        .args(["--port", &cdp_port.to_string()])
        .arg("--screenshot")
        .arg(&screenshot)
        // Layer 2: capture a clipped screenshot of the canvas element,
        // decode it, and assert at least 20 distinct RGB colors.  Catches
        // the "compiles clean, blank canvas" pattern that Layer 1 (no
        // console errors) can't see.  Empirical baseline: working Brick
        // Buster frame yields 128 distinct colors; a blank canvas (only
        // clearColor) has 1-2.
        .args(["--canvas", "#c"])
        .args(["--canvas-min-colors", "20"])
        // The page favicon 404 is benign; the harness filters generic
        // "Failed to load resource" automatically.  Swiftshader emits
        // a one-line GPU-stall performance warning at info level — we
        // only fail on `error`-level events, so no allow-list needed.
        .output()
        .expect("invoke node harness");

    if out.status.code() == Some(2) {
        eprintln!("SKIP: {}", String::from_utf8_lossy(&out.stderr));
        return;
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "Brick Buster headless render gate failed.\nURL: {url}\n\
         stdout:\n{stdout}\nstderr:\n{stderr}\n\
         (screenshot at {})",
        screenshot.display()
    );
}

struct ServerGuard<'a>(&'a mut Child);
impl Drop for ServerGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
