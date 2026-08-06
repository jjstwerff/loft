// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN38 — end-to-end exercise of the loft-callable path-backed
//! Store binding (`store_persist_bind`) via the compiled `loft`
//! binary.
//!
//! Drives `tests/scripts/store_persist_smoke.loft` twice: a fresh-mode
//! run that populates a hash and binds it to a path that doesn't yet
//! exist, then a reload-mode run that binds an empty hash to the
//! same (now-existing) path and reads the keys back.  The two runs'
//! outputs must agree, proving the "the hash IS the file" round-trip.
//!
//! Companion to `tests/store_durable_loft.rs` which exercises the
//! integrity-check / seal half of the same plan family.

#![cfg(feature = "mmap")]

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

/// Serve `bytes` over a minimal HTTP/1.1 server that honours `Range: bytes=a-b`
/// with a `206 Partial Content` + `Content-Range` (200/whole-file otherwise) —
/// the remote store the `store_load_key` HTTP path (Phase 5) fetches from.
/// Returns the URL; the server thread is detached (ends with the test binary).
fn serve_ranges(store: Vec<u8>, sidecar: Option<Vec<u8>>) -> String {
    serve_ranges_tuned(store, sidecar, 0, None).0
}

/// loft#782 — the same server, with the two knobs a ROUND-TRIP claim needs.
///
/// `delay_ms` is injected per request, and every connection is handled on its own thread,
/// so a client that issues its ranges concurrently finishes in ~one delay while a serial
/// one pays `requests × delay`. Both matter: with the single-threaded server the socket
/// would serialise a concurrent client anyway, and the harness would report "no win" for a
/// fix that worked.
///
/// `counter` counts requests served, which is the DETERMINISTIC half — wall-clock is
/// suggestive and flaky, but round-trip depth is exact and is what the defect is about.
fn serve_ranges_tuned(
    store: Vec<u8>,
    sidecar: Option<Vec<u8>>,
    delay_ms: u64,
    counter: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let hits =
        counter.unwrap_or_else(|| std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)));
    let out = std::sync::Arc::clone(&hits);
    let store = std::sync::Arc::new(store);
    let sidecar = std::sync::Arc::new(sidecar);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let store = std::sync::Arc::clone(&store);
            let sidecar = std::sync::Arc::clone(&sidecar);
            let hits = std::sync::Arc::clone(&hits);
            thread::spawn(move || {
                let (store, sidecar) = (&*store, &*sidecar);
                hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if delay_ms > 0 {
                    thread::sleep(std::time::Duration::from_millis(delay_ms));
                }
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                // Path-aware (real servers 404 a missing sidecar): serve the layout
                // sidecar whole at `/store.dschema` (or 404 when absent), so the
                // @PLN97 3b.5 gate fetches a REAL sidecar, not the store bytes.
                let req_path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");
                if req_path.ends_with(".dschema") {
                    match &sidecar {
                        Some(sc) => {
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                sc.len()
                            );
                            let _ = s.write_all(hdr.as_bytes());
                            let _ = s.write_all(sc);
                        }
                        None => {
                            let _ = s.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        }
                    }
                    return; // the sidecar path is answered; this connection is done
                }
                let total = store.len();
                let range = req.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("Range: bytes=")
                        .or_else(|| l.trim().strip_prefix("range: bytes="))
                });
                if let Some(r) = range {
                    let (a, b) = r.split_once('-').unwrap_or(("0", ""));
                    let a: usize = a.trim().parse().unwrap_or(0);
                    let b: usize = b
                        .trim()
                        .parse()
                        .unwrap_or(total.saturating_sub(1))
                        .min(total.saturating_sub(1));
                    let body = if a <= b { &store[a..=b] } else { &store[0..0] };
                    let hdr = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{total}\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(body);
                } else {
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(store);
                }
            });
        }
    });
    (format!("http://127.0.0.1:{port}/store"), out)
}

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn smoke_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_persist_smoke.loft")
}

fn sorted_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_persist_sorted.loft")
}

/// Run a persist script in a chosen mode (fresh / reload) — the generic form
/// of `run_smoke`, parameterised by the script so the hash and sorted cases
/// share one driver.
fn run_mode(script: &Path, path: &Path, mode: &str) -> (String, i32) {
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        eprintln!("{mode} stderr:\n{stderr}");
    }
    (stdout, out.status.code().unwrap_or(-1))
}

fn scratch(test_name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("loft-store-persist-loft").join(test_name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_smoke(path: &Path, mode: &str) -> (String, i32) {
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(smoke_script())
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let code = out.status.code().unwrap_or(-1);
    if !out.status.success() {
        eprintln!("smoke stderr:\n{stderr}");
    }
    (stdout, code)
}

/// Run an arbitrary persist script on a chosen backend with the scratch path.
fn run_script(script: &Path, backend: &str, path: &Path) -> (String, i32) {
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        eprintln!("{backend} stderr:\n{stderr}");
    }
    (stdout, out.status.code().unwrap_or(-1))
}

/// #513 — within one process, every bind into a freshly-declared hash must read
/// the persisted data (not just the first).  Freeing a bound mmap store left it
/// in its slot; reusing that slot `init()`'d the empty header through the mmap,
/// so later binds read empty.  Must hold on BOTH backends.
#[test]
fn multi_bind_reads_data_both_backends() {
    let script = workspace_root().join("tests/scripts/store_persist_multibind_513.loft");
    for backend in ["--interpret", "--native"] {
        let dir = scratch(&format!(
            "multibind_513_{}",
            backend.trim_start_matches('-')
        ));
        let path = dir.join("world.store");
        let (out, code) = run_script(&script, backend, &path);
        assert_eq!(code, 0, "{backend} exit: {out:?}");
        assert!(
            out.contains("counts=3,3,3"),
            "{backend}: every bind must read 3 records (bug gave 3,0,0 / 0,0,0): {out:?}"
        );
    }
}

#[test]
fn fresh_then_reload_round_trip() {
    let dir = scratch("fresh_then_reload_round_trip");
    let path = dir.join("world.store");

    // Pass 1 — path does not exist; the script populates a hash and
    // binds it to disk.
    assert!(!path.exists(), "scratch should start clean");
    let (out1, code1) = run_smoke(&path, "fresh");
    assert_eq!(code1, 0, "fresh exit: stdout={out1:?}");
    assert!(
        out1.contains("fresh keys=7,13,42"),
        "fresh keys missing: {out1:?}"
    );
    assert!(path.exists(), "bind should have created the file");
    let meta = fs::metadata(&path).unwrap();
    assert!(
        meta.len() >= 8192,
        "file should be padded to ≥1024 words: got {} bytes",
        meta.len()
    );

    // Pass 2 — same path, empty in-memory hash; bind should load the
    // on-disk contents.
    let (out2, code2) = run_smoke(&path, "reload");
    assert_eq!(code2, 0, "reload exit: stdout={out2:?}");
    assert!(
        out2.contains("reload keys=7,13,42"),
        "reload keys mismatch: {out2:?}"
    );
    // #523 — a KEY LOOKUP must succeed in this SEPARATE reload process, not
    // just iteration.  Pre-fix, the hash used a per-process random seed so the
    // reload process probed a different bucket and read null (v=1300 vanished)
    // while iteration still listed the key.  The per-hash seed now lives in the
    // bucket record, so any reader re-derives the same buckets.
    assert!(
        out2.contains("reload lookup h[13]=1300"),
        "reload cross-process lookup must find v=1300, not null (#523): {out2:?}"
    );
}

/// @PLN123 A0 — the high-water mark on the shape arc A actually targets: a
/// BOUND store, re-opened from its file.  Gating a shrink on `walk_complete`
/// is only worth anything if a healthy bound store reports it TRUE; were the
/// image to leave a zero-header tail, the gate would refuse forever and arc A
/// would be dead on arrival while every test still passed.  A padded image
/// tiles its arena exactly — `build_padded_store_image` lays one free block
/// over the slack above the last record — and this pins that.
#[test]
fn a_bound_store_image_reports_a_trustworthy_mark() {
    let dir = scratch("bound_store_mark");
    let path = dir.join("world.store");
    let (out, code) = run_smoke(&path, "fresh");
    assert_eq!(code, 0, "fresh exit: stdout={out:?}");

    let store = loft::store::Store::open(path.to_str().expect("utf-8 path"));
    let u = store.usage();
    assert!(
        u.walk_complete,
        "a bound store's block chain must tile its file, or arc A can never \
         shrink one: claimed={} free={} mark={} capacity={}",
        u.claimed_words, u.free_words, u.live_end_words, u.capacity_words
    );
    assert!(
        u.claimed_words > 0,
        "the image holds the smoke script's records"
    );
    assert!(
        u.live_end_words <= u.capacity_words,
        "mark {} past capacity {}",
        u.live_end_words,
        u.capacity_words
    );
    assert!(
        u.capacity_words - u.live_end_words > 0,
        "the image keeps an eighth of slack above the mark (loft#710), which is \
         exactly the tail arc A gives back"
    );
}

/// @PLN97 arc G Phase 0.5b — `store_persist_bind` on a `sorted<T[k]>` must
/// round-trip across processes: a `sorted` is comparison-based (no per-process
/// hash seed), so the reload process must iterate in key order AND key-look-up
/// correctly.  This exercises the `reference`-parameter widening that lets
/// `store_persist_bind` accept any keyed collection, not just `hash`.
#[test]
fn sorted_fresh_then_reload_round_trip() {
    let dir = scratch("sorted_fresh_then_reload_round_trip");
    let path = dir.join("sorted.store");

    assert!(!path.exists(), "scratch should start clean");
    let (out1, code1) = run_mode(&sorted_script(), &path, "fresh");
    assert_eq!(code1, 0, "fresh exit: stdout={out1:?}");
    assert!(
        out1.contains("fresh keys=7,13,42"),
        "fresh sorted keys must be ascending 7,13,42: {out1:?}"
    );
    assert!(path.exists(), "bind should have created the file");

    // Reload in a SEPARATE process: both iteration and lookup must survive.
    let (out2, code2) = run_mode(&sorted_script(), &path, "reload");
    assert_eq!(code2, 0, "reload exit: stdout={out2:?}");
    assert!(
        out2.contains("reload keys=7,13,42"),
        "reload sorted keys mismatch: {out2:?}"
    );
    assert!(
        out2.contains("reload lookup s[13]=1300"),
        "reload cross-process sorted lookup must find v=1300, not null: {out2:?}"
    );
}

fn load_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_smoke.loft")
}

fn text_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_text.loft")
}

fn vec_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_vec.loft")
}

fn textkey_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_textkey.loft")
}

fn range_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_range.loft")
}

fn nested_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_nested.loft")
}

fn vecstruct_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_vecstruct.loft")
}

fn gate_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_layout_gate.loft")
}

fn gate_changed_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_layout_gate_changed.loft")
}

fn vectext_refuse_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_vectext_refuse.loft")
}

/// `run_mode` fixed to `--interpret`, parameterised by backend so the heap
/// `store_load` path can be exercised on the native backend too.
fn run_mode_backend(backend: &str, script: &Path, path: &Path, mode: &str) -> (String, i32) {
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        eprintln!("{backend} {mode} stderr:\n{stderr}");
    }
    (stdout, out.status.code().unwrap_or(-1))
}

/// @PLN97 arc G Phase 1 — `store_load` reads a bind-written store IMAGE into a
/// FRESH, HEAP-backed hash (no mmap) and returns the same keys AND key-lookups.
/// The write uses the mmap path (`store_persist_bind`); the load is the
/// portable heap path, verified on BOTH the interpreter and native (the piece
/// wasm lacked — heap load with no live file handle).
#[test]
fn store_load_reads_persisted_image_both_backends() {
    let dir = scratch("store_load_phase1");
    let path = dir.join("load.store");

    // write the image once (mmap path)
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(
        out_w.contains("write keys=7,13,42"),
        "write keys: {out_w:?}"
    );
    assert!(path.exists(), "bind should create the file");

    // load it back via the heap path on both backends — same keys + lookup
    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &load_script(), &path, "load");
        assert_eq!(code, 0, "{backend} load exit: {out:?}");
        assert!(
            out.contains("load keys=7,13,42"),
            "{backend}: store_load must read all keys: {out:?}"
        );
        assert!(
            out.contains("load lookup h[13]=1300"),
            "{backend}: store_load key lookup must find v=1300: {out:?}"
        );
    }
}

/// A garbage / non-store file must make `store_load` return false (a clean
/// reject), never panic or misread past the signature check.
#[test]
fn store_load_rejects_garbage_file() {
    let dir = scratch("store_load_reject");
    let path = dir.join("garbage.store");
    fs::write(
        &path,
        b"this is not a loft store image, only junk bytes here!!",
    )
    .unwrap();

    let (out, code) = run_mode(&load_script(), &path, "load");
    assert_eq!(
        code, 0,
        "reject should be graceful (script prints FAIL, exit 0): {out:?}"
    );
    assert!(
        out.contains("FAIL load-returned-false"),
        "garbage file must reject cleanly (store_load=false): {out:?}"
    );
}

/// @PLN97 arc G Phase 3a — `store_load_key` fetches ONLY the requested integer
/// key (the working set) from a persisted hash image, not the whole file. On
/// both backends: the loaded key is present with the right value, un-requested
/// keys are absent, and `len == 1` (proving the load is bounded, not a full
/// store_load). The read touches only the pages the lookup needs.
#[test]
fn store_load_key_loads_only_the_requested_key_both_backends() {
    let dir = scratch("store_load_key_phase3a");
    let path = dir.join("k.store");

    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &load_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(
            out.contains("loadkey keys=13"),
            "{backend}: only key 13 must be present (bounded working set): {out:?}"
        );
        assert!(
            out.contains("loadkey lookup h[13]=1300"),
            "{backend}: the loaded key's value must be correct: {out:?}"
        );
        assert!(
            out.contains("loadkey len=1"),
            "{backend}: exactly one entry loaded, not the whole store: {out:?}"
        );
        assert!(
            out.contains("loadkey verify=true"),
            "{backend}: the loaded store must be structurally sound (store_verify): {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3a (plural) — `store_load_keys` fetches a SUBSET of keys
/// ({7, 42}) in one call, returns the count found (2), and leaves un-requested
/// keys (13) absent. Both backends.
#[test]
fn store_load_keys_loads_the_requested_subset_both_backends() {
    let dir = scratch("store_load_keys_phase3a");
    let path = dir.join("ks.store");

    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &load_script(), &path, "loadkeys");
        assert_eq!(code, 0, "{backend} loadkeys exit: {out:?}");
        assert!(
            out.contains("loadkeys got=2"),
            "{backend}: both requested keys found: {out:?}"
        );
        assert!(
            out.contains("h7=700 h42=4200 h13=null"),
            "{backend}: subset values correct + 13 absent: {out:?}"
        );
        assert!(
            out.contains("loadkeys len=2"),
            "{backend}: exactly the subset loaded: {out:?}"
        );
    }
}

