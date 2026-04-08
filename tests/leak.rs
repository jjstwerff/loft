// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Store leak regression tests.
//!
//! Quick-to-run minimal reproductions of fixed store leaks.
//! Use `run_leak_check_str` with inline loft code and add
//! `show_code` / `LogConfig` for bytecode dumps when debugging.

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
    // Uncomment for bytecode dump when debugging a new leak:
    // let mut config = loft::log_config::LogConfig::from_env();
    // config.phases.ir = true;
    // loft::compile::show_code(&mut std::io::stderr(), &mut state, &mut p.data, &config).ok();
    state.execute("test", &p.data);
    state.check_store_leaks();
}

/// #120: struct field initialized from function call — return store orphaned.
#[test]
fn field_init_from_call() {
    run_leak_check_str(
        r#"
struct Pt { x: float not null, y: float not null }
struct Box { pos: Pt }
fn make_pt(a: float, b: float) -> Pt { Pt { x: a, y: b } }
pub fn test() {
  bx = Box { pos: make_pt(1.0, 2.0) };
  assert(bx.pos.x == 1.0, "x");
}
"#,
    );
}

/// Deep-copied struct return with dep on argument — dep prevented FreeRef.
#[test]
fn deep_copy_clears_dep() {
    run_leak_check_str(
        r#"
struct Pt { x: float not null, y: float not null }
fn identity(p: Pt) -> Pt { p }
pub fn test() {
  a = Pt { x: 1.0, y: 2.0 };
  b = identity(a);
  assert(b.x == 1.0, "x");
}
"#,
    );
}

/// #fields iteration — work_ref stores not freed across unrolled iterations.
#[test]
fn field_iter() {
    run_leak_check_str(
        r#"
struct Pt { x: float not null, y: float not null }
pub fn test() {
  p = Pt { x: 1.0, y: 2.0 };
  n = "";
  for pf in p#fields {
    n += pf.name;
  }
  assert(n == "xy", "names: {n}");
}
"#,
    );
}

/// Struct return in a loop — store must be freed/reused each iteration.
#[test]
fn struct_return_in_loop() {
    run_leak_check_str(
        r#"
struct Pt { x: float not null, y: float not null }
fn make_pt(a: float, b: float) -> Pt { Pt { x: a, y: b } }
fn shift_pt(p: const Pt, dx: float) -> Pt { make_pt(p.x + dx, p.y) }
pub fn test() {
  base = make_pt(0.0, 0.0);
  for i in 0..3 {
    p = shift_pt(base, i as float);
    assert(p.x >= 0.0, "x");
  }
}
"#,
    );
}

/// Custom iterator protocol — iter object deep-copied but dep prevented FreeRef.
#[test]
fn iterator_protocol() {
    run_leak_check_str(
        r#"
struct Counter { current: integer, limit: integer }
fn next(self: Counter) -> integer {
  val = self.current;
  self.current = val + 1;
  if val >= self.limit { return null; }
  val
}
pub fn test() {
  c = Counter { current: 0, limit: 2 };
  total = 0;
  for x in c {
    total += x;
  }
  assert(total == 1, "sum: {total}");
}
"#,
    );
}
