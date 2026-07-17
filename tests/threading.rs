// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![cfg(feature = "threading")]

//! Integration tests for parallel execution.
//!
//! Each test compiles a loft program, builds a vector in Rust, and calls
//! `parallel::run_parallel_int` (or `_text`) to verify that workers execute
//! correctly and return results.  Plan-06 phase 4b' retired the loft-side
//! `parallel_for_int` (string-based dispatch) but kept the Rust-side
//! `run_parallel_int` helper alive specifically for these tests — they
//! exercise the parallel runtime independent of the parser/codegen stack.

extern crate loft;

use loft::compile::byte_code;
use loft::database::Stores;
use loft::keys::DbRef;
use loft::parallel::{
    WorkerProgram, run_parallel_discard, run_parallel_fold, run_parallel_int, run_parallel_queue,
    run_parallel_queue_ref, run_parallel_text,
};
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;

/// Parse default library + given loft code; compile to bytecode; return State + Data.
fn compile(code: &str) -> (State, loft::data::Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse_str(code, "threading_test", false);
    assert!(
        p.diagnostics.lines().is_empty(),
        "Parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    (state, p.data)
}

/// Build an integer vector in a fresh store inside `stores` and return the
/// `DbRef` pointing to the vector "header" field.
///
/// Element size is 8 bytes post-2c (a single `integer` = i64 per element).
fn build_int_vector(stores: &mut Stores, values: &[i32]) -> DbRef {
    let db = stores.null(); // allocate an empty store
    let n = values.len() as u32;
    // Vector data record: fld=4 count, fld=8+ elements (8B stride post-2c).
    let vec_words = (n * 8 + 15) / 8;
    let vec_words = vec_words.max(1);
    let vec_cr = stores.claim(&db, vec_words);
    let vec_rec = vec_cr.rec;
    // Header record holds the vector pointer at fld=4.
    let header_cr = stores.claim(&db, 1);
    let header_rec = header_cr.rec;

    {
        let store = stores.store_mut(&db);
        store.set_u32_raw(vec_rec, 4, n);
        for (i, &v) in values.iter().enumerate() {
            store.set_int(vec_rec, 8 + i as u32 * 8, i64::from(v));
        }
        store.set_u32_raw(header_rec, 4, vec_rec);
    }

    DbRef {
        store_nr: db.store_nr,
        rec: header_rec,
        pos: 4,
    }
}

/// Build a `WorkerProgram` snapshot from a fully compiled `State`.
fn worker_program(state: &State) -> WorkerProgram {
    state.worker_program()
}

/// A simple worker that reads back an integer stored directly as the element.
/// The vector is built with `build_int_vector` where each element IS an i32.
/// The worker function reads field 0 from the passed reference (= the element value).
#[test]
fn parallel_returns_correct_values_single_thread() {
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, data) = compile(code);

    let values: Vec<i32> = vec![10, 20, 30, 40, 50];
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_worker_id");
    assert_ne!(d_nr, u32::MAX, "worker function not found");
    let fn_pos = data.def(d_nr).code_position;

    let program = worker_program(&state);
    let results = run_parallel_int(&state.database, program, fn_pos, &input, 8, 1);
    let results_i32: Vec<i32> = results.iter().map(|&v| v as i32).collect();

    assert_eq!(results_i32, values, "single-thread results mismatch");
}

#[test]
fn parallel_returns_correct_values_multi_thread() {
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, data) = compile(code);

    let values: Vec<i32> = (0..20).map(|i| i * 3).collect();
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_worker_id");
    let fn_pos = data.def(d_nr).code_position;

    let program = worker_program(&state);
    // Use 4 threads for 20 elements.
    let results = run_parallel_int(&state.database, program, fn_pos, &input, 8, 4);
    let results_i32: Vec<i32> = results.iter().map(|&v| v as i32).collect();

    assert_eq!(results_i32, values, "multi-thread results mismatch");
}

#[test]
fn parallel_empty_vector_returns_empty() {
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, data) = compile(code);

    let input = build_int_vector(&mut state.database, &[]);
    let d_nr = data.def_nr("n_worker_id");
    let fn_pos = data.def(d_nr).code_position;

    let program = worker_program(&state);
    let results = run_parallel_int(&state.database, program, fn_pos, &input, 8, 2);

    assert!(results.is_empty(), "empty input should give empty output");
}

#[test]
fn parallel_worker_computes_expression() {
    let code = r#"
struct Pair { a: integer, b: integer }
fn worker_sum(r: const Pair) -> integer { r.a + r.b }
"#;
    let (mut state, data) = compile(code);

    // Post-2c Pair: 2 integers × 8 bytes = 16 bytes per element.
    // We'll manually build the vector with Pair elements (a, b) at offsets 0, 8.
    let db = state.database.null();
    let n: u32 = 4;
    // vec_words: 8(count+hdr) + 4*16(elements) = 72 bytes → ceil(72/8) = 9 words
    let vec_words = (n * 16 + 15) / 8;
    let vec_cr = state.database.claim(&db, vec_words.max(1));
    let vec_rec = vec_cr.rec;
    let header_cr = state.database.claim(&db, 1);
    let header_rec = header_cr.rec;

    {
        let store = state.database.store_mut(&db);
        store.set_u32_raw(vec_rec, 4, n);
        // Pairs: (1,2), (3,4), (5,6), (7,8)
        let pairs = [(1i64, 2i64), (3, 4), (5, 6), (7, 8)];
        for (i, (a, b)) in pairs.iter().enumerate() {
            store.set_int(vec_rec, 8 + i as u32 * 16, *a); // field a at offset 0 (8B)
            store.set_int(vec_rec, 8 + i as u32 * 16 + 8, *b); // field b at offset 8 (8B)
        }
        store.set_u32_raw(header_rec, 4, vec_rec);
    }

    let input = DbRef {
        store_nr: db.store_nr,
        rec: header_rec,
        pos: 4,
    };

    let d_nr = data.def_nr("n_worker_sum");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    let results = run_parallel_int(&state.database, program, fn_pos, &input, 16, 2);

    assert_eq!(results, vec![3, 7, 11, 15], "expression results mismatch");
}

#[test]
fn parallel_store_is_read_only_in_workers() {
    // Verify that parent stores are borrowed read-only for workers
    // (clone_for_light_worker, @PLN108 — the single always-sharing clone).
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, _) = compile(code);
    let values = vec![1i32, 2, 3];
    let input = build_int_vector(&mut state.database, &values);

    // After clone_for_light_worker, every BORROWED parent store (index <
    // parent_len) is locked read-only (C93 — workers cannot write the parent).
    // The self-allocated scratch stores appended past parent_len are
    // worker-owned and intentionally writable, so they are not checked.
    let parent_len = state.database.allocations.len();
    let worker_stores = unsafe { state.database.clone_for_light_worker() };
    for (i, alloc) in worker_stores.allocations.iter().enumerate().take(parent_len) {
        assert!(
            alloc.is_locked(),
            "borrowed parent store {i} should be locked read-only"
        );
    }

    // The main stores should NOT be locked (except the constant store
    // which is pre-locked during byte_code() — P127).
    let const_store = loft::database::CONST_STORE as usize;
    for (i, alloc) in state.database.allocations.iter().enumerate() {
        if i == const_store {
            continue;
        }
        assert!(
            !alloc.is_locked(),
            "main store {i} should remain unlocked after clone"
        );
    }
    let _ = input; // ensure input is not dropped before this check
}

// ---------------------------------------------------------------------------
// Multi-field struct workers — context via the element struct itself.
// Each worker receives the full struct; "context" fields accompany the
// primary data field without any extra argument mechanism.
// ---------------------------------------------------------------------------