/// loft#730 — a store does not keep the slack its vectors grew to.
///
/// `Store::resize` leaves an in-place growth at 7/4 of the size that triggered
/// it and never gives it back, so the image carries it. Compaction could not see
/// it: the measure it gated on is free space BETWEEN records, and this is a
/// fully claimed, entirely live record. A store made of nothing else reported as
/// dense, so the shape a rebuild pays best on was the one shape it refused.
///
/// Sizes are asserted with the digest, because a file that got smaller by LOSING
/// data satisfies every size assertion anyone would write.
#[test]
fn a_rebound_store_sheds_the_slack_its_vectors_grew_to() {
    let dir = scratch("store_compact_slack_730");
    let path = dir.join("s.store");
    let script = workspace_root().join("tests/scripts/store_compact_slack_730.loft");

    let field = |out: &str, key: &str| -> String {
        out.split_whitespace()
            .find_map(|t| t.strip_prefix(key).map(str::to_string))
            .unwrap_or_default()
    };

    for backend in ["--interpret", "--native"] {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("store.dschema"));
        let (w, cw) = run_mode_backend(backend, &script, &path, "write");
        assert_eq!(cw, 0, "{backend} write exit: {w:?}");
        let (r, cr) = run_mode_backend(backend, &script, &path, "rebind");
        assert_eq!(cr, 0, "{backend} rebind exit: {r:?}");

        assert_eq!(
            field(&w, "digest="),
            field(&r, "digest="),
            "{backend}: the rebound store must hold the same data:\n  {w}\n  {r}"
        );
        let content: f64 = field(&w, "content=").parse().expect("content");
        let written: f64 = field(&w, "file=").parse().expect("written");
        let rebound: f64 = field(&r, "file=").parse().expect("rebound");
        // The fixture has to ACQUIRE the slack, or the shrink below proves nothing.
        assert!(
            written / content > 1.5,
            "{backend}: growth must leave slack to reclaim: content={content} written={written}"
        );
        assert!(
            rebound < written * 0.8,
            "{backend}: binding must shed the growth slack: written={written} rebound={rebound}"
        );
        // And it must land near the content, not merely somewhere smaller.
        assert!(
            rebound / content < 1.3,
            "{backend}: a compacted store should be its content plus the format's \
             eighth: content={content} rebound={rebound}"
        );
    }
}

/// loft#729 — a loaded working set is sized by what it HOLDS, not by what the
/// source store's records were grown to.
///
/// A vector that grew in place is left at 7/4 of the size that triggered the
/// growth (`Store::resize`) and never gives it back, so the file carries it. A
/// loader claiming `record_words` reproduced it exactly. Compaction cannot see
/// it — it reclaims free space BETWEEN records, and this is slack INSIDE one, so
/// a store full of it correctly reports as dense.
///
/// The digest is asserted alongside the sizes, and that pairing is the point: a
/// working set that got smaller by LOSING data satisfies every size assertion
/// anyone would write.
#[test]
fn a_loaded_working_set_does_not_inherit_the_source_growth_slack() {
    let dir = scratch("store_load_density_729");
    let path = dir.join("d.store");
    let script = workspace_root().join("tests/scripts/store_load_density_729.loft");

    let field = |out: &str, key: &str| -> String {
        out.split_whitespace()
            .find_map(|t| t.strip_prefix(key).map(str::to_string))
            .unwrap_or_default()
    };

    for backend in ["--interpret", "--native"] {
        // Each backend starts from NO file. Binding an existing one compacts it
        // (loft#730), so a second backend writing over the first backend's file
        // would measure an already-dense store and the slack precondition below
        // would fail for a reason that has nothing to do with the loader.
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("store.dschema"));
        let _ = fs::remove_file(format!("{}.loaded", path.display()));
        let _ = fs::remove_file(format!("{}.loaded.dschema", path.display()));
        let (w, cw) = run_mode_backend(backend, &script, &path, "write");
        assert_eq!(cw, 0, "{backend} write exit: {w:?}");
        let (l, cl) = run_mode_backend(backend, &script, &path, "load");
        assert_eq!(cl, 0, "{backend} load exit: {l:?}");

        assert_eq!(
            field(&w, "digest="),
            field(&l, "digest="),
            "{backend}: the loaded set must hold the same data, or the sizes compare nothing:\n  \
             {w}\n  {l}"
        );
        let content: f64 = field(&w, "content=").parse().expect("content");
        let src: f64 = field(&w, "src=").parse().expect("src");
        let dst: f64 = field(&l, "dst=").parse().expect("dst");
        // The source is what growth leaves behind — well above its content.
        assert!(
            src / content > 1.5,
            "{backend}: the fixture must actually ACQUIRE slack, or this test proves \
             nothing: content={content} src={src}"
        );
        // The load must not carry it over.
        assert!(
            dst < src * 0.8,
            "{backend}: the loaded set inherited the source's growth slack: \
             src={src} dst={dst}"
        );
    }
}

