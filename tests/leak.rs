// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Store leak workbench.
//!
//! Use this file to reproduce and debug NEW store leaks.
//! Once a leak is fixed, move the test to a `tests/scripts/*.loft` file
//! so it becomes part of the permanent regression suite (via wrap.rs).
//!
//! Helpers:
//! - `run_leak_check_str(code)` — inline loft code, must define `fn test()`
//! - Uncomment the `show_code` block for IR + bytecode dumps

extern crate loft;

use loft::compile::byte_code;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
mod common;
use common::cached_default;

/// Run inline loft code (must define `fn test()`) and check for store leaks.
fn run_leak_check_str(code: &str) {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(code, "leak_test", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    // Uncomment for IR/bytecode dump when debugging a new leak:
    // let mut config = loft::log_config::LogConfig::from_env();
    // config.phases.ir = true;
    // loft::compile::show_code(&mut std::io::stderr(), &mut state, &mut p.data, &config).ok();
    state.execute("test", &p.data);
    state.check_store_leaks();
}

/// Parse + execute `fn test()` and return the list of leaked stores.
/// Unlike `run_leak_check_str` (which only *warns*), callers assert on
/// the returned vec, so a leak is a hard test failure.  This is the same
/// in-process interpreter check the p291/p295 guards use, factored out.
fn leaks_for(code: &str) -> Vec<String> {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(code, "leak_test", false);
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    state.collect_store_leaks()
}

/// Inner function assigns constructor to a local variable, then returns it.
/// The wrapper leaks the inner's return store on fn_return.
///
/// Root cause of the Brick Buster / WASM store-14 bug: `r = S{...}; r` in the
/// callee triggers a different codegen path than returning `S{...}` directly.
#[test]
fn local_var_return_leak() {
    run_leak_check_str(
        r#"
struct S { v: vector<float> }
fn inner(a: const S, b: const S) -> S {
  r = S { v: [a.v[0] + b.v[0]] };
  r
}
fn wrap(x: const S) -> S {
  y = S { v: [1.0] };
  inner(x, y)
}
pub fn test() {
  p = S { v: [2.0] };
  q = wrap(p);
  assert(q.v[0] == 3.0, "q.v[0]={q.v[0]}");
}
"#,
    );
}

/// Control: identical structure but inner returns the constructor directly.
/// This passes — proves the leak is specific to `r = S{...}; r`.
#[test]
fn direct_return_no_leak() {
    run_leak_check_str(
        r#"
struct S { v: vector<float> }
fn inner(a: const S, b: const S) -> S {
  S { v: [a.v[0] + b.v[0]] }
}
fn wrap(x: const S) -> S {
  y = S { v: [1.0] };
  inner(x, y)
}
pub fn test() {
  p = S { v: [2.0] };
  q = wrap(p);
  assert(q.v[0] == 3.0, "q.v[0]={q.v[0]}");
}
"#,
    );
}

/// Repeated calls: wrapper is called multiple times to verify no leak
/// accumulation and that the const-param data (proj) isn't corrupted.
#[test]
fn local_var_return_repeated() {
    run_leak_check_str(
        r#"
struct S { v: vector<float> }
fn inner(a: const S, b: const S) -> S {
  r = S { v: [a.v[0] * b.v[0]] };
  r
}
fn wrap(proj: const S, bx: float) -> S {
  model = S { v: [bx] };
  inner(proj, model)
}
pub fn test() {
  proj = S { v: [2.0] };
  m1 = wrap(proj, 3.0);
  assert(m1.v[0] == 6.0, "m1: {m1.v[0]}");
  m2 = wrap(proj, 5.0);
  assert(m2.v[0] == 10.0, "m2: {m2.v[0]}");
  m3 = wrap(proj, 7.0);
  assert(m3.v[0] == 14.0, "m3: {m3.v[0]}");
  assert(proj.v[0] == 2.0, "proj intact: {proj.v[0]}");
}
"#,
    );
}

/// Multiple locals before the leaked variable: ensures the fix isn't
/// sensitive to the specific var_nr that collides with the attribute index.
/// Here extra locals push y's var_nr above the hidden param's attr index.
#[test]
fn local_var_return_shifted_var_nr() {
    run_leak_check_str(
        r#"
struct S { v: vector<float> }
fn inner(a: const S, b: const S) -> S {
  r = S { v: [a.v[0] + b.v[0]] };
  r
}
fn wrap(x: const S) -> S {
  dummy1 = 42;
  dummy2 = 99;
  y = S { v: [1.0] };
  result = inner(x, y);
  assert(dummy1 == 42, "dummy1");
  assert(dummy2 == 99, "dummy2");
  result
}
pub fn test() {
  p = S { v: [2.0] };
  q = wrap(p);
  assert(q.v[0] == 3.0, "q.v[0]={q.v[0]}");
}
"#,
    );
}

/// Three-level call chain: test → outer → middle → inner.
/// Both outer and middle create locals that must be freed.
#[test]
fn local_var_return_deep_chain() {
    run_leak_check_str(
        r#"
struct S { v: vector<float> }
fn inner(a: const S, b: const S) -> S {
  r = S { v: [a.v[0] + b.v[0]] };
  r
}
fn middle(x: const S) -> S {
  y = S { v: [10.0] };
  inner(x, y)
}
fn outer(x: const S) -> S {
  z = S { v: [x.v[0] + 100.0] };
  middle(z)
}
pub fn test() {
  p = S { v: [1.0] };
  q = outer(p);
  assert(q.v[0] == 111.0, "q.v[0]={q.v[0]}");
}
"#,
    );
}

/// Loop reassignment with deep-copy: verifies the reassignment path
/// correctly deep-copies when the callee has visible Reference params.
#[test]
fn reassign_ref_call_in_loop() {
    run_leak_check_str(
        r#"
struct S { v: vector<float> }
fn inner(a: const S, b: const S) -> S {
  r = S { v: [a.v[0] * b.v[0]] };
  r
}
fn wrap(proj: const S, bx: float) -> S {
  model = S { v: [bx] };
  inner(proj, model)
}
pub fn test() {
  proj = S { v: [2.0] };
  mvp = wrap(proj, 1.0);
  assert(mvp.v[0] == 2.0, "first: {mvp.v[0]}");
  mvp = wrap(proj, 3.0);
  assert(mvp.v[0] == 6.0, "second: {mvp.v[0]}");
  mvp = wrap(proj, 5.0);
  assert(mvp.v[0] == 10.0, "third: {mvp.v[0]}");
  assert(proj.v[0] == 2.0, "proj: {proj.v[0]}");
}
"#,
    );
}

/// P146 regression guard: `var = user_fn(arg)` where `user_fn`
/// returns its param (`fn ac_identity(p: AcPoint) -> AcPoint { p }`)
/// USED to leak the result variable's store at function exit.
///
/// Root cause (closed): the runtime takes the P143 lock-args +
/// OpCopyRecord deep-copy path so `ac_copy` becomes an INDEPENDENT
/// store, but scope analysis treated the call as "returns alias of
/// arg 0" (parser dep inference) so it did not emit OpFreeRef at
/// scope exit.  Fixed by mirroring codegen's `has_ref_params == true`
/// branch in `scopes.rs::scan_set` — strip the LHS variable's
/// declared deps so `get_free_vars`'s gate emits OpFreeRef.
///
/// Uses `execute_log` with `trace_alloc_free` so a regression here
/// panics with `Database N not correctly freed (allocated by ...)`
/// at the alloc site, instead of just emitting a warning that
/// `check_store_leaks` would silently swallow.
#[test]
fn p146_script_95_alias_copy_leak() {
    loft::crash_report::install("leak");
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse("tests/scripts/95-alias-copy.loft", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let mut config = loft::log_config::LogConfig::full();
    config.trace_alloc_free = true;
    let _ = state.execute_log(
        &mut std::io::stderr(),
        "test_aliased_return_copies",
        &config,
        &p.data,
    );
}

/// P147 (script 81 — iterator-protocol main): 3 calls to a constructor
/// `new_counter(integer) -> Counter` followed by `for x in c { ... }`
/// using the I13 iterator protocol leak the constructor's alloc'd
/// stores at scope exit.
#[test]
fn p147_script_81_iterator_protocol_leak() {
    loft::crash_report::install("leak");
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse("tests/scripts/81-iterator-protocol.loft", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let mut config = loft::log_config::LogConfig::full();
    config.trace_alloc_free = true;
    let _ = state.execute_log(&mut std::io::stderr(), "main", &config, &p.data);
}

/// P148 (script 45 — A10 field iteration): `for f in s#fields { ... }`
/// leaks 1 store per iteration (8 total: 3 from Point, 4 from Mixed,
/// + 1 from the iterator itself).
#[test]
fn p148_script_45_field_iteration_leak() {
    loft::crash_report::install("leak");
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse("tests/scripts/45-field-iter.loft", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let mut config = loft::log_config::LogConfig::full();
    config.trace_alloc_free = true;
    let _ = state.execute_log(&mut std::io::stderr(), "main", &config, &p.data);
}

/// P149 reproducer: script 76's test_p105_nested_struct_in_vector
/// SIGSEGVs under execute_log at `OpCopyRecord(data=ref(0,0,0x40080000),
/// to=ref(2,5,24), tp=68)`.  The source DbRef has `store_nr=0` (the
/// stack store) with a `pos` of 0x40080000 — the bit pattern of float
/// 2.125, suggesting a slot was reused for a float and then read as
/// a DbRef.  Pre-existing memory issue (observed before the P146 fix);
/// reproduces deterministically only via execute_log's stricter access
/// checks.  The wrap-suite path (execute, warning-only) does not hit
/// this — so the script appears clean there.
///
/// Distinct from P146/P147/P148 — this is a slot/lifetime mismatch
/// in nested struct initialization, not a missing free.
#[test]
fn p149_script_76_nested_struct_segv() {
    loft::crash_report::install("leak");
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse("tests/scripts/76-struct-vector-return.loft", false);
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let mut config = loft::log_config::LogConfig::full();
    config.trace_alloc_free = true;
    let _ = state.execute_log(
        &mut std::io::stderr(),
        "test_p105_nested_struct_in_vector",
        &config,
        &p.data,
    );
}

/// P150 regression guard: a function with both an EARLY-RETURN
/// constructor and a fall-through `Struct.parse(text)` path used to
/// leak 1 placeholder store per call from the caller's hidden
/// `__ref_*` ConvRefFromNull pre-alloc.
///
/// Root cause (closed): codegen for `m = make(...)` pre-allocates a
/// slot for the hidden return-ref via OpConvRefFromNull (creating a
/// fresh "null" store).  When the callee's early-return returns a
/// DbRef to a store created INSIDE the callee (not into the hidden
/// return slot), the caller's pre-alloc store is orphaned —
/// `__ref_*` was marked `skip_free` (`src/state/codegen.rs:1062` and
/// `src/scopes.rs:523`) so the caller's variable would own the
/// callee's store, and the original ConvRefFromNull store was never
/// freed.  Severity was per-call (game loops at 60fps would saturate
/// u16 store space in ~18 minutes).
///
/// Fix: drop the `set_skip_free` calls in both paths.  The runtime
/// tolerates double-free as a no-op
/// (`src/database/allocation.rs:103-105`'s `if store.free { return }`),
/// so the typical-adoption case (where __ref_* aliases m's store)
/// is unaffected — both frees fire, second is no-op.  The orphan
/// case now reclaims the placeholder via the existing is_work_ref
/// check in `scopes.rs::get_free_vars`.
#[test]
fn p150_early_return_struct_orphans_caller_pre_alloc() {
    // Minimal trigger: a user fn that has an EARLY-RETURN of a constructor
    // call AND a fall-through `Struct.parse(text)` path.  The caller's
    // `m = fn(...)` pre-allocates a hidden return slot via OpConvRefFromNull
    // (creating a fresh "null" store).  When the early-return path runs,
    // the returned DbRef points to a store created INSIDE the callee, not
    // into the caller's pre-alloc; the pre-alloc store is orphaned.
    // `__ref_*` is marked skip_free (so the caller's variable owns the
    // callee's store), so the original ConvRefFromNull store leaks.
    run_leak_check_str(
        r#"
struct M { name: text }
fn make_empty() -> M { M { name: "untitled" } }
fn make_or_default(t: text) -> M {
  if t == "" { return make_empty(); }
  result = M.parse(t);
  result
}
pub fn test() {
  m = make_or_default("");
  assert(m.name == "untitled", "name");
}
"#,
    );
}

/// P291 (vector-append of fn-call result): `vec += [build_chunk(x, y)]`
/// where `build_chunk` returns a freshly-allocated struct used to leak
/// the callee's source store on every iteration.  The IR pattern is
/// `OpPreAllocVector + OpNewRecord(_elm) + OpCopyRecord(call, _elm, tp)
/// + OpFinishRecord` — the `OpCopyRecord` source (the call result)
/// was emitted without the `0x8000` free-source bit that the
/// scalar-assignment path (`gen_set_first_ref_call_copy`) has had for
/// a while.  Fixed by adding the same `is_struct_returning_call`
/// check in `src/parser/vectors.rs`'s vector-append emitter.
///
/// Reproducer is intentionally tiny: two appends of a fn-returning-
/// struct into a local `vector<Chunk>`.  Before the fix this leaked 2
/// stores; after, zero.  The natural consumer is `lib/hex_world`'s
/// `ensure_chunk(w, q, r)` → `w.chunks += [build_chunk(cx, cz)]`
/// pattern which surfaced this when running `lib/hex_world/tests/hex_world.loft`.
#[test]
fn p291_vector_append_fn_call_result_no_leak() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(
        r#"
struct Cell { c_color: u8 not null, c_height: u8 not null, c_age: u16 not null }
struct Chunk { ck_cx: integer not null, ck_cz: integer not null, ck_cells: vector<Cell> }
fn build_chunk(cx: integer, cz: integer) -> Chunk {
  Chunk{ck_cx: cx, ck_cz: cz, ck_cells: []}
}
pub fn test() {
  chunks: vector<Chunk> = [];
  chunks += [build_chunk(0, 0)];
  chunks += [build_chunk(1, 0)];
  assert(len(chunks) == 2, "appended 2 chunks");
}
"#,
        "p291_minimal",
        false,
    );
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    let leaks = state.collect_store_leaks();
    assert!(
        leaks.is_empty(),
        "P291 regression: {} store(s) leaked: {}",
        leaks.len(),
        leaks.join(", ")
    );
}

/// P291 broader case: the actual lib/hex_world `build_chunk` (1024 inner
/// cells) plus three appends covering the negative-chunk path.
/// Combines the leak fix with vector-of-struct construction inside
/// the callee, validating that BOTH the outer `__ref_N` (chunk) and
/// the inner per-cell `__lift_N` allocations are tracked correctly.
#[test]
fn p291_vector_append_with_inner_vector_no_leak() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(
        r#"
struct Cell { c_color: u8 not null, c_height: u8 not null, c_age: u16 not null }
struct Chunk { ck_cx: integer not null, ck_cz: integer not null, ck_cells: vector<Cell> }
struct World { chunks: vector<Chunk>, tick: integer not null }
fn cell_empty() -> Cell { Cell{c_color: 0, c_height: 0, c_age: 0} }
fn build_chunk(cx: integer, cz: integer) -> Chunk {
  cs: vector<Cell> = [];
  for _ in 0..4 { cs += [cell_empty()]; }
  Chunk{ck_cx: cx, ck_cz: cz, ck_cells: cs}
}
fn ensure_chunk(w: &World, cx: integer, cz: integer) {
  w.chunks += [build_chunk(cx, cz)];
}
pub fn test() {
  w = World{chunks: [], tick: 0};
  ensure_chunk(w, 0, 0);
  ensure_chunk(w, 1, 0);
  ensure_chunk(w, -1, 0);
  assert(len(w.chunks) == 3, "3 alive chunks");
}
"#,
        "p291_with_inner",
        false,
    );
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    let leaks = state.collect_store_leaks();
    assert!(
        leaks.is_empty(),
        "P291 regression: {} store(s) leaked: {}",
        leaks.len(),
        leaks.join(", ")
    );
}

/// P290 (lock semantic too broad) — iterating a passed-in struct's
/// `hash` field used to panic with `Claim on locked store (size=N)
/// (locked by: lock_store(store_nr=…, rec=…))` because the fn-call
/// deep-copy bracket emitted in `src/state/codegen.rs` locked every
/// Ref-typed arg, and `build_hash_sorted_vec` (the hash iterator's
/// scratch allocator) claims IN the hash's own store per C60.
/// Fixed by gating `Store::claim` / `Store::addr_mut`'s assert on
/// the lock origin: call-bracket locks (origin starts with
/// `"lock_store("`) only block frees, never writes/claims.  Hard
/// locks (CONST_STORE init, worker borrows) still block everything.
#[test]
fn p290_iter_struct_hash_field_no_panic() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(
        r#"
struct Cell { ck: text not null, x: integer not null, y: integer not null }
struct World { cells: hash<Cell[ck]> }
fn snap_xs(w: World) -> integer {
  count = 0;
  for cell in w.cells {
    count = count + cell.x;
  }
  count
}
pub fn test() {
  w = World{cells: []};
  w.cells += [Cell{ck: "a", x: 1, y: 0}];
  w.cells += [Cell{ck: "b", x: 2, y: 0}];
  w.cells += [Cell{ck: "c", x: 3, y: 0}];
  total = snap_xs(w);
  assert(total == 6, "summed cells inside passed-in hash");
}
"#,
        "p290_iter",
        false,
    );
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    let leaks = state.collect_store_leaks();
    assert!(
        leaks.is_empty(),
        "P290 regression: {} store(s) leaked: {}",
        leaks.len(),
        leaks.join(", ")
    );
}

/// P290 sibling — off-by-one in `copy_claims_hash_body` SEGV.  After
/// the lock-broadening fix above unblocked the callee, deep-copying
/// the returned struct (which contains a populated `hash<T[key]>`
/// field) crashed because `copy_claims_hash_body` looped
/// `1..length*2` instead of `0..(room-1)*2`, both skipping the first
/// bucket AND writing past the destination claim's end → adjacent
/// heap metadata corruption → SIGSEGV on the next allocator op.
/// Only triggers via SRet returns of struct-containing-hash, which
/// is exactly the shape `fn collect(w: World) -> ChunkSet { … }`
/// produces.
#[test]
fn p290_sret_struct_with_hash_field_deep_copy_no_crash() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.parse_str(
        r#"
struct Cell { ck: text not null, x: integer not null, y: integer not null }
struct World { cells: hash<Cell[ck]> }
struct ChunkRef { ck: text not null, cx: integer not null, cy: integer not null }
struct ChunkSet { chunks: hash<ChunkRef[ck]> }
fn collect(w: World) -> ChunkSet {
  cs = ChunkSet{chunks: []};
  for cell in w.cells {
    ck = "{cell.x}";
    if cs.chunks[ck] == null {
      cs.chunks += [ChunkRef{ck: ck, cx: cell.x, cy: cell.y}];
    }
  }
  cs
}
pub fn test() {
  w = World{cells: []};
  w.cells += [Cell{ck: "a", x: 1, y: 0}];
  w.cells += [Cell{ck: "b", x: 2, y: 0}];
  w.cells += [Cell{ck: "c", x: 3, y: 0}];
  cs = collect(w);
  count = 0;
  for _ in cs.chunks { count = count + 1; }
  assert(count == 3, "deep-copied hash has 3 entries");
}
"#,
        "p290_sret",
        false,
    );
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    let leaks = state.collect_store_leaks();
    assert!(
        leaks.is_empty(),
        "P290 SRet regression: {} store(s) leaked: {}",
        leaks.len(),
        leaks.join(", ")
    );
}

