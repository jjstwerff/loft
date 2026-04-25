// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Characterisation suite for `par` — pins down the current behaviour
//! before plan-06's typed-par redesign refactors the runtime.
//!
//! Each test exercises a specific (input element type) × (output
//! element type) × (thread count) × (worker form) combination and
//! asserts the exact current output.  Subsequent plan-06 phases must
//! keep these tests passing byte-for-byte; any deliberate behaviour
//! change updates the corresponding fixture in the same commit.
//!
//! The matrix is per `doc/claude/plans/06-typed-par/00-baseline-and-bench.md`.
//!
//! # Pre-existing input-element-stride bug (P189-class — discovered
//! while writing this suite)
//!
//! Today's `par(...)` runtime mishandles **primitive-element input
//! vectors** — `vector<integer>`, `vector<float>`, `vector<i32>`,
//! `vector<u8>`, etc.  Each worker reads its input slice with a
//! 12-byte (DbRef) stride regardless of the vector's actual element
//! width, so workers receive garbage from adjacent memory.
//!
//! Confirmed: plain `for x in items` (no par) over a `vector<integer>`
//! works correctly; only par's worker-dispatch is affected.  The bug
//! has been latent since the C54 / P184 narrow-vector migration
//! because every existing par fixture in `tests/scripts/22-threading.loft`
//! uses `vector<Score>` (struct ref) input.
//!
//! Plan-06 phase 4 (typed input/output) is the natural place to fix
//! this — once the runtime reads element stride from the type system
//! instead of a parser-computed integer, primitive-element inputs
//! work uniformly with struct-element inputs.
//!
//! Tests below that exercise primitive-element inputs are gated
//! `#[ignore = "..."]` with the tracking note, so the regression
//! suite will catch it on the day phase 4 lands and the tests get
//! un-`#[ignore]`d.
//!
//! Working tests in this file all use `vector<Score>`-shaped inputs
//! (single-field struct wrapping the underlying scalar) to match
//! today's only-tested path.

mod testing;

use loft::data::Value;
use testing::*;

// Shared test scaffolding: `Score { value }` wraps an integer; every
// per-cell test instantiates a fresh `vector<Score>` and runs a
// worker fn that operates on the score, returning the per-cell
// output type.

const SCORE_DEFS: &str = "struct Score { value: integer }
";

// ── primitive scalar return paths (Score input, primitive output) ──

#[test]
fn par_struct_to_int_t1() {
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 10 }, Score { value: 20 }, Score { value: 30 }];
    sum = 0;
    for s in sl par(r = dbl(s), 1) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(120));
}

#[test]
fn par_struct_to_int_t4() {
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [
        Score { value: 10 }, Score { value: 20 }, Score { value: 30 }, Score { value: 40 },
        Score { value: 50 }, Score { value: 60 }, Score { value: 70 }, Score { value: 80 }
    ];
    sum = 0;
    for s in sl par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(720));
}

#[test]
fn par_struct_to_float_t4() {
    code!(
        "struct Score { value: integer }
fn half(s: const Score) -> float { s.value as float / 2.0 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 10 }, Score { value: 20 }, Score { value: 30 }, Score { value: 40 }];
    total = 0.0;
    for s in sl par(r = half(s), 4) { total += r; }
    total as integer
}"
    )
    .expr("run()")
    .result(Value::Int(50));
}

