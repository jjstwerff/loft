// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! loft#678 — the working-set store loaders on the BROWSER target.
//!
//! `store_load_key` and friends were absent from `wasm32-unknown-unknown`: the whole
//! family sat behind the `remote-store` feature, whose `ureq` client cannot exist there,
//! so `loft --html` failed with `E0599: no method named load_key` pointing into generated
//! Rust. The capability now rides the same asyncify `fetch()` bridge
//! `store_load_url_trusted` uses, with `Range: bytes=a-b` GETs.
//!
//! Two things are asserted, and the second is the feature:
//!  1. the entry actually loads and the copied heap is sound (`store_verify`), and
//!  2. it read a small FRACTION of the file.
//!
//! Without (2) the test would pass over a silent fallback to a whole-file load — which is
//! precisely the outcome the consumer cannot afford (a phone paging a multi-GB block).

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn loft_bin() -> PathBuf {
    repo_root().join("target/release/loft")
}

fn which(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .is_ok_and(|o| o.status.success())
}

fn wasm32_installed() -> bool {
    Command::new("rustc")
        .args(["--print", "target-list"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-unknown-unknown"))
        && repo_root()
            .join("target/wasm32-unknown-unknown/release/libloft.rlib")
            .exists()
}

/// Entries written to the fixture store (~4.5 MB). The size is load-bearing: a keyed
/// lookup costs a fixed handful of 64 KiB pages, so on a small image that constant IS
/// most of the file and a "read less than the whole thing" assertion passes for free.
const ENTRIES: usize = 40000;

/// The page budget a single keyed lookup may spend — the hash-root pages, the bucket it
/// probes, the entry, and its relocated text. Generous, but a CONSTANT: the design claim
/// is that cost depends on the lookup, not on how big the store is, so a whole-file
/// fallback (or a traversal that degrades with size) breaks this no matter the fixture.
const PAGE_BUDGET: u64 = 8 * 64 * 1024;

#[test]
fn store_load_key_pages_over_the_browser_fetch_bridge() {
    if !which("node") {
        eprintln!("SKIP: node not installed");
        return;
    }
    if !wasm32_installed() {
        eprintln!("SKIP: wasm32-unknown-unknown target/rlib not built");
        return;
    }
    if !loft_bin().exists() {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }

    let tmp = std::env::temp_dir().join("loft_paged_browser");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create test dir");
    let store = tmp.join("block.store");

    // 1. Write the fixture store natively.
    let writer = tmp.join("write.loft");
    std::fs::write(
        &writer,
        format!(
            "struct Tile {{ tkey: integer not null, name: text not null }}\n\
             fn main() {{\n\
             \x20 path = env_variable(\"LOFT_PAGED_WRITE_PATH\");\n\
             \x20 t: hash<Tile[tkey]> = [];\n\
             \x20 i = 0;\n\
             \x20 while i < {ENTRIES} {{ t += Tile {{ tkey: i, name: \"tile-{{i}}-padding-to-give-the-image-some-size\" }}; i += 1 }}\n\
             \x20 if !store_persist_bind(t, path) {{ println(\"FAIL write-bind\"); return }}\n\
             \x20 println(\"write ok len={{len(t)}}\");\n\
             }}\n"
        ),
    )
    .expect("write writer script");

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&writer)
        .env("LOFT_PAGED_WRITE_PATH", &store)
        .current_dir(repo_root())
        .output()
        .expect("invoke loft to write the store");
    let so = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        so.contains(&format!("write ok len={ENTRIES}")),
        "fixture store not written: {so} / {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let file_len = std::fs::metadata(&store).expect("store metadata").len();
    assert!(
        file_len > 4 * PAGE_BUDGET,
        "the fixture must be several times the page budget or the bounded-fetch \
         assertion below proves nothing; got {file_len} bytes"
    );

    // 2. Build the browser bundle. The URL is inert — the harness serves the bytes —
    //    but it must be `http://` so `PageSource` selects the range provider.
    let src = tmp.join("paged.loft");
    std::fs::write(
        &src,
        "struct Tile { tkey: integer not null, name: text not null }\n\
         fn main() {\n\
         \x20 tiles: hash<Tile[tkey]> = [];\n\
         \x20 ok = store_load_key(tiles, \"http://127.0.0.1:1/block.store\", 2718);\n\
         \x20 println(\"browser ok={ok} len={len(tiles)}\");\n\
         \x20 got = tiles[2718];\n\
         \x20 println(\"browser name={if got == null { \"null\" } else { got.name }}\");\n\
         \x20 println(\"browser verify={store_verify(tiles)}\");\n\
         }\n",
    )
    .expect("write browser script");

    let html = tmp.join("paged.html");
    let status = Command::new(loft_bin())
        .args([
            "--html",
            html.to_str().unwrap(),
            "--path",
            &format!("{}/", repo_root().display()),
        ])
        .arg(src.to_str().unwrap())
        .current_dir(repo_root())
        .status()
        .expect("invoke loft --html");
    assert!(
        status.success(),
        "loft --html must build a program that calls store_load_key (loft#678: it failed \
         with E0599 inside generated Rust)"
    );

    // 3. Extract the embedded wasm and run it against real ranges.
    let page = std::fs::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = page.find(marker).expect("wasmB64 marker") + marker.len();
    let end = start + page[start..].find('"').expect("wasmB64 closing quote");
    let wasm = tmp.join("paged.wasm");
    std::fs::write(&wasm, loft::base64::decode(&page[start..end])).expect("write wasm");

    let run = Command::new("node")
        .arg(repo_root().join("tools/paged_range_host.mjs"))
        .arg(&wasm)
        .env("LOFT_PAGED_FILE", &store)
        .current_dir(repo_root())
        .output()
        .expect("invoke node harness");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(
        run.status.success(),
        "browser run failed\n  stdout:\n{stdout}\n  stderr:\n{stderr}"
    );

    // The entry arrived, intact, and the copied heap is structurally sound.
    assert!(
        stdout.contains("browser ok=true"),
        "the keyed entry must load in the browser\n  stdout:\n{stdout}\n  stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("browser ok=true len=1"),
        "a working-set load must bring ONE entry, not the whole hash\n  stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("browser name=tile-2718-padding-to-give-the-image-some-size"),
        "the relocated text field must survive the paged copy\n  stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("browser verify=true"),
        "the paged copy must produce a structurally sound heap\n  stdout:\n{stdout}"
    );

    // …and it PAGED. This is the assertion the feature exists for.
    let paged = stdout
        .lines()
        .find(|l| l.starts_with("PAGED "))
        .unwrap_or_else(|| panic!("harness must report its fetch total\n{stdout}"));
    let field = |k: &str| -> u64 {
        paged
            .split_whitespace()
            .find_map(|f| f.strip_prefix(k))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("malformed harness line: {paged}"))
    };
    let fetched = field("bytes_fetched=");
    let total = field("file=");
    let requests = field("requests=");
    assert!(
        requests > 0,
        "the range bridge must actually be called — 0 requests means the loader never \
         reached it: {paged}"
    );
    assert_eq!(
        total, file_len,
        "the harness must be serving the fixture it was given: {paged}"
    );
    assert!(
        fetched <= PAGE_BUDGET,
        "a keyed lookup must cost a BOUNDED number of pages, independent of image size \
         — {fetched} bytes of a {total}-byte store exceeds the {PAGE_BUDGET}-byte budget, \
         which is what a silent whole-file fallback looks like: {paged}"
    );
}
