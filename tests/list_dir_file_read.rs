// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN118 arc E / crawler-**H6** — a live `list_dir` result must survive a file
//! read, at EVERY listing rather than only the first.
//!
//! Reading a file inside a loop over a `list_dir` vector used to free a slot that
//! vector still referenced (the interpreter's store-slot-reuse UAF).  From the
//! SECOND listing onward `len()` collapsed to 0 mid-loop and the remaining
//! entries were silently skipped: a consumer's round-trip gate walked two corpus
//! directories, loaded 3 of 22 entries from the second, and reported a CLEAN PASS
//! on the 13 it saw.  A slightly different shape of the same reuse SIGSEGV'd the
//! interpreter outright.  Native was clean throughout, so this guards the
//! backends against diverging again too.
//!
//! Why this is a Rust test and not a `tests/scripts/*.loft` entry: the script
//! corpus enforces a zero-leak gate (`SCRIPTS_LEAK_ALLOW` is deliberately empty),
//! and walking a SECOND directory trips a pre-existing interpreter store leak —
//! loft#615, reproducible on released 2026.7.2 and unrelated to H6.  Allowlisting
//! the script would have blunted that net for everyone, so the value guarantee
//! lives here and the leak is tracked on its own.

use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

/// Two directories with DIFFERENT entry counts, so reading the wrong listing is
/// distinguishable from reading none, and every entry's CONTENT is checked, so a
/// right-length-wrong-values outcome cannot pass either.  Counts are reported
/// before the loop, after the loop, and against what the loop actually visited —
/// the "two measurements disagreeing" property that caught this in the consumer.
const PROGRAM: &str = r#"
fn main() {
    mkdir_all("h6_d0");
    mkdir_all("h6_d1");
    for i in 0..3 { file("h6_d0/f{i}.t").write("d0f{i}"); }
    for i in 0..5 { file("h6_d1/f{i}.t").write("d1f{i}"); }

    for k in 0..2 {
        d = "h6_d{k}";
        ns = list_dir(d) ?? [];
        before = len(ns);
        seen = 0;
        good = 0;
        for i in 0..len(ns) {
            nm = ns[i] ?? "";
            if nm.ends_with(".t") {
                c = content(file("{d}/{nm}")) ?? "";
                seen = seen + 1;
                if c.starts_with("d{k}f") { good = good + 1; }
            }
        }
        println("dir{k} before={before} after={len(ns)} seen={seen} good={good}");
    }

    for i in 0..3 { delete("h6_d0/f{i}.t"); }
    for i in 0..5 { delete("h6_d1/f{i}.t"); }
    delete("h6_d0");
    delete("h6_d1");
}
"#;

fn run(backend: &str) -> String {
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!(
        "loft_h6_{backend_tag}_{pid}",
        backend_tag = backend.trim_start_matches('-')
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir workdir");
    let prog = dir.join("h6.loft");
    std::fs::write(&prog, PROGRAM).expect("write program");

    let out = Command::new(loft_bin())
        .args([backend])
        .arg(&prog)
        .current_dir(&dir)
        .output()
        .expect("invoke loft");
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    text
}

/// Every listing sees all of its entries, before AND after the file reads, and
/// each entry reads back its own content.  The second directory is the one that
/// used to collapse.
fn assert_both_listings_intact(backend: &str) {
    let out = run(backend);
    assert!(
        out.contains("dir0 before=3 after=3 seen=3 good=3"),
        "{backend}: first listing must be intact; got:\n{out}"
    );
    assert!(
        out.contains("dir1 before=5 after=5 seen=5 good=5"),
        "{backend}: the SECOND listing must survive the file reads — this is H6; got:\n{out}"
    );
}

#[test]
fn a_live_list_dir_survives_a_file_read_on_the_interpreter() {
    assert_both_listings_intact("--interpret");
}

#[test]
fn a_live_list_dir_survives_a_file_read_on_native() {
    assert_both_listings_intact("--native");
}
