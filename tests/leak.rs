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
#[ignore = "P147 — investigating, un-ignore for sharp diagnostic"]
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
#[ignore = "P148 — investigating, un-ignore for sharp diagnostic"]
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
#[ignore = "P149 open — slot reuse SEGV under execute_log; un-ignore for sharp diagnostic"]
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
