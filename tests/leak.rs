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

/// Replicate the WASM breakout store 14 use-after-free.
/// proj is allocated once, its store gets recycled for temporaries
/// in sub-loops, then proj.m is read after the temporary is freed.
#[test]
fn wasm_store_recycling_bug() {
    run_leak_check_str(
        r#"
struct Mat4 { m: vector<float> }
fn make_mat4(a: float, b: float) -> Mat4 {
  Mat4 { m: [a, 0.0, 0.0, 0.0, 0.0, b, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] }
}
fn mat4_mul(ma: const Mat4, mb: const Mat4) -> Mat4 {
  make_mat4(ma.m[0] * mb.m[0], ma.m[5] * mb.m[5])
}
fn ortho() -> Mat4 {
  Mat4 { m: [2.0, 0.0, 0.0, 0.0, 0.0, -2.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, -1.0, 1.0, 0.0, 1.0] }
}
fn rect_mvp(proj: const Mat4, bx: float, by: float, bw: float, bh: float) -> Mat4 {
  model = Mat4 { m: [bw, 0.0, 0.0, 0.0, 0.0, bh, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, bx, by, 0.0, 1.0] };
  mat4_mul(proj, model)
}
pub fn test() {
  proj = ortho();
  bricks = [for _ in 0..50 { 1 }];
  colors_r = [0.9, 0.9, 0.2, 0.2, 0.3];
  lives = 3;
  mvp = make_mat4(0.0, 0.0);
  // Simulate 1000 frames
  for frame in 0..1000 {
    // Draw bricks (50 iterations)
    for dr in 0..5 {
      for dc in 0..10 {
        if bricks[dr * 10 + dc] == 1 {
          mvp = rect_mvp(proj, dc as float * 72.0, dr as float * 20.0, 72.0, 20.0);
          assert(len(mvp.m) == 16, "f{frame} brick mvp");
        }
      }
    }
    // Draw paddle
    mvp = rect_mvp(proj, 350.0, 560.0, 100.0, 15.0);
    assert(len(mvp.m) == 16, "f{frame} paddle mvp");
    // Draw ball
    mvp = rect_mvp(proj, 395.0, 530.0, 10.0, 10.0);
    assert(len(mvp.m) == 16, "f{frame} ball mvp");
    // Draw lives
    for li in 0..lives {
      mvp = rect_mvp(proj, 780.0 - li as float * 18.0, 10.0, 12.0, 12.0);
      assert(len(mvp.m) == 16, "f{frame} life {li}");
    }
    // Verify proj intact after all draws
    assert(proj.m[0] == 2.0, "f{frame} proj.m[0]={proj.m[0]}");
    assert(proj.m[5] == -2.0, "f{frame} proj.m[5]={proj.m[5]}");
  }
}
"#,
    );
}

/// Reproduce the WASM breakout use-after-free locally.
/// Same structure as breakout but uses yield_test library instead of GL.
#[test]
fn breakout_yield_resume() {
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

    fn mock_yield(stores: &mut loft::database::Stores, _stack: &mut loft::keys::DbRef) {
        stores.frame_yield = true;
    }
    state.replace_native("mock_yield_frame", mock_yield);

    // First execute — runs until first yield
    state.execute("main", &p.data);
    assert!(state.database.frame_yield, "should have yielded");

    // Resume for 100 frames — enough to trigger store recycling
    for _frame in 0..1000 {
        let running = state.resume();
        if !running {
            break;
        }
    }
}

