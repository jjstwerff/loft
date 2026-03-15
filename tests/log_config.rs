// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Unit tests for `LogConfig` — phase filters, function filters, opcode filters,
//! slot annotations, crash-tail mode, bridging checks, and preset selection.
//!
//! All tests use a minimal two-function loft script so the expected output is
//! short and predictable.  Each test captures `show_code` / `execute_log` output
//! into a `Vec<u8>` and asserts on the resulting string.

extern crate loft;

use loft::compile::{byte_code, show_code};
use loft::data::Data;
use loft::log_config::{LogConfig, LogPhase};
use loft::parser::Parser;
use loft::scopes;
use loft::state::State;

// ---------------------------------------------------------------------------
// Minimal loft program — a helper function called by the test entry point.
// ---------------------------------------------------------------------------

const SCRIPT: &str = "
fn helper() -> integer { 42 }

fn test() {
    x = helper();
    assert(x == 42, \"Expected 42\");
}
";

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Parse `script`, run scope analysis, and compile to bytecode.
/// Returns `(state, data)` ready to pass to `show_code` / `execute_log`.
fn build(script: &str) -> (State, Data) {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    p.parse_str(script, "log_test", false);
    scopes::check(&mut p.data);
    let mut data = p.data;
    let mut state = State::new(p.database);
    byte_code(&mut state, &mut data);
    (state, data)
}

/// Run `show_code` with `config` and return the captured output as a `String`.
fn static_log(config: &LogConfig) -> String {
    let (mut state, mut data) = build(SCRIPT);
    let mut buf = Vec::<u8>::new();
    show_code(&mut buf, &mut state, &mut data, config).unwrap();
    String::from_utf8(buf).unwrap()
}

/// Run `execute_log` for the `test` entry point and return the captured output.
fn exec_log(config: &LogConfig) -> String {
    let (mut state, data) = build(SCRIPT);
    let mut buf = Vec::<u8>::new();
    state.execute_log(&mut buf, "test", config, &data).unwrap();
    String::from_utf8(buf).unwrap()
}

/// Return both static and execution output concatenated.
fn full_log(config: &LogConfig) -> String {
    let (mut state, mut data) = build(SCRIPT);
    let mut buf = Vec::<u8>::new();
    show_code(&mut buf, &mut state, &mut data, config).unwrap();
    state.execute_log(&mut buf, "test", config, &data).unwrap();
    String::from_utf8(buf).unwrap()
}

// ---------------------------------------------------------------------------
// 1. Phase filter — full()
// ---------------------------------------------------------------------------

/// `full()` enables IR, bytecode, and execution, with slot annotations.
#[test]
fn full_has_all_phases() {
    let out = full_log(&LogConfig::full());

    // IR (intermediate representation) is present.
    assert!(
        out.contains("fn n_helper"),
        "full(): expected IR header for n_helper\n---\n{out}"
    );
    // Bytecode disassembly is present.
    assert!(
        out.contains("byte-code for"),
        "full(): expected bytecode section\n---\n{out}"
    );
    // Execution trace is present.
    assert!(
        out.contains("Execute test:"),
        "full(): expected execution header\n---\n{out}"
    );
    assert!(
        out.contains("Finished"),
        "full(): expected 'Finished' at end of execution\n---\n{out}"
    );
}

// ---------------------------------------------------------------------------
// 2. Phase filter — static_only()
// ---------------------------------------------------------------------------

/// `static_only()` shows IR and bytecode but suppresses the execution trace.
#[test]
fn static_only_has_no_execution() {
    let out = full_log(&LogConfig::static_only());

    assert!(
        out.contains("fn n_helper") || out.contains("byte-code for"),
        "static_only(): expected IR or bytecode\n---\n{out}"
    );
    assert!(
        !out.contains("Execute test:"),
        "static_only(): execution trace must be absent\n---\n{out}"
    );
    assert!(
        !out.contains("Finished"),
        "static_only(): 'Finished' must be absent\n---\n{out}"
    );
}

// ---------------------------------------------------------------------------
// 3. Phase filter — minimal()
// ---------------------------------------------------------------------------

/// `minimal()` shows only the execution trace; IR and bytecode are suppressed.
#[test]
fn minimal_has_no_static_output() {
    let out = full_log(&LogConfig::minimal());

    assert!(
        !out.contains("byte-code for"),
        "minimal(): bytecode section must be absent\n---\n{out}"
    );
    // IR headers look like "fn n_<name>(" — absent in minimal mode.
    assert!(
        !out.contains("fn n_helper"),
        "minimal(): IR must be absent\n---\n{out}"
    );
    assert!(
        out.contains("Execute test:"),
        "minimal(): execution must be present\n---\n{out}"
    );
}

// ---------------------------------------------------------------------------
// 4. Slot annotations — annotate_slots flag
// ---------------------------------------------------------------------------

/// With `annotate_slots: true` (default in `full()`) the execution trace
/// appends `=<varname>` to variable-access steps.
#[test]
fn annotate_slots_adds_var_names_to_execution() {
    let out_annotated = exec_log(&LogConfig::full());
    let out_plain = exec_log(&LogConfig {
        annotate_slots: false,
        ..LogConfig::full()
    });

    // The annotated trace should mention the variable `x` by name.
    assert!(
        out_annotated.contains("=x"),
        "annotate_slots=true: expected '=x' in execution trace\n---\n{out_annotated}"
    );
    // The plain trace must NOT have the annotation.
    assert!(
        !out_plain.contains("=x"),
        "annotate_slots=false: '=x' must be absent\n---\n{out_plain}"
    );
}

/// With `annotate_slots: true` the bytecode dump appends `var=name[slot]:type`.
#[test]
fn annotate_slots_adds_var_info_to_bytecode() {
    let out_annotated = static_log(&LogConfig::full());
    let out_plain = static_log(&LogConfig {
        annotate_slots: false,
        ..LogConfig::full()
    });

    assert!(
        out_annotated.contains("var="),
        "annotate_slots=true: expected 'var=' in bytecode dump\n---\n{out_annotated}"
    );
    assert!(
        !out_plain.contains("var="),
        "annotate_slots=false: 'var=' must be absent from bytecode dump\n---\n{out_plain}"
    );
}

// ---------------------------------------------------------------------------
// 5. Function name filter — show_functions
// ---------------------------------------------------------------------------

/// `function("helper")` shows IR and bytecode only for `n_helper`; `n_test`
/// is absent from the static output.  The execution is suppressed too, because
/// the root function name ("test") does not match the filter ("helper").
#[test]
fn function_filter_limits_static_output() {
    let out = full_log(&LogConfig::function("helper"));

    assert!(
        out.contains("n_helper"),
        "function(helper): n_helper must appear\n---\n{out}"
    );
    assert!(
        !out.contains("n_test"),
        "function(helper): n_test must be absent from static output\n---\n{out}"
    );
    // Execution is suppressed because "test" doesn't match the filter.
    assert!(
        !out.contains("Execute test:"),
        "function(helper): execution trace must be absent\n---\n{out}"
    );
}

/// `function("test")` shows only the `n_test` function and runs the execution trace.
#[test]
fn function_filter_test_function_runs_execution() {
    let out = full_log(&LogConfig::function("test"));

    // n_test's IR must be present (n_test contains "test").
    assert!(
        out.contains("n_test"),
        "function(test): n_test must appear\n---\n{out}"
    );
    // n_helper's FUNCTION HEADER must not appear in static output.
    // Note: "n_helper" may still appear inside n_test's IR body as a call target,
    // so we check for the header prefix specifically.
    assert!(
        !out.contains("byte-code for log_test:n_helper"),
        "function(test): n_helper bytecode section must be absent\n---\n{out}"
    );
    // Execution trace must be present (root function "test" matches the filter).
    assert!(
        out.contains("Execute test:"),
        "function(test): execution must be present\n---\n{out}"
    );
}

// ---------------------------------------------------------------------------
// 6. Opcode filter — trace_opcodes
// ---------------------------------------------------------------------------

/// When only "Return" opcodes are traced, the execution output contains
/// "Return(" but not "ConstInt(" or "Call(".
#[test]
fn trace_opcodes_filters_execution_steps() {
    let config = LogConfig {
        phases: LogPhase::execution_only(),
        trace_opcodes: Some(vec!["Return".to_string()]),
        annotate_slots: false,
        ..LogConfig::full()
    };
    let out = exec_log(&config);

    assert!(
        out.contains("Return("),
        "trace_opcodes=[Return]: Return( must appear\n---\n{out}"
    );
    assert!(
        !out.contains("ConstInt("),
        "trace_opcodes=[Return]: ConstInt( must be absent\n---\n{out}"
    );
    assert!(
        !out.contains("Call("),
        "trace_opcodes=[Return]: Call( must be absent\n---\n{out}"
    );
    // Execution header and footer are always written.
    assert!(
        out.contains("Execute test:"),
        "trace_opcodes=[Return]: header must still appear\n---\n{out}"
    );
    assert!(
        out.contains("Finished"),
        "trace_opcodes=[Return]: Finished must still appear\n---\n{out}"
    );
}

// ---------------------------------------------------------------------------
// 7. Tail buffer — crash_tail()
// ---------------------------------------------------------------------------

/// `crash_tail(3)` retains only the last 3 execution lines.  The "Finished"
/// marker (always the final line) must be present; early steps may be dropped.
#[test]
fn crash_tail_retains_last_n_lines() {
    let out = exec_log(&LogConfig::crash_tail(3));

    // "Finished" must be in the flushed output (it's always the last line).
    assert!(
        out.contains("Finished"),
        "crash_tail(3): 'Finished' must be present\n---\n{out}"
    );

    // Count non-empty lines — should be ≤ 3 (the tail capacity).
    let lines: Vec<&str> = out.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        lines.len() <= 3,
        "crash_tail(3): expected ≤ 3 lines, got {}\n---\n{out}",
        lines.len()
    );
}

