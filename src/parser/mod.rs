// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Parse scripts and create internal code from it.
//! Including type checking.

use crate::data::{
    Argument, Context, Data, DefType, Deps, I32, IntegerSpec, Type, Value, to_default, v_block,
    v_if, v_loop, v_set,
};
use crate::database::{Parts, Stores};
use crate::diagnostics::{DiagEntry, Diagnostics, Level, diagnostic_format};
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
    /// Each entry is `(name_in_library, bind_name)`; `bind_name == name` for a
    /// plain `use lib::name`, or the alias for `use lib::name as bind` (@PLN22
    /// Phase 3).
    Names(Vec<(String, String)>),
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
    /// @PLN11 arc E — set by the driver (`main.rs`) only when the whole-program
    /// startup cache is enabled; gates [`Parser::parsed_sources`] tracking so a
    /// normal (non-cache) run pays nothing.
    pub track_sources: bool,
    /// @PLN11 arc E — paths of every source file parsed (stdlib + lazily-loaded
    /// libs + user file), in load order, recorded only when `track_sources`.
    /// Only the parser sees the dynamically-loaded lib set; the whole-program
    /// cache hashes these files' contents to key the bundle + detect drift.
    /// Paths only (content is re-read once at save time — no per-file memory);
    /// may contain duplicates across the two parse passes, deduped at use.
    pub parsed_sources: Vec<String>,
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
    /// @PLN86 — the host-supplied sandbox policy (profiles + designations).
    /// Empty by default; set by the embedder before parsing.  A script cannot
    /// designate itself — the designation is read from here, not the source.
    pub(crate) sandbox: crate::sandbox::SandboxConfig,
    /// @PLN86 step 1.2 — the sandbox profile NAME each designated function is
    /// parsed under (`def_nr -> profile`), recorded as the function is parsed.
    /// The compile-time admission walk reads this to know which defs are
    /// restricted.  Side-map (not on `Definition`) so it stays out of the IR
    /// serialization — the tag is policy, re-derivable from `sandbox`.
    pub(crate) def_sandbox: HashMap<u32, String>,
    /// @PLN86 step 3.1 — sandboxed defs that contain an unbounded `while` loop,
    /// mapped to one such loop's position.  Recorded at parse (`parse_while`
    /// uniquely knows it is a `while`, where the IR cannot reliably tell a `while`
    /// `Loop` from a bounded comprehension `Loop`); the totality admission reads
    /// it.  Keyed by def_nr so the two parse passes are idempotent.
    pub(crate) sandbox_unbounded_loops: HashMap<u32, crate::lexer::Position>,
    /// @PLN86 step 2.4 — sandboxed defs that perform a RAW WRITE to heap data
    /// (`x.field = v` / `v[i] = v`), mapped to one such write's position.
    /// Recorded at parse (`parse_assign` knows the LHS is a field/index target,
    /// not a bare local or a struct literal); the no-raw-write admission reads it.
    /// A sandboxed script may mutate host data only via allow-listed `*.write`
    /// ops, never a raw field/element assignment.  Keyed by def_nr (idempotent).
    pub(crate) sandbox_raw_writes: HashMap<u32, crate::lexer::Position>,
    /// @PLN86 step 0.1 — true while parsing the BODY of a sandboxed def.  Gates
    /// the parser nesting guard so it never touches trusted code (zero cost
    /// there); set per-def in `parse_function`, cleared at its end.
    pub(crate) in_sandbox: bool,
    /// @PLN86 step 0.1 — current expression-nesting depth, counted ONLY while
    /// `in_sandbox`.  Recursive-descent over hostile deep nesting (`((((…))))`)
    /// overflows the native stack (rc=139); past `SANDBOX_MAX_PARSE_DEPTH` the
    /// parser rejects with a clean diagnostic instead — a LOAD-time rejection,
    /// never a runtime abort.  Reset to 0 at each sandboxed def's body.
    pub(crate) parse_depth: u32,
    /// @PLN86 step 0.1 — latched once the depth limit trips, so the diagnostic
    /// is emitted once per def rather than at every frame as the parser unwinds.
    pub(crate) depth_overflowed: bool,
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
    /// The `(stem, pkg_dir)` registration inputs behind `pending_native_libs`.
    /// Persisted in the startup-cache manifest so a warm load can re-resolve
    /// the cdylibs with cold-equal freshness semantics (#310 — the resolved
    /// PATHS alone can go stale when loft itself is rebuilt).
    pub native_lib_regs: Vec<(String, String)>,
    /// @PLN11 Arc N / N3 — package dirs of libraries that opted into
    /// auto-compilation (`[library] compile = "native"`).  Recorded during `use`
    /// processing; the driver (`main.rs`) marks each library's public
    /// shared-store-dispatchable functions native (after `scopes::check`, before
    /// `byte_code`) and builds + loads the cdylib (after `byte_code`).  A library's
    /// functions are identified by `def.position().file.starts_with(pkg_dir)`.
    pub pending_native_compile: Vec<String>,
    /// PKG.3: package dependencies discovered during manifest reading.
    /// Each entry is (name, dir) — sibling packages are searched in `dir`.
    pending_pkg_deps: Vec<(String, String)>,
    /// Auto-`use` per-file scan cache: a file's `(lib:: refs, .method() calls)`
    /// are deterministic, so it is read + scanned at most once (keyed by path)
    /// and reused across the second pass and any `todo_files` re-parse.
    auto_use_scan_cache: std::collections::HashMap<String, (Vec<String>, Vec<String>)>,
    /// Per-directory cache for the dep-shadowing guard in `lib_path`: the
    /// nearest ancestor manifest's package root + its declared dependency
    /// names.  `None` = no manifest above that directory.
    pkg_dep_cache:
        std::collections::HashMap<String, Option<(String, std::collections::HashSet<String>)>>,
    /// Tier-1 text-method trigger map: `method name -> providing package`,
    /// derived once per top-level parse from the current package's (and its
    /// trigger-enabled dependencies') declared triggers.  `None` until built.
    auto_use_trigger_map: Option<std::collections::HashMap<String, String>>,
    /// Tier-1 lazy *catalog* fallback: `method name -> providing package`,
    /// derived once from the cached registry `index.json` (`triggers` field).
    /// Consulted only for methods the local `auto_use_trigger_map` did not
    /// resolve — i.e. a package the user has NOT declared as a dependency but
    /// that the registry says provides the method.  `None` until first miss;
    /// cached empty when the catalog is absent or the registry feature is off.
    // Only read by the `#[cfg(feature = "registry")]` `catalog_trigger_map`; with
    // the registry feature off the field is initialised but never consulted.
    #[cfg_attr(not(feature = "registry"), allow(dead_code))]
    auto_use_catalog_map: Option<std::collections::HashMap<String, String>>,
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
    // The two-pass contract (H5, @PLAN59): pass 1 registers definitions
    // and FINAL signatures — since @PLAN59 every body-carrying plain fn's
    // hidden `__retbuf` exists from its pass-1 signature parse, so attr
    // counts CANNOT change in pass 2 (`ref_return` debug-asserts it; only
    // lambda-class defs may still grow, and they have no earlier callers).
    // Pass 2 re-parses bodies against those fixed signatures.  Variable
    // tables persist across passes BY NAME (pass 2 re-finds rather than
    // re-creates) — the parser is therefore NOT re-entrant beyond two
    // passes: a third pass leaves tables half-migrated (verified by the
    // #339 third-pass experiment, which segfaulted).  The "attr counts cannot
    // change in pass 2" claim above is now ENFORCED (not just asserted at the
    // `ref_return` growth site) by `assert_pass2_def_attr_stable` — a per-def
    // count snapshot compared end-of-pass-1 vs end-of-pass-2 in debug/armed
    // builds, so any future cross-pass divergence fails loud (H5).
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
    /// @PLN22 Phase 1 — expected enum type for the value currently being parsed
    /// where the operand `var_tp` does not carry it (a call argument, a function
    /// return-body tail).  Set by `parse_call` / the return-body parse before the
    /// value is parsed, consulted by `parse_single` to resolve a bare
    /// value-position variant against the expected enum, then cleared to
    /// `Type::Unknown(0)`.  (`var_tp` already carries the enum for typed-local
    /// decls and `==`, so those need no hint.)
    pub(crate) enum_hint: Type,
    /// Expected destination type for an `f#read` with no explicit `(n)` and
    /// no `as T` cast.  Set by `parse_assign` from the LHS type before
    /// parsing the RHS so that `s.field = f#read` infers the byte width
    /// from `s.field`'s declared type — symmetric with the way `f += s.field`
    /// already takes its width from the field's declared type.  Reset to
    /// `Type::Unknown(0)` after the RHS is parsed.
    pub(crate) read_target_type: Type,
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
    /// @PLN25 E2 — def_nr of the generic type-variable stub (`T`) currently in
    /// scope while parsing a `fn f<T>(…)` signature/body; `u32::MAX` outside a
    /// generic function.  Consulted ONLY by `e2_nullable_elem` so a generic
    /// `vector<T>` is NOT rewritten to `vector<__nullable<T>>` (T is opaque at
    /// definition time — nullability is decided at instantiation by whatever
    /// concrete element type the caller's vector carries).  Reset per function.
    pub(crate) cur_type_var: u32,
    // maps fn-ref variable numbers to their closure record work variable numbers.
    pub(crate) closure_vars: std::collections::HashMap<u16, u16>,
    // last closure work variable created by emit_lambda_code (transient).
    pub(crate) last_closure_work_var: u16,
    // closure allocation expression to inject at the call site.
    pub(crate) last_closure_alloc: Option<Box<Value>>,
    // outer variable numbers captured by the most recently parsed lambda.
    // Consumed by try_fn_ref_call to mark them as read at call-injection time.
    pub(crate) last_closure_captured_vars: Vec<u16>,
    /// #314: capturing lambdas synthesized during each function body in
    /// pass 1, keyed by the enclosing context's def_nr.  Consumed by
    /// `reject_shared_mutable_scalar_captures` at the parent's body end
    /// — the first moment the parent's `scalars_to_box` accumulation is
    /// complete — to diagnose a mutated scalar captured by more than
    /// one closure (GOALS.md § "Stability trumps features").
    pub(crate) fn_lambdas: std::collections::HashMap<u32, Vec<u32>>,
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
    /// Plan-07 phase 4h — per-(struct_d_nr, attr_idx) read counter.
    /// Incremented by `Parser::field()` on the second pass each time a
    /// field is READ (not assigned) on a user struct.  Surfaced at
    /// end of parse: each non-`not_null` field whose read count >=
    /// `HINT_NOT_NULL_THRESHOLD` AND whose `defended_field_reads` set
    /// does NOT contain the (d_nr, attr_idx) gets a `Level::Warning`
    /// at its declaration suggesting `not null`.  Stdlib (`self.default
    /// == true`) is exempt.  Silenceable via `LOFT_NO_HINT_NOT_NULL=1`.
    pub(crate) field_read_counts: std::collections::HashMap<(u32, u32), u32>,
    /// Plan-07 phase 4h — set of (struct_d_nr, attr_idx) that have at
    /// least one defensive read site (`obj.field ?? default` or `if
    /// obj.field != null` flow analysis).  Membership in this set
    /// suppresses the `not null` hint regardless of read count — the
    /// developer has explicitly acknowledged null is possible.
    pub(crate) defended_field_reads: std::collections::HashSet<(u32, u32)>,
    /// Plan-07 phase 4h — site of the most recently parsed field
    /// read.  Set by `Parser::field()` after each read, taken by
    /// `handle_null_coalesce` to mark the read as defended when
    /// `expr ?? default` follows.  `None` between distinct
    /// expressions / statements.  Conservative: covers the common
    /// `p.field ?? default` shape; complex expressions like
    /// `(p.field + 1) ?? 0` and `if p.field != null` are
    /// under-detected today (slice 2).
    pub(crate) last_field_read_site: Option<(u32, u32)>,
    /// @PLAN12 Phase 6.7 — dedupe set for the per-invocation advisory
    /// check.  Once a `(name, version)` tuple has been classified
    /// against the advisory feed during this parser's lifetime, we
    /// don't re-fire the warning.  Critical-severity matches abort
    /// via `process::exit(3)` before the second probe could fire
    /// anyway; this dedupe is for the warning/note tiers.
    #[cfg(feature = "registry")]
    advisory_checked: std::collections::HashSet<(String, String)>,
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

