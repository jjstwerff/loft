// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Parse scripts and create internal code from it.
//! Including type checking.

use crate::data::{
    Argument, Context, Data, DefType, I32, IntegerSpec, Type, Value, to_default, v_block, v_if,
    v_loop, v_set,
};
use crate::database::{Parts, Stores};
use crate::diagnostics::{Diagnostics, Level, diagnostic_format};
use crate::lexer::{LexItem, LexResult, Lexer, Link, Mode, Position};
use crate::platform::{other_sep, sep, sep_str};
use crate::variables::{Function, size as var_size};
use crate::{manifest, scopes, typedef};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::env;
use std::fs::{File, metadata, read_dir};
use std::io::Write;
use std::string::ToString;
use typedef::complete_definition;

/**
The number of defined reserved text worker variables. A worker variable is needed when
two texts are added or a formatting text is used, and the result is used as a parameter to a call.
These are reused when possible. However, when calculating a text, a new text expression
is used a next worker variable is needed.
This number indicated the depth of these expressions, not the number of these expressions in a
function.
*/
// The parser holds several independent boolean mode flags (in_loop, default, first_pass,
// reverse_iterator) that each track a distinct parse phase or context.  Combining them into
// an enum or state machine would add complexity without benefit.
/// Whether a `use lib::...` statement imports all names or a specific subset.
#[derive(Clone)]
enum ImportSpec {
    Wildcard,
    Names(Vec<String>),
}

/// A pending import queued when `use lib::spec` is parsed.
/// Applied after all definitions in `for_source` are fully parsed.
#[derive(Clone)]
struct PendingImport {
    for_source: u16,
    lib_source: u16,
    spec: ImportSpec,
}

/// Pure-resolution result from [`Parser::lib_path_manifest_resolve`].
/// Callers decide when to apply side effects (native-lib registration,
/// dependency queueing, etc.).  The legacy `lib_path_manifest` adapter
/// applies them immediately; Phase A of the package-mode driver
/// consults the manifest to build its package graph, then defers
/// side-effect application until after pass-1 parsing.
struct ResolvedPkg {
    pkg_dir: String,
    entry: String,
    /// `None` when the package directory exists but has no `loft.toml`
    /// (pure multi-file package without a manifest).
    manifest: Option<manifest::Manifest>,
}

#[allow(clippy::struct_excessive_bools)]
pub struct Parser {
    pub todo_files: Vec<(String, u16)>,
    /// All definitions
    pub data: Data,
    pub database: Stores,
    /// The lexer on the current text file
    pub lexer: Lexer,
    /// Are we currently allowing break/continue statements?
    in_loop: bool,
    /// True while parsing an expression inside a format string `{…}`.
    /// Prevents the `v: type = expr` annotation from consuming `:`.
    pub(crate) in_format_expr: bool,
    /// The current file number that is being parsed
    file: u32,
    pub diagnostics: Diagnostics,
    default: bool,
    /// The definition that is currently parsed (function or struct)
    context: u32,
    /// Extra library directories for 'use' resolution (from --lib / --project flags)
    pub lib_dirs: Vec<String>,
    /// Resolved paths of native shared libraries to load after `byte_code()`.
    /// Populated during `use` processing when a package manifest contains `native`.
    pub pending_native_libs: Vec<String>,
    /// PKG.3: package dependencies discovered during manifest reading.
    /// Each entry is (name, dir) — sibling packages are searched in `dir`.
    pending_pkg_deps: Vec<(String, String)>,
    /// Is this the first pass on parsing:
    /// - Do not assume that all struct / enum types are already parsed.
    /// - Define variables, try to determine their type (can become clear from later code).
    /// - Claim working text variables for expressions that gather text data outside variables.
    /// - Links between memory allocations (text, stores) their type knows the variable numbers.
    /// - Move variables to a lower scope if an expression still links to their content.
    /// - Determine mutations to stores and administer these in arguments.
    ///
    /// The second pass:
    /// - Creates code, assumes that all types are known.
    first_pass: bool,
    /// Set by `parse_in_range` when `rev(collection)` (without a `..` range) is parsed.
    /// Consumed by `fill_iter` to add the reverse bit (64) into the `on` byte of OpIterate/OpStep.
    reverse_iterator: bool,
    /// O8.5: range bounds captured by `parse_in_range_body` for const-unroll detection.
    pub(crate) last_range_from: Option<Value>,
    pub(crate) last_range_till: Option<Value>,
    vars: Function,
    /// Last seen line inside the source code, an increase inserts it in the internal code.
    line: u32,
    /// Wildcard and selective imports waiting to be applied once the target source is fully parsed.
    pending_imports: Vec<PendingImport>,
    /// every (for_source, lib_source, ImportSpec) pair that
    /// `apply_pending_imports` applied during this parse pass.  Retained so
    /// that `resolve_deferred_unknowns` can re-apply them with overwrite
    /// semantics after cyclic `use` declarations have left Unknown stubs.
    applied_imports: Vec<PendingImport>,
    /// `DefType::Unknown` stubs collected by `actual_types_deferred`
    /// during each `parse_file` run.  Resolved (or finally reported) by
    /// `resolve_deferred_unknowns` after all files in the recursion have
    /// had their pass-1 / pass-2 definitions registered.
    deferred_unknown: Vec<(u16, u32, Position)>,
    /// Whether the most recently parsed expression is from a `not null` field access.
    /// Set by `get_field`; consumed by `handle_operator` to warn on redundant null checks.
    expr_not_null: bool,
    /// The field name for the most recently parsed `not null` field access (for diagnostics).
    expr_not_null_name: String,
    /// Counter incremented each time a lambda expression is parsed.
    /// Lambda names are `__lambda_N`; the same N is produced on both passes because the counter
    /// advances identically in both passes (same token order → same parse order).
    pub lambda_counter: u32,
    /// Expected `Type::Function(params, ret)` for the argument currently being parsed.
    /// Set by `parse_call` before parsing a function-typed argument so that short-form
    /// lambdas (`|x| { … }`) can infer parameter types from the call-site context.
    /// Cleared to `Type::Unknown(0)` immediately after the argument is parsed.
    pub(crate) lambda_hint: Type,
    /// Set by `iter_op` when `#fields` is encountered. Holds the struct `def_nr`.
    /// Checked by `parse_for` to take the unrolling path. Reset after use.
    pub(crate) fields_of: u32,
    /// Outer-scope variable names and types, populated when parsing a lambda body.
    /// When a variable is not found in the lambda's scope but exists here, it is a capture.
    /// Empty when not inside a lambda.
    pub(crate) capture_context: Vec<(String, Type)>,
    /// Accumulates captured variable names and types during lambda body parsing.
    /// Reset at the start of each lambda; read after parsing to synthesize the closure record.
    pub(crate) captured_names: Vec<(String, Type)>,
    /// Variable number of the __closure parameter inside a lambda body (second pass).
    /// `u16::MAX` when not inside a capturing lambda.
    pub(crate) closure_param: u16,
    // maps fn-ref variable numbers to their closure record work variable numbers.
    pub(crate) closure_vars: std::collections::HashMap<u16, u16>,
    // last closure work variable created by emit_lambda_code (transient).
    pub(crate) last_closure_work_var: u16,
    // closure allocation expression to inject at the call site.
    pub(crate) last_closure_alloc: Option<Box<Value>>,
    // outer variable numbers captured by the most recently parsed lambda.
    // Consumed by try_fn_ref_call to mark them as read at call-injection time.
    pub(crate) last_closure_captured_vars: Vec<u16>,
    /// #91: when > 0, record $.<field> accesses for circular-init detection.
    /// Decremented after each init(expr) is parsed.
    pub(crate) init_field_tracking: bool,
    /// #91: field names accessed via $ during the current init(expr) parse.
    pub(crate) init_field_deps: Vec<String>,
    /// M11-a: true while parsing the body of a `for … par(…) { … }` loop.
    /// `yield` inside a `par()` body is illegal — the worker runs in a separate
    /// thread with its own store; there is no safe coroutine resumption path.
    pub(crate) in_par_body: bool,
    /// Field-capture aliases created by `if expr is Variant { field } { body }`.
    /// Drained by `parse_if` after the body to restore previous name mappings.
    pub(crate) is_capture_aliases: Vec<(String, Option<u16>)>,
    /// Post-2c: captures the most recently parsed `as <alias>` cast target's
    /// def_nr when the alias has a `size(N)` annotation.  Consumed by
    /// `append_to_file` so that `f += x as i32` narrows the serialised
    /// payload to the alias's byte width.  Reset to `u32::MAX` at the start
    /// of each top-level statement; irrelevant outside file-I/O `+=`.
    pub(crate) last_cast_alias: u32,
    /// Field-binding Set nodes created by `if expr is Variant { field }`.
    /// Drained by `parse_if` and prepended to the if-body so they only
    /// execute when the discriminant matches.
    pub(crate) is_capture_bindings: Vec<Value>,
    /// `--show-types --trace`: when `true`, `parse_part` appends one
    /// trace line per resolved sub-expression (after each `.field`,
    /// `.tuple_idx`, `[idx]` step).  Surfaces dep-tracking flow that
    /// the per-variable view misses — e.g. P197 was a missing dep on
    /// the `.0` step of a tuple field read.
    pub trace_types: bool,
    /// Accumulated trace entries; drained by the introspection CLI.
    /// Each entry is a tab-separated record:
    /// `<fn_name>\t<line>:<col>\t<step_kind>\t<type_with_deps>`.
    pub trace_types_lines: Vec<String>,
}

// Operators ordered on their precedence
static OPERATORS: &[&[&str]] = &[
    &["??"],
    &["||", "or"],
    &["&&", "and"],
    &["==", "!=", "<", "<=", ">", ">="],
    &["|"],
    &["^"],
    &["&"],
    &["<<", ">>"],
    &["-", "+"],
    &["*", "/", "%"],
    &["**"],
    &["as"],
];

static SKIP_TOKEN: [&str; 8] = ["}", ".", "<", ">", "^", "+", "-", "#"];
static SKIP_WIDTH: [&str; 10] = ["}", ".", "x", "X", "o", "b", "e", "j", "d", "f"];

pub(crate) struct OutputState<'a> {
    pub(crate) radix: i32,
    pub(crate) width: Value,
    pub(crate) token: &'a str,
    pub(crate) plus: bool,
    pub(crate) note: bool,
    pub(crate) dir: i32,
    pub(crate) float: bool,
}

impl OutputState<'_> {
    pub(crate) fn db_format(&self) -> i32 {
        i32::from(self.note) + if self.radix < 0 { 2 } else { 0 }
    }
}

pub(crate) const OUTPUT_DEFAULT: OutputState = OutputState {
    radix: 10,
    width: Value::Int(0),
    token: " ",
    plus: false,
    note: false,
    dir: 2, // 2 = unset; text defaults to left (-1), numbers to right (1)
    float: false,
};

// Sub-modules
pub(super) mod builtins;
pub(super) mod collections;
pub(super) mod control;
pub(super) mod definitions;
pub(super) mod expressions;
pub(super) mod fields;
pub(super) mod objects;
pub(super) mod operators;
pub(super) mod vectors;

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

fn is_op(name: &str) -> bool {
    name.len() >= 3 && name.starts_with("Op") && name.chars().nth(2).unwrap().is_uppercase()
}

/// Validate function, attribute, value, and field names
fn is_lower(name: &str) -> bool {
    for c in name.chars() {
        if c.is_uppercase() {
            return false;
        }
    }
    true
}

#[allow(dead_code)]
/// Used to validate constant names
fn is_upper(name: &str) -> bool {
    for c in name.chars() {
        if c.is_lowercase() {
            return false;
        }
    }
    true
}

/// Validate type, enum, enum values and struct names
fn is_camel(name: &str) -> bool {
    let c = name.chars().next().unwrap();
    if c.is_lowercase() {
        return false;
    }
    for c in name.chars() {
        if c == '_' {
            return false;
        }
    }
    true
}

impl Parser {
    #[must_use]
    pub fn new() -> Self {
        let mut data = Data::new();
        // Register internal-only functions (i_ prefix) that are never visible to user code.
        // These are resolved by the compiler via data.def_nr("i_...") and mapped to native
        // Rust implementations in native.rs.
        let pos = Position {
            file: String::new(),
            line: 0,
            pos: 0,
        };
        let d = data.add_def("i_parse_errors", &pos, DefType::Function);
        data.definitions[d as usize].returned = Type::Text(Vec::new());
        let d = data.add_def("i_parse_error_push", &pos, DefType::Function);
        data.definitions[d as usize].returned = Type::Void;
        {
            let mut lexer = Lexer::default();
            data.add_attribute(&mut lexer, d, "msg", Type::Text(Vec::new()));
        }
        Parser {
            todo_files: Vec::new(),
            data,
            database: Stores::new(),
            lexer: Lexer::default(),
            in_loop: false,
            in_format_expr: false,
            file: 1,
            diagnostics: Diagnostics::new(),
            default: false,
            context: u32::MAX,
            first_pass: true,
            reverse_iterator: false,
            last_range_from: None,
            last_range_till: None,
            vars: Function::new("", "none"),
            line: 0,
            lib_dirs: Vec::new(),
            pending_native_libs: Vec::new(),
            pending_pkg_deps: Vec::new(),
            pending_imports: Vec::new(),
            applied_imports: Vec::new(),
            deferred_unknown: Vec::new(),
            expr_not_null: false,
            expr_not_null_name: String::new(),
            lambda_counter: 0,
            lambda_hint: Type::Unknown(0),
            fields_of: u32::MAX,
            capture_context: Vec::new(),
            captured_names: Vec::new(),
            closure_param: u16::MAX,
            closure_vars: std::collections::HashMap::new(),
            last_closure_work_var: u16::MAX,
            last_closure_alloc: None,
            last_closure_captured_vars: vec![],
            init_field_tracking: false,
            init_field_deps: Vec::new(),
            in_par_body: false,
            is_capture_aliases: Vec::new(),
            is_capture_bindings: Vec::new(),
            last_cast_alias: u32::MAX,
            trace_types: false,
            trace_types_lines: Vec::new(),
        }
    }

    /// Parse the content of a given file.
    /// - filename: the file to parse
    /// - default: parsing system definitions
    /// # Panics
    /// With filesystem problems.
    pub fn parse(&mut self, filename: &str, default: bool) -> bool {
        // under the `wasm` feature, check VIRT_FS before trying the real filesystem.
        #[cfg(feature = "wasm")]
        if let Some(content) = crate::wasm::virt_fs_get(filename) {
            return self.parse_virtual(&content, filename, default);
        }
        self.default = default;
        self.vars.logging = false;
        self.lexer.switch(filename);
        self.first_pass = true;
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.parse_file();
        self.resolve_deferred_unknowns();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            self.reverse_iterator = false;
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.data.reset();
            self.lambda_counter = 0;
            self.lexer.switch(filename);
            self.parse_file();
            self.resolve_deferred_unknowns();
        }
        self.backfill_native_symbol_crates();
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// after `parse_file` has run (and all `todo_files` have drained,
    /// so every file in the recursion has had its definitions registered),
    /// reconcile any `DefType::Unknown` stubs that `actual_types_deferred`
    /// collected during parsing.
    ///
    /// The cyclic `use` case: file B references a type `Player` defined in
    /// file A, but B's `use A;` fires while A is suspended mid-parse — so
    /// B's body parsed with `Player` as a stub.  After the full recursion
    /// returns, A's `Player` is registered; Phase C re-applies imports with
    /// overwrite semantics (replacing B's stub binding with A's real def),
    /// then rewrites every `Type::Unknown(stub_nr)` occurrence to the real
    /// resolved type.
    ///
    /// Stubs that remain unresolved after this reconciliation surface as
    /// the original "Undefined type" error at the stored `Position`.
    fn resolve_deferred_unknowns(&mut self) {
        // Step 1: re-apply all previously-applied imports with overwrite
        // semantics.  This replaces any target-source `Unknown` stub with
        // the now-registered real def in the library source.
        let applied = std::mem::take(&mut self.applied_imports);
        for pi in &applied {
            match &pi.spec {
                ImportSpec::Wildcard => {
                    self.data.import_all_overwrite(pi.lib_source, pi.for_source);
                }
                ImportSpec::Names(names) => {
                    for name in names {
                        self.data
                            .import_name_overwrite(pi.lib_source, pi.for_source, name);
                    }
                }
            }
        }
        // Keep them on the list for any later pass (pass 2 re-populates).
        self.applied_imports = applied;

        // Step 2: for each deferred stub, resolve via the post-import
        // def binding.  Three outcomes per stub:
        //
        //  (a) The stub def got UPGRADED in-place to a real type (most
        //      common — `parse_struct` does this when it finds an
        //      existing stub by name).  `def(stub_nr).def_type` is no
        //      longer `Unknown`; call `rewrite_unknown_refs(stub, stub)`
        //      so that `Type::Unknown(stub)` references resolve to
        //      `def(stub).returned`.
        //
        //  (b) The stub's source has a DIFFERENT real def (e.g. when
        //      `import_all_overwrite` just routed the source-level
        //      binding to a real def from another source).  Rewrite
        //      Unknown references to point at that real def.
        //
        //  (c) Still unresolved — emit the "Undefined type" error at
        //      the stored `Position`.
        let deferred = std::mem::take(&mut self.deferred_unknown);
        for (source, stub_nr, pos) in deferred {
            let stub_name = self.data.def(stub_nr).name.clone();
            // Case (a): stub upgraded in place
            if !matches!(self.data.def(stub_nr).def_type, DefType::Unknown) {
                self.data.rewrite_unknown_refs(stub_nr, stub_nr);
                continue;
            }
            // Case (b): lookup via post-import source binding
            let resolved_nr = self.data.source_nr(source, &stub_name);
            if resolved_nr != u32::MAX
                && resolved_nr != stub_nr
                && !matches!(self.data.def(resolved_nr).def_type, DefType::Unknown)
            {
                self.data.rewrite_unknown_refs(stub_nr, resolved_nr);
                continue;
            }
            // Case (c): emit the deferred error
            let msg = if stub_name == "string" {
                "Undefined type 'string' — did you mean 'text'?".to_string()
            } else if let Some(s) = self.data.suggest_type_name(&stub_name) {
                format!("Undefined type {stub_name} — did you mean '{s}'?")
            } else {
                format!("Undefined type {stub_name}")
            };
            self.lexer.pos_diagnostic(Level::Error, &pos, &msg);
        }
    }

    /// After both parse passes, every `#native "<sym>"` annotation should map
    /// to its owning native package crate in `native_symbol_crates`.  If the
    /// manifest was registered before the .loft source that declared the
    /// native symbol was parsed, the original mapping pass in
    /// `lib_path_manifest` / `register_native_manifest` saw no definitions
    /// and left the symbol unmapped — which later surfaces as a `todo!()`
    /// stub in the `--native` output and a runtime panic.
    ///
    /// Walk every definition once more: if it has a `#native` symbol not in
    /// the map and exactly one native package is registered, bind the symbol
    /// to that package.  With multiple packages we conservatively skip — the
    /// original per-manifest passes have already matched the first-seen
    /// symbols to their owners.
    fn backfill_native_symbol_crates(&mut self) {
        if self.data.native_packages.len() != 1 {
            return;
        }
        let rust_crate = self.data.native_packages[0].0.replace('-', "_");
        for d_nr in 0..self.data.definitions() {
            let sym = self.data.def(d_nr).native.clone();
            if !sym.is_empty() && !self.data.native_symbol_crates.contains_key(&sym) {
                self.data
                    .native_symbol_crates
                    .insert(sym, rust_crate.clone());
            }
        }
    }