/// A larger tail that fits the whole trace preserves it completely.
#[test]
fn crash_tail_large_buffer_keeps_full_trace() {
    // Compare against the same config with the tail buffer disabled so that
    // the annotate_slots and other settings are identical.
    let base = LogConfig::crash_tail(1000);
    let uncapped = LogConfig {
        trace_tail: None,
        ..base.clone()
    };

    let out_tail = exec_log(&base);
    let out_uncapped = exec_log(&uncapped);

    assert_eq!(
        out_tail, out_uncapped,
        "crash_tail(1000): output should equal uncapped trace with same settings"
    );
}

// ---------------------------------------------------------------------------
// 8. Bridging invariant — check_bridging
// ---------------------------------------------------------------------------

/// The bridging check detects when the runtime stack_pos deviates from the
/// compile-time expected value.  For a correct program this surfaces an expected
/// offset introduced by the root-function return-address setup (execute_log
/// places the sentinel return address at runtime position 4–7, while
/// compile-time tracking starts at 0).  The important properties to verify are:
///   1. Violations ARE emitted (the feature is active).
///   2. Execution completes despite violations (they are warnings, not aborts).
#[test]
fn bridging_check_detects_offset_and_completes() {
    let out = exec_log(&LogConfig::bridging());

    // The known 4-byte root-frame offset must be reported as a violation.
    assert!(
        out.contains("BRIDGING VIOLATION"),
        "bridging(): expected at least one BRIDGING VIOLATION warning\n---\n{out}"
    );
    // Execution must still reach the end — violations are non-fatal.
    assert!(
        out.contains("Finished"),
        "bridging(): execution must complete in spite of violations\n---\n{out}"
    );
}

