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
