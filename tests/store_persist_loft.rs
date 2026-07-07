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
fn serve_ranges(bytes: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let mut buf = [0u8; 2048];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let total = bytes.len();
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
                let body = if a <= b { &bytes[a..=b] } else { &bytes[0..0] };
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
                let _ = s.write_all(&bytes);
            }
        }
    });
    format!("http://127.0.0.1:{port}/store")
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

fn nested_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_nested.loft")
}

fn vecstruct_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_vecstruct.loft")
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
/// as the local-file path — the generic `PageProvider` seam means the traversal
/// + relocating copy are unchanged. This is the "fetch" in the partial store
/// fetcher. Both backends.
#[test]
fn store_load_key_over_http_range() {
    let dir = scratch("store_load_http");
    let path = dir.join("world.store");

    // Write the store locally (mmap bind), then serve its bytes over HTTP Range.
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "{out_w:?}");
    let url = serve_ranges(fs::read(&path).unwrap());

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