/// Dump bytecode of breakout to find FreeRef positions
#[test]
fn dump_breakout_bytecode() {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    p.lib_dirs.push("lib".to_string());
    p.lib_dirs.push("lib/graphics/src".to_string());
    p.parse("lib/graphics/examples/25-breakout.loft", false);
    scopes::check(&mut p.data);
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut p.data);
    let mut config = loft::log_config::LogConfig::from_env();
    config.phases.bytecode = true;
    config.phases.ir = false;
    config.show_all_functions = false;
    config.show_functions = Some(vec!["n_main".to_string()]);
    for l in p.diagnostics.lines() { eprintln!("DIAG: {l}"); }
    loft::compile::show_code(&mut std::io::stderr(), &mut state, &mut p.data, &config).ok();
    use loft::data::DefType;
    for d_nr in 0..p.data.definitions() {
        let def = p.data.def(d_nr);
        if !matches!(def.def_type, DefType::Function) { continue; }
        if def.position.file.starts_with("default/") { continue; }
        let cp = def.code_position;
        eprintln!("  fn {} at bc={cp} file={}", def.name, def.position.file);
    }
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

/// Exact breakout pattern: proj (const Mat4 with vector m) lives across
/// the entire loop while mvp is reassigned ~50 times per frame.
/// Tests store recycling: freed mvp stores must not collide with proj.m.
#[test]
fn breakout_store_recycling() {
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
  bricks = [for _ in 0..50 { 1 }];
  proj = make_mat4(2.0, 3.0);
  mvp = make_mat4(0.0, 0.0);
  // Simulate 100 frames, each drawing all 50 bricks + paddle + ball
  // gl_set_uniform_mat4 reads mvp.m (a vector) after each reassignment
  total = 0.0;
  for frame in 0..100 {
    for bi in 0..50 {
      if bricks[bi] == 1 {
        mvp = rect_mvp(proj, bi as float, frame as float);
        // simulate gl_set_uniform_mat4 reading mvp.m
        for mvi in 0..16 { total += mvp.m[mvi]; }
      }
    }
    // paddle
    mvp = rect_mvp(proj, 4.0, 5.0);
    for mvi in 0..16 { total += mvp.m[mvi]; }
    // ball
    mvp = rect_mvp(proj, 6.0, 7.0);
    for mvi in 0..16 { total += mvp.m[mvi]; }
    // Verify proj is intact after all those reassignments
    assert(proj.m[0] == 2.0, "f{frame} proj.m[0]={proj.m[0]}");
    assert(proj.m[5] == 3.0, "f{frame} proj.m[5]={proj.m[5]}");
    assert(len(proj.m) == 16, "f{frame} proj.m len");
  }
  assert(total > 0.0, "total");
}
"#,
    );
}

/// Simulate the breakout frame loop: outer loop with struct reassignment
/// inside nested loops, reading vectors in the same scope.
/// This catches use-after-free when freed stores get recycled.
#[test]
fn breakout_frame_loop() {
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
  colors = [0.9, 0.2, 0.3];
  // Simulate 3 frames
  for frame in 0..3 {
    // Draw bricks (nested loop with vector read + struct reassign)
    for row in 0..3 {
      for col in 0..4 {
        if bricks[row * 4 + col] == 1 {
          mvp = rect_mvp(proj, col as float, row as float);
          assert(len(mvp.m) == 16, "f{frame} mvp.m len");
          assert(mvp.m[0] == col as float, "f{frame} mvp col={col}");
        }
      }
    }
    // Draw paddle
    mvp = rect_mvp(proj, 4.0, 5.0);
    assert(mvp.m[0] == 4.0, "f{frame} paddle");
    // Draw ball
    mvp = rect_mvp(proj, 6.0, 7.0);
    assert(mvp.m[0] == 6.0, "f{frame} ball");
    // Draw lives (loop with struct reassign)
    for li in 0..3 {
      mvp = rect_mvp(proj, 7.0 - li as float, 0.0);
      assert(mvp.m[0] == 7.0 - li as float, "f{frame} life {li}");
    }
    // Verify vectors are intact
    assert(bricks[0] == 1, "f{frame} bricks[0]");
    assert(colors[0] == 0.9, "f{frame} colors[0]");
    assert(proj.m[0] == 1.0, "f{frame} proj intact");
  }
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