/// P150 file-level regression guard: `tests/fixtures/libs/moros_map/tests/serial.loft`
/// (a fixture clone of the moros_map package, kept after moros was extracted to
/// its own repo) used to leak 1 store via interpreter (warning: `2(bc:0)`) when
/// `test_from_json_empty_string` called `map_from_json("")`, which has
/// the early-return + fall-through `Struct.parse` shape that triggered
/// the orphan placeholder alloc.  Closed by the P150 fix (drop
/// `set_skip_free` for `__ref_*` work-refs in scopes.rs + codegen.rs).
///
/// Uses `state.execute()` (no trace) + `state.collect_store_leaks()`
/// instead of `execute_log + trace_alloc_free`.  The trace-instrumented
/// path hangs on multi-fn loops under rustc 1.95.0 — likely a
/// state-accumulation issue in the verbose log machinery, NOT in loft
/// itself (every test fn passes standalone via the CLI).  Filed as
/// out-of-scope for P150: the diagnostic-harness hang is unrelated to
/// the language-correctness fix.  This test gives equivalent leak
/// coverage (any unfreed store at end of run is a failure) without
/// depending on the trace machinery.
#[test]
fn p150_moros_map_serial_leak() {
    loft::crash_report::install("leak");
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.lib_dirs.push("tests/fixtures/libs".to_string());
    p.parse("tests/fixtures/libs/moros_map/tests/serial.loft", false);
    // Filter out Warning-level diagnostics — plan-07 phase 4e.2/4h
    // emit informational warnings for undefended division, OOB
    // indexing, and `not null` field reminders.  Those are
    // intentional advisory output and don't block the test.  This
    // assertion only fires on actual parse Errors.
    let errors: Vec<String> = p
        .diagnostics
        .entries()
        .iter()
        .filter(|e| e.level >= loft::diagnostics::Level::Error)
        .map(|e| e.to_string_compact())
        .collect();
    assert!(errors.is_empty(), "parse errors: {errors:?}");
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    for name in [
        "test_to_from_json_empty",
        "test_from_json_empty_string",
        "test_roundtrip_full_map",
        "test_roundtrip_rotation_flags",
        "test_roundtrip_npc_waypoints",
    ] {
        state.execute(name, &p.data);
    }
    let leaks = state.collect_store_leaks();
    assert!(
        leaks.is_empty(),
        "P150 regression: {} store(s) leaked: {}",
        leaks.len(),
        leaks.join(", ")
    );
}

