// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(dead_code)]

//! Testing framework
use loft::create;
#[cfg(debug_assertions)]
use loft::data::Context;
use loft::scopes;
extern crate loft;
#[path = "common/mod.rs"]
mod common;
use loft::compile::byte_code;
#[cfg(debug_assertions)]
use loft::compile::show_code;
use loft::data::Data;
use loft::generation::Output;
#[cfg(debug_assertions)]
use loft::log_config::LogConfig;
use std::fs::File;
use std::io::Write;

/// Evaluate the given code.
/// When a result is given, there should be a present @test routine returning this result.
/// When a type is also given, this result should be of that type.
/// Defining an error will not expect a result but will validate that this specific error is thrown.
/// Defining warnings will just validate the given warnings and not expect a change of flow.
#[macro_export]
macro_rules! code {
    ($code:expr) => {
        testing::testing_code($code, stdext::function_name!())
    };
}

/// Directly evaluate a given expression.
/// This is shorthand for a test routine returning this expression.
#[macro_export]
macro_rules! expr {
    ($code:expr) => {
        testing::testing_expr($code, stdext::function_name!())
    };
}

use common::cached_default;
use loft::data::{Type, Value};
use loft::diagnostics::Level;
use loft::parser::Parser;
use loft::state::State;
use std::collections::BTreeSet;
use std::collections::HashMap;

// The test data for one test.
// Many parts can remain empty for each given test.
pub struct Test {
    name: String,
    file: String,
    expr: String,
    code: String,
    warnings: Vec<String>,
    errors: Vec<String>,
    fatal: Vec<String>,
    sizes: HashMap<String, u32>,
    result: Value,
    tp: Type,
    /// Compact slot-mapping spec checked after byte_code (debug builds only).
    /// Format: space-separated tokens of `name(scope)=slot`, e.g. `"_t(4L)=0 b(4L)=4"`.
    /// Scope suffix "L" means the scope is a loop scope; no suffix means a regular scope.
    expected_slots: Option<String>,
}

impl Test {
    /// Expect the parsing of the test to end in this error.
    /// Can be given multiple times to expect more than one error.
    pub fn error(&mut self, text: &str) -> &mut Test {
        if self.result != Value::Null {
            panic!("Cannot combine result with errors");
        }
        self.errors.push(text.to_string());
        self
    }

    pub fn fatal(&mut self, text: &str) -> &mut Test {
        if self.result != Value::Null {
            panic!("Cannot combine result with fatal");
        }
        self.fatal.push(text.to_string());
        self
    }

    /// Expect this warning during parsing.
    /// This will not change if it results in an error or a normal result.
    pub fn warning(&mut self, text: &str) -> &mut Test {
        self.warnings.push(text.to_string());
        self
    }

    /// Shorthand expressions for a test routine that returns a result.
    pub fn expr(&mut self, value: &str) -> &mut Test {
        self.expr = value.to_string();
        self
    }

    /// Assert the stack-slot layout of `n_test` variables after codegen (debug builds only).
    ///
    /// `spec` is a space-separated list of `name(scope)=slot` tokens, e.g.:
    /// `"_t(4L)=0  b(4L)=4"` — `_t` in loop scope 4 at slot 0, `b` in loop scope 4 at slot 4.
    /// Scope suffix "L" asserts a loop scope; no suffix asserts a regular (non-loop) scope.
    #[cfg(debug_assertions)]
    pub fn slots(&mut self, spec: &str) -> &mut Test {
        self.expected_slots = Some(spec.to_string());
        self
    }

    /// The expected result value. Cannot be combined with expected errors.
    pub fn result(&mut self, value: Value) -> &mut Test {
        if !self.errors.is_empty() {
            panic!("Cannot combine result with errors");
        }
        if matches!(value, Value::Boolean(_)) {
            self.tp = Type::Boolean;
        }
        self.result = value;
        self
    }

    /// In some cases the result type will different from its internal type.
    /// This is the case for Type::Boolean or Type::Enum types that return Value::Int(_) values.
    /// Also Value::None results can happen in combination with most other types.
    pub fn tp(&mut self, tp: Type) -> &mut Test {
        self.tp = tp;
        self
    }