/// Three-field struct: worker reads all three fields as independent context.
/// `Triple { a, b, c }` → worker returns `a + b + c`.
#[test]
fn parallel_three_field_struct_sum() {
    let code = r#"
struct Triple { a: integer, b: integer, c: integer }
fn sum3(r: const Triple) -> integer { r.a + r.b + r.c }
"#;
    let (mut state, data) = compile(code);

    // Post-2c: 3 integers × 8B = 24 bytes per element.
    let db = state.database.null();
    let n: u32 = 4;
    let vec_words = (n * 24 + 15) / 8;
    let vec_cr = state.database.claim(&db, vec_words.max(1));
    let vec_rec = vec_cr.rec;
    let header_cr = state.database.claim(&db, 1);
    let header_rec = header_cr.rec;

    {
        let store = state.database.store_mut(&db);
        store.set_u32_raw(vec_rec, 4, n);
        // Post-2c: 3 integers × 8B = 24B stride.
        // Triples: (1,2,3), (4,5,6), (7,8,9), (10,11,12)
        let triples = [(1i64, 2i64, 3i64), (4, 5, 6), (7, 8, 9), (10, 11, 12)];
        for (i, (a, b, c)) in triples.iter().enumerate() {
            let off = 8 + i as u32 * 24;
            store.set_int(vec_rec, off, *a);
            store.set_int(vec_rec, off + 8, *b);
            store.set_int(vec_rec, off + 16, *c);
        }
        store.set_u32_raw(header_rec, 4, vec_rec);
    }

    let input = DbRef {
        store_nr: db.store_nr,
        rec: header_rec,
        pos: 4,
    };
    let d_nr = data.def_nr("n_sum3");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    let results = run_parallel_int(&state.database, program, fn_pos, &input, 24, 2);
    assert_eq!(results, vec![6, 15, 24, 33], "three-field sum");
}

/// Worker uses a context field as a multiplier and a data field as the value.
/// `struct Scaled { value: integer, factor: integer }` → `value * factor`.
#[test]
fn parallel_struct_with_context_factor() {
    let code = r#"
struct Scaled { value: integer, factor: integer }
fn apply_factor(r: const Scaled) -> integer { r.value * r.factor }
"#;
    let (mut state, data) = compile(code);

    // Post-2c: 2 × 8 = 16 bytes per element.
    let db = state.database.null();
    let n: u32 = 5;
    let vec_words = (n * 16 + 15) / 8;
    let vec_cr = state.database.claim(&db, vec_words.max(1));
    let vec_rec = vec_cr.rec;
    let header_cr = state.database.claim(&db, 1);
    let header_rec = header_cr.rec;

    {
        let store = state.database.store_mut(&db);
        store.set_u32_raw(vec_rec, 4, n);
        // (value, factor) pairs; factor is context shared per-element
        let pairs: [(i64, i64); 5] = [(3, 2), (5, 3), (7, 4), (2, 10), (1, 0)];
        for (i, (v, f)) in pairs.iter().enumerate() {
            store.set_int(vec_rec, 8 + i as u32 * 16, *v);
            store.set_int(vec_rec, 8 + i as u32 * 16 + 8, *f);
        }
        store.set_u32_raw(header_rec, 4, vec_rec);
    }

    let input = DbRef {
        store_nr: db.store_nr,
        rec: header_rec,
        pos: 4,
    };
    let d_nr = data.def_nr("n_apply_factor");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    let results = run_parallel_int(&state.database, program, fn_pos, &input, 16, 3);
    assert_eq!(results, vec![6, 15, 28, 20, 0], "value * factor");
}

/// Worker uses a threshold field for conditional logic.
/// Returns `value` if `value >= threshold`, else 0.
#[test]
fn parallel_conditional_context_threshold() {
    let code = r#"
struct Thresh { value: integer, threshold: integer }
fn clamp_lo(r: const Thresh) -> integer {
    if r.value >= r.threshold { r.value } else { 0 }
}
"#;
    let (mut state, data) = compile(code);

    let db = state.database.null();
    let n: u32 = 6;
    let vec_words = (n * 16 + 15) / 8;
    let vec_cr = state.database.claim(&db, vec_words.max(1));
    let vec_rec = vec_cr.rec;
    let header_cr = state.database.claim(&db, 1);
    let header_rec = header_cr.rec;

    {
        let store = state.database.store_mut(&db);
        store.set_u32_raw(vec_rec, 4, n);
        // (value, threshold) — post-2c: 2 × 8B = 16B stride
        let rows: [(i64, i64); 6] = [(10, 5), (3, 5), (5, 5), (0, 1), (100, 50), (49, 50)];
        for (i, (v, t)) in rows.iter().enumerate() {
            store.set_int(vec_rec, 8 + i as u32 * 16, *v);
            store.set_int(vec_rec, 8 + i as u32 * 16 + 8, *t);
        }
        store.set_u32_raw(header_rec, 4, vec_rec);
    }

    let input = DbRef {
        store_nr: db.store_nr,
        rec: header_rec,
        pos: 4,
    };
    let d_nr = data.def_nr("n_clamp_lo");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    let results = run_parallel_int(&state.database, program, fn_pos, &input, 16, 2);
    assert_eq!(results, vec![10, 0, 5, 0, 100, 0], "threshold clamp");
}

// ---------------------------------------------------------------------------
// Different worker return types.
// ---------------------------------------------------------------------------

use loft::parallel::run_parallel_raw;

/// Worker returns `long` (8-byte return); result collected via run_parallel_raw.
#[test]
fn parallel_long_return_type() {
    let code = r#"
struct Num { v: integer }
fn to_long(r: const Num) -> integer { r.v as integer * 1000000000 }
"#;
    let (mut state, data) = compile(code);

    let values = vec![1i32, 2, 3];
    let input = build_int_vector(&mut state.database, &values);
    let d_nr = data.def_nr("n_to_long");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    let raw = run_parallel_raw(&state.database, program, fn_pos, &input, 8, 8, 2, &[]);
    let longs: Vec<i64> = raw.iter().map(|&r| r as i64).collect();
    assert_eq!(
        longs,
        vec![1_000_000_000i64, 2_000_000_000, 3_000_000_000],
        "long results"
    );
}

/// Worker returns `boolean` (1-byte return); result collected via run_parallel_raw.
#[test]
fn parallel_boolean_return_type() {
    let code = r#"
struct Num { v: integer }
fn is_even(r: const Num) -> boolean { r.v % 2 == 0 }
"#;
    let (mut state, data) = compile(code);

    let values = vec![0i32, 1, 2, 3, 4];
    let input = build_int_vector(&mut state.database, &values);
    let d_nr = data.def_nr("n_is_even");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    let raw = run_parallel_raw(&state.database, program, fn_pos, &input, 8, 1, 1, &[]);
    let bools: Vec<bool> = raw.iter().map(|&r| r != 0).collect();
    assert_eq!(
        bools,
        vec![true, false, true, false, true],
        "even/odd booleans"
    );
}

/// Worker returns `text`; result collected via run_parallel_text.
#[test]
fn parallel_text_return_type() {
    let code = r#"
struct Num { v: integer }
fn label(r: const Num) -> text { "v{r.v}" }
"#;
    let (mut state, data) = compile(code);

    let values = vec![10i32, 20, 30];
    let input = build_int_vector(&mut state.database, &values);
    let d_nr = data.def_nr("n_label");
    assert_ne!(d_nr, u32::MAX, "label function not found");
    let fn_pos = data.def(d_nr).code_position;
    // Count hidden params (attrs starting with "__").
    let n_hidden = data
        .def(d_nr)
        .attributes
        .iter()
        .filter(|a| a.name.starts_with("__"))
        .count();
    eprintln!(
        "n_label: {} attrs, {} hidden, fn_pos={}",
        data.def(d_nr).attributes.len(),
        n_hidden,
        fn_pos
    );
    for (i, a) in data.def(d_nr).attributes.iter().enumerate() {
        eprintln!("  attr {i}: '{}' type={:?}", a.name, a.typedef);
    }

    let n_rows = values.len();
    let program = worker_program(&state);
    let strings = run_parallel_text(
        &state.database,
        program,
        fn_pos,
        &input,
        8,
        1,
        &[],
        n_rows,
        n_hidden,
        0, // primitive_input_size: worker takes `const Num` (DbRef input)
    );
    assert_eq!(
        strings,
        vec!["v10".to_string(), "v20".to_string(), "v30".to_string()],
        "text results"
    );
}