    /// Parse `content` as if it were the file at `filename`.
    /// Used by the WASM virtual-FS path to bypass real filesystem access.
    #[cfg(feature = "wasm")]
    fn parse_virtual(&mut self, content: &str, filename: &str, default: bool) -> bool {
        self.default = default;
        self.vars.logging = false;
        self.first_pass = true;
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.lexer.parse_string(content, filename);
        self.parse_file();
        self.resolve_deferred_unknowns();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.data.reset();
            self.lambda_counter = 0;
            self.lexer.parse_string(content, filename);
            self.parse_file();
            self.resolve_deferred_unknowns();
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// Parse all .loft files found in a directory tree in alphabetical ordering.
    /// # Errors
    /// With filesystem problems.
    pub fn parse_dir(&mut self, dir: &str, default: bool, debug: bool) -> std::io::Result<()> {
        let paths = read_dir(dir)?;
        let mut files: BTreeSet<String> = BTreeSet::new();
        for path in paths {
            let p = path?;
            let own_file = p
                .path()
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"));
            let file_name = p.path().to_string_lossy().to_string();
            let data = metadata(&file_name)?;
            if own_file || data.is_dir() {
                files.insert(file_name);
            }
        }
        for f in files {
            let types = self.database.types.len();
            let from = self.data.definitions();
            let data = metadata(&f)?;
            if data.is_dir() {
                self.parse_dir(&f, default, debug)?;
            } else if !self.parse(&f, default) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{}", self.diagnostics),
                ));
            }
            scopes::check(&mut self.data);
            if debug {
                self.output(&f, types, from)?;
            }
        }
        Ok(())
    }

    fn output(&mut self, f: &str, types: usize, from: u32) -> std::io::Result<()> {
        let f_norm = f.replace(other_sep(), sep_str());
        let file = f_norm.rsplit(sep()).next().unwrap_or(f);
        let to = format!("tests/dumps/{file}.txt");
        let _ = std::fs::create_dir_all("tests/dumps");
        if let Ok(mut w) = File::create(to.clone()) {
            let to = self.database.types.len();
            for tp in types..to {
                writeln!(w, "Type {tp}:{}", self.database.show_type(tp as u16, true))?;
            }
            for d_nr in from..self.data.definitions() {
                if self.data.def(d_nr).code == Value::Null {
                    continue;
                }
                write!(w, "{} ", self.data.def(d_nr).header(&self.data, d_nr))?;
                let mut vars = Function::copy(&self.data.def(d_nr).variables);
                self.data
                    .show_code(&mut w, &mut vars, &self.data.def(d_nr).code, 0, false)?;
                writeln!(w, "\n")?;
            }
        } else {
            diagnostic!(self.lexer, Level::Error, "Could not write: {to}");
        }
        Ok(())
    }

    /// Only parse a specific string, only useful for parser tests.
    #[allow(dead_code)]
    pub fn parse_str(&mut self, text: &str, filename: &str, logging: bool) {
        self.first_pass = true;
        self.default = false;
        self.vars.logging = logging;
        self.lexer.parse_string(text, filename);
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.parse_file();
        self.resolve_deferred_unknowns();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            self.diagnostics.fill(self.lexer.diagnostics());
            return;
        }
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.lexer.parse_string(text, filename);
        self.first_pass = false;
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.diagnostics.fill(self.lexer.diagnostics());
    }

    // ********************
    // * Helper functions *
    // ********************

    /// canonical entry point for building a vector
    /// database type from a content `Type`.  Consults
    /// `Data::narrow_vector_content` first (single source of truth
    /// for narrow-detection; shared with `typedef.rs::fill_database`
    /// for struct fields).  Falls back to the content's own
    /// `known_type` when narrow doesn't apply, or to the default
    /// `integer` slot (0) when the content has no registered type
    /// yet.  Every `database.vector(...)` call in `src/parser/`
    /// should route through this helper so locals, parameters,
    /// returns, and literals get the same narrow storage that
    /// struct fields get via fill_database.
    pub(crate) fn vector_of(&mut self, content: &Type) -> u16 {
        if let Some(narrow) = self.data.narrow_vector_content(content, &mut self.database) {
            return self.database.vector(narrow);
        }
        let c_nr = self.data.type_elm(content);
        if c_nr == u32::MAX {
            return self.database.vector(u16::MAX);
        }
        let c_tp = self.data.def(c_nr).known_type;
        // Known_type may be unset for forward references; fall back to
        // the default integer slot (0) so the vector type still
        // registers correctly.  The content's own fill pass will
        // update once it runs.
        let resolved = if c_tp == u16::MAX { 0 } else { c_tp };
        self.database.vector(resolved)
    }

    /// Get an iterator.
    /// The iterable expression is in *code.
    /// Creating the iterator will be in *code afterward.
    /// Return the next expression; with `Value::None` the iterator creation was impossible.
    fn convert(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        if is_type.is_equal(should) {
            return true;
        }
        // Never (return/break/continue) is compatible with any type.
        if matches!(is_type, Type::Never) {
            return true;
        }
        let _ = code;
        // Struct-literal inline constructors are typed as Rewritten(Reference(...)); strip
        // the wrapper so method calls chained on the constructor are accepted correctly.
        if let Type::Rewritten(inner) = is_type {
            return self.convert(code, inner, should);
        }
        if let Type::Rewritten(inner) = should {
            return self.convert(code, is_type, inner);
        }
        // Plan-06 phase 4d: tuple-to-tuple convert is element-wise.
        // Without this, a value with a `Rewritten(Reference)` element
        // (e.g. `(Inner { … }, 11)` from inline struct construction)
        // fails to match a field declared as `(Inner, integer)` even
        // though the underlying types are compatible.  We don't
        // mutate `code` here — element conversions are by-shape only.
        if let (Type::Tuple(src_elems), Type::Tuple(dst_elems)) = (is_type, should)
            && src_elems.len() == dst_elems.len()
        {
            let mut all_compatible = true;
            for (s, d) in src_elems.iter().zip(dst_elems.iter()) {
                let mut placeholder = Value::Null;
                if !self.convert(&mut placeholder, s, d) {
                    all_compatible = false;
                    break;
                }
            }
            if all_compatible {
                return true;
            }
        }
        if let (Type::Reference(ref_tp, _), Type::Enum(enum_tp, true, _)) = (is_type, should) {
            for a in &self.data.def(*enum_tp).attributes {
                if a.name == self.data.def(*ref_tp).name {
                    return true;
                }
            }
        }
        if let Type::RefVar(ref_tp) = is_type
            && self.convert(code, ref_tp, should)
        {
            return true;
        }
        if let Type::RefVar(ref_tp) = should
            && ref_tp.is_equal(is_type)
        {
            if matches!(**ref_tp, Type::Text(_)) {
                // Text → &text: use OpCreateStack for plain variables (write-back),
                // allocate a work-text copy for complex expressions (read-only).
                let orig = std::mem::replace(code, Value::Null);
                if let Value::Var(_) = &orig {
                    *code = self.cl("OpCreateStack", &[orig]);
                } else {
                    let wv = self.vars.work_text(&mut self.lexer);
                    let mut ls = Vec::new();
                    if orig != Value::Text(String::new()) {
                        ls.push(self.cl("OpAppendText", &[Value::Var(wv), orig]));
                    }
                    ls.push(self.cl("OpCreateStack", &[Value::Var(wv)]));
                    *code = v_block(
                        ls,
                        Type::Reference(self.data.def_nr("reference"), vec![wv]),
                        "text_ref",
                    );
                }
            } else {
                let orig = std::mem::replace(code, Value::Null);
                if matches!(orig, Value::Var(_)) {
                    *code = self.cl("OpCreateStack", &[orig]);
                } else {
                    // produce a `Value::Insert` so that scope
                    // analysis (`scopes::scan_args`) hoists the
                    // pre-call Set into the enclosing statement list.
                    // Insert does not form a scope, so the work-ref
                    // lives at function scope and its slot survives
                    // the call.  Using `v_block` instead would create
                    // a block scope whose exit FreeStack clobbers the
                    // ref-target bytes and corrupts preceding args.
                    //
                    // The work-ref holds only a COPY of an existing
                    // DbRef — it does not own a store — so tell
                    // scopes to suppress the scope-exit `OpFreeRef`.
                    // Without `skip_free`, the shared store would
                    // be decremented once per call and eventually
                    // reach ref_count 0, dangling the caller's
                    // owning reference across loop iterations.
                    let wv = self.vars.work_refs(is_type, &mut self.lexer);
                    self.vars.set_skip_free(wv);
                    *code = Value::Insert(vec![
                        v_set(wv, orig),
                        self.cl("OpCreateStack", &[Value::Var(wv)]),
                    ]);
                }
            }
            return true;
        }
        let mut check_type = is_type;
        let r = Type::Reference(self.data.def_nr("reference"), Vec::new());
        let e = Type::Enum(0, false, Vec::new());
        if let Type::Vector(_nr, _) = is_type {
            if let Type::Vector(v, _) = should
                && v.is_unknown()
            {
                return true;
            }
        } else if let Type::Reference(_, _) = is_type {
            if matches!(*should, Type::Reference(0, _)) {
                return true;
            }
            check_type = &r;
        } else if let Type::Enum(_, false, _) = is_type {
            if *should == e {
                return true;
            }
            check_type = &e;
        }
        for &dnr in self.data.get_possible("OpConv", &self.lexer) {
            if self.data.def(dnr).name.ends_with("FromNull") {
                if *is_type == Type::Null {
                    if matches!(self.data.def(dnr).returned, Type::Reference(_, _))
                        && let Type::Reference(_, _) = *should
                    {
                        // Use the non-allocating sentinel instead of OpConvRefFromNull so that
                        // null comparisons (`s == null`, `s != null`) do not leak a store.
                        let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
                        *code = Value::Call(sentinel_nr, vec![]);
                        return true;
                    } else if self.data.def(dnr).returned == *should {
                        *code = Value::Call(dnr, vec![]);
                        return true;
                    }
                }
            } else if self.data.attributes(dnr) > 0
                && self.data.attr_type(dnr, 0).is_equal(check_type)
                && self.data.def(dnr).returned == *should
            {
                *code = Value::Call(dnr, vec![code.clone()]);
                return true;
            }
        }
        false
    }

    /// Cast a type to another type when possible
    /// Returns false when impossible.
    fn cast(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        if self.first_pass {
            return true;
        }
        let mut should_nr = self.data.type_def_nr(should);
        if let Type::Vector(c_tp, _) = should {
            let c_nr = self.data.type_def_nr(c_tp);
            // route through `vector_of` so narrow aliases
            // (vector<i32>, vector<u8>) get the correct narrow element
            // type — matching what fill_database registers for struct
            // fields.
            let tp = self.vector_of(c_tp);
            should_nr = self.data.check_vector(c_nr, tp, self.lexer.pos());
        }
        let should_kt = if should_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(should_nr).known_type
        };
        let is_nr = self.data.type_def_nr(is_type);
        let is_kt = if is_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(is_nr).known_type
        };
        if let Type::Reference(tp, _) = should
            && self.data.def(*tp).returned.is_equal(is_type)
            && matches!(is_type, Type::Enum(_, true, _))
        {
            let get_e = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
            let get = self.cl("OpConvIntFromEnum", &[get_e]);
            if let Value::Enum(nr, _) = self.data.def(*tp).attributes[0].value {
                *code = v_if(
                    self.cl("OpEqInt", &[get, Value::Int(i32::from(nr))]),
                    code.clone(),
                    self.cl("OpConvRefFromNull", &[]),
                );
            }
            return true;
        }
        if matches!(is_type, Type::Text(_))
            && matches!(should, Type::Enum(_, true, _) | Type::Reference(_, _))
        {
            *code = self.cl(
                "OpCastVectorFromText",
                &[code.clone(), Value::Int(i32::from(should_kt))],
            );
            return true;
        }
        for &dnr in self.data.get_possible("OpCast", &self.lexer) {
            if self.data.attributes(dnr) == 1
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned.is_same(should)
            {
                if let Type::Enum(tp, false, _) = should {
                    *code = Value::Call(
                        dnr,
                        vec![
                            code.clone(),
                            Value::Int(i32::from(self.data.def(*tp).known_type)),
                        ],
                    );
                } else {
                    *code = Value::Call(dnr, vec![code.clone()]);
                }
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned.is_same(should)
                && should_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(should_kt))]);
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned.is_same(should)
                && is_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(is_kt))]);
                return true;
            }
        }
        false
    }

    /// Validate that two types are equal
    fn can_convert(&mut self, test_type: &Type, should: &Type) -> bool {
        if *test_type != *should && !test_type.is_unknown() {
            if let Type::RefVar(tp) = should
                && tp.is_equal(test_type)
            {
                return true;
            }
            if let (Type::Enum(_e, _, _), Type::Enum(o, _, _)) = (test_type, should)
                && self.data.def(*o).name == "enumerate"
            {
                return true;
            }
            if let (Type::Reference(r_nr, _), Type::Enum(e_nr, true, _)) = (test_type, should)
                && e_nr == r_nr
            {
                return true;
            }
            if let (Type::Enum(t, false, _), Type::Enum(s, false, _)) = (test_type, should)
                && *t == *s
            {
                return true;
            }
            if let (Type::Enum(_, false, _), Type::Integer(_)) = (test_type, should) {
                return true;
            }
            if let Type::Reference(r, _) = should
                && *r == self.data.def_nr("reference")
                && let Type::Reference(_, _) = test_type
                && self.generic_type_name(test_type).is_none()
            {
                return true;
            }
            // Bare collection parameter (sorted / hash / index / spacial)
            // accepts the corresponding parameterised collection
            // argument.  Mirrors how `Type::Vector(_, _)` matches via
            // is_same: the parameter type carries no element-type
            // constraint, so any concrete instantiation is structurally
            // compatible.  Used for stdlib helpers like `len(both: sorted)`.
            if let Type::Reference(r, _) = should {
                let r = *r;
                let bare = (r == self.data.def_nr("sorted")
                    && matches!(test_type, Type::Sorted(_, _, _)))
                    || (r == self.data.def_nr("hash") && matches!(test_type, Type::Hash(_, _, _)))
                    || (r == self.data.def_nr("index")
                        && matches!(test_type, Type::Index(_, _, _)))
                    || (r == self.data.def_nr("spacial")
                        && matches!(test_type, Type::Spacial(_, _, _)));
                if bare {
                    return true;
                }
            }
            // Text types with different dep lists are structurally compatible.
            if matches!((test_type, should), (Type::Text(_), Type::Text(_))) {
                return true;
            }
            // Function types with compatible params and return type.
            if let (Type::Function(tp, tr, _), Type::Function(sp, sr, _)) = (test_type, should)
                && tp.len() == sp.len()
                && tp.iter().zip(sp.iter()).all(|(a, b)| a.is_equal(b))
                && tr.is_equal(sr)
            {
                return true;
            }
            false
        } else {
            true
        }
    }

    fn validate_convert(&mut self, context: &str, test_type: &Type, should: &Type) {
        if !self.first_pass && !self.can_convert(test_type, should) {
            let res = self.lexer.peek();
            // Plan-07 phase 6 (partial) — "expected E, got G on context"
            // reads the same direction as English ("we expected this,
            // we got that"); the old shape "G should be E on context"
            // forced a mental flip and confused users new to the
            // language.
            specific!(
                &mut self.lexer,
                &res,
                Level::Error,
                "expected {}, got {} on {context}",
                should.name(&self.data),
                test_type.name(&self.data)
            );
        }
    }

    /// Check if a type is a generic type variable (a dummy struct used as T).
    /// Returns the type variable name if it is, None otherwise.
    pub(crate) fn generic_type_name(&self, tp: &Type) -> Option<&str> {
        if let Type::Reference(d, _) = tp {
            let d = *d as usize;
            if d < self.data.definitions.len()
                && self.data.definitions[d].def_type == DefType::Struct
                && self.data.definitions[d].attributes.is_empty()
                && self.context != u32::MAX
                && self.data.definitions[self.context as usize].def_type == DefType::Generic
            {
                return Some(&self.data.definitions[d].name);
            }
        }
        None
    }

    /// Check whether the current generic function's bounds include an interface that
    /// declares the given method.  Returns false if not inside a generic or if no bound
    /// declares the method.
    pub(crate) fn has_bound_for_method(&self, method: &str) -> bool {
        if self.context == u32::MAX {
            return false;
        }
        let bounds = &self.data.definitions[self.context as usize].bounds;
        for &iface_nr in bounds {
            for child_nr in self.data.children_of(iface_nr) {
                let name = &self.data.def(child_nr).name;
                // Interface stubs use "__iface_{d_nr}_{method}" naming
                if let Some(rest) = name.strip_prefix("__iface_")
                    && let Some((_, m)) = rest.split_once('_')
                    && m == method
                {
                    return true;
                }
            }
        }
        false
    }

    /// Search for definitions with the given name and call that with the given parameters.
    fn call(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
        named_args: &[(String, Value, Type)],
    ) -> Type {
        // Create a new list of parameters based on the current ones
        // We still need to know the types.
        let mut d_nr = if self.default && is_op(name) {
            self.data.def_nr(name)
        } else {
            self.data.find_fn(
                source,
                name,
                if types.is_empty() || types[0] == Type::Null {
                    &Type::Unknown(0)
                } else {
                    &types[0]
                },
            )
        };
        // Trace point: post-find_fn dispatch state.  Captures the most
        // common debugging vantage — what name resolved to which
        // d_nr, whether it was a Generic that got skipped, and which
        // pass we're on.  Enable with `LOFT_TRACE=call`.
        crate::loft_trace!(
            call,
            "name={} types={:?} d_nr={} def_type={:?} first_pass={}",
            name,
            types,
            d_nr,
            if d_nr == u32::MAX {
                None
            } else {
                Some(self.data.def(d_nr).def_type.clone())
            },
            self.first_pass,
        );
        // skip generic templates — they are not callable directly.
        if d_nr != u32::MAX && self.data.def(d_nr).def_type == DefType::Generic {
            d_nr = u32::MAX;
        }
        // Plan-17 phase 01 (A) — propagate the substituted return type
        // on first pass so receiving variables (`t = min_max(7, 3)`)
        // get a correctly-typed `Type::Tuple([…])` slot, enabling
        // `t.0` / `t.1` on the SAME pass.  Without this, first-pass
        // returned `Type::Unknown(0)`; the receiving variable stayed
        // Unknown (change_var_type is a no-op for Unknown); downstream
        // `t.0` rejected with "Expect token ;" because tuple element
        // access requires a typed Tuple receiver.  That error aborted
        // second pass entirely (lexer.token() emits errors regardless
        // of pass), so the second-pass full instantiation never ran.
        //
        // The fix splits the work: on first pass, we predict the
        // return type only (no def creation — first-pass body IR is
        // still being built and would produce a stale instantiation);
        // on second pass, the full `try_generic_instantiation`
        // creates the monomorphised def.  First-pass IR for the call
        // is `Value::Null` placeholder; second-pass re-parse builds
        // the real `Value::Call`.
        if d_nr == u32::MAX && !self.default {
            if self.first_pass {
                let predicted = self.predict_generic_return_type(name, types);
                if !predicted.is_unknown() {
                    *code = Value::Null;
                    return predicted;
                }
            } else {
                d_nr = self.try_generic_instantiation(name, types);
            }
        }
        if d_nr != u32::MAX {
            self.call_with_named(code, d_nr, list, types, named_args, true)
        } else if self.first_pass && !self.default {
            Type::Unknown(0)
        } else if name == "len"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(types[0], Type::Index(_, _, _))
        {
            // P192: `len(ix)` for `ix: index<T[key]>`.  Dispatched
            // here (not via stdlib overload) because the runtime
            // helper `tree::count` needs the per-record bookkeeping
            // byte offset, which is `database.fields(tp)` — only
            // computable at parse time once the type is registered.
            //
            // Strict arity / named-arg gates: `len(ix, x)` or
            // `len(ix, key: 1)` should NOT route here — they would
            // fall through standard dispatch to the existing
            // `Unknown function len` error message which lists the
            // method-style alternative.
            let known = self.get_type(&types[0]);
            let op_d_nr = self.data.def_nr("OpLengthIndex");
            if known != u16::MAX && op_d_nr != u32::MAX {
                let fields = self.database.fields(known);
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(fields)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            // Type or op not registered — drop to the standard
            // error path so the user sees the same diagnostic shape
            // they get for any other unresolved `len()` call.  P07.5
            // adds a "did you mean" suffix when a similarly-named
            // user function exists.
            if let Some(s) = self.suggest_function_name(name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown function {name} — did you mean '{s}'?"
                );
            } else {
                diagnostic!(self.lexer, Level::Error, "Unknown function {name}");
            }
            Type::Unknown(0)
        } else {
            // generic-specific error for method calls on T.
            if let Some(tv_name) = types.first().and_then(|t| self.generic_type_name(t)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "generic type {tv_name}: method call requires a concrete type",
                );
            } else {
                // QUALITY 6c (follow-on): when a free call fails but a method
                // `t_<LEN><Type>_<name>` exists on some other type, tell the
                // user to call it as a method.  Mirror image of the
                // field-access hint that covers the method→free direction.
                // P07.5: when no method receiver is found EITHER, fall back to
                // a similar-name suggestion across all user functions.
                let method_types = self.find_method_receivers(name);
                if method_types.is_empty() {
                    if let Some(s) = self.suggest_function_name(name) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown function {name} — did you mean '{s}'?"
                        );
                    } else {
                        diagnostic!(self.lexer, Level::Error, "Unknown function {name}");
                    }
                } else {
                    let receivers = method_types.join(" / ");
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown function {name} — did you mean the method `x.{name}(…)` on {receivers}? (stdlib declared `{name}` as a method; see LOFT.md § Methods and function calls)"
                    );
                }
            }
            Type::Unknown(0)
        }
    }

    /// Scan all definitions for methods named `name` (encoded as
    /// `t_<LEN><TypeName>_<name>`) and return the list of receiver type
    /// names in definition order, de-duplicated.  Powers the 6c
    /// free→method hint in `call`.
    fn find_method_receivers(&self, name: &str) -> Vec<String> {
        let suffix = format!("_{name}");
        let mut receivers: Vec<String> = Vec::new();
        for d_nr in 0..self.data.definitions() {
            let def_name = &self.data.def(d_nr).name;
            let Some(rest) = def_name.strip_prefix("t_") else {
                continue;
            };
            if !rest.ends_with(&suffix) {
                continue;
            }
            let digit_end = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digit_end == 0 {
                continue;
            }
            let Ok(type_len) = rest[..digit_end].parse::<usize>() else {
                continue;
            };
            let type_start = digit_end;
            let Some(type_end) = type_start.checked_add(type_len) else {
                continue;
            };
            if rest.len() != type_end + suffix.len() || !rest.is_char_boundary(type_end) {
                continue;
            }
            let type_name = &rest[type_start..type_end];
            if !type_name.is_empty() && !receivers.iter().any(|t| t == type_name) {
                receivers.push(type_name.to_string());
            }
        }
        receivers
    }

    /// Plan-17 phase 01 (A) — predict the substituted return type of a
    /// generic call WITHOUT instantiating the def.  Used on first pass
    /// so the receiving variable's type is set correctly for downstream
    /// inference (e.g. `t = min_max(7, 3)` followed by `t.0`).  Returns
    /// `Type::Unknown(0)` if no prediction is possible (forward decl,
    /// unresolvable type variable, etc.) — caller falls back to the
    /// existing first-pass-Unknown path.
    ///
    /// Pure read of the generic template's already-populated `returned`
    /// field plus the type-substitution helper.  No state mutation;
    /// safe to call multiple times.
    fn predict_generic_return_type(&self, name: &str, types: &[Type]) -> Type {
        let generic_name = format!("n_{name}");
        let g_nr = self.data.def_nr(&generic_name);
        if g_nr == u32::MAX || self.data.def(g_nr).def_type != DefType::Generic {
            return Type::Unknown(0);
        }
        if types.is_empty() || types[0].is_unknown() {
            return Type::Unknown(0);
        }
        let tv_nr = Self::extract_type_var(&self.data.def(g_nr).attributes[0].typedef);
        if tv_nr == u32::MAX {
            return Type::Unknown(0);
        }
        let concrete =
            Self::resolve_type_var(&self.data.def(g_nr).attributes[0].typedef, tv_nr, &types[0]);
        if concrete.is_unknown() {
            return Type::Unknown(0);
        }
        let tmpl_returned = self.data.definitions[g_nr as usize].returned.clone();
        let predicted = Self::substitute_type(tmpl_returned, tv_nr, &concrete);
        // Trace point: predicted return type for first-pass type
        // inference of generic call sites.  Used during plan-17 (A)
        // debugging.  Enable with `LOFT_TRACE=generic`.
        crate::loft_trace!(
            generic,
            "predict name={} types={:?} concrete={:?} → {:?}",
            name,
            types,
            concrete,
            predicted,
        );
        predicted
    }

    /// Try to instantiate a generic function template for the given call-site types.
    /// Returns the `def_nr` of the instantiated function, or `u32::MAX` if no generic matches.
    fn try_generic_instantiation(&mut self, name: &str, types: &[Type]) -> u32 {
        let generic_name = format!("n_{name}");
        let g_nr = self.data.def_nr(&generic_name);
        if g_nr == u32::MAX || self.data.def(g_nr).def_type != DefType::Generic {
            return u32::MAX;
        }
        if types.is_empty() || types[0].is_unknown() {
            // First-pass argument types may be incomplete; defer the diagnostic
            // to second pass when types are stable.  Returning MAX here is the
            // same effect; it just doesn't emit a noisy first-pass error.
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot infer type for generic parameter — provide an explicit type annotation"
                );
            }
            return u32::MAX;
        }
        // Find the type variable def_nr and resolve the concrete type T maps to.
        let tv_nr = Self::extract_type_var(&self.data.def(g_nr).attributes[0].typedef);
        if tv_nr == u32::MAX {
            return u32::MAX;
        }
        let concrete =
            Self::resolve_type_var(&self.data.def(g_nr).attributes[0].typedef, tv_nr, &types[0]);
        if concrete.is_unknown() {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot resolve generic type parameter from argument type"
                );
            }
            return u32::MAX;
        }
        // Build the mangled name for the instantiated function.
        let type_nr = self.data.type_def_nr(&concrete);
        let mangled = if type_nr == u32::MAX {
            format!("n_{name}")
        } else {
            format!(
                "t_{}{}_{name}",
                self.data.def(type_nr).name.len(),
                self.data.def(type_nr).name
            )
        };
        // Return existing instantiation if already created.
        let existing = self.data.def_nr(&mangled);
        if existing != u32::MAX {
            return existing;
        }
        // Clone the template data before mutating self.data.
        let tmpl_code = self.data.definitions[g_nr as usize].code.clone();
        let tmpl_returned = self.data.definitions[g_nr as usize].returned.clone();
        let tmpl_attrs: Vec<_> = self.data.definitions[g_nr as usize]
            .attributes
            .iter()
            .map(|a| Argument {
                name: a.name.clone(),
                typedef: Self::substitute_type(a.typedef.clone(), tv_nr, &concrete),
                default: a.value.clone(),
                constant: false,
            })
            .collect();
        let tmpl_vars = self.data.definitions[g_nr as usize].variables.clone();
        let tmpl_pos = self.data.definitions[g_nr as usize].position.clone();
        let new_code = Self::substitute_type_in_value(tmpl_code, tv_nr, &concrete, &self.data);
        let new_returned = Self::substitute_type(tmpl_returned, tv_nr, &concrete);
        // Register the new definition.
        let d_nr = self.data.add_def(&mangled, &tmpl_pos, DefType::Function);
        for a in &tmpl_attrs {
            let a_nr = self
                .data
                .add_attribute(&mut self.lexer, d_nr, &a.name, a.typedef.clone());
            self.data.set_attr_value(d_nr, a_nr, a.default.clone());
        }
        self.data.definitions[d_nr as usize].code = new_code;
        self.data.set_returned(d_nr, new_returned.clone());
        // Trace point: full instantiation result.  Used during plan-17
        // (A) debugging when verifying that the second-pass def
        // creation produced the right monomorphised signature.
        // Enable with `LOFT_TRACE=generic`.
        crate::loft_trace!(
            generic,
            "instantiate name={} mangled={} d_nr={} concrete={:?} returned={:?}",
            name,
            mangled,
            d_nr,
            concrete,
            new_returned,
        );
        // Copy the variable table with substituted types.
        let mut vars = Function::copy(&tmpl_vars);
        vars.substitute_type(tv_nr, &concrete);
        self.data.definitions[d_nr as usize].variables = vars;
        // I6: verify the concrete type satisfies every declared bound.
        // Emit a diagnostic and return u32::MAX if any required method is missing.
        if !self.check_satisfaction(g_nr, type_nr) {
            // Return d_nr (not u32::MAX) so `call` doesn't emit a redundant
            // "Unknown function" error — the satisfaction error is sufficient.
            // The function won't execute because parsing will halt on errors.
        }
        d_nr
    }

    /// I6: Check that the concrete type (identified by `concrete_nr`) implements every
    /// interface in `g_nr`'s bounds.  Returns `true` if satisfied (or no bounds),
    /// `false` and emits a diagnostic for the first missing method otherwise.
    fn check_satisfaction(&mut self, g_nr: u32, concrete_nr: u32) -> bool {
        let bounds = self.data.definitions[g_nr as usize].bounds.clone();
        if bounds.is_empty() {
            return true;
        }
        if concrete_nr == u32::MAX {
            return true; // can't check without a concrete type def_nr
        }
        let concrete_name = self.data.def(concrete_nr).name.clone();
        let mut satisfied = true;
        for iface_nr in bounds {
            let iface_name = self.data.def(iface_nr).name.clone();
            let children: Vec<u32> = self.data.children_of(iface_nr).collect();
            for child_nr in children {
                let child_name = self.data.def(child_nr).name.clone();
                // Extract method name from "__iface_{d_nr}_{method}" or legacy "t_4Self_{method}"
                let self_prefix = format!("t_{}Self_", "Self".len());
                let method_suffix = if let Some(rest) = child_name.strip_prefix("__iface_") {
                    rest.split_once('_')
                        .map_or(rest.to_string(), |(_, m)| m.to_string())
                } else if child_name.starts_with(&self_prefix) {
                    child_name[self_prefix.len()..].to_string()
                } else {
                    child_name.clone()
                };
                // I9-prim: use find_fn which checks both the method-style convention
                // (t_7integer_OpLt) and the add_op convention (OpLtInt via possible map).
                let concrete_type = self.data.def(concrete_nr).returned.clone();
                let found = self.data.find_fn(u16::MAX, &method_suffix, &concrete_type);
                if found == u32::MAX {
                    let msg = crate::diagnostics::diagnostic_format(
                        Level::Error,
                        format_args!(
                            "'{concrete_name}' does not satisfy interface '{iface_name}': missing {method_suffix}",
                        ),
                    );
                    let peek_pos = self.lexer.peek().position.clone();
                    self.lexer.pos_diagnostic(Level::Error, &peek_pos, &msg);
                    satisfied = false;
                }
            }
        }
        satisfied
    }

    /// Extract the type variable `def_nr` from a type tree.
    /// Returns the `def_nr` of the first `Reference` that refers to the type variable,
    /// or `u32::MAX` if not found.
    fn extract_type_var(tp: &Type) -> u32 {
        match tp {
            Type::Reference(d, _) => *d,
            Type::Vector(inner, _) => Self::extract_type_var(inner),
            _ => u32::MAX,
        }
    }

    /// Unify a template parameter type with a concrete argument type to extract
    /// what the type variable `tv_nr` resolves to.
    /// E.g. template `vector<T>` + concrete `vector<integer>` → `integer`.
    fn resolve_type_var(template_tp: &Type, tv_nr: u32, concrete_tp: &Type) -> Type {
        match template_tp {
            Type::Reference(d, _) if *d == tv_nr => concrete_tp.clone(),
            Type::Vector(inner, _) => {
                if let Type::Vector(c_inner, _) = concrete_tp {
                    Self::resolve_type_var(inner, tv_nr, c_inner)
                } else {
                    Type::Unknown(0)
                }
            }
            _ => Type::Unknown(0),
        }
    }

    /// Re-resolve a Call target: if the called function's first parameter references
    /// the type variable, look up the correct overload for the concrete type.
    fn re_resolve_call(d_nr: u32, tv_nr: u32, concrete: &Type, data: &Data) -> u32 {
        if d_nr == u32::MAX || d_nr as usize >= data.definitions.len() {
            return d_nr;
        }
        let def = &data.definitions[d_nr as usize];
        if def.attributes.is_empty() {
            return d_nr;
        }
        // Check if any attribute's type references the type variable.
        let has_tv = def
            .attributes
            .iter()
            .any(|a| Self::type_contains_tv(&a.typedef, tv_nr));
        if !has_tv {
            // Also check for Integer(0, tv_nr) patterns — operators sometimes encode
            // type info in the Integer bounds.
            return d_nr;
        }
        // Resolve the concrete first-arg type by substituting tv_nr in the attribute type.
        let concrete_arg =
            Self::substitute_type(def.attributes[0].typedef.clone(), tv_nr, concrete);
        // Extract the user-facing function name from the mangled definition name.
        // Mangled names: "t_<LEN><Type>_<name>" or "n_<name>" or operator names.
        let name = &def.name;
        let fn_name = if let Some(rest) = name.strip_prefix("t_") {
            // Skip the LEN digits and type name, extract name after the underscore.
            if let Some(idx) = rest.find('_') {
                &rest[idx + 1..]
            } else {
                name.as_str()
            }
        } else if let Some(rest) = name.strip_prefix("n_") {
            rest
        } else {
            // Operator name — use as-is for find_fn.
            name.as_str()
        };
        let resolved = data.find_fn(u16::MAX, fn_name, &concrete_arg);
        if resolved != u32::MAX && resolved != d_nr {
            resolved
        } else {
            d_nr
        }
    }

    /// Check if a type references the type variable.
    fn type_contains_tv(tp: &Type, tv_nr: u32) -> bool {
        match tp {
            Type::Reference(d, _) | Type::Unknown(d) => *d == tv_nr,
            Type::Vector(inner, _) => Self::type_contains_tv(inner, tv_nr),
            _ => false,
        }
    }

    /// Substitute all occurrences of `Type::Reference(tv_nr, _)` with `concrete` in a type.
    fn substitute_type(tp: Type, tv_nr: u32, concrete: &Type) -> Type {
        match tp {
            Type::Reference(d, _) if d == tv_nr => concrete.clone(),
            Type::Vector(inner, deps) => Type::Vector(
                Box::new(Self::substitute_type(*inner, tv_nr, concrete)),
                deps,
            ),
            // Plan-17 phase 01 — substitute through tuple element types so a
            // generic `<T: Bound>` returning `(T, T)` (or any tuple shape
            // containing T) monomorphises correctly.  Without this, the
            // signature stayed `(DbRef, DbRef)` (the parametric T form)
            // even when params became `i64`, and native codegen rejected
            // the body's tuple literal with E0308.
            Type::Tuple(elems) => Type::Tuple(
                elems
                    .into_iter()
                    .map(|e| Self::substitute_type(e, tv_nr, concrete))
                    .collect(),
            ),
            other => other,
        }
    }

    /// Recursively substitute types in a Value IR tree and re-resolve Call targets
    /// whose first parameter references the type variable.
    fn substitute_type_in_value(val: Value, tv_nr: u32, concrete: &Type, data: &Data) -> Value {
        match val {
            Value::Call(d, args) => {
                let new_args: Vec<_> = args
                    .into_iter()
                    .map(|a| Self::substitute_type_in_value(a, tv_nr, concrete, data))
                    .collect();
                // Re-resolve call target if it references the type variable.
                let new_d = Self::re_resolve_call(d, tv_nr, concrete, data);
                // I9-vec: fix vector element access with baked-in elm_size=0.
                // The template bakes elm_size=0 for type-variable elements and omits the
                // value-extraction wrapper (OpGetInt/OpGetFloat/etc.).  Fix both here.
                if new_d != u32::MAX
                    && (new_d as usize) < data.definitions.len()
                    && data.def(new_d).name == "OpGetVector"
                    && new_args.len() == 3
                {
                    let cur_size = if let Value::Int(n) = &new_args[1] {
                        *n
                    } else {
                        0
                    };
                    let elm_size = Self::type_element_size(concrete, data);
                    if elm_size != cur_size {
                        let mut fixed = new_args;
                        fixed[1] = Value::Int(elm_size);
                        let call = Value::Call(new_d, fixed);
                        return Self::wrap_vector_get_val(call, concrete, data);
                    }
                    return Self::wrap_vector_get_val(Value::Call(new_d, new_args), concrete, data);
                }
                // I9-text fixup: when a T-stub had an extra __work_1 parameter
                // (for text-returning interface methods) but the concrete method
                // doesn't, drop the trailing argument to match the concrete signature.
                if new_d != d && new_d != u32::MAX && (new_d as usize) < data.definitions.len() {
                    let concrete_params = data.def(new_d).attributes.len();
                    if new_args.len() > concrete_params {
                        let mut trimmed = new_args;
                        trimmed.truncate(concrete_params);
                        return Value::Call(new_d, trimmed);
                    }
                }
                Value::Call(new_d, new_args)
            }
            Value::Block(bl) => Value::Block(Box::new(crate::data::Block {
                operators: bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, data))
                    .collect(),
                result: Self::substitute_type(bl.result, tv_nr, concrete),
                name: bl.name,
                scope: bl.scope,
                var_size: bl.var_size,
            })),
            Value::Set(v, expr) => Value::Set(
                v,
                Box::new(Self::substitute_type_in_value(*expr, tv_nr, concrete, data)),
            ),
            Value::Return(expr) => Value::Return(Box::new(Self::substitute_type_in_value(
                *expr, tv_nr, concrete, data,
            ))),
            Value::If(cond, t, f) => Value::If(
                Box::new(Self::substitute_type_in_value(*cond, tv_nr, concrete, data)),
                Box::new(Self::substitute_type_in_value(*t, tv_nr, concrete, data)),
                Box::new(Self::substitute_type_in_value(*f, tv_nr, concrete, data)),
            ),
            Value::Loop(bl) => Value::Loop(Box::new(crate::data::Block {
                operators: bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, data))
                    .collect(),
                result: Self::substitute_type(bl.result, tv_nr, concrete),
                name: bl.name,
                scope: bl.scope,
                var_size: bl.var_size,
            })),
            Value::Drop(expr) => Value::Drop(Box::new(Self::substitute_type_in_value(
                *expr, tv_nr, concrete, data,
            ))),
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, data))
                    .collect(),
            ),
            Value::Iter(name, create, next, extra) => Value::Iter(
                name,
                Box::new(Self::substitute_type_in_value(
                    *create, tv_nr, concrete, data,
                )),
                Box::new(Self::substitute_type_in_value(*next, tv_nr, concrete, data)),
                Box::new(Self::substitute_type_in_value(
                    *extra, tv_nr, concrete, data,
                )),
            ),
            Value::Span(b) => {
                let (pos, inner) = *b;
                let new_inner = Self::substitute_type_in_value(inner, tv_nr, concrete, data);
                Value::with_span(pos, new_inner)
            }
            other => other,
        }
    }

    /// I9-vec: compute element store size from the Type alone (no database needed).
    fn type_element_size(tp: &Type, data: &Data) -> i32 {
        // Post-2c: honor size(N) on integer aliases.
        if matches!(tp, Type::Integer(_)) {
            let alias_nr = data.type_elm(tp);
            if let Some(n) = data.forced_size(alias_nr) {
                return i32::from(n);
            }
        }
        match tp {
            Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Text(_)
            | Type::Enum(_, false, _) => 4,
            Type::Integer(_) | Type::Float => 8,
            // for Reference(struct_nr), compute the struct's inline field
            // size from its attributes rather than assuming 12 (DbRef size).
            // Vector elements of struct type are stored inline, not as pointers.
            Type::Reference(d_nr, _) => {
                if (*d_nr as usize) < data.definitions.len()
                    && data.def(*d_nr).def_type == DefType::Struct
                {
                    let mut total = 0i32;
                    for attr in &data.def(*d_nr).attributes {
                        if attr.constant {
                            continue;
                        }
                        total += Self::type_element_size(&attr.typedef, data);
                    }
                    if total > 0 {
                        return total;
                    }
                }
                12 // non-struct reference: DbRef = 12 bytes
            }
            _ => 12,
        }
    }

    /// I9-vec: wrap an `OpGetVector` result with the appropriate value-extraction op
    /// for concrete value types (`OpGetInt`, `OpGetFloat`, etc.).  Reference types need
    /// no wrapper — the `DbRef` IS the value.
    fn wrap_vector_get_val(code: Value, tp: &Type, data: &Data) -> Value {
        let p = Value::Int(0);
        let (op_name, extra) = match tp {
            Type::Integer(_) => ("OpGetInt", None),
            Type::Float => ("OpGetFloat", None),
            Type::Single => ("OpGetSingle", None),
            Type::Text(_) => ("OpGetText", None),
            Type::Boolean => ("OpGetByte", Some(true)),
            _ => return code, // reference/struct types: no wrapper needed
        };
        let d = data.def_nr(op_name);
        if d == u32::MAX {
            return code;
        }
        let val = if extra.is_some() {
            // Boolean: GetByte + compare to 1
            Value::Call(d, vec![code, p, Value::Int(0)])
        } else {
            Value::Call(d, vec![code, p])
        };
        if extra.is_some() {
            let d_eq = data.def_nr("OpEqInt");
            if d_eq == u32::MAX {
                val
            } else {
                Value::Call(d_eq, vec![val, Value::Int(1)])
            }
        } else {
            val
        }
    }

    /// Resolve named arguments into positional slots, then delegate to `call_nr`.
    fn call_with_named(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        positional: &[Value],
        pos_types: &[Type],
        named: &[(String, Value, Type)],
        is_method: bool,
    ) -> Type {
        if named.is_empty() {
            return self.call_nr(code, d_nr, positional, pos_types, is_method);
        }
        // Build full argument vector with named args placed at the correct indices.
        let n_params = self.data.attributes(d_nr);
        let mut args = vec![Value::Null; n_params];
        let mut arg_types = vec![Type::Unknown(0); n_params];
        // Place positional args first.
        for (i, (val, tp)) in positional.iter().zip(pos_types.iter()).enumerate() {
            if i < n_params {
                args[i] = val.clone();
                arg_types[i] = tp.clone();
            }
        }
        let pos_count = positional.len();
        // Place named args by looking up parameter names.
        for (name, val, tp) in named {
            let idx = self.data.attr(d_nr, name);
            if idx == usize::MAX {
                if !self.first_pass {
                    diagnostic!(self.lexer, Level::Error, "Unknown parameter '{name}'");
                }
                continue;
            }
            if idx < pos_count {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Parameter '{name}' already provided as positional argument {idx}"
                    );
                }
                continue;
            }
            if args[idx] != Value::Null {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Duplicate named argument '{name}'"
                    );
                }
                continue;
            }
            args[idx] = val.clone();
            arg_types[idx] = tp.clone();
        }
        // Trim trailing Null args — add_defaults will fill them.
        let mut last_provided = args.len();
        while last_provided > 0 && args[last_provided - 1] == Value::Null {
            last_provided -= 1;
        }
        args.truncate(last_provided);
        arg_types.truncate(last_provided);
        self.call_nr(code, d_nr, &args, &arg_types, is_method)
    }

    fn single_op(&mut self, op: &str, f: Value, t: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f], &[t]);
        code
    }

    fn conv_op(&mut self, op: &str, f: Value, n: Value, f_tp: Type, n_tp: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f, n], &[f_tp, n_tp]);
        code
    }

    fn op(&mut self, op: &str, f: Value, n: Value, t: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f, n], &[t.clone(), t]);
        code
    }

    fn get_field(&mut self, d_nr: u32, f_nr: usize, code: Value) -> Value {
        // #91: track $.<field> accesses during init(expr) parsing.
        if self.init_field_tracking && code == Value::Var(0) && f_nr != usize::MAX {
            let name = self.data.attr_name(d_nr, f_nr);
            if !self.init_field_deps.contains(&name) {
                self.init_field_deps.push(name);
            }
        }
        let tp = self.data.attr_type(d_nr, f_nr);
        let nullable = self.data.attr_nullable(d_nr, f_nr);
        self.expr_not_null = !nullable;
        if !nullable && f_nr != usize::MAX {
            self.expr_not_null_name = self.data.attr_name(d_nr, f_nr);
        } else {
            self.expr_not_null_name.clear();
        }
        let pos = if f_nr == usize::MAX {
            0
        } else {
            let nm = self.data.attr_name(d_nr, f_nr);
            self.database.position(self.data.def(d_nr).known_type, &nm)
        };
        // Post-2c: pass the field's alias def_nr so `get_val` can honor
        // size(N) for integer subtypes (e.g. i32 → OpGetInt4).
        let alias = if f_nr == usize::MAX {
            u32::MAX
        } else {
            self.data.def(d_nr).attributes[f_nr].alias_d_nr
        };
        self.get_val(&tp, nullable, u32::from(pos), code, alias)
    }

    fn get_val(&mut self, tp: &Type, nullable: bool, pos: u32, code: Value, alias: u32) -> Value {
        let p = Value::Int(pos as i32);
        match tp {
            Type::Integer(spec) => {
                // Narrow-integer width selection:
                // * `alias` is set → this is a struct-field read whose
                //   captured alias may carry `size(N)`.  Use that, else
                //   the bounds-heuristic `byte_width` (which works for
                //   plain `integer` and `integer limit(...)` fields).
                // * `alias == u32::MAX` AND spec has a forced_size → we're
                //   likely inside a vector element read.  Use
                //   `vector_narrow_width` to mirror Phase 2's actual
                //   storage decision (1 and 4 bytes narrow; 2 stays wide
                //   until the short-encoding Phase 4 round lands); fall
                //   through to 8 when Phase 2 stored wide so reads align.
                // * `alias == u32::MAX` AND no forced_size → bounds
                //   heuristic (struct-field path for plain or limited
                //   `integer`).
                // Narrow-vec path: alias is absent AND spec carries a
                // forced_size AND the gate is open.  This branch maps
                // to `Parts::ShortRaw` for 2-byte (Phase 4b) and to
                // `Parts::Byte` / `Parts::Int` for 1/4-byte (Phase 4a).
                // When the gate is CLOSED for a given forced_size
                // (fallback), storage stays wide (8-byte) and the
                // read must match — use `unwrap_or(8)` so closed-gate
                // forced_size reads dispatch to `OpGetInt`.
                let narrow_vec = alias == u32::MAX
                    && spec.forced_size.is_some()
                    && spec.vector_narrow_width().is_some();
                let s = if alias != u32::MAX {
                    self.data
                        .forced_size(alias)
                        .unwrap_or_else(|| spec.byte_width(nullable))
                } else if spec.forced_size.is_some() {
                    spec.vector_narrow_width().unwrap_or(8)
                } else {
                    spec.byte_width(nullable)
                };
                debug_assert!(
                    matches!(s, 1 | 2 | 4 | 8),
                    "get_val: unexpected integer field width s={s} \
                     (alias_d_nr={alias}) — only 1/2/4/8 are supported \
                     by the OpGet* family"
                );
                if s == 1 {
                    self.cl("OpGetByte", &[code, p, Value::Int(spec.min)])
                } else if s == 2 && narrow_vec {
                    // narrow vector element, direct encoding.
                    self.cl("OpGetShortRaw", &[code, p, Value::Int(spec.min)])
                } else if s == 2 {
                    // Struct field with u16/i16 alias OR bounds-heuristic
                    // landing at 2 bytes: legacy `Parts::Short` `+1` encoding.
                    self.cl("OpGetShort", &[code, p, Value::Int(spec.min)])
                } else if s == 4 {
                    self.cl("OpGetInt4", &[code, p])
                } else {
                    self.cl("OpGetInt", &[code, p])
                }
            }
            Type::Enum(_, false, _) => self.cl("OpGetEnum", &[code, p]),
            Type::Boolean => {
                let val = self.cl("OpGetByte", &[code, p, Value::Int(0)]);
                self.cl("OpEqInt", &[val, Value::Int(1)])
            }
            Type::Float => self.cl("OpGetFloat", &[code, p]),
            Type::Single => self.cl("OpGetSingle", &[code, p]),
            Type::Text(_) => self.cl("OpGetText", &[code, p]),
            Type::Hash(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _)
            | Type::Vector(_, _) => {
                let info = self.type_info(tp);
                self.cl("OpGetField", &[code, p, info])
            }
            Type::Reference(_, _) => {
                // Inline struct field: OpGetField adds the field offset to the base ref.
                // Linked/base type dereference is handled at the call site (fields.rs)
                // using OpVectorRef, which combines the 4-byte pointer read + deref.
                let info = self.type_info(tp);
                self.cl("OpGetField", &[code, p, info])
            }
            Type::Function(_, _, _) => {
                // Storage holds the 4-byte i32 d_nr; the stack-side
                // fn-ref slot is 20 bytes (8B i64 d_nr + 12B null
                // closure DbRef).  Read d_nr via OpGetInt4 (pushes
                // 8B), then push a 12B null sentinel for the closure
                // half.  Mirrors `gen_fn_ref_value`'s "if value
                // generated < 16 bytes, fill" pattern but emits the
                // sequence inline as a Block so callers see a single
                // Type::Function value.
                let read_dnr = self.cl("OpGetInt4", &[code, p]);
                let null_clos = self.cl("OpNullRefSentinel", &[]);
                crate::data::v_block(vec![read_dnr, null_clos], tp.clone(), "fn_ref_field_read")
            }
            Type::Tuple(elems) => {
                // Plan-06 phase 4d: tuple struct field read.  Each
                // element is read from `pos + element_offsets[i]`
                // using the same OpGet* opcodes that ordinary struct
                // fields use; the assembled stack tuple matches the
                // shape `Type::Tuple(...)` consumers expect.
                let elems_vec = elems.clone();
                let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
                let offsets: Vec<u16> = crate::data::stored_tuple_offsets_for_def(
                    &self.data,
                    &self.database,
                    tuple_d_nr,
                    elems_vec.len(),
                )
                .unwrap_or_else(|| {
                    crate::data::element_offsets(&elems_vec)
                        .into_iter()
                        .map(|x| x as u16)
                        .collect()
                });
                let mut tuple_elems = Vec::with_capacity(elems_vec.len());
                for (i, et) in elems_vec.iter().enumerate() {
                    let elem_pos = pos + u32::from(offsets[i]);
                    let elem_val = self.get_val(et, false, elem_pos, code.clone(), u32::MAX);
                    tuple_elems.push(elem_val);
                }
                Value::Tuple(tuple_elems)
            }
            _ => {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Field access not supported on type {}",
                    tp.name(&self.data)
                );
                Value::Null
            }
        }
    }

    fn set_field(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        self.set_field_check(d_nr, f_nr, d_pos, ref_code, val_code, true)
    }

    /// Plan-06 phase 4d: emit the per-element OpSet* sequence for a
    /// tuple struct field's value.  Recurses for nested Tuple element
    /// types so `((1, 2), (3, 4))` written into a `((int, int), (int,
    /// int))` field flattens to four `OpSetInt` calls at offsets
    /// `[0, 8, 16, 24]`.
    ///
    /// `base_pos` is the byte offset within the host record where the
    /// tuple's first element begins.  `elems` lists the tuple's
    /// element types in declaration order.  `val_code` is the IR for
    /// the tuple value.
    ///
    /// Two flavours of `val_code`:
    /// - `Value::Tuple([…])` literal — walked element-wise without a
    ///   temp variable, allowing direct recursion for nested tuples.
    /// - Anything else (variable, function call, etc.) — stashed in a
    ///   work-ref tuple temp first, then read element-by-element via
    ///   `Value::TupleGet`.  Nested tuple elements in this branch
    ///   require codegen support for `TupleGet` on `Type::Tuple`
    ///   (see `state/codegen.rs::Value::TupleGet` stack-tuple arm).
    pub(crate) fn emit_tuple_set_ops(
        &mut self,
        ref_code: &Value,
        base_pos: u16,
        elems: &[Type],
        val_code: Value,
    ) -> Vec<Value> {
        let elems_vec = elems.to_vec();
        let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
        let offsets: Vec<u16> = crate::data::stored_tuple_offsets_for_def(
            &self.data,
            &self.database,
            tuple_d_nr,
            elems_vec.len(),
        )
        .unwrap_or_else(|| {
            crate::data::element_offsets(&elems_vec)
                .into_iter()
                .map(|x| x as u16)
                .collect()
        });
        if let Value::Tuple(values) = val_code {
            // Literal tuple value — recurse element-wise so nested
            // tuple elements flatten into per-leaf OpSet* calls.
            let mut ops = Vec::new();
            for (i, (elem_tp, value_i)) in elems_vec.iter().zip(values).enumerate() {
                // First-pass `base_pos` can be the `database.position`
                // u16::MAX sentinel for not-yet-resolved fields.  The
                // raw `+` panics under dev-profile overflow checks
                // (caught by `make iter`); release silently wraps.
                // The IR built during pass 1 is regenerated in pass 2,
                // so a saturating placeholder is safe and keeps the
                // arithmetic well-defined across both profiles.
                let elem_pos = base_pos.saturating_add(offsets[i]);
                ops.extend(self.emit_set_one_element(ref_code, elem_pos, elem_tp, value_i));
            }
            return ops;
        }
        // Non-literal source: stash to a work-ref Tuple local, then
        // read each element via `Value::TupleGet`.
        let tup_tp = Type::Tuple(elems_vec.clone());
        let tmp = self.vars.work_refs(&tup_tp, &mut self.lexer);
        if !self.first_pass {
            self.change_var_type(tmp, &tup_tp);
        }
        let mut ops = vec![v_set(tmp, val_code)];
        for (i, elem_tp) in elems_vec.iter().enumerate() {
            let elem_pos = base_pos.saturating_add(offsets[i]);
            let elem_val = Value::TupleGet(tmp, i as u16);
            ops.extend(self.emit_set_one_element(ref_code, elem_pos, elem_tp, elem_val));
        }
        ops
    }

    /// Emit the OpSet* (or recursive flatten) for a single tuple
    /// element at a fixed byte offset within the host record.
    /// Returns a vec because nested-tuple elements expand to multiple
    /// per-leaf set ops.
    fn emit_set_one_element(
        &mut self,
        ref_code: &Value,
        pos: u16,
        elem_tp: &Type,
        value: Value,
    ) -> Vec<Value> {
        let pos_v = Value::Int(i32::from(pos));
        let single = match elem_tp {
            Type::Integer(_) => self.cl("OpSetInt", &[ref_code.clone(), pos_v, value]),
            Type::Function(_, _, _) => {
                // P196: storage holds the 4-byte i32 d_nr only.  Reduce
                // `Value::FnRef` to its bare `Value::Int(d_nr)` so the
                // OpSetInt4 template body sees an i64 the interpreter
                // pops in 8 bytes; for fn-ref-shaped sources (Var,
                // TupleGet, function call), interpreter `TupleGet`
                // already pushes only the d_nr's 8 bytes via OpVarInt,
                // and `output_call` projects `.0` natively (see the
                // val_is_fn_ref handling in src/generation/calls.rs).
                let d_nr_only = match value {
                    Value::FnRef(d_nr, _, _) => Value::Int(d_nr),
                    other => other,
                };
                self.cl("OpSetInt4", &[ref_code.clone(), pos_v, d_nr_only])
            }
            Type::Float => self.cl("OpSetFloat", &[ref_code.clone(), pos_v, value]),
            Type::Single => self.cl("OpSetSingle", &[ref_code.clone(), pos_v, value]),
            Type::Character => self.cl("OpSetCharacter", &[ref_code.clone(), pos_v, value]),
            Type::Boolean => {
                let v = v_if(value, Value::Int(1), Value::Int(0));
                self.cl("OpSetByte", &[ref_code.clone(), pos_v, Value::Int(0), v])
            }
            Type::Text(_) => self.cl("OpSetText", &[ref_code.clone(), pos_v, value]),
            Type::Reference(inner_d_nr, _) | Type::Enum(inner_d_nr, true, _) => {
                let type_nr = if self.first_pass {
                    Value::Int(i32::from(u16::MAX))
                } else {
                    Value::Int(i32::from(self.data.def(*inner_d_nr).known_type))
                };
                let field_ref = self.cl(
                    "OpGetField",
                    &[ref_code.clone(), pos_v.clone(), type_nr.clone()],
                );
                self.cl("OpCopyRecord", &[value, field_ref, type_nr])
            }
            Type::Vector(content, _) => {
                let vec_tp = self.vector_of(content);
                let elem_db_tp = self.database.content(vec_tp);
                let field_ref = self.cl(
                    "OpGetField",
                    &[
                        ref_code.clone(),
                        pos_v.clone(),
                        Value::Int(i32::from(vec_tp)),
                    ],
                );
                self.cl(
                    "OpAppendVector",
                    &[field_ref, value, Value::Int(i32::from(elem_db_tp))],
                )
            }
            Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Sorted(_, _, _) => self.cl("OpSetInt4", &[ref_code.clone(), pos_v, value]),
            // Plan-06 phase 4d: nested tuple element — recurse into
            // `emit_tuple_set_ops` with the inner tuple's offsets so
            // each leaf primitive lands at `outer_pos +
            // outer_offsets[i] + inner_offsets[j]`.  The flattened
            // ops are returned as a single Vec.
            Type::Tuple(inner_elems) => {
                return self.emit_tuple_set_ops(ref_code, pos, inner_elems, value);
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Tuple struct field cannot contain element of type {}",
                        elem_tp.name(&self.data)
                    );
                }
                Value::Null
            }
        };
        vec![single]
    }

    fn set_field_no_check(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        self.set_field_check(d_nr, f_nr, d_pos, ref_code, val_code, false)
    }

    fn set_field_check(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
        emit_check: bool,
    ) -> Value {
        let tp = self.data.attr_type(d_nr, f_nr);
        let nm = self.data.attr_name(d_nr, f_nr);
        let pos = self.database.position(self.data.def(d_nr).known_type, &nm);
        let pos_val = Value::Int(if f_nr == usize::MAX {
            i32::from(d_pos)
        } else {
            i32::from(pos + d_pos)
        });
        let has_check = emit_check
            && f_nr != usize::MAX
            && !self.first_pass
            && self
                .data
                .def(d_nr)
                .attributes
                .get(f_nr)
                .is_some_and(|a| a.check != Value::Null);
        let ref_for_check = if has_check {
            Some(ref_code.clone())
        } else {
            None
        };
        let set_op = match tp {
            Type::Integer(ref spec) => {
                let IntegerSpec { min, .. } = *spec;
                let m = Value::Int(min);
                // Post-2c: honor size(N) on the alias recorded during field
                // parsing; fall back to the limit()-based heuristic.
                let alias_nr = if f_nr == usize::MAX {
                    u32::MAX
                } else {
                    self.data.def(d_nr).attributes[f_nr].alias_d_nr
                };
                // narrow-vec path mirrors `get_val`.
                // Reached by `insert(vector<u16>, ...)` where
                // `set_field` is invoked with `f_nr == usize::MAX` and
                // `tp` is the narrow element Type.  Struct-field
                // writes (alias_nr != u32::MAX) stay on the legacy
                // `OpSetShort` / `Parts::Short` `+1` encoding path.
                let narrow_vec = alias_nr == u32::MAX
                    && spec.forced_size.is_some()
                    && spec.vector_narrow_width().is_some();
                let s = self.data.forced_size(alias_nr).unwrap_or_else(|| {
                    if narrow_vec {
                        spec.vector_narrow_width().unwrap()
                    } else {
                        tp.size(self.data.attr_nullable(d_nr, f_nr))
                    }
                });
                // Size-consistency gate: the size resolved from
                // `forced_size` / limit must be one of the four
                // supported widths.  Any other value indicates a
                // post-2c regression in `size()` or a novel alias that
                // needs a matching Op emission branch here.
                debug_assert!(
                    matches!(s, 1 | 2 | 4 | 8),
                    "set_field_check: unexpected integer field width \
                     s={s} for {}.{} (alias_d_nr={alias_nr}) — only \
                     1/2/4/8 are supported by the OpSet* family",
                    self.data.def(d_nr).name,
                    if f_nr == usize::MAX {
                        "<unknown>".to_string()
                    } else {
                        self.data.def(d_nr).attributes[f_nr].name.clone()
                    },
                );
                if s == 1 {
                    self.cl("OpSetByte", &[ref_code, pos_val, m, val_code])
                } else if s == 2 && narrow_vec {
                    self.cl("OpSetShortRaw", &[ref_code, pos_val, m, val_code])
                } else if s == 2 {
                    self.cl("OpSetShort", &[ref_code, pos_val, m, val_code])
                } else if s == 4 {
                    self.cl("OpSetInt4", &[ref_code, pos_val, val_code])
                } else {
                    self.cl("OpSetInt", &[ref_code, pos_val, val_code])
                }
            }
            Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Sorted(_, _, _) => {
                // Collection header is a 4-byte u32 record pointer.  Post-2c
                // `OpSetInt` writes 8 bytes (i64), which overflows into the
                // next field.  Use `OpSetInt4` to write only 4 bytes.
                self.cl("OpSetInt4", &[ref_code, pos_val, val_code])
            }
            Type::Function(_, _, _) => {
                // Storage holds the 4-byte i32 d_nr only — closures
                // (the 12-byte trailing half of a 20-byte stack
                // fn-ref slot) are NOT stored here.  Reduce
                // `Value::FnRef` to its bare `Value::Int(d_nr)` so
                // the literal lambda case bypasses any tuple shape;
                // for non-literal sources (Var of fn-ref, TupleGet
                // of a fn-ref tuple element, function-call return)
                // the interpreter's `TupleGet`/`Var` codegen pushes
                // only the d_nr's 8 bytes via `OpVarInt`, and the
                // native template substitution in
                // `src/generation/calls.rs` projects `.0` from the
                // `(u32, DbRef)` fn-ref tuple before the i32 cast.
                let d_nr_only = match val_code {
                    Value::FnRef(d_nr, _, _) => Value::Int(d_nr),
                    other => other,
                };
                self.cl("OpSetInt4", &[ref_code, pos_val, d_nr_only])
            }
            Type::Tuple(ref elems) => {
                // Plan-06 phase 4d: tuple struct field assignment.
                // Storage layout matches the synthetic `__tuple<…>`
                // struct's element positions (registered via
                // `tuple_def`) so each element write goes through
                // the same `OpSetInt`/`OpSetFloat`/etc. opcodes used
                // for ordinary struct fields.  Recurses for nested
                // Tuple element types — see `emit_tuple_set_ops`.
                let elems_vec = elems.clone();
                let host_field_pos = if let Value::Int(p) = pos_val {
                    p as u16
                } else {
                    0
                };
                let ops = self.emit_tuple_set_ops(&ref_code, host_field_pos, &elems_vec, val_code);
                v_block(ops, Type::Void, "tuple_field_set")
            }
            Type::Character => self.cl("OpSetCharacter", &[ref_code, pos_val, val_code]),
            Type::Reference(inner_tp, _) => {
                // The value is a 12-byte DbRef; OpSetInt would only read 4 bytes of it.
                // Copy the struct bytes into the embedded field instead.
                let type_nr = if self.first_pass {
                    Value::Int(i32::from(u16::MAX))
                } else {
                    Value::Int(i32::from(self.data.def(inner_tp).known_type))
                };
                let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
                // Note: the free-source high-bit for Issue #120 is set in
                // copy_ref() (operators.rs), which is the path for struct
                // field reassignment. This set_field_check path is for
                // construction (initial field population).
                self.cl("OpCopyRecord", &[val_code.clone(), field_ref, type_nr])
            }
            Type::Enum(_, false, _) => self.cl("OpSetEnum", &[ref_code, pos_val, val_code]),
            Type::Enum(nr, true, _) => self.cl(
                "OpCopyRecord",
                &[
                    val_code,
                    ref_code,
                    Value::Int(i32::from(self.data.def(nr).known_type)),
                ],
            ),
            Type::Boolean => {
                let v = v_if(val_code, Value::Int(1), Value::Int(0));
                self.cl("OpSetByte", &[ref_code, pos_val, Value::Int(0), v])
            }
            Type::Float => self.cl("OpSetFloat", &[ref_code, pos_val, val_code]),
            Type::Single => self.cl("OpSetSingle", &[ref_code, pos_val, val_code]),
            Type::Text(_) => self.cl("OpSetText", &[ref_code, pos_val, val_code]),
            _ => {
                if self.first_pass {
                    Value::Null
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot assign to field '{}' of type {}",
                        self.data.attr_name(d_nr, f_nr),
                        self.data.attr_type(d_nr, f_nr).name(&self.data)
                    );
                    Value::Null
                }
            }
        };
        self.emit_field_constraint(set_op, ref_for_check, d_nr, f_nr, &nm)
    }

    /// Wrap a set operation with a constraint assertion if the field has one.
    fn emit_field_constraint(
        &mut self,
        set_op: Value,
        ref_for_check: Option<Value>,
        d_nr: u32,
        f_nr: usize,
        field_name: &str,
    ) -> Value {
        let Some(ref_val) = ref_for_check else {
            return set_op;
        };
        let check = self.data.def(d_nr).attributes[f_nr].check.clone();
        let bound = Self::replace_record_ref(check, &ref_val);
        let msg = if let Value::Text(s) = &self.data.def(d_nr).attributes[f_nr].check_message {
            Value::Text(s.clone())
        } else {
            Value::Text(format!(
                "field constraint failed on {}.{field_name}",
                self.data.def(d_nr).name
            ))
        };
        let assert_dnr = self.data.def_nr("n_assert");
        let pos = self.lexer.pos();
        let assert_call = Value::Call(
            assert_dnr,
            vec![
                bound,
                msg,
                Value::Text(pos.file.clone()),
                Value::Int(pos.line as i32),
            ],
        );
        Value::Insert(vec![set_op, assert_call])
    }

    /// Append a `--show-types --trace` entry for the type at the
    /// current parse position.  No-op unless `trace_types` is set
    /// AND we are on the second pass (first-pass types are
    /// placeholders and would emit thousands of meaningless lines).
    pub(crate) fn record_type_trace(&mut self, t: &Type) {
        if !self.trace_types || self.first_pass {
            return;
        }
        let pos = self.lexer.pos();
        let fn_name = self.vars.name.clone();
        if fn_name.is_empty() {
            return;
        }
        let type_str = t.show(&self.data, &self.vars);
        self.trace_types_lines
            .push(format!("{fn_name}\t{}:{}\t{type_str}", pos.line, pos.pos));
    }

    fn cl(&mut self, op: &str, list: &[Value]) -> Value {
        let d_nr = self.data.def_nr(op);
        if d_nr == u32::MAX {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Internal error: missing built-in operation (report this as a bug)"
            );
            Value::Null
        } else {
            Value::Call(d_nr, list.to_vec())
        }
    }

    /// Try to find a matching defined operator. There can be multiple possible definitions for each operator.
    fn call_op(&mut self, code: &mut Value, op: &str, list: &[Value], types: &[Type]) -> Type {
        // I8.1: if any operand is a generic type variable, skip the main operator loop
        // and go straight to the T-stub lookup.  The main loop would otherwise false-match
        // concrete operators (e.g. OpEqRef, OpEqBool) via implicit type conversions on T.
        let generic_name = types.iter().find_map(|t| self.generic_type_name(t));
        if let Some(tv_name) = generic_name {
            if self.first_pass {
                // Return the type variable type so assignments keep a consistent type
                // through the first pass (Type::Void would trigger "cannot change type").
                let tv_nr = self.data.def_nr(tv_name);
                return if tv_nr == u32::MAX {
                    Type::Unknown(0)
                } else {
                    Type::Reference(tv_nr, Vec::new())
                };
            }
            let op_method = format!("Op{}", rename(op));
            let stub_name = format!("t_{}{}_{}", tv_name.len(), tv_name, op_method);
            let stub_nr = self.data.def_nr(&stub_name);
            // Only use the T-stub if the CURRENT function's bounds declare this method.
            // Without this check, T-stubs from unrelated bounded generics (e.g., stdlib's
            // sum<T: Addable>) would leak into unbound generics like `fn bad<T>(x+y)`.
            if stub_nr != u32::MAX
                && self.context != u32::MAX
                && self.has_bound_for_method(&op_method)
            {
                let tp = self.call_nr(code, stub_nr, list, types, false);
                if tp != Type::Null {
                    return tp;
                }
            }
        } else {
            let mut possible = Vec::new();
            for pos in self
                .data
                .get_possible(&format!("Op{}", rename(op)), &self.lexer)
            {
                possible.push(*pos);
            }
            for pos in possible {
                // skip OpEqBool when comparing character with text —
                // prevents 'a' == "b" from resolving as true == true.
                if self.data.def(pos).name == "OpEqBool"
                    && types.len() >= 2
                    && ((matches!(types[0], Type::Character) && matches!(types[1], Type::Text(_)))
                        || (matches!(types[0], Type::Text(_))
                            && matches!(types[1], Type::Character)))
                {
                    continue;
                }
                let tp = self.call_nr(code, pos, list, types, false);
                if tp != Type::Null {
                    // We cannot compare two different types of enums, both will be integers in the same range
                    if let (Some(Type::Enum(f, _, _)), Some(Type::Enum(s, _, _))) =
                        (types.first(), types.get(1))
                        && f != s
                    {
                        break;
                    }
                    return tp;
                }
            }
        }
        // generic-specific error message for operators on T.
        let generic_name = types.iter().find_map(|t| self.generic_type_name(t));
        if let Some(tv_name) = generic_name {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "generic type {tv_name}: operator '{op}' requires a concrete type",
            );
        } else if types.len() > 1 {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "No matching operator '{op}' on '{}' and '{}'",
                types[0].name(&self.data),
                types[1].name(&self.data)
            );
        } else {
            specific!(
                self.lexer,
                &self.lexer.peek(),
                Level::Error,
                "No matching operator {op} on {}",
                types[0].name(&self.data)
            );
        }
        Type::Unknown(0)
    }

    /// Call a specific definition
    fn call_nr(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        list: &[Value],
        types: &[Type],
        report: bool,
    ) -> Type {
        let mut all_types = Vec::from(types);
        if self.data.def_type(d_nr) == DefType::Dynamic {
            for a_nr in 0..self.data.attributes(d_nr) {
                let Type::Routine(r_nr) = self.data.attr_type(d_nr, a_nr) else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Incorrect dynamic function {}",
                        self.data.def(d_nr).name
                    );
                    return Type::Void;
                };
                if self.data.attr_type(r_nr, 0).is_equal(&types[0]) {
                    return self.call_nr(code, r_nr, list, types, report);
                }
            }
            diagnostic!(
                self.lexer,
                Level::Error,
                "No matching function {}",
                self.data.def(d_nr).name
            );
        } else if !matches!(self.data.def_type(d_nr), DefType::Function) {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown definition {}",
                    self.data.def(d_nr).name
                );
            }
            return Type::Null;
        }
        let mut actual = self.process_call_args(d_nr, list, types, &mut all_types, report);
        if actual.is_empty() && !types.is_empty() {
            return Type::Null;
        }
        self.add_defaults(d_nr, &mut actual, &mut all_types);
        let tp = self.call_dependencies(d_nr, &all_types);
        *code = Value::Call(d_nr, actual);
        tp
    }

    /// Convert and validate each positional argument for a call.
    fn process_call_args(
        &mut self,
        d_nr: u32,
        list: &[Value],
        types: &[Type],
        all_types: &mut [Type],
        report: bool,
    ) -> Vec<Value> {
        let mut actual = Vec::new();
        if types.is_empty() {
            return actual;
        }
        if list.len() > self.data.attributes(d_nr) {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Too many parameters for {}",
                    self.data.def(d_nr).name
                );
            }
            return actual;
        }
        for (nr, a_code) in list.iter().enumerate() {
            let tp = self.data.attr_type(d_nr, nr);
            let Some(actual_type) = types.get(nr) else {
                continue;
            };
            let mut actual_code = a_code.clone();
            if let (Type::Vector(to_tp, _), Type::Vector(a_tp, _)) = (&tp, actual_type)
                && a_tp.is_unknown()
                && !to_tp.is_unknown()
            {
                self.change_var(&actual_code, &tp);
                actual.push(actual_code);
                continue;
            }
            // empty `[]` literal → create temp vector where parameter type is known.
            if matches!(&actual_code, Value::Insert(ops) if ops.len() <= 1)
                && let Type::Vector(elm_tp, dep) = &tp
            {
                let vec = self.create_unique("vec", &Type::Vector(elm_tp.clone(), dep.clone()));
                let mut ls = self.vector_db(elm_tp, vec);
                ls.push(Value::Var(vec));
                actual.push(v_block(ls, tp.clone(), "empty_vector_arg"));
                all_types[nr] = tp.clone();
                continue;
            }
            // L4: reject non-variable expressions passed to `&` parameters (except &text
            // which has its own work-text copy handling in convert()).  The `&` modifier
            // means "mutations propagate back to the caller" — passing a literal means
            // the mutations are silently discarded, which is almost certainly a bug.
            // also accept "addressable" expressions — vector element access
            // (`v[i]`), field access (`s.field`), and chains thereof — since these
            // produce a DbRef into existing mutable storage.
            if let Type::RefVar(inner) = &tp
                && !matches!(inner.as_ref(), Type::Text(_))
                && !matches!(&actual_code, Value::Var(_))
                && !Self::is_addressable(&actual_code, &self.data)
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot pass a literal or expression to a '&' parameter — \
                     assign to a variable first"
                );
                actual.push(actual_code);
                continue;
            }
            if actual_type.is_unknown() && matches!(&tp, Type::Vector(_, _)) {
                self.change_var(&actual_code, &tp);
                actual.push(actual_code);
                continue;
            }
            if let (Type::Integer(_), Type::Enum(_, true, _)) = (&tp, actual_type) {
                let cd = if matches!(actual_code, Value::Enum(_, _)) {
                    actual_code
                } else {
                    self.cl("OpGetEnum", &[actual_code, Value::Int(0)])
                };
                actual.push(self.cl("OpConvIntFromEnum", &[cd]));
                continue;
            }
            if !self.convert(&mut actual_code, actual_type, &tp) {
                if report {
                    let context = format!("call to {}", self.data.def(d_nr).original_name());
                    self.validate_convert(&context, actual_type, &tp);
                } else if !self.can_convert(actual_type, &tp) {
                    return Vec::new();
                }
            }
            actual.push(actual_code);
        }
        actual
    }

    // Gather depended on variables from arguments of the given called routine.
    fn call_dependencies(&mut self, d_nr: u32, types: &[Type]) -> Type {
        let tp = self.data.def(d_nr).returned.clone();
        // for Reference returns (structs), filter out hidden return-mechanism
        // attributes from dep resolution. The struct owns its store independently —
        // hidden return-store buffers are implementation artifacts.
        // Text/Vector returns genuinely depend on their hidden work buffers.
        let attrs = &self.data.def(d_nr).attributes;
        let filter_hidden = |d: &[u16]| -> Vec<u16> {
            d.iter()
                .copied()
                .filter(|&i| (i as usize) >= attrs.len() || !attrs[i as usize].hidden)
                .collect()
        };
        if let Type::Text(d) = tp {
            Type::Text(Self::resolve_deps(types, &d))
        } else if let Type::Vector(to, d) = tp {
            Type::Vector(to, Self::resolve_deps(types, &d))
        } else if let Type::Sorted(to, key, d) = tp {
            Type::Sorted(to, key, Self::resolve_deps(types, &d))
        } else if let Type::Hash(to, key, d) = tp {
            Type::Hash(to, key, Self::resolve_deps(types, &d))
        } else if let Type::Index(to, key, d) = tp {
            Type::Index(to, key, Self::resolve_deps(types, &d))
        } else if let Type::Spacial(to, key, d) = tp {
            Type::Spacial(to, key, Self::resolve_deps(types, &d))
        } else if let Type::Reference(to, d) = tp {
            Type::Reference(to, Self::resolve_deps(types, &filter_hidden(&d)))
        } else if let Type::Enum(to, true, d) = tp {
            Type::Enum(to, true, Self::resolve_deps(types, &filter_hidden(&d)))
        } else {
            tp
        }
    }

    fn resolve_deps(types: &[Type], d: &[u16]) -> Vec<u16> {
        let mut dp = HashSet::new();
        for ar in d {
            if *ar as usize >= types.len() {
                continue;
            }
            if let Type::Text(ad)
            | Type::Vector(_, ad)
            | Type::Sorted(_, _, ad)
            | Type::Hash(_, _, ad)
            | Type::Index(_, _, ad)
            | Type::Spacial(_, _, ad)
            | Type::Reference(_, ad)
            | Type::Enum(_, true, ad) = &types[*ar as usize]
            {
                for a in ad {
                    dp.insert(*a);
                }
            }
        }
        Vec::from_iter(dp)
    }

    fn add_defaults(&mut self, d_nr: u32, actual: &mut Vec<Value>, all_types: &mut Vec<Type>) {
        // When filling extra attrs for a recursive self-call on the second pass, use a
        // separate __rref_N counter so we don't consume __ref_N slots that the outer
        // function's return-value work-ref needs to keep the same name it had on the
        // first pass (allowing ref_return to find the name match instead of adding a
        // new attribute and growing the function's attr count across passes).
        let is_recursive_self = d_nr == self.context && !self.first_pass;
        // Extend to full parameter count so we can fill gaps from named arguments.
        while actual.len() < self.data.attributes(d_nr) {
            actual.push(Value::Null);
            all_types.push(Type::Unknown(0));
        }
        {
            // Fill all missing (Null) parameter slots with defaults.
            for a_nr in 0..self.data.attributes(d_nr) {
                if actual[a_nr] != Value::Null {
                    continue;
                }
                let default = self.data.def(d_nr).attributes[a_nr].value.clone();
                let tp = self.data.attr_type(d_nr, a_nr);
                if let Type::Vector(content, _) = &tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    let vr = if is_recursive_self {
                        self.vars.work_refs_recursive(&tp, &mut self.lexer)
                    } else {
                        self.vars.work_refs(&tp, &mut self.lexer)
                    };
                    self.data.vector_def(&mut self.lexer, content);
                    all_types[a_nr] = Type::Vector(content.clone(), vec![vr]);
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::Reference(content, _) = tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    let vr = if is_recursive_self {
                        self.vars.work_refs_recursive(&tp, &mut self.lexer)
                    } else {
                        self.vars.work_refs(&tp, &mut self.lexer)
                    };
                    all_types[a_nr] = Type::Reference(content, vec![vr]);
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::RefVar(vtp) = &tp {
                    let mut ls = Vec::new();
                    let vr = if matches!(**vtp, Type::Text(_)) {
                        let wv = self.vars.work_text(&mut self.lexer);
                        // clear the work buffer before each call so loop
                        // iterations start fresh (matches fn-ref path in control.rs).
                        ls.push(v_set(wv, Value::Text(String::new())));
                        if default != Value::Null
                            && if let Value::Text(t) = &default {
                                !t.is_empty()
                            } else {
                                true
                            }
                        {
                            ls.push(self.cl("OpAppendText", &[Value::Var(wv), default]));
                        }
                        wv
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unexpected reference type {}",
                            vtp.name(&self.data)
                        );
                        0
                    };
                    ls.push(self.cl("OpCreateStack", &[Value::Var(vr)]));
                    actual[a_nr] = v_block(
                        ls,
                        Type::Reference(self.data.def_nr("reference"), vec![vr]),
                        "default ref",
                    );
                    all_types[a_nr] = tp.clone();
                } else {
                    // default expressions may reference earlier
                    // parameters by `Var(N)` slots (e.g. `b: integer = a * 2`
                    // produces a tree with `Var(0)`).  Substitute those
                    // references with the caller's actual argument values
                    // so the emitted code uses the caller's scope, not
                    // the callee's (which wouldn't resolve at the call
                    // site).  Only parameters 0..a_nr are earlier; no
                    // recursion into the current or later default.
                    let substituted = Self::substitute_param_refs(default, &actual[..a_nr]);
                    actual[a_nr] = substituted;
                    all_types[a_nr] = tp.clone();
                }
            }
        }
    }

    /// replace `Value::Var(from)` with `Value::Var(to)` throughout
    /// a default-expression tree.  Used by `parse_arguments` to rewrite
    /// internally-allocated slot numbers into stable argument indices
    /// before the default is stored on the function definition.
    pub(crate) fn remap_var_nr(val: Value, from: u16, to: u16) -> Value {
        match val {
            Value::Var(n) if n == from => Value::Var(to),
            Value::Call(op, xs) => Value::Call(
                op,
                xs.into_iter()
                    .map(|x| Self::remap_var_nr(x, from, to))
                    .collect(),
            ),
            Value::CallRef(op, xs) => Value::CallRef(
                op,
                xs.into_iter()
                    .map(|x| Self::remap_var_nr(x, from, to))
                    .collect(),
            ),
            Value::Set(v, inner) => {
                let v = if v == from { to } else { v };
                Value::Set(v, Box::new(Self::remap_var_nr(*inner, from, to)))
            }
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|x| Self::remap_var_nr(x, from, to))
                    .collect(),
            ),
            other => other,
        }
    }

    /// replace `Value::Var(i)` for `i < args.len()` with `args[i]`
    /// in a default-expression tree.  Used at call sites to transplant a
    /// default's earlier-parameter references into the caller's scope.
    fn substitute_param_refs(val: Value, args: &[Value]) -> Value {
        match val {
            Value::Var(n) if (n as usize) < args.len() => args[n as usize].clone(),
            Value::Call(op, xs) => Value::Call(
                op,
                xs.into_iter()
                    .map(|x| Self::substitute_param_refs(x, args))
                    .collect(),
            ),
            Value::CallRef(op, xs) => Value::CallRef(
                op,
                xs.into_iter()
                    .map(|x| Self::substitute_param_refs(x, args))
                    .collect(),
            ),
            Value::Set(v, inner) => {
                Value::Set(v, Box::new(Self::substitute_param_refs(*inner, args)))
            }
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|x| Self::substitute_param_refs(x, args))
                    .collect(),
            ),
            Value::Span(b) => {
                let (pos, inner) = *b;
                let new_inner = Self::substitute_param_refs(inner, args);
                Value::with_span(pos, new_inner)
            }
            other => other,
        }
    }
    // ********************
    // * Parser functions *
    // ********************

    /// Parse data from the current lexer.
    #[allow(clippy::too_many_lines)] // two-pass parser dispatch — splitting would lose context
    fn parse_file(&mut self) {
        let start_def = self.data.definitions();
        while self.lexer.has_token("use") {
            if let Some(id) = self.lexer.has_identifier() {
                // Parse optional import spec: `::*` for wildcard or `::name1, name2` for selective.
                let spec = if self.lexer.has_token("::") {
                    if self.lexer.has_token("*") {
                        Some(ImportSpec::Wildcard)
                    } else {
                        let mut names = Vec::new();
                        if let Some(name) = self.lexer.has_identifier() {
                            names.push(name);
                            while self.lexer.has_token(",") {
                                if let Some(name) = self.lexer.has_identifier() {
                                    names.push(name);
                                }
                            }
                        }
                        if names.is_empty() {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Expected name or '*' after '::'"
                            );
                            None
                        } else {
                            Some(ImportSpec::Names(names))
                        }
                    }
                } else {
                    None
                };
                if self.data.use_exists(&id) {
                    let lib_source = self.data.get_source(&id);
                    // Plain `use foo` (no ::* or ::names) implicitly imports
                    // all pub definitions so they are visible in this source.
                    let import_spec = spec.unwrap_or(ImportSpec::Wildcard);
                    self.pending_imports.push(PendingImport {
                        for_source: self.data.source,
                        lib_source,
                        spec: import_spec,
                    });
                    if !self.lexer.has_token(";") {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Missing ';' after 'use {id}' — use statements must end with a semicolon"
                        );
                    }
                    continue;
                }
                let f = self.lib_path(&id);
                let f_exists = std::path::Path::new(&f).exists() || {
                    #[cfg(feature = "wasm")]
                    {
                        crate::wasm::virt_fs_get(&f).is_some()
                    }
                    #[cfg(not(feature = "wasm"))]
                    {
                        false
                    }
                };
                if f_exists {
                    let cur = &self.lexer.pos().file;
                    self.todo_files.push((cur.clone(), self.data.source));
                    self.data.use_add(&id);
                    // spec is consumed (tokens already read); the import will be recorded
                    // when this `use` statement is seen again via todo_files with use_exists=true.
                    drop(spec);
                    self.lexer.switch(&f);
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Library '{id}' not found — searched lib/, lib_dirs, and sibling packages"
                    );
                    self.lexer.has_token(";");
                }
            }
        }
        // PKG.3: load transitive dependencies discovered during manifest reading.
        // Dependencies are queued by lib_path_manifest when it reads [dependencies].
        while !self.pending_pkg_deps.is_empty() {
            let deps = std::mem::take(&mut self.pending_pkg_deps);
            for (dep_id, parent_dir) in deps {
                if self.data.use_exists(&dep_id) {
                    continue;
                }
                // First try the sibling package directory (same parent as the
                // depending package), then fall back to the normal lib_path search.
                let f = if let Some(entry) = self.lib_path_manifest(&parent_dir, &dep_id) {
                    entry
                } else {
                    self.lib_path(&dep_id)
                };
                if std::path::Path::new(&f).exists() {
                    let cur = &self.lexer.pos().file;
                    self.todo_files.push((cur.clone(), self.data.source));
                    self.data.use_add(&dep_id);
                    self.lexer.switch(&f);
                }
            }
        }
        // Apply wildcard/selective imports queued for this source now that the while-use loop
        // has resolved all libraries.  Must run before the definitions loop so that imported
        // names are visible when function bodies and type annotations are parsed.
        self.apply_pending_imports();
        self.file += 1;
        self.line = 0;
        loop {
            let is_pub = self.lexer.has_token("pub");
            let before = self.data.definitions();
            if self.lexer.diagnostics().level() == Level::Fatal
                || (!self.parse_enum()
                    && !self.parse_typedef()
                    && !self.parse_function()
                    && !self.parse_struct()
                    && !self.parse_interface()
                    && !self.parse_constant())
            {
                break;
            }
            // mark newly created definitions as pub-visible.
            if is_pub {
                for d_nr in before..self.data.definitions() {
                    self.data.def_mut(d_nr).pub_visible = true;
                }
            }
        }
        let res = self.lexer.peek();
        if res.has != LexItem::None && self.lexer.diagnostics().level() != Level::Fatal {
            if self.lexer.peek_token("use") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "use statements must appear before all definitions"
                );
            } else {
                let token = match &res.has {
                    crate::lexer::LexItem::Token(s) | crate::lexer::LexItem::Identifier(s) => {
                        format!("'{s}'")
                    }
                    crate::lexer::LexItem::CString(s) => format!("\"{s}\""),
                    other => format!("{other:?}"),
                };
                diagnostic!(self.lexer, Level::Error, "Syntax error: unexpected {token}");
            }
        }
        // defer `Undefined type` errors to `resolve_deferred_unknowns`
        // so forward-references across cyclic intra-package `use` declarations
        // get a chance to resolve once both sides of the cycle are registered.
        typedef::actual_types_deferred(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
            Some(&mut self.deferred_unknown),
        );
        typedef::fill_all(
            &mut self.data,
            &mut self.database,
            &mut self.lexer,
            start_def,
        );
        self.database.finish();
        // Validate layouts of all registered types — catches late-
        // mutation bugs (e.g. P191's bookkeeping fields landing at
        // overlapping positions because finish_type already ran).
        // Skip when the parser has already reported errors: an
        // incomplete struct (e.g. a self-referential type that the
        // parser correctly rejected) can have unlaid fields whose
        // position == u16::MAX, and re-flagging that here just adds
        // noise on top of the real diagnostic.
        let already_failed = matches!(
            self.lexer.diagnostics().level(),
            Level::Error | Level::Fatal
        );
        if !already_failed {
            for issue in self.database.validate_all_layouts() {
                diagnostic!(self.lexer, Level::Error, "type layout: {}", issue);
            }
        }
        self.enum_fn();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            return;
        }
        // Parse all files left in the todo_files list, as they are halted to parse a use file.
        while let Some((t, s)) = self.todo_files.pop() {
            self.lexer.switch(&t);
            self.data.source = s;
            self.parse_file();
        }
    }

    /// Apply all pending imports whose target source matches the currently active source.
    fn apply_pending_imports(&mut self) {
        let cur = self.data.source;
        // Partition: imports targeting `cur` are applied now; others wait for their source.
        let mut to_apply = Vec::new();
        let mut remaining = Vec::new();
        for pi in self.pending_imports.drain(..) {
            if pi.for_source == cur {
                to_apply.push(pi);
            } else {
                remaining.push(pi);
            }
        }
        self.pending_imports = remaining;
        for pi in to_apply {
            // retain a copy so `resolve_deferred_unknowns` can re-apply
            // with overwrite semantics after a cyclic `use` has finished
            // registering the partner file's definitions.
            self.applied_imports.push(pi.clone());
            match pi.spec {
                ImportSpec::Wildcard => {
                    self.data.import_all(pi.lib_source, cur);
                }
                ImportSpec::Names(names) => {
                    for name in &names {
                        if !self.data.import_name(pi.lib_source, cur, name) {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Name '{name}' not found in library"
                            );
                        }
                    }
                }
            }
        }
    }

    fn lib_path(&mut self, id: &str) -> String {
        // Under the `wasm` feature, VIRT_FS wins over filesystem lookups.
        #[cfg(feature = "wasm")]
        if crate::wasm::virt_fs_get(&format!("{id}.loft")).is_some() {
            return format!("{id}.loft");
        }

        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let cur_dir_owned = script_dir(&cur_script).to_string();
        let base_dir_owned = tests_base_dir(&cur_dir_owned).to_string();
        let cur_dir = cur_dir_owned.as_str();
        let base_dir = base_dir_owned.as_str();

        // Try each strategy in order; first one that finds the file wins.
        // `f` starts as the cheapest guess (project-local `lib/<id>.loft` or
        // `<id>.loft`); subsequent strategies overwrite only when `f` does
        // not yet resolve to an existing file.
        let mut f = Self::probe_project_lib(id);

        Self::probe_cur_dir_lib(id, cur_dir, &mut f);
        Self::probe_base_dir_lib(id, base_dir, &mut f);
        self.probe_sibling_package(id, cur_dir, &mut f);
        Self::probe_script_sibling_dir(id, &cur_script, &mut f);
        self.probe_cmdline_lib_dirs(id, &mut f);
        self.probe_cmdline_lib_dirs_manifest(id, &mut f);
        Self::probe_loft_lib_flat(id, &mut f);
        self.probe_loft_lib_manifest(id, &mut f);
        self.probe_user_installed(id, &mut f);
        Self::probe_cur_dir_flat(id, cur_dir, &mut f);
        Self::probe_base_dir_flat(id, base_dir, &mut f);

        f
    }

    /// Initial guess: the project-supplied `lib/<id>.loft`, falling back to
    /// `<id>.loft` in the current working directory.
    fn probe_project_lib(id: &str) -> String {
        let f = format!("lib{0}{id}.loft", sep_str());
        if std::path::Path::new(&f).exists() {
            f
        } else {
            format!("{id}.loft")
        }
    }

    /// `<cur_dir>/lib/<id>.loft` — lib dir relative to the script being parsed.
    fn probe_cur_dir_lib(id: &str, cur_dir: &str, f: &mut String) {
        if !cur_dir.is_empty() && !std::path::Path::new(f).exists() {
            *f = format!("{cur_dir}{0}lib{0}{id}.loft", sep_str());
        }
    }

    /// `<base_dir>/lib/<id>.loft` — lib dir relative to the base directory
    /// when the script lives inside a `/tests/` tree.
    fn probe_base_dir_lib(id: &str, base_dir: &str, f: &mut String) {
        if !base_dir.is_empty() && !std::path::Path::new(f).exists() {
            *f = format!("{base_dir}{0}lib{0}{id}.loft", sep_str());
        }
    }

    /// Walk up from `cur_dir` looking for a `loft.toml`; on hit, the package's
    /// parent directory may contain sibling packages.  When the sibling is
    /// found directly (not via `lib_path_manifest`), the sibling's own
    /// `loft.toml` must be registered so its `#native` symbols resolve.
    fn probe_sibling_package(&mut self, id: &str, cur_dir: &str, f: &mut String) {
        if std::path::Path::new(f).exists() || cur_dir.is_empty() {
            return;
        }
        let mut search_dir = std::path::Path::new(cur_dir).to_path_buf();
        loop {
            if search_dir.join("loft.toml").exists() {
                if let Some(parent) = search_dir.parent()
                    && let Some(path) = Self::find_sibling_file(parent, id)
                {
                    *f = path.to_string_lossy().to_string();
                    let pkg_root = parent.join(id);
                    let manifest = pkg_root.join("loft.toml");
                    if manifest.exists() {
                        self.register_native_manifest(&manifest, &pkg_root);
                    }
                }
                break;
            }
            let Some(p) = search_dir.parent() else {
                break;
            };
            search_dir = p.to_path_buf();
        }
    }

    /// Sibling lookup: prefer `<parent>/<id>/src/<id>.loft`, fall back to
    /// flat `<parent>/<id>.loft`.
    fn find_sibling_file(parent: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
        let nested = parent.join(id).join("src").join(format!("{id}.loft"));
        if nested.exists() {
            return Some(nested);
        }
        let flat = parent.join(format!("{id}.loft"));
        flat.exists().then_some(flat)
    }

    /// A directory named after the current script (minus the `.loft` suffix).
    fn probe_script_sibling_dir(id: &str, cur_script: &str, f: &mut String) {
        if !std::path::Path::new(f).exists() && cur_script.len() >= 5 {
            *f = format!(
                "{}{}{id}.loft",
                &cur_script[0..cur_script.len() - 5],
                sep_str()
            );
        }
    }

    /// `--lib` / `--project` command-line flag directories, flat layout.
    /// Registers any discovered `loft.toml` in the file's ancestry.
    fn probe_cmdline_lib_dirs(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let lib_dirs = self.lib_dirs.clone();
        for l in &lib_dirs {
            let candidate = format!("{l}{}{id}.loft", sep_str());
            if std::path::Path::new(&candidate).exists() {
                f.clone_from(&candidate);
                self.register_manifest_in_ancestors(&candidate);
                break;
            }
        }
    }

    /// Scan ancestor directories for a `loft.toml` and register the first one
    /// found.  Called when a file was resolved directly (not through
    /// `lib_path_manifest`), so its owning package's native crate would
    /// otherwise never be registered.
    fn register_manifest_in_ancestors(&mut self, candidate: &str) {
        let mut search = std::path::Path::new(candidate)
            .parent()
            .map(std::path::Path::to_path_buf);
        while let Some(dir) = search {
            let manifest = dir.join("loft.toml");
            if manifest.exists() {
                self.register_native_manifest(&manifest, &dir);
                return;
            }
            search = dir.parent().map(std::path::Path::to_path_buf);
        }
    }

    /// `--lib` / `--project` directories, packaged layout
    /// (`<dir>/<id>/src/<id>.loft`).
    fn probe_cmdline_lib_dirs_manifest(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let lib_dirs = self.lib_dirs.clone();
        for l in &lib_dirs {
            if let Some(entry) = self.lib_path_manifest(l, id) {
                *f = entry;
                return;
            }
        }
    }

    /// `LOFT_LIB` env var, flat layout (`<dir>/<id>.loft`).
    fn probe_loft_lib_flat(id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let Some(v) = env::var_os("LOFT_LIB") else {
            return;
        };
        for l in env::split_paths(&v) {
            let candidate = l.join(format!("{id}.loft"));
            if candidate.exists() {
                *f = candidate.to_string_lossy().replace(other_sep(), sep_str());
                return;
            }
        }
    }

    /// `LOFT_LIB` env var, packaged layout (via `lib_path_manifest`).
    fn probe_loft_lib_manifest(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let Some(v) = env::var_os("LOFT_LIB") else {
            return;
        };
        for l in env::split_paths(&v) {
            let l = l.to_string_lossy().replace(other_sep(), sep_str());
            if let Some(entry) = self.lib_path_manifest(&l, id) {
                *f = entry;
                return;
            }
        }
    }

    /// `~/.loft/lib/<id>/src/<id>.loft` — packages installed via `loft install`.
    fn probe_user_installed(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let user_lib = format!("{home}/.loft/lib");
        if let Some(entry) = self.lib_path_manifest(&user_lib, id) {
            *f = entry;
        }
    }

    /// Final fallback: beside the parsed file itself.
    fn probe_cur_dir_flat(id: &str, cur_dir: &str, f: &mut String) {
        if !cur_dir.is_empty() && !std::path::Path::new(f).exists() {
            *f = format!("{cur_dir}{0}{id}.loft", sep_str());
        }
    }

    /// Final fallback for scripts inside a `/tests/` tree.
    fn probe_base_dir_flat(id: &str, base_dir: &str, f: &mut String) {
        if !base_dir.is_empty() && !std::path::Path::new(f).exists() {
            *f = format!("{base_dir}{0}{id}.loft", sep_str());
        }
    }

    /// Register native crate info from a loft.toml manifest.
    /// Called when a .loft file was found directly via lib_dirs (not through
    /// lib_path_manifest), so the manifest's native crate registration would
    /// otherwise be skipped.
    fn register_native_manifest(
        &mut self,
        manifest_path: &std::path::Path,
        pkg_dir: &std::path::Path,
    ) {
        let Some(m) = manifest::read_manifest(manifest_path.to_str().unwrap_or("")) else {
            return;
        };
        let pkg_dir = pkg_dir.to_string_lossy().to_string();
        if let Some(ref crate_name) = m.native_crate {
            let rust_crate = crate_name.replace('-', "_");
            if !self
                .data
                .native_packages
                .iter()
                .any(|(c, _)| c == crate_name)
            {
                self.data
                    .native_packages
                    .push((crate_name.clone(), pkg_dir));
            }
            // Map all #native symbols from already-parsed definitions to this crate.
            for d_nr in 0..self.data.definitions() {
                let sym = &self.data.def(d_nr).native;
                if !sym.is_empty() && !self.data.native_symbol_crates.contains_key(sym) {
                    self.data
                        .native_symbol_crates
                        .insert(sym.clone(), rust_crate.clone());
                }
            }
        }
    }

    /// Check whether `<dir>/<id>` contains a valid loft package layout.
    /// Reads `loft.toml` when present and validates the interpreter version
    /// requirement.  Emits a fatal diagnostic on version mismatch.
    /// Returns `Some(entry_path)` when the layout exists and the version passes,
    /// `None` otherwise.
    ///
    /// Legacy path: delegates to [`lib_path_manifest_resolve`] for pure
    /// resolution, then applies side-effects via [`apply_manifest_side_effects`].
    /// Phase A calls `lib_path_manifest_resolve`
    /// directly and builds the package graph explicitly.
    fn lib_path_manifest(&mut self, dir: &str, id: &str) -> Option<String> {
        let resolved = self.lib_path_manifest_resolve(dir, id)?;
        if let Some(m) = resolved.manifest.as_ref() {
            self.apply_manifest_side_effects(dir, &resolved.pkg_dir, m);
        }
        Some(resolved.entry)
    }

    /// Pure resolution of `<dir>/<id>` against disk + manifest.  No side
    /// effects on `self.data` / `self.lib_dirs` / `self.pending_*`; the only
    /// state touched is `self.lexer.diagnostics` on a version-mismatch fatal.
    ///
    /// This is the entry point used by Phase A of the package-mode driver
    /// , which needs to enumerate files + package edges without
    /// spilling symbol-table side-effects before pass-1 parsing begins.
    fn lib_path_manifest_resolve(&mut self, dir: &str, id: &str) -> Option<ResolvedPkg> {
        let pkg_dir = format!("{dir}/{id}");
        if !std::path::Path::new(&pkg_dir).is_dir() {
            return None;
        }
        let manifest_path = format!("{pkg_dir}/loft.toml");
        let (entry, manifest) = if std::path::Path::new(&manifest_path).exists() {
            let m = manifest::read_manifest(&manifest_path)?;
            if let Some(ref req) = m.loft_version {
                let current = env!("CARGO_PKG_VERSION");
                if !manifest::check_version(req, current) {
                    diagnostic!(
                        self.lexer,
                        Level::Fatal,
                        "Package '{id}' requires loft {req} but interpreter is {current}"
                    );
                    return None;
                }
            }
            let entry = m.entry.as_ref().map_or_else(
                || format!("{pkg_dir}/src/{id}.loft"),
                |e| format!("{pkg_dir}/{e}"),
            );
            (entry, Some(m))
        } else {
            (format!("{pkg_dir}/src/{id}.loft"), None)
        };
        if std::path::Path::new(&entry).exists() {
            Some(ResolvedPkg {
                pkg_dir,
                entry,
                manifest,
            })
        } else {
            None
        }
    }

    /// Apply the parser-state side effects that the legacy `lib_path_manifest`
    /// performs for a resolved package: native-lib registration,
    /// native-symbol / native-crate bookkeeping, sibling-dependency search
    /// paths (`lib_dirs`), and queued transitive package loads
    /// (`pending_pkg_deps`).
    fn apply_manifest_side_effects(&mut self, dir: &str, pkg_dir: &str, m: &manifest::Manifest) {
        // register native shared library path for loading after byte_code().
        // Try pre-built location first, then auto-build from source.
        if let Some(ref stem) = m.native {
            let filename = crate::extensions::platform_lib_name(stem);
            let prebuilt = format!("{pkg_dir}/native/{filename}");
            if std::path::Path::new(&prebuilt).exists() {
                self.pending_native_libs.push(prebuilt);
            } else if let Some(built) = crate::extensions::auto_build_native(pkg_dir, stem) {
                self.pending_native_libs.push(built);
            }
        }
        // PKG.4: register native function symbols and package crate info.
        if let Some(ref crate_name) = m.native_crate {
            let rust_crate = crate_name.replace('-', "_");
            if !self
                .data
                .native_packages
                .iter()
                .any(|(c, _)| c == crate_name)
            {
                self.data
                    .native_packages
                    .push((crate_name.clone(), pkg_dir.to_string()));
            }
            for (loft_name, rust_symbol) in &m.native_functions {
                self.data
                    .native_symbols
                    .insert(loft_name.clone(), rust_symbol.clone());
            }
            // Map all #native symbols from this package to their crate.
            // Definitions parsed so far include this package's functions.
            for d_nr in 0..self.data.definitions() {
                let sym = &self.data.def(d_nr).native;
                if !sym.is_empty() && !self.data.native_symbol_crates.contains_key(sym) {
                    self.data
                        .native_symbol_crates
                        .insert(sym.clone(), rust_crate.clone());
                }
            }
        }
        // PKG.3: register the package's parent directory so that
        // dependencies declared in [dependencies] can be found as sibling
        // packages during normal `use` resolution.
        if !m.dependencies.is_empty() && !self.lib_dirs.contains(&dir.to_string()) {
            self.lib_dirs.push(dir.to_string());
        }
        for (dep_name, _dep_version) in &m.dependencies {
            if !self.data.use_exists(dep_name) {
                self.pending_pkg_deps
                    .push((dep_name.clone(), dir.to_string()));
            }
        }
    }

    /// Plan-07 phase 5 suggestions.  Find a similar user-defined
    /// function name (typed without the `n_` prefix) that might be
    /// the correct spelling.  Returns `None` when no candidate is
    /// within Levenshtein distance 2.
    ///
    /// Uses the same `suggest_similar` primitive as the existing
    /// variable-suggestion path at `parser/objects.rs::known_var_or_type`.
    pub fn suggest_function_name(&self, name: &str) -> Option<String> {
        let candidates_owned: Vec<String> = self
            .data
            .user_fn_d_nrs()
            .iter()
            .filter_map(|&d_nr| {
                let n = &self.data.def(d_nr).name;
                if let Some(stripped) = n.strip_prefix("n_") {
                    // Skip synthetic lambda names — they're not user-typeable.
                    if stripped.starts_with("__lambda_") {
                        None
                    } else {
                        Some(stripped.to_string())
                    }
                } else {
                    None
                }
            })
            .collect();
        let candidates: Vec<&str> = candidates_owned.iter().map(String::as_str).collect();
        crate::diagnostics::suggest_similar_capped(name, &candidates).map(String::from)
    }

    /// Plan-07 phase 5 suggestions.  Find a similar field name on
    /// the given struct definition.  Skips synthetic compiler-
    /// generated attributes (those starting with `_` or `#`).
    pub fn suggest_field_name(&self, struct_d_nr: u32, name: &str) -> Option<String> {
        if struct_d_nr == u32::MAX {
            return None;
        }
        let candidates_owned: Vec<&str> = self
            .data
            .def(struct_d_nr)
            .attributes
            .iter()
            .filter_map(|a| {
                if a.name.starts_with('_') || a.name.starts_with('#') {
                    None
                } else {
                    Some(a.name.as_str())
                }
            })
            .collect();
        crate::diagnostics::suggest_similar_capped(name, &candidates_owned).map(String::from)
    }

    /// Plan-07 phase 5 suggestions.  Find a similar type name —
    /// thin wrapper around `Data::suggest_type_name` so callers in
    /// the parser don't have to thread `self.data` explicitly.
    pub fn suggest_type_name(&self, name: &str) -> Option<String> {
        self.data.suggest_type_name(name)
    }

    // Determine if there need to be special enum functions that call enum_value variants.
    pub fn create_var(&mut self, name: &str, var_type: &Type) -> u16 {
        if self.context == u32::MAX {
            return u16::MAX;
        }
        self.vars.add_variable(name, var_type, &mut self.lexer)
    }

    fn create_unique(&mut self, name: &str, var_type: &Type) -> u16 {
        self.vars.unique(name, var_type, &mut self.lexer)
    }

    fn var_usages(&mut self, vnr: u16, plus: bool) {
        if vnr == u16::MAX {
            return;
        }
        if plus {
            self.vars.in_use(vnr, true);
        } else if self.vars.uses(vnr) > 0 {
            self.vars.in_use(vnr, false);
        }
    }

    /// check whether a value is "addressable" — rooted in a Var and
    /// reached through field access (OpGetField) or vector element access
    /// (OpGetVector / OpVectorRef) chains.  Addressable values produce a
    /// DbRef into existing mutable storage, so they are safe to pass as
    /// `&` parameters.
    fn is_addressable(val: &Value, data: &Data) -> bool {
        // Plan-07 phase 1: unspan() so wraps on `[` / `.` (steps 1.11
        // / 1.12) don't hide an addressable shape from the `&` arg
        // check.
        match val.unspan() {
            Value::Var(_) => true,
            Value::Call(d_nr, args) => {
                let name = &data.def(*d_nr).name;
                (name == "OpGetField" || name == "OpGetVector" || name == "OpVectorRef")
                    && !args.is_empty()
                    && Self::is_addressable(&args[0], data)
            }
            _ => false,
        }
    }

    /// Plan-06 PRIORITY.md spine step 5 — par-result use-site analyser.
    ///
    /// After the function body is fully parsed, find each
    /// `let r = parallel_for(...)` (or fused `for x in input par(r=...,
    /// N) { body }` whose result-vector flows into the body), and walk
    /// the body for uses of `r`.  Each use is classified:
    ///
    /// - **Streaming-eligible**: the result is iterated once via `for x
    ///   in r { … }` — lowers to `Stitch::Queue` in spine step 5b.
    /// - **Materialising**: random access (`r[i]`), length-of-filter,
    ///   multi-pass, alias (`r2 = r`), passed as `vector<S>` arg, stored
    ///   in `vector<S>` field, returned from `-> vector<S>` fn.  These
    ///   require the materialised vector (today's path) and will fail
    ///   compilation in spine step 7 unless rewritten via the explicit
    ///   `par_to_vec(...)` helper (planned phase 11).
    ///
    /// Spine step 7 (DONE 2026-04-29): emits `Level::Error` on
    /// materialising uses.  The deprecation window from step 5
    /// (warning) closed once the audit in step 6 confirmed zero
    /// corpus sites trigger it.  Materialising par results now fail
    /// to compile.  Streaming-eligible patterns (single
    /// `for x in r {}` use) stay silent and keep working through the
    /// existing materialised path until step 8 lands the streaming
    /// rewrite.
    fn check_par_result_singlepass(&mut self) {
        let par_for_d_nr = self.data.def_nr("n_parallel_for");
        if par_for_d_nr == u32::MAX || self.context == u32::MAX {
            return;
        }
        let body = self.data.def(self.context).code.clone();
        // Find each (var, par_call_pos) where `Set(var, Call(n_parallel_for, ...))`.
        let mut par_results: Vec<u16> = Vec::new();
        Self::collect_par_assignments(&body, par_for_d_nr, &mut par_results);
        for v in par_results {
            // Walk the body counting uses of v.  Each use lands in a
            // category — `streaming` (Iter init position) or `other`
            // (everything else, treated as materialising for the warn
            // policy).
            let mut streaming = 0usize;
            let mut other = 0usize;
            Self::classify_var_uses(&body, v, &mut streaming, &mut other);
            // The `Set(v, par_call)` itself counts as a use.  Subtract
            // exactly one read (the par-call's appearance in arg-position
            // of the assignment is an artifact of how Set is encoded —
            // some encodings do, some don't; conservative: don't double
            // count by subtracting the assignment read).
            //
            // Heuristic: if `other > 0` AND the only `streaming` use is
            // 0, the result is purely materialising — warn.  If
            // `streaming` >= 1 and `other` >= 1, mixed-use — warn (one
            // read can't be both streamed and materialised).
            let var_name = self.vars.name(v).to_string();
            // Skip compiler-generated bindings: the fused for-par
            // desugar in `parse_parallel_for_loop` emits a hidden
            // `_par_results_N` var that's bound to a parallel_for
            // call and then indexed per iteration.  That's the
            // existing materialised path we WILL retire (step 8),
            // but warning on the compiler's own desugaring spams
            // every fused for-par site with output the user can't
            // act on.  Names starting with `_` are unique-counter
            // generated; user-typed bindings never start with `_`.
            if var_name.starts_with('_') {
                continue;
            }
            if streaming == 0 && other == 0 {
                // Result is bound but never read — par dispatched for
                // worker side effects only.  Suggest the explicit
                // form so the materialised result vector isn't
                // allocated for nothing.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par result '{var_name}' is never read — the materialised \
                     result vector is allocated but unused.  Use a fused `for x \
                     in input par(_=fn(x), N) {{}}` loop with discard policy \
                     (plan-06 spine step 3c) instead, or remove the assignment \
                     if the worker has no observable side effects."
                );
            } else if other > 0 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par result '{var_name}' is used in a materialising context \
                     (random access, multi-pass, or stored as vector<S>) — \
                     phase 10 of plan-06 will require single-pass consumption.  \
                     Either rewrite to a fused `for x in input par(r=fn(x), N) {{ body }}` \
                     loop, or call `par_to_vec(input, fn, N)` (planned phase 11) \
                     for an explicit materialised vector.  \
                     {streaming} streaming use(s), {other} materialising use(s) detected."
                );
            }
            // streaming == 1 && other == 0 — single-pass eligible.
            // Step 5b's lowering would rewrite to a fused for-par
            // dispatching through `run_parallel_queue` (no result
            // vector).  Deferred — bundles with step 8 (Concat
            // retirement) where the runtime question becomes
            // concrete and the rewrite has a clear destination IR.
            // For now this case is silent: existing code keeps
            // working through the materialised path.
        }
    }

    /// Plan-06 spine step 5 — find each `Set(v, Call(par_for_d_nr, ...))`
    /// occurrence in the IR tree and push `v` into `result`.  Recurses
    /// through every compound variant (Block / Loop / If / Insert /
    /// Iter / Span / ParFor) so a par assignment buried inside an `if`
    /// branch or a sub-block is still found.
    fn collect_par_assignments(val: &Value, par_for_d_nr: u32, result: &mut Vec<u16>) {
        match val {
            Value::Set(v, inner) => {
                if let Value::Call(d, _) = inner.as_ref().unspan()
                    && *d == par_for_d_nr
                {
                    result.push(*v);
                }
                Self::collect_par_assignments(inner, par_for_d_nr, result);
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    Self::collect_par_assignments(op, par_for_d_nr, result);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::collect_par_assignments(op, par_for_d_nr, result);
                }
            }
            Value::If(c, t, e) => {
                Self::collect_par_assignments(c, par_for_d_nr, result);
                Self::collect_par_assignments(t, par_for_d_nr, result);
                Self::collect_par_assignments(e, par_for_d_nr, result);
            }
            Value::Iter(_, a, b, c) => {
                Self::collect_par_assignments(a, par_for_d_nr, result);
                Self::collect_par_assignments(b, par_for_d_nr, result);
                Self::collect_par_assignments(c, par_for_d_nr, result);
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::collect_par_assignments(a, par_for_d_nr, result);
                }
            }
            Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
                Self::collect_par_assignments(v, par_for_d_nr, result);
            }
            Value::BreakWith(_, v) | Value::TuplePut(_, _, v) => {
                Self::collect_par_assignments(v, par_for_d_nr, result);
            }
            Value::Span(b) => Self::collect_par_assignments(&b.1, par_for_d_nr, result),
            Value::ParFor(b) => {
                Self::collect_par_assignments(&b.input, par_for_d_nr, result);
                Self::collect_par_assignments(&b.worker, par_for_d_nr, result);
                Self::collect_par_assignments(&b.threads, par_for_d_nr, result);
                Self::collect_par_assignments(&b.body, par_for_d_nr, result);
            }
            _ => {}
        }
    }

    /// Plan-06 spine step 5 — classify each `Value::Var(v)` read in the
    /// IR tree.  A read inside the `init` arm of a `Value::Iter` (the
    /// collection being iterated) counts as **streaming**; every other
    /// read counts as **other** (materialising).
    ///
    /// Counts are accumulated into the caller's mutable refs.  The
    /// `Set(v, …)` site that introduced `v` is intentionally NOT a
    /// read (Set's first field is a write target, not a read).
    fn classify_var_uses(val: &Value, v: u16, streaming: &mut usize, other: &mut usize) {
        match val {
            Value::Var(u) if *u == v => {
                *other += 1;
            }
            Value::Var(_) => {}
            Value::Iter(_, init, next, extra) => {
                // Iter's init is the iterable expression.  A bare
                // `Var(v)` in init means `for x in v { … }` — count
                // as streaming and don't recurse into the var read.
                if let Value::Var(u) = init.as_ref()
                    && *u == v
                {
                    *streaming += 1;
                } else {
                    Self::classify_var_uses(init, v, streaming, other);
                }
                Self::classify_var_uses(next, v, streaming, other);
                Self::classify_var_uses(extra, v, streaming, other);
            }
            Value::Set(_, inner) => Self::classify_var_uses(inner, v, streaming, other),
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::classify_var_uses(a, v, streaming, other);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    Self::classify_var_uses(op, v, streaming, other);
                }
            }
            Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
                for op in ops {
                    Self::classify_var_uses(op, v, streaming, other);
                }
            }
            Value::If(c, t, e) => {
                Self::classify_var_uses(c, v, streaming, other);
                Self::classify_var_uses(t, v, streaming, other);
                Self::classify_var_uses(e, v, streaming, other);
            }
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                Self::classify_var_uses(inner, v, streaming, other);
            }
            Value::BreakWith(_, inner) | Value::TuplePut(_, _, inner) => {
                Self::classify_var_uses(inner, v, streaming, other);
            }
            Value::Span(b) => Self::classify_var_uses(&b.1, v, streaming, other),
            Value::ParFor(b) => {
                // The input position is streaming-equivalent (par
                // dispatcher iterates).  worker/threads/body are
                // ordinary expression contexts.
                if let Value::Var(u) = &b.input
                    && *u == v
                {
                    *streaming += 1;
                } else {
                    Self::classify_var_uses(&b.input, v, streaming, other);
                }
                Self::classify_var_uses(&b.worker, v, streaming, other);
                Self::classify_var_uses(&b.threads, v, streaming, other);
                Self::classify_var_uses(&b.body, v, streaming, other);
            }
            _ => {}
        }
    }

    /// After parsing a function body, check that each `&` (`RefVar`) argument is actually
    /// mutated somewhere in the body. If not, emit a compile error suggesting to drop the `&`.
    /// Also check for redundant `const` annotations on primitive parameters that are never
    /// written to — the `const` has no effect when the parameter is not modified.
    fn check_ref_mutations(&mut self, arguments: &[Argument]) {
        let code = self.data.def(self.context).code.clone();
        let mut written: HashSet<u16> = HashSet::new();
        // interprocedural param-write cache, local to this check.
        // Re-created per function-body check; small cost, avoids
        // persisting state across passes or across unrelated checks.
        let mut callee_cache: HashMap<u32, Vec<bool>> = HashMap::new();
        find_written_vars(&code, &self.data, &mut written, &mut callee_cache);
        // Enhancement: when a for-loop variable is FIELD-WRITTEN (OpSet*
        // through the loop var, not just loop-advance Set), also mark the
        // collection it iterates over as written.  The dep chain is:
        //   it: ref(T)[_vector_1]  →  _vector_1: vector<T>[items]
        // Only propagate for vars that have a field-level write (OpSet*,
        // OpCopyRecord, OpNewRecord etc.) — not plain Set (which is just
        // the loop-iterator advance).
        let mut field_written: HashSet<u16> = HashSet::new();
        find_field_written_vars(&code, &self.data, &mut field_written);
        let mut propagated: HashSet<u16> = HashSet::new();
        for &w in &field_written {
            if w < self.vars.next_var() {
                for dep in self.vars.tp(w).depend() {
                    propagated.insert(dep);
                    if dep < self.vars.next_var() {
                        for dep2 in self.vars.tp(dep).depend() {
                            propagated.insert(dep2);
                        }
                    }
                }
            }
        }
        written.extend(propagated);
        for (a_nr, a) in arguments.iter().enumerate() {
            if matches!(a.typedef, Type::RefVar(_))
                && !a.constant
                && !written.contains(&(a_nr as u16))
            {
                let src = self.vars.var_source(a_nr as u16);
                self.lexer.to(src);
                // T1.6: RefVar(Tuple) — downgrade to warning since elements are stack values;
                // other RefVar types are an error (the & serves no purpose and misleads).
                if matches!(a.typedef, Type::RefVar(ref inner) if matches!(**inner, Type::Tuple(_)))
                {
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        "Parameter '{}' does not need to be a reference",
                        a.name
                    );
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Parameter '{}' has & but is never modified; remove the &",
                        a.name
                    );
                }
            }
            // warn when `const` is used on a primitive parameter that is never
            // written to — the annotation is redundant since the parameter would not
            // have been modified anyway.  Compound types (vector, reference, struct)
            // are exempt: `const` serves as read-only documentation on those.
            let base_tp = if let Type::RefVar(inner) = &a.typedef {
                inner.as_ref()
            } else {
                &a.typedef
            };
            if a.constant
                && !written.contains(&(a_nr as u16))
                && matches!(
                    base_tp,
                    Type::Integer(_) | Type::Float | Type::Single | Type::Boolean | Type::Character
                )
            {
                let src = self.vars.var_source(a_nr as u16);
                self.lexer.to(src);
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    "Parameter '{}' is const but is never modified; \
                     'const' has no effect on an unmodified primitive parameter",
                    a.name
                );
            }
        }
    }

    // <function> ::= 'fn' <identifier> '(' <attributes> ] [ '->' <type> ] (';' <rust> | <code>)
    pub fn null(&mut self, tp: &Type) -> Value {
        match tp {
            Type::Integer(_) | Type::Character => self.cl("OpConvIntFromNull", &[]),
            Type::Boolean => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot use null with boolean — boolean has no null representation"
                    );
                }
                Value::Boolean(false)
            }
            Type::Enum(tp, _, _) => self.cl(
                "OpConvEnumFromNull",
                &[Value::Int(i32::from(self.data.def(*tp).known_type))],
            ),
            Type::Float => self.cl("OpConvFloatFromNull", &[]),
            Type::Single => self.cl("OpConvSingleFromNull", &[]),
            Type::Text(_) => self.cl("OpConvTextFromNull", &[]),
            Type::RefVar(tp) if matches!(**tp, Type::Text(_)) => self.cl("OpConvTextFromNull", &[]),
            Type::Reference(_, _) => self.cl("OpNullRefSentinel", &[]),
            _ => Value::Null,
        }
    }

    // For now, assume that returned texts are always related to internal variables
}

