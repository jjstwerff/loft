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

/// Narrow integers, at the edges of every declared width.
///
/// These crossed WRONG before @PLN119 arc B: the marshal wrote a narrow value
/// at its storage width into a stack cell that is eight bytes wide, so `-1` as
/// an `i8` came back as `255` and a `u8` argument inherited whatever the
/// previous call had left in the upper seven bytes.  The edges are the point —
/// a sweep of small positive numbers passes on a marshal that gets sign and
/// width entirely wrong.
#[test]
fn every_integer_width_crosses_with_its_sign() {
    let library = "pub fn e_u8(v: u8) -> u8 { v }\n\
                   pub fn e_i8(v: i8) -> i8 { v }\n\
                   pub fn e_u16(v: u16) -> u16 { v }\n\
                   pub fn e_i16(v: i16) -> i16 { v }\n\
                   pub fn e_u32(v: u32) -> u32 { v }\n\
                   pub fn e_i32(v: i32) -> i32 { v }\n\
                   pub fn mixed(a: u8, b: boolean, c: text, d: i16, e: integer) -> integer {\n\
                   \x20   r = e + d + a + len(c);\n\
                   \x20   if b { r += 1000; }\n\
                   \x20   r\n\
                   }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   println(\"u8  {e_u8(255)} {e_u8(0)}\");\n\
                    \x20   println(\"i8  {e_i8(-128)} {e_i8(-1)} {e_i8(127)}\");\n\
                    \x20   println(\"u16 {e_u16(65535)}\");\n\
                    \x20   println(\"i16 {e_i16(-32768)} {e_i16(-1)}\");\n\
                    \x20   println(\"u32 {e_u32(4294967294)}\");\n\
                    \x20   println(\"i32 {e_i32(-2147483648)} {e_i32(-1)}\");\n\
                    \x20   println(\"mix {mixed(200, true, \"abc\", -9, 7)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("narrow", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("narrow integers", &inproc, &placed);
    // Hand-computed, so this is not just two runs agreeing on a wrong answer:
    // 7 + (-9) + 200 + 3 + 1000 = 1201.
    assert!(
        inproc.stdout.contains("i8  -128 -1 127") && inproc.stdout.contains("mix 1201"),
        "the hand-computed values are not what ran: {:?}",
        inproc.stdout
    );
}

/// `single` and a text RETURN — the two shapes arc A refused rather than
/// approximate.
///
/// The text return is the interesting one: it does not come back on the stack,
/// and the call site picks between two conventions depending on where the
/// result is going.  So this exercises both — assigned to a variable, and used
/// in a value position (inside a format string) — plus nesting one call in
/// another, reassigning the same variable, and a loop, because a destination
/// that leaked or was reused would cross two answers rather than lose one.
#[test]
fn single_and_text_returns_cross() {
    let library = "pub fn echo_single(v: single) -> single { v }\n\
                   pub fn scale(v: single, by: integer) -> single { v * by }\n\
                   pub fn shout(s: text) -> text { \"<{s}>\" }\n\
                   pub fn tag(s: text) -> text { \"[{s}]\" }\n\
                   pub fn grow(n: integer) -> text {\n\
                   \x20   out = \"\";\n\
                   \x20   for i in 0..n { out += \"a{i * 0}\"; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn mix(a: integer, b: boolean, c: text, d: single, e: integer) -> text {\n\
                   \x20   \"{a}|{b}|{c}|{d}|{e}\"\n\
                   }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   println(\"s1 {echo_single(0.5 as single)} {echo_single(-2.25 as single)}\");\n\
                    \x20   println(\"s2 {scale(1.5 as single, 4)}\");\n\
                    \x20   v = shout(\"hi\");\n\
                    \x20   println(\"assigned {v}\");\n\
                    \x20   println(\"inline {shout(\"hi\")}\");\n\
                    \x20   println(\"unicode {shout(\"héllo ✓\")}\");\n\
                    \x20   println(\"nested {tag(shout(\"x\"))}\");\n\
                    \x20   println(\"twice {tag(\"a\")}{tag(\"b\")}\");\n\
                    \x20   println(\"long {len(grow(300))}\");\n\
                    \x20   println(\"mix {mix(7, true, \"cc\", 0.5 as single, -9)}\");\n\
                    \x20   r = shout(\"1\");\n\
                    \x20   r = shout(\"2\");\n\
                    \x20   println(\"reassigned {r}\");\n\
                    \x20   acc = \"\";\n\
                    \x20   for i in 0..3 { acc += tag(\"{i}\"); }\n\
                    \x20   println(\"loop {acc}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("single_text", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("single and text returns", &inproc, &placed);
    for expect in [
        "s1 0.5 -2.25",
        "assigned <hi>",
        "inline <hi>",
        "nested [<x>]",
        "twice [a][b]",
        "long 600",
        "mix 7|true|cc|0.5|-9",
        "reassigned <2>",
        "loop [0][1][2]",
    ] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?} in the output, got {:?}",
            inproc.stdout
        );
    }
}