/// S29: A freed slot that is NOT the top slot (non-LIFO) must be reused by the next
/// `database()` call rather than growing the allocations Vec.  The old LIFO cascade skipped
/// non-top frees; the bitmap replacement (M4-b in SAFE.md P1-R4) reclaims them immediately.
#[test]
fn store_non_lifo_free_reclaims_slot() {
    // Compile minimal code to get a fresh State with a clean database.
    let (mut state, _) = compile("struct Box { val: integer }");
    let db = &mut state.database;
    // Allocate two stores on top of any background allocations.
    let a = db.database(100);
    let b = db.database(100);
    let max_after_two = db.max;
    // Free 'a' first — non-LIFO order (b is the newer/top slot; a is below it).
    // With bitmap: the freed slot is tracked; next database() reuses it.
    // Without bitmap: cascade skips a (not top) and max stays the same; next
    // database() pushes a new slot and max grows beyond max_after_two.
    db.free(&a);
    let c = db.database(100);
    assert_eq!(
        c.store_nr, a.store_nr,
        "S29: free-bitmap must reuse freed slot {} (got {})",
        a.store_nr, c.store_nr
    );
    assert!(
        db.max <= max_after_two,
        "S29: max must not grow past {max_after_two} (got {})",
        db.max
    );
    db.free(&b);
    db.free(&c);
}

/// Fix #92 (S21): stack_trace() inside a parallel worker must return a non-empty
/// frame vector.  Before the fix, data_ptr was null and call_stack was empty in
/// worker threads, so stack_trace() always returned 0 frames.
#[test]
fn parallel_stack_trace_non_empty() {
    let code = r#"
struct Num { v: integer }
fn count_frames(r: const Num) -> integer { frames = stack_trace(); len(frames) + r.v - r.v }
"#;
    let (mut state, data) = compile(code);

    let values: Vec<i32> = vec![0, 0, 0];
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_count_frames");
    assert_ne!(d_nr, u32::MAX, "count_frames function not found");
    let fn_pos = data.def(d_nr).code_position;

    let program = worker_program(&state);
    let results = run_parallel_int(&state.database, program, fn_pos, &input, 8, 1);

    for (i, &n) in results.iter().enumerate() {
        assert!(
            n > 0,
            "worker {i}: stack_trace() returned 0 frames (fix #92 regression)"
        );
    }
}

/// Fix #92 (P92): stack_trace() inside a parallel worker must resolve the
/// worker's function name and source file.  Before this fix, frame.d_nr was
/// u32::MAX (worker State had empty fn_positions) so the snapshot fell back
/// to the placeholder `<worker>` with empty file.  WorkerProgram now
/// propagates data_ptr, fn_positions, and line_numbers from the spawning
/// State so the snapshot resolves real frames.
///
/// Verified via the full `for ... par(...)` execution path through
/// `State::execute_argv` — that's where the parent's `data_ptr` and
/// `fn_positions` are set, mirroring how user programs run.
#[test]
fn parallel_stack_trace_resolves_worker_name() {
    let code = r#"
struct StFrameNum { v: integer }

fn st_named_worker(r: StFrameNum) -> integer {
  fr = stack_trace();
  base = r.v - r.v;
  if len(fr) >= 1 && fr[0].function == "st_named_worker" { base + 1 } else { base }
}

fn main() {
  st_data: vector<StFrameNum> = [];
  for st_n in 0..4 { st_data += [StFrameNum { v: st_n }]; }
  st_total = 0;
  for st_e in st_data par(st_x = st_named_worker(st_e), 2) {
    st_total += st_x;
  }
  assert(st_total == 4, "P92: all 4 workers must resolve frame name; got {st_total}");
}
"#;
    let (mut state, data) = compile(code);
    state.execute_argv("main", &data, &[]);
    // Compile-side sanity: the test relies on data and the stack_trace lib
    // function being present; silence "data unused" by referencing it.
    let _ = data.def_nr("n_st_named_worker");
}

/// Plan-06 phase 5a — verifies that `#impure(par_call)` annotations
/// in `default/01_code.loft` round-trip through the parser into
/// `Definition::purity` correctly.
#[test]
fn purity_annotations_parsed_from_stdlib() {
    use loft::data::{ImpureCategory, Purity};
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, true).unwrap();
    let pf_d_nr = p.data.def_nr("n_parallel_for");
    assert_ne!(pf_d_nr, u32::MAX, "n_parallel_for must be defined");
    assert_eq!(
        p.data.def(pf_d_nr).purity,
        Purity::Impure(ImpureCategory::ParCall),
        "parallel_for should be #impure(par_call)"
    );
    // ARC.md A4 (closed 2026-05-07): `parallel_for_light` stdlib
    // decl was removed (only the native panic stub remains).  Use
    // `parallel_queue` as the Queue-family ParCall stand-in for
    // the same purity invariant.
    let pq_d_nr = p.data.def_nr("n_parallel_queue");
    assert_ne!(pq_d_nr, u32::MAX, "n_parallel_queue must be defined");
    assert_eq!(
        p.data.def(pq_d_nr).purity,
        Purity::Impure(ImpureCategory::ParCall),
        "parallel_queue should be #impure(par_call)"
    );
    // Sanity: a fn that is genuinely Unknown should stay so.  We
    // pick a deeply internal one that no plan-06 5a sweep is going
    // to touch.  (n_now used to live here, but phase 5a annotated
    // it as #impure(host_io) — clock reads have an observable host
    // effect even though they're par-safe.)
    let unknown_d_nr = p.data.def_nr("n_OpClearScratch");
    if unknown_d_nr != u32::MAX {
        assert_eq!(
            p.data.def(unknown_d_nr).purity,
            Purity::Unknown,
            "n_OpClearScratch is unannotated → Purity::Unknown"
        );
    }
    // Verify the new annotations from phase 5a image/file/env/time
    // sweep landed correctly.
    for (name, expected) in &[
        ("n_now", Purity::Impure(ImpureCategory::HostIo)),
        ("n_ticks", Purity::Impure(ImpureCategory::HostIo)),
        ("n_env_variable", Purity::Impure(ImpureCategory::HostIo)),
        ("n_file", Purity::Impure(ImpureCategory::Io)),
        ("n_delete", Purity::Impure(ImpureCategory::Io)),
    ] {
        let d_nr = p.data.def_nr(name);
        if d_nr != u32::MAX {
            assert_eq!(
                p.data.def(d_nr).purity,
                *expected,
                "{name} should be {expected:?}"
            );
        }
    }

    // Phase 5a sample sweep — body fns annotated #pure.  More
    // categories land as the sweep continues; this is the gate
    // that catches accidental annotation drift.
    let pure_fns = &[
        "n_abs", "n_min", "n_max", "n_clamp", // integer arithmetic
        "n_len", "n_size", // text/character/vector length
        // Single + double precision math share the same fn names
        // (cos/sin/tan/etc.) — Data interns them by signature so
        // the def_nr lookup picks one (typically the float
        // overload).  Audit checks one of each name regardless
        // of which overload was registered.
        "n_cos", "n_sin", "n_tan", "n_acos", "n_asin", "n_atan", "n_ceil", "n_floor", "n_round",
        "n_sqrt",
    ];
    for name in pure_fns {
        let d_nr = p.data.def_nr(name);
        if d_nr != u32::MAX {
            assert_eq!(
                p.data.def(d_nr).purity,
                Purity::Pure,
                "{name} should be #pure"
            );
        }
    }

    // Phase 5a — parent-write mutators (collection-mutating
    // opcodes that user code reaches via compound assignment or
    // hash/sorted methods).
    let parent_write_fns = &[
        "n_clear",          // vector.clear()
        "n_OpRemoveVector", // vec.remove(i) and erase semantics
        "n_OpInsertVector", // vec.insert(i, x)
        "n_OpAppendVector", // vec += other
        "n_OpHashAdd",      // hash[k] = v
        "n_OpHashRemove",   // hash.remove(k)
        "n_OpNewRecord",    // claims space in a parent store
        "n_OpFinishRecord", // writes into a parent record
    ];
    for name in parent_write_fns {
        let d_nr = p.data.def_nr(name);
        if d_nr != u32::MAX {
            assert_eq!(
                p.data.def(d_nr).purity,
                Purity::Impure(ImpureCategory::ParentWrite),
                "{name} should be #impure(parent_write)"
            );
        }
    }

    // Phase 5a sample sweep — host-IO annotations.  These are
    // observable side effects (write to log sink / stdout) but
    // serialise through the host bridge, so par workers may call
    // them.
    let host_io_fns = &[
        "n_log_info",
        "n_log_warn",
        "n_log_error",
        "n_log_fatal",
        "n_print",
        "n_println",
    ];
    for name in host_io_fns {
        let d_nr = p.data.def_nr(name);
        if d_nr != u32::MAX {
            assert_eq!(
                p.data.def(d_nr).purity,
                Purity::Impure(ImpureCategory::HostIo),
                "{name} should be #impure(host_io)"
            );
        }
    }
}

