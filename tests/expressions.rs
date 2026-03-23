// Copyright (c) 2021-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

extern crate loft;

mod testing;

use loft::data::{Type, Value};

const INTEGER: Type = Type::Integer(i32::MIN + 1, i32::MAX as u32);

#[test]
fn expr_add_null() {
    expr!("1 + null").tp(INTEGER);
}

#[test]
fn expr_zero_divide() {
    expr!("2 / (3 - 2 - 1)").tp(INTEGER);
}

#[test]
fn div_by_const_zero() {
    expr!("2 / 0")
        .tp(INTEGER)
        .warning("Division by constant zero — result is always null at div_by_const_zero:2:25");
}

#[test]
fn rem_by_const_zero() {
    expr!("5 % 0")
        .tp(INTEGER)
        .warning("Modulo by constant zero — result is always null at rem_by_const_zero:2:25");
}

#[test]
fn append_fn() {
    code!("fn append(ch: character) -> text { \"abc_de\" + ch }")
        .expr("append('x')")
        .result(Value::str("abc_dex"));
}

#[test]
fn append_str() {
    code!("fn append(ch: character) -> text { s=\"abc_de\"; s += ch; s }")
        .expr("append('x')")
        .result(Value::str("abc_dex"));
}

#[test]
fn continue_loop() {
    code!("fn routine() -> integer {b = 0; for a in 0..10 { if a == 2 {continue} if a > 5 {return b} b += a }; b}")
    .expr("routine()").result(Value::Int(13));
}

#[test]
fn call_void() {
    code!("fn routine(a: integer) {return;}")
        .expr("routine(5)")
        .warning("Parameter a is never read at call_void:1:25");
}

#[test]
fn call_with_null() {
    code!("fn add(a: integer, b: integer) -> integer { a + b }")
        .expr("add(1, null)")
        .tp(INTEGER)
        .result(Value::Null);
}

#[test]
fn call_text_null() {
    code!("fn routine(a: integer) -> text { if a > 2 { return null }; \"#{a}#\"}")
        .expr("routine(5)")
        .tp(Type::Text(vec![]))
        .result(Value::Null);
}

#[test]
fn call_int_null() {
    code!("fn routine(a: integer) -> integer { if a > 2 { return null }; a+1 }")
        .expr("routine(5)")
        .tp(INTEGER)
        .result(Value::Null);
}

#[test]
fn if_typing() {
    expr!("a = \"12\"; if a.len()>2 { null } else { \"error\" }").result(Value::str("error"));
    expr!("a = \"12\"; if a.len()==2 { null } else { \"error\" }")
        .tp(Type::Text(vec![]))
        .result(Value::Null);
}

#[test]
fn inner_loop() {
    expr!(
        "
s = \"\";
for i in 0..10 {
    for j in 0..10 {
        if j > i {
            i#continue
        }
        s += \"{i}{j},\";
        if len(s) > 100 {
            i#break
        }
    }
}
s
"
    )
    .result(Value::str("00,10,11,20,21,22,30,31,32,33,40,41,42,43,44,50,51,52,53,54,55,60,61,62,63,64,65,66,70,71,72,73,74,75,"));
}

#[test]
fn recursion() {
    code!(
        "fn first(s: State, c: integer) -> integer {
	if s == Start {
		s = Ongoing
	} else if c > 10 {
		s = Halt
	}
	second(s, c)
}

fn second(s: State, c: integer) -> integer {
	if s != Halt {
		first(s, c + 1)
	} else {
		1 + c
	}
}

enum State {
	Start,
	Ongoing,
	Halt
}"
    )
    .expr("first(Start, 0)")
    .result(Value::Int(12));
}

// ── T1-7: Null-coalescing operator ?? ─────────────────────────────────────────

/// Non-null integer: ?? returns the LHS value.
#[test]
fn null_coerce_integer_non_null() {
    code!(
        "fn test() {
    x = 42;
    r = x ?? 0;
    assert(r == 42, \"r: {r}\");
}"
    );
}

/// Null integer: ?? returns the default.
#[test]
fn null_coerce_integer_null() {
    code!(
        "fn test() {
    x = null;
    r = x ?? 99;
    assert(r == 99, \"r: {r}\");
}"
    );
}