/// Directory portion of a normalised script path, or `""` if the path is bare.
fn script_dir(cur_script: &str) -> &str {
    cur_script.rfind(sep()).map_or("", |p| &cur_script[0..p])
}

/// If `cur_dir` lives inside a `/tests/` tree, return the ancestor above
/// `tests/`; otherwise `""`.  Used to locate the project-root `lib/`
/// directory when running a script from `tests/`.
fn tests_base_dir(cur_dir: &str) -> &str {
    let tests_infix = format!("{0}tests{0}", sep());
    if let Some(idx) = cur_dir.find(tests_infix.as_str()) {
        &cur_dir[..idx]
    } else {
        ""
    }
}

fn merge_dependencies(a: &Type, b: &Type) -> Type {
    // Never (return/break/continue) defers to the other branch's type.
    if matches!(a, Type::Never) {
        return b.clone();
    }
    if matches!(b, Type::Never) {
        return a.clone();
    }
    if let (Type::Text(da), Type::Text(db)) = (a, b) {
        let mut d = HashSet::new();
        for v in da {
            d.insert(*v);
        }
        for v in db {
            d.insert(*v);
        }
        Type::Text(d.into_iter().collect())
    } else {
        a.clone()
    }
}

fn field_id(key: &[(String, bool)], name: &mut String) {
    for (k_nr, (k, asc)) in key.iter().enumerate() {
        if k_nr > 0 {
            *name += ",";
        }
        if !asc {
            *name += "-";
        }
        *name += k;
    }
    *name += "]>";
}