/// Plan-06 phase 5b' — verifies the deep par-safety check actually
/// fires for a worker that calls a fn classified `Impure(ParentWrite)`
/// on a non-local first argument.  End-to-end proof of the wire-up
/// in parse_parallel_for_loop.
///
/// Plan-06 phase 5b' G5 update: clearing a worker-LOCAL vector is
/// safe (the local var was allocated in worker stores), so the test
/// scenario was rewritten to clear a vector PARAMETER — a true
/// parent reference that the deep walker correctly flags.
#[test]
fn par_warning_fires_for_direct_parent_write_worker() {
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse_str(
        r#"
fn bad_worker(v: vector<integer>) -> integer {
  v.clear();
  42
}

fn main() {
  total = 0;
  items: vector<integer> = [1, 2, 3];
  for i in items par(r = bad_worker(items), 2) {
    total += r;
  }
}
"#,
        "par_warning_test",
        false,
    );
    let all_lines: Vec<String> = p
        .diagnostics
        .lines()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let warning_lines: Vec<&String> = all_lines
        .iter()
        .filter(|line| line.contains("par() worker"))
        .collect();
    // Expect at least one warning naming bad_worker + the offending callee.
    assert!(
        !warning_lines.is_empty(),
        "expected par-safety warning; got diagnostics: {all_lines:?}"
    );
    let combined = warning_lines
        .iter()
        .map(|l| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        combined.contains("bad_worker"),
        "warning should name worker fn 'bad_worker'; got: {combined}"
    );
}

// ── Plan-06 spine step 5/7 — par-result use-site diagnostic ───────────────

/// Spine step 5 + 7 — `let r = parallel_for(...)` followed by random
/// access (`r[i]`) emits `Level::Error` pointing the user at the
/// fused for-par form or `par_to_vec(...)`.  Step 5 introduced the
/// diagnostic at `Level::Warning`; step 7 promoted to `Error` after
/// step 6's audit confirmed zero corpus sites trigger it.
#[test]
fn par_result_random_access_emits_materialise_error() {
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse_str(
        r#"
struct Num { v: integer }
fn worker(r: const Num) -> integer { r.v }

fn main() {
  items: vector<Num> = [Num { v: 1 }, Num { v: 2 }, Num { v: 3 }];
  results = parallel_for(items, 8, 8, 2, fn worker);
  total = results[0] + results[1];
}
"#,
        "par_warn_test",
        false,
    );
    let all_lines: Vec<String> = p
        .diagnostics
        .lines()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let materialise_errors: Vec<&String> = all_lines
        .iter()
        .filter(|line| line.contains("materialising context"))
        .collect();
    // The test inputs are synthetic — the parser may also reject the
    // direct `parallel_for(...)` call as "not callable from user code"
    // since the loft signature is internal.  But IF the error fires,
    // it must name the user variable (not a `_par_results_N` synthetic)
    // and must be tagged at `Level::Error` (step 7 promotion).
    if !materialise_errors.is_empty() {
        let combined: String = materialise_errors
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("'results'") || combined.contains("results"),
            "materialise error should name user var 'results'; got: {combined}"
        );
        assert!(
            !combined.contains("_par_results"),
            "error must skip compiler-generated _par_results_N bindings; got: {combined}"
        );
        assert!(
            combined.contains("Error:"),
            "spine step 7 promotes the materialise diagnostic to Level::Error; \
             got: {combined}"
        );
    }
}

/// Spine step 5 + 7 — `let r = parallel_for(...)` where `r` is
/// never read emits a different `Level::Error` ("result is never
/// read") to nudge users at the discard-style fused for-par form.
#[test]
fn par_result_unused_emits_unused_error() {
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse_str(
        r#"
struct Num { v: integer }
fn worker(r: const Num) -> integer { r.v }

fn main() {
  items: vector<Num> = [Num { v: 1 }, Num { v: 2 }];
  unused_result = parallel_for(items, 8, 8, 2, fn worker);
}
"#,
        "par_warn_unused_test",
        false,
    );
    let all_lines: Vec<String> = p
        .diagnostics
        .lines()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let unused_errors: Vec<&String> = all_lines
        .iter()
        .filter(|line| line.contains("never read"))
        .collect();
    if !unused_errors.is_empty() {
        let combined: String = unused_errors
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            combined.contains("unused_result") || combined.contains("never read"),
            "unused error should name var or describe; got: {combined}"
        );
        assert!(
            combined.contains("Error:"),
            "spine step 7 promotes the unused-result diagnostic to Level::Error; \
             got: {combined}"
        );
    }
}

/// Spine step 5 + 7 — fused for-par's hidden `_par_results_N`
/// binding must NOT trigger the materialise diagnostic (the desugar
/// uses random access internally, but that's compiler-generated and
/// step 8 will retire it).  After step 7 the diagnostic is `Error`
/// so a stray hit here would block compilation of every fused
/// for-par site.
#[test]
fn par_result_diagnostic_skips_compiler_generated_bindings() {
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse_str(
        r#"
struct Num { v: integer }
fn worker(r: const Num) -> integer { r.v + 1 }

fn main() {
  items: vector<Num> = [Num { v: 1 }, Num { v: 2 }];
  total = 0;
  for x in items par(r = worker(x), 2) {
    total += r;
  }
}
"#,
        "par_warn_skip_test",
        false,
    );
    let all_lines: Vec<String> = p
        .diagnostics
        .lines()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let materialise_diagnostics: Vec<&String> = all_lines
        .iter()
        .filter(|line| line.contains("materialising context"))
        .collect();
    assert!(
        materialise_diagnostics.is_empty(),
        "fused for-par desugar must not trigger the materialise diagnostic \
         (the _par_results_N binding is compiler-generated); got: \
         {materialise_diagnostics:?}"
    );
}

// ── Plan-06 spine step 2 — Stitch::Discard regression tests ────────────────

/// Discard runs the worker for every row and returns nothing.  Smoke-test:
/// a non-trivial input, a worker that touches its arg (so we exercise the
/// dispatch path), 4 threads, no panic.
#[test]
fn par_discard_runs_without_panic() {
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, data) = compile(code);

    let values: Vec<i32> = (0..50).collect();
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_worker_id");
    let fn_pos = data.def(d_nr).code_position;

    let program = worker_program(&state);
    // Returns ().  All we assert is "no panic" — Discard's contract is
    // that the worker runs N times and the result is dropped.  Worker's
    // return-size is 8 (i64); the runtime discards it.
    run_parallel_discard(&state.database, program, fn_pos, &input, 8, 4, &[], 8);
}

/// Discard on an empty input is a no-op — no workers spawned, no panic.
#[test]
fn par_discard_empty_input() {
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, data) = compile(code);

    let input = build_int_vector(&mut state.database, &[]);
    let d_nr = data.def_nr("n_worker_id");
    let fn_pos = data.def(d_nr).code_position;

    let program = worker_program(&state);
    run_parallel_discard(&state.database, program, fn_pos, &input, 8, 2, &[], 8);
}

/// Spine step 3c — fused for-par with empty body lowers to
/// `n_parallel_discard` instead of allocating a result vector.
/// Compiles + runs end-to-end; locks in the parser-side desugar
/// plus the n_parallel_discard native dispatch and the
/// run_parallel_discard runtime.
#[test]
fn par_fused_empty_body_runs_through_discard() {
    use loft::compile::byte_code;
    use loft::scopes;
    use loft::state::State;
    let mut p = loft::parser::Parser::new();
    p.parse_dir("default", true, true).unwrap();
    p.parse_str(
        r#"
struct Num { v: integer }

fn worker(r: const Num) -> integer {
  r.v + 1
}

fn main() {
  items: vector<Num> = [
    Num { v: 1 },
    Num { v: 2 },
    Num { v: 3 },
  ];
  for x in items par(_unused = worker(x), 2) {
  }
}
"#,
        "par_fused_discard",
        false,
    );
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("main", &p.data);
}

