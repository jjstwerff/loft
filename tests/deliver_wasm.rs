// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 2c/2d — the deliver WHOLE-SLICE falsifier: a loft value delivered from a
// `--html`-built wasm is reconstructed IN JS, driven only by the layout descriptor, and equals
// the value the program delivered. This closes the parity gate interpret == native == --html:
// `deliver_parity.rs` pins interpret == native (the loopback bytes); this pins that the generic
// JS reader (doc/loft-deliver.js, the twin of `read_via_descriptor`) reproduces the same value in
// the browser target.
//
// Self-skips when the wasm toolchain is unavailable (node / wasm32 target / release binary) —
// same policy as tests/html_wasm.rs.

use std::path::PathBuf;
use std::process::Command;

fn which(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wasm32_installed() -> bool {
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-unknown-unknown"))
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Build `source` via `loft --html`, extract the embedded wasm, run it through the deliver node
/// harness, and return its stdout — or `None` if the toolchain self-skips.
fn run_deliver(name: &str, source: &str) -> Option<String> {
    if !which("node") {
        eprintln!("SKIP: node not installed");
        return None;
    }
    if !wasm32_installed() {
        eprintln!("SKIP: wasm32-unknown-unknown target not installed");
        return None;
    }
    let loft_bin = repo_root().join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release`)");
        return None;
    }

    let tmp = std::env::temp_dir().join(format!("loft_deliver_{name}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create per-test dir");
    let src = tmp.join(format!("{name}.loft"));
    let html = tmp.join(format!("{name}.html"));
    let wasm = tmp.join(format!("{name}.wasm"));
    std::fs::write(&src, source).expect("write source");

    let status = Command::new(&loft_bin)
        .args([
            "--html",
            html.to_str().unwrap(),
            "--path",
            &format!("{}/", repo_root().display()),
        ])
        .arg(src.to_str().unwrap())
        .status()
        .expect("invoke loft --html");
    assert!(status.success(), "loft --html failed for {name}");

    // Extract the embedded wasm (`const wasmB64="…"`), exactly as tests/html_wasm.rs does.
    let page = std::fs::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = page.find(marker).expect("wasmB64 marker") + marker.len();
    let end = start + page[start..].find('"').expect("wasmB64 closing quote");
    std::fs::write(&wasm, loft::base64::decode(&page[start..end])).expect("write wasm");

    let harness = repo_root().join("tools/deliver_repro.mjs");
    assert!(harness.exists(), "tools/deliver_repro.mjs missing");
    let out = Command::new("node")
        .arg(&harness)
        .arg(&wasm)
        .output()
        .expect("invoke node deliver harness");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "deliver harness failed for {name}\nstdout:{stdout}\nstderr:{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(stdout)
}

#[test]
fn deliver_reconstructs_nested_value_in_js() {
    // Exercises every node kind in the serializable subset the reader mirrors: text (interned),
    // a nested record, an i64 scalar, an f64 scalar, a scalar vector (the fast lane), and a bool.
    let src = r#"
struct Inner { a: integer, b: float }
struct Outer { name: text, inner: Inner, nums: vector<integer>, ok: boolean }
fn main() {
  o = Outer { name: "hi", inner: Inner { a: 7, b: 1.5 }, nums: [10, 20, 30], ok: true };
  deliver(1, o);
}
"#;
    let Some(stdout) = run_deliver("nested", src) else {
        return; // toolchain self-skip
    };
    // The JS reader must reconstruct the exact value the program delivered. Pin the tag + value,
    // NOT the internal type-id (which shifts with the stdlib).
    let want_value =
        "\"value\":{\"name\":\"hi\",\"inner\":{\"a\":7,\"b\":1.5},\"nums\":[10,20,30],\"ok\":true}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value) && stdout.contains("\"tag\":1"),
        "reconstructed value mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_top_level_hash_in_js() {
    // @PLN105 Phase 3 slice — a top-level keyed `hash<T>` has no byte layout a reader can walk, so
    // deliver PRE-FLATTENS it (materialise to a scratch array of element records, the `for x in h`
    // path) and hands JS a synthetic `Array(elem)` descriptor. JS reconstructs the elements via the
    // existing array reader, KEY-SORTED, touching no hash-layout knowledge.
    let src = r#"
struct Item { ik: integer, name: text }
fn main() {
  h: hash<Item[ik]> = [];
  h[30] = Item { ik: 30, name: "thirty" };
  h[10] = Item { ik: 10, name: "ten" };
  h[20] = Item { ik: 20, name: "twenty" };
  deliver(1, h);
}
"#;
    let Some(stdout) = run_deliver("hash", src) else {
        return; // toolchain self-skip
    };
    // Inserted out of order (30,10,20); reconstructed KEY-SORTED (10,20,30) == the interpreter's
    // `for x in h` order (both use build_hash_sorted_vec).
    let want_value = "\"value\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"},{\"ik\":30,\"name\":\"thirty\"}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "flattened hash mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_nested_hash_field_in_js() {
    // @PLN105 Phase 3 — a struct with a keyed FIELD: the scalar/text fields are read in place, and
    // the `hash` field is flattened to a key-sorted array (a FlatArray descriptor node carrying the
    // materialised data record) — so JS reconstructs `{…scalars…, field: [elements]}` with no
    // hash-layout knowledge.
    let src = r#"
struct Item { ik: integer, name: text }
struct Bag { label: text, items: hash<Item[ik]>, count: integer }
fn main() {
  b: Bag = Bag { label: "mybag", items: [], count: 3 };
  b.items[20] = Item { ik: 20, name: "twenty" };
  b.items[10] = Item { ik: 10, name: "ten" };
  deliver(1, b);
}
"#;
    let Some(stdout) = run_deliver("nesthash", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"label\":\"mybag\",\"items\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"}],\"count\":3}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "nested-hash-field mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_top_level_spatial_in_js() {
    // @PLN105 Phase 3 — a `spatial`/radix collection: pre-flattened via build_radix_sorted_vec (the
    // same tree-walk `for x in xs` uses), so JS reconstructs the elements in NATURAL (Morton)
    // order, identical to the interpreter's iteration.
    let src = r#"
struct Pt { x: integer, y: integer }
fn main() {
  xs: spatial<Pt[x, y]> = [];
  xs += [Pt{x: 3, y: 3}];
  xs += [Pt{x: 1, y: 2}];
  xs += [Pt{x: 0, y: 0}];
  deliver(1, xs);
}
"#;
    let Some(stdout) = run_deliver("spatial", src) else {
        return; // toolchain self-skip
    };
    // Morton/Z-order (== the interpreter's `for m in xs`): (0,0), (1,2), (3,3).
    let want_value = "\"value\":[{\"x\":0,\"y\":0},{\"x\":1,\"y\":2},{\"x\":3,\"y\":3}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "flattened spatial mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_reads_top_level_sorted_in_js() {
    // @PLN105 Phase 3 — a `sorted<T>` is stored as an INLINE vector kept in key order
    // (`sorted_finish` inserts each `+=` in sorted position), so it needs NO materialisation: it is
    // reclassified to an in-place `Vector` node and read where it lives. Inserted {30,10,20} →
    // key-sorted [10,20,30] == the interpreter's `for it in s`.
    let src = r#"
struct Item { k: integer, name: text }
fn main() {
  s: sorted<Item[k]> = [];
  s += [Item{k: 30, name: "thirty"}];
  s += [Item{k: 10, name: "ten"}];
  s += [Item{k: 20, name: "twenty"}];
  deliver(1, s);
}
"#;
    let Some(stdout) = run_deliver("sorted", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":[{\"k\":10,\"name\":\"ten\"},{\"k\":20,\"name\":\"twenty\"},{\"k\":30,\"name\":\"thirty\"}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "sorted mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_reads_nested_sorted_field_in_js() {
    // @PLN105 Phase 3 — a struct with a `sorted` field: scalars in place, the sorted field
    // reclassified to a `Vector` node (in-place, key-ordered).
    let src = r#"
struct Item { k: integer, name: text }
struct Cat { title: text, items: sorted<Item[k]>, n: integer }
fn main() {
  c: Cat = Cat { title: "cat", items: [], n: 5 };
  c.items += [Item{k: 30, name: "thirty"}];
  c.items += [Item{k: 10, name: "ten"}];
  deliver(1, c);
}
"#;
    let Some(stdout) = run_deliver("nestsorted", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"title\":\"cat\",\"items\":[{\"k\":10,\"name\":\"ten\"},{\"k\":30,\"name\":\"thirty\"}],\"n\":5}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "nested-sorted-field mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_top_level_index_in_js() {
    // @PLN105 Phase 3 — an `index<T>` (red-black tree): pre-flattened via build_index_sorted_vec (an
    // in-order first→next walk), so JS reconstructs the elements in ASCENDING KEY order == the
    // interpreter's `for it in ix`. The node's #left/#right/#color tree bookkeeping is SKIPPED (a
    // `#`-prefixed synthetic field), so only the user fields appear.
    let src = r#"
struct IRec { ik: integer, name: text }
fn main() {
  ix: index<IRec[ik]> = [];
  ix += [IRec{ik: 30, name: "thirty"}];
  ix += [IRec{ik: 10, name: "ten"}];
  ix += [IRec{ik: 20, name: "twenty"}];
  deliver(1, ix);
}
"#;
    let Some(stdout) = run_deliver("index", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"},{\"ik\":30,\"name\":\"thirty\"}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "flattened index mismatch (leaked tree fields? wrong order?)\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_nested_index_field_in_js() {
    // @PLN105 Phase 3 — a struct with an `index` field: scalars in place, the index field
    // materialised to a key-ordered array (FlatArray), tree bookkeeping skipped.
    let src = r#"
struct IRec { ik: integer, name: text }
struct Db { label: text, idx: index<IRec[ik]>, n: integer }
fn main() {
  d: Db = Db { label: "db", idx: [], n: 7 };
  d.idx += [IRec{ik: 20, name: "twenty"}];
  d.idx += [IRec{ik: 10, name: "ten"}];
  deliver(1, d);
}
"#;
    let Some(stdout) = run_deliver("nestindex", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"label\":\"db\",\"idx\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"}],\"n\":7}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "nested-index-field mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}