    fn test(&self) -> String {
        let mut res = match &self.result {
            Value::Long(v) => v.to_string() + "l",
            Value::Int(v) => v.to_string(),
            Value::Enum(v, _) => v.to_string(),
            Value::Boolean(v) if *v => "true".to_string(),
            Value::Boolean(_) => "false".to_string(),
            Value::Text(v) => replace_tokens(v),
            Value::Float(v) => v.to_string(),
            Value::Single(v) => v.to_string(),
            Value::Null if matches!(self.tp, Type::Text(_) | Type::Integer(_, _, _)) => {
                "null".to_string()
            }
            Value::Null if !matches!(self.tp, Type::Text(_)) => {
                return format!("pub fn test() {{\n    {};\n}}", self.expr);
            }
            _ => panic!("test {:?}", self.result),
        };
        let mut message = res.clone();
        if matches!(self.result, Value::Text(_)) {
            message = "\\\"".to_string() + &res + "\\\"";
            res = "\"".to_string() + &res + "\"";
        }
        format!(
            "pub fn test() {{\n    test_value = {{{}}};\n    assert(\n        test_value == {res},\n        \"Test failed {{test_value}} != {message}\"\n    );\n}}",
            self.expr
        )
    }

    #[cfg(debug_assertions)]
    fn output_code(
        &mut self,
        data: &mut Data,
        types: usize,
        code: &mut String,
        state: &mut State,
        config: &LogConfig,
    ) -> File {
        let _ = std::fs::create_dir_all("tests/dumps");
        let mut w = File::create(format!("tests/dumps/{}_{}.txt", self.file, self.name)).unwrap();
        writeln!(w, "{code}").unwrap();
        let to = state.database.types.len();
        for tp in types..to {
            writeln!(w, "Type {tp}:{}", state.database.show_type(tp as u16, true)).unwrap();
        }
        show_code(&mut w, state, data, config).unwrap();
        w
    }
}

fn replace_tokens(res: &str) -> String {
    res.replace("{", "{{")
        .replace("}", "}}")
        .replace("\n", "\\n")
        .replace("\"", "\\\"")
}