/// A text return the compiler never gave a work buffer — the usual case being a
/// constant, `fn version() -> text { "1.0" }`.
///
/// The caller offers nowhere for such an answer to live, so the function is not
/// placed and runs in-process.  What must hold is that this is INVISIBLE: the
/// program behaves identically either way, which is the whole claim placement
/// makes.  Without the refusal the dispatcher would push a `Str` over a String
/// freed the moment the call returned.
#[test]
fn a_text_return_with_no_work_buffer_still_behaves_identically() {
    let library = "pub fn version() -> text { \"1.0.0\" }\n\
                   pub fn add(a: integer, b: integer) -> integer { a + b }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   println(\"v = {version()}\");\n\
                    \x20   w = version();\n\
                    \x20   println(\"w = {w} / {add(1, 2)}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("constret", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("a constant text return", &inproc, &placed);
    assert!(
        inproc.stdout.contains("v = 1.0.0") && inproc.stdout.contains("w = 1.0.0 / 3"),
        "{:?}",
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

/// `--native` runs a placed library IN-PROCESS, and saying so is the point.
///
/// That backend compiles the library's own body into the whole-program binary,
/// so its calls never leave the process however they were marked.  By the
/// invariant that is the same PROGRAM — but it is not the same ISOLATION, and a
/// deployment that declared `placement = "process"` to contain a crash would
/// have got no worker and no trace of it in any output.  (It was worse than
/// silent: a worker was started per library and then never called.)
///
/// `LOFT_REQUIRE_PLACEMENT=1` is how a deployment insists, so it must refuse
/// here for the same reason it refuses on a platform with no transport.
/// One library, some functions placed and some not — which is the ordinary case,
/// not an edge one, because a real library has signatures on both sides of the
/// line (a struct, a constant text return).
///
/// The two kinds of call sit next to each other on the caller's stack, so this
/// is where a dispatcher that popped the wrong number of cells would show up:
/// not as a failure at the placed call, but as a wrong value in whatever ran
/// next.  Hence the interleaving, and a placed call whose ARGUMENTS come from an
/// unplaceable one.
#[test]
fn placed_and_unplaceable_calls_interleave() {
    let library = "pub fn placed_add(a: integer, b: integer) -> integer { a + b }\n\
                   pub fn version() -> text { \"1.0.0\" }\n\
                   pub struct Point { x: integer, y: integer }\n\
                   pub fn make_point(x: integer, y: integer) -> Point { Point { x: x, y: y } }\n\
                   pub fn sum_point(p: Point) -> integer { p.x + p.y }\n\
                   pub fn placed_txt(s: text) -> text { \"<{s}>\" }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   p = make_point(3, 4);\n\
                    \x20   println(\"a {placed_add(1, 2)} {version()} {sum_point(p)} {placed_txt(\"z\")}\");\n\
                    \x20   println(\"b {placed_add(sum_point(p), len(version()))}\");\n\
                    \x20   println(\"c {placed_txt(version())}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("mixed", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("mixed placeable and unplaceable", &inproc, &placed);
    // Hand-computed: sum_point = 7, len("1.0.0") = 5, so b = 12.
    for expect in ["a 3 1.0.0 7 <z>", "b 12", "c <1.0.0>"] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// @PLN119 arc B — every compound shape, both directions.
///
/// A struct or a vector does not fit in a call frame: it is a graph of records,
/// and it crosses through the shared arena.  The rows here are the composition
/// matrix's type-kind axis, and the ones that matter are the ones whose bytes
/// are NOT plain words in the record — a `text` field is a pointer to a
/// sub-record, a `vector` is a pointer to an element block, and both have to be
/// re-homed into the arena's own store or the receiving side reads a rec-id that
/// means something else over there.
///
/// `vector<text>` is here on purpose: @PLN97's paged relocator REFUSES it,
/// because that path moves a matched entry into a different store and each
/// element's pointer would dangle.  The arena is not that path — both sides map
/// the same store — so the refusal does not apply, and this is what says so.
#[test]
fn compound_values_cross_in_both_directions() {
    let library = "pub struct P { x: integer, y: integer }\n\
                   pub struct N { id: integer, label: text }\n\
                   pub struct O { tag: text, inner: P, tail: vector<integer> }\n\
                   pub fn sum_p(p: P) -> integer { p.x + p.y }\n\
                   pub fn make_p(x: integer, y: integer) -> P { P { x: x, y: y } }\n\
                   pub fn describe(n: N) -> text { \"{n.id}:{n.label}\" }\n\
                   pub fn make_n(id: integer, label: text) -> N { N { id: id, label: label } }\n\
                   pub fn sum_v(v: vector<integer>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn make_o(tag: text, n: integer) -> O {\n\
                   \x20   v: vector<integer> = [];\n\
                   \x20   for i in 0..n { v += [i * 2]; }\n\
                   \x20   O { tag: tag, inner: P { x: n, y: n * 10 }, tail: v }\n\
                   }\n\
                   pub fn show_o(o: O) -> text {\n\
                   \x20   t = 0;\n\
                   \x20   for e in o.tail { t += e; }\n\
                   \x20   \"{o.tag}|{o.inner.x + o.inner.y}|{t}|{len(o.tail)}\"\n\
                   }\n\
                   pub fn names(n: integer) -> vector<N> {\n\
                   \x20   out: vector<N> = [];\n\
                   \x20   for i in 0..n { out += [N { id: i, label: \"n{i}\" }]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn join_names(v: vector<N>) -> text {\n\
                   \x20   s = \"\";\n\
                   \x20   for e in v { s += \"{e.id}={e.label};\"; }\n\
                   \x20   s\n\
                   }\n\
                   pub fn words(n: integer) -> vector<text> {\n\
                   \x20   out: vector<text> = [];\n\
                   \x20   for i in 0..n { out += [\"w{i}\"]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn join_words(v: vector<text>) -> text {\n\
                   \x20   s = \"\";\n\
                   \x20   for e in v { s += \"{e},\"; }\n\
                   \x20   s\n\
                   }\n\
                   pub fn maybe(p: P?) -> integer { if p == null { -1 } else { p.x + p.y } }\n\
                   pub fn nested(n: integer) -> vector<vector<integer>> {\n\
                   \x20   out: vector<vector<integer>> = [];\n\
                   \x20   for i in 0..n {\n\
                   \x20       row: vector<integer> = [];\n\
                   \x20       for j in 0..n { row += [i * 10 + j]; }\n\
                   \x20       out += [row];\n\
                   \x20   }\n\
                   \x20   out\n\
                   }\n\
                   pub fn sum_nested(v: vector<vector<integer>>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for row in v { for e in row { t += e; } }\n\
                   \x20   t\n\
                   }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   p = make_p(3, 4);\n\
                    \x20   println(\"struct {sum_p(p)} {p.x} {p.y}\");\n\
                    \x20   n = make_n(7, \"héllo\");\n\
                    \x20   println(\"text-field {describe(n)} / {n.id} {n.label}\");\n\
                    \x20   v = range_v(5);\n\
                    \x20   println(\"vector {sum_v(v)} {len(v)} {v[4]}\");\n\
                    \x20   o = make_o(\"tag\", 4);\n\
                    \x20   println(\"nested-struct {show_o(o)} / {o.inner.y} {o.tail[3]}\");\n\
                    \x20   vn = names(3);\n\
                    \x20   println(\"vec-struct {join_names(vn)} {len(vn)} {vn[1].label}\");\n\
                    \x20   vw = words(3);\n\
                    \x20   println(\"vec-text {join_words(vw)} {len(vw)} {vw[2]}\");\n\
                    \x20   empty: vector<integer> = [];\n\
                    \x20   println(\"empty {sum_v(empty)} {len(empty)}\");\n\
                    \x20   absent: P? = null;\n\
                    \x20   println(\"null {maybe(p)} {maybe(absent)}\");\n\
                    \x20   g = nested(3);\n\
                    \x20   println(\"depth2 {sum_nested(g)} {len(g)} {g[1][1]}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("compound", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("compound values", &inproc, &placed);
    // Hand-computed, so this is not two runs agreeing on a wrong answer:
    // sum_p = 7; sum_v(0..5) = 10; show_o("tag",4) = inner 4+40 = 44, tail
    // 0+2+4+6 = 12, len 4; nested(3) sums 0+1+2 + 10+11+12 + 20+21+22 = 99.
    for expect in [
        "struct 7 3 4",
        "text-field 7:héllo / 7 héllo",
        "vector 10 5 4",
        "nested-struct tag|44|12|4 / 40 6",
        "vec-struct 0=n0;1=n1;2=n2; 3 n1",
        "vec-text w0,w1,w2, 3 w2",
        "empty 0 0",
        "null 7 -1",
        "depth2 99 3 11",
    ] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// The cell a copy-only crossing fails.
///
/// loft passes a compound BY REFERENCE: `pub fn bump(p: P)` that assigns `p.x`
/// changes the CALLER's `p`, and a vector parameter that is appended to grows the
/// caller's vector.  A marshal that copied the argument over and stopped would
/// leave both silently unchanged under `placement = "process"` — a divergence
/// with no error, no warning, and a plausible-looking answer.
///
/// The text case is the sharper one: the field is a pointer to a sub-record, so
/// the write has to come back re-homed into the caller's store rather than as a
/// rec-id that means something else there.
#[test]
fn a_callee_writing_to_a_compound_parameter_is_seen_by_the_caller() {
    let library = "pub struct P { x: integer, label: text }\n\
                   pub fn bump(p: P) -> integer {\n\
                   \x20   p.x = p.x + 100;\n\
                   \x20   p.label = \"{p.label}!\";\n\
                   \x20   p.x\n\
                   }\n\
                   pub fn push(v: vector<integer>) -> integer {\n\
                   \x20   v += [99];\n\
                   \x20   len(v)\n\
                   }\n\
                   pub fn both(a: P, b: P) -> integer {\n\
                   \x20   a.x = a.x + 1;\n\
                   \x20   b.x = b.x + 10;\n\
                   \x20   a.x + b.x\n\
                   }\n\
                   pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   p = P { x: 11, label: \"a\" };\n\
                    \x20   r = bump(p);\n\
                    \x20   println(\"write {r} caller-sees {p.x} {p.label}\");\n\
                    \x20   v = range_v(3);\n\
                    \x20   n = push(v);\n\
                    \x20   println(\"append {n} caller-sees {len(v)} {v[3]}\");\n\
                    \x20   q = P { x: 5, label: \"q\" };\n\
                    \x20   println(\"alias {both(q, q)} caller-sees {q.x}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("byref", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("a written-to compound parameter", &inproc, &placed);
    // `both(q, q)` passes ONE record twice, so the two writes compound: 5 → 6 →
    // 16, and `a.x + b.x` reads the same cell twice = 32.  Two independent
    // copies would answer 6 + 15 = 21 and leave the caller at 6 or 15.
    for expect in [
        "write 111 caller-sees 111 a!",
        "append 4 caller-sees 4 99",
        "alias 32 caller-sees 16",
    ] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// A value far larger than the arena's initial size, and enough repetitions that
/// a per-call leak would show.
///
/// Growth is where the two mappings can disagree: a claim that outgrows the file
/// resizes and re-mmaps it in the WRITER, and the reader's mapping still covers
/// the old length — reading past it is a `SIGBUS`, not a wrong answer.  So the
/// writer publishes its word count with every frame and the reader maps again.
///
/// The loop is the other half: the arena is reset per call rather than freed
/// record by record, so a reset that did not actually reclaim would show here as
/// a growing file rather than as a wrong value.
#[test]
fn a_value_that_outgrows_the_arena_still_crosses() {
    let library = "pub struct N { id: integer, label: text }\n\
                   pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn sum_v(v: vector<integer>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn names(n: integer) -> vector<N> {\n\
                   \x20   out: vector<N> = [];\n\
                   \x20   for i in 0..n { out += [N { id: i, label: \"n{i}\" }]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn count(v: vector<N>) -> integer { len(v) }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   big = range_v(200000);\n\
                    \x20   println(\"big {len(big)} {big[199999]} {sum_v(big)}\");\n\
                    \x20   ns = names(20000);\n\
                    \x20   println(\"names {count(ns)} {ns[19999].label}\");\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..500 {\n\
                    \x20       v = range_v(50);\n\
                    \x20       acc += sum_v(v);\n\
                    \x20   }\n\
                    \x20   println(\"loop {acc}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("grow", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("a value larger than the arena", &inproc, &placed);
    // Hand-computed: sum 0..200000 = 19999900000; 500 × sum(0..50) = 500 × 1225.
    for expect in [
        "big 200000 199999 19999900000",
        "names 20000 n19999",
        "loop 612500",
    ] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

/// The leak half of the gate, made falsifiable.
///
/// `check_store_leaks` reports what is unfreed at EXIT, so it cannot see a
/// program that allocates a store per call and frees it — which is exactly the
/// shape a placed call has, and exactly the shape that runs a long program out
/// of store slots.  `LOFT_STRICT_STORES` is what makes it visible: it stops the
/// allocator recycling a released slot, so the peak slot count becomes a
/// straight count of how many stores a run ever needed, and the two placements
/// have to agree on it.
///
/// They did not.  A placed struct return minted its destination store while the
/// call arena was registered, so the arena's slots could never come back below
/// it; and adopting the arena PUSHED a slot rather than taking the one at the
/// watermark, so the table walked upward once per call.  Together that was two
/// slots per call against four for the whole in-process run — all 65535 gone
/// after ~32k iterations, with the same loop in-process flat.
///
/// Comparing the numbers rather than asserting a constant is what keeps this
/// honest: it fails if EITHER side regresses, and it cannot pass by both being
/// wrong in the same direction, because in-process placement is not doing
/// anything this plan changed.
#[test]
fn placement_does_not_change_how_many_stores_a_run_needs() {
    let library = "pub struct P { x: integer, y: integer }\n\
                   pub fn make_p(x: integer) -> P { P { x: x, y: x * 2 } }\n\
                   pub fn sum_p(p: P) -> integer { p.x + p.y }\n\
                   pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..2000 {\n\
                    \x20       p = make_p(i);\n\
                    \x20       acc += sum_p(p);\n\
                    \x20       v = range_v(4);\n\
                    \x20       acc += len(v);\n\
                    \x20   }\n\
                    \x20   println(\"acc {acc}\");\n\
                    }\n";
    let root = scratch("slots");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(&consumer_path, consumer).expect("write consumer");

    let peak_of = |mode: &str| -> (u32, String) {
        write_library(&root, mode, library);
        let out = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--interpret")
            .arg("--lib")
            .arg(root.join("libs"))
            .arg(&consumer_path)
            .env("LOFT_TIMEOUT", "120")
            .env("LOFT_NO_NATIVE_LIBS", "1")
            .env("LOFT_STRICT_STORES", "1")
            .env("LOFT_ALLOC_REPORT", "1")
            .output()
            .expect("failed to invoke loft");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let peak = stderr
            .lines()
            .find_map(|l| l.strip_prefix("loft-alloc: peak="))
            .and_then(|r| r.split_whitespace().next())
            .and_then(|n| n.parse::<u32>().ok())
            .unwrap_or_else(|| panic!("no alloc report under {mode}: {stderr}"));
        (peak, String::from_utf8_lossy(&out.stdout).into_owned())
    };

    let (inproc_peak, inproc_out) = peak_of("inproc");
    let (placed_peak, placed_out) = peak_of("process");
    assert_eq!(
        inproc_out, placed_out,
        "the two runs did not even produce the same answer"
    );
    // 2000 iterations, so a per-call slot lost is a peak in the thousands. The
    // in-process side is the reference; a small constant either way would be a
    // real difference in how many stores are live AT ONCE, which is not what
    // this is measuring.
    assert!(
        placed_peak <= inproc_peak + 4,
        "placement needed {placed_peak} store slots where in-process needed \
         {inproc_peak} — a slot per call is a table that runs out"
    );
    assert!(
        inproc_peak < 100,
        "the in-process reference is itself growing ({inproc_peak}), so this \
         test is measuring nothing"
    );
}

/// A signature the arena does not carry runs in-process, and that has to be
/// INVISIBLE — which is the whole claim placement makes.
///
/// A polymorphic enum and a keyed collection are both reference-shaped, so they
/// look placeable from a distance; they are refused because their crossing has
/// questions of its own (an enum's payload type is a runtime discriminant, and a
/// keyed collection carries an index whose ordering is the caller's).  The risk
/// is not that they fail — it is that they are quietly marked and then read as
/// the wrong shape, so this pins the refusal by requiring the program to be
/// identical either way.
#[test]
fn a_compound_the_arena_does_not_carry_still_behaves_identically() {
    let library = "pub struct P { a: integer, b: integer }\n\
                   pub enum Shape { Circle { r: integer }, Square { s: integer } }\n\
                   pub fn area(sh: Shape) -> integer {\n\
                   \x20   match sh { Circle { r } => r * r * 3, Square { s } => s * s }\n\
                   }\n\
                   pub fn make_circle(r: integer) -> Shape { Circle { r: r } }\n\
                   pub fn tally(h: hash<P[a]>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in h { t += e.b; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn sum_p(p: P) -> integer { p.a + p.b }\n";
    let consumer = "use parity;\n\
                    fn main() {\n\
                    \x20   c = make_circle(4);\n\
                    \x20   println(\"enum {area(c)}\");\n\
                    \x20   h: hash<P[a]> = [];\n\
                    \x20   h += [P { a: 1, b: 10 }];\n\
                    \x20   h += [P { a: 2, b: 20 }];\n\
                    \x20   println(\"hash {tally(h)}\");\n\
                    \x20   println(\"placed {sum_p(P { a: 6, b: 7 })}\");\n\
                    }\n";
    let (inproc, placed) = both_placements("unsupported", library, consumer);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    assert_indistinguishable("an unsupported compound", &inproc, &placed);
    for expect in ["enum 48", "hash 30", "placed 13"] {
        assert!(
            inproc.stdout.contains(expect),
            "expected {expect:?}, got {:?}",
            inproc.stdout
        );
    }
}

#[test]
fn native_does_not_place_and_says_so_when_asked_to_insist() {
    let root = scratch("native_require");
    let consumer = root.join("consumer.loft");
    std::fs::write(
        &consumer,
        "use parity;\nfn main() {\n    println(\"v = {add(2, 3)}\");\n}\n",
    )
    .expect("write consumer");
    write_library(
        &root,
        "process",
        "pub fn add(a: integer, b: integer) -> integer { a + b }\n",
    );

    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(&consumer)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_REQUIRE_PLACEMENT", "1")
        .output()
        .expect("failed to invoke loft");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "insisting on placement under --native must refuse; it exited {:?} with {stderr}",
        out.status.code()
    );
    assert!(
        stderr.contains("LOFT_REQUIRE_PLACEMENT") && stderr.contains("--native"),
        "the refusal must name the reason, not just fail: {stderr}"
    );
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