#[test]
fn par_struct_to_single_t4() {
    code!(
        "struct Score { value: integer }
fn quad(s: const Score) -> single { (s.value * 4) as single }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    sum = 0;
    for s in sl par(r = quad(s), 4) { sum += r as integer; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(40));
}

#[test]
fn par_struct_to_i32_t4() {
    code!(
        "struct Score { value: integer }
fn neg(s: const Score) -> i32 { (-s.value) as i32 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    sum = 0;
    for s in sl par(r = neg(s), 4) { sum += r as integer; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(-10));
}

#[test]
fn par_struct_to_byte_t4() {
    code!(
        "struct Score { value: integer }
fn small(s: const Score) -> u8 { s.value as u8 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }, Score { value: 5 }];
    sum = 0;
    for s in sl par(r = small(s), 4) { sum += r as integer; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(15));
}

#[test]
fn par_struct_to_bool_t4() {
    code!(
        "struct Score { value: integer }
fn pos(s: const Score) -> boolean { s.value > 0 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: -2 }, Score { value: -1 }, Score { value: 0 }, Score { value: 1 }, Score { value: 2 }];
    count = 0;
    for s in sl par(r = pos(s), 4) { if r { count += 1; } }
    count
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

#[test]
fn par_struct_to_text_t4() {
    code!(
        "struct Score { value: integer }
fn label(s: const Score) -> text { \"v{s.value}\" }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    total_len = 0;
    for s in sl par(r = label(s), 4) { total_len += len(r); }
    total_len
}"
    )
    .expr("run()")
    .result(Value::Int(8));
}

// ── reference / struct return paths ────────────────────────────────────

#[test]
fn par_struct_to_struct_t4() {
    code!(
        "struct Score { value: integer }
struct Point { x: integer, y: integer }
fn make_point(s: const Score) -> Point { Point { x: s.value, y: s.value + 1 } }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    sum = 0;
    for s in sl par(p = make_point(s), 4) { sum += p.x + p.y; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(24));
}

#[test]
fn par_struct_to_enum_t4() {
    code!(
        "struct Score { value: integer }
enum Sign { Neg, Zero, Pos }
fn classify(s: const Score) -> Sign {
    if s.value < 0 { Neg } else if s.value == 0 { Zero } else { Pos }
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: -2 }, Score { value: -1 }, Score { value: 0 }, Score { value: 1 }, Score { value: 2 }];
    pos_count = 0;
    for s in sl par(g = classify(s), 4) { if g == Pos { pos_count += 1; } }
    pos_count
}"
    )
    .expr("run()")
    .result(Value::Int(2));
}

// ── degenerate inputs ──────────────────────────────────────────────────

#[test]
fn par_empty_input() {
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [];
    sum = 0;
    for s in sl par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(0));
}

#[test]
fn par_single_element_t4() {
    // Worker count clamps to 1 internally but the surface accepts 4.
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 42 }];
    sum = 0;
    for s in sl par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(84));
}

#[test]
fn par_max_threads_overprovisioned() {
    // Threads > input length: extra workers should immediately exit.
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }, Score { value: 5 }];
    sum = 0;
    for s in sl par(r = dbl(s), 16) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(30));
}

// ── Form 2: method-style worker (`.method`) ────────────────────────────

#[test]
fn par_form2_method() {
    // Note: under the `code!()` harness, the parser warns "Variable s
    // is never read" when the body doesn't reference `s` (it sees the
    // method-receiver `s.dbl()` as a structural call, not a read of s).
    // 22-threading.loft sidesteps this by always using both x and r in
    // the body; we acknowledge the warning here so the test passes.
    code!(
        "struct Score { value: integer }
fn dbl(self: const Score) -> integer { self.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }];
    sum = 0;
    for s in sl par(r = s.dbl(), 4) { sum += r; }
    sum
}"
    )
    .warning("Variable s is never read at par_form2_method:6:26")
    .expr("run()")
    .result(Value::Int(12));
}

// ── correctness — order independence within the result vector ─────────

#[test]
fn par_results_pair_with_inputs_in_order() {
    // Body sees x and r paired in input order — fused-form invariant
    // that plan-06 must preserve.
    code!(
        "struct Score { value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 10 }, Score { value: 20 }, Score { value: 30 }, Score { value: 40 }];
    weighted = 0;
    pos = 1;
    for s in sl par(r = dbl(s), 4) {
        weighted += pos * r;
        pos += 1;
    }
    weighted
}"
    )
    .expr("run()")
    .result(Value::Int(600));
}

// ── auto-light heuristic — observable via execution-correctness ────────

