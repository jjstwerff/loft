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
use loft::data::DefType;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;
mod common;
use common::cached_default;

/// Run inline loft code (must define `fn test()`) and check for store leaks.
#[allow(dead_code)]
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

/// Run all test_* / main functions in a script, check leaks per function.
#[allow(dead_code)]
fn run_leak_check(path: &str) {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    let start_def = p.data.definitions();
    p.parse(path, false);
    if p.diagnostics.level() >= loft::diagnostics::Level::Error {
        panic!("{path}: parse errors: {:?}", p.diagnostics.lines());
    }
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let end_def = p.data.definitions();
    let mut ran = 0;
    for d_nr in start_def..end_def {
        let def = p.data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) {
            continue;
        }
        if !def.name.starts_with("n_test_") && def.name != "n_main" {
            continue;
        }
        if !def.attributes.is_empty() || def.position.file.starts_with("default/") {
            continue;
        }
        let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
        state.execute(user_name, &p.data);
        state.check_store_leaks_context(&format!("{path}::{user_name}"));
        ran += 1;
    }
    assert!(ran > 0, "{path}: no entry-point functions found");
}

// ── Active leak investigations go below ─────────────────────────────

#[test]
fn reassign_struct_in_loop() {
    run_leak_check("tests/scripts/83-reassign-struct-loop.loft");
}

/// Breakout pattern: struct with vector field, reassigned in nested loop
/// while other vectors are read in the same loop.
/// Verifies both leak-free AND data integrity after each reassignment.
#[test]
fn breakout_pattern() {
    run_leak_check_str(
        r#"
struct Mat4 { m: vector<float> }
fn make_mat4(a: float, b: float) -> Mat4 {
  Mat4 { m: [a, 0.0, 0.0, 0.0, 0.0, b, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] }
}
fn mat4_mul(ma: const Mat4, mb: const Mat4) -> Mat4 {
  make_mat4(ma.m[0] * mb.m[0], ma.m[5] * mb.m[5])
}
fn rect_mvp(proj: const Mat4, bx: float, by: float) -> Mat4 {
  model = make_mat4(bx, by);
  mat4_mul(proj, model)
}
pub fn test() {
  bricks = [for _ in 0..12 { 1 }];
  proj = make_mat4(1.0, 1.0);
  mvp = make_mat4(0.0, 0.0);
  drawn = 0;
  for row in 0..3 {
    for col in 0..4 {
      assert(bricks[row * 4 + col] == 1, "brick[{row},{col}] should be 1");
      mvp = rect_mvp(proj, col as float, row as float);
      assert(len(mvp.m) == 16, "mvp.m should have 16 elements, got {len(mvp.m)}");
      assert(mvp.m[0] == col as float, "mvp.m[0] should be {col}, got {mvp.m[0]}");
      assert(mvp.m[5] == row as float, "mvp.m[5] should be {row}, got {mvp.m[5]}");
      drawn += 1;
    }
  }
  assert(drawn == 12, "should draw 12 bricks, drew {drawn}");
  // Verify proj wasn't corrupted by the loop
  assert(proj.m[0] == 1.0, "proj.m[0] intact: {proj.m[0]}");
  assert(len(proj.m) == 16, "proj.m len intact: {len(proj.m)}");
}
"#,
    );
}

/// Simulate multiple frame iterations — run the brick loop twice,
/// verify data integrity is preserved between iterations.
#[test]
fn breakout_two_frames() {
    run_leak_check_str(
        r#"
struct Mat4 { m: vector<float> }
fn make_mat4(a: float, b: float) -> Mat4 {
  Mat4 { m: [a, 0.0, 0.0, 0.0, 0.0, b, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] }
}
fn mat4_mul(ma: const Mat4, mb: const Mat4) -> Mat4 {
  make_mat4(ma.m[0] * mb.m[0], ma.m[5] * mb.m[5])
}
fn rect_mvp(proj: const Mat4, bx: float, by: float) -> Mat4 {
  model = make_mat4(bx, by);
  mat4_mul(proj, model)
}
pub fn test() {
  bricks = [for _ in 0..12 { 1 }];
  proj = make_mat4(1.0, 1.0);
  mvp = make_mat4(0.0, 0.0);
  // "Frame 1"
  for row in 0..3 {
    for col in 0..4 {
      assert(bricks[row * 4 + col] == 1, "f1 brick[{row},{col}]");
      mvp = rect_mvp(proj, col as float, row as float);
      assert(mvp.m[0] == col as float, "f1 mvp col={col}");
    }
  }
  // "Frame 2" — same loop again, verify no corruption
  for row2 in 0..3 {
    for col2 in 0..4 {
      assert(bricks[row2 * 4 + col2] == 1, "f2 brick[{row2},{col2}]");
      mvp = rect_mvp(proj, col2 as float, row2 as float);
      assert(mvp.m[0] == col2 as float, "f2 mvp col={col2}");
    }
  }
  assert(proj.m[0] == 1.0, "proj intact");
}
"#,
    );
}

/// Breakout-style: vector + struct reassignment in nested loops.
#[test]
fn reassign_with_vector() {
    run_leak_check_str(
        r#"
struct Pt { x: float not null, y: float not null }
fn make_pt(a: float, b: float) -> Pt { Pt { x: a, y: b } }
pub fn test() {
  bricks = [for _ in 0..10 { 1 }];
  p = make_pt(0.0, 0.0);
  for i in 0..3 {
    for j in 0..4 {
      if bricks[j] == 1 {
        p = make_pt(j as float, i as float);
      }
    }
  }
  assert(p.x == 3.0, "x: {p.x}");
}
"#,
    );
}