/// Collect all `Value::Var` indices reachable anywhere in `val`.
fn collect_vars_in(val: &Value, result: &mut HashSet<u16>) {
    match val {
        Value::Var(v) => {
            result.insert(*v);
        }
        Value::Set(_, body) => collect_vars_in(body, result),
        Value::Call(_, args) => {
            for a in args {
                collect_vars_in(a, result);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                collect_vars_in(op, result);
            }
        }
        Value::Insert(list) => {
            for item in list {
                collect_vars_in(item, result);
            }
        }
        Value::If(c, t, e) => {
            collect_vars_in(c, result);
            collect_vars_in(t, result);
            collect_vars_in(e, result);
        }
        Value::Return(v) | Value::Drop(v) => collect_vars_in(v, result),
        Value::Iter(_, a, b, c) => {
            collect_vars_in(a, result);
            collect_vars_in(b, result);
            collect_vars_in(c, result);
        }
        Value::Span(b) => collect_vars_in(&b.1, result),
        _ => {}
    }
}

/// Recursively walk a Value IR tree and collect all variable indices that are written.
/// A variable is considered written if:
/// - It appears as the target of `Value::Set(v, ...)`,
/// - It is passed as a `RefVar`-typed argument to a `Value::Call`, or
/// - It appears anywhere in the first argument of a field-write operator (`OpSet*`),
///   which covers the pattern `v[idx].field = val` where `v: &vector<T>`.
/// - It flows into a callee whose own body mutates that parameter
///   (directly or transitively via further calls).  The
///   interprocedural lookup is memoised via `callee_cache`.
fn find_written_vars(
    code: &Value,
    data: &Data,
    written: &mut HashSet<u16>,
    callee_cache: &mut HashMap<u32, Vec<bool>>,
) {
    match code {
        Value::Set(v, body) => {
            written.insert(*v);
            find_written_vars(body, data, written, callee_cache);
        }
        Value::Call(fn_nr, args) => {
            let def = data.def(*fn_nr);
            let attrs = &def.attributes;
            // Operators whose FIRST argument is mutated (collection / field writes).
            // vector ops folded in here so `c.items += other_vec` (where `c.items`
            // is `OpGetField(Var(c), …)`) correctly marks `c` as written via
            // collect_vars_in.  Previously the OpAppend*/OpClear* family only checked for
            // a bare `Value::Var` arg, missing the field-access shape.
            let first_arg_write = def.name.starts_with("OpSet")
                || def.name.starts_with("OpAppendStack")
                || def.name.starts_with("OpClearStack")
                || def.name == "OpNewRecord"
                || def.name == "OpAppendCopy"
                || def.name == "OpAppendVector"
                || def.name == "OpClearVector"
                || def.name == "OpInsertVector"
                || def.name == "OpRemoveVector";
            // OpCopyRecord(src, dst, type) writes through `dst` (arg[1]).
            // Used by struct field whole-replacement (`s.i = fresh`) where the
            // destination is `OpGetField(s, …)`.
            let second_arg_write = def.name == "OpCopyRecord";
            for (i, arg) in args.iter().enumerate() {
                if i < attrs.len()
                    && matches!(attrs[i].typedef, Type::RefVar(_))
                    && let Value::Var(v) = arg
                {
                    written.insert(*v);
                }
                if i == 0 && first_arg_write {
                    collect_vars_in(arg, written);
                }
                if i == 1 && second_arg_write {
                    collect_vars_in(arg, written);
                }
                find_written_vars(arg, data, written, callee_cache);
            }
            // the callee may mutate one of its by-value parameters
            // through a field write (e.g. `fn add(self: Box, x) { self.items += [x] }`).
            // Look up its param-write effects and mark the corresponding
            // caller-side arg vars so `check_ref_mutations` sees them as
            // mutated.  Skip natives (`def.code == Value::Null`) — their
            // effects are already encoded by the OpSet*/OpAppend*/OpCopyRecord
            // patterns above.  Args are collected with `collect_vars_in` so
            // wrapped sources (field access, `OpCreateStack(Var(_))` from
            // the hoisted-preamble path) still propagate the mutation to
            // their root var.
            if def.code != Value::Null {
                let callee_writes = callee_param_writes(*fn_nr, data, callee_cache);
                for (i, arg) in args.iter().enumerate() {
                    if i < callee_writes.len() && callee_writes[i] {
                        collect_vars_in(arg, written);
                    }
                }
            }
        }
        Value::Block(block) | Value::Loop(block) => {
            for item in &block.operators {
                find_written_vars(item, data, written, callee_cache);
            }
        }
        Value::Insert(list) => {
            for item in list {
                find_written_vars(item, data, written, callee_cache);
            }
        }
        Value::If(cond, then, els) => {
            find_written_vars(cond, data, written, callee_cache);
            find_written_vars(then, data, written, callee_cache);
            find_written_vars(els, data, written, callee_cache);
        }
        Value::Return(v) | Value::Drop(v) => {
            find_written_vars(v, data, written, callee_cache);
        }
        // T1.5: TuplePut writes to the ref-tuple variable via its element assignment.
        Value::TuplePut(var_nr, _, inner) => {
            written.insert(*var_nr);
            find_written_vars(inner, data, written, callee_cache);
        }
        Value::Iter(_, create, next, extra) => {
            find_written_vars(create, data, written, callee_cache);
            find_written_vars(next, data, written, callee_cache);
            find_written_vars(extra, data, written, callee_cache);
        }
        Value::Span(b) => find_written_vars(&b.1, data, written, callee_cache),
        _ => {}
    }
}

