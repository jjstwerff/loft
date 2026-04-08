// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Store leak regression tests.
//!
//! Runs loft scripts and asserts no stores are leaked at program exit.

extern crate loft;

use loft::compile::{byte_code, show_code};
use loft::data::DefType;
use loft::log_config::LogConfig;
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
    let mut config = LogConfig::from_env();
    config.phases.ir = true;
    config.show_variables = true;
    let mut w = std::io::stderr();
    let _ = show_code(&mut w, &mut state, &mut p.data, &config);

    state.execute("test", &p.data);
    state.check_store_leaks();
}

/// Run all zero-parameter test functions in a script and check for store leaks.
fn run_leak_check(path: &str) {
    let mut p = Parser::new();
    let (data, db) = cached_default();
    p.data = data;
    p.database = db;
    let start_def = p.data.definitions();
    p.parse(path, false);
    if p.diagnostics.level() >= loft::diagnostics::Level::Error {
        panic!(
            "{path}: parse errors: {:?}",
            p.diagnostics.lines()
        );
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
        if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
            continue;
        }
        if !def.attributes.is_empty() {
            continue;
        }
        if def.position.file.starts_with("default/") {
            continue;
        }
        let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
        state.execute(user_name, &p.data);
        state.check_store_leaks_context(&format!("{path}::{user_name}"));
        ran += 1;
    }
    assert!(ran > 0, "{path}: no entry-point functions found");
}

#[test]
fn block_copy_opt_no_leak() {
    run_leak_check("tests/scripts/33-block-copy-opt.loft");
}

#[test]
fn field_iter_no_leak() {
    run_leak_check("tests/scripts/45-field-iter.loft");
}

#[test]
fn index_range_no_leak() {
    run_leak_check("tests/scripts/62-index-range-queries.loft");
}

#[test]
fn alias_copy_no_leak() {
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