impl Drop for Test {
    // The actual evaluation of the test happens when the Test object is dropped.
    // So there is no need for an 'activate' method call.
    #[allow(unused_variables)]
    fn drop(&mut self) {
        let mut p = Parser::new();
        let (data, db) = cached_default();
        p.data = data;
        p.database = db;
        let types = p.database.types.len();
        let start = p.data.definitions();
        let mut code = self.code.clone();
        if !self.expr.is_empty() {
            if !code.is_empty() {
                code += "\n\n";
            }
            code += &self.test();
        }
        p.parse_str(&code, &self.name, false);
        for (d, s) in &self.sizes {
            let size = p.database.size(p.data.def(p.data.def_nr(d)).known_type);
            assert_eq!(u32::from(size), *s, "Size of {}", *d);
        }
        scopes::check(&mut p.data);
        #[cfg(debug_assertions)]
        self.generate_code(&p, start).unwrap();
        // Validate that we found the correct warnings and errors. Halt when differences are found.
        self.assert_diagnostics(&p);
        // Do not interpret anything when parsing did not succeed.
        if p.diagnostics.level() >= Level::Error {
            return;
        }
        // generate_code (fill.rs) is now done via `make fill` to avoid
        // file-write races during parallel test execution.  Per-test native
        // codegen output still goes to tests/generated/<test>.rs below.
        create::generate_lib(&p.data).unwrap();
        let mut state = State::new(p.database);
        byte_code(&mut state, &mut p.data);
        #[cfg(debug_assertions)]
        if let Some(spec) = &self.expected_slots {
            let test_nr = p.data.def_nr("n_test");
            let f = &p.data.def(test_nr).variables;
            // Build the full calculated layout, sorted by slot, for diff output.
            let mut all: Vec<(u16, u16, u16)> = (0..f.next_var())
                .filter(|&v| f.stack(v) != u16::MAX)
                .map(|v| (f.stack(v), f.scope(v), v))
                .collect();
            all.sort_by_key(|&(slot, _, _)| slot);
            // Scope ranks: sorted unique non-arg scope numbers → depth 0, 1, 2, ...
            let scope_ranks: std::collections::HashMap<u16, usize> = {
                let unique: std::collections::BTreeSet<u16> = all
                    .iter()
                    .filter(|&&(_, _, v)| !f.is_argument(v))
                    .map(|&(_, scope, _)| scope)
                    .filter(|&s| s != u16::MAX)
                    .collect();
                unique
                    .into_iter()
                    .enumerate()
                    .map(|(i, s)| (s, i))
                    .collect()
            };
            // Build the calculated visual: a scope-header line on first entry into each scope,
            // followed by variable lines with depth bars but no per-line scope label.
            let mut seen_scopes: std::collections::HashSet<u16> = std::collections::HashSet::new();
            let mut lines: Vec<String> = Vec::new();
            let mut full_tokens: Vec<String> = Vec::new();
            for &(slot, scope, v) in &all {
                let is_arg = f.is_argument(v);
                let ctx = if is_arg {
                    Context::Argument
                } else {
                    Context::Variable
                };
                let sz = f.size(v, &ctx);
                let scope_str = if is_arg {
                    "arg".to_string()
                } else if scope == u16::MAX {
                    "-".to_string()
                } else if f.is_loop_scope(scope) {
                    format!("{scope}L")
                } else {
                    scope.to_string()
                };
                let origin: &str = if is_arg || scope == u16::MAX {
                    ""
                } else {
                    f.scope_origin(scope)
                };
                let rank = if is_arg {
                    0
                } else {
                    scope_ranks.get(&scope).copied().unwrap_or(0)
                };
                let bars = "│ ".repeat(rank);
                let scope_key = if is_arg { u16::MAX } else { scope };
                if seen_scopes.insert(scope_key) {
                    let seq_range = if !is_arg && scope != u16::MAX {
                        if let Some((s, e)) = f.loop_seq_range(scope) {
                            format!(" [seq {s}..{e}]")
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    let header = if origin.is_empty() {
                        format!("  {bars}{scope_str}{seq_range}")
                    } else {
                        format!("  {bars}{origin}:{scope_str}{seq_range}")
                    };
                    lines.push(header);
                }
                let interval = if is_arg {
                    String::new()
                } else {
                    let fd = f.first_def(v);
                    let lu = f.last_use(v);
                    if fd == u32::MAX {
                        " [never]".to_string()
                    } else {
                        format!(" [{fd}..{lu}]")
                    }
                };
                lines.push(format!("  {bars}{}+{sz}={slot}{interval}", f.name(v)));
                full_tokens.push(format!("{}({scope_str})+{sz}={slot}", f.name(v)));
            }
            let calculated = lines.join("\n");
            // Compact single-line spec for copy-pasting slot values.
            let spec_line = full_tokens.join("  ");
            if spec.is_empty() {
                panic!(
                    "slots not asserted; calculated:\n{calculated}\n\n  .slots(\"{spec_line}\")"
                );
            }
            if spec.trim() != calculated.trim() {
                panic!(
                    "slots mismatch:\n  asserted:\n{}\n  calculated:\n{calculated}\n\n  .slots(\"{spec_line}\")",
                    spec.lines()
                        .map(|l| format!("    {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
        #[cfg(debug_assertions)]
        {
            let config = LogConfig::from_env();
            let mut w = self.output_code(&mut p.data, types, &mut code, &mut state, &config);
            state.execute_log(&mut w, "test", &config, &p.data).unwrap();
        }
        #[cfg(not(debug_assertions))]
        state.execute("test", &p.data);
    }
}

impl Test {
    fn generate_code(&self, p: &Parser, start: u32) -> std::io::Result<()> {
        std::fs::create_dir_all("tests/generated")?;
        let w = &mut File::create("tests/generated/default.rs")?;
        let mut o = Output {
            data: &p.data,
            stores: &p.database,
            counter: 0,
            indent: 0,
            def_nr: 0,
            declared: Default::default(),
            reachable: Default::default(),
            loop_stack: Vec::new(),
        };
        o.output_native(w, 0, start)?;
        // Write code output when the result is tested, not only for errors or warnings.
        if self.result != Value::Null || !self.tp.is_unknown() {
            let w = &mut File::create(format!("tests/generated/{}_{}.rs", self.file, self.name))?;
            let def_nr = p.data.definitions();
            // Find the entry function n_test and emit only reachable functions.
            let test_fn = p.data.def_nr("n_test");
            if test_fn != u32::MAX {
                o.output_native_reachable(w, start, def_nr, &[test_fn])?;
            } else {
                o.output_native(w, start, def_nr)?;
            }
            writeln!(w, "#[test]\nfn code_{}() {{", self.name)?;
            writeln!(w, "    let mut stores = Stores::new();")?;
            writeln!(w, "    init(&mut stores);")?;
            writeln!(w, "    n_test(&mut stores);")?;
            writeln!(w, "}}")?;
        }
        Ok(())
    }

    fn assert_diagnostics(&self, p: &Parser) {
        let mut expected = BTreeSet::new();
        for w in &self.warnings {
            expected.insert(format!("Warning: {w}"));
        }
        for w in &self.errors {
            expected.insert(format!("Error: {w}"));
        }
        for w in &self.fatal {
            expected.insert(format!("Fatal: {w}"));
        }
        let mut found = "".to_string();
        for l in p.diagnostics.lines() {
            if expected.contains(l) {
                expected.remove(l);
            } else {
                if !found.is_empty() {
                    found += "|";
                }
                found += l;
            }
        }
        let mut was = "".to_string();
        for e in expected {
            if !was.is_empty() {
                was += "|";
            }
            was += &e;
        }
        if !found.is_empty() || !was.is_empty() {
            panic!("Found '{found}' Expected '{was}'");
        }
    }

    // Try to decipher the correct return type from value() and tp() data.
    fn return_type(&self) -> &str {
        let tp = if self.tp.is_unknown() {
            if let Value::Int(_) = self.result {
                Type::Integer(i32::MIN, i32::MAX as u32, false)
            } else if let Value::Long(_) = self.result {
                Type::Long
            } else if let Value::Text(_) = self.result {
                Type::Text(Vec::new())
            } else if let Value::Float(_) = self.result {
                Type::Float
            } else if let Value::Null = self.result {
                return "";
            } else {
                Type::Unknown(0)
            }
        } else {
            self.tp.clone()
        };
        if let Type::Integer(_, _, _) = tp {
            "integer"
        } else if let Type::Text(_) = tp {
            "text"
        } else if let Type::Long = tp {
            "long"
        } else if let Type::Boolean = tp {
            "boolean"
        } else if let Type::Float = tp {
            "float"
        } else {
            panic!("Unknown type {tp:?}");
        }
    }
}

fn short(name: &str) -> String {
    let s: Vec<&str> = name.split("::").collect();
    s[s.len() - 1].to_string()
}

fn front(name: &str) -> String {
    let s: Vec<&str> = name.split("::").collect();
    s[s.len() - 2].to_string()
}

pub fn testing_code(code: &str, test: &str) -> Test {
    Test {
        name: short(test),
        file: front(test),
        expr: "".to_string(),
        code: code.to_string(),
        warnings: vec![],
        errors: vec![],
        fatal: vec![],
        result: Value::Null,
        tp: Type::Unknown(0),
        sizes: HashMap::new(),
        expected_slots: None,
    }
}

pub fn testing_expr(expr: &str, test: &str) -> Test {
    Test {
        name: short(test),
        file: front(test),
        expr: expr.to_string(),
        code: "".to_string(),
        warnings: vec![],
        errors: vec![],
        fatal: vec![],
        result: Value::Null,
        tp: Type::Unknown(0),
        sizes: HashMap::new(),
        expected_slots: None,
    }
}