#[test]
fn par_pure_arithmetic_worker_runs_clean() {
    // Pure-arithmetic worker: no allocation, no shared writes.
    // Today's parser-side check_light_eligible classifies this as light;
    // the auto-light analyser in plan-06 phase 5 must continue to.
    code!(
        "struct Score { value: integer }
fn cube(s: const Score) -> integer { s.value * s.value * s.value }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }, Score { value: 4 }];
    sum = 0;
    for s in sl par(r = cube(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(100));
}

// ─────────────────────────────────────────────────────────────────────
// Shapes that don't work today.  Each `#[ignore]` entry is a canary
// — un-`#[ignore]` it when the relevant plan-06 phase lands and the
// shape starts working.  The plan file is the authoritative inventory
// of these gaps with their fix targets; see
// `doc/claude/plans/06-typed-par/01-output-store.md § Surface gaps
// closed by phase 1`.  We do NOT file one PROBLEMS.md entry per gap —
// the plan is the single source of truth.
// ─────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "par-primitive-input: vector<integer> input gives garbage; planned fix in plan-06 phase 4 (typed input/output)"]
fn par_int_to_int_t4_primitive_input() {
    code!(
        "fn dbl(x: integer) -> integer { x * 2 }
fn run() -> integer {
    items: vector<integer> = [10, 20, 30, 40, 50, 60, 70, 80];
    sum = 0;
    for x in items par(r = dbl(x), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(720));
}

#[test]
#[ignore = "par-primitive-input: vector<float> input gives garbage; planned fix in plan-06 phase 4"]
fn par_float_input_t4() {
    code!(
        "fn dbl(x: float) -> float { x * 2.0 }
fn run() -> integer {
    items: vector<float> = [1.0, 2.0, 3.0, 4.0];
    total = 0.0;
    for x in items par(r = dbl(x), 4) { total += r; }
    total as integer
}"
    )
    .expr("run()")
    .result(Value::Int(20));
}

#[test]
#[ignore = "par-primitive-input: vector<i32> input gives garbage; planned fix in plan-06 phase 4"]
fn par_i32_input_t4() {
    code!(
        "fn dbl(x: i32) -> integer { (x as integer) * 2 }
fn run() -> integer {
    items: vector<i32> = [10 as i32, 20 as i32, 30 as i32, 40 as i32];
    sum = 0;
    for x in items par(r = dbl(x), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(200));
}

#[test]
#[ignore = "par-primitive-input: vector<u8> input gives garbage; planned fix in plan-06 phase 4"]
fn par_u8_input_t4() {
    code!(
        "fn dbl(x: u8) -> integer { (x as integer) * 2 }
fn run() -> integer {
    items: vector<u8> = [1 as u8, 2 as u8, 3 as u8, 4 as u8];
    sum = 0;
    for x in items par(r = dbl(x), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(20));
}

#[test]
#[ignore = "par-primitive-input: vector<text> input gives garbage; planned fix in plan-06 phase 4"]
fn par_text_input_t4() {
    code!(
        "fn count_chars(s: text) -> integer { len(s) }
fn run() -> integer {
    items: vector<text> = [\"hi\", \"hello\", \"x\", \"abc\"];
    sum = 0;
    for x in items par(r = count_chars(x), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(11));
}

#[test]
#[ignore = "par-struct-enum-return: parser rejects size > 8; planned fix in plan-06 phase 1 (per-worker output stores remove the fixed-size dispatch)"]
fn par_struct_to_struct_enum_t4() {
    // Struct-enum (variant with fields) currently fires the parser
    // diagnostic `Parallel worker return type '<Enum>' (size N) is
    // not supported` because the runtime hard-codes return-type
    // dispatch on size <= 8.  After plan-06 phase 1 lands, workers
    // write into per-worker output Stores using normal struct-write
    // ops; arbitrary variant payloads work uniformly.
    code!(
        "struct Score { value: integer }
enum Verdict {
    Pass { score: integer },
    Fail { reason: text }
}
fn classify(s: const Score) -> Verdict {
    if s.value >= 0 { Pass { score: s.value } } else { Fail { reason: \"negative\" } }
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 10 }, Score { value: -5 }, Score { value: 20 }];
    pass_sum = 0;
    for s in sl par(v = classify(s), 4) {
        if v is Pass { score } { pass_sum += score; }
    }
    pass_sum
}"
    )
    .expr("run()")
    .result(Value::Int(30));
}

// ── phase 0d canaries — additional D11 spectrum entries ─────────────
// Each test asserts the eventual correct behaviour.  Today the
// shape either fails to compile or returns garbage; the relevant
// plan-06 phase un-`#[ignore]`s the test as the gap closes.

#[test]
#[ignore = "par-enum-input: vector<EnumTag> input untested; planned canary for plan-06 phase 1 (type-driven element stride)"]
fn par_enum_input_t4() {
    code!(
        "enum Color { Red, Green, Blue }
fn idx(c: Color) -> integer { if c == Red { 1 } else if c == Green { 2 } else { 3 } }
fn run() -> integer {
    items: vector<Color> = [Red, Green, Blue, Red];
    sum = 0;
    for c in items par(r = idx(c), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(7));
}

#[test]
#[ignore = "par-vec-of-refs-input: vector<Reference<X>> input untested; planned canary for plan-06 phase 1"]
fn par_vec_of_refs_input_t4() {
    // Workers receive a Reference (DbRef into the parent's store)
    // and return a derived value.  Distinct from vector<Struct>
    // because the elements are ref-typed, not inline struct.
    code!(
        "struct Item { value: integer }
struct Holder { i: reference Item }
fn from_holder(h: const Holder) -> integer { h.i.value * 2 }
fn run() -> integer {
    a: Item = Item { value: 10 };
    b: Item = Item { value: 20 };
    items: vector<Holder> = [Holder { i: a }, Holder { i: b }];
    sum = 0;
    for h in items par(r = from_holder(h), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(60));
}

#[test]
#[ignore = "par-nested-vector-input: vector<vector<T>> input untested; planned canary for plan-06 phase 4"]
fn par_nested_vector_input_t4() {
    // Worker iterates the inner vector.  Outer is the par input.
    code!(
        "struct Bag { items: vector<integer> }
fn sum_bag(b: const Bag) -> integer {
    s = 0;
    for x in b.items { s += x; }
    s
}
fn run() -> integer {
    bags: vector<Bag> = [
        Bag { items: [1, 2, 3] },
        Bag { items: [10, 20] },
        Bag { items: [100] }
    ];
    total = 0;
    for b in bags par(r = sum_bag(b), 4) { total += r; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(136));
}

#[test]
#[ignore = "par-vec-of-fns-input: vector<fn(...) -> T> input untested; planned canary for plan-06 phase 1"]
fn par_vec_of_fns_input_t4() {
    // Workers receive a fn-ref and call it on a fixed input.
    // Whether vector<fn> is even constructable today is part of
    // what the canary documents.
    code!(
        "fn dbl(x: integer) -> integer { x * 2 }
fn triple(x: integer) -> integer { x * 3 }
fn quad(x: integer) -> integer { x * 4 }
fn apply(f: fn(integer) -> integer) -> integer { f(10) }
fn run() -> integer {
    fs: vector<fn(integer) -> integer> = [dbl, triple, quad];
    total = 0;
    for f in fs par(r = apply(f), 4) { total += r; }
    total
}"
    )
    .expr("run()")
    .result(Value::Int(90));
}

#[test]
#[ignore = "par-sorted-input: sorted<T[key]> input untested; planned canary for plan-06 phase 4 (typed input)"]
fn par_sorted_input_t4() {
    code!(
        "struct Score { value: integer not null }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    sorted_items: sorted<Score[value]> = [];
    sorted_items += Score { value: 30 };
    sorted_items += Score { value: 10 };
    sorted_items += Score { value: 20 };
    sum = 0;
    for s in sorted_items par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(120));
}

#[test]
#[ignore = "par-hash-input: hash<T[key]> input untested; planned canary for plan-06 phase 4"]
fn par_hash_input_t4() {
    code!(
        "struct Score { name: text not null, value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    h: hash<Score[name]> = [];
    h += Score { name: \"a\", value: 10 };
    h += Score { name: \"b\", value: 20 };
    h += Score { name: \"c\", value: 30 };
    sum = 0;
    for s in h par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(120));
}

#[test]
#[ignore = "par-index-input: index<T[key]> input untested; planned canary for plan-06 phase 4"]
fn par_index_input_t4() {
    code!(
        "struct Score { name: text not null, value: integer }
fn dbl(s: const Score) -> integer { s.value * 2 }
fn run() -> integer {
    ix: index<Score[name]> = [];
    ix += Score { name: \"a\", value: 10 };
    ix += Score { name: \"b\", value: 20 };
    sum = 0;
    for s in ix par(r = dbl(s), 4) { sum += r; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(60));
}

#[test]
#[ignore = "par-large-struct-return: worker returning a value-struct > 8 bytes is rejected; planned fix in plan-06 phase 1"]
fn par_struct_to_large_struct_t4() {
    // Today's parser-side check_light_eligible / parse_parallel_for
    // rejects any return type with var_size > 8 unless it's a
    // Reference.  A large by-value struct hits this.
    code!(
        "struct Score { value: integer }
struct Wide { a: integer, b: integer, c: integer }
fn make_wide(s: const Score) -> Wide { Wide { a: s.value, b: s.value * 2, c: s.value * 3 } }
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }, Score { value: 3 }];
    sum = 0;
    for s in sl par(w = make_wide(s), 4) { sum += w.a + w.b + w.c; }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(36));
}

#[test]
#[ignore = "par-vector-return: worker returning vector<T> rejected today; planned fix in plan-06 phase 1"]
fn par_struct_to_vector_t4() {
    // Worker constructs and returns a vector per element.  Today
    // the runtime can't represent this.  After phase 1, the per-
    // worker output store holds vector records like any other.
    code!(
        "struct Score { value: integer }
fn replicate(s: const Score) -> vector<integer> {
    out: vector<integer> = [];
    for i in 0..s.value { out += [i]; }
    out
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 2 }, Score { value: 3 }];
    total_len = 0;
    for s in sl par(v = replicate(s), 4) { total_len += len(v); }
    total_len
}"
    )
    .expr("run()")
    .result(Value::Int(5));
}

#[test]
#[ignore = "par-keyed-collection-return: worker returning hash/sorted/index rejected; planned fix in plan-06 phase 1"]
fn par_struct_to_keyed_collection_t4() {
    code!(
        "struct Score { value: integer }
struct Tag { id: integer not null, label: text }
fn build_tags(s: const Score) -> sorted<Tag[id]> {
    out: sorted<Tag[id]> = [];
    out += Tag { id: s.value, label: \"v{s.value}\" };
    out += Tag { id: s.value + 100, label: \"hi\" };
    out
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: 2 }];
    total_len = 0;
    for s in sl par(t = build_tags(s), 4) { total_len += len(t); }
    total_len
}"
    )
    .expr("run()")
    .result(Value::Int(4));
}

#[test]
#[ignore = "par-fn-return: worker returning a fn-ref untested; planned canary for plan-06 phase 1"]
fn par_struct_to_fn_t4() {
    // Worker selects and returns a fn-ref based on its input.
    code!(
        "struct Score { value: integer }
fn dbl(x: integer) -> integer { x * 2 }
fn triple(x: integer) -> integer { x * 3 }
fn pick(s: const Score) -> fn(integer) -> integer {
    if s.value > 0 { dbl } else { triple }
}
fn run() -> integer {
    sl: vector<Score> = [Score { value: 1 }, Score { value: -1 }, Score { value: 2 }];
    sum = 0;
    for s in sl par(f = pick(s), 4) { sum += f(10); }
    sum
}"
    )
    .expr("run()")
    .result(Value::Int(70));
}
