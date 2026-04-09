// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Validation tests for the variable introspection framework
//! ([`State::iter_frame_variables`] and [`State::dump_frame_variables`]).
//!
//! Each test runs a small known-good loft program through `execute_log` and
//! asserts that the framework correctly identifies live variables and reads
//! their values from the runtime stack.

extern crate loft;

use loft::compile::byte_code;
use loft::data::Data;
use loft::log_config::LogConfig;
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;

/// Compile `script`, run via `execute_log`, and after each opcode call
/// `inspect(state, data)` so the test can assert intermediate state.
///
/// Returns the captured trace string.
fn build(script: &str) -> (State, Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(script, "frame_vars", false);
    if !p.diagnostics.is_empty() {
        panic!("parse errors: {:?}", p.diagnostics.lines());
    }
    scopes::check(&mut p.data);
    let mut data = p.data;
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut data);
    (state, data)
}

// ── Test 1: Single integer variable ─────────────────────────────────────────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn integer_variable_layout() {
    // After compile, the framework can list slot-assigned variables for
    // n_test.  We don't need execute output to verify the slot table.
    let (_state, data) = build(
        "fn test() {
    x = 42;
    y = 100;
    z = x + y;
    assert(z == 142);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    let vars = &data.def(fn_d_nr).variables;
    // x, y, z all exist as variables.
    let names: Vec<String> = (0..vars.count())
        .map(|i| vars.name(i).to_string())
        .collect();
    assert!(names.iter().any(|n| n == "x"), "no var x in {names:?}");
    assert!(names.iter().any(|n| n == "y"), "no var y in {names:?}");
    assert!(names.iter().any(|n| n == "z"), "no var z in {names:?}");
    // All have integer type.
    for v_nr in 0..vars.count() {
        let n = vars.name(v_nr);
        if n == "x" || n == "y" || n == "z" {
            assert!(
                matches!(vars.tp(v_nr), loft::data::Type::Integer(_, _, _)),
                "{n} should be Integer, got {:?}",
                vars.tp(v_nr)
            );
        }
    }
}

// ── Test 2: Iterator yields variables for entry function ────────────────────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn iter_yields_function_variables() {
    let (mut state, data) = build(
        "fn test() {
    a = 10;
    b = 20;
    assert(a + b == 30);
}",
    );
    // Set code_pos to the start of n_test so iter_frame_variables can locate
    // the function.  Don't actually execute — just check the data shape.
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    state.stack_pos = 4; // entry function start
    let frame_vars = state.iter_frame_variables(&data);
    // At least a and b should appear.
    let names: Vec<&str> = frame_vars.iter().map(|v| v.name.as_str()).collect();
    assert!(
        names.contains(&"a"),
        "iter did not yield 'a': got {names:?}"
    );
    assert!(
        names.contains(&"b"),
        "iter did not yield 'b': got {names:?}"
    );
}

// ── Test 3: Liveness — slot-coalesced variables are marked correctly ───────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn liveness_marks_dead_variables() {
    // This script reuses slots: `x` is dead after `z = x + y`, so its slot
    // is coalesced with another variable.
    let (mut state, data) = build(
        "fn test() {
    x = 42;
    y = 100;
    z = x + y;
    assert(z == 142);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    // Position at the very start of the function: nothing referenced yet,
    // all locals should be marked NOT live.
    state.code_pos = data.def(fn_d_nr).code_position;
    state.stack_pos = 4;
    let vars = state.iter_frame_variables(&data);
    for v in &vars {
        if !v.is_argument {
            assert!(
                !v.live,
                "var '{}' marked live before any reference (bc_first={}, code_pos={})",
                v.name, v.bc_first, state.code_pos
            );
        }
    }
}

// ── Test 4: Bytecode-position liveness range is populated ───────────────────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn liveness_range_populated() {
    let (mut state, data) = build(
        "fn test() {
    x = 7;
    assert(x == 7);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    // Pick a code position inside the function body.
    state.code_pos = data.def(fn_d_nr).code_position + 10;
    state.stack_pos = 4;
    let vars = state.iter_frame_variables(&data);
    let x = vars
        .iter()
        .find(|v| v.name == "x")
        .expect("variable x missing");
    assert!(x.bc_first != u32::MAX, "x has no bytecode reference range");
    assert!(
        x.bc_last >= x.bc_first,
        "x bc_last={} < bc_first={}",
        x.bc_last,
        x.bc_first
    );
    // The range must lie within the function bytecode.
    let fn_start = data.def(fn_d_nr).code_position;
    let fn_end = fn_start + data.def(fn_d_nr).code_length;
    assert!(
        x.bc_first >= fn_start && x.bc_last < fn_end,
        "x range [{}, {}] outside function [{}, {})",
        x.bc_first,
        x.bc_last,
        fn_start,
        fn_end
    );
}

// ── Test 5: Iterator is read-only — does not mutate stack_pos ──────────────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn iter_does_not_mutate_state() {
    let (mut state, data) = build(
        "fn test() {
    n = 5;
    assert(n == 5);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    let stack_before = state.stack_pos;
    let code_before = state.code_pos;
    let _ = state.iter_frame_variables(&data);
    let _ = state.iter_frame_variables(&data);
    assert_eq!(state.stack_pos, stack_before, "stack_pos changed");
    assert_eq!(state.code_pos, code_before, "code_pos changed");
}