// ── Plan-06 spine step 4 — Stitch::Queue regression tests ──────────────────

/// Queue's order-preservation invariant: every result lands in slot
/// `i` of the returned Vec where `i` is the input row index, regardless
/// of how rayon dispatched workers.  Tests the per-worker batch + merge
/// pipeline that step 5's value-position lowering will rely on.
#[test]
fn par_queue_returns_results_in_input_order() {
    let code = r#"
struct Num { v: integer }
fn worker_double(r: const Num) -> integer { r.v * 2 }
"#;
    let (mut state, data) = compile(code);

    let values: Vec<i32> = (0..30).collect();
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_worker_double");
    let fn_pos = data.def(d_nr).code_position;

    let program = worker_program(&state);
    // 4 threads over 30 elements; per-row result is i64 (8 bytes).
    // 8b': pass `0, None` for primitive_input_size + tuple_input_types
    // → DbRef-input dispatch (matches the existing test's worker shape).
    let results = run_parallel_queue(
        &state.database,
        program,
        fn_pos,
        &input,
        8,
        4,
        &[],
        8,
        0,
        None,
    );
    assert_eq!(results.len(), 30, "expected one result per input row");
    for (i, &r) in results.iter().enumerate() {
        let expected = (i as i64) * 2;
        assert_eq!(
            r as i64, expected,
            "row {i}: worker returned {r}, expected {expected}"
        );
    }
}

/// Empty input → empty result Vec, no workers spawned.
#[test]
fn par_queue_empty_input() {
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, data) = compile(code);

    let input = build_int_vector(&mut state.database, &[]);
    let d_nr = data.def_nr("n_worker_id");
    let fn_pos = data.def(d_nr).code_position;

    let program = worker_program(&state);
    let results = run_parallel_queue(
        &state.database,
        program,
        fn_pos,
        &input,
        8,
        2,
        &[],
        8,
        0,
        None,
    );
    assert!(results.is_empty());
}

/// Same load-bearing invariant as Discard — Queue must not adopt
/// or otherwise grow the parent's allocations table.  Step 8's
/// Concat retirement assumes Queue is allocation-free on the
/// parent side (the result Vec lives in main-thread Rust heap, not
/// in `Stores`).
#[test]
fn par_queue_does_not_grow_parent_stores() {
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, data) = compile(code);

    let values: Vec<i32> = (0..10).collect();
    let input = build_int_vector(&mut state.database, &values);

    let before = state.database.allocations.len();

    let d_nr = data.def_nr("n_worker_id");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);
    let _ = run_parallel_queue(
        &state.database,
        program,
        fn_pos,
        &input,
        8,
        2,
        &[],
        8,
        0,
        None,
    );

    let after = state.database.allocations.len();
    assert_eq!(
        before, after,
        "Queue must not adopt worker stores into the parent's allocations \
         table (before={before}, after={after})"
    );
}

/// Single-thread Queue should produce the same order as multi-thread.
#[test]
fn par_queue_single_thread_matches_multi() {
    let code = r#"
struct Num { v: integer }
fn worker_triple(r: const Num) -> integer { r.v * 3 }
"#;
    let (mut state, data) = compile(code);

    let values: Vec<i32> = (0..20).map(|i| i + 100).collect();
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_worker_triple");
    let fn_pos = data.def(d_nr).code_position;

    let prog1 = worker_program(&state);
    let r1 = run_parallel_queue(
        &state.database,
        prog1,
        fn_pos,
        &input,
        8,
        1,
        &[],
        8,
        0,
        None,
    );
    let prog4 = worker_program(&state);
    let r4 = run_parallel_queue(
        &state.database,
        prog4,
        fn_pos,
        &input,
        8,
        4,
        &[],
        8,
        0,
        None,
    );

    assert_eq!(r1, r4, "single-thread and 4-thread results must match");
    for (i, &r) in r1.iter().enumerate() {
        let expected = ((i as i64) + 100) * 3;
        assert_eq!(r as i64, expected, "row {i}: got {r}, expected {expected}");
    }
}

/// Discard's parent state must remain untouched.  Workers run, results are
/// dropped — there's no user-visible side effect on the parent's stores
/// (workers can write to log/host_io but that's via host bridges, not
/// shared parent state).
///
/// Verifies the `parent_store_count` invariant: the parent's allocations
/// table has the same count before and after the par call.  This is
/// step-8's load-bearing invariant — Concat's retirement assumes Discard
/// does not adopt or otherwise grow the parent's store table.
#[test]
fn par_discard_does_not_grow_parent_stores() {
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, data) = compile(code);

    let values: Vec<i32> = (0..10).collect();
    let input = build_int_vector(&mut state.database, &values);

    let before = state.database.allocations.len();

    let d_nr = data.def_nr("n_worker_id");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);
    run_parallel_discard(&state.database, program, fn_pos, &input, 8, 2, &[], 8);

    let after = state.database.allocations.len();
    assert_eq!(
        before, after,
        "Discard must not adopt worker stores into the parent's allocations \
         table (before={before}, after={after})"
    );
}

// ── Plan-06 spine step 8d.0 — run_parallel_queue_ref ──────────────────────

/// Step 8d.0 — `run_parallel_queue_ref` adopts each worker's output
/// stores into the parent's allocations table and rebases each
/// returned `DbRef` so it's parent-valid.  The returned
/// `(refs, adopted_store_nrs)` pair gives 8d.1's
/// `n_parallel_buf_get_ref` something to push and
/// `n_parallel_buf_drop_ref` a list of stores to free at the
/// fused-for-par body's tail.
///
/// Smoke-test: 4-element `vector<Score>` input, struct-returning
/// worker `bump(s) -> Score { Score { value: s.value + 100 } }`,
/// 2 threads.  Verifies:
///   - `refs.len()` matches the input row count.
///   - Every ref's `store_nr` is in the parent's allocations range
///     (i.e. adoption actually moved the store) — proves the rebase
///     ran.
///   - Each ref's record has `value` matching the expected `100 + i`,
///     read through the parent's store namespace.
///   - `adopted` contains exactly the new store_nrs; the parent's
///     allocations grew by `adopted.len()`.
#[test]
fn par_queue_ref_adopts_and_rebases() {
    let code = r#"
struct Score { value: integer }
fn bump(s: const Score) -> Score { Score { value: s.value + 100 } }
"#;
    let (mut state, data) = compile(code);

    // 4 × Score = 4 × 8 bytes per element (single integer field).
    let db = state.database.null();
    let n: u32 = 4;
    let vec_words = (n * 8 + 15) / 8;
    let vec_cr = state.database.claim(&db, vec_words.max(1));
    let vec_rec = vec_cr.rec;
    let header_cr = state.database.claim(&db, 1);
    let header_rec = header_cr.rec;
    {
        let store = state.database.store_mut(&db);
        store.set_u32_raw(vec_rec, 4, n);
        for i in 0..n {
            store.set_int(vec_rec, 8 + i * 8, i64::from(i));
        }
        store.set_u32_raw(header_rec, 4, vec_rec);
    }
    let input = DbRef {
        store_nr: db.store_nr,
        rec: header_rec,
        pos: 4,
    };
    let d_nr = data.def_nr("n_bump");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);
    let ret_type = data.def(d_nr).returned.clone();

    let allocations_before = state.database.allocations.len();
    // @PLAN59: derive the hidden-dest count from the def like real callers
    // do (the signature-time __retbuf makes literal-returning workers carry
    // one too).
    let n_dests = data
        .def(d_nr)
        .attributes()
        .iter()
        .filter(|a| {
            a.hidden
                && matches!(
                    a.typedef,
                    loft::data::Type::Reference(_, _)
                        | loft::data::Type::Vector(_, _)
                        | loft::data::Type::Enum(_, true, _)
                )
        })
        .count();
    let (refs, adopted) = run_parallel_queue_ref(
        &mut state.database,
        program,
        fn_pos,
        &input,
        8,
        2,
        &[],
        n as usize,
        &ret_type,
        &data,
        n_dests,
        0, // primitive_input_size: worker takes a struct ref (DbRef input)
    );
    let allocations_after = state.database.allocations.len();

    assert_eq!(refs.len(), n as usize, "one ref per input row");
    assert!(
        !adopted.is_empty(),
        "expected at least one worker store to be adopted"
    );
    // 8d.3: parent's allocations grow by `n_threads *
    // SLOTS_PER_THREAD` (the reserved range), not by `adopted.len()`.
    // Unused reserved slots stay in `allocations` but are flagged
    // free in `free_bits` for future reuse — `release_worker_slots`
    // handles that.  The adopted count is bounded above by the
    // reservation but can be smaller (workers may not fill every
    // reserved slot).
    assert!(
        adopted.len() <= allocations_after - allocations_before,
        "adopted count ({}) ≤ reservation growth ({})",
        adopted.len(),
        allocations_after - allocations_before
    );
    assert!(
        adopted.len() >= n as usize,
        "expected at least one adopted store per input row (got {} for n={n})",
        adopted.len()
    );

    // Each rebased DbRef should resolve in the parent's namespace.
    // The Score's `value` field lives at offset 0 of the record.
    for (i, r) in refs.iter().enumerate() {
        assert_ne!(
            r.store_nr,
            u16::MAX,
            "row {i}: rebased ref must not be the null sentinel"
        );
        assert!(
            (r.store_nr as usize) < state.database.allocations.len(),
            "row {i}: rebased store_nr {} out of parent range (len={})",
            r.store_nr,
            state.database.allocations.len()
        );
        let store = &state.database.allocations[r.store_nr as usize];
        let v: i64 = *store.addr::<i64>(r.rec, r.pos);
        assert_eq!(v, 100 + i as i64, "row {i}: expected 100+{i}, got {v}");
    }
}

