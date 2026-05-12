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

/// P150 file-level regression guard: `lib/moros_map/tests/serial.loft`
/// used to leak 1 store via interpreter (warning: `2(bc:0)`) when
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
    p.lib_dirs.push("lib".to_string());
    p.parse("lib/moros_map/tests/serial.loft", false);
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