/// for the given user-defined function, return a boolean per
/// parameter indicating whether its body writes that parameter
/// (directly or through a transitive call).  Results are memoised
/// in `cache`; a placeholder (all-false) is inserted before recursive
/// analysis so cycles are broken.  Caller should iterate to fixpoint
/// if precise transitive effects across recursion chains are needed;
/// for linear forwarding (the common case) one pass suffices.
fn callee_param_writes(fn_nr: u32, data: &Data, cache: &mut HashMap<u32, Vec<bool>>) -> Vec<bool> {
    if let Some(v) = cache.get(&fn_nr) {
        return v.clone();
    }
    let def = data.def(fn_nr);
    let n = def.attributes.len();
    // Break recursion: insert a placeholder before walking the body.
    cache.insert(fn_nr, vec![false; n]);
    if def.code == Value::Null || n == 0 {
        return vec![false; n];
    }
    let body = def.code.clone();
    let mut written: HashSet<u16> = HashSet::new();
    find_written_vars(&body, data, &mut written, cache);
    let result: Vec<bool> = (0..n).map(|i| written.contains(&(i as u16))).collect();
    // Monotone merge with any prior placeholder entry.
    let prev = cache.get(&fn_nr).cloned().unwrap_or_else(|| vec![false; n]);
    let merged: Vec<bool> = prev
        .iter()
        .zip(result.iter())
        .map(|(a, b)| *a || *b)
        .collect();
    cache.insert(fn_nr, merged.clone());
    merged
}

