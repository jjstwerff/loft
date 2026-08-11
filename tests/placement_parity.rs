// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN119 — THE GATE.
//
// The plan rests on one invariant:
//
//   A call to a library is indistinguishable — in type, effect,
//   ownership/lifetime, and error behaviour — from the same call in-process.
//   Where it runs is deployment policy, not source.
//
// So the test is not "does a placed call return the right number" (that is
// `placement_worker.rs`); it is "does flipping ONE LINE OF MANIFEST change
// anything observable". One unchanged consumer, one unchanged library, run
// under `placement = "inproc"` and `placement = "process"`, requiring identical
// stdout, identical stderr, and identical exit status.
//
// stderr carries the leak half of the gate: `check_store_leaks` runs on
// `--interpret` and prints there, so a placed call that leaked a store — or
// freed one the caller still owned — shows up as a stderr difference rather
// than needing a separate instrument.
//
// Any divergence here falsifies the invariant, which is exactly what it is for.

#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("loft-placement-parity").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write the library with `placement` set to `mode`, leaving its source alone.
fn write_library(root: &Path, mode: &str, source: &str) {
    let pkg = root.join("libs").join("parity");
    std::fs::create_dir_all(pkg.join("src")).expect("create package");
    std::fs::write(
        pkg.join("loft.toml"),
        format!(
            "[package]\nname = \"parity\"\nversion = \"0.1.0\"\n\n\
             [library]\nplacement = \"{mode}\"\n"
        ),
    )
    .expect("write manifest");
    std::fs::write(pkg.join("src").join("parity.loft"), source).expect("write source");
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(root: &Path, consumer: &Path) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(consumer)
        // Bound the run: a wire that never answers would otherwise hang the suite.
        .env("LOFT_TIMEOUT", "60")
        // Vary ONE axis. Left alone, the in-process side auto-compiles the
        // library to a cdylib and the placed side does not (a worker holds the
        // loft source), so the two runs would differ in whether a native build
        // ran at all — and any chatter from that build reads as a placement
        // difference when it is nothing of the kind. Pinning both to the
        // interpreter makes placement the only thing that changed, which is what
        // the invariant is about.
        .env("LOFT_NO_NATIVE_LIBS", "1")
        .output()
        .expect("failed to invoke loft");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

/// Run `source` + `consumer` under both placements and return the two results.
fn both_placements(name: &str, source: &str, consumer_src: &str) -> (Run, Run) {
    let root = scratch(name);
    let consumer = root.join("consumer.loft");
    std::fs::write(&consumer, consumer_src).expect("write consumer");

    write_library(&root, "inproc", source);
    let inproc = run(&root, &consumer);
    write_library(&root, "process", source);
    let placed = run(&root, &consumer);
    (inproc, placed)
}

fn assert_indistinguishable(what: &str, inproc: &Run, placed: &Run) {
    assert_eq!(
        inproc.stdout, placed.stdout,
        "{what}: placement changed the program's OUTPUT\n\
         --- inproc ---\n{}\n--- process ---\n{}",
        inproc.stdout, placed.stdout
    );
    assert_eq!(
        inproc.stderr, placed.stderr,
        "{what}: placement changed what was reported on stderr (the leak half of \
         the gate)\n--- inproc ---\n{}\n--- process ---\n{}",
        inproc.stderr, placed.stderr
    );
    assert_eq!(
        inproc.code, placed.code,
        "{what}: placement changed the exit status ({} vs {})",
        inproc.code, placed.code
    );
}

#[test]
fn placement_changes_nothing_observable() {
    let library = "pub fn add(a: integer, b: integer) -> integer {\n    a + b\n}\n\
                   pub fn tally(label: text, n: integer) -> integer {\n    len(label) + n\n}\n\
                   pub fn flag(on: boolean) -> boolean {\n    !on\n}\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   println(\"add   = {add(2, 3)}\");\n\
                    \x20   println(\"edge  = {add(-9007199254740993, 1)}\");\n\
                    \x20   println(\"tally = {tally(\"héllo\", 10)}\");\n\
                    \x20   println(\"flag  = {flag(true)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("basic", library, consumer);
    assert_eq!(
        inproc.code, 0,
        "the in-process run must succeed: {}",
        inproc.stderr
    );
    assert_indistinguishable("scalar and text calls", &inproc, &placed);

    // Prove the gate is measuring something: the run really did produce output,
    // so "identical" is not two empty strings agreeing.
    assert!(
        inproc.stdout.contains("add   = 5"),
        "the consumer did not run as expected: {:?}",
        inproc.stdout
    );
}

#[test]
fn a_call_in_a_loop_keeps_its_answer() {
    // State and repetition: the worker holds the library across calls, so an
    // accumulating loop is where a stale frame or a mismatched response would
    // show up as a wrong total rather than a crash.
    let library = "pub fn step(n: integer) -> integer {\n    n * 2 + 1\n}\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..50 {\n\
                    \x20       acc += step(i);\n\
                    \x20   }\n\
                    \x20   println(\"acc = {acc}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("loop", library, consumer);
    assert_eq!(
        inproc.code, 0,
        "the in-process run must succeed: {}",
        inproc.stderr
    );
    assert_indistinguishable("repeated calls", &inproc, &placed);
    assert!(
        inproc.stdout.contains("acc = 2500"),
        "expected the hand-computed total 2500, got {:?}",
        inproc.stdout
    );
}

#[test]
fn a_warning_in_the_library_does_not_decide_whether_it_can_be_placed() {
    // A library that is CORRECT but not diagnostic-free. The worker loads it
    // through `parse_dir`, which used to refuse a directory whose parse
    // reported ANYTHING — so one `never-read` warning made the consumer exit 1
    // under `placement = "process"` and 0 in-process, i.e. placement decided
    // whether the program ran at all. Errors still stop the load; warnings and
    // advice never did gate anything else in loft, and now do not gate this.
    let library = "pub fn ok(a: integer, unused: integer) -> integer {\n    a * 2\n}\n";
    let consumer = "use parity;\nfn main() {\n    println(\"ok = {ok(21, 5)}\");\n}\n";
    let (inproc, placed) = both_placements("warned", library, consumer);
    assert_eq!(
        inproc.code, 0,
        "the in-process run must succeed: {}",
        inproc.stderr
    );
    // The warning has to be REAL, or this test would pass on a library that
    // never had one — the same blindness `the_gate_can_fail` guards against.
    assert!(
        inproc.stderr.contains("never read"),
        "the probe library no longer warns, so it proves nothing: {:?}",
        inproc.stderr
    );
    assert_indistinguishable("a library that warns", &inproc, &placed);
    assert!(inproc.stdout.contains("ok = 42"), "{:?}", inproc.stdout);
}

#[test]
fn the_gate_can_fail() {
    // A parity gate that cannot report a difference would pass forever. Give the
    // two placements DIFFERENT library sources and require the comparison to
    // notice — this is the control for every assertion above.
    let root = scratch("control");
    let consumer = root.join("consumer.loft");
    std::fs::write(
        &consumer,
        "use parity;\nfn main() {\n    println(\"v = {add(2, 3)}\");\n}\n",
    )
    .expect("write consumer");

    write_library(
        &root,
        "inproc",
        "pub fn add(a: integer, b: integer) -> integer { a + b }\n",
    );
    let inproc = run(&root, &consumer);
    write_library(
        &root,
        "process",
        "pub fn add(a: integer, b: integer) -> integer { a + b + 1 }\n",
    );
    let placed = run(&root, &consumer);

    assert_ne!(
        inproc.stdout, placed.stdout,
        "the gate compared two different libraries and saw no difference — it is blind"
    );
}