/// Outcome of [`Parser::parse_statement`] — the REPL read-eval step's parse half.
#[derive(Debug)]
pub enum ParseResult {
    /// Fully parsed.  Any new top-level definition is registered in `data` and
    /// its body IR is built, ready for codegen + execute.  `entry_def_nr` is the
    /// newest definition's number, or `u32::MAX` when the statement added no
    /// runnable definition.
    Ready { entry_def_nr: u32 },
    /// Input ends mid-construct (open bracket, unterminated string, trailing
    /// operator).  The REPL should read another line and re-call with the
    /// concatenated input.
    NeedMore,
    /// Parse failed.  `data` has been rolled back to its pre-call state; the
    /// entries are the diagnostics this statement produced.
    Error(Vec<DiagEntry>),
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
        data.definitions[d as usize].returned = Type::Text(Deps::none());
        let d = data.add_def("i_parse_error_push", &pos, DefType::Function);
        data.definitions[d as usize].returned = Type::Void;
        {
            let mut lexer = Lexer::default();
            data.add_attribute(&mut lexer, d, "msg", Type::Text(Deps::none()));
        }
        Parser {
            todo_files: Vec::new(),
            track_sources: false,
            parsed_sources: Vec::new(),
            data,
            database: Stores::new(),
            lexer: Lexer::default(),
            in_loop: false,
            in_format_expr: false,
            sandbox: crate::sandbox::SandboxConfig::default(),
            def_sandbox: HashMap::new(),
            sandbox_unbounded_loops: HashMap::new(),
            sandbox_raw_writes: HashMap::new(),
            in_sandbox: false,
            parse_depth: 0,
            depth_overflowed: false,
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
            native_lib_regs: Vec::new(),
            pending_native_compile: Vec::new(),
            pending_pkg_deps: Vec::new(),
            auto_use_scan_cache: std::collections::HashMap::new(),
            pkg_dep_cache: std::collections::HashMap::new(),
            auto_use_trigger_map: None,
            auto_use_catalog_map: None,
            pending_imports: Vec::new(),
            applied_imports: Vec::new(),
            deferred_unknown: Vec::new(),
            expr_not_null: false,
            expr_not_null_name: String::new(),
            lambda_counter: 0,
            lambda_hint: Type::Unknown(0),
            enum_hint: Type::Unknown(0),
            read_target_type: Type::Unknown(0),
            fields_of: u32::MAX,
            capture_context: Vec::new(),
            captured_names: Vec::new(),
            fn_lambdas: std::collections::HashMap::new(),
            closure_param: u16::MAX,
            cur_type_var: u32::MAX,
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
            field_read_counts: std::collections::HashMap::new(),
            defended_field_reads: std::collections::HashSet::new(),
            last_field_read_site: None,
            #[cfg(feature = "registry")]
            advisory_checked: std::collections::HashSet::new(),
        }
    }

    /// Parse the content of a given file.
    /// - filename: the file to parse
    /// - default: parsing system definitions
    /// @PLN86 step 1.2 — install the host's sandbox policy before parsing.  The
    /// designation is host-controlled; a script can never mark itself sandboxed.
    pub fn set_sandbox_config(&mut self, config: crate::sandbox::SandboxConfig) {
        self.sandbox = config;
    }

    /// @PLN86 step 1.2 — the sandbox profile a function was parsed under, or
    /// `None` for unrestricted (trusted) code.  The admission walk reads this to
    /// apply the totality / capability rules only to sandboxed defs.
    #[must_use]
    pub fn def_sandbox_profile(&self, def_nr: u32) -> Option<&str> {
        self.def_sandbox.get(&def_nr).map(String::as_str)
    }

    /// @PLN86 step 1.3 — the sandbox-reachable set: every def reachable from the
    /// sandboxed entries via calls + fn-ref literals, descending only into
    /// sandboxed defs (trusted symbols are leaves).  Step 2.3 capability-checks
    /// the non-sandboxed members.  Call after parsing, when bodies are resolved.
    #[must_use]
    pub fn sandbox_reachable_set(&self) -> std::collections::HashSet<u32> {
        crate::sandbox::reachable_set(&self.data, &self.def_sandbox)
    }

    /// @PLN86 step 1.4 — external `[native]` cdylib bridges reachable from the
    /// sandboxed entries.  Empty unless a sandboxed def reaches an external FFI
    /// symbol; admission rejects a non-empty result when the profile forbids
    /// native FFI (RCE by construction).  Backend force-interpret + the
    /// no-default-features cdylib removal are the remaining 1.4 work.
    #[must_use]
    pub fn sandbox_ffi_bridges(&self) -> Vec<u32> {
        crate::sandbox::reachable_ffi_bridges(&self.data, &self.sandbox_reachable_set())
    }

    /// @PLN86 step 2.1 — the `#cap "group"` capability group a def declares, or
    /// `None` if unannotated.  Step 2.3 gates each trusted symbol in the reachable
    /// set against the profile via `SandboxConfig::allows`.
    #[must_use]
    pub fn def_cap_group(&self, def_nr: u32) -> Option<&str> {
        let cap = self.data.def(def_nr).cap();
        (!cap.is_empty()).then_some(cap)
    }

    /// @PLN86 step 2.3 — the capability-admission walk: every trusted symbol a
    /// sandboxed def reaches must carry a `#cap` group its profile permits (or be
    /// `native_ffi`-allowed).  An empty result means the sandboxed code is
    /// admitted; otherwise each `CapViolation` names the offending reference for
    /// a diagnostic.  Run after parsing.
    #[must_use]
    pub fn sandbox_admit(&self) -> Vec<crate::sandbox::CapViolation> {
        crate::sandbox::admit_capabilities(&self.data, &self.sandbox, &self.def_sandbox)
    }

    /// @PLN86 step 2.2 — the capability-coverage lint: public functions lacking a
    /// `#cap "group"`.  The host runs this over the stdlib + libraries to find the
    /// surface still to tag; an empty result is full coverage (L3-cap).
    #[must_use]
    pub fn untagged_public_symbols(&self) -> Vec<u32> {
        crate::sandbox::untagged_public_symbols(&self.data)
    }

    /// @PLN86 — the library a def belongs to (derived from its source), the
    /// wholesale-admission key for a profile's `allow_libs`.  `None` for a
    /// synthetic / sourceless def.
    #[must_use]
    pub fn def_library(&self, def_nr: u32) -> Option<String> {
        crate::sandbox::def_library(&self.data, def_nr)
    }

    /// @PLN86 step P3 — totality violations: the sandboxed script is rejected if
    /// it cannot be proven to terminate (an unbounded `while`, or a recursion
    /// cycle).  Empty for a provably-total script.
    #[must_use]
    pub fn sandbox_totality(&self) -> Vec<crate::sandbox::TotalityViolation> {
        crate::sandbox::admit_totality(&self.data, &self.def_sandbox, &self.sandbox_unbounded_loops)
    }

    /// @PLN86 step 2.4 — no-raw-write violations: sandboxed defs that directly
    /// mutate heap data (`x.field = v` / `v[i] = v`).  Empty when every mutation
    /// goes through an allow-listed `*.write` op.
    #[must_use]
    pub fn sandbox_raw_writes(&self) -> Vec<crate::sandbox::RawWriteViolation> {
        crate::sandbox::raw_write_violations(&self.def_sandbox, &self.sandbox_raw_writes)
    }

    /// @PLN86 step 2.5 — ALL sandbox admission errors (capability + totality +
    /// no-raw-write), each rendered as a correct, specific, actionable message
    /// (position + the rule + the fix).  Empty when the script is admitted.  This
    /// is the host's primary surface — the contract a modder iterates against.
    #[must_use]
    pub fn sandbox_admission_errors(&self) -> Vec<String> {
        let caps = self.sandbox_admit().into_iter().map(|v| {
            crate::sandbox::describe_violation(&self.data, &self.sandbox, &self.def_sandbox, &v)
        });
        let totality = self
            .sandbox_totality()
            .into_iter()
            .map(|v| crate::sandbox::describe_totality_violation(&self.data, &v));
        let raw_writes = self
            .sandbox_raw_writes()
            .into_iter()
            .map(|v| crate::sandbox::describe_raw_write_violation(&self.data, &v));
        caps.chain(totality).chain(raw_writes).collect()
    }

    /// @PLN86 step 1.4 — does this program contain sandboxed code that must run
    /// on the interpreter?  True iff ANY def is designated sandboxed.  This is
    /// non-negotiable, NOT a per-profile choice: generating + compiling Rust on
    /// the host is RCE by construction, and the native backend traps where the
    /// interpreter is total (div-by-zero yields null — 3.3).  loft's own CLI
    /// run-path forces interpret (and refuses an explicit `--native`) when true.
    #[must_use]
    pub fn sandbox_forces_interpret(&self) -> bool {
        !self.def_sandbox.is_empty()
    }

    /// @PLN86 step 3.4 — the worst-case complexity DEGREE of the sandboxed code:
    /// its step count is `O(n^degree)` in the largest input size.  An admitted
    /// script is total, so this is finite; the host reads it to bound the inputs
    /// so no single frame stalls (L5).
    #[must_use]
    pub fn sandbox_complexity_degree(&self) -> u32 {
        crate::sandbox::sandbox_complexity_degree(&self.data, &self.def_sandbox)
    }

    /// @PLN86 step 3.4 — the human-readable worst-case complexity report.
    #[must_use]
    pub fn sandbox_complexity_report(&self) -> String {
        crate::sandbox::complexity_report(self.sandbox_complexity_degree())
    }

    /// # Panics
    /// With filesystem problems.
    pub fn parse(&mut self, filename: &str, default: bool) -> bool {
        // under the `wasm` feature, check VIRT_FS before trying the real filesystem.
        #[cfg(feature = "wasm")]
        if let Some(content) = crate::wasm::virt_fs_get(filename) {
            return self.parse_virtual(&content, filename, default);
        }
        // @PLN11 arc E — record the input file for the whole-program cache key.
        if self.track_sources {
            self.parsed_sources.push(filename.to_string());
        }
        // @PLAN49 T1 — set the breadcrumb phase + initial file/line so
        // a watchdog-fired hard-kill localises any parse-time hang.
        crate::timeout::checkpoint_parse(filename, 0);
        self.default = default;
        // #255 / @PLN9: establish `source_dir` (the running program's own
        // directory) once, at the first non-default parse.  This is the single
        // home every execution path inherits — CLI run, `loft --test`, the wrap
        // integration runner, and the wasm/native front-ends all reach this
        // `parse()`.  `data.reset()` below clears `Data` but not `Stores`, so the
        // value survives the two-pass re-parse; the `is_empty()` guard keeps the
        // *first* (main) file winning over later directory/import re-parses.
        // (main.rs additionally sets it for the startup-cache path, which loads a
        // pre-parsed snapshot and never calls `parse()`.)
        if !default && self.database.source_dir.is_empty() {
            self.database.source_dir = std::path::Path::new(filename)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        self.vars.logging = false;
        self.lexer.switch(filename);
        self.first_pass = true;
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        // @PLN86 1.2 — the def→profile side-map is keyed by def_nr, which
        // `data.reset()` reassigns; clear it so a re-parse re-derives the
        // designation rather than reading a stale entry.
        self.def_sandbox.clear();
        self.sandbox_unbounded_loops.clear();
        self.sandbox_raw_writes.clear();
        self.data.reset();
        // @PLN22 Phase 2 — the main program parses under its own source
        // (MAIN_SOURCE), distinct from the stdlib prelude (source 0), so a user
        // definition shadows a prelude name instead of colliding on `(name, 0)`.
        // `reset()` set source to 0; the default stdlib parse stays at 0.
        if !default {
            self.data.source = crate::data::MAIN_SOURCE;
        }
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.parse_file();
        self.resolve_deferred_unknowns();
        // H5 — two-pass name-stability contract: snapshot every def's attribute
        // count at the end of pass 1.  `data.reset()` between passes preserves
        // `definitions`, so def numbers are stable and pass 2 must reproduce these
        // counts exactly.  Post-arity-cascade (signatures freeze at declaration)
        // this is an INVARIANT, not an aspiration — a divergence is a real
        // cross-pass bug, asserted below after pass 2.
        #[cfg(debug_assertions)]
        let pass1_attr_counts: Vec<usize> = (0..self.data.definitions.len())
            .map(|d| self.data.attributes(d as u32))
            .collect();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            self.reverse_iterator = false;
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.data.reset();
            if !default {
                self.data.source = crate::data::MAIN_SOURCE;
            }
            self.lambda_counter = 0;
            self.fn_lambdas.clear();
            self.lexer.switch(filename);
            self.parse_file();
            self.resolve_deferred_unknowns();
            #[cfg(debug_assertions)]
            self.assert_pass2_def_attr_stable(&pass1_attr_counts);
        }
        self.backfill_native_symbol_crates();
        // Plan-07 phase 4h — emit `not null` field-reminder hints
        // after the second pass completes (so all reads have been
        // counted).  Silenceable via `LOFT_NO_HINT_NOT_NULL=1`.
        if !self.first_pass && !default {
            self.emit_not_null_hints();
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// H5 (two-pass name-stability contract): assert pass 2 reproduced pass 1's
    /// per-def attribute counts exactly.  The parser is not re-entrant beyond two
    /// passes (the #339 third-pass experiment segfaulted on half-migrated variable
    /// tables) and pass 2 re-finds synthesized names by position, so a def whose
    /// attribute count changed between passes is a silent contract break.  Post the
    /// @PLAN59 arity cascade (signatures freeze at declaration) this is an
    /// INVARIANT — a firing assert is a real cross-pass bug, not noise.
    ///
    /// This attribute-count check is the COMPLETE H5 validation: the H5 spec's other
    /// named residual — work-ref (`__ref_N`) counter equality per fn — was dissolved
    /// by H1 (@PLAN59), exactly as the spec's item 3 ("re-evaluate after H1") foresaw.
    /// `work_refs()` now fires zero times across the whole debug corpus, and the
    /// counter's value in a stored table is unconditionally reset to 0 by
    /// `Function::append` at store time, so a work-ref-counter assert here would be
    /// permanently vacuous.  The one failure mode it could ever have caught — a
    /// cross-pass `__ref_N` name shift making `ref_return` add a spurious attr — IS
    /// caught here, because that spurious attr is itself an attribute-count divergence.
    #[cfg(debug_assertions)]
    fn assert_pass2_def_attr_stable(&self, pass1_attr_counts: &[usize]) {
        debug_assert_eq!(
            pass1_attr_counts.len(),
            self.data.definitions.len(),
            "H5: definition COUNT diverged across passes (pass1={}, pass2={})",
            pass1_attr_counts.len(),
            self.data.definitions.len(),
        );
        for (d, &c1) in pass1_attr_counts.iter().enumerate() {
            let c2 = self.data.attributes(d as u32);
            debug_assert_eq!(
                c1,
                c2,
                "H5 two-pass contract: def `{}` (#{d}) attribute count diverged across \
                 passes (pass1={c1}, pass2={c2})",
                self.data.def(d as u32).name(),
            );
        }
    }

    /// Plan-07 phase 4h — walk user struct definitions and emit
    /// `Level::Warning` for each non-`not_null` field whose read
    /// count exceeds `HINT_NOT_NULL_THRESHOLD` AND whose
    /// `defended_field_reads` set is empty.  The hint suggests
    /// adding `not null` at the field declaration so the
    /// constructor enforces non-null at write-time, eliminating
    /// the entire class of fault sites for that field.
    ///
    /// Threshold is conservative (10 reads) to keep the hint
    /// from firing on small / illustrative struct definitions.
    /// Lower thresholds + smarter defended-detection are slice 2.
    ///
    /// Stdlib (`self.default == true`) is exempt — its struct
    /// definitions are language-internal and the suggestion target
    /// is user code, not the host library.
    fn emit_not_null_hints(&mut self) {
        const HINT_NOT_NULL_THRESHOLD: u32 = 10;
        if std::env::var("LOFT_NO_HINT_NOT_NULL").is_ok_and(|v| v == "1" || v == "true") {
            self.field_read_counts.clear();
            self.defended_field_reads.clear();
            return;
        }
        // Snapshot the keys so we can borrow `self.lexer` mutably
        // while iterating.
        let counts: Vec<((u32, u32), u32)> = self
            .field_read_counts
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        for ((d_nr, attr_idx), count) in counts {
            if count < HINT_NOT_NULL_THRESHOLD {
                continue;
            }
            if self.defended_field_reads.contains(&(d_nr, attr_idx)) {
                continue;
            }
            // Re-check `nullable` at emission time — definition
            // mutations between count + emit are unlikely but not
            // forbidden.
            let attrs = self.data.def(d_nr).attributes();
            if attr_idx as usize >= attrs.len() {
                continue;
            }
            let attr = &attrs[attr_idx as usize];
            if !attr.nullable {
                continue;
            }
            let struct_name = self.data.def(d_nr).name().to_string();
            let field_name = attr.name.clone();
            let pos = self.data.def(d_nr).position().clone();
            self.lexer.pos_diagnostic(
                Level::Warning,
                &pos,
                &format!(
                    "field `{struct_name}.{field_name}` is read {count} times and never \
                     defended with `??` or `if x.{field_name} != null` — consider \
                     marking it `not null` so the constructor enforces non-null at \
                     write-time"
                ),
            );
        }
        self.field_read_counts.clear();
        self.defended_field_reads.clear();
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
                    for (name, bind) in names {
                        self.data
                            .import_name_overwrite(pi.lib_source, pi.for_source, name, bind);
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
            let stub_name = self.data.def(stub_nr).name().to_string();
            // Case (a): stub upgraded in place
            if !matches!(self.data.def(stub_nr).def_type(), DefType::Unknown) {
                self.data.rewrite_unknown_refs(stub_nr, stub_nr);
                continue;
            }
            // Case (b): lookup via post-import source binding
            let resolved_nr = self.data.source_nr(source, &stub_name);
            if resolved_nr != u32::MAX
                && resolved_nr != stub_nr
                && !matches!(self.data.def(resolved_nr).def_type(), DefType::Unknown)
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
    /// Walk every definition once more and bind each `#native` symbol still
    /// missing from the map to the native package that OWNS its definition:
    /// the registered package whose directory is a prefix of the def's source
    /// file (longest prefix wins, for nested packages).  Ownership-by-path is
    /// the same invariant the per-manifest pass enforces, run once at the end
    /// when every def exists, so it covers ANY number of native packages.
    /// (The earlier `len() == 1` shortcut silently skipped programs using two
    /// native packages — e.g. `graphics` + `random` — leaving BOTH unmapped:
    /// the interpreter still dispatched them via `def.native` + dlopen, but
    /// `--native` rejected the first reachable call with a P269 compile error.)
    fn backfill_native_symbol_crates(&mut self) {
        if self.data.native_packages.is_empty() {
            return;
        }
        let mut binds: Vec<(String, String)> = Vec::new();
        for d_nr in 0..self.data.definitions() {
            let def = self.data.def(d_nr);
            let sym = def.native();
            if sym.is_empty() || self.data.native_symbol_crates.contains_key(sym) {
                continue;
            }
            let sym = sym.to_string();
            let file = def.position().file.clone();
            // Owner = the registered native package whose dir is the longest
            // prefix of this def's source file.
            if let Some((crate_name, _)) = self
                .data
                .native_packages
                .iter()
                .filter(|(_, pkg_dir)| file.starts_with(pkg_dir.as_str()))
                .max_by_key(|(_, pkg_dir)| pkg_dir.len())
            {
                binds.push((sym, crate_name.replace('-', "_")));
            }
        }
        for (sym, rust_crate) in binds {
            self.data.native_symbol_crates.insert(sym, rust_crate);
        }
    }

    /// Parse `content` as if it were the file at `filename`.
    /// Used by the WASM virtual-FS path to bypass real filesystem access.
    #[cfg(feature = "wasm")]
    fn parse_virtual(&mut self, content: &str, filename: &str, default: bool) -> bool {
        // @PLN11 arc E — record the input file for the whole-program cache key.
        if self.track_sources {
            self.parsed_sources.push(filename.to_string());
        }
        self.default = default;
        self.vars.logging = false;
        self.first_pass = true;
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
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
            self.fn_lambdas.clear();
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
                if *self.data.def(d_nr).code() == Value::Null {
                    continue;
                }
                write!(w, "{} ", self.data.def(d_nr).header(&self.data, d_nr))?;
                let mut vars = Function::copy(self.data.def(d_nr).variables());
                self.data
                    .show_code(&mut w, &mut vars, self.data.def(d_nr).code(), 0, false)?;
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
        // Start each standalone parse on a fresh lexer.  `restart` resets the
        // cursor but not the diagnostics or format-mode flags, and
        // `Diagnostics::level` is monotonic — so reusing the lexer would leak a
        // prior parse's errors into this one (poisoning REPL re-entrancy: a typo
        // would make every later input mis-parse).  A new lexer is fully clean.
        self.lexer = Lexer::default();
        self.first_pass = true;
        self.default = false;
        self.vars.logging = logging;
        self.lexer.parse_string(text, filename);
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
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
        self.fn_lambdas.clear();
        self.lexer.parse_string(text, filename);
        self.first_pass = false;
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.diagnostics.fill(self.lexer.diagnostics());
    }

    /// Session-scope snippet parse (#350, live-reload): like
    /// [`parse_str`](Self::parse_str) but KEEPS the session's import scoping —
    /// `use_names` and the applied imports survive, and both passes run under
    /// `source` (the def-source of the fn being replaced) — so the snippet
    /// resolves the same library names its original file did.  `parse_str`'s
    /// `data.reset()` exists for whole-program loads, where each pass re-runs
    /// the `use` statements and re-registers scoping in order; a snippet has
    /// no `use` lines, so the reset left it with no library scope at all
    /// ("Unknown library" on every lib-qualified name).  No library loading
    /// happens here: every `use` the program needs already ran at install.
    pub fn parse_snippet(&mut self, text: &str, filename: &str, source: u16) {
        self.lexer = Lexer::default();
        self.first_pass = true;
        self.default = false;
        self.vars.logging = false;
        self.lexer.parse_string(text, filename);
        self.deferred_unknown.clear();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.data.source = source;
        self.parse_file();
        self.resolve_deferred_unknowns();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            self.diagnostics.fill(self.lexer.diagnostics());
            return;
        }
        self.deferred_unknown.clear();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.lexer.parse_string(text, filename);
        self.first_pass = false;
        self.data.source = source;
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.diagnostics.fill(self.lexer.diagnostics());
    }

    /// @PLN12 phase 02 — does `input` end mid-construct, so a REPL should read
    /// more lines before trying to parse it?
    ///
    /// Returns `true` when a bracket is still open (`(`, `[`, `{`), a `"…"`
    /// string literal is unterminated, or the last meaningful token is a
    /// binary / continuation operator (`1 +`, `x.`).  `//` line comments and
    /// escaped quotes are skipped.  It is deliberately conservative: a missed
    /// "incomplete" just produces the ordinary parse error the caller already
    /// handles, never a crash.  Pure over the input string — no parser state.
    #[must_use]
    pub fn statement_incomplete(input: &str) -> bool {
        let chars: Vec<char> = input.chars().collect();
        let mut depth: i32 = 0;
        let (mut in_str, mut esc) = (false, false);
        let mut last: Option<char> = None;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                    last = Some('"');
                }
                i += 1;
                continue;
            }
            // `//` to end of line is a comment — skip it.
            if c == '/' && chars.get(i + 1) == Some(&'/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '(' | '[' | '{' => {
                    depth += 1;
                    last = Some(c);
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    last = Some(c);
                }
                w if w.is_whitespace() => {}
                other => last = Some(other),
            }
            i += 1;
        }
        if in_str || depth > 0 {
            return true;
        }
        // A trailing binary / continuation operator means more is coming.
        matches!(
            last,
            Some('+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '=' | '<' | '>' | ',' | '.')
        )
    }

    /// @PLN12 phase 02 — parse one REPL input against the live session.
    ///
    /// Reuses the parser's accumulated `data` + `database`, so a definition
    /// from an earlier call is visible to this one.  This works because
    /// `data.reset()` (which `parse_str` calls between its two passes) clears
    /// only import scoping, never `definitions` — so `parse_str` *appends* the
    /// input's new definitions to the existing stdlib + session.
    ///
    /// Returns [`ParseResult::NeedMore`] for input that ends mid-construct,
    /// [`ParseResult::Error`] (with `data` rolled back) on a parse error, or
    /// [`ParseResult::Ready`] with the new definition's number on success.
    ///
    /// A top-level definition (`struct`/`enum`/`fn`/`type`/…) is parsed as-is.
    /// Any other input (an expression, a call, an assignment) is wrapped in a
    /// synthetic runnable `fn repl_<n>()` so it parses and can be executed;
    /// `entry_def_nr` then points at that wrapper.  Locals declared in such a
    /// statement do NOT yet persist across inputs — the `__repl_session`
    /// local-persistence path is the remaining increment, paired with phase 03's
    /// runtime that keeps the session instance alive.
    pub fn parse_statement(&mut self, input: &str) -> ParseResult {
        if Self::statement_incomplete(input) {
            return ParseResult::NeedMore;
        }
        let pre_defs = self.data.definitions();
        let pre_diag = self.diagnostics.entries().len();
        let is_def = Self::starts_top_level_def(input);
        // The wrapper's name is keyed on the pre-call def count, which is
        // monotonic across successful statements (each commit adds ≥1 def), so
        // it never collides; a rolled-back attempt truncates `definitions` back
        // to `pre_defs`, freeing the number for the next statement.
        let wrapper = format!("n_repl_{pre_defs}");
        if is_def {
            self.parse_str(input, "<repl>", false);
        } else {
            let src = format!("fn repl_{pre_defs}() {{\n{input}\n}}");
            self.parse_str(&src, "<repl>", false);
        }
        // Only diagnostics this statement produced — `Diagnostics::level` is
        // monotonic, so a prior error would otherwise mask a now-clean parse.
        let produced: Vec<DiagEntry> = self.diagnostics.entries()[pre_diag..].to_vec();
        if produced.iter().any(|e| e.level >= Level::Error) {
            self.data.rollback_to(pre_defs);
            return ParseResult::Error(produced);
        }
        // A definition isn't executed (nothing to run); a wrapped expression
        // returns its synthetic fn, resolved by name so a lambda appended inside
        // the body can't be mistaken for the entry point.
        let entry_def_nr = if is_def {
            u32::MAX
        } else {
            self.data.def_nr(&wrapper)
        };
        ParseResult::Ready { entry_def_nr }
    }

    /// True if `input` opens with a top-level definition keyword, so it can be
    /// parsed directly rather than wrapped in a synthetic REPL fn.
    pub(crate) fn starts_top_level_def(input: &str) -> bool {
        let word: String = input
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        matches!(
            word.as_str(),
            "struct" | "enum" | "fn" | "type" | "pub" | "use" | "interface" | "typedef" | "const"
        )
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
        let c_tp = self.data.def(c_nr).known_type();
        // Known_type may be unset for forward references; fall back to
        // the default integer slot (0) so the vector type still
        // registers correctly.  The content's own fill pass will
        // update once it runs.
        let resolved = if c_tp == u16::MAX { 0 } else { c_tp };
        self.database.vector(resolved)
    }

    /// @P314 — the element STORAGE type id to pass as the `OpAppendVector` /
    /// `vector_add` `tp` arg for a vector whose element type is `content`.
    /// Routes through `vector_of` (which consults `narrow_vector_content`) so a
    /// NARROW element (`u8`/`i16`/`i32`) resolves to its narrow storage type
    /// rather than the wide `integer`.  Using `def(type_def_nr(content))
    /// .known_type` instead loses the narrowness for integer aliases, so
    /// `vector_add` strides by 8 over a 1-/2-/4-byte-packed vector and corrupts
    /// (zeroes) the appended elements.  `single` (a distinct base type) is
    /// unaffected either way; this only matters for narrow integer aliases.
    pub(crate) fn append_elem_tp(&mut self, content: &Type) -> i32 {
        // A NESTED element (`content` is itself a vector) stores 16-byte
        // inline vector headers whose record type is the inner vector's own
        // database type — `vector_of(content)` already IS that element type
        // (the same `known` the proven `vv += [inner]` build path passes to
        // `record_new`, see @PLAN58 cluster IV in vectors.rs).  Unwrapping
        // it once more with `.content()` lands on the SCALAR type one level
        // down (`integer` → 0), so `vector_add` strides 8 over 16-byte rows
        // and never deep-copies the sub-vector claims — nested `a += b`
        // silently corrupted every row.
        let vec_tp = self.vector_of(content);
        if matches!(content, Type::Vector(_, _)) {
            return i32::from(vec_tp);
        }
        i32::from(self.database.content(vec_tp))
    }

    /// Get an iterator.
    /// The iterable expression is in *code.
    /// Creating the iterator will be in *code afterward.
    /// Return the next expression; with `Value::None` the iterator creation was impossible.
    /// @PLAN48 P2: true when converting `src` → `dst` narrows a loft integer to a
    /// smaller explicit width (e.g. `integer` → `i32`, or `i32` → `u8`), which
    /// loses data.  Widening (`i32` → `integer`) and same-width are not narrowing.
    /// A plain `integer`/`wide`/`u32` has no `forced_size` and is treated as 8 bytes.
    fn is_narrowing_int(src: &Type, dst: &Type) -> bool {
        let (Type::Integer(s), Type::Integer(d)) = (src, dst) else {
            return false;
        };
        let Some(dw) = d.forced_size else {
            return false; // widening / plain-integer target — never narrowing
        };
        let sw = s.forced_size.map_or(8u8, std::num::NonZeroU8::get);
        sw > dw.get()
    }

    /// @PLAN48 P2: render an integer type with its explicit narrow alias
    /// (`i32`/`u8`/`u16`/`i8`/`i16`) so a narrowing diagnostic doesn't print
    /// the bare `integer` for both sides (they share bounds).
    fn int_type_name(&self, t: &Type) -> String {
        if let Type::Integer(s) = t {
            match s.forced_size.map(std::num::NonZeroU8::get) {
                Some(4) => return "i32".to_string(),
                Some(2) => return if s.min < 0 { "i16" } else { "u16" }.to_string(),
                Some(1) => return if s.min < 0 { "i8" } else { "u8" }.to_string(),
                _ => {}
            }
        }
        t.name(&self.data)
    }

    /// @PLAN48 P2: literal exemption — true when `code` is a constant integer that
    /// provably fits `dst`'s full declared range, so `x: i32 = 5` / `f(5)` /
    /// `f(65535)` to a `u16` param stay legal without `as`.  This is the TYPE-fit
    /// question (a full-width register value); the narrower nullable-narrow-FIELD
    /// sentinel reservation is a separate, store-only check
    /// ([`Self::nullable_sentinel_hint`]) applied at the field-store sites.
    fn int_value_fits(&self, code: &Value, dst: &Type) -> bool {
        let Type::Integer(spec) = dst else {
            return false;
        };
        let n = match code.unspan() {
            Value::Int(n) => i64::from(*n),
            Value::Long(n) => *n,
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Int(n)) => i64::from(n),
                Some(Value::Long(n)) => n,
                _ => return false,
            },
        };
        n >= i64::from(spec.min) && n <= i64::from(spec.max)
    }

    /// When a literal stored into a NULLABLE narrow field fits the type's full
    /// range but lands on the reserved null sentinel (out of the usable range),
    /// return a hint explaining WHY — e.g. `255` in a nullable `u8`.  This tells
    /// the developer the value is the null encoding, not just "too big", and
    /// points at `not null` for the full range.  `None` for the ordinary
    /// out-of-range case (a generic narrowing message fits that).
    fn nullable_sentinel_hint(&self, code: &Value, dst: &Type, dst_name: &str) -> Option<String> {
        let Type::Integer(spec) = dst else {
            return None;
        };
        if spec.not_null {
            return None;
        }
        let n = match code.unspan() {
            Value::Int(n) => i64::from(*n),
            Value::Long(n) => *n,
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Int(n)) => i64::from(n),
                Some(Value::Long(n)) => n,
                _ => return None,
            },
        };
        let fits_full = n >= i64::from(spec.usable_min(false)) && n <= spec.usable_max(false);
        let fits_usable = n >= i64::from(spec.usable_min(true)) && n <= spec.usable_max(true);
        if fits_full && !fits_usable {
            Some(format!(
                "{n} is reserved as the null sentinel of a nullable {dst_name} \
                 (usable {}..={}); declare the field `not null` for the full range, \
                 or cast with `as {dst_name}`",
                spec.usable_min(true),
                spec.usable_max(true),
            ))
        } else {
            None
        }
    }

    fn convert(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        // @PLAN48 P2: implicitly narrowing a loft `integer` to a smaller explicit
        // width (e.g. `integer` → `i32`) loses data and must be an explicit `as`.
        // A constant that provably fits is exempt.  Emit here, then fall through to
        // the `is_equal` accept — the Error fails compilation, and returning via
        // `is_equal` avoids a second (generic) diagnostic from the caller's
        // `validate_convert` (which still sees integer/i32 as can_convert-compatible).
        if !self.first_pass
            && Self::is_narrowing_int(is_type, should)
            && !self.int_value_fits(code, should)
        {
            let src = self.int_type_name(is_type);
            let dst = self.int_type_name(should);
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot implicitly narrow {src} to {dst} (may lose data) — cast explicitly with `as {dst}`"
            );
        }
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
            for a in self.data.def(*enum_tp).attributes() {
                if a.name == self.data.def(*ref_tp).name() {
                    return true;
                }
            }
            // @PLN25 — a nullable struct SOURCE (`Reference(S)`, possibly the null
            // sentinel) flows into a synthetic `__nullable<S>` field.  Accept it here;
            // `handle_field` emits the wrap (null → discriminant 0, present → `Some`).
            if self.data.def(*enum_tp).name
                == format!("__nullable<{}>", self.data.def(*ref_tp).name())
            {
                return true;
            }
        }
        // @PLN25 single-payload — the REVERSE coercion: a `__nullable<S>` value flows
        // into a dense `S` slot (`f(v[i])` where `fn f(r: S)`, a `??` result, a
        // dense-local assign, a return).  The dense `S` IS the `Some` variant's inline
        // `payload` field, so unwrap by SUB-REFERENCING the payload — the sub-ref is a
        // valid dense `S` reference (it shares S's offset table), with NO copy.
        // (NOTE: the by-REF `&S` arg form is NOT handled here — a sub-ref reaches the
        // arg path as a by-VALUE `Reference` and gets copied, so a `&mut` write would not
        // propagate.  That seam is handled at the call-arg site, not in `convert`.)
        if let (Type::Enum(enum_d, true, _), Type::Reference(struct_d, _)) = (is_type, should)
            && self.data.def(*enum_d).name
                == format!("__nullable<{}>", self.data.def(*struct_d).name())
        {
            if !self.first_pass {
                let enum_d = *enum_d;
                let struct_d = *struct_d;
                let some_d = self.data.variant_of(enum_d, "Some");
                if some_d != u32::MAX {
                    // `get_val` for an INLINE struct field produces a sub-ref (pos+offset),
                    // not a pointer deref — the same form the field-access unwrap uses
                    // (fields.rs).  A raw `OpGetField` with the struct type would deref the
                    // payload bytes as a DbRef → garbage.  The payload IS dense `S`.
                    let payload_pos = u32::from(
                        self.database
                            .position(self.data.def(some_d).known_type(), "payload"),
                    );
                    *code = self.get_val(
                        &Type::Reference(struct_d, crate::data::Deps::none()),
                        false,
                        payload_pos,
                        code.clone(),
                        u32::MAX,
                    );
                }
            }
            return true;
        }
        // @PLN25 E2 — a synth `__nullable<S>` value used as a BOOLEAN (a condition
        // or bool arg, e.g. `if hash[k]` / `assert(hash[k])`) coerces to its
        // truthiness = "is present" = discriminant != 0.  Read the disc directly
        // (`OpGetEnum` @ offset 0, which has the rec==0 null-record guard, so an
        // ABSENT lookup result — rec 0 — reads disc 0 = not present), mirroring the
        // inline branch of `null_check_builder`.  Without this the raw nullable
        // DbRef reaches the bool context and native emits `(DbRef) as u8` (E0605);
        // a dense `Reference` lookup gets `OpConvBoolFromRef` on the same path.
        if let (Type::Enum(enum_d, true, _), Type::Boolean) = (is_type, should)
            && self.data.def(*enum_d).name.starts_with("__nullable<")
        {
            if !self.first_pass {
                let get_enum = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
                let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
                let is_null = self.cl("OpEqInt", &[disc, Value::Int(0)]);
                *code = self.cl("OpNot", &[is_null]);
            }
            return true;
        }
        if let Type::RefVar(ref_tp) = is_type
            && self.convert(code, ref_tp, should)
        {
            return true;
        }
        // @PLN25 single-payload — a `__nullable<S>` value flows into a `&S` (`RefVar(Reference
        // (S))`) parameter (`fn f(r: &S)` that may MUTATE through `r`).  Unwrap to the payload
        // sub-ref, then pass it BY-REFERENCE via a work-ref + `OpCreateStack` (with `skip_free`,
        // since the work-ref holds only a borrowed DbRef into the element, not its own store) —
        // exactly the complex-expression by-ref path below.  A `&mut` write through the sub-ref
        // then propagates straight back to the source element's payload.  This must run BEFORE
        // the `ref_tp.is_equal(is_type)` arm (which fails: `Reference(S)` != `Enum(__nullable<S>)`).
        if let Type::RefVar(ref_tp) = should
            && let Type::Reference(struct_d, _) = &**ref_tp
            && let Type::Enum(enum_d, true, _) = is_type
            && self.data.def(*enum_d).name
                == format!("__nullable<{}>", self.data.def(*struct_d).name())
        {
            if !self.first_pass {
                let struct_d = *struct_d;
                let enum_d = *enum_d;
                let some_d = self.data.variant_of(enum_d, "Some");
                if some_d != u32::MAX {
                    let payload_pos = u32::from(
                        self.database
                            .position(self.data.def(some_d).known_type(), "payload"),
                    );
                    let dense = Type::Reference(struct_d, Deps::none());
                    let sub = self.get_val(&dense, false, payload_pos, code.clone(), u32::MAX);
                    let wv = self.vars.work_refs(&dense, &mut self.lexer);
                    self.vars.set_skip_free(wv);
                    *code = Value::Insert(vec![
                        v_set(wv, sub),
                        self.cl("OpCreateStack", &[Value::Var(wv)]),
                    ]);
                }
            }
            return true;
        }
        if let Type::RefVar(ref_tp) = should
            && ref_tp.is_equal(is_type)
        {
            // #266: a receiver/argument that is ITSELF an already-borrowed
            // reference (its declared var type is `RefVar(_)`, e.g. a `&self`
            // parameter) must be forwarded to a `&`-parameter by VALUE, not
            // re-wrapped in `OpCreateStack`.  A borrowed-reference var's slot
            // already holds a DbRef that points one level up (toward the
            // owning struct); `OpCreateStack` would build a DbRef pointing at
            // *that slot*, so the callee's single `OpGetStackRef` deref lands
            // on the borrowing frame instead of the struct — the inner
            // method's writes hit the stack store and never persist (and
            // clobber the caller's reference slot).  Passing the var through
            // unchanged gives the callee exactly the same live borrow the
            // current frame holds.  Native passes the reference by the Rust
            // ABI and is immune, so this is interpreter-correctness parity.
            // Only non-text references need this — `&text` borrows have their
            // own work-text copy semantics handled in the Text arm below.
            if !matches!(**ref_tp, Type::Text(_))
                && let Value::Var(v) = code
                && matches!(self.vars.tp(*v), Type::RefVar(vinner) if vinner.is_equal(ref_tp))
            {
                return true;
            }
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
                        Type::Reference(self.data.def_nr("reference"), Deps::frame1(wv)),
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
        let r = Type::Reference(self.data.def_nr("reference"), Deps::none());
        let e = Type::Enum(0, false, Deps::none());
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
        // @PLN25: a null literal returned/assigned where a vector is expected becomes
        // the null sentinel (store_nr=u16::MAX), reusing the reference sentinel producer
        // — distinct from an empty `[]` (a valid store with length 0).
        if *is_type == Type::Null && matches!(should, Type::Vector(_, _)) {
            let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
            *code = Value::Call(sentinel_nr, vec![]);
            return true;
        }
        for &dnr in self.data.get_possible("OpConv", &self.lexer) {
            if self.data.def(dnr).name().ends_with("FromNull") {
                if *is_type == Type::Null {
                    if matches!(self.data.def(dnr).returned(), Type::Reference(_, _))
                        && let Type::Reference(_, _) = *should
                    {
                        // Use the non-allocating sentinel instead of OpConvRefFromNull so that
                        // null comparisons (`s == null`, `s != null`) do not leak a store.
                        let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
                        *code = Value::Call(sentinel_nr, vec![]);
                        return true;
                    } else if self.data.def(dnr).returned().is_equal(should) {
                        *code = Value::Call(dnr, vec![]);
                        return true;
                    }
                }
            } else if self.data.attributes(dnr) > 0
                && self.data.attr_type(dnr, 0).is_equal(check_type)
                && self.data.def(dnr).returned().is_equal(should)
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
            self.data.def(should_nr).known_type()
        };
        let is_nr = self.data.type_def_nr(is_type);
        let is_kt = if is_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(is_nr).known_type()
        };
        if let Type::Reference(tp, _) = should
            && self.data.def(*tp).returned().is_equal(is_type)
            && matches!(is_type, Type::Enum(_, true, _))
        {
            let get_e = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
            let get = self.cl("OpConvIntFromEnum", &[get_e]);
            if let Value::Enum(nr, _) = self.data.def(*tp).attributes()[0].value {
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
                && self.data.def(dnr).returned().is_same(should)
            {
                if let Type::Enum(tp, false, _) = should {
                    *code = Value::Call(
                        dnr,
                        vec![
                            code.clone(),
                            Value::Int(i32::from(self.data.def(*tp).known_type())),
                        ],
                    );
                } else {
                    *code = Value::Call(dnr, vec![code.clone()]);
                }
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned().is_same(should)
                && should_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(should_kt))]);
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned().is_same(should)
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
                && self.data.def(*o).name() == "enumerate"
            {
                return true;
            }
            if let (Type::Reference(r_nr, _), Type::Enum(e_nr, true, _)) = (test_type, should)
                && e_nr == r_nr
            {
                return true;
            }
            // @PLN25 E2 — a `__nullable<S>` value is accepted into a dense `S`
            // slot; `convert` emits the payload sub-ref (gap 2).
            if let (Type::Enum(enum_d, true, _), Type::Reference(struct_d, _)) = (test_type, should)
                && self.data.def(*enum_d).name
                    == format!("__nullable<{}>", self.data.def(*struct_d).name())
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

    fn validate_convert(&mut self, context: &str, test_type: &Type, should: &Type, pos: &Position) {
        if !self.first_pass && !self.can_convert(test_type, should) {
            // Plan-07 phase 6 (partial) — "expected E, got G on context"
            // reads the same direction as English ("we expected this,
            // we got that"); the old shape "G should be E on context"
            // forced a mental flip and confused users new to the
            // language.  `pos` is the offending value's start, captured at
            // parse time — the lexer cursor has drifted to the `;` by now.
            diagnostic_at!(
                self.lexer,
                pos,
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
                let name = self.data.def(child_nr).name();
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
    #[allow(clippy::too_many_arguments)]
    fn call(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
        named_args: &[(String, Value, Type)],
        arg_pos: &[Position],
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
                Some(self.data.def(d_nr).def_type().clone())
            },
            self.first_pass,
        );
        // skip generic templates — they are not callable directly.
        if d_nr != u32::MAX && self.data.def(d_nr).def_type() == DefType::Generic {
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
            self.call_with_named(code, d_nr, list, types, named_args, true, arg_pos)
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
            let def_name = self.data.def(d_nr).name();
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
    /// #395 — route a monomorph's concrete `Type::Tuple` return through the
    /// synthetic `__tuple<…>` struct, exactly as `parser/definitions.rs`'s
    /// `needs_tuple_rewrite` (~line 897) does for a normally-parsed tuple return.
    /// A generic TEMPLATE deliberately defers that rewrite ("`T` resolves later")
    /// and nothing re-applied it once `T` was concrete — so the copied body returns
    /// a DbRef (the template compiled `T` as a `Reference` dummy) while the declared
    /// return stayed `Tuple`, and the caller read the DbRef inline as garbage
    /// (interp) / mismatched the Rust tuple ABI (native E0308).  Both the first-pass
    /// prediction and the second-pass instantiation route through HERE so the
    /// receiving variable's type agrees across passes.  Wide (>8 B) or
    /// lifetime-bearing tuples are rewritten; an 8-byte pure-value tuple and
    /// fn-element tuples keep their existing ABI (mirrors the deferral predicate).
    fn tuple_return_rewrite(&mut self, returned: Type, from_type_var: bool) -> Type {
        // Only the `-> T` shape needs this.  When the template return type IS the
        // bare type variable, the body delivers T as a DbRef (the template compiled
        // T as a `Reference` dummy), so a tuple substitution must wrap it in the
        // synthetic struct.  A return type that is a LITERAL tuple in the signature
        // (e.g. `-> (integer, integer)`) is constructed BY VALUE in the body and
        // correctly uses the bare-tuple ABI — rewriting it would break the
        // value-tuple generic returns (p329/p330/p240/plan17).
        if !from_type_var {
            return returned;
        }
        let Type::Tuple(elems) = &returned else {
            return returned;
        };
        let wide = u32::from(crate::variables::size(
            &returned,
            &crate::data::Context::Argument,
        )) > 8;
        let has_fn = elems.iter().any(|e| matches!(e, Type::Function(_, _, _)));
        if elems.iter().any(crate::data::has_lifetime_concern) || (wide && !has_fn) {
            let elems_clone = elems.clone();
            let synth = self.data.tuple_def(&mut self.lexer, &elems_clone);
            Type::Reference(synth, crate::data::Deps::none())
        } else {
            returned
        }
    }

    /// Reads the generic template's already-populated `returned` field, applies
    /// the type substitution, then routes a concrete tuple return through the
    /// synthetic `__tuple` struct (see [`tuple_return_rewrite`]) so the predicted
    /// type matches what `try_generic_instantiation` later produces (else the
    /// receiving variable would "change type" between passes — #395).  Registers
    /// the synthetic struct on first encounter (idempotent via `tuple_def`);
    /// otherwise side-effect-free and safe to call repeatedly.
    fn predict_generic_return_type(&mut self, name: &str, types: &[Type]) -> Type {
        let generic_name = format!("n_{name}");
        let g_nr = self.data.def_nr(&generic_name);
        if g_nr == u32::MAX || self.data.def(g_nr).def_type() != DefType::Generic {
            return Type::Unknown(0);
        }
        if types.is_empty() || types[0].is_unknown() {
            return Type::Unknown(0);
        }
        let tv_nr = Self::extract_type_var(&self.data.def(g_nr).attributes()[0].typedef);
        if tv_nr == u32::MAX {
            return Type::Unknown(0);
        }
        let concrete = Self::resolve_type_var(
            &self.data.def(g_nr).attributes()[0].typedef,
            tv_nr,
            &types[0],
        );
        if concrete.is_unknown() {
            return Type::Unknown(0);
        }
        let tmpl_returned = self.data.definitions[g_nr as usize].returned.clone();
        let from_tv = matches!(&tmpl_returned, Type::Reference(d, _) if *d == tv_nr);
        let predicted = self.tuple_return_rewrite(
            Self::substitute_type(tmpl_returned, tv_nr, &concrete),
            from_tv,
        );
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
        if g_nr == u32::MAX || self.data.def(g_nr).def_type() != DefType::Generic {
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
        let tv_nr = Self::extract_type_var(&self.data.def(g_nr).attributes()[0].typedef);
        if tv_nr == u32::MAX {
            return u32::MAX;
        }
        let concrete = Self::resolve_type_var(
            &self.data.def(g_nr).attributes()[0].typedef,
            tv_nr,
            &types[0],
        );
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
            // @PLN25 E2 — this mangled name becomes a Rust function identifier in
            // native codegen.  A concrete element type whose NAME carries angle
            // brackets / commas (synthetic wrappers — `__nullable<Row>`,
            // `__tuple<…>`) would emit `fn t_15__nullable<Row>_count(…)`, which
            // rustc parses as a chained comparison.  Flatten those to
            // identifier-safe chars.  The replacement is 1:1 (each bracket/comma
            // → one `_`), so the LEN prefix that `original_name` /
            // `find_method_receivers` parse back stays correct.  Plain names
            // (user structs, `vector`) contain none of these and are unchanged.
            let safe = self
                .data
                .def(type_nr)
                .name()
                .replace(['<', '>', ',', ' '], "_");
            format!("t_{}{}_{name}", safe.len(), safe)
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
        // `from_tv` computed on the PRE-substitution template return, identically to
        // `predict_generic_return_type`, so the second-pass instantiated return type
        // matches the first-pass prediction (the cross-pass H5 contract).
        let from_tv = matches!(&tmpl_returned, Type::Reference(d, _) if *d == tv_nr);
        let new_returned = self.tuple_return_rewrite(
            Self::substitute_type(tmpl_returned, tv_nr, &concrete),
            from_tv,
        );
        // Register the new definition.
        let d_nr = self.data.add_def(&mangled, &tmpl_pos, DefType::Function);
        for a in &tmpl_attrs {
            let a_nr = self
                .data
                .add_attribute(&mut self.lexer, d_nr, &a.name, a.typedef.clone());
            self.data.set_attr_value(d_nr, a_nr, a.default.clone());
        }
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
        // P241 fix (2026-05-11): post-substitution rewrite of the
        // parametric vector-element-write triplet to the primitive
        // shape, plus elm-var type patch.  Runs after both code
        // substitution AND vars substitution because the patch needs
        // to override `vars`' substituted-to-primitive elm var type
        // back to `Reference(...)` (it holds a DbRef, not the
        // primitive value).  See `rewrite_generic_vector_writes`.
        let new_code = Self::rewrite_generic_vector_writes(
            new_code,
            &concrete,
            &mut vars,
            &self.data,
            &mut self.database,
        );
        self.data.definitions[d_nr as usize].code = new_code;
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
        let concrete_name = self.data.def(concrete_nr).name().to_string();
        let mut satisfied = true;
        for iface_nr in bounds {
            let iface_name = self.data.def(iface_nr).name().to_string();
            let children: Vec<u32> = self.data.children_of(iface_nr).collect();
            for child_nr in children {
                let child_name = self.data.def(child_nr).name().to_string();
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
                let concrete_type = self.data.def(concrete_nr).returned().clone();
                let mut found = self.data.find_fn(u16::MAX, &method_suffix, &concrete_type);
                // @PLN25 E2 — a synth `__nullable<S>` delegates method/interface
                // resolution to its underlying `S` (a method call on a nullable
                // element unwraps through `Some` to call `S`'s method), so satisfy
                // the bound against `S`'s methods when the wrapper itself lacks them.
                // Gate-off-inert (no `__nullable<` type exists).
                if found == u32::MAX
                    && let Some(inner) = concrete_name
                        .strip_prefix("__nullable<")
                        .and_then(|r| r.strip_suffix('>'))
                {
                    let s_nr = self.data.def_nr(inner);
                    if s_nr != u32::MAX {
                        let s_type = self.data.def(s_nr).returned().clone();
                        found = self.data.find_fn(u16::MAX, &method_suffix, &s_type);
                    }
                }
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
        // `Rewritten(T)` is a value-construction marker (e.g. `P { v: 99 }`
        // becoming an Insert sequence) that should not propagate into the
        // bound T — the type variable describes the data shape, not how
        // a particular argument got assembled.  Strip it before unifying.
        if let Type::Rewritten(inner) = concrete_tp {
            return Self::resolve_type_var(template_tp, tv_nr, inner);
        }
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
        if def.attributes().is_empty() {
            return d_nr;
        }
        // Check if any attribute's type references the type variable.
        let has_tv = def.attributes.iter().any(|a| a.typedef.contains_def(tv_nr));
        if !has_tv {
            // Also check for Integer(0, tv_nr) patterns — operators sometimes encode
            // type info in the Integer bounds.
            return d_nr;
        }
        // Resolve the concrete first-arg type by substituting tv_nr in the attribute type.
        let concrete_arg =
            Self::substitute_type(def.attributes()[0].typedef.clone(), tv_nr, concrete);
        // Extract the user-facing function name from the mangled definition name.
        // Mangled names: "t_<LEN><Type>_<name>" or "n_<name>" or operator names.
        let name = def.name();
        let fn_name = if let Some(rest) = name.strip_prefix("t_") {
            // Skip the LEN digits and type name, extract name after the underscore.
            if let Some(idx) = rest.find('_') {
                &rest[idx + 1..]
            } else {
                name
            }
        } else if let Some(rest) = name.strip_prefix("n_") {
            rest
        } else {
            // Operator name — use as-is for find_fn.
            name
        };
        let mut resolved = data.find_fn(u16::MAX, fn_name, &concrete_arg);
        // @PLN25 E2 — a bounded-generic method call whose receiver monomorphises to a synth
        // `__nullable<S>` (a nullable vector element, e.g. `for x in v: vector<T>` where
        // `T = IfItem` → `__nullable<IfItem>`, then `x.is_valid()`) must resolve to S's CONCRETE
        // method via the `Some` payload — the nullable enum itself has no methods, so `find_fn`
        // returns MAX and the parametric bound stub `t_1T_<m>` (emitted `todo!()` in native)
        // would leak to runtime (86).  Mirror the interface-satisfaction unwrap above: retry
        // against S.  Gate-off-inert (no `__nullable<` type exists).
        if resolved == u32::MAX
            && let Type::Enum(nd, true, _) = &concrete_arg
            && data.def(*nd).name().starts_with("__nullable<")
        {
            let some = data.variant_of(*nd, "Some");
            let pa = if some == u32::MAX {
                usize::MAX
            } else {
                data.attr(some, "payload")
            };
            if pa != usize::MAX
                && let Type::Reference(s, _) = data.attr_type(some, pa)
            {
                let s_type = data.def(s).returned().clone();
                resolved = data.find_fn(u16::MAX, fn_name, &s_type);
            }
        }
        if resolved != u32::MAX && resolved != d_nr {
            resolved
        } else {
            d_nr
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
    /// Walks a generic-template's IR and substitutes the type variable
    /// `tv_nr` with the concrete `concrete` type, both in variable types
    /// and in IR-shape decisions that depend on T's resolved shape.
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
                //
                // P252 fix (2026-05-11): also recognise `OpGetVectorNullable`
                // — plan-07 phase 4 step 4.6 swapped the for-loop iter step
                // from `OpGetVector` to its Nullable peer (so OOB at end-of-
                // iteration returns null instead of raising).  Without this
                // arm, bounded-generic for-loops over a struct vector left
                // the iter step at `OpGetVectorNullable(v, 0, idx)` with
                // size=0 — every iteration read element 0, producing the
                // FIRST item's value for every iteration (P252).  The
                // Nullable peer's arg shape is identical to OpGetVector
                // (r, size, idx) so the elm_size fixup logic is unchanged.
                if new_d != u32::MAX
                    && (new_d as usize) < data.definitions.len()
                    && (data.def(new_d).name() == "OpGetVector"
                        || data.def(new_d).name() == "OpGetVectorNullable")
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
                // P239 fix (2026-05-11): the for-loop iter-termination
                // check generated by `parser/collections.rs::iter_for`
                // emits `OpConvBoolFromRef(Var(loop_var))` for any
                // loop variable typed `Reference` — including
                // `Reference(T_d_nr, …)` for generic-T element
                // iteration.  When T monomorphises to a primitive
                // (integer / text / float / single / character /
                // enum), the substituted Var is now that primitive
                // type but the IR still has `OpConvBoolFromRef`,
                // which crashes interp (treats `i64` as a `DbRef` —
                // SIGSEGV) and breaks native (rustc E0610 `i64.rec`).
                // Swap the conversion op to the matching primitive
                // peer when the substituted concrete type tells us
                // what shape the value actually is.
                if new_d != u32::MAX
                    && (new_d as usize) < data.definitions.len()
                    && data.def(new_d).name() == "OpConvBoolFromRef"
                    && new_args.len() == 1
                {
                    let conv_name = match concrete {
                        Type::Integer(_) => Some("OpConvBoolFromInt"),
                        Type::Text(_) => Some("OpConvBoolFromText"),
                        Type::Float => Some("OpConvBoolFromFloat"),
                        Type::Single => Some("OpConvBoolFromSingle"),
                        Type::Enum(_, false, _) => Some("OpConvBoolFromEnum"),
                        // Reference / Vector / struct-enum / tuple stay
                        // on OpConvBoolFromRef (the existing behaviour
                        // works for any DbRef-shaped loop variable).
                        _ => None,
                    };
                    if let Some(name) = conv_name {
                        let conv_d_nr = data.def_nr(name);
                        if conv_d_nr != u32::MAX {
                            return Value::Call(conv_d_nr, new_args);
                        }
                    }
                }
                // I9-text fixup: when a T-stub had an extra __work_1 parameter
                // (for text-returning interface methods) but the concrete method
                // doesn't, drop the trailing argument to match the concrete signature.
                if new_d != d && new_d != u32::MAX && (new_d as usize) < data.definitions.len() {
                    let concrete_params = data.def(new_d).attributes().len();
                    if new_args.len() > concrete_params {
                        let mut trimmed = new_args;
                        trimmed.truncate(concrete_params);
                        return Value::Call(new_d, trimmed);
                    }
                }
                Value::Call(new_d, new_args)
            }
            Value::Block(bl) => {
                let recursed: Vec<Value> = bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, data))
                    .collect();
                Value::Block(Box::new(crate::data::Block {
                    operators: recursed,
                    result: Self::substitute_type(bl.result, tv_nr, concrete),
                    name: bl.name,
                    scope: bl.scope,
                    var_size: bl.var_size,
                }))
            }
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
            // P237: tuple-constructor elements may contain calls to
            // bound-supplied operator stubs (`t_1T_OpAdd(x, x)` etc.).
            // Without this recursion the call stayed pointing at the
            // generic stub instead of the concrete `t_<len>integer_OpAdd`,
            // producing rustc E0308 (`expected DbRef, found i64`) at
            // codegen time and silent garbage / SIGSEGV under interp.
            Value::Tuple(elems) => Value::Tuple(
                elems
                    .into_iter()
                    .map(|e| Self::substitute_type_in_value(e, tv_nr, concrete, data))
                    .collect(),
            ),
            Value::TuplePut(v, idx, val) => Value::TuplePut(
                v,
                idx,
                Box::new(Self::substitute_type_in_value(*val, tv_nr, concrete, data)),
            ),
            Value::BreakWith(n, val) => Value::BreakWith(
                n,
                Box::new(Self::substitute_type_in_value(*val, tv_nr, concrete, data)),
            ),
            Value::Yield(val) => Value::Yield(Box::new(Self::substitute_type_in_value(
                *val, tv_nr, concrete, data,
            ))),
            Value::CallRef(v_nr, args) => Value::CallRef(
                v_nr,
                args.into_iter()
                    .map(|a| Self::substitute_type_in_value(a, tv_nr, concrete, data))
                    .collect(),
            ),
            other => other,
        }
    }

    /// P241 fix (2026-05-11) — slice 2: integer-only.  POST-PASS that
    /// walks the substituted IR + patches the variable table to
    /// rewrite the parametric vector-element-write triplet to its
    /// primitive shape.  Runs AFTER `substitute_type_in_value` AND
    /// AFTER `vars.substitute_type` so it sees the substituted-types
    /// IR and can patch the elm variable's type back to `Reference`
    /// (it was originally `Reference(T_d_nr, deps)` and
    /// `vars.substitute_type` turned it into the wrong primitive
    /// type because it holds a DbRef, not a primitive value).
    ///
    /// Detects the post-substitution triplet:
    ///
    /// ```ignore
    /// // Triplet at adjacent positions i, i+1, i+2:
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_T), Int(MAX)]))
    /// Call(OpCopyRecord, [src_value, Var(elm_var), Int(t_T)])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_T), Int(MAX)])
    /// ```
    ///
    /// When `concrete` is a primitive (slice 2: only `Type::Integer`),
    /// rewrites the triplet to the primitive shape:
    ///
    /// ```ignore
    /// // 4-op sequence:
    /// Call(OpPreAllocVector, [Var(out_var), Int(1), Int(elem_size)])
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_concrete_vec), Int(MAX)]))
    /// Call(OpSetInt, [Var(elm_var), Int(0), src_value])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_concrete_vec), Int(MAX)])
    /// ```
    ///
    /// AND patches `vars.set_type(elm_var, Type::Reference(...))`.
    ///
    /// Recurses into nested Blocks / Loops / Ifs / etc.  Slice 2 ships
    /// only `Type::Integer`; struct-T is a no-op (the existing
    /// OpCopyRecord path is correct because the source IS a DbRef).
    /// P241 fix slice 3 — true when `concrete` is a primitive type
    /// whose vector-element write uses a primitive setter
    /// (`OpSetInt` / `OpSetText` / `OpSetFloat` / etc.) at the
    /// parse-time concrete-T path instead of `OpCopyRecord`.
    /// `Type::Reference` / `Type::Vector` / etc. are NOT primitive
    /// targets — their vector-element shape uses `OpCopyRecord`,
    /// which the parametric IR already encodes correctly (the source
    /// IS a DbRef regardless of substitution).
    pub(crate) fn is_primitive_vector_element_target(tp: &Type) -> bool {
        matches!(
            tp,
            Type::Integer(_)
                | Type::Float
                | Type::Single
                | Type::Boolean
                | Type::Character
                | Type::Text(_)
                | Type::Function(_, _, _)
                | Type::Enum(_, false, _) // plain enum (struct-enums use OpCopyRecord)
        )
    }

    /// Does the rewrite apply to this concrete type?  Both primitive and
    /// struct (Reference) targets need the type-id substitution; the only
    /// difference is whether OpCopyRecord is replaced (primitive) or kept
    /// with patched type-id args (struct).
    pub(crate) fn is_rewritable_vector_element_target(tp: &Type) -> bool {
        Self::is_primitive_vector_element_target(tp) || matches!(tp, Type::Reference(_, _))
    }

    /// P241 fix slice 3 — build the per-type primitive setter Call
    /// for the rewritten triplet's middle op.  Mirrors the parse-time
    /// concrete-T dispatch in `parser/vectors.rs:1560-1599`.
    /// Returns `None` only when the type is not a primitive target
    /// (the caller should pre-check via `is_primitive_vector_element_target`).
    fn primitive_setter_call(
        concrete: &Type,
        elm_var: u16,
        src_value: Value,
        data: &Data,
    ) -> Option<Value> {
        let elm = Value::Var(elm_var);
        let pos = Value::Int(0);
        // Resolve op def_nrs.  Each branch resolves only the ones it needs.
        let op = match concrete {
            Type::Integer(spec) => {
                // narrow-int dispatch mirrors `vectors.rs:1576-1586`.
                // size(N) on an integer alias selects a narrower setter.
                let alias_nr = data.type_elm(concrete);
                let forced = data.forced_size(alias_nr);
                match forced {
                    Some(1) => {
                        let m = Value::Int(spec.min);
                        let d = data.def_nr("OpSetByte");
                        Value::Call(d, vec![elm, pos, m, src_value])
                    }
                    Some(2) => {
                        let m = Value::Int(spec.min);
                        let d = data.def_nr("OpSetShortRaw");
                        Value::Call(d, vec![elm, pos, m, src_value])
                    }
                    Some(4) => {
                        let d = data.def_nr("OpSetInt4");
                        Value::Call(d, vec![elm, pos, src_value])
                    }
                    _ => {
                        let d = data.def_nr("OpSetInt");
                        Value::Call(d, vec![elm, pos, src_value])
                    }
                }
            }
            Type::Float => {
                let d = data.def_nr("OpSetFloat");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Single => {
                let d = data.def_nr("OpSetSingle");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Boolean => {
                // Booleans store as a 0/1 byte; same shape as `set_field`'s
                // Boolean arm at `parser/mod.rs:2799`.  Must wrap the
                // raw boolean value in `if val { 1 } else { 0 }` so the
                // OpSetByte writes the correct integer encoding —
                // without the wrap, OpSetByte sees a DbRef-shaped
                // boolean that codegen can't decode.
                let d = data.def_nr("OpSetByte");
                let wrapped = crate::data::v_if(src_value, Value::Int(1), Value::Int(0));
                Value::Call(d, vec![elm, pos, Value::Int(0), wrapped])
            }
            Type::Character => {
                let d = data.def_nr("OpSetCharacter");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Text(_) => {
                let d = data.def_nr("OpSetText");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Function(_, _, _) => {
                // Plan-06 phase 4d.A.2 — fn-ref vector elements store the
                // 4-byte i32 d_nr.  Same shape as `vectors.rs:1597`.
                let d = data.def_nr("OpSetInt4");
                Value::Call(d, vec![elm, pos, src_value])
            }
            Type::Enum(_, false, _) => {
                // Plain enum — variants encode as a small integer index.
                let d = data.def_nr("OpSetEnum");
                Value::Call(d, vec![elm, pos, src_value])
            }
            _ => return None,
        };
        // Defensive: if any def_nr lookup returned u32::MAX, return None
        // so the caller falls through (rather than emitting a malformed Call).
        if let Value::Call(d, _) = &op
            && *d == u32::MAX
        {
            return None;
        }
        Some(op)
    }

    pub(crate) fn rewrite_generic_vector_writes(
        val: Value,
        concrete: &Type,
        vars: &mut crate::variables::Function,
        data: &Data,
        database: &mut Stores,
    ) -> Value {
        // Slice 2: Integer.  Slice 3: extended to all primitive
        // types whose vector-element shape uses a primitive setter
        // instead of OpCopyRecord.  P255 (2026-05-12): also handle
        // struct T (Type::Reference) — the OpCopyRecord shape is
        // kept but its tp arg AND the surrounding OpNewRecord /
        // OpFinishRecord parent_tp args must be patched from the
        // parametric T type-id to the concrete struct's type-ids,
        // otherwise the runtime reads the wrong record size.
        if !Self::is_rewritable_vector_element_target(concrete) {
            return val;
        }
        match val {
            Value::Block(bl) => {
                let recursed: Vec<Value> = bl
                    .operators
                    .into_iter()
                    .map(|v| Self::rewrite_generic_vector_writes(v, concrete, vars, data, database))
                    .collect();
                let rewritten =
                    Self::rewrite_vector_write_triplets(recursed, concrete, vars, data, database);
                Value::Block(Box::new(crate::data::Block {
                    operators: rewritten,
                    result: bl.result,
                    name: bl.name,
                    scope: bl.scope,
                    var_size: bl.var_size,
                }))
            }
            Value::Loop(lp) => {
                let recursed: Vec<Value> = lp
                    .operators
                    .into_iter()
                    .map(|v| Self::rewrite_generic_vector_writes(v, concrete, vars, data, database))
                    .collect();
                let rewritten =
                    Self::rewrite_vector_write_triplets(recursed, concrete, vars, data, database);
                Value::Loop(Box::new(crate::data::Block {
                    operators: rewritten,
                    result: lp.result,
                    name: lp.name,
                    scope: lp.scope,
                    var_size: lp.var_size,
                }))
            }
            Value::If(c, t, f) => Value::If(
                Box::new(Self::rewrite_generic_vector_writes(
                    *c, concrete, vars, data, database,
                )),
                Box::new(Self::rewrite_generic_vector_writes(
                    *t, concrete, vars, data, database,
                )),
                Box::new(Self::rewrite_generic_vector_writes(
                    *f, concrete, vars, data, database,
                )),
            ),
            Value::Set(v, expr) => Value::Set(
                v,
                Box::new(Self::rewrite_generic_vector_writes(
                    *expr, concrete, vars, data, database,
                )),
            ),
            Value::Return(expr) => Value::Return(Box::new(Self::rewrite_generic_vector_writes(
                *expr, concrete, vars, data, database,
            ))),
            Value::Drop(expr) => Value::Drop(Box::new(Self::rewrite_generic_vector_writes(
                *expr, concrete, vars, data, database,
            ))),
            Value::Span(b) => {
                let (pos, inner) = *b;
                Value::Span(Box::new((
                    pos,
                    Self::rewrite_generic_vector_writes(inner, concrete, vars, data, database),
                )))
            }
            Value::Call(d, args) => Value::Call(
                d,
                args.into_iter()
                    .map(|a| Self::rewrite_generic_vector_writes(a, concrete, vars, data, database))
                    .collect(),
            ),
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|v| Self::rewrite_generic_vector_writes(v, concrete, vars, data, database))
                    .collect(),
            ),
            other => other,
        }
    }

    /// P241 fix (2026-05-11) — slice 2: integer-only.  Walks a Block's
    /// post-substitution operator list looking for the parametric
    /// vector-element-write triplet emitted by
    /// `parser/vectors.rs::new_record` (for parametric T):
    ///
    /// ```ignore
    /// // Triplet at adjacent positions i, i+1, i+2:
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_T), Int(MAX)]))
    /// Call(OpCopyRecord, [src_value, Var(elm_var), Int(t_T_known)])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_T), Int(MAX)])
    /// ```
    ///
    /// When `concrete` is a primitive (slice 2: only `Type::Integer`),
    /// rewrites the triplet to the primitive shape:
    ///
    /// ```ignore
    /// // 4-op sequence:
    /// Call(OpPreAllocVector, [Var(out_var), Int(1), Int(elem_size)])
    /// Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(t_concrete_vec), Int(MAX)]))
    /// Call(OpSetInt, [Var(elm_var), Int(0), src_value])
    /// Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(t_concrete_vec), Int(MAX)])
    /// ```
    ///
    /// Where:
    /// - `elem_size = type_element_size(concrete, data)`
    /// - `t_concrete_vec = database.vector(database.db_type(concrete, data))`
    ///   (mirrors the parse-time concrete path at `vectors.rs:1532-1535`).
    ///
    /// Slice 2 ships only the `Type::Integer` arm.  Slice 3 extends to
    /// Text / Float / Single / Boolean / Character / Enum / Function +
    /// narrow-int variants.  For struct T (the `Type::Reference` shape)
    /// the rewrite is a no-op — the existing OpCopyRecord path is
    /// correct because the source IS a DbRef.
    ///
    /// Tolerates `Value::Span` and `Value::Line` markers between or
    /// inside the triplet operators by using `unspan` for shape
    /// matching.  Multi-element pushes (`out += [a, b, c]`) produce
    /// N adjacent triplets; this function processes them
    /// independently — each triplet rewrites once.
    fn rewrite_vector_write_triplets(
        ops: Vec<Value>,
        concrete: &Type,
        vars: &mut crate::variables::Function,
        data: &Data,
        database: &mut Stores,
    ) -> Vec<Value> {
        // Slice 3 covers all primitive vector-element targets
        // (Integer/Text/Float/Single/Boolean/Character/Enum/Function +
        // narrow-int variants).  P255 extends to struct T (Reference)
        // by keeping OpCopyRecord and patching its tp arg.
        if !Self::is_rewritable_vector_element_target(concrete) {
            return ops;
        }
        let is_struct_target = matches!(concrete, Type::Reference(_, _));
        // Resolve op def_nrs once — re-resolution per-triplet would
        // cost N lookups for a long Block.
        let new_record_d = data.def_nr("OpNewRecord");
        let copy_record_d = data.def_nr("OpCopyRecord");
        let finish_record_d = data.def_nr("OpFinishRecord");
        let pre_alloc_d = data.def_nr("OpPreAllocVector");
        if new_record_d == u32::MAX
            || copy_record_d == u32::MAX
            || finish_record_d == u32::MAX
            || pre_alloc_d == u32::MAX
        {
            return ops; // missing op definitions — bail safely
        }
        // Look up the concrete vector-element record type-id.
        // Mirrors `vectors.rs:1532-1535` — `database.vector(content_db_type)`
        // returns the synthetic vector<concrete> type id (registers
        // it on first use; idempotent on subsequent calls).
        let content_db_type = database.db_type(concrete, data);
        let concrete_vec_tp = i32::from(database.vector(content_db_type));
        let elem_size = Self::type_element_size(concrete, data);
        // Walk operators looking for the triplet.  Build a new vec
        // with rewrites applied; copy unchanged ops verbatim.
        // Drain `ops` into a deque-like cursor so we can take owned
        // values without index juggling — rewriting consumes 3 ops
        // and produces 4, so we can't use simple in-place mutation.
        let mut iter = ops.into_iter();
        let mut out: Vec<Value> = Vec::new();
        let mut buf: Vec<Value> = Vec::new();
        loop {
            // Refill buffer to at least 3 entries (the triplet length).
            while buf.len() < 3 {
                match iter.next() {
                    Some(v) => buf.push(v),
                    None => break,
                }
            }
            if buf.len() < 3 {
                // Tail — nothing left that could be a full triplet.
                out.extend(buf);
                break;
            }
            let matched = Self::match_vector_write_triplet(
                &buf[0],
                &buf[1],
                &buf[2],
                new_record_d,
                copy_record_d,
                finish_record_d,
            );
            if let Some((elm_var, out_var, src_value)) = matched {
                // Consume the matched triplet.
                buf.drain(0..3);
                // Patch the elm var's type back to `Reference(content_def_nr, [out_var])`.
                // After `vars.substitute_type`, `Reference(T_d_nr, deps)` became
                // `Type::<concrete>` (deps lost — primitive types don't carry deps).
                // But the elm var is the destination of `OpNewRecord`, so it
                // holds a DbRef regardless of T's concrete type.  Without this
                // patch, codegen emits the wrong opcodes for elm reads/writes
                // (e.g. `VarInt` instead of `VarRef`) and the runtime reads
                // garbage.  The dep on `out_var` mirrors `unique_elm_var`'s
                // `self.vars.depend(elm, vec)` so elm doesn't outlive the
                // backing store.
                let content_def_nr = data.type_def_nr(concrete);
                vars.set_type(
                    elm_var,
                    Type::Reference(content_def_nr, Deps::frame1(out_var)),
                );
                // 1. OpPreAllocVector(Var(out_var), Int(1), Int(elem_size))
                //    Mirrors `vectors.rs:1161-1178` for perf parity with
                //    concrete-T vector pushes.
                out.push(Value::Call(
                    pre_alloc_d,
                    vec![Value::Var(out_var), Value::Int(1), Value::Int(elem_size)],
                ));
                // 2. Set(elm_var, OpNewRecord(Var(out_var), Int(concrete_vec_tp), Int(MAX)))
                out.push(Value::Set(
                    elm_var,
                    Box::new(Value::Call(
                        new_record_d,
                        vec![
                            Value::Var(out_var),
                            Value::Int(concrete_vec_tp),
                            Value::Int(i32::from(u16::MAX)),
                        ],
                    )),
                ));
                // 3. Middle op:
                //    - Primitive T: per-type setter (OpSetInt / OpSetText / …)
                //      replaces OpCopyRecord (no DbRef to copy).
                //    - Struct T: keep OpCopyRecord but patch its tp arg
                //      from the parametric T's known_type to the concrete
                //      struct's known_type so `state::copy_record` reads
                //      the correct record size.
                if is_struct_target {
                    let known_tp = if (content_def_nr as usize) < data.definitions.len() {
                        i32::from(data.def(content_def_nr).known_type())
                    } else {
                        i32::from(u16::MAX)
                    };
                    out.push(Value::Call(
                        copy_record_d,
                        vec![src_value, Value::Var(elm_var), Value::Int(known_tp)],
                    ));
                } else {
                    let setter = Self::primitive_setter_call(concrete, elm_var, src_value, data);
                    if let Some(call) = setter {
                        out.push(call);
                    } else {
                        // Concrete type matched `is_primitive_vector_element_target`
                        // but no setter mapping exists — should not happen.
                        // Bail by emitting a no-op (the OpFinishRecord still
                        // fires; the value just isn't written).  Defensive
                        // fall-through to keep the IR well-formed.
                    }
                }
                // 4. OpFinishRecord(Var(out_var), Var(elm_var), Int(concrete_vec_tp), Int(MAX))
                out.push(Value::Call(
                    finish_record_d,
                    vec![
                        Value::Var(out_var),
                        Value::Var(elm_var),
                        Value::Int(concrete_vec_tp),
                        Value::Int(i32::from(u16::MAX)),
                    ],
                ));
            } else {
                // No triplet at this position — emit one op and slide.
                out.push(buf.remove(0));
            }
        }
        out
    }

    /// P241 fix slice 2 — pattern-matches the parametric vector-element
    /// write triplet.  Returns `Some((elm_var, out_var, src_value))`
    /// when all three statements match the expected shape AND the
    /// per-statement vars cross-reference correctly:
    /// - The Set's target var equals the OpCopyRecord's 2nd arg's
    ///   var equals the OpFinishRecord's 2nd arg's var (`elm_var`).
    /// - The OpNewRecord's 1st arg's var equals the OpFinishRecord's
    ///   1st arg's var (`out_var`).
    /// - All Int args match expected sentinel shape (Int(MAX) for
    ///   `fld`; matching parametric type-id between OpNewRecord and
    ///   OpFinishRecord).
    ///
    /// `src_value` is the OpCopyRecord's 1st arg (the value being
    /// copied into the new vector slot).  Returned by-value (cloned
    /// from the matched IR) so the caller can construct the new
    /// `OpSetInt` Call without borrowing back into `ops`.
    fn match_vector_write_triplet(
        op0: &Value,
        op1: &Value,
        op2: &Value,
        new_record_d: u32,
        copy_record_d: u32,
        finish_record_d: u32,
    ) -> Option<(u16, u16, Value)> {
        // op0: Set(elm_var, Call(OpNewRecord, [Var(out_var), Int(_), Int(MAX)]))
        let (elm_var_set, out_var, _new_record_tp) = match op0.unspan() {
            Value::Set(elm, set_val) => {
                let inner = set_val.unspan();
                if let Value::Call(d, args) = inner
                    && *d == new_record_d
                    && args.len() == 3
                    && let Value::Var(out) = args[0].unspan()
                    && let Value::Int(tp) = args[1].unspan()
                    && let Value::Int(fld) = args[2].unspan()
                    && *fld == i32::from(u16::MAX)
                {
                    (*elm, *out, *tp)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // op1: Call(OpCopyRecord, [src_value, Var(elm_var), Int(_)])
        let src_value = match op1.unspan() {
            Value::Call(d, args) => {
                if *d == copy_record_d
                    && args.len() == 3
                    && let Value::Var(elm_in_copy) = args[1].unspan()
                    && *elm_in_copy == elm_var_set
                {
                    args[0].clone()
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        // op2: Call(OpFinishRecord, [Var(out_var), Var(elm_var), Int(_), Int(MAX)])
        match op2.unspan() {
            Value::Call(d, args) => {
                if *d == finish_record_d
                    && args.len() == 4
                    && let Value::Var(out_in_finish) = args[0].unspan()
                    && let Value::Var(elm_in_finish) = args[1].unspan()
                    && let Value::Int(_finish_tp) = args[2].unspan()
                    && let Value::Int(fld) = args[3].unspan()
                    && *out_in_finish == out_var
                    && *elm_in_finish == elm_var_set
                    && *fld == i32::from(u16::MAX)
                {
                    Some((elm_var_set, out_var, src_value))
                } else {
                    None
                }
            }
            _ => None,
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
                    && data.def(*d_nr).def_type() == DefType::Struct
                {
                    let mut total = 0i32;
                    for attr in data.def(*d_nr).attributes() {
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
        let op_name = match tp {
            Type::Integer(_) => "OpGetInt",
            Type::Float => "OpGetFloat",
            Type::Single => "OpGetSingle",
            Type::Text(_) => "OpGetText",
            // @PLN17: byte-stored boolean read, preserving 0/1/255 (like enum).
            Type::Boolean => "OpGetBoolean",
            _ => return code, // reference/struct types: no wrapper needed
        };
        let d = data.def_nr(op_name);
        if d == u32::MAX {
            return code;
        }
        Value::Call(d, vec![code, p])
    }

    /// Resolve named arguments into positional slots, then delegate to `call_nr`.
    #[allow(clippy::too_many_arguments)]
    fn call_with_named(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        positional: &[Value],
        pos_types: &[Type],
        named: &[(String, Value, Type)],
        is_method: bool,
        arg_pos: &[Position],
    ) -> Type {
        if named.is_empty() {
            return self.call_nr(code, d_nr, positional, pos_types, is_method, arg_pos);
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
        // Named args are reordered into parameter slots, so the parse-order
        // `arg_pos` no longer aligns; fall back to the cursor for the rare
        // named-mismatch case.
        self.call_nr(code, d_nr, &args, &arg_types, is_method, &[])
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
            self.database
                .position(self.data.def(d_nr).known_type(), &nm)
        };
        // Post-2c: pass the field's alias def_nr so `get_val` can honor
        // size(N) for integer subtypes (e.g. i32 → OpGetInt4).
        let alias = if f_nr == usize::MAX {
            u32::MAX
        } else {
            self.data.def(d_nr).attributes()[f_nr].alias_d_nr
        };
        // P215: for fn-ref fields with the legacy 4B int layout, the
        // database has no `<attr>__closure_rec` half — reading at
        // pos+4 would corrupt the next attribute.  Synthesise the
        // null DbRef sentinel for the closure half instead.  The 8B
        // split layout (assigned by a capturing lambda) keeps the
        // existing dual-read path.  The split/legacy answer comes
        // from `fn_ref_field_is_split` (the database layout), NOT
        // from `assigned_lambda_d_nr` directly — the flag is only
        // set when the assigning body parses, so a body parsed
        // earlier would wrongly see the legacy layout (#313).
        if let Type::Function(_, _, _) = &tp
            && f_nr != usize::MAX
            && !self.fn_ref_field_is_split(d_nr, f_nr)
        {
            let read_dnr = self.cl("OpGetInt4", &[code, Value::Int(i32::from(pos))]);
            let read_clos = self.cl("OpNullRefSentinel", &[]);
            return crate::data::v_block(
                vec![read_dnr, read_clos],
                tp.clone(),
                "fn_ref_field_read",
            );
        }
        self.get_val(&tp, nullable, u32::from(pos), code, alias)
    }

    /// #322: switch the lexer to a `use`-resolved dependency file,
    /// recording it for the program startup-cache manifest first.
    /// Library files are parsed via an inline lexer switch (never a
    /// `parse()` entry), so without this record the manifest misses
    /// them and an edited library keeps executing from the stale
    /// cached program.
    fn switch_to_dep(&mut self, f: &str) {
        if self.track_sources && !self.parsed_sources.iter().any(|s| s == f) {
            self.parsed_sources.push(f.to_string());
        }
        self.lexer.switch(f);
    }

    /// Is the fn-ref struct field `d_nr.f_nr` stored in the split 8B
    /// layout (`<attr>` d_nr + `<attr>__closure_rec`) rather than the
    /// legacy 4B int layout?
    ///
    /// In the second pass the database layout is the answer's one
    /// stable home: it was built from the COMPLETE first pass, while
    /// `assigned_lambda_d_nr` is derived during body parsing — a body
    /// parsed before the assigning body cannot trust the flag and
    /// would bake in the wrong field shape (#313).  Deriving from the
    /// layout keeps the read/write shape in lockstep with the bytes
    /// actually laid out.  In the first pass the layout does not
    /// exist yet, so fall back to the flag; the first pass's IR is
    /// discarded and only the flag's end state (consumed by
    /// `typedef::fill_database`) matters.
    fn fn_ref_field_is_split(&self, d_nr: u32, f_nr: usize) -> bool {
        if self.first_pass {
            self.data.def(d_nr).attributes()[f_nr].assigned_lambda_d_nr != u32::MAX
        } else {
            let nm = self.data.attr_name(d_nr, f_nr);
            let kt = self.data.def(d_nr).known_type();
            self.database.position(kt, &format!("{nm}__closure_rec")) != u16::MAX
        }
    }

    /// #318: does `tp` (transitively) store a capturing closure — a
    /// struct with a capturing-lambda fn field (`assigned_lambda_d_nr`
    /// set), reached through struct fields, vector/keyed-collection
    /// content, or tuple elements?
    ///
    /// Values of such types are frame-bound: the closure record holds
    /// raw DbRefs into the stores of the frame that owns the captures,
    /// and copying the value into storage that outlives that frame (a
    /// return value, another struct's field, a collection element)
    /// leaves dangling DbRefs — silent cross-object corruption once
    /// the store slot is reused.  The three escape sinks reject on
    /// this predicate; locals and downward argument passing stay free.
    pub(crate) fn type_carries_closure(&self, tp: &Type) -> bool {
        // Like `fn_ref_field_is_split`, derive from the registered
        // database layout (built from the COMPLETE first pass) rather
        // than `assigned_lambda_d_nr`, so the answer is independent of
        // pass-2 body-parse order.
        fn walk_def(
            data: &Data,
            db: &crate::database::Stores,
            d: u32,
            seen: &mut std::collections::HashSet<u32>,
        ) -> bool {
            if d == u32::MAX || (d as usize) >= data.definitions.len() || !seen.insert(d) {
                return false;
            }
            let kt = data.def(d).known_type();
            data.def(d).attributes().iter().any(|a| {
                db.position(kt, &format!("{}__closure_rec", a.name)) != u16::MAX
                    || walk(data, db, &a.typedef, seen)
            })
        }
        fn walk(
            data: &Data,
            db: &crate::database::Stores,
            tp: &Type,
            seen: &mut std::collections::HashSet<u32>,
        ) -> bool {
            match tp {
                // #328: a `reference<T>` POINTER field (the u16::MAX share
                // marker) copies only a 12-byte DbRef, never the record —
                // it cannot smuggle a closure record's bytes, so the
                // carrying-walk stops at the pointer boundary.  (Escaping
                // a pointer to frame-local state is the generic, documented
                // reference<T> borrow hazard — not the #318 copy class.)
                Type::Reference(_, deps) if deps.contains(&u16::MAX) => false,
                Type::Reference(d, _) => walk_def(data, db, *d, seen),
                Type::Vector(c, _) => walk(data, db, c, seen),
                Type::Hash(d, _, _)
                | Type::Sorted(d, _, _)
                | Type::Index(d, _, _)
                | Type::Spacial(d, _, _) => walk_def(data, db, *d, seen),
                Type::Tuple(elems) => elems.iter().any(|e| walk(data, db, e, seen)),
                _ => false,
            }
        }
        walk(
            &self.data,
            &self.database,
            tp,
            &mut std::collections::HashSet::new(),
        )
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
                // H4-medium: the op KIND comes from the ONE width→op home
                // (`NarrowIntKind::of`), so this READ op and the matching WRITE
                // op in `set_field_check` cannot drift.  A nullable byte STRUCT
                // FIELD decodes the reserved 256th code to null (`ByteNullable`);
                // a narrow-vector element keeps the raw direct encoding (its
                // stride/value contract is the narrow-vector one, not the
                // field-sentinel one) — `narrow_vec` selects that.
                let kind = crate::data::NarrowIntKind::of(s, nullable, narrow_vec);
                if kind.takes_min() {
                    // H6: a nullable narrow FIELD reserves the all-ones code for
                    // null, so its usable range (and the `min` the read decodes
                    // against) shrinks by one edge — `usable_min` is the one home
                    // shared with the write op + range-check.  Narrow-VECTOR
                    // elements use the raw path (no field sentinel), so they keep
                    // the full `min`.
                    let mn = spec.usable_min(nullable && !narrow_vec);
                    self.cl(kind.get_op(), &[code, p, Value::Int(mn)])
                } else {
                    self.cl(kind.get_op(), &[code, p])
                }
            }
            Type::Enum(_, false, _) => self.cl("OpGetEnum", &[code, p]),
            // @PLN17: byte-stored boolean read, preserving 0/1/255 (like enum).
            Type::Boolean => self.cl("OpGetBoolean", &[code, p]),
            Type::Float => self.cl("OpGetFloat", &[code, p]),
            Type::Single => self.cl("OpGetSingle", &[code, p]),
            // A `vector<character>` element read had no `get_val` arm (only the
            // write side — OpSetCharacter — existed), so `v[0]` / `for c in v`
            // fell through to "Field access not supported on type character".
            // Mirror the OpSetCharacter write with the OpGetCharacter read.
            Type::Character => self.cl("OpGetCharacter", &[code, p]),
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
            Type::Reference(_, deps) => {
                if deps.is_empty() {
                    // Inline struct field: OpGetField adds the field offset to the base ref.
                    // Linked/base type dereference is handled at the call site (fields.rs)
                    // using OpVectorRef, which combines the 4-byte pointer read + deref.
                    let info = self.type_info(tp);
                    self.cl("OpGetField", &[code, p, info])
                } else {
                    // Plan-22 phase 02b (2026-05-12): auto-Reference field
                    // — the field stores a 12-byte DbRef pointing at the
                    // source record (shared storage).  Read the full
                    // DbRef back via OpGetDbRef.  Phase 02c is the
                    // producer that gives a closure-record attribute
                    // non-empty deps; user struct fields always have
                    // empty deps (legacy inline-bytes path above).
                    self.cl("OpGetDbRef", &[code, p])
                }
            }
            Type::Function(_, _, _) => {
                // P213: storage is two database fields per loft attribute
                //   `<attr>`              — 4B i32 holding the lambda's d_nr
                //   `<attr>__closure_rec` — 4B vector header at pos+4
                //                            (empty = non-capturing /
                //                            default-init; populated =
                //                            capturing closure record
                //                            co-located in host's Store).
                // Read both halves: `OpGetInt4` pushes the 8B d_nr,
                // `OpVectorFirstOrNull` pushes the 12B closure DbRef
                // (or the null sentinel when the vector is empty).
                // Together they form the 20B stack-side fn-ref slot
                // shape `fn_call_ref` already consumes unchanged.
                let read_dnr = self.cl("OpGetInt4", &[code.clone(), p.clone()]);
                let crec_pos = match &p {
                    Value::Int(pi) => Value::Int(pi + 4),
                    _ => Value::Int(0),
                };
                let crec_field = self.cl("OpGetField", &[code, crec_pos, Value::Int(0)]);
                let read_clos = self.cl("OpRefFromChildRec", &[crec_field]);
                crate::data::v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
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
            // Pass-1 deferral: reading a field whose declared type is still
            // `Unknown` (a forward-referenced or cross-package field type, e.g.
            // `struct Box { inner: Cell }` parsed above `struct Cell`) must not
            // emit — `actual_types_deferred` resolves the field type after this
            // pass, and pass-2 re-reads it with the concrete type.  Emitting
            // here would (via the pass-2 gate) suppress that resolution.  In
            // pass-2 a still-`Unknown` type is genuinely undefined and its
            // "Undefined type" error already fires from type resolution, so
            // fall through to the diagnostic only there.
            Type::Unknown(_) if self.first_pass => Value::Null,
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
        // PLAN51 V-b — when the source is a Call returning the
        // heap-promoted form of THIS tuple shape (i.e.
        // `Type::Reference(__tuple<…>, _)` whose def_nr matches
        // tuple_d_nr we just resolved), the native codegen would
        // otherwise stash a `DbRef` return into a `Type::Tuple` work-ref
        // and emit an illegal Rust `as (DbRef, DbRef)` cast at
        // `src/generation/dispatch.rs:459-468` (rustc E0605).  Skip
        // the per-element TupleGet stash and emit a SINGLE
        // OpCopyRecord that deep-copies the entire inner-tuple struct
        // into the host field at `base_pos`.  Mirrors the
        // `Type::Reference(_, deps.is_empty())` arm in
        // `set_field_check` (line 3202-3216).
        let promoted_src_def: Option<u32> = if self.first_pass {
            None
        } else {
            match val_code.unspan() {
                Value::Call(d_nr, _) => {
                    if let Type::Reference(d, _) = self.data.def(*d_nr).returned()
                        && *d == tuple_d_nr
                    {
                        Some(tuple_d_nr)
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };
        if let Some(inner_d) = promoted_src_def {
            let inner_kt = i32::from(self.data.def(inner_d).known_type());
            let field_ref = self.cl(
                "OpGetField",
                &[
                    ref_code.clone(),
                    Value::Int(i32::from(base_pos)),
                    Value::Int(inner_kt),
                ],
            );
            return vec![self.cl("OpCopyRecord", &[val_code, field_ref, Value::Int(inner_kt)])];
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
                //
                // P251 fix (2026-05-11): for `Value::Var(v)` where `v`
                // has type `Function`, wrap in `Value::FnRefDnr(v)` so
                // native emit projects `(var_v.0 as i64)` (the d_nr
                // half of the runtime `(u32, DbRef)` tuple).  Without
                // this projection, native codegen emits
                // `let _v_val = (var_v); ... _v_val as i32` which
                // rustc rejects (E0308: expected `(u32, DbRef)`,
                // found `i64`; E0605: non-primitive cast).  The
                // direct fn-ref-struct-field-write path
                // (`emit_fn_ref_field_write`, parser/mod.rs:4886)
                // already does this projection — extending it to the
                // tuple-element-of-struct-field path closes the
                // remaining gap.
                let d_nr_only = match value {
                    Value::FnRef(d_nr, _, _) => Value::Int(d_nr),
                    Value::Var(v)
                        if matches!(self.vars.tp(v), Type::Function(_, _, _))
                            && !self.closure_vars.contains_key(&v) =>
                    {
                        Value::FnRefDnr(v)
                    }
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
                    Value::Int(i32::from(self.data.def(*inner_d_nr).known_type()))
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

    /// @PLN25 single-payload — emit the steps that turn a `Some` record (`some_ref`, type
    /// `some_d`) into the PRESENT state from a dense-`S` source `src`: set the discriminant
    /// present (offset 0), then copy the whole dense `S` into the inline `payload` field.
    /// Single-payload makes `Some = { enum, payload: S }`, so this is ONE record copy (the
    /// `Reference` arm of `set_field_check` emits the `OpGetField`+`OpCopyRecord`), replacing
    /// the per-field copy loops the individual-field layout forced.  The caller owns the
    /// null branch (set the discriminant 0) and any source stashing.
    pub(crate) fn build_some_present(
        &mut self,
        some_d: u32,
        some_ref: Value,
        src: Value,
    ) -> Vec<Value> {
        let payload_attr = self.data.attr(some_d, "payload");
        vec![
            self.cl(
                "OpSetEnum",
                &[some_ref.clone(), Value::Int(0), Value::Enum(2, u16::MAX)],
            ),
            self.set_field_no_check(some_d, payload_attr, 0, some_ref, src),
        ]
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
        // #318 sink R2: a closure-carrying struct value cannot be
        // copied into another struct's field — the copy's closure
        // record keeps raw DbRefs into the constructing frame, which
        // the field's host may outlive (silent corruption on slot
        // reuse).  The direct fn-field write (Type::Function arm) IS
        // the supported feature and stays; `emit_check == false` is
        // the closure-record population path (captures share by
        // DbRef, no copy) and is exempt.
        if emit_check
            && !self.first_pass
            && !matches!(tp, Type::Function(_, _, _))
            && self.type_carries_closure(&tp)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "field `{nm}` would store a value of a type that holds a capturing \
                 closure; such values are bound to the function frame that owns the \
                 captures and cannot be copied into another struct — keep the closure \
                 holder in a local variable and pass it down as an argument (#318)"
            );
            return Value::Null;
        }
        let pos = self
            .database
            .position(self.data.def(d_nr).known_type(), &nm);
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
                // Post-2c: honor size(N) on the alias recorded during field
                // parsing; fall back to the limit()-based heuristic.
                let alias_nr = if f_nr == usize::MAX {
                    u32::MAX
                } else {
                    self.data.def(d_nr).attributes()[f_nr].alias_d_nr
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
                    self.data.def(d_nr).name(),
                    if f_nr == usize::MAX {
                        "<unknown>".to_string()
                    } else {
                        self.data.def(d_nr).attributes()[f_nr].name.clone()
                    },
                );
                // H4-medium: same width→op home (`NarrowIntKind::of`) as the
                // READ in `get_val`, so the write op matches the read op for the
                // field.  A NULLABLE byte STRUCT FIELD reserves the 256th code as
                // the null sentinel (the Nullable op pair translates null ↔ the
                // sentinel); `not null` fields and narrow-vector elements keep the
                // raw op.
                let nullable = f_nr != usize::MAX && self.data.attr_nullable(d_nr, f_nr);
                let kind = crate::data::NarrowIntKind::of(s, nullable, narrow_vec);
                // H6: the WRITE op encodes against the same `usable_min` the READ
                // op (`get_val`) decodes against — a nullable narrow field reserves
                // the all-ones code for null, shrinking the usable range by one
                // edge; narrow-VECTOR elements keep the full `min` (raw path).
                let m = Value::Int(spec.usable_min(nullable && !narrow_vec));
                if kind.takes_min() {
                    self.cl(kind.set_op(), &[ref_code, pos_val, m, val_code])
                } else {
                    self.cl(kind.set_op(), &[ref_code, pos_val, val_code])
                }
            }
            Type::Vector(ref content, _)
                if f_nr != usize::MAX && !matches!(val_code.unspan(), Value::Int(_)) =>
            {
                // A real struct/tuple vector FIELD write — e.g. the tuple-return
                // heap-promotion (`control.rs::rewrite_tail_tuple_with_work_ref`)
                // hands `val_code` the WHOLE vector.  Deep-copy it into the
                // field's own freshly-allocated store, exactly as
                // `emit_set_one_element` (above) and the struct constructor
                // (`objects.rs`) already do.  A bare `OpSetInt4` here writes the
                // 8-byte vector DbRef as a 4-byte int → stack skew → garbage
                // write into the locked CONST_STORE (`store.rs:1374` tuple-return
                // crash; native rejects the `DbRef as i32` cast).  The
                // `f_nr == usize::MAX` narrow-vec `insert` path (raw 4-byte
                // header) keeps `OpSetInt4` in the arm below.
                let vec_tp = self.vector_of(content);
                let elem_db_tp = self.database.content(vec_tp);
                let field_ref = self.cl(
                    "OpGetField",
                    &[ref_code, pos_val, Value::Int(i32::from(vec_tp))],
                );
                self.cl(
                    "OpAppendVector",
                    &[field_ref, val_code, Value::Int(i32::from(elem_db_tp))],
                )
            }
            Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Spacial(_, _, _)
            | Type::Sorted(_, _, _) => {
                // Collection header is a 4-byte u32 record pointer.  Post-2c
                // `OpSetInt` writes 8 bytes (i64), which overflows into the
                // next field.  Use `OpSetInt4` to write only 4 bytes.  Reached
                // by the narrow-vec `insert` raw-header path (`f_nr ==
                // usize::MAX`); keyed-collection struct fields deep-copy earlier
                // in `objects.rs`.
                self.cl("OpSetInt4", &[ref_code, pos_val, val_code])
            }
            Type::Function(_, _, _) => {
                // P213: storage is now TWO database fields per loft
                // attribute — `<attr>` (4B int holding the lambda's
                // d_nr; database name matches the loft attribute name
                // so `database.position(name)` lookups still resolve)
                // and `<attr>__closure_rec` (4B vector header at
                // `pos + 4`, pointing at the co-located closure record
                // in host's Store; empty for non-capturing).  At
                // read time `OpGetInt4` recovers the d_nr;
                // `OpVectorFirstOrNull` recovers the closure DbRef.
                //
                // Field-write paths:
                //   - Non-capturing lambda (`FnRef(d, MAX, _)`):
                //     write d_nr only; the closure_rec vector stays
                //     at its zero default (no records).
                //   - Capturing lambda (a `fn_ref_with_closure` Block
                //     ending in `FnRef(d, w, _)`): run the lambda's
                //     existing alloc_steps to build the closure
                //     record in parent's Store under `var(w)`, then
                //     write d_nr + `OpAppendVector` deep-copies the
                //     parent-Store closure record into element [0]
                //     of host's `__closure_rec` vector.  Parent's
                //     record gets freed by the existing
                //     parent-scope `OpFreeRef(w)`; host owns its own
                //     deep-copied copy.
                //   - Non-inline source (Var / Call returning a
                //     fn-ref): diagnose ("only inline lambda
                //     literals can be stored in fn-ref struct
                //     fields in this release") — out of scope here;
                //     follow-on plan extends to the non-inline case.
                //
                // Record the lambda's d_nr on the host attribute so
                // `typedef::fill_database`'s `Type::Function` arm
                // can register the `__closure_rec` field with the
                // correct closure-record schema.  Heterogeneous
                // captures across multiple constructors of the same
                // host struct are diagnosed at the second site.
                if let Some((lambda_d, _w)) = find_capturing_fn_ref(&self.data, &val_code)
                    && f_nr != usize::MAX
                {
                    let lambda_d_u = lambda_d as u32;
                    let prev = self.data.def(d_nr).attributes()[f_nr].assigned_lambda_d_nr;
                    if prev == u32::MAX {
                        self.data.definitions[d_nr as usize].attributes[f_nr]
                            .assigned_lambda_d_nr = lambda_d_u;
                    } else if prev != lambda_d_u && !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "heterogeneous capture shapes per fn-ref struct field are not supported \
                             (this lambda's captured environment differs from the previously-assigned \
                              lambda's); split into two structs or unify the captures"
                        );
                    }
                    // #318 sink R2: the host being written must be
                    // rooted in a frame-local — writing a capturing
                    // closure into (a field of) an ARGUMENT claims the
                    // closure record into a store that outlives this
                    // frame, while the record's DbRefs point at this
                    // frame's captures (silent corruption on slot
                    // reuse once the frame dies).
                    if !self.first_pass
                        && let Some(base) = ref_code.base_var()
                        && self.vars.is_argument(base)
                    {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "cannot store a capturing closure into a struct received as an \
                             argument — the closure references state owned by this \
                             function's frame, which the argument's struct outlives; \
                             construct the closure in the frame that owns the captured \
                             state (#318)"
                        );
                        return Value::Null;
                    }
                }
                emit_fn_ref_field_write(self, d_nr, f_nr, ref_code, pos_val, &val_code)
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
            Type::Reference(inner_tp, deps) => {
                if deps.is_empty() {
                    // The value is a 12-byte DbRef; OpSetInt would only read 4 bytes of it.
                    // Copy the struct bytes into the embedded field instead.
                    let type_nr = if self.first_pass {
                        Value::Int(i32::from(u16::MAX))
                    } else {
                        Value::Int(i32::from(self.data.def(inner_tp).known_type()))
                    };
                    let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
                    // Note: the free-source high-bit for Issue #120 is set in
                    // copy_ref() (operators.rs), which is the path for struct
                    // field reassignment. This set_field_check path is for
                    // construction (initial field population).
                    self.cl("OpCopyRecord", &[val_code.clone(), field_ref, type_nr])
                } else {
                    // Plan-22 phase 02b (2026-05-12): auto-Reference field
                    // — store the source's 12-byte DbRef directly,
                    // sharing the underlying record.  Phase 02c sets
                    // non-empty deps for mutated Reference captures on
                    // closure records; today's user code paths keep
                    // empty deps and stay on the legacy OpCopyRecord
                    // path above.
                    self.cl("OpSetDbRef", &[ref_code, pos_val, val_code])
                }
            }
            Type::Enum(_, false, _) => self.cl("OpSetEnum", &[ref_code, pos_val, val_code]),
            Type::Enum(nr, true, _) => {
                // A struct-enum field holds an inline record; copy the source enum
                // into the field AT ITS OFFSET via a field sub-ref — exactly like
                // the Type::Reference arm above.  Copying into `ref_code` (the
                // struct base) instead landed a non-first struct-enum field at
                // offset 0, clobbering field 0 and reading a garbage discriminant
                // (#406: `Entry { key: a, value: b }` from enum variables).
                let type_nr = if self.first_pass {
                    // known_type() is u16::MAX until the enum registers in pass 2;
                    // emit the placeholder now (codegen re-runs in pass 2), matching
                    // the Type::Reference arm.
                    Value::Int(i32::from(u16::MAX))
                } else {
                    Value::Int(i32::from(self.data.def(nr).known_type()))
                };
                let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
                self.cl("OpCopyRecord", &[val_code, field_ref, type_nr])
            }
            // @PLN17: store the boolean's u8 form (0/1/255) directly, like enum —
            // the old `if val {1} else {0}` forced 0/1 and dropped the null sentinel.
            Type::Boolean => self.cl("OpSetBoolean", &[ref_code, pos_val, val_code]),
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
        let check = self.data.def(d_nr).attributes()[f_nr].check.clone();
        let bound = Self::replace_record_ref(check, &ref_val);
        let msg = if let Value::Text(s) = &self.data.def(d_nr).attributes()[f_nr].check_message {
            Value::Text(s.clone())
        } else {
            Value::Text(format!(
                "field constraint failed on {}.{field_name}",
                self.data.def(d_nr).name()
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
                    Type::Reference(tv_nr, Deps::none())
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
                let tp = self.call_nr(code, stub_nr, list, types, false, &[]);
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
                if self.data.def(pos).name() == "OpEqBool"
                    && types.len() >= 2
                    && ((matches!(types[0], Type::Character) && matches!(types[1], Type::Text(_)))
                        || (matches!(types[0], Type::Text(_))
                            && matches!(types[1], Type::Character)))
                {
                    continue;
                }
                let tp = self.call_nr(code, pos, list, types, false, &[]);
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
        // @PLN25 E2 — `!nullable` (a `__nullable<S>` value in boolean position)
        // means "is absent" (discriminant 0): no struct operator overload
        // matches, so lower it directly (mirrors the `== null` is-null lowering).
        // The common shape is `!v[oob]` ("out-of-bounds is null").
        if op == "Not"
            && types.len() == 1
            && let Type::Enum(syn, true, _) = &types[0]
            && self.data.def(*syn).name.starts_with("__nullable<")
        {
            let get_enum = self.cl("OpGetEnum", &[list[0].clone(), Value::Int(0)]);
            let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
            *code = self.cl("OpEqInt", &[disc, Value::Int(0)]);
            return Type::Boolean;
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
        arg_pos: &[Position],
    ) -> Type {
        let mut all_types = Vec::from(types);
        if self.data.def_type(d_nr) == DefType::Dynamic {
            for a_nr in 0..self.data.attributes(d_nr) {
                let Type::Routine(r_nr) = self.data.attr_type(d_nr, a_nr) else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Incorrect dynamic function {}",
                        self.data.def(d_nr).name()
                    );
                    return Type::Void;
                };
                if self.data.attr_type(r_nr, 0).is_equal(&types[0]) {
                    return self.call_nr(code, r_nr, list, types, report, arg_pos);
                }
            }
            diagnostic!(
                self.lexer,
                Level::Error,
                "No matching function {}",
                self.data.def(d_nr).name()
            );
        } else if !matches!(self.data.def_type(d_nr), DefType::Function) {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown definition {}",
                    self.data.def(d_nr).name()
                );
            }
            return Type::Null;
        }
        let mut actual = self.process_call_args(d_nr, list, types, &mut all_types, report, arg_pos);
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
        arg_pos: &[Position],
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
                    self.data.def(d_nr).name()
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
                // Defer on pass 1 (#375): a field access on a struct whose
                // layout is not yet finalised — because one of its fields is a
                // forward / cross-package reference still resolving — lowers to
                // `Null` here, which is not addressable.  Erroring now would
                // abort pass 1 before the type resolves; on pass 2 the layout is
                // complete and the access lowers to `OpGetField` (addressable),
                // so the check passes.  A genuine literal-to-`&` is still an
                // error: it is non-addressable on pass 2 too, where this fires.
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot pass a literal or expression to a '&' parameter — \
                         assign to a variable first"
                    );
                }
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
                    // `arg_pos[nr]` is the argument's start, captured in
                    // `parse_call`; the lexer cursor has drifted to `)` / `,`.
                    // Synthetic / reordered call paths pass an empty slice and
                    // fall back to the cursor.
                    let pos = arg_pos
                        .get(nr)
                        .cloned()
                        .unwrap_or_else(|| self.lexer.pos().clone());
                    self.validate_convert(&context, actual_type, &tp, &pos);
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
        let tp = self.data.def(d_nr).returned().clone();
        // for Reference returns (structs), filter out hidden return-mechanism
        // attributes from dep resolution. The struct owns its store independently —
        // hidden return-store buffers are implementation artifacts.
        // Text/Vector returns genuinely depend on their hidden work buffers.
        let attrs = self.data.def(d_nr).attributes();
        let filter_hidden = |d: &[u16]| -> Vec<u16> {
            d.iter()
                .copied()
                .filter(|&i| (i as usize) >= attrs.len() || !attrs[i as usize].hidden)
                .collect()
        };
        if let Type::Text(d) = tp {
            Type::Text(Deps::frame(Self::resolve_deps(types, d.as_attr_indices())))
        } else if let Type::Vector(to, d) = tp {
            Type::Vector(
                to,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Sorted(to, key, d) = tp {
            Type::Sorted(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Hash(to, key, d) = tp {
            Type::Hash(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Index(to, key, d) = tp {
            Type::Index(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Spacial(to, key, d) = tp {
            Type::Spacial(
                to,
                key,
                Deps::frame(Self::resolve_deps(types, d.as_attr_indices())),
            )
        } else if let Type::Reference(to, d) = tp {
            Type::Reference(
                to,
                Deps::frame(Self::resolve_deps(
                    types,
                    &filter_hidden(d.as_attr_indices()),
                )),
            )
        } else if let Type::Enum(to, true, d) = tp {
            Type::Enum(
                to,
                true,
                Deps::frame(Self::resolve_deps(
                    types,
                    &filter_hidden(d.as_attr_indices()),
                )),
            )
        } else {
            tp
        }
    }

    /// THE def→frame dep converter (H2 / DEPS_INVENTORY): maps the
    /// callee's ATTR-INDEX deps through the actual argument types at a
    /// call site into caller FRAME var deps.  `d` must be attr-space
    /// (callers read it via `Deps::as_attr_indices`); the result is
    /// wrapped `Deps::frame` by `call_dependencies`.
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
        // @PLAN59 phase 2: the `__rref_N` recursive-self counter dance is
        // gone.  It existed to keep `__ref_N` numbering pass-stable so
        // `ref_return` could re-find its promoted attr BY NAME instead of
        // growing the attr count across passes.  With the signature-time
        // `__retbuf` the attr exists before any body parses and the
        // promotion re-find no longer depends on work-ref numbering.
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
                let default = self.data.def(d_nr).attributes()[a_nr].value.clone();
                let tp = self.data.attr_type(d_nr, a_nr);
                if let Type::Vector(content, _) = &tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    // #306: the attr's dep list is callee-internal (attr
                    // indices); inherited verbatim it reads as CALLER var
                    // numbers and mislabels the fresh buffer a borrow.
                    let buf_tp = Type::Vector(content.clone(), Deps::none());
                    let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
                    // @PLAN51 Cluster IV: tag this work-ref so parse_code's
                    // preamble emits Set(vr, Null) regardless of vr's
                    // typedef dep list.  Without it, if-tail / recursion
                    // shapes leave vr without a first_def → slot allocator
                    // skips → codegen panics.
                    self.vars.mark_caller_hidden_buf(vr);
                    self.data.vector_def(&mut self.lexer, content);
                    all_types[a_nr] = Type::Vector(content.clone(), Deps::frame1(vr));
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::Reference(content, _) = tp {
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    // #306: strip the callee-internal dep list (see Vector arm).
                    let buf_tp = Type::Reference(content, Deps::none());
                    let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
                    self.vars.mark_caller_hidden_buf(vr);
                    all_types[a_nr] = Type::Reference(content, Deps::frame1(vr));
                    actual[a_nr] = Value::Var(vr);
                } else if let Type::Enum(content, true, _) = tp {
                    // @P301 — struct-enums are heap records like
                    // Reference/Vector, so a struct-enum return-slot
                    // promoted to a hidden caller arg by `ref_return`
                    // (parser/control.rs) needs a pre-allocated work-ref
                    // passed in.  Without this arm the hidden param
                    // stayed `Value::Null` → emitted as `()` natively
                    // (E0308: expected DbRef, found ()).  Mirrors the
                    // Reference arm above, keeping the struct-enum
                    // discriminator in the result type.
                    assert_eq!(
                        default,
                        Value::Null,
                        "Expect a null default on database references"
                    );
                    // #306: strip the callee-internal dep list (see Vector arm).
                    let buf_tp = Type::Enum(content, true, Deps::none());
                    let vr = self.vars.work_refs(&buf_tp, &mut self.lexer);
                    self.vars.mark_caller_hidden_buf(vr);
                    all_types[a_nr] = Type::Enum(content, true, Deps::frame1(vr));
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
                    } else if self.first_pass {
                        // Defer on pass 1 (#375): this missing-slot filler runs
                        // when an arg lowered to `Null`.  A `&` field arg whose
                        // owning struct's layout is not yet finalised (a forward /
                        // cross-package field still resolving) lowers to `Null`
                        // here, so it looks like a missing parameter.  Erroring
                        // would abort pass 1; on pass 2 the access lowers to a real
                        // `OpGetField` (non-Null), this filler is skipped, and a
                        // genuinely-missing non-text `&` default still errors then.
                        0
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
                        Type::Reference(self.data.def_nr("reference"), Deps::frame1(vr)),
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
        // #255 / @PLN9: file-level `#cwd` directive — opt this program out of the
        // program-relative default so a *relative* file path resolves against the
        // process cwd (CLI-tool semantics) rather than the program's own
        // directory.  Whole-program; must precede declarations.  At file top the
        // lexer's first token can only be this directive (`#rust`/`#native`/etc.
        // are declaration-scoped, consumed later by `parse_rust`).
        if self.lexer.has_token("#") {
            match self.lexer.has_identifier().as_deref() {
                Some("cwd") => {
                    self.database.program_relative = false;
                    let _ = self.lexer.has_token(";");
                }
                other => {
                    let name = other.unwrap_or("").to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unknown file directive '#{name}' (expected '#cwd')"
                    );
                }
            }
        }
        // Tier-0 lazy auto-`use`: the file the lexer is on right now, captured
        // before the use-loop may switch away.  Scanned for `lib::` references
        // after the use-region (see the load loop below).
        let auto_use_scan_file = self.lexer.pos().file.clone();
        // A file that writes any `use` — or the stdlib, parsed with
        // `self.default` — is in *explicit* mode: the author manages their
        // libraries by hand, so a `lib::` to an un-`use`d library is a forgotten
        // `use`, not a request to auto-load.  Skip the pre-scan for those files.
        let mut had_use = self.default;
        // Use-region fixpoint.  Pre-scan explicit `use`s, then load manifest
        // `[dependencies]`.  Loading a manifest dep `switch_to_dep`s the lexer
        // ONTO it; a MULTI-FILE dependency lands on an entry file that still has
        // its own leading `use` statements — which the pending loop, unlike this
        // pre-scan, does NOT handle.  Loop until the cursor rests on a use-free
        // file with nothing pending, so every file the main definition-loop
        // parses has had its uses processed (otherwise a dependency's legitimate
        // top `use` is misread as "use after definitions" — and never imported).
        loop {
            while self.lexer.has_token("use") {
                if let Some(id) = self.lexer.has_identifier() {
                    had_use = true;
                    // @PLN22 Phase 3 — optional library alias: `use lib as m;` → `m::fn`.
                    let lib_alias = if self.lexer.has_token("as") {
                        self.lexer.has_identifier()
                    } else {
                        None
                    };
                    // Parse optional import spec: `::*` wildcard, a single
                    // `::name [as bind]`, or the grouped `::(a [as x], b, …)`.
                    // @PLN22 Phase 4 — multiple names MUST be grouped in parentheses;
                    // the flat top-level comma list (`use lib::a, b`) is dropped (it
                    // read poorly — `b` didn't visually bind to `lib::`).
                    let spec = if self.lexer.has_token("::") {
                        if self.lexer.has_token("*") {
                            Some(ImportSpec::Wildcard)
                        } else {
                            let grouped = self.lexer.has_token("(");
                            let mut names = Vec::new();
                            while let Some(name) = self.lexer.has_identifier() {
                                // @PLN22 Phase 3 — `Name as Alias` binds the imported
                                // name under the bare alias (works inside `(…)` too).
                                let bind = if self.lexer.has_token("as") {
                                    self.lexer.has_identifier().unwrap_or_else(|| name.clone())
                                } else {
                                    name.clone()
                                };
                                names.push((name, bind));
                                if !self.lexer.has_token(",") {
                                    break;
                                }
                            }
                            if grouped {
                                self.lexer.token(")");
                            } else if names.len() > 1 {
                                // @PLN22 Phase 4 — flat comma list dropped; the names
                                // are still bound (recovery) so the rest parses cleanly.
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "import multiple names with parentheses: `use {id}::(a, b, …)`"
                                );
                            }
                            if names.is_empty() {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "Expected name, '*', or '(' after '::'"
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
                        // @PLN22 Phase 3 — register the library alias for `m::` access.
                        if let Some(alias) = &lib_alias {
                            self.data.use_alias(alias, lib_source);
                        }
                        // Plain `use foo` (no spec) wildcard-imports all pub defs.
                        // `use foo as m;` (alias, no spec) does NOT — it only provides
                        // the `m::` qualifier (the disambiguation escape hatch).  An
                        // explicit `::` spec is honoured in either case.
                        let import_spec = match spec {
                            Some(s) => Some(s),
                            None if lib_alias.is_some() => None,
                            None => Some(ImportSpec::Wildcard),
                        };
                        if let Some(import_spec) = import_spec {
                            self.pending_imports.push(PendingImport {
                                for_source: self.data.source,
                                lib_source,
                                spec: import_spec,
                            });
                        }
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
                        // @PLN22 Phase 3 — register the library alias now that the lib's
                        // source exists (use_add set self.data.source to it).  The
                        // import itself is recorded on the second encounter (via
                        // todo_files re-parse with use_exists=true).
                        if let Some(alias) = &lib_alias {
                            self.data.use_alias(alias, self.data.source);
                        }
                        drop(spec);
                        self.switch_to_dep(&f);
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
                        self.switch_to_dep(&f);
                    }
                }
            }
            // Fixpoint exit: the cursor rests on a use-free file and no manifest
            // dependency is still queued.  `peek_token` (not `has_token`) so the
            // check never consumes a leading `use` that the next iteration must see.
            if !self.lexer.peek_token("use") && self.pending_pkg_deps.is_empty() {
                break;
            }
        }
        // Tier-0 lazy auto-`use`: scan this file for `lib::` references and load
        // any that name an unloaded, available library — here in the use-region,
        // before the defs-loop, exactly like an explicit `use` (so the two-pass
        // model never sees a redefinition).  Only when the lexer is still on the
        // scanned file (the use-loop has not switched away to an explicitly-used
        // library — that file re-parses via `todo_files` and scans itself then).
        // Resolve every path first (before a `switch` changes the lexer's cwd),
        // then load with the same `todo_files` + `use_add` + `switch` shape as
        // the `pending_pkg_deps` loop.  A name that is not a real library is
        // skipped and falls through to the normal "unknown" error.
        // `!had_use`: explicit-mode files (any `use`, and the stdlib) are skipped
        // entirely — the author manages their libraries by hand there.  Read +
        // scan each remaining file at most once (cache keyed by path).
        if !had_use && self.lexer.pos().file == auto_use_scan_file {
            let (refs, calls) = if let Some(c) = self.auto_use_scan_cache.get(&auto_use_scan_file) {
                c.clone()
            } else {
                let src = Self::read_source(&auto_use_scan_file);
                let pair = (
                    crate::libscan::scan_qualified_lib_refs(&src),
                    crate::libscan::scan_method_calls(&src),
                );
                self.auto_use_scan_cache
                    .insert(auto_use_scan_file.clone(), pair.clone());
                pair
            };
            // Tier-0: `lib::x` — the library is named directly.
            let mut to_load: Vec<String> = Vec::new();
            for name in refs {
                if self.data.use_exists(&name) || self.data.def_nr(&name) != u32::MAX {
                    continue;
                }
                if !to_load.contains(&name) {
                    to_load.push(name);
                }
            }
            // Tier-1: `obj.method(…)` — map the method to its providing package
            // via the trigger surface of the current package (+ trigger-enabled
            // deps), derived once and cached.
            if !calls.is_empty() {
                let map = self.trigger_map(&auto_use_scan_file);
                // Catalog fallback is built lazily — only read index.json once a
                // method misses the local (current package + deps) trigger map.
                let mut catalog: Option<std::collections::HashMap<String, String>> = None;
                for m in calls {
                    let pkg = if let Some(p) = map.get(&m) {
                        Some(p.clone())
                    } else {
                        if catalog.is_none() {
                            catalog = Some(self.catalog_trigger_map());
                        }
                        catalog.as_ref().and_then(|c| c.get(&m).cloned())
                    };
                    if let Some(pkg) = pkg
                        && !self.data.use_exists(&pkg)
                        && !to_load.contains(&pkg)
                    {
                        to_load.push(pkg);
                    }
                }
            }
            // Resolve every path first (before a `switch` changes the cwd), then
            // load with the proven todo_files + use_add + switch shape.
            let mut resolved: Vec<(String, String)> = Vec::new();
            for n in to_load {
                if self.data.use_exists(&n) {
                    continue;
                }
                let f = self.lib_path(&n);
                if std::path::Path::new(&f).exists() {
                    resolved.push((n, f));
                }
            }
            for (name, f) in resolved {
                if self.data.use_exists(&name) {
                    continue;
                }
                let cur = self.lexer.pos().file.clone();
                self.todo_files.push((cur, self.data.source));
                self.data.use_add(&name);
                self.switch_to_dep(&f);
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
                    for (name, bind) in &names {
                        if !self.data.import_name(pi.lib_source, cur, name, bind) {
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

    /// Read a source file's content for the Tier-0 auto-`use` pre-scan.
    /// Honours the wasm VirtFS; an empty string on a read error (the scan then
    /// finds nothing and normal resolution proceeds unchanged).
    fn read_source(filename: &str) -> String {
        #[cfg(feature = "wasm")]
        if let Some(c) = crate::wasm::virt_fs_get(filename) {
            return c;
        }
        std::fs::read_to_string(filename).unwrap_or_default()
    }

    /// Tier-1: build (once, cached) the `method name -> providing package` map
    /// from the current package's declared triggers — `[triggers] enabled` plus
    /// the text-methods derived from its source — and those of its
    /// trigger-enabled dependencies.  The package is located by walking up from
    /// `from_file` to the nearest `loft.toml`.
    fn trigger_map(&mut self, from_file: &str) -> std::collections::HashMap<String, String> {
        if let Some(m) = &self.auto_use_trigger_map {
            return m.clone();
        }
        let mut map = std::collections::HashMap::new();
        let mut dir = std::path::Path::new(from_file)
            .parent()
            .map(std::path::Path::to_path_buf);
        while let Some(d) = dir {
            let toml = d.join("loft.toml");
            if toml.exists() {
                Self::add_pkg_triggers(&toml, &d, &mut map);
                if let Some(man) = crate::manifest::read_manifest(&toml.to_string_lossy()) {
                    for (dep, _ver) in &man.dependencies {
                        if let Some(entry) = self.lib_path_manifest(&d.to_string_lossy(), dep)
                            && let Some(root) = std::path::Path::new(&entry)
                                .parent()
                                .and_then(|p| p.parent())
                        {
                            Self::add_pkg_triggers(&root.join("loft.toml"), root, &mut map);
                        }
                    }
                }
                break;
            }
            dir = d.parent().map(std::path::Path::to_path_buf);
        }
        self.auto_use_trigger_map = Some(map.clone());
        map
    }

    /// Add a package's derived text-method triggers (`method -> package`) to
    /// `map`, when the package opts in via `[triggers] enabled`.
    fn add_pkg_triggers(
        toml: &std::path::Path,
        pkg_root: &std::path::Path,
        map: &mut std::collections::HashMap<String, String>,
    ) {
        let Some(man) = crate::manifest::read_manifest(&toml.to_string_lossy()) else {
            return;
        };
        if !man.trigger_enabled {
            return;
        }
        let Some(name) = man.name else { return };
        let entry = man.entry.unwrap_or_else(|| format!("src/{name}.loft"));
        let src = std::fs::read_to_string(pkg_root.join(&entry)).unwrap_or_default();
        for mt in crate::triggers::derive_triggers(&src).methods {
            map.entry(mt.name).or_insert_with(|| name.clone());
        }
    }

    /// Tier-1 lazy *catalog* fallback (`method -> package`), read once from the
    /// cached registry `index.json`.  This is what makes `line.matches(p)` work
    /// against a package the user has NOT declared as a dependency: the local
    /// `trigger_map` misses, so we look the method up across the whole catalog,
    /// get the package name, and hand it to `lib_path` — which resolves it via
    /// the same lockfile → installed → auto-install chain an explicit `use`
    /// would take.  Ambiguity is dropped by `trigger_providers` (registry
    /// submission already rejects true collisions).  Cached (empty on absent
    /// catalog) so the file read happens at most once per parse.
    #[cfg(feature = "registry")]
    fn catalog_trigger_map(&mut self) -> std::collections::HashMap<String, String> {
        if let Some(m) = &self.auto_use_catalog_map {
            return m.clone();
        }
        let mut map = std::collections::HashMap::new();
        let (idx_path, _, _) = crate::registry_index::index_paths();
        if let Ok(content) = std::fs::read_to_string(&idx_path)
            && let Ok(index) = crate::registry_index::parse_index(&content)
        {
            map = crate::registry_index::trigger_providers(&index)
                .into_iter()
                .collect();
        }
        self.auto_use_catalog_map = Some(map.clone());
        map
    }

    /// No-op when the registry feature is off — there is no cached catalog to
    /// consult, so Tier-1 resolves only via declared dependencies.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn catalog_trigger_map(&mut self) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
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
        // Dep-shadowing guard: a name declared under [dependencies] in the
        // current package's manifest must resolve as a dependency — never to
        // a same-named `.loft` file inside the declaring package.  Without
        // this, `use server` in a package that also contains `server.loft`
        // loads the package file (the package root sits in `lib_dirs` for
        // intra-package `use`), silently shadowing the library: its types
        // vanish and every consumer in the package breaks.
        let shadow_root = self
            .package_declared_deps(cur_dir)
            .filter(|(_, deps)| deps.contains(id))
            .map(|(root, _)| root);
        let blocked = |candidate: &str| {
            shadow_root.as_deref().is_some_and(|root| {
                // Canonicalize so relative candidates (cwd inside the
                // package) and the canonical root compare in one space.
                let cand = std::path::Path::new(candidate);
                cand.canonicalize()
                    .unwrap_or_else(|_| cand.to_path_buf())
                    .starts_with(root)
            })
        };

        let mut f = Self::probe_project_lib(id);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        Self::probe_dir_lib(id, cur_dir, &mut f);
        Self::probe_dir_lib(id, base_dir, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_manifest_path_dep(id, cur_dir, &mut f);
        self.probe_sibling_package(id, cur_dir, &mut f);
        Self::probe_script_sibling_dir(id, &cur_script, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_cmdline_lib_dirs(id, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_cmdline_lib_dirs_manifest(id, &mut f);
        Self::probe_loft_lib_flat(id, &mut f);
        self.probe_loft_lib_manifest(id, &mut f);
        self.probe_user_installed(id, &mut f);
        self.probe_sidecar_lockfile(id, &mut f);
        self.probe_project_lockfile(id, &mut f);
        self.probe_registry_installed(id, &mut f);
        self.probe_auto_install(id, &mut f);
        Self::probe_cur_dir_flat(id, cur_dir, &mut f);
        Self::probe_base_dir_flat(id, base_dir, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        f
    }

    /// The package context owning `cur_dir`: the nearest ancestor directory
    /// holding a `loft.toml`, plus that manifest's declared dependency names.
    /// Cached per directory (manifests don't change mid-parse).
    fn package_declared_deps(
        &mut self,
        cur_dir: &str,
    ) -> Option<(String, std::collections::HashSet<String>)> {
        if let Some(cached) = self.pkg_dep_cache.get(cur_dir) {
            return cached.clone();
        }
        let mut found = None;
        let start = if cur_dir.is_empty() { "." } else { cur_dir };
        let mut search = std::path::Path::new(start).canonicalize().ok();
        while let Some(dir) = search {
            let manifest_path = dir.join("loft.toml");
            if manifest_path.exists() {
                if let Some(manifest) =
                    crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                {
                    let deps: std::collections::HashSet<String> = manifest
                        .dependencies
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect();
                    found = Some((dir.to_string_lossy().to_string(), deps));
                }
                break;
            }
            search = dir.parent().map(std::path::Path::to_path_buf);
        }
        self.pkg_dep_cache
            .insert(cur_dir.to_string(), found.clone());
        found
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

    /// `<dir>/lib/<id>.loft` — a `lib/` directory relative to `dir`
    /// (called for the script's own dir, then for the base dir when the
    /// script lives inside a `/tests/` tree).
    fn probe_dir_lib(id: &str, dir: &str, f: &mut String) {
        if !dir.is_empty() && !std::path::Path::new(f).exists() {
            *f = format!("{dir}{0}lib{0}{id}.loft", sep_str());
        }
    }

    /// #337 / PACKAGES.md resolution step 2 — `[dependencies] <id> =
    /// { path = "…" }` in the consuming package's `loft.toml`.  Walk up
    /// from `cur_dir` to the owning manifest; if it declares `id` as a
    /// path dependency, resolve the path relative to the manifest's
    /// directory and locate the dep package's entry file (its own
    /// `[library] entry`, defaulting to `src/<id>.loft`).  Registers the
    /// dep's manifest so its `#native` symbols resolve, mirroring
    /// `probe_sibling_package`.
    fn probe_manifest_path_dep(&mut self, id: &str, cur_dir: &str, f: &mut String) {
        if std::path::Path::new(f).exists() || cur_dir.is_empty() {
            return;
        }
        let mut search_dir = std::path::Path::new(cur_dir).to_path_buf();
        loop {
            let manifest_path = search_dir.join("loft.toml");
            if manifest_path.exists() {
                let dep_rel = crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                    .and_then(|m| {
                        m.dependencies.iter().find_map(|(name, value)| {
                            (name == id)
                                .then(|| crate::manifest::extract_path_dep(value))
                                .flatten()
                                .map(str::to_string)
                        })
                    });
                if let Some(rel) = dep_rel {
                    let pkg_root = search_dir.join(rel);
                    let dep_manifest = pkg_root.join("loft.toml");
                    let entry = dep_manifest
                        .exists()
                        .then(|| crate::manifest::read_manifest(&dep_manifest.to_string_lossy()))
                        .flatten()
                        .and_then(|m| m.entry)
                        .unwrap_or_else(|| format!("src{}{id}.loft", sep_str()));
                    let file = pkg_root.join(entry);
                    if file.exists() {
                        *f = file.to_string_lossy().to_string();
                        if dep_manifest.exists() {
                            self.register_native_manifest(&dep_manifest, &pkg_root);
                        }
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

    /// @PLAN12 Phase 6.7 — per-invocation advisory classifier.
    ///
    /// Called by each lockfile-based probe (sidecar / project /
    /// registry-installed) after resolving a `(name, version)`
    /// tuple.  Loads the cached advisory feed (lazily, once per
    /// process), classifies the tuple, and emits severity-tiered
    /// output:
    ///
    /// - `security_critical` → loud `error:` block; **continue
    ///   running** (the user may be running their code precisely to
    ///   test the upgrade, and security fixes can introduce
    ///   breaking changes; refusing here is worse than warning).
    ///   Opt-in refusal: `LOFT_STRICT_SECURITY=1` (env var) OR
    ///   `--strict-security` (CLI flag, intended for CI gates) — in
    ///   either, critical aborts with `process::exit(3)` UNLESS
    ///   `LOFT_SECURITY_OVERRIDE=<advisory-id>` is set (comma-
    ///   separated for multiple).
    /// - `security_high` → loud `warning:` block; continue.
    /// - `security_low` / `bug` → one-line warning.
    /// - `deprecated` → one-line note.
    ///
    /// The Cargo / npm precedent: `cargo audit` defaults to warn,
    /// `--deny warnings` is opt-in for CI.  Pure refusal blocks the
    /// user precisely when they're trying to ship a fix or assess
    /// the upgrade — the wrong default.
    ///
    /// Dedupes via `self.advisory_checked` so a package surfaced by
    /// multiple `use` paths (sidecar + auto-install re-probe) only
    /// reports once.  Feed loading is best-effort offline + cache-
    /// only: no network fetch fires during script execution, only
    /// during explicit `loft audit` / install commands.
    #[cfg(feature = "registry")]
    fn check_advisory(&mut self, name: &str, version: &str) {
        let key = (name.to_string(), version.to_string());
        if !self.advisory_checked.insert(key) {
            return;
        }
        let Some(feed) = Self::advisory_feed() else {
            return;
        };
        let hits = crate::registry_advisories::classify(name, version, feed);
        if hits.is_empty() {
            return;
        }
        let strict = std::env::var("LOFT_STRICT_SECURITY").is_ok();
        let overrides: std::collections::HashSet<String> = std::env::var("LOFT_SECURITY_OVERRIDE")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        for hit in &hits {
            use crate::registry_advisories::Severity;
            let overridden = overrides.contains(&hit.advisory_id);
            match hit.severity {
                Severity::SecurityCritical => {
                    // Loud error block — same in strict and non-strict
                    // mode.  The DIFFERENCE is only whether we then
                    // refuse to proceed.
                    eprintln!(
                        "error: {} {} was yanked for a security vulnerability",
                        hit.package, hit.version
                    );
                    eprintln!("  advisory: {}", hit.advisory_id);
                    eprintln!("  summary:  {}", hit.summary);
                    if let Some(fix) = &hit.fixed_in {
                        eprintln!(
                            "  fix:      {} >= {} (run `loft install {}@{}`)",
                            hit.package, fix, hit.package, fix
                        );
                    }
                    if strict && !overridden {
                        eprintln!("  refused under LOFT_STRICT_SECURITY=1");
                        eprintln!(
                            "  override (audit-trail required): LOFT_SECURITY_OVERRIDE={}",
                            hit.advisory_id
                        );
                        std::process::exit(3);
                    }
                    if strict && overridden {
                        eprintln!(
                            "[security] override applied: {} ({} {})",
                            hit.advisory_id, hit.package, hit.version
                        );
                    }
                    // Non-strict mode: error block printed, but we
                    // proceed.  User retains the right to run their
                    // code while they investigate / fix.
                }
                Severity::SecurityHigh => {
                    eprintln!(
                        "warning: {} {} has a known security issue",
                        hit.package, hit.version
                    );
                    eprintln!("  advisory: {}", hit.advisory_id);
                    eprintln!("  summary:  {}", hit.summary);
                    if let Some(fix) = &hit.fixed_in {
                        eprintln!("  fix:      {} >= {}", hit.package, fix);
                    }
                }
                Severity::SecurityLow | Severity::Bug => {
                    eprintln!(
                        "warning: {} {} — {} (advisory {})",
                        hit.package, hit.version, hit.summary, hit.advisory_id
                    );
                }
                Severity::Deprecated => {
                    eprintln!(
                        "note: {} {} is deprecated — {} (advisory {})",
                        hit.package, hit.version, hit.summary, hit.advisory_id
                    );
                }
            }
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self, dead_code)]
    fn check_advisory(&mut self, _name: &str, _version: &str) {}

    /// Lazy process-global advisory feed loader.  Cached for the
    /// duration of the process; offline-respecting; never refetches
    /// from network mid-parse (the explicit `loft audit` flow does
    /// that).  Returns `None` when the registry doesn't yet host an
    /// advisory feed (current real state — HTTP 404 soft-fails to
    /// "no advisories").
    #[cfg(feature = "registry")]
    fn advisory_feed() -> Option<&'static crate::registry_advisories::AdvisoryFeed> {
        use std::sync::OnceLock;
        static FEED: OnceLock<Option<crate::registry_advisories::AdvisoryFeed>> = OnceLock::new();
        FEED.get_or_init(|| {
            let opts = crate::registry_advisories::LoadOptions {
                allow_unsigned: true,
                // Mid-parse, never hit the network — always cache-only.
                // `loft audit` does the freshness refresh on demand.
                offline: true,
                refresh: false,
            };
            crate::registry_advisories::load_or_fetch(&opts).unwrap_or(None)
        })
        .as_ref()
    }

    /// @PLAN12 Phase 6.6 — walk-up project root detection.
    ///
    /// From the script's directory, walks up to `/` looking for a
    /// `loft.toml`.  Returns the dir containing it (the project
    /// root) — used by:
    /// - `probe_project_lockfile` to find the lockfile that pins
    ///   manifest-declared deps.
    /// - `probe_auto_install` to redirect the lockfile WRITE path
    ///   to the project root, so auto-installs in a project
    ///   context update the project's lockfile rather than cwd's.
    ///
    /// Returns `None` for script-mode invocations (no `loft.toml`
    /// anywhere in the parent chain).  Script mode falls back to
    /// cwd's `loft.lock` (existing behaviour) or to the sidecar
    /// (when `loft pin <script>` has been run).
    #[cfg(feature = "registry")]
    fn find_project_root(script_path: &str) -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(script_path);
        if script_path.is_empty() {
            return None;
        }
        let start_dir = if p.is_dir() {
            p.to_path_buf()
        } else if let Some(parent) = p.parent() {
            if parent.as_os_str().is_empty() {
                std::env::current_dir().ok()?
            } else {
                parent.to_path_buf()
            }
        } else {
            return None;
        };
        // Canonicalize so the walk-up doesn't terminate prematurely
        // on relative `./` prefixes that loop on themselves.
        let abs_start = std::fs::canonicalize(&start_dir).unwrap_or(start_dir);
        let mut cur = abs_start.as_path();
        loop {
            if cur.join("loft.toml").exists() {
                return Some(cur.to_path_buf());
            }
            let parent = cur.parent()?;
            if parent == cur {
                return None;
            }
            cur = parent;
        }
    }

    /// @PLAN12 Phase 6.6 — project-mode lockfile resolution.
    ///
    /// Walks up from the script's directory looking for `loft.toml`;
    /// if found, reads the adjacent `loft.lock` and resolves `id`
    /// against it.  Means a script at `myproject/src/foo.loft`
    /// resolves registry libraries via `myproject/loft.lock` no
    /// matter where `loft` is invoked from (cwd-independent).
    ///
    /// Cargo-style: the project root owns the lockfile; cwd is
    /// irrelevant.  Inserted in the probe chain BEFORE
    /// `probe_registry_installed` (which uses cwd as a script-mode
    /// fallback when no project is found).
    #[cfg(feature = "registry")]
    fn probe_project_lockfile(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let Some(project_root) = Self::find_project_root(&cur_script) else {
            return;
        };
        let lock_path = project_root.join("loft.lock");
        if !lock_path.exists() {
            return;
        }
        let lock = match crate::lockfile::read_lockfile(&lock_path) {
            Ok(Some(l)) => l,
            _ => return,
        };
        let version = match lock.packages.iter().find(|p| p.name == id) {
            Some(p) => p.version.clone(),
            None => return,
        };
        let install_dir = crate::registry_index::extract_dir(id, &version);
        let parent = match install_dir.parent().and_then(std::path::Path::to_str) {
            Some(p) => p.to_string(),
            None => return,
        };
        let versioned_name: String = match install_dir.file_name().and_then(std::ffi::OsStr::to_str)
        {
            Some(n) => n.to_string(),
            None => return,
        };
        if let Some(entry) = self.lib_path_manifest(&parent, &versioned_name) {
            self.check_advisory(id, &version);
            *f = entry;
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn probe_project_lockfile(&mut self, _id: &str, _f: &mut String) {}

    /// @PLAN12 Phase 6.6 — sidecar lockfile next to the script.
    ///
    /// `<script>.loft.lock` (e.g. `hello.loft.lock` next to `hello.loft`)
    /// pins the registry versions a one-file script uses.  Generated by
    /// `loft pin <script>`; takes precedence over the cwd `loft.lock`
    /// because the sidecar belongs TO the script, while cwd's lockfile
    /// belongs to wherever the user happens to be invoking from.
    ///
    /// Without the sidecar, single-file scripts inherit cwd's lockfile
    /// (or auto-install latest active).  With it, the script is
    /// reproducible regardless of cwd or registry-state drift.
    #[cfg(feature = "registry")]
    fn probe_sidecar_lockfile(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        if cur_script.is_empty() {
            return;
        }
        let sidecar = format!("{cur_script}.lock");
        if !std::path::Path::new(&sidecar).exists() {
            return;
        }
        let lock = match crate::lockfile::read_lockfile(std::path::Path::new(&sidecar)) {
            Ok(Some(l)) => l,
            _ => return,
        };
        let version = match lock.packages.iter().find(|p| p.name == id) {
            Some(p) => p.version.clone(),
            None => return,
        };
        let install_dir = crate::registry_index::extract_dir(id, &version);
        let parent = match install_dir.parent().and_then(std::path::Path::to_str) {
            Some(p) => p.to_string(),
            None => return,
        };
        let versioned_name: String = match install_dir.file_name().and_then(std::ffi::OsStr::to_str)
        {
            Some(n) => n.to_string(),
            None => return,
        };
        if let Some(entry) = self.lib_path_manifest(&parent, &versioned_name) {
            self.check_advisory(id, &version);
            *f = entry;
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn probe_sidecar_lockfile(&mut self, _id: &str, _f: &mut String) {}

    /// `~/.loft/registry/<id>-<version>/` — packages installed via `loft install`
    /// against the package registry.  Resolves the version via the cwd's
    /// `loft.lock` (written by `loft install`).  When loft.lock is absent or
    /// doesn't list `id`, this probe is a no-op and resolution falls through
    /// to the remaining strategies.
    ///
    /// @PLAN12 phase 3.5a wiring (2026-05-24): closes the "loft install →
    /// use installed package" loop.  Before this, `loft install crypto`
    /// downloaded + extracted correctly but `use crypto;` in a subsequent
    /// run still required a manual `--lib` flag.
    #[cfg(feature = "registry")]
    fn probe_registry_installed(&mut self, id: &str, f: &mut String) {
        // Use `loft::*` (not `crate::*`) because this module is compiled
        // into BOTH the loft library AND the loft binary; the binary
        // doesn't have `lockfile` / `registry_index` declared as `mod`,
        // but accesses them as deps via the `loft::` library path.
        if std::path::Path::new(f).exists() {
            return;
        }
        let cwd = match env::current_dir() {
            Ok(c) => c,
            Err(_) => return,
        };
        let lock_path = cwd.join("loft.lock");
        if !lock_path.exists() {
            return;
        }
        let lock = match crate::lockfile::read_lockfile(&lock_path) {
            Ok(Some(l)) => l,
            _ => return,
        };
        let version = match lock.packages.iter().find(|p| p.name == id) {
            Some(p) => p.version.clone(),
            None => return,
        };
        let install_dir = crate::registry_index::extract_dir(id, &version);
        let parent = match install_dir.parent().and_then(std::path::Path::to_str) {
            Some(p) => p.to_string(),
            None => return,
        };
        let versioned_name: String = match install_dir.file_name().and_then(std::ffi::OsStr::to_str)
        {
            Some(n) => n.to_string(),
            None => return,
        };
        if let Some(entry) = self.lib_path_manifest(&parent, &versioned_name) {
            self.check_advisory(id, &version);
            *f = entry;
        }
    }

    /// No-op when registry feature is off — registry-installed packages
    /// only resolve when the `loft install` machinery is compiled in.
    /// `self` is kept to match the `#[cfg(feature = "registry")]` method
    /// signature (it is a genuine method on the registry-enabled build).
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn probe_registry_installed(&mut self, _id: &str, _f: &mut String) {}

    /// @PLAN12 Phase 6.6 — auto-install on `use`.
    ///
    /// When `id` doesn't resolve via any of the prior strategies
    /// (path-dep, sibling lookup, lockfile + cached registry install)
    /// AND `id` is a known package name in the registry catalog,
    /// fire `install_one` to fetch + extract + lockfile-update,
    /// then re-run the cached-registry-install probe.
    ///
    /// The Python comparison: `python my_script.py` with
    /// `import requests` works if `pip install requests` was done
    /// once.  Loft's equivalent — `loft my_script.loft` with
    /// `use gridmesh;` — Just Works on first run by doing the
    /// `pip install` step on the user's behalf.
    ///
    /// Off-switches: `LOFT_OFFLINE=1` and `LOFT_NO_AUTO_INSTALL=1`
    /// both suppress this probe.  Surprise reduction: every cold
    /// install prints `[registry] ...` lines (mirrors Cargo's
    /// "Downloading…" output); steady-state (cache hit, resolves
    /// via probe_registry_installed) is silent.
    #[cfg(feature = "registry")]
    fn probe_auto_install(&mut self, id: &str, f: &mut String) {
        if std::path::Path::new(f).exists() {
            return;
        }
        // Off-switches.
        if std::env::var("LOFT_OFFLINE").is_ok() {
            return;
        }
        if std::env::var("LOFT_NO_AUTO_INSTALL").is_ok() {
            return;
        }
        // Bootstrap state: the loft binary may not have an embedded
        // trust root yet (K_tmp → K_real rotation per
        // PKG_REGISTRY.md / REGISTRY_BOOTSTRAP.md).  Mirror what
        // `loft install` / `loft search` / `loft info` do — accept
        // unsigned indexes during the trust-bootstrap window.  Once
        // the production key is embedded, signed indexes verify
        // cleanly and this flag becomes a no-op for the happy path.
        //
        // Lockfile WRITE path: walk up from the script's directory
        // looking for `loft.toml`.  Found → project mode — write to
        // `<project_root>/loft.lock` so the project's manifest +
        // lockfile stay co-located regardless of where loft was
        // invoked from.  Not found → script mode — `lock_path: None`
        // falls back to cwd's `loft.lock` (the existing default).
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let project_root = Self::find_project_root(&cur_script);
        let lock_path = project_root.as_ref().map(|p| p.join("loft.lock"));
        let opts = crate::install::InstallOptions {
            allow_unsigned: true,
            refresh: false,
            // LOFT_OFFLINE=1 makes resolution HERMETIC: a missing package
            // fails fast and deterministically instead of fetching — what a
            // test-spawned fixture (or an air-gapped box) wants.  Mirrors
            // the CLI paths (src/main.rs) that already honour it.
            offline: std::env::var_os("LOFT_OFFLINE").is_some(),
            allow_prerelease: false,
            lock_path,
        };
        match crate::install::auto_install_if_in_catalog(id, &opts) {
            Ok(Some(_report)) => {
                // Install succeeded; re-probe via lockfile-based
                // resolution to populate `f`.  Try project lockfile
                // first (where we just wrote the new entry); if
                // that's not active, fall back to cwd's lockfile
                // (script-mode case where lock_path was None).
                self.probe_project_lockfile(id, f);
                self.probe_registry_installed(id, f);
            }
            Ok(None) => {
                // `id` is not a registry package; let the remaining
                // resolution strategies handle it (or fail with
                // the standard "library not found" diagnostic).
            }
            Err(e) => {
                // Network failure, sig mismatch, or similar.
                // Print a notice but let resolution fall through —
                // the user may have a path-dep or sibling that
                // resolves anyway, or they may want the standard
                // error.
                eprintln!("[registry] auto-install failed for {id}: {e}");
            }
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    fn probe_auto_install(&mut self, _id: &str, _f: &mut String) {}

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
        // Register the dlopen-side native lib path (interpreter mode).  The
        // `[library] native = "..."` form registers the cdylib for dlopen;
        // the separate `[native] crate = "..."` block (handled below)
        // registers the rlib for the native-compile path.  Both must run
        // here for sibling-package and ancestor-walk paths, otherwise
        // packages depended on by a no-native parent (e.g. an examples
        // package that uses `lib/server`) lose their native bindings in
        // interpreter mode.
        if let Some(ref stem) = m.native
            && let Some(path) = crate::extensions::resolve_native_lib(&pkg_dir, stem)
            && !self.pending_native_libs.contains(&path)
        {
            self.pending_native_libs.push(path);
            self.native_lib_regs.push((stem.clone(), pkg_dir.clone()));
        }
        // @PLN11 N3 Step 3 (default-native) / F2 — mirror `apply_manifest_side_effects`:
        // a normal loft library reached via THIS direct-resolution / sibling-package /
        // ancestor-walk path must ALSO be recorded as a native-compile candidate.
        // Without it the two resolution paths diverge: a library pulled in transitively
        // (or used directly *after* it was already loaded transitively, so the direct
        // `use` dedups) is loaded here but never recorded — so it never builds its own
        // cdylib and its direct calls interpret.  `native` (hand-written cdylib) takes
        // precedence — don't double-compile.
        if m.native.is_none() && !self.pending_native_compile.iter().any(|d| d == &pkg_dir) {
            self.pending_native_compile.push(pkg_dir.clone());
        }
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
                    .push((crate_name.clone(), pkg_dir.clone()));
            }
            // @PLAN12 phase 2 step 2 (2026-05-24) — same manifest-driven
            // `native_symbols` + `def.native` population as
            // `apply_manifest_side_effects` (the legacy path).  The
            // sibling-package probe (used when a script reaches a library
            // via cwd ancestry rather than `--lib`) hits THIS function;
            // without the same population, `[native.functions]` entries
            // would be visible only to native codegen via the legacy
            // path's `native_symbols` lookup — the sibling path would
            // leave def.native empty and the interpreter dispatch path
            // would silently fall through to OpCall with an empty body,
            // returning null instead of dispatching to the cdylib.
            for (loft_name, rust_symbol) in &m.native_functions {
                self.data
                    .native_symbols
                    .insert(loft_name.clone(), rust_symbol.clone());
            }
            for (loft_name, rust_symbol) in &m.native_functions {
                let candidates = [format!("n_{loft_name}"), loft_name.clone()];
                for d_nr in 0..self.data.definitions() {
                    let def = self.data.def(d_nr);
                    if !def.native().is_empty() {
                        continue;
                    }
                    if !candidates.iter().any(|c| c == def.name()) {
                        continue;
                    }
                    if !def.position().file.starts_with(&pkg_dir) {
                        continue;
                    }
                    rust_symbol.clone_into(&mut self.data.definitions[d_nr as usize].native);
                }
            }
            // lib_plan-29 W1c — same population in the sibling-probe path
            // as in apply_manifest_side_effects (see comments below for
            // why both paths need this).
            if let Some(ref bridge_crate) = m.wasm_bridge_crate {
                if !self
                    .data
                    .wasm_bridge_packages
                    .iter()
                    .any(|(c, _)| c == bridge_crate)
                {
                    self.data
                        .wasm_bridge_packages
                        .push((bridge_crate.clone(), pkg_dir.clone()));
                }
                for (loft_sym, bridge_fn) in &m.wasm_bridge_routes {
                    self.data
                        .wasm_bridge_routes
                        .insert(loft_sym.clone(), (bridge_crate.clone(), bridge_fn.clone()));
                }
            }
            // lib_plan-29 W2 — host_js mirror.
            if let Some(ref host_js_rel) = m.wasm_bridge_host_js {
                let abs = std::path::Path::new(&pkg_dir).join(host_js_rel);
                let abs_str = abs.to_string_lossy().to_string();
                if !self.data.wasm_bridge_host_js_files.contains(&abs_str) {
                    self.data.wasm_bridge_host_js_files.push(abs_str);
                }
            }
            // P266: same ownership-driven restriction as
            // `apply_manifest_side_effects` above — only map `#native`
            // symbols whose definition lives in THIS package's source
            // tree, so out-of-order manifest/source loading can't make
            // one package claim another's symbols.
            for d_nr in 0..self.data.definitions() {
                let def = self.data.def(d_nr);
                let sym = def.native();
                if sym.is_empty() {
                    continue;
                }
                if !def.position().file.starts_with(&pkg_dir) {
                    continue;
                }
                if self.data.native_symbol_crates.contains_key(sym) {
                    continue;
                }
                self.data
                    .native_symbol_crates
                    .insert(sym.to_string(), rust_crate.clone());
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
        // @P296-sibling (Windows) — build the package paths with
        // `Path::join` rather than `format!("{dir}/{id}")`.  When `dir` is
        // an absolute Windows path (`D:\a\loft\loft\lib`, e.g. a `--lib`
        // argument built via `PathBuf::join`), interpolating a literal `/`
        // produces a mixed-separator path; `Path::join` uses the platform
        // separator consistently so nested `--lib` packages resolve on
        // Windows (the crystal_gold CI failure: "Library 'audience_crystal'
        // not found").  No behavioural change on Linux/macOS.
        // #408 — accept `--lib` pointing AT the package dir itself (its basename
        // matches `id` and it carries a loft.toml), not only at its parent.
        // Without this, `--lib path/to/crypto` looks for `path/to/crypto/crypto`,
        // fails, and `use crypto` falls through to an installed registry copy —
        // the shadowing that made a local lib's `[wasm.bridge]` routes unreachable
        // in `--html`. Falls back to `<dir>/<id>` for the normal parent-dir search.
        let dir_pb = std::path::Path::new(dir);
        let pkg_dir_pb = if dir_pb.file_name() == Some(std::ffi::OsStr::new(id))
            && dir_pb.join("loft.toml").is_file()
        {
            dir_pb.to_path_buf()
        } else {
            dir_pb.join(id)
        };
        if !pkg_dir_pb.is_dir() {
            return None;
        }
        let pkg_dir = pkg_dir_pb.to_string_lossy().into_owned();
        let manifest_pb = pkg_dir_pb.join("loft.toml");
        let nested_entry = || {
            pkg_dir_pb
                .join("src")
                .join(format!("{id}.loft"))
                .to_string_lossy()
                .into_owned()
        };
        let (entry, manifest) = if manifest_pb.exists() {
            let manifest_path = manifest_pb.to_string_lossy().into_owned();
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
            let entry = m.entry.as_ref().map_or_else(nested_entry, |e| {
                pkg_dir_pb.join(e).to_string_lossy().into_owned()
            });
            (entry, Some(m))
        } else {
            (nested_entry(), None)
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
        // Pre-built location first, then auto-build from source (one home:
        // `extensions::resolve_native_lib`, shared with the warm-cache load).
        if let Some(ref stem) = m.native
            && let Some(path) = crate::extensions::resolve_native_lib(pkg_dir, stem)
            && !self.pending_native_libs.contains(&path)
        {
            self.pending_native_libs.push(path);
            self.native_lib_regs
                .push((stem.clone(), pkg_dir.to_string()));
        }
        // @PLN11 Arc N / N3 Step 3 — **default-native**.  Every `use`d normal loft
        // library is a native candidate: record the package dir; the driver marks +
        // builds + loads after scope analysis (see `pending_native_compile`).  No
        // opt-in, no flag — "libraries compile, scripts interpret" is the default.
        //   - `native` (a hand-written cdylib via `[library] native = ...`) takes
        //     precedence — don't double-compile.
        //   - `LOFT_NO_NATIVE_LIBS=1` is the interpret escape (handled in the driver:
        //     it clears `pending_native_compile`), used by the dev/edit loop + the
        //     parity reference until Step 4 (dev-interpret-on-edit) lands.
        //   - A build failure silently interprets (Step 2), so recording a library
        //     that can't compile native is harmless.
        // The legacy `[library] compile = "native"` opt-in is now redundant but still
        // accepted (a no-op): default-native already records it.
        if m.native.is_none() && !self.pending_native_compile.iter().any(|d| d == pkg_dir) {
            self.pending_native_compile.push(pkg_dir.to_string());
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
            // @PLAN12 phase 2 step 2 (2026-05-24) — make
            // `[native.functions]` entries fully equivalent to
            // `#native "symbol"` annotations.  For each manifest
            // entry whose loft fn name matches a definition owned by
            // this package, populate `def.native` so the interpreter's
            // `wire_native_fns` (which keys off `def.native`) and the
            // bytecode dispatch (`state/codegen.rs:2207-2217`, which
            // also reads `def.native`) see the binding without
            // requiring the redundant `#native` annotation.  Native
            // codegen already consulted `native_symbols` directly so
            // this isn't strictly required for the native path, but
            // unifying the two paths is the whole point of step 2.
            //
            // Ownership check (`def.position.file.starts_with(pkg_dir)`)
            // mirrors the @P266-style guard below: only populate defs
            // physically inside this package's source tree.
            for (loft_name, rust_symbol) in &m.native_functions {
                let candidates = [format!("n_{loft_name}"), loft_name.clone()];
                for d_nr in 0..self.data.definitions() {
                    let def = self.data.def(d_nr);
                    if !def.native().is_empty() {
                        continue;
                    }
                    if !candidates.iter().any(|c| c == def.name()) {
                        continue;
                    }
                    if !def.position().file.starts_with(pkg_dir) {
                        continue;
                    }
                    rust_symbol.clone_into(&mut self.data.definitions[d_nr as usize].native);
                }
            }
            // P266: map only `#native` symbols whose definition lives in
            // THIS package's source tree, not every unmapped symbol in
            // the whole `data.definitions()` list.  The earlier
            // walk-and-claim shape over-assigned: when manifests were
            // registered out-of-order with their sources (e.g. lib/web
            // manifest registered before lib/server's source was parsed,
            // then lib/server's manifest registered with both packages'
            // defs already in the table), it left lib/web's symbols
            // pointing at `loft_server` and lib/server's symbols pointing
            // at `loft_web`.  Restricting by `position.file.starts_with(
            // pkg_dir)` makes the assignment ownership-driven instead of
            // call-order-driven.
            for d_nr in 0..self.data.definitions() {
                let def = self.data.def(d_nr);
                let sym = def.native();
                if sym.is_empty() {
                    continue;
                }
                if !def.position().file.starts_with(pkg_dir) {
                    continue;
                }
                if self.data.native_symbol_crates.contains_key(sym) {
                    continue;
                }
                self.data
                    .native_symbol_crates
                    .insert(sym.to_string(), rust_crate.clone());
            }
        }
        // lib_plan-29 W1c (2026-05-29) — register WASM bridge crate +
        // routes for `--html` builds.  Mirrors the `[native]` block
        // above, scoped to the browser-WASM target.
        if let Some(ref bridge_crate) = m.wasm_bridge_crate {
            if !self
                .data
                .wasm_bridge_packages
                .iter()
                .any(|(c, _)| c == bridge_crate)
            {
                self.data
                    .wasm_bridge_packages
                    .push((bridge_crate.clone(), pkg_dir.to_string()));
            }
            for (loft_sym, bridge_fn) in &m.wasm_bridge_routes {
                self.data
                    .wasm_bridge_routes
                    .insert(loft_sym.clone(), (bridge_crate.clone(), bridge_fn.clone()));
            }
        }
        // lib_plan-29 W2 (2026-05-29) — resolve `[wasm.bridge].host_js`
        // relative to the package root and register the absolute path.
        // The `--html` driver concatenates each registered file into
        // the HTML preamble.
        if let Some(ref host_js_rel) = m.wasm_bridge_host_js {
            let abs = std::path::Path::new(pkg_dir).join(host_js_rel);
            let abs_str = abs.to_string_lossy().to_string();
            if !self.data.wasm_bridge_host_js_files.contains(&abs_str) {
                self.data.wasm_bridge_host_js_files.push(abs_str);
            }
        }
        // PKG.3: register dirs for dependency resolution.
        //
        // For plain-version deps (`foo = "0.1"`) and the legacy
        // sibling-probe shape (no path declared): register the
        // package's parent dir `dir` so `<dir>/<dep_name>` resolves
        // via sibling lookup.
        //
        // @PLAN12 phase 3.5b (2026-05-24) — for path-deps
        // (`foo = { path = "../external/foo" }`), resolve the path
        // relative to this package's directory (`pkg_dir`), then
        // register the PARENT of the resolved location.  This lets
        // the existing sibling-probe at
        // `<resolved-parent>/<dep_name>/loft.toml` find the external
        // package.  Without this, the inline-table path field was
        // decorative — manifests like `lib/audience_crystal/loft.toml`
        // worked by coincidence (their `path = "../gridmesh"`
        // happened to point at a sibling in `lib/`, also covered by
        // the `--lib lib` cmdline arg).  This change makes the path
        // field actually do what it says, unlocking external library
        // extractions.
        for (dep_name, dep_value) in &m.dependencies {
            let resolved_parent = if let Some(path) = manifest::extract_path_dep(dep_value) {
                let dep_pkg_path = std::path::Path::new(pkg_dir).join(path);
                dep_pkg_path
                    .parent()
                    .map_or_else(|| dir.to_string(), |p| p.to_string_lossy().into_owned())
            } else {
                dir.to_string()
            };
            if !self.lib_dirs.contains(&resolved_parent) {
                self.lib_dirs.push(resolved_parent.clone());
            }
            if !self.data.use_exists(dep_name) {
                self.pending_pkg_deps
                    .push((dep_name.clone(), resolved_parent));
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
                let n = self.data.def(d_nr).name();
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
        // @P387 — a captured-var number can arrive from a DIFFERENT var table
        // (a capturing lambda passed as a fn-value records its captures, which
        // the outer call then marks): such a number is out of range here.  Skip
        // it rather than index OOB — consistent with the `u16::MAX` guard.
        if vnr == u16::MAX || vnr >= self.vars.count() {
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
                let name = data.def(*d_nr).name();
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
        let body = self.data.def(self.context).code().clone();
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
        let code = self.data.def(self.context).code().clone();
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
                // @PLN17 spike: boolean is tri-state (255 = null sentinel).
                // Supersedes the #256 rejection — `null` on a boolean now emits the
                // 255 sentinel; truthiness contexts coerce it to false.
                self.cl("OpConvBoolFromNull", &[])
            }
            Type::Enum(tp, _, _) => self.cl(
                "OpConvEnumFromNull",
                &[Value::Int(i32::from(self.data.def(*tp).known_type()))],
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

/// Walk a parsed expression looking for a `Value::FnRef` with a
/// non-MAX closure_var — the marker that the closure captures one or
/// P213: locate the capturing `Value::FnRef(d_nr, w, _)` inside `v` and
/// return `(d_nr, w)`.  Mirrors `capturing_fn_ref` (the bool variant)
/// but extracts the lambda's `d_nr` so callers can record it on the
/// host attribute via `Attribute::assigned_lambda_d_nr`.  Returns
/// `None` when no capturing FnRef is present.  Walks Block / Set /
/// Span wrappers built by `parser/vectors.rs` around the `OpDatabase`
/// allocation steps.
fn find_capturing_fn_ref(data: &Data, v: &Value) -> Option<(i32, u16)> {
    match v.unspan() {
        // `w != MAX` only appears in the second pass (`emit_lambda_code`
        // builds the closure-allocation block there).  In the FIRST
        // pass a capturing lambda still emits as a plain
        // `FnRef(d, MAX)`, so also accept a lambda whose def carries a
        // synthesized closure record — `synthesize_closure_record`
        // runs in both passes, making it the pass-1-visible capture
        // marker.  Without this, `assigned_lambda_d_nr` is never set
        // in pass 1 and `fill_database` lays the field out as the
        // legacy 4B int — no `<attr>__closure_rec` half (#313).
        Value::FnRef(d, w, _)
            if *w != u16::MAX || data.def(*d as u32).closure_record() != u32::MAX =>
        {
            Some((*d, *w))
        }
        // First pass only: `emit_lambda_code` emits a lambda as a bare
        // `Int(d_nr)` there (no closure-allocation block yet), so a
        // capturing lambda is recognisable only through its def's
        // closure record.  Inside the fn-ref write arm an Int IS the
        // lambda's d_nr by construction (the non-capturing write path
        // stores exactly this Int).
        Value::Int(d)
            if *d >= 0
                && (*d as usize) < data.definitions.len()
                && data.def(*d as u32).closure_record() != u32::MAX =>
        {
            Some((*d, u16::MAX))
        }
        Value::Block(bl) => bl
            .operators
            .iter()
            .find_map(|op| find_capturing_fn_ref(data, op)),
        Value::Set(_, rhs) => find_capturing_fn_ref(data, rhs),
        _ => None,
    }
}

/// P213: emit the IR for writing a fn-ref to a struct field.
///
/// Three cases:
/// - Inline non-capturing lambda (`Value::FnRef(d, MAX, _)`): write
///   `OpSetInt4(d_nr)`; the closure_rec vector stays empty.
/// - Inline capturing lambda (a `fn_ref_with_closure` Block ending in
///   `Value::FnRef(d, w, _)`): run the lambda's existing alloc_steps
///   (allocate the closure record in parent's Store under `var(w)`),
///   write `OpSetInt4(d_nr)`, then `OpAppendVector` deep-copies the
///   parent-Store closure record into element [0] of the host's
///   `__closure_rec` vector field.  Parent's record is freed by the
///   existing parent-scope `OpFreeRef(w)`; host owns its own copy.
/// - Non-inline source (`Var` / `Call` returning a fn-ref) or unrecognised
///   shape: emit a placeholder `OpSetInt4(0)` write and a parse-time
///   diagnostic so the user sees the limitation in the second pass.
fn emit_fn_ref_field_write(
    p: &mut Parser,
    d_nr: u32,
    f_nr: usize,
    ref_code: Value,
    pos_val: Value,
    val_code: &Value,
) -> Value {
    let unspanned = val_code.unspan().clone();
    match unspanned {
        Value::FnRef(d, w, _) if w == u16::MAX => {
            // Non-capturing inline lambda — just write the d_nr.
            p.cl("OpSetInt4", &[ref_code, pos_val, Value::Int(d)])
        }
        Value::Block(bl) if bl.name == "fn_ref_with_closure" => {
            // Capturing inline lambda.  bl.operators = [
            //   alloc_steps...,
            //   FnRef(d, w, _)  // last element
            // ].
            let last = bl.operators.last().cloned().unwrap_or(Value::Null);
            let Value::FnRef(d, w, _) = last.unspan() else {
                if !p.first_pass {
                    diagnostic!(
                        p.lexer,
                        Level::Error,
                        "internal: fn_ref_with_closure block did not end in FnRef"
                    );
                }
                return Value::Null;
            };
            let (lambda_d, w_var) = (*d, *w);
            let mut ops: Vec<Value> = bl
                .operators
                .iter()
                .take(bl.operators.len() - 1)
                .cloned()
                .collect();
            // Write the d_nr at the loft-attribute position (which maps
            // to the database-side `<attr>` field — the d_nr half).
            ops.push(p.cl(
                "OpSetInt4",
                &[ref_code.clone(), pos_val.clone(), Value::Int(lambda_d)],
            ));
            // Deep-copy the parent-Store closure record into the host's
            // `__closure_rec` vector at pos+4.  We need the host's
            // closure_rec field as a DbRef + the closure record's
            // known_type for `OpAppendVector`'s type parameter.
            if w_var != u16::MAX && f_nr != usize::MAX && !p.first_pass {
                let closure_rec_d = p.data.def(lambda_d as u32).closure_record();
                if closure_rec_d != u32::MAX {
                    let closure_kt = p.data.def(closure_rec_d).known_type();
                    let crec_pos = match &pos_val {
                        Value::Int(pi) => Value::Int(pi + 4),
                        _ => Value::Int(0),
                    };
                    // OpGetField(host_ref, pos+4, type_id) yields a DbRef
                    // pointing at the host's closure_rec field.
                    let crec_field = p.cl(
                        "OpGetField",
                        &[
                            ref_code.clone(),
                            crec_pos,
                            Value::Int(i32::from(closure_kt)),
                        ],
                    );
                    ops.push(p.cl(
                        "OpClaimChildRec",
                        &[
                            crec_field,
                            Value::Var(w_var),
                            Value::Int(i32::from(closure_kt)),
                        ],
                    ));
                }
            }
            v_block(ops, Type::Void, "fn_ref_field_set")
        }
        Value::Var(v) => {
            // P215: lift the deferred diagnostic when both sides are
            // non-capturing — target field is 4B int layout
            // (`assigned_lambda_d_nr == u32::MAX`) AND the source var
            // is not in `closure_vars` (presence in that map signals a
            // capturing-lambda assignment per
            // `parser/expressions.rs:1217`).  In that case writing
            // just the d_nr is lossless: the source's closure DbRef
            // component is the null sentinel and there's no
            // `__closure_rec` half on the target to receive it.
            //
            // Closure-record-internal fn-ref fields always satisfy the
            // target check (synthesize_closure_record never sets
            // `assigned_lambda_d_nr`), so capturing fn-ref names from
            // outer scopes now write through to the closure record
            // when the captured lambda itself is non-capturing —
            // which is the canonical P215 reproducer.
            // Like the read side, the split/legacy answer must come
            // from the database layout (`fn_ref_field_is_split`), not
            // the body-parse-order-dependent flag (#313).
            let target_is_4b = if (d_nr as usize) < p.data.definitions.len()
                && f_nr < p.data.def(d_nr).attributes().len()
            {
                !p.fn_ref_field_is_split(d_nr, f_nr)
            } else {
                false
            };
            let source_is_noncapturing =
                matches!(p.vars.tp(v), Type::Function(_, _, _)) && !p.closure_vars.contains_key(&v);
            if target_is_4b && source_is_noncapturing {
                return p.cl("OpSetInt4", &[ref_code, pos_val, Value::FnRefDnr(v)]);
            }
            if !p.first_pass {
                diagnostic!(
                    p.lexer,
                    Level::Error,
                    "only inline lambda literals can be stored in fn-ref struct fields; \
                     non-inline (variable / call) sources are not yet supported when the \
                     source closure may capture (P215-deferred)"
                );
            }
            // Emit a no-op so the rest of construction proceeds.
            p.cl("OpSetInt4", &[ref_code, pos_val, Value::Int(0)])
        }
        Value::Call(_, _) => {
            // P213-deferred: non-inline source.  Capturing-call
            // sources need closure-DbRef copy; deferred.
            if !p.first_pass {
                diagnostic!(
                    p.lexer,
                    Level::Error,
                    "only inline lambda literals can be stored in fn-ref struct fields in this release; \
                     bind the lambda directly inside the struct constructor"
                );
            }
            p.cl("OpSetInt4", &[ref_code, pos_val, Value::Int(0)])
        }
        other => {
            // Fallback (Null default-init etc.) — write 0.
            let d_nr_only = match other {
                Value::FnRef(d, _, _) => Value::Int(d),
                Value::Int(n) => Value::Int(n),
                Value::Null => Value::Int(0),
                v => v,
            };
            p.cl("OpSetInt4", &[ref_code, pos_val, d_nr_only])
        }
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
        Type::Text(Deps::frame(d.into_iter().collect()))
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
            let attrs = def.attributes();
            // Operators whose FIRST argument is mutated (collection / field writes).
            // vector ops folded in here so `c.items += other_vec` (where `c.items`
            // is `OpGetField(Var(c), …)`) correctly marks `c` as written via
            // collect_vars_in.  Previously the OpAppend*/OpClear* family only checked for
            // a bare `Value::Var` arg, missing the field-access shape.
            let first_arg_write = def.name().starts_with("OpSet")
                || def.name().starts_with("OpAppendStack")
                || def.name().starts_with("OpClearStack")
                || def.name() == "OpNewRecord"
                || def.name() == "OpAppendCopy"
                || def.name() == "OpAppendVector"
                || def.name() == "OpClearVector"
                || def.name() == "OpClearKeyed"
                || def.name() == "OpSetKeyed"
                // @P320: keyed-remove `coll[key] = null` lowers to
                // `OpHashRemove(coll, …)` (collections.rs::towards_set_hash_remove),
                // so it mutates its first arg just like OpSetKeyed/OpClearKeyed.
                // Without this a `&` param whose only mutation is a keyed remove
                // was wrongly rejected as "never modified".
                || def.name() == "OpHashRemove"
                || def.name() == "OpInsertVector"
                || def.name() == "OpRemoveVector";
            // OpCopyRecord(src, dst, type) writes through `dst` (arg[1]).
            // Used by struct field whole-replacement (`s.i = fresh`) where the
            // destination is `OpGetField(s, …)`.
            let second_arg_write = def.name() == "OpCopyRecord";
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
            if *def.code() != Value::Null {
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
    let n = def.attributes().len();
    // Break recursion: insert a placeholder before walking the body.
    cache.insert(fn_nr, vec![false; n]);
    if *def.code() == Value::Null || n == 0 {
        return vec![false; n];
    }
    let body = def.code().clone();
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
            let first_arg_write = def.name().starts_with("OpSet")
                || def.name() == "OpNewRecord"
                || def.name() == "OpAppendCopy"
                || def.name() == "OpAppendVector"
                || def.name() == "OpClearVector"
                || def.name() == "OpClearKeyed"
                || def.name() == "OpSetKeyed"
                // @P320: keyed-remove `coll[key] = null` → `OpHashRemove(coll, …)`;
                // mirror the find_written_vars set so a keyed remove inside a
                // `for … in &coll` loop also counts as a mutation.
                || def.name() == "OpHashRemove"
                || def.name() == "OpInsertVector"
                || def.name() == "OpRemoveVector";
            let second_arg_write = def.name() == "OpCopyRecord";
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

#[cfg(test)]
mod p269_native_backfill_tests {
    use super::*;

    /// P269 regression — with MORE THAN ONE native package registered,
    /// `backfill_native_symbol_crates` must still bind each unmapped `#native`
    /// symbol to its owning package (the registered package whose directory is a
    /// prefix of the def's source file).  The old `len() != 1` guard bailed and
    /// left them unmapped → a `--native` P269 compile error on a symbol the
    /// interpreter dispatched fine.  Two native packages (`imaging` +
    /// `p269_native_b`); we then clear the map to reproduce the crawler
    /// precondition (imaging's `#native` symbols left unbound by the per-manifest
    /// pass, which runs before the package source is parsed) so backfill is their
    /// only binding site.
    #[test]
    fn backfill_binds_symbols_under_multiple_native_packages() {
        let sep = crate::platform::sep_str();
        let mut p = Parser::new();
        p.parse_dir("default", true, true).unwrap();
        p.lib_dirs = vec![
            format!("tests{sep}lib"),
            format!("tests{sep}fixtures{sep}libs"),
        ];
        p.parse(
            &format!("tests{sep}lib{sep}p269_two_native_pkgs_main.loft"),
            false,
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "unexpected parse errors: {:?}",
            p.diagnostics.lines()
        );

        // The bug trigger: more than one registered native package.
        assert!(
            p.data.native_packages.len() >= 2,
            "expected >= 2 native packages (imaging + p269_native_b), got {:?}",
            p.data.native_packages
        );

        // Collect imaging's #native symbols FROM the parsed defs (those whose
        // source lives under imaging's package dir), so the test never hardcodes
        // a library `n_*` symbol in src/ — the @PLAN12 extraction-hygiene gate
        // forbids that, since imaging is an extracted library.
        let imaging_dir = p
            .data
            .native_packages
            .iter()
            .find(|(c, _)| c == "loft-imaging")
            .map(|(_, d)| d.clone())
            .expect("imaging registered as a native package");
        let imaging_syms: Vec<String> = (0..p.data.definitions())
            .map(|d| p.data.def(d))
            .filter(|def| {
                !def.native().is_empty() && def.position().file.starts_with(imaging_dir.as_str())
            })
            .map(|def| def.native().to_string())
            .collect();
        assert!(
            !imaging_syms.is_empty(),
            "expected imaging to contribute #native symbols"
        );

        // Reproduce the precondition the crawler hit: those symbols are not yet
        // bound, so backfill is the only place they can bind.
        p.data.native_symbol_crates.clear();
        p.backfill_native_symbol_crates();

        for sym in &imaging_syms {
            assert_eq!(
                p.data.native_symbol_crates.get(sym).map(String::as_str),
                Some("loft_imaging"),
                "{sym} not bound to loft_imaging after backfill - map={:?}",
                p.data.native_symbol_crates
            );
        }
    }

    /// @PLN26 phase 1 — two native packages exporting the SAME `#native` symbol
    /// must be detected: the C-ABI flat namespace (and the interpreter's
    /// symbol-keyed `BRIDGE_REGISTRY`) can't disambiguate them, so native
    /// codegen rejects the program with a `compile_error!`.  Parse a consumer of
    /// collide_a + collide_b (both export `collide_shared`) and check the
    /// detector reports it.
    #[test]
    fn native_symbol_collision_across_packages_detected() {
        let sep = crate::platform::sep_str();
        let mut p = Parser::new();
        p.parse_dir("default", true, true).unwrap();
        p.lib_dirs = vec![format!("tests{sep}lib")];
        p.parse(&format!("tests{sep}lib{sep}collide_main.loft"), false);
        let collisions = p.data.native_symbol_collisions();
        assert!(
            collisions
                .iter()
                .any(|(sym, srcs)| sym == "collide_shared" && srcs.len() >= 2),
            "expected a `collide_shared` collision across >= 2 sources, got {collisions:?}"
        );
    }
}

#[cfg(test)]
mod plan86_sandbox_designation_tests {
    use super::*;
    use crate::sandbox::parse_sandbox_config;

    /// @PLN86 step 1.2 — a host designation (`fn:<name>`) tags exactly the named
    /// function with its profile; other functions stay unrestricted; and the tag
    /// comes from the host policy, never the source (only `fn:scripted` is
    /// designated, yet `host` in the same file is untagged).
    #[test]
    fn fn_designation_tags_only_the_designated_function() {
        let mut p = Parser::new();
        p.set_sandbox_config(parse_sandbox_config(
            "[sandbox]\nmod-script = [\"fn:scripted\"]\n[profile.mod-script]\nallow_caps = [\"math\"]\n",
        ));
        let dir = std::env::temp_dir();
        let path = dir.join(format!("plan86_designation_{}.loft", std::process::id()));
        std::fs::write(&path, "fn scripted() { }\nfn host() { }\n").unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "unexpected parse errors: {:?}",
            p.diagnostics.lines()
        );
        let scripted = p.data.def_nr("n_scripted");
        let host = p.data.def_nr("n_host");
        assert_ne!(scripted, u32::MAX, "scripted fn registered");
        assert_ne!(host, u32::MAX, "host fn registered");
        assert_eq!(p.def_sandbox_profile(scripted), Some("mod-script"));
        assert_eq!(p.def_sandbox_profile(host), None);
    }
}

#[cfg(test)]
mod plan86_nesting_guard_tests {
    use super::*;
    use crate::sandbox::parse_sandbox_config;

    fn sandboxed_parser() -> Parser {
        let mut p = Parser::new();
        p.set_sandbox_config(parse_sandbox_config(
            "[sandbox]\nmod-script = [\"fn:scripted\"]\n[profile.mod-script]\nallow_caps = [\"math\"]\n",
        ));
        p
    }

    fn parse_source(p: &mut Parser, src: &str) {
        let path =
            std::env::temp_dir().join(format!("plan86_nest_{}_{:p}.loft", std::process::id(), src));
        std::fs::write(&path, src).unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
    }

    /// @PLN86 0.1 — hostile deep nesting inside a sandboxed def is a clean
    /// LOAD-time parse error, NOT a native stack overflow (rc=139).  Runs on an
    /// explicit 8 MB stack so the result is deterministic across CI harnesses
    /// (a stack overflow aborts the whole process, it is not a catchable panic):
    /// WITH the guard, 2000-deep parens bail at the limit (~1.3 MB); WITHOUT it
    /// they overflow even 8 MB (2000 × ~10 KB ≈ 20 MB).  Process surviving + the
    /// latched diagnostic together prove the guard fired.
    #[test]
    fn deep_nesting_in_sandboxed_def_is_a_clean_error_not_a_crash() {
        let (has_error, has_msg) = std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(|| {
                let depth = 2000; // >> SANDBOX_MAX_PARSE_DEPTH and into the overflow zone
                let src = format!(
                    "fn scripted() {{ x = {}1{}; }}\n",
                    "(".repeat(depth),
                    ")".repeat(depth)
                );
                let mut p = sandboxed_parser();
                parse_source(&mut p, &src);
                let has_error = p.diagnostics.level() >= crate::diagnostics::Level::Error;
                let has_msg = p
                    .diagnostics
                    .lines()
                    .iter()
                    .any(|l| l.contains("nesting too deep"));
                (has_error, has_msg)
            })
            .unwrap()
            .join()
            .expect("the parse thread must not overflow/panic");
        assert!(has_error, "expected a depth error");
        assert!(has_msg, "expected the nesting-depth diagnostic");
    }

    /// The guard must not false-trip: ordinary nesting well below the limit —
    /// and many sibling expressions — parse cleanly (proves the depth counter is
    /// balanced: each sub-expression decrements, so siblings don't accumulate).
    #[test]
    fn ordinary_nesting_in_sandboxed_def_parses_clean() {
        let mut body = String::from("fn scripted() {\n");
        for i in 0..200 {
            body.push_str(&format!("  a{i} = ((({i})));\n")); // shallow, 200 siblings
        }
        body.push_str("}\n");
        let mut p = sandboxed_parser();
        parse_source(&mut p, &body);
        assert!(
            !p.diagnostics
                .lines()
                .iter()
                .any(|l| l.contains("nesting too deep")),
            "guard false-tripped on ordinary code: {:?}",
            p.diagnostics.lines()
        );
    }
}

#[cfg(test)]
mod plan86_reachable_set_tests {
    use super::*;
    use crate::sandbox::parse_sandbox_config;

    fn parse_with_sandbox(selectors: &[&str], src: &str) -> Parser {
        let list = selectors
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let cfg = format!("[sandbox]\nmod = [{list}]\n[profile.mod]\nallow_caps = [\"x\"]\n");
        let mut p = Parser::new();
        p.set_sandbox_config(parse_sandbox_config(&cfg));
        p.parse_dir("default", true, true).unwrap(); // `integer` et al. live in the stdlib
        let path = std::env::temp_dir().join(format!(
            "plan86_reach_{}_{:p}.loft",
            std::process::id(),
            src
        ));
        std::fs::write(&path, src).unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
        p
    }

    /// @PLN86 1.3 — the closure descends into sandboxed defs (so their callees are
    /// reachable) but treats a trusted symbol as a LEAF: `untrusted_helper` is
    /// recorded yet NOT descended, so the `hidden_leaf` it alone reaches is
    /// excluded — the trust boundary §4 demands.
    #[test]
    fn closure_descends_sandboxed_defs_and_stops_at_trusted_leaves() {
        let src = "fn allowed_leaf() -> integer { 1 }\n\
                   fn hidden_leaf() -> integer { 2 }\n\
                   fn inner() -> integer { allowed_leaf() }\n\
                   fn untrusted_helper() -> integer { hidden_leaf() }\n\
                   fn scripted() -> integer { inner(); untrusted_helper() }\n";
        let p = parse_with_sandbox(&["fn:scripted", "fn:inner"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let reach = p.sandbox_reachable_set();
        let id = |n: &str| p.data.def_nr(n);
        assert!(reach.contains(&id("n_scripted")));
        assert!(reach.contains(&id("n_inner"))); // sandboxed → descended
        assert!(reach.contains(&id("n_allowed_leaf"))); // reached via inner
        assert!(reach.contains(&id("n_untrusted_helper"))); // referenced → leaf
        assert!(
            !reach.contains(&id("n_hidden_leaf")),
            "a trusted leaf must not be descended"
        );
    }

    /// L4 — a fn-ref passed as a `fn(...)`-typed argument (emitted as a bare
    /// `Int(def_nr)`) is in the set, so an indirect call can't escape admission.
    #[test]
    fn fnref_passed_as_argument_is_reachable_l4() {
        let src = "fn target(n: integer) -> integer { n }\n\
                   fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
                   fn scripted() -> integer { apply(target, 5) }\n";
        let p = parse_with_sandbox(&["fn:scripted"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_reachable_set()
                .contains(&p.data.def_nr("n_target")),
            "fn-ref argument must be reachable (L4)"
        );
    }

    /// L4 — a fn-ref laundered through a `fn(...)`-typed local (`f = target`,
    /// emitted as `Set(f, Int(def_nr))`) is still in the set.
    #[test]
    fn fnref_laundered_through_a_variable_is_reachable_l4() {
        let src = "fn target(n: integer) -> integer { n }\n\
                   fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
                   fn scripted() -> integer { f = target; apply(f, 5) }\n";
        let p = parse_with_sandbox(&["fn:scripted"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_reachable_set()
                .contains(&p.data.def_nr("n_target")),
            "fn-ref laundered through a variable must be reachable (L4)"
        );
    }

    /// @PLN86 1.4 — a sandboxed def reaching an EXTERNAL native bridge (its
    /// `#native` symbol owned by a native package) is flagged; a `#native` symbol
    /// with no external-package owner (a built-in op) is not.
    /// `native_symbol_crates` is the discriminator `backfill_native_symbol_crates`
    /// fills for package-owned symbols.
    #[test]
    fn reachable_external_ffi_bridge_is_flagged_local_native_is_not() {
        let src = "fn ext_fn() -> integer; #native \"ext_sym\"\n\
                   fn local_native() -> integer; #native \"local_sym\"\n\
                   fn scripted() -> integer { ext_fn(); local_native() }\n";
        let mut p = parse_with_sandbox(&["fn:scripted"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        // Simulate `ext_sym` being owned by an external native package — exactly
        // what `backfill_native_symbol_crates` records for a def under a
        // `native_packages` dir.  `local_sym` is left unowned.
        p.data
            .native_symbol_crates
            .insert("ext_sym".to_string(), "extcrate".to_string());
        let bridges = p.sandbox_ffi_bridges();
        assert!(
            bridges.contains(&p.data.def_nr("n_ext_fn")),
            "reachable external FFI bridge must be flagged, got {bridges:?}"
        );
        assert!(
            !bridges.contains(&p.data.def_nr("n_local_native")),
            "a #native symbol with no external-package owner must NOT be flagged"
        );
    }

    /// @PLN86 2.1 — `#cap "group"` parses onto a def and is readable; the
    /// read/write distinction (the `Vector.get` vs `Vector.clear` case)
    /// round-trips, and an unannotated def reads as `None`.
    #[test]
    fn cap_annotation_is_parsed_and_readable() {
        let src = "fn reader() -> integer;\n#native\n#cap \"collections.read\"\n\
                   fn writer() -> integer;\n#native\n#cap \"collections.write\"\n\
                   fn plain() -> integer { 0 }\n";
        let p = parse_with_sandbox(&[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert_eq!(
            p.def_cap_group(p.data.def_nr("n_reader")),
            Some("collections.read")
        );
        assert_eq!(
            p.def_cap_group(p.data.def_nr("n_writer")),
            Some("collections.write")
        );
        assert_eq!(p.def_cap_group(p.data.def_nr("n_plain")), None);
    }
}

#[cfg(test)]
mod plan86_admission_tests {
    use super::*;
    use crate::sandbox::{CapViolation, TotalityViolation, parse_sandbox_config};

    fn quoted(items: &[&str]) -> String {
        items
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn parse_admit_libs(
        designations: &[&str],
        allow_libs: &[&str],
        allow_caps: &[&str],
        src: &str,
    ) -> Parser {
        let cfg = format!(
            "[sandbox]\nmod = [{}]\n[profile.mod]\nallow_libs = [{}]\nallow_caps = [{}]\n",
            quoted(designations),
            quoted(allow_libs),
            quoted(allow_caps),
        );
        let mut p = Parser::new();
        p.set_sandbox_config(parse_sandbox_config(&cfg));
        p.parse_dir("default", true, true).unwrap();
        let path = std::env::temp_dir().join(format!(
            "plan86_admit_{}_{:p}.loft",
            std::process::id(),
            src
        ));
        std::fs::write(&path, src).unwrap();
        p.parse(path.to_str().unwrap(), false);
        let _ = std::fs::remove_file(&path);
        p
    }

    fn parse_admit(designations: &[&str], allow_caps: &[&str], src: &str) -> Parser {
        parse_admit_libs(designations, &[], allow_caps, src)
    }

    fn viol_symbol(v: &CapViolation) -> u32 {
        match v {
            CapViolation::UngrantedCap { symbol, .. }
            | CapViolation::UntaggedSymbol { symbol, .. }
            | CapViolation::ExternalFfi { symbol, .. } => *symbol,
        }
    }

    /// @PLN86 2.3 — the admission convergence: a granted-cap reference admits, an
    /// ungranted-cap reference is rejected naming the group, an untagged symbol is
    /// rejected (deny-by-default).
    #[test]
    fn admission_grants_allowed_caps_and_rejects_ungranted_and_untagged() {
        let src = "fn cap_fs_read() -> integer;\n#native\n#cap \"fs.read\"\n\
                   fn cap_coll_read() -> integer;\n#native\n#cap \"collections.read\"\n\
                   fn cap_untagged() -> integer;\n#native\n\
                   fn ok() -> integer { cap_coll_read() }\n\
                   fn bad() -> integer { cap_fs_read() }\n\
                   fn uses_untagged() -> integer { cap_untagged() }\n";
        let p = parse_admit(
            &["fn:ok", "fn:bad", "fn:uses_untagged"],
            &["collections.read"],
            src,
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let v = p.sandbox_admit();
        // bad reaches fs.read which is not granted → named violation.
        assert!(
            v.contains(&CapViolation::UngrantedCap {
                from: p.data.def_nr("n_bad"),
                symbol: p.data.def_nr("n_cap_fs_read"),
                group: "fs.read".to_string(),
            }),
            "expected UngrantedCap(fs.read), got {v:?}"
        );
        // uses_untagged reaches an unclassified symbol → deny-by-default.
        assert!(
            v.contains(&CapViolation::UntaggedSymbol {
                from: p.data.def_nr("n_uses_untagged"),
                symbol: p.data.def_nr("n_cap_untagged"),
            }),
            "expected UntaggedSymbol, got {v:?}"
        );
        // ok reaches only collections.read (granted) → admitted clean.
        let coll = p.data.def_nr("n_cap_coll_read");
        assert!(
            !v.iter().any(|viol| viol_symbol(viol) == coll),
            "granted collections.read must admit clean, got {v:?}"
        );
    }

    /// @PLN86 2.3 / L4 — an indirect call through a fn-ref cannot escape: passing
    /// `cap_fs_read` to a higher-order fn still puts it in the reachable set, so
    /// admission rejects the ungranted `fs.read` group.
    #[test]
    fn admission_rejects_indirect_fnref_call_l4() {
        let src = "fn cap_fs_read(n: integer) -> integer;\n#native\n#cap \"fs.read\"\n\
                   fn apply(f: fn(integer) -> integer, n: integer) -> integer { f(n) }\n\
                   fn sneaky() -> integer { apply(cap_fs_read, 5) }\n";
        let p = parse_admit(&["fn:sneaky", "fn:apply"], &["collections.read"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let v = p.sandbox_admit();
        assert!(
            v.contains(&CapViolation::UngrantedCap {
                from: p.data.def_nr("n_sneaky"),
                symbol: p.data.def_nr("n_cap_fs_read"),
                group: "fs.read".to_string(),
            }),
            "L4 indirect fn-ref to fs.read must be rejected, got {v:?}"
        );
    }

    /// @PLN86 2.2 — the coverage lint lists an untagged public function and omits a
    /// tagged one (the work-list for tagging the stdlib/library surface).
    #[test]
    fn coverage_lint_lists_untagged_public_functions() {
        let src = "pub fn tagged_fn() -> integer;\n#native\n#cap \"math\"\n\
                   pub fn untagged_fn() -> integer;\n#native\n";
        let p = parse_admit(&[], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let untagged = p.untagged_public_symbols();
        assert!(
            untagged.contains(&p.data.def_nr("n_untagged_fn")),
            "an untagged public fn must be listed"
        );
        assert!(
            !untagged.contains(&p.data.def_nr("n_tagged_fn")),
            "a tagged public fn must NOT be listed"
        );
    }

    /// @PLN86 2.2 — the REAL stdlib fs/env surface gates correctly: `mtime`
    /// (tagged `fs.read`, bodiless so no untagged deps) is rejected naming the
    /// group when fs.read is not granted, while `env_variable` (`env`, granted)
    /// admits — and the tagged fs/env fns have left the coverage lint.
    #[test]
    fn stdlib_fs_env_caps_gate_real_functions() {
        let src = "fn reads_mtime() -> integer { mtime(\"x\") }\n\
                   fn reads_env() -> text { env_variable(\"X\") }\n";
        let p = parse_admit(&["fn:reads_mtime", "fn:reads_env"], &["env"], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        // the fs/env surface is no longer flagged by the coverage lint
        let untagged = p.untagged_public_symbols();
        for n in [
            "n_content",
            "n_write",
            "n_env_variable",
            "n_mtime",
            "n_file",
        ] {
            assert!(
                !untagged.contains(&p.data.def_nr(n)),
                "{n} should be tagged (off the lint)"
            );
        }
        let v = p.sandbox_admit();
        // mtime (fs.read) is not granted → rejected naming the real group.
        assert!(
            v.contains(&CapViolation::UngrantedCap {
                from: p.data.def_nr("n_reads_mtime"),
                symbol: p.data.def_nr("n_mtime"),
                group: "fs.read".to_string(),
            }),
            "mtime must be rejected naming fs.read, got {v:?}"
        );
        // env_variable (env) is granted → admits clean.
        let envv = p.data.def_nr("n_env_variable");
        assert!(
            !v.iter().any(|x| viol_symbol(x) == envv),
            "granted env must admit, got {v:?}"
        );
    }

    /// @PLN86 — library-first admission: a wholesale-allowed library admits its
    /// UNTAGGED functions with no `#cap` tag (the common "include a whole
    /// library" case).  `now()` lives in the `files` stdlib module and is
    /// untagged; under `allow_libs=["files"]` it admits, without it it is an
    /// `UntaggedSymbol`.  Tags stay the fine-grained layer, not a requirement.
    #[test]
    fn wholesale_allowed_library_admits_untagged_functions() {
        // def_library identifies the stdlib module from the source file.
        let p0 = parse_admit(&[], &[], "fn ignore() -> integer { 1 }\n");
        assert_eq!(
            p0.def_library(p0.data.def_nr("n_mtime")).as_deref(),
            Some("files")
        );

        let src = "fn uses_now() -> integer { now() }\n";
        // (a) files allowed wholesale → untagged now() admits, no tag needed.
        let p = parse_admit_libs(&["fn:uses_now"], &["files"], &[], src);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_admit().is_empty(),
            "wholesale files must admit untagged now(), got {:?}",
            p.sandbox_admit()
        );
        // (b) nothing allowed → now() is rejected (deny-by-default).
        let p2 = parse_admit_libs(&["fn:uses_now"], &[], &[], src);
        let now = p2.data.def_nr("n_now");
        assert!(
            p2.sandbox_admit().iter().any(|x| viol_symbol(x) == now),
            "without allow_libs, untagged now() must be flagged, got {:?}",
            p2.sandbox_admit()
        );
    }

    /// @PLN86 2.5 — admission errors are actionable: each class names the
    /// position, the symbol, the rule, and BOTH fixes (wholesale lib + the
    /// fine-grained cap / native_ffi).
    #[test]
    fn admission_errors_name_symbol_rule_and_fix() {
        // UngrantedCap — mtime needs fs.read; only env is granted.
        let p = parse_admit(
            &["fn:reads_mtime"],
            &["env"],
            "fn reads_mtime() -> integer { mtime(\"x\") }\n",
        );
        let errs = p.sandbox_admission_errors();
        assert_eq!(errs.len(), 1, "{errs:?}");
        let e = &errs[0];
        eprintln!("CAP_DIAG: {e}");
        assert!(e.contains("mtime"), "names the symbol: {e}");
        assert!(e.contains("fs.read"), "names the group: {e}");
        assert!(e.contains("fix:"), "points at the fix: {e}");
        assert!(
            e.contains("allow_caps") && e.contains("allow_libs"),
            "offers both fixes: {e}"
        );
        assert!(e.contains(".loft:"), "carries a source position: {e}");

        // UntaggedSymbol — now() in `files`, nothing allowed.
        let p2 = parse_admit_libs(
            &["fn:uses_now"],
            &[],
            &[],
            "fn uses_now() -> integer { now() }\n",
        );
        assert!(
            p2.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("now")
                    && e.contains("library `files`")
                    && e.contains("allow_libs")),
            "untagged error names the library + allow_libs: {:?}",
            p2.sandbox_admission_errors()
        );
    }

    /// @PLN86 2.5 — an external-FFI rejection names the crate and `native_ffi`.
    #[test]
    fn admission_error_for_external_ffi_names_crate() {
        let src = "fn ext_fn() -> integer; #native \"ext_sym\"\n\
                   fn calls_ext() -> integer { ext_fn() }\n";
        let mut p = parse_admit(&["fn:calls_ext"], &[], src);
        p.data
            .native_symbol_crates
            .insert("ext_sym".to_string(), "extcrate".to_string());
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("ext_fn")
                    && e.contains("extcrate")
                    && e.contains("native_ffi")),
            "external-FFI error names the crate + native_ffi: {:?}",
            p.sandbox_admission_errors()
        );
    }

    /// @PLN86 3.1 — totality rejects an unbounded `while`, admits a bounded `for`.
    #[test]
    fn totality_rejects_while_admits_for() {
        let p = parse_admit_libs(
            &["fn:loops"],
            &["code"],
            &[],
            "fn loops() -> integer { x = 0; while x < 10 { x += 1 } x }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        let t = p.sandbox_totality();
        assert!(
            t.iter()
                .any(|v| matches!(v, TotalityViolation::UnboundedLoop { .. })),
            "a `while` must be rejected, got {t:?}"
        );
        // the rendered error points at the bounded alternative
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("while") && e.contains("for ")),
            "{:?}",
            p.sandbox_admission_errors()
        );

        let p2 = parse_admit_libs(
            &["fn:counts"],
            &["code"],
            &[],
            "fn counts() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
        );
        assert!(
            p2.sandbox_totality().is_empty(),
            "a bounded `for` must be total, got {:?}",
            p2.sandbox_totality()
        );
    }

    /// @PLN86 3.2 — totality rejects recursion (self + mutual), admits acyclic.
    #[test]
    fn totality_rejects_recursion_admits_acyclic() {
        // self-recursion `f -> f`
        let p = parse_admit_libs(
            &["fn:rec"],
            &["code"],
            &[],
            "fn rec(n: integer) -> integer { rec(n + 1) }\n",
        );
        assert!(
            p.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
            "self-recursion must be rejected, got {:?}",
            p.sandbox_totality()
        );
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("recursion") && e.contains("rec")),
            "{:?}",
            p.sandbox_admission_errors()
        );

        // mutual recursion `a -> b -> a`
        let p2 = parse_admit_libs(
            &["fn:a", "fn:b"],
            &["code"],
            &[],
            "fn a(n: integer) -> integer { b(n) }\nfn b(n: integer) -> integer { a(n) }\n",
        );
        assert!(
            p2.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
            "mutual recursion must be rejected, got {:?}",
            p2.sandbox_totality()
        );

        // acyclic call chain `top -> helper`
        let p3 = parse_admit_libs(
            &["fn:top", "fn:helper"],
            &["code"],
            &[],
            "fn helper(n: integer) -> integer { n + 1 }\n\
             fn top(n: integer) -> integer { helper(n) }\n",
        );
        assert!(
            !p3.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::Recursion { .. })),
            "an acyclic script must be total, got {:?}",
            p3.sandbox_totality()
        );
    }

    /// @PLN86 3.3 — total-op check: an explicit-abort op (`assert`) is excluded
    /// (it faults the script), while arithmetic stays total — a divide-by-zero is
    /// the interpreter's null sentinel, NOT a rejected op.
    #[test]
    fn totality_excludes_abort_ops_admits_total_arithmetic() {
        // assert faults → excluded as a partial op
        let p = parse_admit_libs(
            &["fn:checks"],
            &["code"],
            &[],
            "fn checks(n: integer) -> integer { assert(n > 0, \"pos\"); n }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::PartialOp { .. })),
            "assert must be excluded as a partial op, got {:?}",
            p.sandbox_totality()
        );
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("assert") && e.contains("fix")),
            "{:?}",
            p.sandbox_admission_errors()
        );

        // a divide-by-zero is total on the interpreter → NOT a partial op
        let p2 = parse_admit_libs(
            &["fn:divides"],
            &["code"],
            &[],
            "fn divides(a: integer, b: integer) -> integer { a / b ?? 0 }\n",
        );
        assert!(
            !p2.sandbox_totality()
                .iter()
                .any(|v| matches!(v, TotalityViolation::PartialOp { .. })),
            "arithmetic is total (not excluded), got {:?}",
            p2.sandbox_totality()
        );
    }

    /// @PLN86 1.4 — any designated def forces interpret-only (unconditional); a
    /// program with no sandboxed defs leaves the native backend available.
    #[test]
    fn sandbox_forces_interpret_on_any_designation() {
        let p = parse_admit_libs(
            &["fn:scripted"],
            &["code"],
            &[],
            "fn scripted() -> integer { 1 }\n",
        );
        assert!(
            p.sandbox_forces_interpret(),
            "a sandboxed def must force interpret-only"
        );
        let p2 = parse_admit_libs(&[], &["code"], &[], "fn plain() -> integer { 1 }\n");
        assert!(
            !p2.sandbox_forces_interpret(),
            "no sandboxed defs → native allowed"
        );
    }

    /// @PLN86 3.4 — the worst-case complexity degree counts loop nesting, and
    /// composes across the acyclic call graph (a loop calling a looping fn → n²).
    #[test]
    fn complexity_degree_counts_loop_nesting_inter_procedural() {
        // no loop → O(1)
        let p0 = parse_admit_libs(
            &["fn:flat"],
            &["code"],
            &[],
            "fn flat() -> integer { 1 + 2 }\n",
        );
        assert_eq!(p0.sandbox_complexity_degree(), 0, "no loops → O(1)");

        // one loop → O(n)
        let p1 = parse_admit_libs(
            &["fn:one"],
            &["code"],
            &[],
            "fn one() -> integer { s = 0; for i in 0..10 { s += i } s }\n",
        );
        assert_eq!(p1.sandbox_complexity_degree(), 1, "one loop → O(n)");

        // nested loops → O(n^2)
        let p2 = parse_admit_libs(
            &["fn:nest"],
            &["code"],
            &[],
            "fn nest() -> integer { s = 0; for i in 0..10 { for j in 0..10 { s += i } } s }\n",
        );
        assert_eq!(p2.sandbox_complexity_degree(), 2, "nested loops → O(n^2)");
        assert!(
            p2.sandbox_complexity_report().contains("O(n^2)"),
            "{}",
            p2.sandbox_complexity_report()
        );

        // inter-procedural: a loop calling a looping fn → O(n^2)
        let p3 = parse_admit_libs(
            &["fn:outer", "fn:inner"],
            &["code"],
            &[],
            "fn inner() -> integer { s = 0; for j in 0..10 { s += j } s }\n\
             fn outer() -> integer { s = 0; for i in 0..10 { s += inner() } s }\n",
        );
        assert_eq!(
            p3.sandbox_complexity_degree(),
            2,
            "a loop calling a looping fn → O(n^2)"
        );
    }

    /// @PLN86 2.4 — no-raw-write rejects a field/index assignment to heap data,
    /// while struct construction + local-variable writes stay clean.
    #[test]
    fn no_raw_write_rejects_field_index_admits_construction() {
        // field write → rejected, with an actionable message
        let p = parse_admit_libs(
            &["fn:scripted"],
            &["code", "prog"],
            &[],
            "struct Ent { health: integer }\n\
             fn scripted(e: Ent) -> integer { e.health = 0; e.health }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert_eq!(
            p.sandbox_raw_writes().len(),
            1,
            "a field write must be flagged, got {:?}",
            p.sandbox_raw_writes()
        );
        assert!(
            p.sandbox_admission_errors()
                .iter()
                .any(|e| e.contains("raw write") && e.contains("fix")),
            "{:?}",
            p.sandbox_admission_errors()
        );

        // index write → rejected
        let p2 = parse_admit_libs(
            &["fn:scripted"],
            &["code", "prog"],
            &[],
            "fn scripted(v: vector<integer>) -> integer { v[0] = 9; v[0] }\n",
        );
        assert!(
            !p2.sandbox_raw_writes().is_empty(),
            "an index write must be flagged, got {:?}",
            p2.sandbox_raw_writes()
        );

        // struct construction + local writes → admitted (no raw write)
        let p3 = parse_admit_libs(
            &["fn:scripted"],
            &["code", "prog"],
            &[],
            "struct Ent { health: integer }\n\
             fn scripted() -> integer { s = 0; for i in 0..3 { s += i } e = Ent { health: s }; e.health }\n",
        );
        assert!(
            p3.sandbox_raw_writes().is_empty(),
            "construction + local writes must be clean, got {:?}",
            p3.sandbox_raw_writes()
        );
    }

    /// @PLN86 2.4 (ownership-aware) — a mod may MUTATE the data it owns (a local of
    /// a script-defined struct — the dogfood `e.alive = false` pattern), but NOT
    /// host data (a parameter — the type also catches aliasing).
    #[test]
    fn no_raw_write_admits_script_owned_struct_mutation() {
        // local script-owned struct mutation → admitted (no violation at all)
        let p = parse_admit_libs(
            &["fn:scripted"],
            &["code"],
            &[],
            "struct Mob { hp: integer }\n\
             fn scripted() -> integer { m = Mob { hp: 5 }; m.hp = 0; m.hp }\n",
        );
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "parse errors: {:?}",
            p.diagnostics.lines()
        );
        assert!(
            p.sandbox_raw_writes().is_empty(),
            "mutating a script-owned struct must be allowed, got {:?}",
            p.sandbox_raw_writes()
        );
        assert!(
            p.sandbox_admission_errors().is_empty(),
            "a self-owned mutation script must fully admit, got {:?}",
            p.sandbox_admission_errors()
        );

        // the SAME write to a PARAMETER (host data) is still rejected
        let p2 = parse_admit_libs(
            &["fn:scripted"],
            &["code"],
            &[],
            "struct Mob { hp: integer }\n\
             fn scripted(m: Mob) -> integer { m.hp = 0; m.hp }\n",
        );
        assert!(
            !p2.sandbox_raw_writes().is_empty(),
            "a write to a parameter (host data) must be rejected, got {:?}",
            p2.sandbox_raw_writes()
        );
    }
}