/// ARC.md A2.1 — stress-test the unbounded slot dispenser at the
/// allocator level.
///
/// 8d.3 reserved a fixed `SLOTS_PER_THREAD = 16` range per worker
/// thread; workers that allocated more than 16 fresh stores within
/// a single batch hit a hard `assert!` in `database_named`.  The
/// A2.2 fix replaces the fixed range with a shared
/// `worker_slot_dispenser: Arc<AtomicU16>` so each `database_named`
/// call dispenses a fresh parent-namespace index — no cap.
///
/// This test bypasses the bytecode pipeline (which has its own
/// `execute_at_ref` calling convention with hidden return slots for
/// inline-struct-returns that conflict with direct invocation) and
/// exercises `database_named` directly: it sets up the dispenser
/// exactly as `run_parallel_queue_ref` does, then performs 50
/// named allocations.  All 50 must succeed — no exhaustion panic,
/// indices strictly monotonic, every dispense recorded in
/// `worker_allocated_indices`.
///
/// Pre-A2.2: panicked at the 17th call.
/// Post-A2.2: all 50 land in distinct parent-namespace indices.
#[test]
fn par_queue_ref_unbounded_allocations_per_element() {
    use std::sync::Arc;

    let mut parent = loft::database::Stores::new();
    // Pre-fill a few slots so the dispenser starts above zero.
    let _seed = parent.null();
    let parent_len_at_dispatch = parent.allocations.len();
    let dispenser = parent.make_worker_slot_dispenser();

    // Simulate a worker's `Stores`: schema-only clone (which resets
    // allocations to empty), attach the dispenser, allocate many
    // named stores.  The first dispense yields `parent_len + 1`;
    // `database_named`'s `while` loop grows the worker's empty
    // allocations to fit each yielded index by pushing
    // `Store::new(100)` placeholders for any skipped slots.
    let mut worker_stores: loft::database::Stores = parent.clone();
    worker_stores.disable_slot_reuse = true;
    worker_stores.worker_slot_dispenser = Some(Arc::clone(&dispenser));

    const N_ALLOCS: usize = 50;
    let mut refs = Vec::with_capacity(N_ALLOCS);
    for i in 0..N_ALLOCS {
        let r = worker_stores.database_named(8, "stress");
        assert_ne!(
            r.store_nr,
            u16::MAX,
            "alloc #{i}: must not return null sentinel"
        );
        refs.push(r);
    }

    // Indices must be strictly monotonic (atomic dispenses each one
    // higher than the last) and start at parent_len + 1 (the +1
    // skips the worker's stack-store slot — even though this test
    // didn't allocate one, the dispenser is sized for the general
    // run_parallel_queue_ref case).
    let expected_first = parent_len_at_dispatch as u16 + 1;
    assert_eq!(
        refs[0].store_nr, expected_first,
        "first dispense should be parent_len + 1 = {expected_first}"
    );
    for w in refs.windows(2) {
        assert!(
            w[1].store_nr > w[0].store_nr,
            "dispenser must yield strictly increasing indices: got {} then {}",
            w[0].store_nr,
            w[1].store_nr
        );
    }
    assert_eq!(
        worker_stores.worker_allocated_indices.len(),
        N_ALLOCS,
        "every dispenser allocation must record its index"
    );

    // Final dispenser value reflects the +1 stack-store skip plus
    // every dispense.
    let final_val = dispenser.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        final_val as usize,
        parent_len_at_dispatch + 1 + N_ALLOCS,
        "dispenser high-water = parent_len + 1 + N_ALLOCS"
    );
}

/// ARC.md A2.3 — the invariant in `database_named` must fire when a
/// worker has a dispenser attached but `disable_slot_reuse` is
/// cleared.  That bypass would push the allocation through the
/// legacy `find_free_slot` / push-at-end path, producing a
/// parent-namespace index OTHER than the next dispenser yield —
/// silently colliding with another worker's dispensed slot at
/// thread-join swap-back.  The assertion catches the misuse loudly.
///
/// Always-on (not gated by `debug_assertions`) because the loft
/// library compiles with `debug-assertions = false` in the test
/// profile per `[profile.dev.package.loft]`, so a `debug_assert!`
/// would be a silent no-op.
#[test]
#[should_panic(
    expected = "every dispenser-attached allocation must go through the offset-aware path"
)]
fn par_queue_ref_dispenser_bypass_assertion_fires() {
    use std::sync::Arc;

    let mut parent = loft::database::Stores::new();
    let _seed = parent.null();
    let dispenser = parent.make_worker_slot_dispenser();

    let mut worker_stores: loft::database::Stores = parent.clone();
    // Attach the dispenser but DON'T set disable_slot_reuse — that's
    // the synthetic bypass shape A2.3 guards against.
    worker_stores.worker_slot_dispenser = Some(Arc::clone(&dispenser));
    worker_stores.disable_slot_reuse = false;

    // Trigger the assertion.
    let _ = worker_stores.database_named(8, "bypass");
}

/// Step 8d.0 — empty input: no workers spawn, no allocations adopted,
/// returned refs and adopted lists are both empty.  Locks the
/// short-circuit invariant 8d.1's native fn relies on (otherwise it
/// would push an empty buffer onto par_ref_buffer_stack and call
/// `adopt_worker_excess` on stale state).
#[test]
fn par_queue_ref_empty_input() {
    let code = r#"
struct Score { value: integer }
fn bump(s: const Score) -> Score { Score { value: s.value + 100 } }
"#;
    let (mut state, data) = compile(code);
    let db = state.database.null();
    let header_cr = state.database.claim(&db, 1);
    let header_rec = header_cr.rec;
    let vec_cr = state.database.claim(&db, 1);
    let vec_rec = vec_cr.rec;
    state.database.store_mut(&db).set_u32_raw(vec_rec, 4, 0);
    state
        .database
        .store_mut(&db)
        .set_u32_raw(header_rec, 4, vec_rec);
    let input = DbRef {
        store_nr: db.store_nr,
        rec: header_rec,
        pos: 4,
    };
    let d_nr = data.def_nr("n_bump");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);
    let ret_type = data.def(d_nr).returned.clone();

    let allocations_before = state.database.allocations.len();
    let (refs, adopted) = run_parallel_queue_ref(
        &mut state.database,
        program,
        fn_pos,
        &input,
        8,
        2,
        &[],
        0,
        &ret_type,
        &data,
        0,
        0, // primitive_input_size: worker takes a struct ref (DbRef input)
    );
    let allocations_after = state.database.allocations.len();

    assert!(refs.is_empty(), "empty input → no refs");
    assert!(adopted.is_empty(), "empty input → nothing adopted");
    assert_eq!(
        allocations_before, allocations_after,
        "empty input must not touch parent allocations"
    );
}