/// Like `find_written_vars` but only collects variables that are FIELD-written
/// (OpSet*, OpCopyRecord, OpNewRecord first-arg).  Excludes plain `Value::Set`
/// which includes loop-iterator advance — that's not a user-initiated mutation.
/// Used by check_ref_mutations to detect when a for-loop variable's field
/// writes should propagate back to the iterated `&` collection.
fn find_field_written_vars(code: &Value, data: &Data, written: &mut HashSet<u16>) {
    match code {
        Value::Call(fn_nr, args) => {
            let def = data.def(*fn_nr);
            let first_arg_write = def.name.starts_with("OpSet")
                || def.name == "OpNewRecord"
                || def.name == "OpAppendCopy"
                || def.name == "OpAppendVector"
                || def.name == "OpClearVector"
                || def.name == "OpInsertVector"
                || def.name == "OpRemoveVector";
            let second_arg_write = def.name == "OpCopyRecord";
            for (i, arg) in args.iter().enumerate() {
                if i == 0 && first_arg_write {
                    collect_vars_in(arg, written);
                }
                if i == 1 && second_arg_write {
                    collect_vars_in(arg, written);
                }
                find_field_written_vars(arg, data, written);
            }
        }
        Value::Set(_, body) => find_field_written_vars(body, data, written),
        Value::Block(block) | Value::Loop(block) => {
            for item in &block.operators {
                find_field_written_vars(item, data, written);
            }
        }
        Value::Insert(list) => {
            for item in list {
                find_field_written_vars(item, data, written);
            }
        }
        Value::If(cond, then, els) => {
            find_field_written_vars(cond, data, written);
            find_field_written_vars(then, data, written);
            find_field_written_vars(els, data, written);
        }
        Value::Return(v) | Value::Drop(v) => find_field_written_vars(v, data, written),
        Value::Iter(_, create, next, extra) => {
            find_field_written_vars(create, data, written);
            find_field_written_vars(next, data, written);
            find_field_written_vars(extra, data, written);
        }
        Value::Span(b) => find_field_written_vars(&b.1, data, written),
        _ => {}
    }
}

/// Map an operator token to its CamelCase name suffix used in `OpCamelCase` identifiers.
/// E.g. `"<"` → `"Lt"`, so the method name becomes `"OpLt"`.
/// Also used by I3.1 (`op <token>` sugar in interface bodies).
pub(crate) fn rename(op: &str) -> &str {
    match op {
        "*" => "Mul",
        "+" => "Add",
        "-" => "Min",
        "/" => "Div",
        "&" => "Land",
        "|" => "Lor",
        "^" => "Eor",
        "<<" => "SLeft",
        ">>" => "SRight",
        "==" => "Eq",
        "!=" => "Ne",
        "<" => "Lt",
        "<=" => "Le",
        ">" => "Gt",
        ">=" => "Ge",
        "%" => "Rem",
        "**" => "Pow",
        "!" => "Not",
        "~" => "BitNot",
        "+=" => "Append",
        _ => op,
    }
}