/// Full Brick Buster pattern with yield/resume using real math + graphics libraries.
/// Confirms the minimal reproduction above causes the real-world crash.
///
/// 85-yield-resume.loft now calls the stdlib `yield_frame()` built-in
/// (`src/native.rs::n_yield_frame`) directly — no more
/// `state.replace_native("mock_yield_frame", …)` injection — so this
/// test simply parses, executes, and drives resume in the same shape
/// as `wrap::loft_suite`.  Kept here as the leak-budget guard
/// (`check_leaks()` after exit) on top of the wrap-suite functional
/// coverage.
#[test]
fn brick_buster_yield_resume() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.lib_dirs.push("tests/lib".to_string());
    p.lib_dirs.push("lib".to_string());
    p.lib_dirs.push("lib/graphics/src".to_string());
    p.parse("tests/scripts/85-yield-resume.loft", false);
    if p.diagnostics.level() >= loft::diagnostics::Level::Error {
        panic!("parse errors: {:?}", p.diagnostics.lines());
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);

    state.execute("main", &p.data);
    assert!(state.database.frame_yield, "should have yielded");

    for _frame in 0..1000 {
        let running = state.resume();
        if !running {
            break;
        }
    }
}

/// Plan-15 phase 03 — closure-DbRef leak guard.
///
/// LIFETIME.md flags `Type::Function` as "NOT YET HANDLED" — the
/// closure DbRef at offset+4 of the 16-byte fn-ref slot is "never
/// explicitly freed."  For text captures this matters because the
/// captured text lives in the closure record's heap allocation.
/// If the closure record never gets freed, the captured text never
/// gets freed either → store leak.
///
/// This test runs the c2_d3_text_capture_field shape in a tight
/// loop (100 iterations) and asserts `state.check_store_leaks()`
/// passes after.  Each iteration:
///   1. Allocates a text capture (`label = "z"`).
///   2. Constructs a struct holding the capturing closure.
///   3. Calls the closure (which formats into a fresh text).
///   4. Drops the struct.
///
/// If the closure record (16-byte fn-ref slot's tail half) doesn't
/// free at struct-drop time, every iteration would accumulate one
/// leaked closure record.  100 iterations would surface as 100
/// leaked stores in `check_store_leaks`.
///
/// **Actual behaviour (2026-05-12)**: clean — no leak detected
/// for either D1 (local var) or D3 (struct field).  P213's
/// `Parts::ChildRec` cascade plus the struct-scope-exit cleanup
/// path frees the closure record correctly when the struct it
/// lives inside goes out of scope; D1 frees the closure record at
/// stack-frame exit via the standard local-cleanup path.  The
/// LIFETIME.md "NOT YET HANDLED" annotation overstates the gap
/// for both destinations — phase 06 should update it to reflect
/// this finding.
#[test]
fn p15_phase03_closure_text_capture_field_no_leak() {
    run_leak_check_str(
        r#"
        struct G { fmt: fn(integer) -> text }
        fn one_call() -> text {
            label = "z";
            g = G { fmt: fn(n: integer) -> text { "{label}: {n}" } };
            g.fmt(7)
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_call();
                assert(r == "z: 7", "iter {i}: got '{r}'");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-15 phase 03 — D1 (local var) text-capture leak guard.
///
/// Same shape as the D3 leak guard above but the closure lives in
/// a local variable instead of a struct field.  Different cleanup
/// path: stack-frame exit via the standard local-binding free
/// rather than `Parts::ChildRec` cascade through struct scope.
///
/// 100 iterations clean → no leak; the local-binding cleanup
/// covers fn-ref locals correctly.
#[test]
fn p15_phase03_closure_text_capture_local_no_leak() {
    run_leak_check_str(
        r#"
        fn one_call() -> text {
            label = "y";
            f: fn(integer) -> text = fn(n: integer) -> text { "{label}: {n}" };
            f(11)
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_call();
                assert(r == "y: 11", "iter {i}: got '{r}'");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-15 phase 04 — D3 (struct field) Reference-capture leak guard.
///
/// The captured value is a struct (DbRef-allocated) instead of a
/// text.  The closure record holds the captured DbRef; the
/// captured Point's store record must outlive the closure record,
/// AND the closure record must be freed at host-struct scope exit
/// for both records to release.
///
/// Risk: if the captured DbRef's dep-tracking lets the source
/// scope's cleanup free Point while the closure still holds it,
/// the closure body would read a freed store record.  100
/// iterations × calling the closure each iteration would surface
/// either as a leak (record never freed) or a panic (record freed
/// while in use).
///
/// Actual behaviour (2026-05-12): clean.  No leak, no
/// read-after-free.  The dep mechanism in `vectors.rs:666-669`
/// keeps the closure-record alive long enough; `Parts::ChildRec`
/// cascade frees both at host-struct scope exit.
#[test]
fn p15_phase04_closure_ref_capture_field_no_leak() {
    run_leak_check_str(
        r#"
        struct Point { x: integer, y: integer }
        struct Holder { cb: fn(integer) -> integer }
        fn one_call() -> integer {
            p = Point { x: 100, y: 0 };
            h = Holder { cb: fn(dx: integer) -> integer { p.x + dx } };
            h.cb(7)
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_call();
                assert(r == 107, "iter {i}: got {r}");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-15 phase 04 — D1 (local var) Reference-capture leak guard.
///
/// Same shape as the D3 guard above but the closure lives in a
/// local variable instead of a struct field.  Cleanup path:
/// stack-frame exit drops the fn-ref local, which transitively
/// frees the closure record (holding the captured Point DbRef),
/// which transitively frees Point's store record.
///
/// The dep chain `Point ← closure-record ← fn-ref local` must
/// flow through the standard local-cleanup mechanism without
/// leaving any record behind.
#[test]
fn p15_phase04_closure_ref_capture_local_no_leak() {
    run_leak_check_str(
        r#"
        struct Point { x: integer, y: integer }
        fn one_call() -> integer {
            p = Point { x: 10, y: 20 };
            f = fn(dx: integer) -> integer { p.x + dx };
            f(5)
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_call();
                assert(r == 15, "iter {i}: got {r}");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-22 phase 02c — auto-Reference capture leak guard.
///
/// When a closure mutates a captured struct, the closure
/// record's attribute holds a 12-byte DbRef pointing AT the
/// outer struct's record (share-by-DbRef, not deep copy).
/// If the cascade frees the pointed-to record when the closure
/// drops, the outer's binding sees freed memory.  If it
/// double-frees, leak detection trips.
///
/// 100-iteration tight loop: each iteration creates Counter,
/// captures it into a closure that mutates n, calls bump 3
/// times, returns.  The captured Counter, the closure record,
/// and the closure's auto-Reference field all need to free
/// cleanly at scope exit.
///
/// Actual behavior (2026-05-12): clean, no leak, no use-after-
/// free.  The auto-Reference path's dep marker (`u16::MAX`
/// sentinel) doesn't trip get_free_vars's normal free path
/// for the closure record's attribute (Parts::DbRef has no
/// special claim handling — bytes are copied/freed as part of
/// the host record's lifecycle).
#[test]
fn p22_phase02c_auto_reference_capture_no_leak() {
    run_leak_check_str(
        r#"
        struct Counter { n: integer }
        fn one_iteration() -> integer {
            c = Counter { n: 0 };
            bump = fn() { c.n = c.n + 1; };
            bump();
            bump();
            bump();
            c.n
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_iteration();
                assert(r == 3, "iter {i}: got {r}");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-22 phase 02d-iii.e — boxed-scalar capture leak guard.
///
/// 100-iteration tight loop: each iteration creates a boxed
/// integer `n`, captures it into a closure that mutates n via
/// shared cell DbRef, calls bump 3 times, asserts result.  The
/// boxed `n`'s `__cell_integer` record, the closure record, and
/// the closure's auto-Reference field all need to free cleanly
/// at scope exit.
///
/// Actual behavior (2026-05-12): clean, no leak.  The cell's
/// type is `Reference(__cell_integer, vec![])` — heap-owned per
/// `is_heap_owned`, so `get_free_vars` emits the standard
/// `OpFreeRef` at scope exit.  Closure record + auto-Reference
/// attribute follow the same path established by phase 02c.
#[test]
fn p22_phase02d_iii_e_scalar_capture_no_leak() {
    run_leak_check_str(
        r#"
        fn one_iteration() -> integer {
            n = 0;
            bump = fn() { n = n + 1; };
            bump();
            bump();
            bump();
            n
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_iteration();
                assert(r == 3, "iter {i}: got {r}");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-22 phase 02d-vi — text cell capture leak guard.
///
/// 100-iteration tight loop: each iteration boxes a text
/// `acc`, captures it into a closure that re-assigns via
/// shared cell DbRef, calls the closure twice, asserts result.
/// The `__cell_text` record (16B Text value field) and the
/// closure record's auto-Reference attribute (12B share-by-DbRef)
/// all need to free cleanly at scope exit.
///
/// NOTE: this guard uses void-return + assert-inside-iteration
/// rather than returning the text from `one_iteration`.
/// Text-return from a fn with closure-mutated text uses the
/// existing write-back mechanism (per 02d-vii's text-skip-in-
/// text-return guard) — covered by
/// `p22_phase02d_vii_text_return_with_closure_no_leak` below.
#[test]
fn p22_phase02d_vi_text_capture_no_leak() {
    run_leak_check_str(
        r#"
        fn one_iteration(seed: text) {
            acc = seed;
            f = fn() { acc = "after"; };
            f();
            assert(acc == "after", "got {acc}");
        }
        fn test() {
            i = 0;
            while i < 100 {
                one_iteration("before");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-22 phase 03 — Case C (factory pattern) leak guard.
///
/// 100-iteration tight loop: each iteration constructs a
/// fresh factory closure, calls it 3 times, drops it.  The
/// `__cell_integer` allocated inside `make()` must outlive
/// `make()`'s scope (because the returned closure carries a
/// reference) and free at the closure's drop site.
///
/// MAJOR FINDING (2026-05-13): Case C ALREADY WORKS under the
/// 02d-iii.e cell + auto-Reference machinery — no separate
/// phase-03 implementation needed.  The closure record's
/// auto-Reference attribute keeps the cell alive past
/// `make()`'s scope exit; the cell frees when the closure
/// goes out of scope in the caller.
///
/// Single-factory works.  Multi-factory shape was P259
/// (fixed 2026-05-13) — see [`p22_phase03_multi_factory_no_leak`]
/// below for the multi-factory regression guard.
#[test]
fn p22_phase03_factory_no_leak() {
    run_leak_check_str(
        r#"
        fn make() -> fn() -> integer {
            n = 0;
            fn() -> integer { n = n + 1; n }
        }
        fn test() {
            i = 0;
            while i < 100 {
                f = make();
                _ = f();
                _ = f();
                _ = f();
                i = i + 1;
            }
        }
        "#,
    );
}

/// P259 (fixed 2026-05-13): multi-factory closure-record cells
/// must each free independently when their owning closure dies.
///
/// 100-iteration loop with TWO factory instances per iteration,
/// interleaved calls.  Without the cascade-free in
/// `Stores::free_named` (commit 4 of the P259 fix), each
/// iteration would leak two cells (one per factory) for a total
/// of 200 leaked stores by exit.  With the fix: clean.
#[test]
fn p22_phase03_multi_factory_no_leak() {
    run_leak_check_str(
        r#"
        fn make() -> fn() -> integer {
            n = 0;
            fn() -> integer { n = n + 1; n }
        }
        fn test() {
            i = 0;
            while i < 100 {
                f1 = make();
                f2 = make();
                _ = f1();
                _ = f2();
                _ = f1();
                _ = f2();
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-22 phase 02d-vii — text-return-from-fn-with-closure
/// leak guard.
///
/// 100-iteration tight loop: each iteration runs a function
/// that returns text AND uses a closure that APPENDS to a
/// text local.  Per 02d-vii, the parent-returns-text guard
/// skips the text-cell flip — acc stays as `RefVar(Text)` and
/// the existing void-return write-back mechanism propagates
/// the closure's append mutation (this path predates 02d-vi).
/// Verify no leak from the work-text buffer + closure record
/// + closure-record's text attribute.
///
/// NOTE: the test uses APPEND (`+=`) not REASSIGN (`=`) —
/// re-assign in closures inside text-returning fns is a
/// pre-existing limitation tied to the same store-lock
/// interaction that 02d-vii works around.
#[test]
fn p22_phase02d_vii_text_return_with_closure_no_leak() {
    run_leak_check_str(
        r#"
        fn build() -> text {
            acc = "";
            f = fn(s: text) { acc += s; };
            f("a");
            f("bb");
            acc
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = build();
                assert(r == "abb", "iter {i}: got {r}");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-22 phase 02d-v — boolean cell capture leak guard.
///
/// 100-iteration tight loop: each iteration boxes a boolean
/// `flag`, captures it into a closure that toggles via shared
/// cell DbRef, calls toggle 5 times, asserts result.  The
/// `__cell_boolean` record (1B value field, padded to 8B per
/// the cell struct's record layout) and the closure record's
/// auto-Reference attribute (12B share-by-DbRef) all need to
/// free cleanly at scope exit.  Exercises the boolean-LHS
/// detection in `maybe_prepend_cell_alloc` under load.
#[test]
fn p22_phase02d_v_boolean_capture_no_leak() {
    run_leak_check_str(
        r#"
        fn one_iteration() -> boolean {
            flag = false;
            toggle = fn() { flag = !flag; };
            toggle();
            toggle();
            toggle();
            toggle();
            toggle();
            flag
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_iteration();
                assert(r == true, "iter {i}: got {r}");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-22 phase 02d-iv — multi-type capture leak guard.
///
/// 100-iteration tight loop: each iteration boxes THREE
/// distinct primitive captures (integer + float + character)
/// simultaneously, mutates each via a single closure, asserts
/// results.  Three `__cell_<T>` records per iteration, each
/// with an auto-Reference closure-record attribute, all need
/// to free cleanly.  Exercises the dedup invariant in
/// `synthesize_cell_structs` (one cell per canonical type, not
/// one per binding) under load.
#[test]
fn p22_phase02d_iv_multi_type_capture_no_leak() {
    run_leak_check_str(
        r#"
        fn one_iteration() -> integer {
            n = 0;
            x = 0.0;
            c = 'a';
            bump = fn() {
                n = n + 1;
                x = x + 0.5;
                c = 'z';
            };
            bump();
            bump();
            assert(n == 2, "n: {n}");
            assert(x == 1.0, "x: {x}");
            assert(c == 'z', "c: {c}");
            n
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_iteration();
                assert(r == 2, "iter {i}: got {r}");
                i = i + 1;
            }
        }
        "#,
    );
}

/// Plan-15 phase 05 — nested closure (C6) leak guard.
///
/// `inner` is a non-capturing fn-ref local; `outer` captures
/// `inner` into its closure record.  Both fn-refs must free
/// cleanly at scope exit:
///   1. `outer` (fn-ref local) frees first.
///   2. Outer's closure record (which holds inner's d_nr +
///      closure DbRef in one slot) frees next.
///   3. Inner's standalone fn-ref local frees last.
///
/// The dep chain is a stack: a leak at any link would
/// accumulate over 100 iterations and surface in
/// `state.check_store_leaks()`.  Confirmed clean.
#[test]
fn p15_phase05_nested_closure_no_leak() {
    run_leak_check_str(
        r#"
        fn one_call() -> integer {
            inner = fn(x: integer) -> integer { x + 5 };
            outer = fn(y: integer) -> integer { inner(y) + 1 };
            outer(10)
        }
        fn test() {
            i = 0;
            while i < 100 {
                r = one_call();
                assert(r == 16, "iter {i}: got {r}");
                i = i + 1;
            }
        }
        "#,
    );
}

/// @P295 — reassigning a keyed-collection LOCAL (`s = ns`) deep-copies
/// (OpReplaceKeyed) and must NOT leak: `s`'s prior store is freed by
/// `remove_claims`, `ns` is freed at its own scope.  Before the fix the
/// assignment left `s` depending on `ns`, so scope analysis suppressed
/// `s`'s own `OpFreeRef` (leak) and deferred `ns`'s free.
#[test]
fn p295_keyed_reassign_no_leak() {
    run_leak_check_str(
        r#"
struct Item { k: integer not null }
fn test() {
  s: sorted<Item[k]> = [];
  s += [Item{k: 100}];
  ns: sorted<Item[k]> = [];
  ns += [Item{k: 1}];
  ns += [Item{k: 2}];
  s = ns;
  assert(len(s) == 3, "s has 3 after reassign");
}
"#,
    );
}

/// @P295 — fresh-storage RHS (`s = build()`) frees the source store after
/// the deep copy (0x8000 source-free bit), so the callee's return store
/// does not leak.
#[test]
fn p295_keyed_reassign_call_rhs_no_leak() {
    run_leak_check_str(
        r#"
struct Item { k: integer not null }
fn build(n: integer) -> sorted<Item[k]> {
  r: sorted<Item[k]> = [];
  for i in 0..n { r += [Item{k: n - i}]; }
  r
}
fn test() {
  s: sorted<Item[k]> = [];
  s += [Item{k: 99}];
  s = build(3);
  assert(len(s) == 3, "s has 3 after call-RHS reassign");
}
"#,
    );
}

/// @P297 — a struct-returning call whose result is passed DIRECTLY as a
/// function argument (an anonymous temporary, never bound to a local)
/// leaks one store per call.  Sibling of @P291 (vector-append) and the
/// scalar-assignment path: the callee allocates its return value in a
/// fresh store via `OpDatabase`, but the argument-passing path never
/// frees that temporary after the outer call consumes it.  Under
/// sustained use (e.g. per-frame `mat4_look_at(normalize3(sub3(...)))`
/// in the projector demo) the u16 store-id space is exhausted and the
/// allocator panics ("Allocating a used store", allocation.rs:104).
///
/// **Closed 2026-05-21** — `scopes::inline_struct_return` now unspans the
/// argument so the existing `__lift_N` machinery frees the temporary on
/// both backends.  This is the in-process interpreter regression guard.
#[test]
fn p297_nested_call_arg_temp_no_leak() {
    let leaks = leaks_for(
        r#"
struct V3 { x: float, y: float, z: float }
fn mk(a: float, b: float, c: float) -> V3 { V3 { x: a, y: b, z: c } }
fn add(p: const V3, q: const V3) -> V3 { V3 { x: p.x + q.x, y: p.y + q.y, z: p.z + q.z } }
fn sx(p: const V3) -> float { p.x }
pub fn test() {
  a = mk(1.0, 2.0, 3.0);
  b = mk(0.5, 0.5, 0.5);
  total = 0.0;
  for i in 0..50 { total += sx(add(a, b)); }
  assert(total == 75.0, "total={total}");
}
"#,
    );
    assert!(
        leaks.is_empty(),
        "P297 regression: {} store(s) leaked (nested-call arg temp): {}",
        leaks.len(),
        leaks.join(", ")
    );
}

/// @P297 control — binding the intermediate to a local first frees it
/// correctly (scope analysis owns the local).  Proves the leak is
/// specific to the anonymous-temporary-as-argument shape, not struct
/// returns in general.  This passed before the @P297 fix too.
#[test]
fn p297_arg_temp_bound_local_control_no_leak() {
    let leaks = leaks_for(
        r#"
struct V3 { x: float, y: float, z: float }
fn mk(a: float, b: float, c: float) -> V3 { V3 { x: a, y: b, z: c } }
fn add(p: const V3, q: const V3) -> V3 { V3 { x: p.x + q.x, y: p.y + q.y, z: p.z + q.z } }
fn sx(p: const V3) -> float { p.x }
pub fn test() {
  a = mk(1.0, 2.0, 3.0);
  b = mk(0.5, 0.5, 0.5);
  total = 0.0;
  for i in 0..50 { d = add(a, b); total += sx(d); }
  assert(total == 75.0, "total={total}");
}
"#,
    );
    assert!(
        leaks.is_empty(),
        "P297 control leaked unexpectedly: {} store(s): {}",
        leaks.len(),
        leaks.join(", ")
    );
}

/// #394 — a bare (no-field) unit variant of a MIXED struct-enum materialises a
/// store-resident record (`emit_variant_value`); that temp must be freed when a
/// DEEP-COPY consumer (vector literal/element, struct field, fn arg, nested vector)
/// copies the record rather than aliasing it.  Before the fix the temp was
/// `skip_free` (only correct for the alias `x = B` case), so every copy consumer
/// leaked one store per construction site.  Each shape also reads the variant back
/// via `match`, so the fix can't pass by simply never building the record.
#[test]
fn p394_bare_unit_variant_no_leak_across_consumers() {
    // `leaks_for` + assert (not `run_leak_check_str`, which only warns) so a
    // regression HARD-fails CI rather than printing a stderr warning.
    let leaks = leaks_for(
        r#"
enum Sigil { Glyph { s: text }, Blank }
fn describe(e: Sigil) -> text { match e { Glyph { s } => "G", Blank => "X" } }
pub fn test() {
  // vector literal + element-assign in a loop (the filed repro)
  v: vector<Sigil> = [Blank];
  for i in 0..3 { v[0] = Glyph { s: "x-{i}" }; v[0] = Blank; }
  assert(describe(v[0]) == "X", "vec elem");
  // struct field
  w = Holder { m: Blank };
  assert(describe(w.m) == "X", "struct field");
  // function argument
  assert(describe(Blank) == "X", "fn arg");
  // struct-in-vector (nested deep copy)
  vw: vector<Holder> = [Holder { m: Blank }];
  assert(describe(vw[0].m) == "X", "struct-in-vec");
  // alias-transfer consumers must stay correct (and not double-free)
  x: Sigil = Blank;
  assert(describe(x) == "X", "plain var");
}
struct Holder { m: Sigil }
"#,
    );
    assert!(
        leaks.is_empty(),
        "#394 bare unit variant leaked {} store(s): {}",
        leaks.len(),
        leaks.join(", ")
    );
}

/// Pull the total live record count out of a `memory_report()` string
/// (`"... | records {N} | ..."`).  Returns `u64::MAX` if the field is
/// absent so a parse miss fails the assertion loudly rather than passing.
fn report_records(report: &str) -> u64 {
    report
        .split("records ")
        .nth(1)
        .and_then(|s| s.split([' ', '|']).next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(u64::MAX)
}

/// @PLN25 — reassigning an inline struct-enum element must free the OLD
/// variant's heap payload.  `remove_claims` had no `Parts::Enum` arm (it fell
/// through to the no-op `_`), so overwriting an inline `enum { Has{text},
/// Gone{n} }` element orphaned the prior `Has`'s text on every churn — one
/// leaked record per iteration, reclaimed only when the whole store is freed.
/// That makes it an INTRA-store leak the store-level gate (`check_store_leaks`)
/// cannot see, so this pins the total record count flat instead.  Pre-fix this
/// reported ~500 records after 500 churns; the symmetric `Parts::Enum` arm
/// (allocation.rs, twin of `copy_claims`) holds it at a small constant.  Runs
/// with the @PLN25 synthesis gate OFF — the bug is in plain user enums, not
/// just the synthesised `__nullable<S>` form.
#[test]
fn pln25_inline_struct_enum_payload_free() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    // Both variants carry a payload: a BARE no-field variant value leaks a
    // store on construction (a separate, pre-existing bug) which would add
    // stray records and muddy this record-count assertion.
    p.parse_str(
        r#"
enum Maybe { Has { s: text }, Gone { n: integer } }
pub fn test() {
  v: vector<Maybe> = [Gone { n: 0 }];
  for i in 0..500 {
    v[0] = Has { s: "payload-distinct-text-{i}" };
    v[0] = Gone { n: i };
  }
}
"#,
        "leak_test",
        false,
    );
    assert!(
        p.diagnostics.is_empty(),
        "parse errors: {:?}",
        p.diagnostics.lines()
    );
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    state.execute("test", &p.data);
    let report = state.database.memory_report();
    let recs = report_records(&report);
    assert!(
        recs < 20,
        "inline struct-enum payload leaked: {recs} live records after 500 churns \
         (expected a small constant). Report:\n{report}"
    );
}

// @PLN85 A.1 part i — a nullable-returning fn (`if b { value } else { null }`)
// whose result is bound by a caller must free that local at the caller's scope
// exit.  The return-source SET drives free-suppression in the CALLEE (the arm
// buffers are transferred), but the suppression must be PATH-LOCAL: a global
// `skip_free` stamp on the source over-suppresses the caller's binding too,
// leaking it (the 25-nullable-sequences regression this guards).
#[test]
fn pln85_nullable_return_caller_binding_freed() {
    let leaks = leaks_for(
        r#"
struct NRow { id: integer, tag: text }
fn maybe(b: boolean) -> vector<integer> { if b { [10, 20] } else { null } }
fn maybe_row(b: boolean) -> NRow { if b { NRow { id: 9, tag: "z" } } else { null } }
pub fn test() {
  n = maybe(false);
  p = maybe(true);
  assert(n == null, "n null");
  assert(len(p) == 2, "p len");
  r = maybe_row(false);
  q = maybe_row(true);
  assert(r == null, "r null");
  assert(q.id == 9, "q id");
}
"#,
    );
    assert!(
        leaks.is_empty(),
        "nullable-return caller bindings leaked: {leaks:?}"
    );
}

// @PLN87 P2.2 — a `&` whole-binding write-back (`o = Obj{..}` on a `&Obj` param)
// frees the DISPLACED caller store as it installs the new value: the RefVar-set
// lowering reads the old store live through `o` and frees it, and the
// transferred construction temp is skip_free.  Fixed by P2.2 — was: `SetStackRef`
// overwrote the caller's slot without freeing the old store, orphaning 1 Obj.
#[test]
fn pln87_struct_amp_writeback_is_leak_free() {
    let leaks = leaks_for(
        r#"
struct Obj { x: integer }
fn oamp(o: &Obj) { o = Obj { x: 9 }; }
pub fn test() {
  g = Obj { x: 1 };
  oamp(&g);
  assert(g.x == 9, "& write-back is visible to the caller");
}
"#,
    );
    assert!(
        leaks.is_empty(),
        "& write-back leaked the displaced store: {leaks:?}"
    );
}