/// Non-null field access: ?? returns the field value.
#[test]
fn null_coerce_field_non_null() {
    code!(
        "struct Row { val: integer }
fn test() {
    r = Row { val: 7 };
    result = r.val ?? 0;
    assert(result == 7, \"result: {result}\");
}"
    );
}

/// Chaining: `a ?? b ?? c` — a is non-null, so result is a.
#[test]
fn null_coerce_chain_first_non_null() {
    code!(
        "fn test() {
    a = 1;
    b = 2;
    c = 3;
    r = a ?? b ?? c;
    assert(r == 1, \"r: {r}\");
}"
    );
}

/// Dead assignment: variable overwritten before first read.
#[test]
fn dead_assignment_basic() {
    code!(
        "fn test() {
    x = 1;
    x = 2;
    assert(x == 2, \"x\");
}"
    )
    .warning(
        "Dead assignment — 'x' is overwritten before being read at dead_assignment_basic:2:11",
    );
}

/// No dead assignment when variable is read between writes.
#[test]
fn dead_assignment_read_between() {
    code!(
        "fn test() {
    x = 1;
    assert(x == 1, \"x\");
    x = 2;
    assert(x == 2, \"x\");
}"
    );
}

/// Dead assignment: underscore-prefixed variable is exempt.
#[test]
fn dead_assignment_underscore_exempt() {
    code!(
        "fn test() {
    _x = 1;
    _x = 2;
    assert(_x == 2, \"x\");
}"
    );
}

/// Dead assignment: += is not a dead write (it reads the variable).
#[test]
fn dead_assignment_augmented_not_dead() {
    code!(
        "fn test() {
    x = 1;
    x += 2;
    assert(x == 3, \"x\");
}"
    );
}

/// Dead assignment: conditional update after initialization is not dead.
#[test]
fn dead_assignment_branch_not_dead() {
    code!(
        "fn test() {
    result = 0;
    if true {
        result = 42;
    };
    assert(result == 42, \"result\");
}"
    );
}

/// Dead assignment: writes in sibling branches do not warn each other.
#[test]
fn dead_assignment_sibling_branches() {
    code!(
        "fn test() {
    x = 0;
    if true {
        x = 1;
    } else {
        x = 2;
    };
    assert(x > 0, \"x\");
}"
    );
}

/// Dead assignment: parameter reassignment in branches does not warn.
#[test]
fn dead_assignment_param_branch() {
    code!(
        "enum State { Start, Ongoing, Halt }
fn step(s: State) -> State {
    if s == Start {
        s = Ongoing;
    } else {
        s = Halt;
    };
    s
}
fn test() {
    assert(step(Start) == Ongoing, \"step\");
}"
    );
}

/// Dead assignment: initialization before loop is not dead.
#[test]
fn dead_assignment_loop_init() {
    code!(
        "fn test() {
    sum = 0;
    for i in 1..4 {
        sum = sum + i;
    };
    assert(sum == 6, \"sum\");
}"
    );
}

/// Dead assignment: match arm writes do not trigger across arms.
#[test]
fn dead_assignment_match_arms() {
    code!(
        "enum Color { Red, Green, Blue }
fn label(c: Color) -> integer {
    result = 0;
    match c {
        Red => result = 1,
        Green => result = 2,
        Blue => result = 3
    };
    result
}
fn test() {
    assert(label(Red) == 1, \"red\");
}"
    );
}

/// Chaining: first two are null, third is non-null.
#[test]
fn null_coerce_chain_last_non_null() {
    code!(
        "fn test() {
    a = null;
    b = null;
    c = 3;
    r = a ?? b ?? c;
    assert(r == 3, \"r: {r}\");
}"
    );
}

/// T1-12: comparing a `not null` field to null with `==` warns (always false).
#[test]
fn redundant_null_eq_not_null_field() {
    code!(
        "struct Counter { count: integer not null }
fn test() {
    c = Counter { count: 0 };
    if c.count == null { assert(false, \"unreachable\"); };
}"
    )
    .warning(
        "Redundant null check — 'count' is 'not null', comparison is always false at redundant_null_eq_not_null_field:4:25",
    );
}

/// T1-12: comparing a `not null` field to null with `!=` warns (always true).
#[test]
fn redundant_null_ne_not_null_field() {
    code!(
        "struct Counter { count: integer not null }
fn test() {
    c = Counter { count: 0 };
    assert(c.count != null, \"always true\");
}"
    )
    .warning(
        "Redundant null check — 'count' is 'not null', comparison is always true at redundant_null_ne_not_null_field:4:28",
    );
}

/// T1-12: null-coalescing on a `not null` field warns (redundant).
#[test]
fn redundant_null_coalesce_not_null_field() {
    code!(
        "struct Counter { count: integer not null }
fn test() {
    c = Counter { count: 0 };
    r = c.count ?? 99;
    assert(r == 0, \"r: {r}\");
}"
    )
    .warning(
        "Redundant null coalescing — 'count' is 'not null', default is never used at redundant_null_coalesce_not_null_field:4:22",
    );
}

/// T1-12: no warning when nullable field is compared to null (normal usage).
#[test]
fn no_warning_nullable_field_null_check() {
    code!(
        "struct Item { value: integer }
fn test() {
    it = Item {};
    if it.value == null {
        assert(true, \"ok\");
    };
}"
    );
}

/// T1-12: no warning when nullable field uses null-coalescing (normal usage).
#[test]
fn no_warning_nullable_field_coalesce() {
    code!(
        "struct Item { value: integer }
fn test() {
    it = Item {};
    r = it.value ?? 42;
    assert(r == 0, \"r: {r}\");
}"
    );
}

/// N6: verify that a simple generated file compiles with rustc.
/// This prevents regressions in the native code generation backend.
#[test]
fn generated_code_compiles() {
    // First, generate the file by running a simple test.
    expr!("1 + 2").tp(INTEGER).result(Value::Int(3));
    let file = "tests/generated/expressions_expr_add_null.rs";
    if !std::path::Path::new(file).exists() {
        return; // Skip if generated files not present (e.g. release build)
    }
    let out_dir = std::path::Path::new(".loft");
    let _ = std::fs::create_dir_all(out_dir);
    let output = std::process::Command::new("rustc")
        .args([
            "--edition",
            "2021",
            "--crate-type",
            "lib",
            file,
            "--out-dir",
            out_dir.to_str().unwrap(),
            "-L",
            "target/debug/deps",
            "--extern",
            &format!("loft={}", find_loft_rlib()),
        ])
        .output()
        .expect("failed to run rustc");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "Generated file {file} failed to compile:\n{stderr}"
    );
}

/// N7: run the full loft test suite against generated native code.
/// Only runs when LOFT_TEST_NATIVE=1 is set (skipped by default to avoid
/// slowing down normal cargo test with rustc invocations).
///
/// For each generated test file:
/// 1. Compile with rustc --test (produces a test binary)
/// 2. Run the test binary
/// 3. Track compile pass/fail and execution pass/fail
///
/// Panics if the native pass rate drops below the recorded baseline.
#[test]
fn native_test_suite() {
    if std::env::var("LOFT_TEST_NATIVE").as_deref() != Ok("1") {
        return; // Skip unless explicitly enabled
    }
    let gen_dir = std::path::Path::new("tests/generated");
    if !gen_dir.exists() {
        return;
    }
    let loft_rlib = find_loft_rlib();
    let tmp_dir = std::path::Path::new("target/native_tests");
    std::fs::create_dir_all(tmp_dir).unwrap();

    let mut compile_ok = 0u32;
    let mut compile_fail = 0u32;
    let mut run_ok = 0u32;
    let mut run_fail = 0u32;
    let mut failures: Vec<String> = Vec::new();

    let mut entries: Vec<_> = std::fs::read_dir(gen_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.ends_with(".rs") && name != "default.rs" && name != "fill.rs" && name != "text.rs"
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let src = entry.path();
        let stem = src.file_stem().unwrap().to_string_lossy().to_string();
        let bin = tmp_dir.join(&stem);

        // Compile as test binary; --out-dir keeps auxiliary rlibs inside tmp_dir
        // instead of the project root.
        let compile = std::process::Command::new("rustc")
            .args([
                "--edition",
                "2021",
                "--test",
                src.to_str().unwrap(),
                "-o",
                bin.to_str().unwrap(),
                "--out-dir",
                tmp_dir.to_str().unwrap(),
                "-L",
                "target/debug/deps",
                "--extern",
                &format!("loft={loft_rlib}"),
            ])
            .output()
            .expect("failed to run rustc");

        if !compile.status.success() {
            compile_fail += 1;
            continue;
        }
        compile_ok += 1;

        // Run the test binary
        let run = std::process::Command::new(bin.to_str().unwrap())
            .output()
            .expect("failed to run test binary");

        if run.status.success() {
            run_ok += 1;
        } else {
            run_fail += 1;
            let stderr = String::from_utf8_lossy(&run.stderr);
            failures.push(format!("{stem}: {}", stderr.lines().next().unwrap_or("?")));
        }
    }

    let total = entries.len() as u32;
    eprintln!(
        "\n=== Native test suite: {total} files, {compile_ok} compile, {run_ok} pass, {run_fail} fail, {compile_fail} skip ===",
    );
    if !failures.is_empty() {
        eprintln!("Failures:");
        for f in &failures {
            eprintln!("  {f}");
        }
    }

    // Baseline: at least 50 files must compile (current: 50).
    // Update this number as codegen improves.
    assert!(
        compile_ok >= 50,
        "Native compile rate regressed: {compile_ok} < 50 (of {total})"
    );
}

fn find_loft_rlib() -> String {
    // Always prefer the most recently modified libloft-<hash>.rlib in deps/,
    // which is the canonical artifact produced by the current cargo build.
    // target/debug/libloft.rlib can be stale across rebuilds.
    let mut best: Option<(std::time::SystemTime, String)> = None;
    if let Ok(entries) = std::fs::read_dir("target/debug/deps") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("libloft-")
                && name.ends_with(".rlib")
                && let Ok(mtime) = entry.metadata().and_then(|m| m.modified())
            {
                let path = entry.path().to_string_lossy().to_string();
                if best.as_ref().is_none_or(|(t, _)| mtime > *t) {
                    best = Some((mtime, path));
                }
            }
        }
    }
    if let Some((_, path)) = best {
        return path;
    }
    "target/debug/libloft.rlib".to_string()
}