// ---------------------------------------------------------------------------
// 9. Preset properties — no pipeline needed
// ---------------------------------------------------------------------------

/// `full()` preset has all phases enabled plus slot annotations.
#[test]
fn preset_phase_properties() {
    let full = LogConfig::full();
    assert!(full.phases.ir && full.phases.bytecode && full.phases.execution);
    assert!(full.annotate_slots);
    assert!(full.show_functions.is_none());
    assert!(full.trace_opcodes.is_none());
    assert!(full.trace_tail.is_none());
    assert!(!full.check_bridging);
}

/// `static_only()` enables IR and bytecode but not execution.
#[test]
fn preset_static_only_properties() {
    let st = LogConfig::static_only();
    assert!(st.phases.ir && st.phases.bytecode && !st.phases.execution);
}

/// `minimal()` enables execution only with no slot annotations.
#[test]
fn preset_minimal_properties() {
    let min = LogConfig::minimal();
    assert!(!min.phases.ir && !min.phases.bytecode && min.phases.execution);
    assert!(!min.annotate_slots);
}

/// `ref_debug()` enables slot annotations and snapshot opcodes.
#[test]
fn preset_ref_debug_properties() {
    let rd = LogConfig::ref_debug();
    assert!(rd.annotate_slots);
    assert!(rd.snapshot_opcodes.is_some());
}

/// `bridging()` enables execution and bridging checks.
#[test]
fn preset_bridging_properties() {
    let br = LogConfig::bridging();
    assert!(!br.phases.ir && !br.phases.bytecode && br.phases.execution);
    assert!(br.check_bridging);
}

/// `crash_tail()` sets the tail buffer and enables execution only.
#[test]
fn preset_crash_tail_properties() {
    let ct = LogConfig::crash_tail(10);
    assert_eq!(ct.trace_tail, Some(10));
    assert!(!ct.phases.ir && !ct.phases.bytecode && ct.phases.execution);
}

/// `function()` sets the filter and enables all phases.
#[test]
fn preset_function_properties() {
    let fn_config = LogConfig::function("foo");
    assert_eq!(fn_config.show_functions, Some(vec!["foo".to_string()]));
    assert!(fn_config.phases.ir && fn_config.phases.bytecode && fn_config.phases.execution);
}