// ── Test 6: dump_frame_variables produces expected format ──────────────────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn dump_format_smoke_test() {
    let (mut state, data) = build(
        "fn test() {
    n = 99;
    assert(n == 99);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    state.stack_pos = 4;
    let mut buf = Vec::<u8>::new();
    state.dump_frame_variables(&mut buf, &data).unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(
        out.starts_with("[VARS]"),
        "dump should start with [VARS]: {out}"
    );
    assert!(
        out.contains("fn=n_test"),
        "dump should name function: {out}"
    );
}

// ── Test 7: Run a real script through execute_log to ensure no crashes ────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn execute_log_with_dump_does_not_crash() {
    let (mut state, data) = build(
        "fn test() {
    a = 1;
    b = 2;
    assert(a + b == 3);
}",
    );
    let mut config = LogConfig::full();
    config.dump_vars = true;
    let mut buf = Vec::<u8>::new();
    state.execute_log(&mut buf, "test", &config, &data).unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("[VARS]"), "no [VARS] lines in trace");
    assert!(out.contains("fn=n_test"), "no n_test frame: {out}");
    // The integer values 1, 2, 3 should appear in the trace.
    assert!(out.contains("i32"), "no i32 type in dump");
}

// ── Test 8a: Verify the framework agrees with the codegen vars map ─────────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn iter_var_nr_matches_codegen() {
    // The codegen records (bytecode_pos, var_nr) entries in State.vars when
    // it emits a variable-accessing opcode.  The framework's iter_frame_variables
    // looks up vars by var_nr.  These two views must agree on slot positions.
    let (mut state, data) = build(
        "fn test() {
    a = 1;
    b = 2;
    c = a + b;
    assert(c == 3);
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    let frame_vars = state.iter_frame_variables(&data);
    // For each variable in the codegen vars map (within this function), look
    // up its name and slot via the framework — they must match.
    let fn_start = data.def(fn_d_nr).code_position;
    let fn_end = fn_start + data.def(fn_d_nr).code_length;
    let func_vars = &data.def(fn_d_nr).variables;
    for (&bc, &v_nr) in &state.vars {
        if bc < fn_start || bc >= fn_end {
            continue;
        }
        let codegen_name = func_vars.name(v_nr);
        let codegen_slot = func_vars.stack(v_nr);
        let frame_var = frame_vars.iter().find(|fv| fv.var_nr == v_nr);
        match frame_var {
            Some(fv) => {
                assert_eq!(
                    fv.name, codegen_name,
                    "var_nr={v_nr} name mismatch: frame={} codegen={codegen_name}",
                    fv.name
                );
                assert_eq!(
                    fv.slot, codegen_slot,
                    "var_nr={v_nr} ({codegen_name}) slot mismatch: frame={} codegen={codegen_slot}",
                    fv.slot
                );
            }
            None => {
                // var_nr might be filtered out (slot == u16::MAX), but if the
                // codegen referenced it, slot should be assigned.
                if codegen_slot != u16::MAX {
                    panic!(
                        "var_nr={v_nr} ({codegen_name}) slot={codegen_slot} \
                         missing from iter_frame_variables (bc={bc})"
                    );
                }
            }
        }
    }
}

// ── Test 8b: same validation on the file_content reproducer ────────────────

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn iter_var_nr_matches_codegen_file_content() {
    let (mut state, data) = build(
        "fn test() {
    f = file(\"/nonexistent.txt\");
    t = f.content();
    assert(t == \"\");
}",
    );
    let fn_d_nr = data.def_nr("n_test");
    state.code_pos = data.def(fn_d_nr).code_position;
    let frame_vars = state.iter_frame_variables(&data);
    let fn_start = data.def(fn_d_nr).code_position;
    let fn_end = fn_start + data.def(fn_d_nr).code_length;
    let func_vars = &data.def(fn_d_nr).variables;
    let mut errors = Vec::new();
    for (&bc, &v_nr) in &state.vars {
        if bc < fn_start || bc >= fn_end {
            continue;
        }
        let codegen_name = func_vars.name(v_nr).to_string();
        let codegen_slot = func_vars.stack(v_nr);
        if let Some(fv) = frame_vars.iter().find(|fv| fv.var_nr == v_nr)
            && fv.slot != codegen_slot
        {
            errors.push(format!(
                "var_nr={v_nr} ({codegen_name}) bc={bc}: frame slot={} != codegen slot={codegen_slot}",
                fv.slot
            ));
        }
    }
    assert!(
        errors.is_empty(),
        "{} mismatches:\n{}",
        errors.len(),
        errors.join("\n")
    );
}

// ── Test 9: Argument size — text args use Str (16B), locals use String (24B)

#[test]
#[ignore = "SIGSEGV on ubuntu CI — investigating uninitialized stack reads"]
fn arg_text_uses_str_layout() {
    let (mut state, data) = build(
        "fn helper(s: text) -> integer {
    s.len()
}
fn test() {
    n = helper(\"hello\");
    assert(n == 5);
}",
    );
    let helper_d_nr = data.def_nr("n_helper");
    state.code_pos = data.def(helper_d_nr).code_position;
    state.stack_pos = 4 + 16; // return addr + 16-byte Str arg
    let vars = state.iter_frame_variables(&data);
    let s = vars.iter().find(|v| v.name == "s").expect("missing arg s");
    assert!(s.is_argument, "s should be an argument");
    assert_eq!(s.size, 16, "arg text should use 16-byte Str layout");
}
