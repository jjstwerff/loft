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
use loft::parallel::{WorkerProgram, run_parallel_int, run_parallel_text};
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
    // Verify that input stores are locked for workers (clone_for_worker).
    let code = r#"
struct Num { v: integer }
fn worker_id(r: const Num) -> integer { r.v }
"#;
    let (mut state, _) = compile(code);
    let values = vec![1i32, 2, 3];
    let input = build_int_vector(&mut state.database, &values);

    // After clone_for_worker, all stores in the clone should be locked.
    let worker_stores = state.database.clone_for_worker();
    for alloc in &worker_stores.allocations {
        assert!(alloc.is_locked(), "worker store should be locked read-only");
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
    let pfl_d_nr = p.data.def_nr("n_parallel_for_light");
    assert_ne!(pfl_d_nr, u32::MAX, "n_parallel_for_light must be defined");
    assert_eq!(
        p.data.def(pfl_d_nr).purity,
        Purity::Impure(ImpureCategory::ParCall),
        "parallel_for_light should be #impure(par_call)"
    );
    // Sanity: an unannotated stdlib fn should be Purity::Unknown
    // (the conservative default — phase 5b's analyser treats it as
    // ParentWrite-impure for safety until annotated).
    let now_d_nr = p.data.def_nr("n_now");
    if now_d_nr != u32::MAX {
        assert_eq!(
            p.data.def(now_d_nr).purity,
            Purity::Unknown,
            "n_now is unannotated → Purity::Unknown"
        );
    }
}