/// loft#729 — the loader reads a record's body in ONE fetch, and that must not
/// change a single loaded byte.
///
/// It used to read word by word: one `resolve` per 4 bytes, which on a real
/// 162 MB store made 1 073 065 requests for one 42-key viewport and 101 of every
/// other size. `LOFT_LOADER_WORDWISE` keeps that path so the two can be compared
/// in ONE binary — a build-to-build comparison would fold in every other change,
/// and the obvious oracle (persist both and diff the files) is blind here:
/// `store_persist_bind` is not byte-deterministic run to run, so the same path
/// twice already differs. The content is what must agree, so the content is what
/// this reads.
#[test]
fn a_bulk_record_read_loads_exactly_what_the_word_at_a_time_read_did() {
    let dir = scratch("store_load_bulk_729");
    let path = dir.join("vs.store");

    let (out_w, code_w) = run_mode(&vecstruct_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (bulk, c1) = run_mode_backend(backend, &vecstruct_script(), &path, "loadkey");
        assert_eq!(c1, 0, "{backend} bulk exit: {bulk:?}");
        let wordwise = {
            let out = Command::new(loft_bin())
                .arg(backend)
                .arg(vecstruct_script())
                .env("LOFT_PERSIST_TEST_PATH", &path)
                .env("LOFT_PERSIST_TEST_MODE", "loadkey")
                .env("LOFT_LOADER_WORDWISE", "1")
                .current_dir(workspace_root())
                .output()
                .expect("failed to invoke loft binary");
            assert!(out.status.success(), "{backend} wordwise exit");
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        assert_eq!(
            bulk, wordwise,
            "{backend}: the bulk read must load exactly what the word-at-a-time read did"
        );
        // The control: an oracle that reports agreement without reading anything
        // would pass this too. These pin the values, so the comparison is between
        // two CORRECT reads rather than two empty ones.
        assert!(
            bulk.contains("vs e0=10,ten") && bulk.contains("vs e2=30,thirty"),
            "{backend}: relocated vector<struct> elements must read correctly: {bulk:?}"
        );
        assert!(
            bulk.contains("vs verify=true"),
            "{backend}: the loaded heap must be sound: {bulk:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.2 — a hash whose entry has a `text` field is now
/// partially loadable: `store_load_key` relocates the source string sub-record
/// into the local store and repoints the field. The loaded entry reads the right
/// string, un-requested keys are absent, and the result is structurally sound
/// (`store_verify`). Both backends.
#[test]
fn store_load_key_relocates_a_text_field_both_backends() {
    let dir = scratch("store_load_text");
    let path = dir.join("text.store");

    let (out_w, code_w) = run_mode(&text_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &text_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("text loadkey=true"), "{backend}: {out:?}");
        assert!(
            out.contains("text name13=thirteen-longer-string-spanning"),
            "{backend}: the relocated text must read correctly: {out:?}"
        );
        assert!(
            out.contains("text name7=ABSENT"),
            "{backend}: un-requested key must be absent: {out:?}"
        );
        assert!(out.contains("text len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("text verify=true"),
            "{backend}: the relocated heap must be sound (no dangling text ptr): {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.3 — a hash whose entry has a `vector<integer>` field is
/// now partially loadable: `store_load_key` relocates the vector's flat inner
/// record. The loaded entry reads the right elements and is structurally sound.
/// Both backends.
#[test]
fn store_load_key_relocates_a_vector_field_both_backends() {
    let dir = scratch("store_load_vec");
    let path = dir.join("vec.store");

    let (out_w, code_w) = run_mode(&vec_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &vec_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("vec loadkey=true"), "{backend}: {out:?}");
        assert!(
            out.contains("vec tags_len=3"),
            "{backend}: the relocated vector length must be right: {out:?}"
        );
        assert!(
            out.contains("vec tags=10,20,30"),
            "{backend}: the relocated vector elements must be right: {out:?}"
        );
        assert!(out.contains("vec len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("vec verify=true"),
            "{backend}: the relocated heap must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.4 — a hash whose entry has an INLINE nested struct field
/// (`sub: Inner`, itself holding a text) is now partially loadable: `load_one`
/// relocates the nested heap fields at their NESTED offsets. The loaded entry
/// reads the nested scalar and text, and is structurally sound. Both backends.
#[test]
fn store_load_key_relocates_a_nested_struct_both_backends() {
    let dir = scratch("store_load_nested");
    let path = dir.join("nest.store");

    let (out_w, code_w) = run_mode(&nested_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &nested_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("nest loadkey=true"), "{backend}: {out:?}");
        assert!(
            out.contains("nest sub.a=42"),
            "{backend}: nested scalar must relocate: {out:?}"
        );
        assert!(
            out.contains("nest sub.b=deep-nested-string"),
            "{backend}: nested text must relocate at its nested offset: {out:?}"
        );
        assert!(out.contains("nest len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("nest verify=true"),
            "{backend}: the relocated heap with a nested text must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.4b — a hash whose entry has a `vector<struct>` field
/// (elements carrying their own text) is now partially loadable: `load_one`
/// copies the inner vector record and relocates EACH element's heap fields. Every
/// element reads correctly and the result is sound. Both backends.
#[test]
fn store_load_key_relocates_a_vector_of_struct_both_backends() {
    let dir = scratch("store_load_vecstruct");
    let path = dir.join("vs.store");

    let (out_w, code_w) = run_mode(&vecstruct_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &vecstruct_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("vs loadkey=true"), "{backend}: {out:?}");
        assert!(out.contains("vs items_len=3"), "{backend}: {out:?}");
        assert!(
            out.contains("vs e0=10,ten"),
            "{backend}: element 0 (scalar + relocated text): {out:?}"
        );
        assert!(
            out.contains("vs e2=30,thirty"),
            "{backend}: element 2 (scalar + relocated text): {out:?}"
        );
        assert!(out.contains("vs len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("vs verify=true"),
            "{backend}: the relocated vector<struct> heap must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b (safe-refusal, ongoing) — a `vector<text>` field
/// (elements that are string pointers) is not handled (copying the inner record
/// leaves the element strings dangling), so it must still be REFUSED. The
/// refused-empty result is structurally sound. Both backends.
#[test]
fn store_load_key_refuses_a_vector_of_text_field_both_backends() {
    let dir = scratch("store_load_vectext_refuse");
    let path = dir.join("vt.store");

    let (out_w, code_w) = run_mode(&vectext_refuse_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &vectext_refuse_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(
            out.contains("vt loadkey=false"),
            "{backend}: a vector<text> entry must be REFUSED: {out:?}"
        );
        assert!(out.contains("vt len=0"), "{backend}: {out:?}");
        assert!(
            out.contains("vt verify=true"),
            "{backend}: the refused-empty store must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 5 — the REMOTE fetch. `store_load_key` over an `http://`
/// URL pulls only the ranges the lookup touches from a `Range`-capable server
/// (the #517 shape), producing the SAME bounded, structurally-sound working set
/// as the local-file path — the generic `PageProvider` seam leaves the
/// traversal and relocating copy unchanged. This is the "fetch" in the partial
/// store fetcher. Both backends.
#[test]
fn store_load_key_over_http_range() {
    let dir = scratch("store_load_http");
    let path = dir.join("world.store");

    // Write the store locally (mmap bind), then serve its bytes over HTTP Range.
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "{out_w:?}");
    // Serve the real `.dschema` beside the store so the 3b.5 layout gate does a
    // genuine remote Match (not a fall-through to the absent-sidecar path).
    let sidecar = fs::read(format!("{}.dschema", path.display())).ok();
    let url = serve_ranges(fs::read(&path).unwrap(), sidecar);

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &load_script(), Path::new(&url), "loadkey");
        assert_eq!(code, 0, "{backend} http loadkey exit: {out:?}");
        assert!(
            out.contains("loadkey keys=13"),
            "{backend}: remote fetch must load key 13: {out:?}"
        );
        assert!(
            out.contains("loadkey lookup h[13]=1300"),
            "{backend}: remote fetch must read the right value: {out:?}"
        );
        assert!(
            out.contains("loadkey len=1"),
            "{backend}: remote fetch is bounded to the working set: {out:?}"
        );
        assert!(
            out.contains("loadkey verify=true"),
            "{backend}: the working set fetched over http must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.6 — a TEXT-keyed hash (`hash<Place[name:text]>`) is
/// partially loadable by string key: `store_load_key_text` finds by hashing the
/// Content::Str and comparing the entry's text key over the reader. The loaded
/// entry reads correctly, un-requested keys are absent, and it's sound. Both
/// backends.
#[test]
fn store_load_key_text_both_backends() {
    let dir = scratch("store_load_textkey");
    let path = dir.join("places.store");

    let (out_w, code_w) = run_mode(&textkey_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write ok"), "{out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &textkey_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("tk loadkey=true"), "{backend}: {out:?}");
        assert!(
            out.contains("tk berlin=3700000"),
            "{backend}: the text-keyed entry must load: {out:?}"
        );
        assert!(
            out.contains("tk amsterdam=-1"),
            "{backend}: un-requested key absent: {out:?}"
        );
        assert!(out.contains("tk len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("tk verify=true"),
            "{backend}: text-keyed working set must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 4 / 3b.7 — `store_load_range` over a `sorted<Tile[id]>`
/// fetches the [13,42] range (13,20,42) in key order, relocates each element's
/// text, leaves out-of-range keys absent, and produces a sound sorted collection.
/// Both backends. This is routing's tile-window fetch.
#[test]
fn store_load_range_over_sorted_both_backends() {
    let dir = scratch("store_load_range");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&range_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write ok"), "{out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &range_script(), &path, "range");
        assert_eq!(code, 0, "{backend} range exit: {out:?}");
        assert!(out.contains("rng loaded=3"), "{backend}: {out:?}");
        assert!(
            out.contains("rng keys=13,20,42"),
            "{backend}: the range must load in key order: {out:?}"
        );
        assert!(
            out.contains("rng name20=twenty"),
            "{backend}: element text must relocate: {out:?}"
        );
        assert!(
            out.contains("rng g7=absent"),
            "{backend}: out-of-range key absent: {out:?}"
        );
        assert!(out.contains("rng len=3"), "{backend}: {out:?}");
        assert!(
            out.contains("rng verify=true"),
            "{backend}: the loaded sorted collection must be sound: {out:?}"
        );
    }
}

/// @PLN97 3b.5 — the layout-identity gate (local file). Persisting writes a
/// `.dschema` sidecar; a working-set load whose Tile layout MATCHES the
/// persisted one proceeds, but a load with an EXTRA field is REFUSED before
/// reading any foreign-layout bytes (leaving the collection empty, not
/// corrupt). Both backends. The sound gate behind trusting a partial store.
#[test]
fn layout_gate_rejects_changed_struct_both_backends() {
    let dir = scratch("layout_gate");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&gate_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write ok"), "{out_w:?}");
    assert!(
        Path::new(&format!("{}.dschema", path.display())).exists(),
        "persist must write a .dschema sidecar beside the store"
    );

    for backend in ["--interpret", "--native"] {
        // MATCHING layout — the load proceeds.
        let (out, code) = run_mode_backend(backend, &gate_script(), &path, "load");
        assert_eq!(code, 0, "{backend} match exit: {out:?}");
        assert!(
            out.contains("gate ok=true"),
            "{backend}: a matching layout must load: {out:?}"
        );
        assert!(out.contains("gate name=forty-two"), "{backend}: {out:?}");
        assert!(out.contains("gate len=1"), "{backend}: {out:?}");

        // CHANGED layout (extra field) — the gate refuses.
        let (out, code) = run_mode_backend(backend, &gate_changed_script(), &path, "load");
        assert_eq!(code, 0, "{backend} mismatch exit: {out:?}");
        assert!(
            out.contains("changed ok=false"),
            "{backend}: a changed layout MUST be refused, not read raw: {out:?}"
        );
        assert!(
            out.contains("changed len=0"),
            "{backend}: a refused load leaves the collection empty: {out:?}"
        );
    }
}

/// loft#700 — the layout gate on the WHOLE-IMAGE loader, `store_load`.
///
/// It kept the target slot's type and reinterpreted the file's bytes through it, so a
/// struct that had GROWN a field read every older record at the new, larger stride:
/// `len()` on the added collection returned wild values and iterating one read arbitrary
/// memory, silently, on both backends. The `.dschema` sidecar already recorded the
/// layout the file was written with and the paged loaders already gated on it — this
/// pins the same gate on the whole-image path.
///
/// The matching half is what keeps the check honest: a gate that refused everything
/// would pass the mismatch assertion on its own.
#[test]
fn store_load_refuses_a_changed_layout_both_backends() {
    let dir = scratch("layout_gate_whole");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&gate_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        // MATCHING layout — the whole-image load proceeds and yields the data.
        let (out, code) = run_mode_backend(backend, &gate_script(), &path, "whole");
        assert_eq!(code, 0, "{backend} match exit: {out:?}");
        assert!(
            out.contains("gate whole ok=true"),
            "{backend}: an unchanged layout must still load whole: {out:?}"
        );
        assert!(out.contains("gate whole len=2"), "{backend}: {out:?}");
        assert!(
            out.contains("gate whole name=forty-two"),
            "{backend}: {out:?}"
        );

        // CHANGED layout (extra field) — refused, rather than read at the wrong stride.
        let (out, code) = run_mode_backend(backend, &gate_changed_script(), &path, "whole");
        assert_eq!(code, 0, "{backend} mismatch exit: {out:?}");
        assert!(
            out.contains("changed whole ok=false"),
            "{backend}: a changed layout MUST be refused, not read raw: {out:?}"
        );
        assert!(
            out.contains("changed whole len=0"),
            "{backend}: a refused load leaves the collection empty: {out:?}"
        );
    }
}

/// @PLN97 3b.5 — the layout-identity gate over HTTP: the remote loader fetches
/// `<url>.dschema` and rejects a mismatched layout, never range-reading foreign
/// bytes across the network (the safety gate for a REMOTE store read, #522).
/// A matching layout still loads. Both backends.
#[test]
fn layout_gate_over_http_rejects_changed_struct() {
    let dir = scratch("layout_gate_http");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&gate_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    let sidecar = fs::read(format!("{}.dschema", path.display())).ok();
    assert!(sidecar.is_some(), "persist must write a .dschema sidecar");
    let url = serve_ranges(fs::read(&path).unwrap(), sidecar);

    for backend in ["--interpret", "--native"] {
        // MATCHING layout over http — the remote gate does a real Match.
        let (out, code) = run_mode_backend(backend, &gate_script(), Path::new(&url), "load");
        assert_eq!(code, 0, "{backend} http match exit: {out:?}");
        assert!(
            out.contains("gate ok=true"),
            "{backend}: a matching remote layout must load: {out:?}"
        );

        // CHANGED layout over http — the remote gate refuses.
        let (out, code) =
            run_mode_backend(backend, &gate_changed_script(), Path::new(&url), "load");
        assert_eq!(code, 0, "{backend} http mismatch exit: {out:?}");
        assert!(
            out.contains("changed ok=false"),
            "{backend}: a changed layout MUST be refused over http: {out:?}"
        );
        assert!(out.contains("changed len=0"), "{backend}: {out:?}");
    }
}

/// @PLN97 arc G Phase 0 — `store_load_url` fetches a WHOLE persisted store IMAGE
/// from a URL and adopts it ONLY after its SHA-256 matches the caller-pinned
/// digest (the registry's fetch→verify→trust discipline bridged onto the store
/// loader).  A correct hash loads every key + is structurally sound; a WRONG hash
/// is REFUSED (returns false, adopts nothing — the collection stays empty).  Both
/// backends.  A `file://` URL is used so the test needs no network (the loader's
/// `http_get_bytes` treats `file://` the same as an HTTP GET).
#[cfg(feature = "registry")]
#[test]
fn store_load_url_verifies_sha_before_adopting_both_backends() {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    let dir = scratch("store_load_url_phase0");
    let path = dir.join("world.store");

    // Write the image once (mmap bind), then compute its SHA-256 in the harness.
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "write: {out_w:?}");

    let bytes = fs::read(&path).unwrap();
    let sha = {
        let mut h = Sha256::new();
        h.update(&bytes);
        let mut s = String::with_capacity(64);
        for b in h.finalize() {
            let _ = write!(s, "{b:02x}");
        }
        s
    };
    let url = format!("file://{}", path.display());
    let wrong = "0".repeat(64);

    let run = |backend: &str, sha_arg: &str| -> (String, i32) {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(load_script())
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_PERSIST_TEST_MODE", "loadurl")
            .env("LOFT_PERSIST_TEST_URL", &url)
            .env("LOFT_PERSIST_TEST_SHA", sha_arg)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        if !out.status.success() {
            eprintln!(
                "{backend} stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    };

    for backend in ["--interpret", "--native"] {
        // Correct hash — the load succeeds, all keys present, heap sound.
        let (out, code) = run(backend, &sha);
        assert_eq!(code, 0, "{backend} verified load exit: {out:?}");
        assert!(
            out.contains("loadurl ok=true"),
            "{backend}: a SHA-matched image must load: {out:?}"
        );
        assert!(
            out.contains("loadurl keys=7,13,42"),
            "{backend}: all keys must be present after a verified load: {out:?}"
        );
        assert!(
            out.contains("loadurl lookup h[13]=1300"),
            "{backend}: key lookup must read the right value: {out:?}"
        );
        assert!(
            out.contains("loadurl verify=true"),
            "{backend}: the adopted store must be structurally sound: {out:?}"
        );

        // Wrong hash — REFUSED before adopting: nothing loaded.
        let (out, code) = run(backend, &wrong);
        assert_eq!(code, 0, "{backend} tampered load exit: {out:?}");
        assert!(
            out.contains("loadurl ok=false"),
            "{backend}: a SHA MISMATCH must refuse the load: {out:?}"
        );
        assert!(
            out.contains("loadurl lookup h[13]=null"),
            "{backend}: a refused load must adopt NOTHING (h stays empty): {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 2 — `store_load_untrusted` reads a local store IMAGE that
/// may be untrusted and adopts it ONLY after `validate_structure` passes: a
/// well-formed image loads every key + is sound; a garbage file is rejected
/// (`false`) cleanly — never a hang or a misread. Both backends.
#[test]
fn store_load_untrusted_validates_before_adopting_both_backends() {
    let dir = scratch("store_load_untrusted");
    let path = dir.join("u.store");

    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_mode_backend(backend, &load_script(), &path, "loaduntrusted");
        assert_eq!(code, 0, "{backend} loaduntrusted exit: {out:?}");
        assert!(
            out.contains("untrusted keys=7,13,42"),
            "{backend}: a valid image must load all keys: {out:?}"
        );
        assert!(
            out.contains("untrusted lookup h[13]=1300"),
            "{backend}: key lookup must be correct: {out:?}"
        );
        assert!(
            out.contains("untrusted verify=true"),
            "{backend}: the adopted store must be structurally sound: {out:?}"
        );
    }

    // A garbage file must reject cleanly (false), not hang or misread.
    let garbage = dir.join("garbage.store");
    fs::write(
        &garbage,
        b"not a loft store image, just junk bytes right here!!",
    )
    .unwrap();
    let (out, code) = run_mode(&load_script(), &garbage, "loaduntrusted");
    assert_eq!(code, 0, "garbage reject should be graceful: {out:?}");
    assert!(
        out.contains("FAIL untrusted-load-returned-false"),
        "a garbage file must reject cleanly (store_load_untrusted=false): {out:?}"
    );
}

/// @PLN97 arc G Phase 0 — `store_load_url_trusted` fetches a whole store IMAGE
/// over HTTP from a TRUSTED source (no SHA pin) and adopts it — the instant read,
/// still structurally validated. Both backends, over a real HTTP server.
#[test]
fn store_load_url_trusted_fetches_over_http_both_backends() {
    let dir = scratch("store_load_url_trusted");
    let path = dir.join("t.store");

    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "write: {out_w:?}");

    // Serve the store over HTTP (a plain GET → 200 whole file).
    let url = serve_ranges(fs::read(&path).unwrap(), None);

    for backend in ["--interpret", "--native"] {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(load_script())
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_PERSIST_TEST_MODE", "loadurltrusted")
            .env("LOFT_PERSIST_TEST_URL", &url)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let so = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exit: {so} / {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            so.contains("urltrusted keys=7,13,42"),
            "{backend}: a trusted HTTP fetch must load all keys: {so}"
        );
        assert!(
            so.contains("urltrusted lookup h[13]=1300"),
            "{backend}: key lookup must be correct: {so}"
        );
        assert!(
            so.contains("urltrusted verify=true"),
            "{backend}: the fetched store must be structurally sound: {so}"
        );
    }
}

#[test]
fn fresh_returns_true_and_file_appears() {
    let dir = scratch("fresh_returns_true_and_file_appears");
    let path = dir.join("only_fresh.store");

    let (out, code) = run_smoke(&path, "fresh");
    assert_eq!(code, 0, "{out:?}");
    assert!(!out.contains("FAIL"), "fresh should not fail; got: {out:?}");
    assert!(path.exists(), "bind should create the file on first call");
}

#[test]
fn reload_on_missing_file_returns_true_with_empty_view() {
    // When the path doesn't exist at reload time either, the binding
    // takes the "fresh" branch internally (snapshot-empty-hash → write
    // → mmap).  The script should still emit a non-FAIL line, and the
    // file should appear afterwards even though it was a reload-mode
    // invocation.
    let dir = scratch("reload_on_missing_file");
    let path = dir.join("never_existed.store");
    assert!(!path.exists());

    let (out, code) = run_smoke(&path, "reload");
    assert_eq!(code, 0, "{out:?}");
    assert!(
        !out.contains("FAIL"),
        "reload on missing path should still bind: {out:?}"
    );
    assert!(path.exists(), "bind should create the file");
}

/// `run_mode` that also hands back stderr — the refusal channel the paged
/// loaders warn on.
fn run_mode_with_stderr(script: &Path, path: &Path, mode: &str) -> (String, String, i32) {
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// #632 — a paged load that REFUSES must say so. `store_load_key` and friends
/// report failure as `false` / `0`, which is indistinguishable from "that key is
/// absent", so a silent refusal reads as missing data and hides an unsupported
/// shape. The layout gate already warns for exactly this reason; every other
/// refusal now routes through the same channel.
///
/// #632 part 2 — a HASH declared as a struct field is now paged-loadable. The
/// bound store records the WRAPPER STRUCT as its type; the loader descends to the
/// keyed-collection field and loads it. The reported failure ("silently return
/// nothing for a collection declared as a struct field") is gone: the key loads,
/// the value is right, and the copied heap verifies.
#[test]
fn paged_load_accepts_a_hash_declared_as_a_struct_field() {
    let dir = scratch("paged_field_accept");
    let path = dir.join("field.store");
    let script = workspace_root().join("tests/scripts/store_load_field_refusal.loft");

    let (w_out, _, w_code) = run_mode_with_stderr(&script, &path, "write");
    assert_eq!(w_code, 0, "{w_out:?}");
    assert!(w_out.contains("write ok"), "write failed: {w_out:?}");

    let (out, err, code) = run_mode_with_stderr(&script, &path, "loadkey");
    assert_eq!(code, 0, "{out:?}\n{err:?}");
    assert!(
        out.contains("field loadkey=true len=1 n=7717 verify=true"),
        "hash-as-field must load the key with the right value + sound heap: {out:?}"
    );
    assert!(
        !err.contains("refusing"),
        "the hash field form must no longer refuse: {err:?}"
    );
}

/// #632 — a SORTED collection declared as a struct field still refuses (a
/// promoted/linked `ordered` field's elements sit behind an indirection the
/// positional range reader can't follow — @PLN97 arc G). What this pins is that
/// the refusal is AUDIBLE, never a silent `0` indistinguishable from "key absent".
#[test]
fn paged_refusal_on_sorted_field_is_audible() {
    let dir = scratch("paged_sorted_field_refusal");
    let path = dir.join("sfield.store");
    let script = workspace_root().join("tests/scripts/store_load_field_refusal.loft");

    let (w_out, _, w_code) = run_mode_with_stderr(&script, &path, "write_sorted");
    assert_eq!(w_code, 0, "{w_out:?}");
    assert!(w_out.contains("write ok"), "write failed: {w_out:?}");

    let (out, err, code) = run_mode_with_stderr(&script, &path, "loadrange_sorted");
    assert_eq!(code, 0, "{out:?}");
    assert!(out.contains("sorted loadrange=0 len=0"), "{out:?}");
    assert!(
        err.contains("refusing") && err.contains("not an absent key"),
        "a refusal must be reported, not silent; stderr was: {err:?}"
    );
    assert!(
        err.contains("SWrap") && err.contains("annotated local"),
        "the warning must name the wrapper type and the fix; stderr was: {err:?}"
    );
}

// ── loft#710 — the persisted file's size is a fact about its CONTENT ─────────

/// Build the size-probe store and return `(file bytes, digest line)`.
///
/// `seed` fixes `LOFT_HASH_SEED` when given, which is what makes a build
/// byte-reproducible.
fn persist_size(test: &str, n: u32, per: u32, mode: &str, seed: Option<&str>) -> (u64, String) {
    let dir = scratch(test);
    let path = dir.join("size.store");
    let script = workspace_root().join("tests/scripts/store_persist_size_710.loft");
    let mut cmd = Command::new(loft_bin());
    cmd.arg("--interpret")
        .arg(&script)
        .env("LOFT_PERSIST_TEST_PATH", &path)
        .env("LOFT_PERSIST_TEST_N", n.to_string())
        .env("LOFT_PERSIST_TEST_PER", per.to_string())
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root());
    if let Some(s) = seed {
        cmd.env("LOFT_HASH_SEED", s);
    } else {
        cmd.env_remove("LOFT_HASH_SEED");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{mode} failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("persist true"), "{mode}: {stdout:?}");
    let bytes = fs::metadata(&path).expect("store file").len();
    (bytes, stdout.trim().to_string())
}

/// loft#710 — a persisted store's size must follow what it HOLDS.
///
/// It did not: the image was the arena's whole capacity, so the file carried
/// whatever the last 7/3 growth over-allocated.  Two consequences, both
/// measured here at the shapes the report used.
///
/// 1. The size did not move with the content.  125 records x 1000 coordinates
///    and 125 x 2312 — 2.3x the data — persisted to the SAME 3,816,152 bytes,
///    so a consumer sizing its hosting from file bytes was reading the
///    allocator, and a block that doubled because a category was added looked
///    exactly like one that doubled by allocation.
///
/// 2. At the reported shape, construction order alone decided the size: filling
///    each vector whole before inserting gave 1.84x what growing them
///    interleaved gave, for byte-identical data.  Interleaved is the shape that
///    matters — it is what a streaming generator does.
///
/// What this does NOT claim: that construction order stops mattering.  Once the
/// capacity rounding is gone, the INTERIOR free space each order leaves is
/// visible, and it genuinely differs.  Reclaiming that means relocating records
/// and rewriting every DbRef — compaction, which loft#710 still asks for.
#[test]
fn persisted_size_tracks_content_not_construction() {
    let (small, _) = persist_size("size710_small", 125, 1000, "interleaved", None);
    let (large, _) = persist_size("size710_large", 125, 2312, "interleaved", None);
    let growth = large as f64 / small as f64;
    assert!(
        growth > 1.4,
        "2.3x the coordinates must show in the file (both were 3,816,152 bytes): \
         small={small} large={large} ratio={growth:.2}"
    );

    let (whole, d_whole) = persist_size("size710_whole", 125, 2312, "whole", None);
    let (inter, d_inter) = persist_size("size710_interleaved", 125, 2312, "interleaved", None);
    let digest = |line: &str| line.split("digest").nth(1).unwrap_or("").trim().to_string();
    assert_eq!(
        digest(&d_whole),
        digest(&d_inter),
        "the two orders must hold identical data, or the sizes below compare nothing:\n  {d_whole}\n  {d_inter}"
    );
    let ratio = whole.max(inter) as f64 / whole.min(inter) as f64;
    assert!(
        ratio < 1.3,
        "construction order decided the size 1.84x at this shape; it must not: \
         whole={whole} interleaved={inter} ratio={ratio:.2}"
    );
}

/// @PLN123 A3 — `store_reclaim(collection)` hands a BOUND store's file back.
///
/// The shape the plan exists for: bind small, grow ten-fold, drop back to the
/// original live set.  A bound store only ever grew — `resize_store` returns
/// early on any request at or below its size — so the file stayed at its peak
/// for the rest of the program's life.
///
/// Every size assertion here is paired with the digest the script prints, and
/// that pairing is the point: a file that got smaller by LOSING data satisfies
/// every size assertion anyone would write.  Both backends, because the call
/// runs through the interpreter's registry entry on one and the `#rust`
/// template on the other — two implementations of one answer.
#[test]
fn store_reclaim_shrinks_a_bound_file_both_backends() {
    let script = workspace_root().join("tests/scripts/store_reclaim_123.loft");
    let field = |out: &str, line: &str, key: &str| -> i64 {
        let l = out
            .lines()
            .find(|l| l.starts_with(line))
            .unwrap_or_else(|| panic!("no `{line}` line in:\n{out}"));
        let mut it = l.split_whitespace();
        while let Some(t) = it.next() {
            if t == key {
                return it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| panic!("`{key}` is not a number in `{l}`"));
            }
        }
        panic!("no `{key}` in `{l}`");
    };

    for backend in ["--interpret", "--native"] {
        let dir = scratch(&format!("reclaim123_{}", backend.trim_start_matches('-')));
        let path = dir.join("world.store");
        let (out, code) = run_script(&script, backend, &path);
        assert_eq!(code, 0, "{backend} exit: {out:?}");

        let before = field(&out, "grown_then_dropped", "before");
        let freed = field(&out, "grown_then_dropped", "freed");
        let after = field(&out, "grown_then_dropped", "after");
        assert!(
            freed > 0,
            "{backend}: a store grown 10x and dropped back has a tail to give: {out}"
        );
        assert_eq!(
            after,
            before - freed,
            "{backend}: the bytes it REPORTS must be the bytes the file lost — \
             a reclaim that misreports is worse than one that does nothing: {out}"
        );
        assert_eq!(
            fs::metadata(&path).expect("store file").len() as i64,
            field(&out, "second", "size"),
            "{backend}: the file on disk is the size the program saw"
        );

        // The data is untouched: same live count, same digest as at bind time.
        assert_eq!(
            field(&out, "kept", "digest"),
            field(&out, "kept", "was"),
            "{backend}: the surviving records must be bit-for-bit what they were \
             before the truncation, or the size numbers above compare nothing: {out}"
        );
        assert_eq!(field(&out, "kept", "live"), 300, "{backend}: {out}");

        // Nothing left to give, and saying so costs the file nothing.
        assert_eq!(
            field(&out, "second", "freed"),
            0,
            "{backend}: a second reclaim has no tail left: {out}"
        );
        assert_eq!(
            field(&out, "second", "size"),
            after,
            "{backend}: and it must not touch the file to find that out: {out}"
        );

        // Still a working store afterwards.
        assert_eq!(field(&out, "after_write", "live"), 301, "{backend}: {out}");
        assert!(
            out.contains("new written after the truncation") && out.contains("old hex 7"),
            "{backend}: a truncated store still takes records and still reads the \
             ones that survived: {out}"
        );

        // A shortened file is only a store if it still OPENS as one.  A fresh
        // process binds to what the first left behind: same count, same digest.
        // Nothing in the writing process could have caught a truncation that
        // left the block chain unable to tile the file.
        let (re, re_code) = run_mode_backend(backend, &script, &path, "reload");
        assert_eq!(re_code, 0, "{backend} reload exit: {re:?}");
        assert_eq!(
            field(&re, "reload", "live"),
            301,
            "{backend}: the re-opened file holds every record: {re}"
        );
        assert_eq!(
            field(&re, "reload", "digest"),
            field(&out, "after_write", "digest"),
            "{backend}: and holds them unchanged — a truncated file re-read in a \
             fresh process:\n  wrote:  {out}\n  reread: {re}"
        );
    }
}

/// One `count/sum/xor/sound` line from `store_digest_b0.loft`.
#[derive(PartialEq, Eq, Debug, Clone)]
struct B0 {
    count: i64,
    sum: i64,
    xor: i64,
    sound: bool,
}

/// Run the B0 oracle script and read back one tagged report line.
fn b0_run(
    backend: &str,
    path: &Path,
    order: &str,
    damage: &str,
    mode: &str,
    tag: &str,
    seed: Option<&str>,
) -> B0 {
    let script = workspace_root().join("tests/scripts/store_digest_b0.loft");
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(&script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .env("B0_ORDER", order)
        .env("B0_DAMAGE", damage)
        .env("B0_N", "200")
        .current_dir(workspace_root());
    match seed {
        Some(s) => cmd.env("LOFT_HASH_SEED", s),
        None => cmd.env_remove("LOFT_HASH_SEED"),
    };
    let out = cmd.output().expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{backend} {order}/{damage}/{mode} failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = stdout
        .lines()
        .find(|l| l.starts_with(tag))
        .unwrap_or_else(|| panic!("no `{tag}` line for {backend} {order}/{damage}:\n{stdout}"));
    let num = |key: &str| -> i64 {
        let mut it = line.split_whitespace();
        while let Some(t) = it.next() {
            if t == key {
                return it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                    panic!("`{key}` is not a number in `{line}`");
                });
            }
        }
        panic!("no `{key}` in `{line}`");
    };
    B0 {
        count: num("count"),
        sum: num("sum"),
        xor: num("xor"),
        sound: line.ends_with("sound true"),
    }
}

/// @PLN123 B0 — the oracle arc B will be built against, calibrated BEFORE any
/// compaction code exists.
///
/// Invariant B is that a compacted image holds exactly the records reachable
/// from the root and reloads value-identical. No size assertion can check that:
/// a file that got smaller by losing data satisfies every one of them. So the
/// question here is not "is compaction correct" — there is no compaction yet —
/// but "would this digest KNOW". Two halves:
///
/// **It must see every loss.** Five injuries, one per way a rebuild could drop
/// something: a missing record, a changed scalar, a shortened text, a truncated
/// vector, a changed nested-struct field. Each must move the numbers. An oracle
/// that cannot fail is worse than none, because it reports success.
///
/// **It must be a function of the DATA, not the representation.** A rebuild
/// changes where everything sits, so anything the digest picks up from layout
/// would read as data loss — wrong in the direction that gets a fix applied to
/// working code. The five runs below hold identical records in five different
/// representations (three build orders × two bucket seeds), and the assertion
/// that the resulting FILES differ is what stops "same digest" from meaning
/// "nothing was perturbed".
///
/// **F9 is not a live hazard, and that took establishing.** The step assumed a
/// rebuild reorders iteration, so the digest was written order-independent
/// (sum/xor/count). Probing it with a deliberately order-DEPENDENT fold showed
/// no build order and no seed changes the order records come out in — loft
/// iterates a keyed collection through a key-sorted snapshot, so a rebuild
/// cannot reorder it. The order-independent form stays because it costs
/// nothing and keeps the oracle off that implementation detail; what could
/// not stay was the *assertion*, which no available lever can make fail.
///
/// The loss half earned its place immediately: the first draft truncated the
/// vector of a record whose vector was ALREADY one element, and the digest came
/// back identical. That reads exactly like a blind oracle and was a blind
/// injury — the failure mode this whole test exists to make visible.
#[test]
fn store_digest_b0_oracle_sees_loss_not_layout() {
    for backend in ["--interpret", "--native"] {
        let tag = backend.trim_start_matches('-');
        let dir = scratch(&format!("b0_{tag}"));
        let run = |file: &str, order: &str, damage: &str, mode: &str, want: &str, seed| {
            b0_run(backend, &dir.join(file), order, damage, mode, want, seed)
        };

        // Same 200 records, five representations.  Every run pins the bucket
        // seed, because an unseeded hash draws a RANDOM one per process (the
        // P253 hash-DoS defense) — leave it unpinned and the images differ no
        // matter what else you vary, so the control below would pass while
        // attributing nothing.  Pinned, each row differs from the first by
        // exactly one named lever.
        let reps: [(&str, &str, &str, &str); 5] = [
            ("base.store", "forward", "7", "the reference representation"),
            ("again.store", "forward", "7", "a repeat of it"),
            (
                "reverse.store",
                "reverse",
                "7",
                "records inserted back to front",
            ),
            ("inter.store", "interleaved", "7", "evens then odds"),
            ("seed1.store", "forward", "1", "a different bucket seed"),
        ];
        let mut images: Vec<Vec<u8>> = Vec::new();
        let mut base: Option<B0> = None;
        for (file, order, seed, what) in reps {
            let got = run(file, order, "none", "", "persisted", Some(seed));
            assert!(got.sound, "{backend}: {what} must verify");
            assert!(
                got.count == 200 && got.sum != 0 && got.xor != 0,
                "{backend}: {what} produced an empty digest: {got:?}"
            );
            match &base {
                None => base = Some(got),
                Some(b) => assert_eq!(
                    &got, b,
                    "{backend}: {what} holds the same records — a digest that \
                     moved with the layout would report arc B's rebuild as data loss"
                ),
            }
            images.push(fs::read(dir.join(file)).expect("store file"));
        }
        let base = base.expect("five representations");

        // The controls for the claim above.  Negative: same order, same seed,
        // byte-identical — so the harness is deterministic and the differences
        // below are attributable rather than noise.
        assert_eq!(
            images[0], images[1],
            "{backend}: two identical runs must produce identical bytes, or \
             nothing measured here can be attributed to anything"
        );
        // Positive: each lever really does reach the stored bytes, so the
        // invariance above is a statement about data surviving a CHANGED
        // representation — not about nothing having changed.
        for (i, lever) in [
            (2, "insertion order"),
            (3, "insertion order"),
            (4, "the bucket seed"),
        ] {
            assert_ne!(
                images[0], images[i],
                "{backend}: {lever} did not reach the stored bytes, so the \
                 digest's invariance across it proves nothing"
            );
        }

        // Each injury must move the numbers — the proof the digest can fail.
        for damage in ["drop", "scalar", "text", "vector", "inner"] {
            let hurt = run("damaged.store", "forward", damage, "", "built", None);
            assert_ne!(
                hurt, base,
                "{backend}: a `{damage}` loss is invisible to the digest — an \
                 oracle that cannot fail reports success"
            );
            if damage == "drop" {
                assert_eq!(hurt.count, base.count - 1, "{backend}: a record vanished");
            }
        }

        // Round trip: bind wrote the image above; a FRESH process reads it
        // back, and deliberately UNSEEDED — it draws its own random bucket
        // seed and must still find every record, because the seed the writer
        // used travels in the image.  This is the comparison arc B's compacted
        // image has to pass.
        let reloaded = run("base.store", "forward", "none", "reload", "reload", None);
        assert_eq!(
            reloaded, base,
            "{backend}: the image must reload value-identical"
        );
        assert!(reloaded.sound, "{backend}: and with a sound heap graph");
    }
}

/// @PLN123 B1 — rebuild-and-swap, measured against B0's oracle.
///
/// Arc A gives back the free tail; the interior is what is left, and arc B's
/// answer is to rebuild the collection into a fresh store and swap — the shape
/// `OPTIMIZE TABLE` uses. This runs that rebuild at loft level, record by
/// record, so the cost is known before it is written in Rust.
///
/// Measured (300 → 6,000 surviving records, grown 10× and dropped back):
/// **77–86% of the post-`store_reclaim` file comes back**, at ~0.6 µs/record on
/// `--native` and ~1.0 µs interpreted, linear across a 20× scale range. The
/// step's standing warning — that a record-by-record re-insert produced a
/// LARGER file — does not reproduce for a direct rebuild; it produced one 4–7×
/// smaller.
///
/// What this test pins is the part B2 depends on, not the numbers:
///
/// - **The digest survives every stage.** A smaller file that lost data would
///   satisfy every size assertion here, so B0's oracle rides along.
/// - **The rebuild is IDEMPOTENT.** A second and third rebuild land on the same
///   byte count. This is what makes B2 safe to run automatically at persist: a
///   compaction that grew the file a little every time would be worse than
///   none, and that is the failure this rules out.
/// - **Both backends agree byte for byte**, so the loft-level measurement is a
///   property of the store, not of one execution path.
///
/// A rebuild does NOT reach the from-scratch ceiling — 1.24× at 300 records,
/// 1.38× at 6,000 — and the whole gap is the VECTOR field: shapes with only
/// scalars or only text rebuild to exactly the fresh size, and the rebuilt size
/// is flat across vector lengths 1–9 where a from-scratch build scales with
/// them. So the copy path claims a quantised block per vector where a fresh
/// build claims by length. **B2 should claim each destination vector at its
/// LENGTH**; `reserve` on the source side does not do it (measured: identical
/// bytes with and without).
#[test]
fn store_rebuild_b1_recovers_the_interior_and_is_idempotent() {
    let script = workspace_root().join("tests/scripts/store_rebuild_b1.loft");
    // tag, count, sum, xor, bytes — one row per stage the script reports.
    type Row = (String, i64, i64, i64, i64);
    let mut per_backend: Vec<Vec<Row>> = Vec::new();
    for backend in ["--interpret", "--native"] {
        let dir = scratch(&format!("b1_{}", backend.trim_start_matches('-')));
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("m"))
            .env("B1_SHAPE", "full")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} failed: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // tag count sum xor bytes us
        let rows: Vec<Row> = stdout
            .lines()
            .filter(|l| l.contains(" count ") && l.contains(" bytes "))
            .map(|l| {
                let t: Vec<&str> = l.split_whitespace().collect();
                let num = |key: &str| -> i64 {
                    let i = t.iter().position(|x| *x == key).expect(key);
                    t[i + 1].parse().expect("number")
                };
                (
                    t[0].to_string(),
                    num("count"),
                    num("sum"),
                    num("xor"),
                    num("bytes"),
                )
            })
            .collect();
        let get = |tag: &str| {
            rows.iter()
                .find(|r| r.0 == tag)
                .unwrap_or_else(|| panic!("no `{tag}` row in:\n{stdout}"))
                .clone()
        };
        let frag = get("fragmented");
        let rebuilt = get("rebuilt");
        let again = get("rebuilt2");
        let third = get("rebuilt3");
        let fresh = get("fresh");

        // B0's oracle rides along: every stage holds the same records.
        for row in &rows {
            assert_eq!(
                (row.1, row.2, row.3),
                (frag.1, frag.2, frag.3),
                "{backend}: `{}` lost or changed data, so its size proves nothing:\n{stdout}",
                row.0
            );
        }

        // The interior really does come back.  The bound is generous — the
        // measurement is 77-86% — because this guards the mechanism, not the
        // number; the numbers live in the plan.
        assert!(
            (rebuilt.4 as f64) < 0.5 * frag.4 as f64,
            "{backend}: a rebuild must at least halve the post-reclaim file \
             (got {} from {}):\n{stdout}",
            rebuilt.4,
            frag.4
        );

        // The property B2's automatic mode rests on.
        assert_eq!(
            (rebuilt.4, rebuilt.4),
            (again.4, third.4),
            "{backend}: rebuilding must reach a FIXED POINT — a compaction that \
             grows the file a little each time is worse than none:\n{stdout}"
        );

        // Honest about the ceiling: a rebuild does not reach a from-scratch
        // build, and if it ever does, this is the assertion that says so.
        assert!(
            rebuilt.4 > fresh.4,
            "{backend}: a rebuild now MATCHES the from-scratch ceiling ({} vs \
             {}) — the vector-claim gap B2 was told to close is gone, so update \
             @PLN123 B1 rather than this test:\n{stdout}",
            rebuilt.4,
            fresh.4
        );
        per_backend.push(rows);
    }
    assert_eq!(
        per_backend[0], per_backend[1],
        "the interpreter and --native must agree on every size and digest, or \
         this measures an execution path rather than the store"
    );
}

/// @PLN123 B2/B3 — compaction at LOAD, ON by default
/// (`LOFT_NO_COMPACT_ON_LOAD` opts out).
///
/// Arc A returns the free tail; the interior between surviving records needs the
/// collection rebuilt somewhere dense. That moves records, and a `DbRef` is a
/// POSITION — so it is sound only where an interior reference cannot be live.
/// The step originally specified the WRITE path; that is exactly where they CAN
/// be live (a program keeps `e = h[7]` across `store_persist_bind`, and
/// `bind_path` adopts the image it writes as the live store). The load path is
/// safe for a stronger reason than absence: `Store::load` already replaces the
/// slot's bytes wholesale, so a reference held across it is already meaningless.
///
/// What this pins:
///
/// - **Opting out is exact.** With `LOFT_NO_COMPACT_ON_LOAD` the reloaded file
///   is byte-identical to the source — the pre-B2 behaviour, still reachable.
/// - **On is correct, not merely smaller.** B0's oracle rides along — the digest
///   over every field must be unchanged, and `store_verify` must call the
///   rebuilt heap graph sound. A compaction that lost data would satisfy every
///   size assertion in this test.
/// - **The root survives.** The collection variable is a `DbRef` at the root, so
///   the root is the one position compaction may not move; the script reads an
///   element through it immediately after the load, and keeps using the
///   collection after.
/// - **Refusal is attributable and safe.** A record holding a `reference<T>`
///   points into ANOTHER store, which this rebuild does not carry — compaction
///   must decline, say so, and leave a correct load behind. A refusal that reads
///   the same as a dense store is one nobody can test.
#[test]
fn store_compact_b2_rebuilds_at_load_without_losing_anything() {
    let script = workspace_root().join("tests/scripts/store_compact_b2.loft");
    let run = |backend: &str, dir: &Path, shape: &str, compact: bool| -> String {
        let mut cmd = Command::new(loft_bin());
        cmd.arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("c"))
            .env("B2_SHAPE", shape)
            .env("LOFT_LOADER_STATS", "1")
            .current_dir(workspace_root());
        if compact {
            cmd.env_remove("LOFT_NO_COMPACT_ON_LOAD"); // @PLN123 B3 — on by default
        } else {
            cmd.env("LOFT_NO_COMPACT_ON_LOAD", "1");
        }
        let out = cmd.output().expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success(),
            "{backend} {shape} compact={compact} failed: {stdout}\n{stderr}"
        );
        assert!(!stdout.contains("FAIL"), "{backend} {shape}: {stdout}");
        format!("{stdout}{stderr}")
    };
    let field = |out: &str, line: &str, key: &str| -> i64 {
        let l = out
            .lines()
            .find(|l| l.starts_with(line))
            .unwrap_or_else(|| panic!("no `{line}` line in:\n{out}"));
        let t: Vec<&str> = l.split_whitespace().collect();
        let i = t.iter().position(|x| *x == key).expect(key);
        t[i + 1].parse().expect("number")
    };

    for backend in ["--interpret", "--native"] {
        let tag = backend.trim_start_matches('-');

        // OPTED OUT — the load is a wholesale byte replacement, as it always was.
        let off = run(backend, &scratch(&format!("b2off_{tag}")), "rich", false);
        assert_eq!(
            field(&off, "loaded", "bytes"),
            field(&off, "source", "bytes"),
            "{backend}: with the flag off the reloaded store must be the source, \
             byte for byte:\n{off}"
        );

        // ON — smaller, and the same data.
        let on = run(backend, &scratch(&format!("b2on_{tag}")), "rich", true);
        let src = field(&on, "source", "bytes");
        let out = field(&on, "loaded", "bytes");
        assert!(
            out * 2 < src,
            "{backend}: compaction must at least halve a store whose live set \
             fell to a tenth (got {out} from {src}):\n{on}"
        );
        for key in ["count", "sum", "xor"] {
            assert_eq!(
                field(&on, "source", key),
                field(&on, "loaded", key),
                "{backend}: `{key}` changed across the rebuild — the file got \
                 smaller by LOSING data, which every size assertion above would \
                 have accepted:\n{on}"
            );
        }
        assert!(
            on.lines()
                .filter(|l| l.starts_with("loaded"))
                .all(|l| l.ends_with("sound true")),
            "{backend}: the rebuilt heap graph must verify:\n{on}"
        );
        assert!(
            on.contains("store_load: compacted"),
            "{backend}: compaction must actually have run, or every assertion \
             above passed on an untouched store:\n{on}"
        );
        // The root reference survived, and the store still works afterwards.
        assert!(
            on.contains("root_ref label label-7") && on.contains("old label-13"),
            "{backend}: an element read through the root after the rebuild, and \
             the store still takes writes:\n{on}"
        );

        // REFUSAL — a cross-store `reference<T>` is a shape the rebuild declines.
        let refused = run(backend, &scratch(&format!("b2ref_{tag}")), "refuse", true);
        assert!(
            refused.contains("store_load: not compacted") && refused.contains("cannot carry"),
            "{backend}: a collection holding a reference into another store must \
             be declined, with the reason said out loud:\n{refused}"
        );
        assert!(
            refused.contains("refuse count 30") && refused.contains("sound true"),
            "{backend}: and a declined compaction must leave a correct load \
             behind:\n{refused}"
        );
    }
}

/// @PLN123 B3 — a BOUND store's file follows what it holds, across runs.
///
/// The case the plan opened with: `resize_store` refuses to shrink, so a bound
/// store only ever grew — a collection that peaked at 2,000 records and settled
/// at 200 kept the peak-sized file for the rest of its life, and for every life
/// after, because the next run mapped the same file. Arc A takes the tail when
/// the program asks; this is the interior, taken automatically at the one moment
/// records may move.
///
/// Compaction here runs on `bind_path`'s EXISTING-file branch, which is a LOAD —
/// the collection was declared empty, so no interior `DbRef` can be live. The
/// fresh-file branch is a WRITE and is deliberately untouched (§ B2).
///
/// The assertions that matter beyond size: the digest over every field is
/// unchanged, `store_verify` calls the rebuilt graph sound, the root reference
/// reads after the rebuild, and the collection is **still bound** — writing to it
/// must still reach the file, or compaction would have quietly turned a
/// persisted collection into a heap copy that stops persisting.
#[test]
fn store_compact_b3_shrinks_a_bound_file_across_runs() {
    let script = workspace_root().join("tests/scripts/store_compact_bound_b3.loft");
    let run = |backend: &str, path: &Path, mode: &str, compact: bool| -> String {
        let mut cmd = Command::new(loft_bin());
        cmd.arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", path)
            .env("MODE", mode)
            .env("LOFT_LOADER_STATS", "1")
            .current_dir(workspace_root());
        if compact {
            cmd.env_remove("LOFT_NO_COMPACT_ON_LOAD");
        } else {
            cmd.env("LOFT_NO_COMPACT_ON_LOAD", "1");
        }
        let out = cmd.output().expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success() && !stdout.contains("FAIL"),
            "{backend} mode={mode} compact={compact}: {stdout}\n{stderr}"
        );
        format!("{stdout}{stderr}")
    };
    let field = |out: &str, line: &str, key: &str| -> i64 {
        let l = out
            .lines()
            .find(|l| l.starts_with(line))
            .unwrap_or_else(|| panic!("no `{line}` line in:\n{out}"));
        let t: Vec<&str> = l.split_whitespace().collect();
        let i = t.iter().position(|x| *x == key).expect(key);
        t[i + 1].parse().expect("number")
    };

    for backend in ["--interpret", "--native"] {
        let tag = backend.trim_start_matches('-');

        // Opted out: re-binding leaves the file exactly as the last run left it.
        let dir = scratch(&format!("b3off_{tag}"));
        let path = dir.join("world.store");
        let wrote = run(backend, &path, "", false);
        let off = run(backend, &path, "reload", false);
        assert_eq!(
            field(&off, "rebound", "bytes"),
            field(&wrote, "wrote", "bytes"),
            "{backend}: opted out, a bound store's file must not change on \
             re-bind:\n{off}"
        );

        // Default: the file follows the content.
        let dir = scratch(&format!("b3on_{tag}"));
        let path = dir.join("world.store");
        let wrote = run(backend, &path, "", true);
        let on = run(backend, &path, "reload", true);
        let before = field(&wrote, "wrote", "bytes");
        let after = field(&on, "rebound", "bytes");
        assert!(
            after * 2 < before,
            "{backend}: a store that peaked at 2000 records and settled at 200 \
             must give its file back on the next run (got {after} from \
             {before}):\n{on}"
        );
        for key in ["count", "digest"] {
            assert_eq!(
                field(&on, "rebound", key),
                field(&wrote, "wrote", key),
                "{backend}: `{key}` changed across the rebuild — the file got \
                 smaller by LOSING data:\n{wrote}{on}"
            );
        }
        assert!(
            on.contains("sound true") && on.contains("label label-7"),
            "{backend}: the rebuilt graph must verify, and the root reference \
             must still read:\n{on}"
        );
        // STILL BOUND — and only a THIRD process can show it.  The `reload` run
        // added a record after compacting; if compaction had left a heap store
        // in the slot, that write would have gone to memory and died with the
        // process, while every size, digest and count assertion above still
        // passed.  So the question is not "did the file change" but "does the
        // next run see what the last one wrote".
        assert_eq!(
            field(&on, "after_write", "count"),
            field(&on, "rebound", "count") + 1,
            "{backend}: the compacted collection must accept a write:\n{on}"
        );
        let seen = run(backend, &path, "verify", true);
        assert_eq!(
            field(&seen, "verify", "count"),
            field(&on, "after_write", "count"),
            "{backend}: a write made AFTER compaction must be on disk for the \
             next run — otherwise the collection quietly stopped persisting:\n{on}{seen}"
        );

        // THE GATE — a dense store must be measured and left alone.  Without
        // this, removing the gate entirely still passes every assertion above,
        // because compacting a dense store is merely wasteful, not wrong.
        let dir = scratch(&format!("b3dense_{tag}"));
        let path = dir.join("dense.store");
        let _dense_wrote = run(backend, &path, "dense", true);
        let dense_reload = run(backend, &path, "reload", true);
        // The REASON is the assertion, not the resulting size.  A dense store
        // rebuilds to about the size it already was, so "did the file change"
        // cannot tell a gate that declined from a rebuild that ran and gained
        // nothing — only the refusal it names can.
        assert!(
            dense_reload.contains("not compacted") && dense_reload.contains("under the eighth"),
            "{backend}: a store with no interior free space must be declined by \
             the GATE (before any rebuild), not discovered to be pointless \
             afterwards:\n{dense_reload}"
        );
        // Its file size is deliberately NOT asserted here.  It is stable now
        // (`persisted_image_keeps_its_slack_after_store_reclaim` owns that
        // property), but it is not what this test is about, and an earlier
        // draft that DID assert it read a 2.33x jump as re-bind growth when the
        // growth was really this script's own digest traversal claiming its
        // snapshot inside the bound store.
    }
}

/// @PLN123 — a persisted image keeps its eighth of slack even when the caller
/// called `store_reclaim` first.
///
/// loft#710 sizes an image at the high-water mark PLUS an eighth, and says why:
/// a bound store stays live and growth multiplies by 7/3, so a store with no
/// room left pays a **2.33× file resize on its very next claim** — worse than
/// the tail that was removed. The clamp keeping the image "never larger than the
/// arena" was safe while capacity always sat well above the mark. `store_reclaim`
/// (arc A) trims capacity TO the mark, so the clamp collapsed the eighth to zero
/// for exactly the stores someone had just tidied, and arc A quietly disabled the
/// protection loft#710 added.
///
/// The claim that trips it is the most ordinary one there is — READING the
/// collection. Iterating a keyed collection materialises a key-sorted snapshot,
/// claimed inside the store when the store is bound. Measured before the fix, a
/// 2,000-record hash: reclaim-then-bind wrote 187,784 bytes and one read took it
/// to 438,160 — **2.07× larger than never reclaiming at all** (211,256).
///
/// So the guard is not "the file is small" but **"tidying up first must not make
/// it worse"**: both paths must land on the same size, and neither may grow when
/// the collection is read.
#[test]
fn persisted_image_keeps_its_slack_after_store_reclaim() {
    let script = workspace_root().join("tests/scripts/store_bind_slack.loft");
    let run = |backend: &str, dir: &Path, reclaim: bool| -> (i64, i64) {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("s.store"))
            .env("N", "2000")
            .env("RECLAIM", if reclaim { "1" } else { "0" })
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && !stdout.contains("FAIL"),
            "{backend} reclaim={reclaim}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let t: Vec<&str> = stdout
            .lines()
            .find(|l| l.starts_with("reclaim"))
            .unwrap_or_else(|| panic!("no report in:\n{stdout}"))
            .split_whitespace()
            .collect();
        let num = |k: &str| -> i64 {
            let i = t.iter().position(|x| *x == k).expect(k);
            t[i + 1].parse().expect("number")
        };
        (num("after_bind"), num("after_read"))
    };

    for backend in ["--interpret", "--native"] {
        let tag = backend.trim_start_matches('-');
        let (tidy_bind, tidy_read) = run(backend, &scratch(&format!("slack_y_{tag}")), true);
        let (plain_bind, plain_read) = run(backend, &scratch(&format!("slack_n_{tag}")), false);

        assert_eq!(
            tidy_read, tidy_bind,
            "{backend}: reading a bound collection must not resize its file — \
             the image's eighth of slack has to absorb the iteration snapshot, \
             or the first read pays the 7/3 ladder ({tidy_bind} -> {tidy_read})"
        );
        assert_eq!(
            plain_read, plain_bind,
            "{backend}: and the same without `store_reclaim` ({plain_bind} -> \
             {plain_read})"
        );
        assert_eq!(
            tidy_bind, plain_bind,
            "{backend}: `store_reclaim` before a bind must not change the image \
             size — the image is already sized from the mark, so tidying first \
             can only remove the slack the format wants"
        );
    }
}

/// @PLN123 F4/F6 — the two refusals that shipped without a test.
///
/// A durable `.dmeta` sidecar records the main file's byte LENGTH and CRC, so
/// shortening it (`store_reclaim`) or rewriting it (compaction at load) turns a
/// healthy store into a corrupt one at the next `store_durable_check`. The plan
/// wrote that down as F4 and STDLIB.md already promised the refusal — but the
/// guard asked `durable_meta_path.is_some()`, which only `Store::open_durable`
/// sets, and **no loft program can reach that entry point**. The loft surface is
/// path-based (`store_durable_seal(path)`), so the reachable hazard was entirely
/// unguarded: seal, reclaim, and a healthy store reported CORRUPT (measured,
/// 156,344 → 138,976 bytes, check true → false). A guard on *how you got here*
/// cannot see a fact about *what is on disk*.
///
/// So the assertions are about the contract, not the mechanism: a sealed store
/// must come through `store_reclaim` and a re-bind unchanged and still check
/// clean, an unsealed one must still be reclaimable (or the "fix" is just a
/// disable), and each refusal must name itself.
#[test]
fn reclaim_and_compaction_refuse_a_sealed_store_and_a_floor_sized_one() {
    let script = workspace_root().join("tests/scripts/store_reclaim_refusals.loft");
    let run_grow = |backend: &str, dir: &Path, mode: &str, seal: bool, n: &str, grow: bool| {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("r.store"))
            .env("MODE", mode)
            .env("SEAL", if seal { "1" } else { "0" })
            .env("GROW", if grow { "1" } else { "0" })
            .env("N", n)
            .env("LOFT_LOADER_STATS", "1")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success() && !stdout.contains("FAIL"),
            "{backend} mode={mode} seal={seal}: {stdout}\n{stderr}"
        );
        format!("{stdout}{stderr}")
    };
    let run = |backend: &str, dir: &Path, mode: &str, seal: bool, n: &str| -> String {
        run_grow(backend, dir, mode, seal, n, false)
    };
    let field = |out: &str, line: &str, key: &str| -> String {
        let l = out
            .lines()
            .find(|l| l.starts_with(line))
            .unwrap_or_else(|| panic!("no `{line}` line in:\n{out}"));
        let t: Vec<&str> = l.split_whitespace().collect();
        let i = t.iter().position(|x| *x == key).expect(key);
        t[i + 1].to_string()
    };
    let num = |out: &str, line: &str, key: &str| -> i64 {
        field(out, line, key).parse().expect("number")
    };

    for backend in ["--interpret", "--native"] {
        let tag = backend.trim_start_matches('-');

        // F4 — `store_reclaim` on a SEALED store.
        let dir = scratch(&format!("f4_{tag}"));
        let sealed = run(backend, &dir, "", true, "3000");
        assert_eq!(field(&sealed, "seal", "check_before"), "true", "{sealed}");
        assert_eq!(
            num(&sealed, "seal", "freed"),
            0,
            "{backend}: a sealed store must not be truncated:\n{sealed}"
        );
        assert_eq!(
            num(&sealed, "seal", "after"),
            num(&sealed, "seal", "before"),
            "{backend}: and its file must be byte-length identical:\n{sealed}"
        );
        assert_eq!(
            field(&sealed, "seal", "check_after"),
            "true",
            "{backend}: the seal must still validate — the whole point of F4 is \
             that a healthy store never reports corrupt:\n{sealed}"
        );

        // CONTROL — without the sidecar, reclaim must still do its job.  A
        // refusal that fires everywhere is just the feature turned off.
        //
        // The control GROWS the store after binding, because a store that came
        // straight from `bind_path` has nothing to give back: the image is
        // written at the mark plus an eighth, and the eighth is exactly what
        // `store_reclaim` now leaves behind (loft#727 — trimming to the bare
        // mark put the store one claim away from a 7/3 re-grow, so the call
        // made the file bigger).  `freed 0` there is the right answer, and a
        // control that read it as a refusal would be asserting the old bug.
        let dir = scratch(&format!("f4ctl_{tag}"));
        let plain = run_grow(backend, &dir, "", false, "3000", true);
        assert!(
            num(&plain, "seal", "freed") > 0
                && num(&plain, "seal", "after") < num(&plain, "seal", "before"),
            "{backend}: an unsealed store must still be reclaimable:\n{plain}"
        );

        // F4 at LOAD — compaction rewrites the whole image, same hazard.
        let dir = scratch(&format!("f4load_{tag}"));
        let wrote = run(backend, &dir, "", true, "3000");
        let reload = run(backend, &dir, "reload", true, "3000");
        assert!(
            reload.contains("not compacted") && reload.contains("durable sidecar"),
            "{backend}: compaction must decline a store with a live sidecar, \
             and say so:\n{reload}"
        );
        assert_eq!(
            num(&reload, "rebound", "bytes"),
            num(&wrote, "seal", "after"),
            "{backend}: and leave the file alone:\n{reload}"
        );

        // F6 — a store at the image floor has nothing to give.
        let dir = scratch(&format!("f6_{tag}"));
        run(backend, &dir, "", false, "40");
        let tiny = run(backend, &dir, "reload", false, "40");
        assert!(
            tiny.contains("not compacted") && tiny.contains("image floor"),
            "{backend}: a floor-sized store must be declined before any rebuild \
             — a bound image is padded up to the floor regardless:\n{tiny}"
        );
    }
}

/// @PLN123 — compaction across COLLECTION KINDS.
///
/// Compaction at load is default-on and rebuilds through `copy_claims`, which
/// `type_is_compactable` waves through for `Sorted`, `Array`/`Ordered`, `Index`
/// and `ChildRec` as well as `Hash` — only `Radix` (spatial) and a cross-store
/// `DbRef` are refused. Every other compaction test builds a `hash<Rec[id]>`, so
/// the rest of that surface shipped by default with no coverage, and the
/// array/ordered destination builder carries a documented history of exactly the
/// failure a digest catches (@P309: copied collections "read back as length 0").
///
/// Per kind: build, fragment, persist, `store_load` back. The count, the
/// order-independent digest and `store_verify` must all survive, and the loader
/// must reach the verdict this kind should reach.
///
/// **Two shapes could not be built at all, both pre-existing** (they reproduce
/// on the released 2026.7.2 binary, so neither is compaction's doing):
/// - fragmenting an index with `#remove` in a filtered loop SIGSEGVs the
///   interpreter and overflows the native stack when the record owns a `text`
///   (loft#718) — worked around here with key assignment, which is a different
///   path;
/// - the `ordered<T>` secondary shape needs a struct declaring BOTH a
///   `sorted<T[..]>` and an `index<T[..]>` field, and merely DECLARING that
///   hangs the interpreter and miscompiles on `--native` (loft#719). That cell
///   is not skipped for convenience — the shape itself does not work.
#[test]
fn compaction_is_correct_for_every_collection_kind_it_accepts() {
    let script = workspace_root().join("tests/scripts/store_compact_kinds.loft");
    // kind, and the verdict the loader must reach for it.
    let kinds: [(&str, &str); 5] = [
        ("hash", "compacted"),
        ("sorted", "compacted"),
        ("index", "compacted"),
        ("nested", "compacted"),
        // The one refusal in the accept-list: a spatial collection is a `Radix`,
        // which `copy_claims` panics on and the keystone walks as empty.
        ("spatial", "cannot carry"),
    ];
    let run = |backend: &str, dir: &Path, kind: &str, reload: bool| -> String {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("k.store"))
            .env("KIND", kind)
            .env("MODE", if reload { "reload" } else { "" })
            .env("LOFT_LOADER_STATS", "1")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success() && !stdout.contains("FAIL"),
            "{backend} kind={kind} reload={reload}: {stdout}\n{stderr}"
        );
        format!("{stdout}{stderr}")
    };
    let field = |out: &str, key: &str| -> String {
        let l = out
            .lines()
            .find(|l| l.starts_with("kind "))
            .unwrap_or_else(|| panic!("no report in:\n{out}"));
        let t: Vec<&str> = l.split_whitespace().collect();
        let i = t.iter().position(|x| *x == key).expect(key);
        t[i + 1].to_string()
    };

    for backend in ["--interpret", "--native"] {
        for (kind, verdict) in kinds {
            let tag = backend.trim_start_matches('-');
            let dir = scratch(&format!("kinds_{kind}_{tag}"));
            let wrote = run(backend, &dir, kind, false);
            let read = run(backend, &dir, kind, true);

            for key in ["count", "sum"] {
                assert_eq!(
                    field(&wrote, key),
                    field(&read, key),
                    "{backend}/{kind}: `{key}` changed across the load — a \
                     rebuild that lost data would satisfy every size assertion \
                     anyone would write:\n{wrote}{read}"
                );
            }
            assert_ne!(field(&wrote, "sum"), "0", "{backend}/{kind}: empty digest");
            assert!(
                field(&wrote, "sound") == "true" && field(&read, "sound") == "true",
                "{backend}/{kind}: the heap graph must verify on both sides:\n{wrote}{read}"
            );
            assert!(
                read.contains(verdict),
                "{backend}/{kind}: expected the loader to report `{verdict}`, \
                 which is what makes this cell test anything:\n{read}"
            );
        }
    }
}

/// loft#710 — `LOFT_HASH_SEED` makes a persisted store byte-reproducible.
///
/// A hash's seed is drawn at random (the P253 hash-DoS defense) and stored in
/// the bucket record, where it decides the bucket ORDER — so rebuilding the
/// same data gave a different file every time, and a per-block checksum could
/// not tell "the data changed" from "it was rebuilt".  The unseeded pair is the
/// control: without it, two identical files would prove nothing about the seed.
#[test]
fn fixed_hash_seed_makes_a_persisted_store_reproducible() {
    let read = |dir: &str, seed: Option<&str>| {
        persist_size(dir, 120, 30, "interleaved", seed);
        fs::read(scratch_existing(dir).join("size.store")).expect("store file")
    };
    let a = read("seed710_a", Some("12345"));
    let b = read("seed710_b", Some("12345"));
    assert_eq!(a.len(), b.len(), "same seed, same data: same length");
    assert!(
        a == b,
        "same seed, same data must give byte-identical bytes"
    );

    let c = read("seed710_c", None);
    let d = read("seed710_d", None);
    assert!(
        c != d,
        "without LOFT_HASH_SEED the bytes must still vary — otherwise this test proves nothing"
    );
}

/// loft#727 — READING a keyed collection must cost its store nothing, in
/// memory and on disk.
///
/// `for r in h` walks a key-sorted snapshot of rec-nrs claimed inside the
/// hash's own store, and the loop epilogue released it only when it had a store
/// of its own (the read-only/`expose`d case). The co-located case — every
/// ordinary hash — was a no-op, on the reasoning that the records go back when
/// the store dies. They do; but a collection outlives the loops that read it,
/// and a BOUND collection's store is a FILE that outlives the process, so the
/// leak accumulated across runs and grew the file with nothing else touching
/// it. Measured before the fix: a 4,000-record hash read once per run, sixteen
/// runs, no writes and no `store_reclaim` — 566,472 bytes to 1,321,768.
///
/// The two halves are one test because they are one defect seen through two
/// windows. The in-memory half is the sensitive one (it counts records, so it
/// fails on the first leaked pass); the bound half is the one that was filed,
/// and it is what makes the leak permanent rather than merely unbounded.
///
/// Which cells actually falsify, replayed through the pre-fix binary: the
/// memory census (4,012 records → 4,092 over 40 passes) and the bound
/// `reclaim=true` series (347,216 → 365,936 → 384,664 → 403,384). The bound
/// `reclaim=false` cell passes on the broken binary at this size — four runs of
/// leak still fit in the arena's slack, and it took sixteen to break through —
/// so it is a companion, not the guard. It stays because it is the shape a
/// program actually has.
#[test]
fn reading_a_collection_leaks_nothing_into_its_store() {
    let script = workspace_root().join("tests/scripts/store_iter_scratch.loft");
    let run = |backend: &str, dir: &Path, mode: &str, reclaim: bool| -> String {
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("s.store"))
            .env("LOFT_HASH_SEED", "727")
            .env("MODE", mode)
            .env("N", "2000")
            .env("PASSES", "40")
            .env("RECLAIM", if reclaim { "1" } else { "0" })
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && !stdout.contains("FAIL"),
            "{backend} {mode}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
    };
    // The `store_memory()` row for the collection, from the census printed
    // after `marker`. Carries the record count and the largest free block, both
    // exact — the MB column is rounded and would hide two records.
    let rows = |stdout: &str, marker: &str| -> Vec<String> {
        let found: Vec<String> = stdout
            .lines()
            .skip_while(|l| !l.starts_with(marker))
            .take_while(|l| !l.starts_with("census") || l.starts_with(marker))
            .filter(|l| l.contains("type hash<") || l.contains("type spatial<"))
            // Drop the leading slot number: two runs need not land in the same
            // slot, and the slot is not the subject.
            .filter_map(|l| l.split_once("MB").map(|(_, rest)| rest.trim().to_string()))
            .collect();
        assert!(
            !found.is_empty(),
            "no census row after '{marker}' in:\n{stdout}"
        );
        found
    };

    for backend in ["--interpret", "--native"] {
        let tag = backend.trim_start_matches('-');

        // In memory: 41 traversals must leave the census exactly where 1 did.
        let mem = run(
            backend,
            &scratch(&format!("iter_mem_{tag}")),
            "memory",
            false,
        );
        assert_eq!(
            rows(&mem, "census one"),
            rows(&mem, "census many"),
            "{backend}: 40 more reads of the same collection changed its store\n{mem}"
        );

        // Every way OUT of such a loop, and both builders.  `break` leaves
        // through the epilogue, `return` skips it for the scope exit, a nested
        // loop holds two scratches at once, and a spatial collection builds
        // its scratch through a different pair of builders.  The release must
        // also survive being called twice on the same value, which is what the
        // emitted IR does — the epilogue's free and the scope exit's free are
        // adjacent, and the null-assignment meant to separate them is dropped.
        let exits = run(
            backend,
            &scratch(&format!("iter_exits_{tag}")),
            "exits",
            false,
        );
        assert_eq!(
            rows(&exits, "census one"),
            rows(&exits, "census many"),
            "{backend}: 40 rounds of break / return / nested / spatial / range \
             changed a store\n{exits}"
        );

        // Bound: the same read, in a fresh process each time, against a file
        // grown post-bind so its capacity sits above the mark.  With
        // `store_reclaim` too — that call gives the free tail back, so it
        // removes the slack a leak would otherwise hide in, and one run then
        // shows what sixteen showed without it.
        for reclaim in [false, true] {
            let dir = scratch(&format!("iter_bound_{tag}_{}", u8::from(reclaim)));
            run(backend, &dir, "build", reclaim);
            let sizes: Vec<String> = (0..4)
                .map(|_| run(backend, &dir, "read", reclaim))
                .map(|s| {
                    s.lines()
                        .find(|l| l.starts_with("read "))
                        .expect("read report")
                        .to_string()
                })
                .collect();
            // Every line carries the digest as well as the size, so a file that
            // stopped growing by losing records fails here too.
            assert!(
                sizes.windows(2).all(|w| w[0] == w[1]),
                "{backend} reclaim={reclaim}: reading a bound collection resized \
                 its file — the iteration snapshot is being left behind in it \
                 (loft#727):\n{}",
                sizes.join("\n")
            );
        }
    }
}

/// loft#752 — a BOUND store's file must be sized by its CONTENT once the
/// binding is released, not by the rung of the 7/3 growth ladder the build
/// happened to stop on.
///
/// `store_persist_bind` FIRST makes the FILE the live arena, and the arena grows
/// by 7/3 and never shrinks by itself, so the file left behind was quantized to
/// a ladder — up to 57% (`1 - 3/7`) above its content, and two builds one rung
/// apart differing by 133% with identical records. loft#710 decided a persisted
/// store's size must follow its content and fixed the IMAGE-write path; the
/// bind-first path never went through it.
///
/// Two invariants, and each is vacuous without the other:
///
/// * **an explicit `store_reclaim` must find nothing left** — the release
///   already handed the tail back, so the two runs land on the same size. On its
///   own this passes trivially if the release reclaimed nothing and the explicit
///   call reclaimed nothing either;
///
/// * **more data must produce a bigger file** — the ladder's signature is two
///   different data sets agreeing to the byte. Measured before the fix, 40 000
///   and 60 000 features both wrote exactly 7,196,280 bytes.
///
/// The record counts and the payload digest are compared too: a file that
/// stopped growing by LOSING records would otherwise satisfy both.
#[test]
fn bound_store_file_is_content_sized_when_the_binding_is_released() {
    let script = workspace_root().join("tests/scripts/store_bind_release_size.loft");
    let run = |backend: &str, dir: &Path, n: u32, reclaim: bool| -> (u64, String) {
        let path = dir.join(format!("s{n}_{}.store", u8::from(reclaim)));
        let out = Command::new(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("N", n.to_string())
            .env("RECLAIM", if reclaim { "1" } else { "0" })
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && !stdout.contains("FAIL"),
            "{backend} n={n} reclaim={reclaim}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let report = stdout
            .lines()
            .find(|l| l.starts_with("reclaim"))
            .unwrap_or_else(|| panic!("no report in:\n{stdout}"))
            .to_string();
        assert!(
            report.contains("verify true"),
            "{backend} n={n} reclaim={reclaim}: store_verify failed — {report}"
        );
        // Everything after the `reclaim <bool>` prefix is the CONTENT digest:
        // tile / road counts and the checksum of every stored point.
        let content = report
            .split_once(" tiles ")
            .expect("report shape")
            .1
            .to_string();
        let size = fs::metadata(&path)
            .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
            .len();
        (size, content)
    };

    for backend in ["--interpret", "--native"] {
        let tag = backend.trim_start_matches('-');
        let dir = scratch(&format!("release_size_{tag}"));

        let (plain_40k, content_40k) = run(backend, &dir, 40_000, false);
        let (tidy_40k, tidy_content_40k) = run(backend, &dir, 40_000, true);
        let (plain_60k, content_60k) = run(backend, &dir, 60_000, false);

        assert_eq!(
            content_40k, tidy_content_40k,
            "{backend}: the two 40k runs must store the same data, else their \
             sizes compare nothing"
        );
        assert_eq!(
            plain_40k, tidy_40k,
            "{backend}: releasing a bound store must hand its tail back, so an \
             explicit `store_reclaim` finds nothing left to give — without \
             reclaim {plain_40k} bytes, with it {tidy_40k} (loft#752)"
        );
        assert_ne!(
            content_40k, content_60k,
            "{backend}: the 40k and 60k runs must differ in content, else the \
             size comparison below is vacuous"
        );
        assert!(
            plain_60k > plain_40k,
            "{backend}: 50% more data must produce a bigger file — both counts \
             wrote {plain_40k} bytes, which is the 7/3 ladder answering instead \
             of the content (loft#752)"
        );
    }
}

/// `scratch` wipes the directory; this returns the same path without wiping, so
/// a caller can read back what a just-finished run wrote there.
fn scratch_existing(test_name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("loft-store-persist-loft").join(test_name)
}

/// @PLN129 arc A — a collection BOUND to a lazy source fetches on a MISS, and
/// fetches only what was asked for.
///
/// The load-bearing assertions are the resident COUNTS. Every value assertion
/// here would also pass under an eager whole-image load; `resident=1` then
/// `resident2=2` against a THREE-entry image is what proves the read was lazy.
/// The unbound control at the top is what proves the collection started empty,
/// without which the counts could be measuring nothing.
#[test]
fn lazy_bound_collection_fetches_only_the_touched_entry_both_backends() {
    let dir = scratch("lazy_bind_129");
    let path = dir.join("lz.store");
    let script = workspace_root().join("tests/scripts/129-lazy-bind.loft");

    let (out_w, code_w) = run_lazy("--interpret", &script, &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("seeded=3"), "write: {out_w:?}");

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_lazy(backend, &script, &path, "read");
        assert_eq!(code, 0, "{backend} read exit: {out:?}");

        // The control: nothing is resident, and an unbound miss stays a miss.
        assert!(
            out.contains("unbound_miss=true unbound_len=0"),
            "{backend}: an UNBOUND collection must answer absent and hold nothing — \
             without this the counts below prove nothing: {out:?}"
        );
        assert!(out.contains("bound=true"), "{backend}: bind: {out:?}");

        // The fault itself, and the count that makes it lazy.
        assert!(
            out.contains("fetched=true name=grace resident=1"),
            "{backend}: a miss must fetch exactly ONE entry from a 3-entry image: {out:?}"
        );
        // One record per key, and a hit costs nothing.
        assert!(
            out.contains("same=true after_hit=1"),
            "{backend}: the same key twice is one record, and a hit fetches nothing: {out:?}"
        );
        assert!(
            out.contains("second=alan resident2=2"),
            "{backend}: a second fault adds exactly one: {out:?}"
        );
        // Absent stays absent, and does not grow the working set.
        assert!(
            out.contains("absent=true resident3=2"),
            "{backend}: a key absent from the SOURCE must not materialise anything: {out:?}"
        );
        assert!(
            out.contains("verify=true"),
            "{backend}: the partially-loaded heap must be structurally sound: {out:?}"
        );

        // @PLN129 arc C — the failure channel. A reachable source that simply
        // lacks the key must leave NO error, or "absent" and "unreachable" are
        // the same answer and the channel is useless.
        assert!(
            out.contains("err_after_absent=[]"),
            "{backend}: a genuine absence must leave the collection healthy: {out:?}"
        );
        assert!(out.contains("gone_bound=true"), "{backend}: bind: {out:?}");
        assert!(
            out.contains("gone_faults=1"),
            "{backend}: a failed fetch must be COUNTED: {out:?}"
        );
        // The shape that was silently wrong before stickiness: fail once, then
        // succeed. The collection is missing data, so it must NOT read healthy.
        assert!(
            out.contains("mixed_ok=true mixed_still_faulted=true"),
            "{backend}: a later success must NOT clear an earlier failure — that \
             traversal is missing data and 'healthy' would be a lie: {out:?}"
        );
        assert!(
            out.contains("cleared=true after_clear=0"),
            "{backend}: an explicit acknowledgement is what clears: {out:?}"
        );
        assert!(
            out.contains("clear_again=false"),
            "{backend}: clearing twice reports nothing left to clear: {out:?}"
        );
        // @PLN129 arc D — the control for the source pin: an UNCHANGED source
        // must fault nothing. Without this a pin that refused every fetch would
        // look correct.
        // @PLN129 arc E — evict-and-refault, and the safety that makes it
        // offerable: a held reference to an evicted record reads null.
        assert!(
            out.contains("ev_filled=grace ev_len=1"),
            "{backend}: the working set filled: {out:?}"
        );
        assert!(
            out.contains("ev_emptied=0"),
            "{backend}: `= []` must empty the working set: {out:?}"
        );
        assert!(
            out.contains("ev_held=grace"),
            "{backend}: a held reference must SURVIVE eviction with its value — \
             the deps system keeps a referenced record alive while the rest is \
             reclaimed, and that is what makes eviction safe to offer: {out:?}"
        );
        assert!(
            out.contains("ev_refaulted=alan ev_len2=1"),
            "{backend}: the BINDING must survive eviction so the next lookup \
             re-faults — otherwise emptying a collection silently unbinds it: {out:?}"
        );
        assert!(
            out.contains("stable=grace,alan stable_faults=0"),
            "{backend}: a stable source must serve a whole traversal with no \
             drift fault — otherwise the pin is refusing everything: {out:?}"
        );
        assert!(
            out.contains("gone_null=true gone_err_empty=false"),
            "{backend}: an UNREACHABLE source must answer null AND leave a reason — \
             the whole point is that these two nulls are told apart: {out:?}"
        );
    }
}

/// `run_mode_backend`'s sibling for the @PLN129 script, which reads its mode
/// from `LOFT_LAZY_MODE` so it cannot be confused with the persist scripts'.
fn run_lazy(backend: &str, script: &Path, path: &Path, mode: &str) -> (String, i32) {
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_LAZY_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        eprintln!(
            "{backend} {mode} stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    (stdout, out.status.code().unwrap_or(-1))
}

/// @PLN129 arc F — the gate: a real graph traversed lazily, with the fetch count
/// the design predicts.
///
/// Two assertions carry this test, and neither is about a value. **Identity**:
/// two persons at the same company must reach ONE company record, because
/// identity falls out of the collection rather than a side map — if that fails
/// the whole design is wrong. **Count**: `c=1` after the second hop is what
/// proves the hop HIT the working set; every value assertion here would pass
/// under an eager whole-image load, and only the counts would not.
#[test]
fn lazy_graph_traversal_fetches_only_what_it_touches_both_backends() {
    let dir = scratch("lazy_graph_129");
    let persons = dir.join("persons.store");
    let companies = dir.join("companies.store");
    let script = workspace_root().join("tests/scripts/129-lazy-graph.loft");

    let (out_w, code_w) = run_graph("--interpret", &script, &persons, &companies, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(
        out_w.contains("seeded companies=3 persons=4"),
        "seed: {out_w:?}"
    );

    for backend in ["--interpret", "--native"] {
        let (out, code) = run_graph(backend, &script, &persons, &companies, "read");
        assert_eq!(code, 0, "{backend} read exit: {out:?}");

        // The control: both collections start empty, so the counts below mean
        // something.
        assert!(out.contains("start p=0 c=0"), "{backend}: {out:?}");
        assert!(
            out.contains("hop1 ada@Acme p=1 c=1"),
            "{backend}: one hop fetches one person and one company: {out:?}"
        );
        // The second person shares Acme: the company hop must HIT, so c stays 1.
        assert!(
            out.contains("hop2 grace@Acme p=2 c=1"),
            "{backend}: a second person at the SAME company must NOT re-fetch it \
             — c must stay 1: {out:?}"
        );
        assert!(
            out.contains("identity=true"),
            "{backend}: two paths to one company must give ONE record — identity \
             falls out of the collection, and if this fails the design is wrong: {out:?}"
        );
        assert!(
            out.contains("hop3 alan@Globex p=3 c=2"),
            "{backend}: a different company IS fetched: {out:?}"
        );
        // 3 persons + 2 companies for 3 hops — not 6, and edsger/Initech were
        // never asked for and must not be resident.
        assert!(
            out.contains("touched=5"),
            "{backend}: fetches must equal records TOUCHED, not reachable — a \
             lazy read that pulls the closure is an eager read with extra steps: {out:?}"
        );
        assert!(
            out.contains("sound=true,true"),
            "{backend}: both partially-loaded heaps must be structurally sound: {out:?}"
        );
    }
}

/// The @PLN129 graph script takes TWO scratch paths, one per collection —
/// per-collection binding is the point, so one path would not exercise it.
fn run_graph(
    backend: &str,
    script: &Path,
    persons: &Path,
    companies: &Path,
    mode: &str,
) -> (String, i32) {
    let out = Command::new(loft_bin())
        .arg(backend)
        .arg(script)
        .env("LOFT_GRAPH_PERSONS", persons)
        .env("LOFT_GRAPH_COMPANIES", companies)
        .env("LOFT_LAZY_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        eprintln!(
            "{backend} {mode} stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    (stdout, out.status.code().unwrap_or(-1))
}

/// loft#783 — a pointer-bearing vector element relocates in BULK, not field-at-a-time.
///
/// `store_load_keys` bulk-read a `vector<Pt>` element block in one request, then fell off
/// that path the moment an element carried a POINTER: a `text` or a nested vector cost ~3
/// extra four-byte reads PER ELEMENT. Measured on a 100×250 `vector<Named>`, 25 000
/// elements drew **75 893** four-byte reads. In the consumer that filed it, one viewport
/// issued 804 313 of them — 175 ms of pure CPU with the bytes already in a prefetch buffer,
/// and ~1.3 s projected for a cold view. It was never bytes; it was call count.
///
/// Two of the three were avoidable and both are now gone: the field's POINTER word was
/// re-fetched over the reader although the caller had just flat-copied it into the local
/// record, and the sub-record's size and length words were fetched separately although
/// they are adjacent.
///
/// **The read counts are the assertion.** Every arm loaded the right values before the fix
/// too, so a value-only test says nothing about the defect — but the values are asserted
/// as well, because a relocation that dropped the strings would otherwise read as a win.
/// `LOFT_LOADER_WORDWISE=1` is the pre-fix route on the same binary: the control that
/// proves the count can move, so a green here is not a harness that measures nothing.
#[test]
fn a_pointer_bearing_element_relocates_in_bulk() {
    let dir = scratch("ptr_element_reads_783");
    let script = workspace_root().join("tests/scripts/783-ptr-element-reads.loft");
    let path = dir.join("p");

    let run = |mode: &str, wordwise: bool| -> String {
        let mut cmd = Command::new(loft_bin());
        cmd.arg("--interpret")
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_LOADER_STATS", "1")
            .env("LOFT_TIMEOUT", "180")
            .current_dir(workspace_root());
        if !mode.is_empty() {
            cmd.env("P783_MODE", mode);
        }
        if wordwise {
            cmd.env("LOFT_LOADER_WORDWISE", "1");
        }
        let out = cmd.output().expect("failed to invoke loft binary");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success(),
            "mode={mode} wordwise={wordwise}: {all}"
        );
        assert!(!all.contains("FAIL"), "mode={mode}: {all}");
        all
    };

    /// The count in the 4-byte bucket of `reads=[(2, N, bytes), …]` — one entry per
    /// power-of-two size class, so bucket 2 is exactly the word-at-a-time traffic.
    fn four_byte_reads(out: &str) -> u64 {
        let at = out.find("reads=[").expect("loader stats must report reads");
        let rest = &out[at..];
        let open = rest.find("(2, ").map_or(usize::MAX, |i| i + 4);
        assert_ne!(open, usize::MAX, "no 4-byte bucket in: {rest}");
        let tail = &rest[open..];
        let end = tail.find(',').expect("a count then a comma");
        tail[..end].trim().parse().expect("a numeric read count")
    }
    /// The one result line, so the two routes can be compared value for value.
    fn result(out: &str) -> &str {
        out.lines()
            .find(|l| l.starts_with("loaded="))
            .expect("the script must report its load")
    }

    run("write", false);
    let fast = run("", false);
    let slow = run("", true);

    // Correctness FIRST: fewer reads that lose data is not a fix.
    assert_eq!(
        result(&fast),
        result(&slow),
        "the bulk route must load exactly what the word-at-a-time route does"
    );
    assert!(
        result(&fast).contains("elements=1000") && result(&fast).contains("chars=2800"),
        "every element and every string must survive the relocation: {}",
        result(&fast)
    );

    let (fast_reads, slow_reads) = (four_byte_reads(&fast), four_byte_reads(&slow));
    // The control: the pre-fix route really is word-at-a-time, so this test can fail.
    assert!(
        slow_reads > 1000,
        "the wordwise control must still be word-at-a-time ({slow_reads} reads) — if it is \
         not, this test is measuring nothing"
    );
    // 1000 elements. The defect was ~3 four-byte reads EACH; the bound is deliberately
    // loose (well under one per element) so it pins the CLASS rather than today's count.
    assert!(
        fast_reads < 500,
        "a pointer-bearing element must not cost a four-byte read each: {fast_reads} reads \
         for 1000 elements (wordwise control: {slow_reads})"
    );
}

/// loft#782 — the ROUND-TRIP DEPTH harness: `store_load_keys` issues its ranges serially.
///
/// `Stores::load_keys` is `for &kv in keys_vals { load_one(…) }`, and under it
/// `PageProvider::fetch` is synchronous and single-range. So a keyed load costs one
/// sequential round trip per page it discovers it needs, and over a real link the round
/// trip is the entire cost — the consumer that filed this measured a 1-byte range and a
/// 64 kB range at the SAME ~45 ms, with 764 serial reads making a cold viewport's 16–26 s
/// wait. It is depth, not bytes.
///
/// **This test is the gate, not the fix.** It exists so a batching change can be proved
/// rather than asserted, and it is written to measure the two things such a change moves:
///
/// - `requests` — how many round trips the load takes. Deterministic, and the real metric.
/// - wall time under injected per-request latency — the consequence, and the reason to care.
///
/// The server handles each connection on its own thread deliberately: with the
/// single-threaded one a concurrent client would be serialised by the socket, and this
/// harness would report "no improvement" for a fix that worked. That failure mode is worth
/// more care than the assertion itself.
///
/// What it pins today is the SHAPE: with N keys the load takes multiple round trips, and
/// wall time tracks `requests × latency`. When phase batching lands, the depth becomes
/// roughly constant in N and this test tightens to say so.
#[test]
fn store_load_keys_round_trip_depth_is_measurable() {
    let dir = scratch("load_keys_depth_782");
    let path = dir.join("world.store");
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");

    let sidecar = fs::read(format!("{}.dschema", path.display())).ok();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let delay_ms = 20;
    let (url, served) = serve_ranges_tuned(
        fs::read(&path).unwrap(),
        sidecar,
        delay_ms,
        Some(std::sync::Arc::clone(&hits)),
    );

    let started = std::time::Instant::now();
    let (out, code) = run_mode_backend("--interpret", &load_script(), Path::new(&url), "loadkey");
    let elapsed = started.elapsed();
    assert_eq!(code, 0, "http loadkey exit: {out:?}");
    assert!(
        out.contains("loadkey verify=true"),
        "the working set fetched over http must be sound: {out:?}"
    );

    let requests = served.load(std::sync::atomic::Ordering::Relaxed);
    // Report the measurement: this test's job is to make depth VISIBLE, so the numbers
    // belong in the output whether it passes or fails.
    eprintln!("loft#782 depth harness: requests={requests} latency={delay_ms}ms wall={elapsed:?}");
    // The harness must actually be measuring something: no requests means the store came
    // from somewhere else and every number below is meaningless.
    assert!(
        requests > 0,
        "the load issued no HTTP requests — this harness is measuring nothing"
    );
    // Latency is REACHING the client: with requests served serially, the floor is
    // `requests × delay`. Generous (half) because a warm page costs nothing and the
    // request count includes the sidecar; the point is that depth is visible in the clock,
    // not a precise timing claim.
    let floor = std::time::Duration::from_millis(delay_ms * (requests as u64) / 2);
    assert!(
        elapsed >= floor,
        "wall time {elapsed:?} is below {floor:?} for {requests} requests at {delay_ms}ms — \
         the injected latency is not reaching the loader, so this harness cannot show a \
         batching win either"
    );
}
