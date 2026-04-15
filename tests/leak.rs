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

/// P146 reproducer: `var = user_fn(arg)` where `user_fn` returns its
/// param (`fn ac_identity(p: AcPoint) -> AcPoint { p }`) leaks the
/// result variable's store at function exit.
///
/// Diagnosis via `LOFT_LOG=alloc_free` (un-ignore to run):
/// - The runtime takes the P143 lock-args + OpCopyRecord deep-copy
///   path, so `ac_copy` becomes an INDEPENDENT store.
/// - Scope analysis at parse time treats `ac_identity(ac_orig)` as
///   "returns alias of arg 0", so the result variable is not given an
///   `OpFreeRef` at scope exit (the parser thinks the borrow is freed
///   transitively via `ac_orig`).
/// - At function exit only `ac_orig` is freed; `ac_copy`'s store
///   leaks.
///
/// Same family as P143 / dep-inference.  Fix lives in `src/scopes.rs`:
/// when a user-defined fn returns its param AND the call site goes
/// through the deep-copy path (because the LHS variable already
/// exists), the LHS needs independent scope cleanup.
///
/// Affects 5 wrap-suite scripts: 45, 62, 76, 81, 95.  Closing this
/// (or its dep-inference cousin) likely closes most of the zero-leak
/// gate at once.  Un-ignore to get a sharp `Database N not correctly
/// freed` panic with the exact alloc op + pc.
#[test]
#[ignore = "P146 open — alias-return + deep-copy leak; un-ignore for sharp diagnostic"]
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