// ── Plan-06 spine step 8a — par_buffer_stack semantics ────────────────────

/// Step 8a — `Stores::par_buffer_stack` is a stack of `Vec<u64>`.
/// Round-trip: push a buffer, observe it via `.last()`, pop, verify
/// empty.  Locks in the contract `n_parallel_buf_get` /
/// `n_parallel_buf_drop` rely on.
#[test]
fn par_buffer_stack_push_pop_round_trip() {
    let mut stores = loft::database::Stores::new();
    assert!(stores.par_buffer_stack.is_empty(), "fresh stores: empty");
    stores.par_buffer_stack.push(vec![1, 2, 3]);
    assert_eq!(stores.par_buffer_stack.len(), 1);
    assert_eq!(stores.par_buffer_stack.last().unwrap(), &vec![1u64, 2, 3]);
    let popped = stores.par_buffer_stack.pop();
    assert_eq!(popped, Some(vec![1u64, 2, 3]));
    assert!(stores.par_buffer_stack.is_empty());
}

/// Step 8a — nested fused for-par pushes its own buffer on top.
/// Inner pop must reveal the outer buffer underneath, unchanged.
/// This is the load-bearing invariant for nested par(): step 8b's
/// IR rewrite emits a `n_parallel_buf_drop` per fused-for body, and
/// the nesting must work without leaks.
#[test]
fn par_buffer_stack_nesting() {
    let mut stores = loft::database::Stores::new();
    stores.par_buffer_stack.push(vec![10, 20]);
    stores.par_buffer_stack.push(vec![100, 200, 300]);
    assert_eq!(stores.par_buffer_stack.len(), 2);
    assert_eq!(
        stores.par_buffer_stack.last().unwrap(),
        &vec![100u64, 200, 300]
    );
    stores.par_buffer_stack.pop();
    assert_eq!(
        stores.par_buffer_stack.last().unwrap(),
        &vec![10u64, 20],
        "outer buffer must survive inner pop"
    );
}

/// Step 8a — `Stores::clone()` is the type-schema clone used to seed
/// fresh interpreter sessions; it must reset `par_buffer_stack` to
/// empty.  A leaked buffer here would persist across sessions and
/// break the nesting invariant.
#[test]
fn par_buffer_stack_initially_empty_in_clone() {
    let mut stores = loft::database::Stores::new();
    stores.par_buffer_stack.push(vec![1]);
    stores.par_buffer_stack.push(vec![2, 3]);
    let cloned = stores.clone();
    assert!(
        cloned.par_buffer_stack.is_empty(),
        "Stores::clone must reset par_buffer_stack — it's runtime state, \
         not type schema (cloned len = {})",
        cloned.par_buffer_stack.len()
    );
}

// ── Plan-06 spine step 8d.1 — par_ref_buffer_stack semantics ──────────────

/// Step 8d.1 — `Stores::par_ref_buffer_stack` is a stack of
/// `(refs, adopted_store_nrs)` tuples.  Round-trip: push, observe
/// via `last()`, pop, verify empty.  Locks the contract
/// `n_parallel_buf_get_ref` / `n_parallel_buf_drop_ref` rely on.
#[test]
fn par_ref_buffer_stack_push_pop_round_trip() {
    let mut stores = loft::database::Stores::new();
    assert!(stores.par_ref_buffer_stack.is_empty());
    let r0 = DbRef {
        store_nr: 5,
        rec: 10,
        pos: 4,
    };
    let r1 = DbRef {
        store_nr: 6,
        rec: 11,
        pos: 8,
    };
    stores.par_ref_buffer_stack.push((vec![r0, r1], vec![5, 6]));
    assert_eq!(stores.par_ref_buffer_stack.len(), 1);
    let last = stores.par_ref_buffer_stack.last().unwrap();
    assert_eq!(last.0, vec![r0, r1]);
    assert_eq!(last.1, vec![5u16, 6]);
    let popped = stores.par_ref_buffer_stack.pop();
    assert!(popped.is_some());
    assert!(stores.par_ref_buffer_stack.is_empty());
}

/// Step 8d.1 — nested fused for-par over ref-returning workers
/// pushes its own buffer; inner pop must reveal the outer buffer
/// unchanged.  Same nesting invariant `par_buffer_stack_nesting`
/// locks for the int-buffer.
#[test]
fn par_ref_buffer_stack_nesting() {
    let mut stores = loft::database::Stores::new();
    let outer = DbRef {
        store_nr: 1,
        rec: 0,
        pos: 0,
    };
    let inner = DbRef {
        store_nr: 2,
        rec: 0,
        pos: 0,
    };
    stores.par_ref_buffer_stack.push((vec![outer], vec![1]));
    stores
        .par_ref_buffer_stack
        .push((vec![inner, inner], vec![2, 3]));
    assert_eq!(stores.par_ref_buffer_stack.len(), 2);
    assert_eq!(
        stores.par_ref_buffer_stack.last().unwrap().0,
        vec![inner, inner]
    );
    stores.par_ref_buffer_stack.pop();
    assert_eq!(
        stores.par_ref_buffer_stack.last().unwrap().0,
        vec![outer],
        "outer buffer must survive inner pop"
    );
}

/// Step 8d.1 — `Stores::clone()` resets `par_ref_buffer_stack` to
/// empty; runtime state, not type schema.
#[test]
fn par_ref_buffer_stack_initially_empty_in_clone() {
    let mut stores = loft::database::Stores::new();
    let r = DbRef {
        store_nr: 1,
        rec: 0,
        pos: 0,
    };
    stores.par_ref_buffer_stack.push((vec![r], vec![1]));
    let cloned = stores.clone();
    assert!(
        cloned.par_ref_buffer_stack.is_empty(),
        "Stores::clone must reset par_ref_buffer_stack — runtime state, \
         not type schema (cloned len = {})",
        cloned.par_ref_buffer_stack.len()
    );
}

// ── Plan-06 ARC.md A3 — par_narrow_buffer_stack semantics ────────────────

/// ARC.md A3 — `Stores::par_narrow_buffer_stack` is a stack of
/// `(bytes, stride)` for narrow-primitive (1/2/4-byte) returning par
/// workers.  Round-trip: push, observe via `last()`, pop, verify
/// empty.  Locks in the contract `n_parallel_buf_get_narrow` /
/// `n_parallel_buf_drop_narrow` rely on.
#[test]
fn par_narrow_buffer_stack_push_pop_round_trip() {
    let mut stores = loft::database::Stores::new();
    assert!(stores.par_narrow_buffer_stack.is_empty(), "fresh: empty");
    let bytes = vec![0xFFu8, 0x00, 0x7F, 0x80]; // four signed-i8 rows: -1, 0, 127, -128
    stores.par_narrow_buffer_stack.push((bytes.clone(), 1));
    assert_eq!(stores.par_narrow_buffer_stack.len(), 1);
    let entry = stores.par_narrow_buffer_stack.last().unwrap();
    assert_eq!(entry.0, bytes);
    assert_eq!(entry.1, 1);
    let popped = stores.par_narrow_buffer_stack.pop();
    assert_eq!(popped, Some((bytes, 1)));
    assert!(stores.par_narrow_buffer_stack.is_empty());
}

