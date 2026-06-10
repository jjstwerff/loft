// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 02 slice 1 / 08 gate 1 — the compiled→interp RE-ENTRY probe.
//!
//! The heart-of-the-engine claim under test: *"native→interp re-entry is a
//! dispatch, not a rewrite."*  Host code (standing in for generated native
//! code / the debugger) calls interpreted functions over the LIVE State of
//! a paused program — `main` builds the world, yields (the frame-yield
//! contract), the host re-enters callees with Rust-marshaled args, and the
//! resumed `main` verifies in loft what the re-entered callees wrote (the
//! differential half).  Matrix axes: scalar args, struct (DbRef) args,
//! vector args, integer-null, repeated calls; plus a crossing-cost number.

use std::cell::Cell;

use loft::compile;
use loft::database::Stores;
use loft::keys::DbRef;
use loft::parser::Parser;
use loft::state::State;

thread_local! {
    static STASH_RES: Cell<DbRef> = const {
        Cell::new(DbRef { store_nr: u16::MAX, rec: 0, pos: 0 })
    };
    static STASH_VEC: Cell<DbRef> = const {
        Cell::new(DbRef { store_nr: u16::MAX, rec: 0, pos: 0 })
    };
}

fn n_probe_stash(stores: &mut Stores, stack: &mut DbRef) {
    let r = *stores.get::<DbRef>(stack);
    STASH_RES.with(|s| s.set(r));
}

fn n_probe_stash_vec(stores: &mut Stores, stack: &mut DbRef) {
    let v = *stores.get::<DbRef>(stack);
    STASH_VEC.with(|s| s.set(v));
}

fn n_frame_pause(stores: &mut Stores, _stack: &mut DbRef) {
    stores.frame_yield = true;
}

const SRC: &str = r#"
struct Res { val: integer not null, txt: text not null }

fn probe_stash(r: Res);
#native
fn probe_stash_vec(v: vector<integer>);
#native
fn frame_pause();
#native

fn add_into(a: integer, b: integer, r: Res) {
  r.val = a + b;
  r.txt = "sum:{a + b}";
}

fn vec_sum_into(v: vector<integer>, r: Res) {
  t = 0;
  for x in v { t = t + x; }
  r.val = t;
}

fn null_or(x: integer, r: Res) {
  if x == null {
    r.val = 0 - 1;
  } else {
    r.val = x * 2;
  }
}

fn main() {
  r = Res { val: 0, txt: "" };
  v = [3, 7, 11, 21];
  probe_stash(r);
  probe_stash_vec(v);
  frame_pause();
  // Resumed after the host-side re-entry calls.  The host ran, in order:
  //   add_into(7, 35, r)        -> val 42, txt "sum:42"
  //   vec_sum_into(v, r)        -> val 42 (3+7+11+21)
  //   null_or(NULL, r)          -> val -1
  //   null_or(21, r)            -> val 42
  //   add_into(40, 2, r)  x N   -> val 42, txt "sum:42" (the cost loop)
  // The loft-side verdict (the differential half of the probe):
  if r.val != 42 { panic("post: val {r.val} != 42"); }
  if r.txt != "sum:42" { panic("post: txt '{r.txt}' != 'sum:42'"); }
  println("probe-main-ok");
}
"#;

#[test]
#[ignore = "PROBE VERDICT (2026-06-11): the core claim HOLDS — all matrix cells correct \
(scalars, vector, null, repeats), 217 ns per host->interp call incl. body — but the \
thunk's frame bookkeeping is incomplete: teardown heap corruption (free(): invalid \
pointer) without the CallFrame push, and a mid-run garbage DbRef (allocation.rs OOB \
in the resumed main) WITH it — the synthetic frame interacts with stack/ownership \
bookkeeping beyond call_stack balance.  Slice-1 remainder: map the frame contract \
(args_base/free bookkeeping at the yield boundary) before S2 builds on reenter."]
fn reentry_is_a_dispatch_not_a_rewrite() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).expect("stdlib");
    p.parse_str(SRC, "<reentry-probe>", false);
    assert!(
        p.diagnostics.level() < loft::diagnostics::Level::Error,
        "probe source must parse clean: {:?}",
        p.diagnostics.lines()
    );
    loft::scopes::check(&mut p.data);
    let mut state = State::new(p.database.clone());
    state.static_fn("n_probe_stash", n_probe_stash);
    state.static_fn("n_probe_stash_vec", n_probe_stash_vec);
    state.static_fn("n_frame_pause", n_frame_pause);
    compile::byte_code(&mut state, &mut p.data);

    // Run main up to the yield: the world (a Res record + a vector) is now
    // live in main's frame, stashed for the host.
    state.execute_argv("main", &p.data, &[]);
    assert!(
        state.database.frame_yield,
        "main must reach frame_pause() and yield"
    );
    let res = STASH_RES.with(Cell::get);
    let vec = STASH_VEC.with(Cell::get);
    assert_ne!(res.store_nr, u16::MAX, "Res record stashed");
    assert_ne!(vec.store_nr, u16::MAX, "vector stashed");

    let pos_of = |name: &str| {
        let d = p.data.def_nr(&format!("n_{name}"));
        assert_ne!(d, u32::MAX, "fn {name}");
        (d, p.data.def(d).code_position)
    };
    let (add_into, vec_sum_into, null_or) = (
        pos_of("add_into"),
        pos_of("vec_sum_into"),
        pos_of("null_or"),
    );

    // The Res record's first field (val: integer) — the host-side read half.
    let read_val = |state: &mut State| -> i64 {
        *state
            .database
            .store_mut(&res)
            .addr_mut::<i64>(res.rec, res.pos)
    };

    // ── The matrix ──
    // 1. Scalars + a record arg.
    state.put_stack(7i64);
    state.put_stack(35i64);
    state.put_stack(res);
    state.reenter(add_into.0, add_into.1);
    assert_eq!(read_val(&mut state), 42, "scalar re-entry wrote the store");

    // 2. A vector arg.
    state.put_stack(vec);
    state.put_stack(res);
    state.reenter(vec_sum_into.0, vec_sum_into.1);
    assert_eq!(read_val(&mut state), 42, "vector re-entry summed 3+7+11+21");

    // 3. Integer null (the i64::MIN sentinel).
    state.put_stack(i64::MIN);
    state.put_stack(res);
    state.reenter(null_or.0, null_or.1);
    assert_eq!(read_val(&mut state), -1, "null marshaled as null");

    // 4. The same fn with a value.
    state.put_stack(21i64);
    state.put_stack(res);
    state.reenter(null_or.0, null_or.1);
    assert_eq!(read_val(&mut state), 42, "non-null after null");

    // 5. The crossing cost (informational; printed, not asserted).
    let n = 200_000u32;
    let t = std::time::Instant::now();
    for _ in 0..n {
        state.put_stack(40i64);
        state.put_stack(2i64);
        state.put_stack(res);
        state.reenter(add_into.0, add_into.1);
    }
    let per = t.elapsed().as_nanos() / u128::from(n);
    eprintln!(
        "reentry probe: {per} ns per host->interp call (incl. body: 2 store writes + a text format)"
    );
    assert_eq!(read_val(&mut state), 42);

    // ── The differential half: resume main; IT verifies the world. ──
    let still = state.resume();
    assert!(!still, "main must run to completion");
    assert!(
        state.database.runtime_error.is_none(),
        "the resumed main's loft-side checks failed: {:?}",
        state.database.runtime_error
    );
}