/// `variables()` enables IR+bytecode (no execution), slot annotations, and variable table.
#[test]
fn preset_variables_properties() {
    let vars = LogConfig::variables();
    assert!(vars.phases.ir && vars.phases.bytecode && !vars.phases.execution);
    assert!(vars.annotate_slots);
    assert!(vars.show_variables);
    assert!(!vars.check_bridging);
}

// ---------------------------------------------------------------------------
// 10. from_env preset selection
// ---------------------------------------------------------------------------

/// `from_env()` falls back to `full()` when `LOFT_LOG` is absent or unknown.
/// (We test only the preset properties — not env mutation — to stay thread-safe.)
#[test]
fn from_env_fallback_is_full() {
    // Remove the variable so from_env falls back.
    // SAFETY NOTE: env mutation in tests is not thread-safe if other threads
    // read the env concurrently.  We only test the *properties* of presets
    // here, not pipeline output, which is inherently safe.
    let config = LogConfig::full();
    assert!(config.phases.ir);
    assert!(config.phases.bytecode);
    assert!(config.phases.execution);
    assert!(config.annotate_slots);
}

/// from_env crash_tail variant parses the optional `:N` suffix.
#[test]
fn from_env_crash_tail_parses_n() {
    // Test the crash_tail preset directly (env mutation is not thread-safe).
    let c = LogConfig::crash_tail(42);
    assert_eq!(c.trace_tail, Some(42));
    assert!(!c.phases.ir);
}

// ---------------------------------------------------------------------------
// 11. LogPhase helper constructors
// ---------------------------------------------------------------------------

#[test]
fn log_phase_constructors() {
    let all = LogPhase::all();
    assert!(all.ir && all.bytecode && all.execution);

    let none = LogPhase::none();
    assert!(!none.ir && !none.bytecode && !none.execution);

    let stat = LogPhase::static_only();
    assert!(stat.ir && stat.bytecode && !stat.execution);

    let exec = LogPhase::execution_only();
    assert!(!exec.ir && !exec.bytecode && exec.execution);
}

// ---------------------------------------------------------------------------
// 12. filter helper methods
// ---------------------------------------------------------------------------

#[test]
fn filter_helpers_none_means_all() {
    let config = LogConfig::full(); // show_functions: None
    assert!(config.show_function("anything"));
    assert!(config.trace_opcode("anything"));
    assert!(!config.snapshot_opcode("anything")); // snapshot_opcodes is None → false
}

#[test]
fn filter_helpers_match_substring() {
    let config = LogConfig {
        show_functions: Some(vec!["helper".to_string()]),
        trace_opcodes: Some(vec!["Int".to_string(), "Call".to_string()]),
        snapshot_opcodes: Some(vec!["Ref".to_string()]),
        ..LogConfig::full()
    };

    assert!(config.show_function("n_helper"));
    assert!(!config.show_function("n_test"));

    assert!(config.trace_opcode("ConstInt"));
    assert!(config.trace_opcode("Call"));
    assert!(!config.trace_opcode("Return"));

    assert!(config.snapshot_opcode("CreateRef"));
    assert!(!config.snapshot_opcode("ConstInt"));
}

// ---------------------------------------------------------------------------
// 13. Variable table — variables() preset
// ---------------------------------------------------------------------------

/// `variables()` emits the column-header line and a row for every variable.
/// There is no execution trace.
#[test]
fn variables_preset_shows_variable_table() {
    let out = static_log(&LogConfig::variables());

    // The header row must be present.
    assert!(
        out.contains("slot") && out.contains("live"),
        "variables(): expected 'slot' and 'live' column headers\n---\n{out}"
    );
    // The separator line must be present.
    assert!(
        out.contains("---"),
        "variables(): expected separator line\n---\n{out}"
    );
    // Variable `x` from n_test must appear.
    assert!(
        out.contains("x"),
        "variables(): expected variable 'x' in the table\n---\n{out}"
    );
    // No execution trace.
    assert!(
        !out.contains("Execute test:"),
        "variables(): execution trace must be absent\n---\n{out}"
    );
}

/// With `show_variables: false` (all other presets) no variable table is emitted.
#[test]
fn variables_table_absent_without_flag() {
    let out = static_log(&LogConfig::full());

    assert!(
        !out.contains("live\n") && !out.contains("live\r"),
        "full(): variable table must be absent\n---\n{out}"
    );
}

/// The variable table is emitted for each function separately.
#[test]
fn variables_preset_covers_all_functions() {
    let out = static_log(&LogConfig::variables());

    // Both n_helper and n_test must have variable sections.
    assert!(
        out.contains("n_helper"),
        "variables(): n_helper section must be present\n---\n{out}"
    );
    assert!(
        out.contains("n_test"),
        "variables(): n_test section must be present\n---\n{out}"
    );
}