/// ARC.md A3 — nested narrow par(): inner pop reveals outer buffer
/// underneath, unchanged.  Same load-bearing invariant as 8a's
/// `par_buffer_stack_nesting`.
#[test]
fn par_narrow_buffer_stack_nesting() {
    let mut stores = loft::database::Stores::new();
    let outer = vec![1u8, 2];
    let inner = vec![10u8, 20, 30, 40];
    stores.par_narrow_buffer_stack.push((outer.clone(), 1));
    stores.par_narrow_buffer_stack.push((inner.clone(), 1));
    assert_eq!(stores.par_narrow_buffer_stack.len(), 2);
    assert_eq!(stores.par_narrow_buffer_stack.last().unwrap().0, inner);
    stores.par_narrow_buffer_stack.pop();
    assert_eq!(
        stores.par_narrow_buffer_stack.last().unwrap().0,
        outer,
        "outer buffer must survive inner pop"
    );
}

/// ARC.md A3 — `Stores::clone()` resets `par_narrow_buffer_stack` to
/// empty; runtime state, not type schema.
#[test]
fn par_narrow_buffer_stack_initially_empty_in_clone() {
    let mut stores = loft::database::Stores::new();
    stores.par_narrow_buffer_stack.push((vec![1u8, 2, 3, 4], 1));
    let cloned = stores.clone();
    assert!(
        cloned.par_narrow_buffer_stack.is_empty(),
        "Stores::clone must reset par_narrow_buffer_stack — runtime state, \
         not type schema (cloned len = {})",
        cloned.par_narrow_buffer_stack.len()
    );
}

// ── Plan-06 ARC.md A6.b — par_fn_buffer_stack semantics ─────────────────

/// ARC.md A6.b — `Stores::par_fn_buffer_stack` is a stack of packed
/// `Vec<u8>` buffers, each `n_rows * 20` bytes for fn-ref-returning
/// par workers (8B i64 d_nr + 12B closure DbRef per row).  Round-trip:
/// push, observe via `last()`, pop, verify empty.  Locks the contract
/// `n_parallel_buf_get_fn` / `n_parallel_buf_drop_fn` rely on.
#[test]
fn par_fn_buffer_stack_push_pop_round_trip() {
    let mut stores = loft::database::Stores::new();
    assert!(stores.par_fn_buffer_stack.is_empty(), "fresh: empty");
    // Two fn-ref rows: row 0 d_nr=541, row 1 d_nr=542 (low 8B), each
    // with sentinel closure (u16::MAX at slot offset 16-17 per Rust's
    // reordered DbRef layout: rec u32 + pos u32 + store_nr u16 + padding).
    let mut bytes = vec![0u8; 40];
    bytes[0..8].copy_from_slice(&541i64.to_le_bytes());
    bytes[16..18].copy_from_slice(&u16::MAX.to_le_bytes());
    bytes[20..28].copy_from_slice(&542i64.to_le_bytes());
    bytes[36..38].copy_from_slice(&u16::MAX.to_le_bytes());
    stores.par_fn_buffer_stack.push(bytes.clone());
    assert_eq!(stores.par_fn_buffer_stack.len(), 1);
    assert_eq!(stores.par_fn_buffer_stack.last().unwrap(), &bytes);
    let popped = stores.par_fn_buffer_stack.pop();
    assert_eq!(popped, Some(bytes));
    assert!(stores.par_fn_buffer_stack.is_empty());
}

/// ARC.md A6.b — nested fused for-par: inner pop reveals outer
/// buffer underneath, unchanged.  Same load-bearing invariant as
/// `par_buffer_stack_nesting`.
#[test]
fn par_fn_buffer_stack_nesting() {
    let mut stores = loft::database::Stores::new();
    let outer = vec![0xAAu8; 20];
    let inner = vec![0xBBu8; 40];
    stores.par_fn_buffer_stack.push(outer.clone());
    stores.par_fn_buffer_stack.push(inner.clone());
    assert_eq!(stores.par_fn_buffer_stack.len(), 2);
    assert_eq!(stores.par_fn_buffer_stack.last().unwrap(), &inner);
    stores.par_fn_buffer_stack.pop();
    assert_eq!(
        stores.par_fn_buffer_stack.last().unwrap(),
        &outer,
        "outer buffer must survive inner pop"
    );
}

/// ARC.md A6.b — `Stores::clone()` resets `par_fn_buffer_stack` to
/// empty; runtime state, not type schema.
#[test]
fn par_fn_buffer_stack_initially_empty_in_clone() {
    let mut stores = loft::database::Stores::new();
    stores.par_fn_buffer_stack.push(vec![1u8; 20]);
    let cloned = stores.clone();
    assert!(
        cloned.par_fn_buffer_stack.is_empty(),
        "Stores::clone must reset par_fn_buffer_stack — runtime state, \
         not type schema (cloned len = {})",
        cloned.par_fn_buffer_stack.len()
    );
}

// ── Plan-06 ARC.md A5 — Stitch::Reduce / par_fold runtime ───────────────────

/// A5 v1 acceptance: `par_fold` over `vector<integer>` correctly sums
/// to the expected i64 total.  Workers each accumulate over their slice;
/// main combines per-worker partials with the same fold fn.  The monoid
/// (i64 + i64, identity 0) is associative + commutative, so any worker
/// split produces the same result.
#[test]
fn par_fold_int_sum() {
    let code = r#"
fn add_pair(acc: integer, row: integer) -> integer { acc + row }
"#;
    let (mut state, data) = compile(code);
    // Sum 1..=100 = 5050.
    let values: Vec<i32> = (1..=100).collect();
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_add_pair");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    let result = run_parallel_fold(
        &state.database,
        program,
        fn_pos,
        &input,
        8,   // element_size: integer (8 bytes)
        0,   // init = 0
        4,   // n_threads
        &[], // no extras
    );
    assert_eq!(
        result, 5050,
        "par_fold int sum: expected 5050, got {result}"
    );
}

/// A5 acceptance: `par_fold` over an empty vector returns `init`
/// without dispatching workers.
#[test]
fn par_fold_empty_input() {
    let code = r#"
fn add_pair(acc: integer, row: integer) -> integer { acc + row }
"#;
    let (mut state, data) = compile(code);
    let input = build_int_vector(&mut state.database, &[]);

    let d_nr = data.def_nr("n_add_pair");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    let result = run_parallel_fold(&state.database, program, fn_pos, &input, 8, 42, 4, &[]);
    assert_eq!(
        result, 42,
        "par_fold empty: expected init (42), got {result}"
    );
}

/// A5 acceptance: max via fold (non-commutative-looking but still
/// associative — exercises a fold fn that isn't simple addition).
/// The fold returns the larger of (acc, row); init = i64::MIN
/// guarantees row wins on first call per worker.
#[test]
fn par_fold_max() {
    let code = r#"
fn max_pair(acc: integer, row: integer) -> integer {
    if row > acc { row } else { acc }
}
"#;
    let (mut state, data) = compile(code);
    let values: Vec<i32> = vec![3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9, 7, 9, 3];
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_max_pair");
    let fn_pos = data.def(d_nr).code_position;
    let program = worker_program(&state);

    // Use i32::MIN as init (well below any element value).
    let result = run_parallel_fold(
        &state.database,
        program,
        fn_pos,
        &input,
        8,
        i64::from(i32::MIN),
        4,
        &[],
    );
    assert_eq!(result, 9, "par_fold max: expected 9, got {result}");
}

/// A5 acceptance: single-thread fold matches the multi-thread result.
/// Confirms the combine step (main-thread partial reduction) doesn't
/// drop or double-count any partial.
#[test]
fn par_fold_single_thread_matches_multi_thread() {
    let code = r#"
fn add_pair(acc: integer, row: integer) -> integer { acc + row }
"#;
    let (mut state, data) = compile(code);
    let values: Vec<i32> = (1..=50).collect();
    let input = build_int_vector(&mut state.database, &values);

    let d_nr = data.def_nr("n_add_pair");
    let fn_pos = data.def(d_nr).code_position;

    let program_1 = worker_program(&state);
    let result_1 = run_parallel_fold(&state.database, program_1, fn_pos, &input, 8, 0, 1, &[]);

    let program_8 = worker_program(&state);
    let result_8 = run_parallel_fold(&state.database, program_8, fn_pos, &input, 8, 0, 8, &[]);

    assert_eq!(result_1, 1275, "1-thread sum 1..=50: expected 1275");
    assert_eq!(
        result_1, result_8,
        "1-thread vs 8-thread: {result_1} vs {result_8}"
    );
}
