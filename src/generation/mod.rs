// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use crate::data::{Context, Data, DefType, Type, Value};
use crate::database::Stores;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
mod calls;
mod coroutine;
mod dispatch;
mod emit;
mod pre_eval;
mod text;

/// Entry produced by `collect_pre_evals`: `(temp_var, type_str, expr_code, def_nr, stores_fn)`.
type PreEvalEntry = (String, String, String, u32, bool);

/// Walk the Value IR tree and collect all function definition numbers
/// referenced by `Value::Call(def_nr, _)` nodes.
fn collect_calls(val: &Value, data: &Data, calls: &mut HashSet<u32>) {
    match val {
        Value::Call(d, args) => {
            calls.insert(*d);
            // n_parallel_for passes a worker function as args[4]: an integer
            // literal that is resolved to a closure in native output_call.
            // Detect it here so the worker is included in the reachable set.
            if matches!(
                data.def(*d).name.as_str(),
                "n_parallel_for" | "n_parallel_for_light"
            ) && args.len() >= 5
                && let Value::Int(fn_d_nr) = &args[4]
                && *fn_d_nr >= 0
            {
                calls.insert((*fn_d_nr).cast_unsigned());
            }
            for a in args {
                collect_calls(a, data, calls);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                collect_calls(op, data, calls);
            }
        }
        Value::If(test, t, f) => {
            collect_calls(test, data, calls);
            collect_calls(t, data, calls);
            collect_calls(f, data, calls);
        }
        Value::Set(_, v) | Value::Return(v) | Value::Drop(v) => collect_calls(v, data, calls),
        Value::Insert(ops) => {
            for op in ops {
                collect_calls(op, data, calls);
            }
        }
        Value::Iter(_, create, next, extra) => {
            collect_calls(create, data, calls);
            collect_calls(next, data, calls);
            collect_calls(extra, data, calls);
        }
        // N8b.1: walk into yield expressions so helper functions are included in the
        // reachable set and emitted before the coroutine state-machine struct.
        Value::Yield(inner) => collect_calls(inner, data, calls),
        _ => {}
    }
}

/// Recursively collect all `Int` literals from a value tree that may represent
/// fn-ref constants (e.g. inside `if`/`block` branches of a function-typed `Set`).
fn collect_int_fn_refs(val: &Value, calls: &mut HashSet<u32>) {
    match val {
        Value::Int(n) => {
            if *n >= 0 {
                calls.insert((*n).cast_unsigned());
            }
        }
        // FnRef(d_nr, clos_var, _) is used for closure fn-refs.
        Value::FnRef(d_nr, _, _) => {
            if *d_nr >= 0 {
                calls.insert((*d_nr).cast_unsigned());
            }
        }
        Value::If(test, t, f) => {
            collect_int_fn_refs(test, calls);
            collect_int_fn_refs(t, calls);
            collect_int_fn_refs(f, calls);
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                collect_int_fn_refs(op, calls);
            }
        }
        Value::Return(v) | Value::Drop(v) => collect_int_fn_refs(v, calls),
        _ => {}
    }
}

/// Scan a definition's code for fn-ref literals:
/// - `Set(var, Int(n))` where `var` has a `Function` or `Routine` type
/// - `Call(d, args)` where a parameter of `d` is `Function`/`Routine` typed and the
///   corresponding arg is `Int(n)`
///
/// These are function-pointer uses like `f = fn double_it` or `apply_fn(fn double_it, x)`.
fn collect_fn_ref_literals(
    val: &Value,
    data: &Data,
    variables: &crate::variables::Function,
    calls: &mut HashSet<u32>,
) {
    match val {
        Value::Set(var, inner) => {
            if matches!(
                variables.tp(*var),
                Type::Function(_, _, _) | Type::Routine(_)
            ) {
                collect_int_fn_refs(inner, calls);
            }
            collect_fn_ref_literals(inner, data, variables, calls);
        }
        Value::Call(d, args) => {
            let callee = data.def(*d);
            for (idx, a) in args.iter().enumerate() {
                if idx < callee.attributes.len()
                    && matches!(
                        callee.attributes[idx].typedef,
                        Type::Function(_, _, _) | Type::Routine(_)
                    )
                {
                    collect_int_fn_refs(a, calls);
                }
                collect_fn_ref_literals(a, data, variables, calls);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                collect_fn_ref_literals(op, data, variables, calls);
            }
        }
        Value::If(test, t, f) => {
            collect_fn_ref_literals(test, data, variables, calls);
            collect_fn_ref_literals(t, data, variables, calls);
            collect_fn_ref_literals(f, data, variables, calls);
        }
        Value::Return(v) | Value::Drop(v) => collect_fn_ref_literals(v, data, variables, calls),
        Value::Insert(ops) => {
            for op in ops {
                collect_fn_ref_literals(op, data, variables, calls);
            }
        }
        Value::Iter(_, create, next, extra) => {
            collect_fn_ref_literals(create, data, variables, calls);
            collect_fn_ref_literals(next, data, variables, calls);
            collect_fn_ref_literals(extra, data, variables, calls);
        }
        // FnRef inside a Block result (closure allocation block).
        Value::FnRef(d_nr, _, _) => {
            if *d_nr >= 0 {
                calls.insert((*d_nr).cast_unsigned());
            }
        }
        _ => {}
    }
}

/// Compute the set of function definitions reachable from `entry_defs` via
/// transitive calls and fn-ref literals.  Returns the full reachable set
/// including `entry_defs`.
#[must_use]
pub fn reachable_functions(data: &Data, entry_defs: &[u32]) -> HashSet<u32> {
    let mut reachable = HashSet::new();
    let mut queue: VecDeque<u32> = entry_defs.iter().copied().collect();
    while let Some(d) = queue.pop_front() {
        if !reachable.insert(d) {
            continue;
        }
        let def = data.def(d);
        let mut calls = HashSet::new();
        collect_calls(&def.code, data, &mut calls);
        collect_fn_ref_literals(&def.code, data, &def.variables, &mut calls);
        for c in calls {
            if !reachable.contains(&c) {
                queue.push_back(c);
            }
        }
    }
    reachable
}

/// Use this to drive Rust code generation from a compiled loft program.
/// It bundles the read-only compile-time data with the mutable emission state
/// so that individual emits functions don't need to pass both separately.
pub struct Output<'a> {
    pub data: &'a Data,
    pub stores: &'a Stores,
    pub counter: u32,
    pub def_nr: u32,
    pub indent: u32,
    pub declared: HashSet<u16>,
    /// Set of reachable `def_nrs` for native output (populated by `output_native_reachable`).
    pub reachable: HashSet<u32>,
    /// Stack of enclosing loop scope ids, innermost last.
    /// Used to emit Rust labeled breaks for `Value::Break(n)` with n > 0.
    pub loop_stack: Vec<u16>,
    /// O7: number of consecutive format/append ops following the current
    /// `OpClearStackText`/`OpClearText`.  Set by `output_block` before each
    /// op is emitted; consumed (and reset to 0) by `clear_stack_text`.
    pub next_format_count: usize,
    /// When true, `Value::Yield(expr)` emits `__values.push((expr) as i64);`
    /// instead of `yield expr`.  Used in the eager-collect factory function
    /// for `ForLoopBody` coroutine segments.
    pub yield_collect: bool,
    /// When true, `Value::Int` emits a `(d_nr_u32, null_DbRef)` tuple
    /// instead of `d_nr_i32`.  Set during fn-ref variable assignment so
    /// if-else branches produce the correct tuple type.
    pub fn_ref_context: bool,
    /// When set, `output_block` inserts this code right after the opening `{`.
    /// Used to inject `cr_call_push` / `CallGuard` for shadow call stack support.
    pub call_stack_prefix: Option<String>,
    /// When true, emit `#[no_mangle] pub extern "C" fn loft_start()`
    /// instead of `fn main()` and use WASM imports for native package functions.
    pub wasm_browser: bool,
}

/// Use this to convert loft names that contain `#` into valid Rust identifiers.
/// Loft uses `#` as a separator in compiler-generated names (e.g., loop iterators).
fn sanitize(name: &str) -> String {
    name.replace('#', "__")
}

/// Use this to determine whether a type is a narrow integer subtype (u8/u16/i8/i16).
/// Returns `Some("u8")` etc. when a cast from `i32` to that type is needed at return sites.
/// Returns `None` for `i32`, `i64`, and all non-integer types.
#[must_use]
fn narrow_int_cast(tp: &Type) -> Option<&'static str> {
    match tp {
        Type::Integer(from, to, _)
            if i64::from(*to) - i64::from(*from) <= 255 && i64::from(*from) >= 0 =>
        {
            Some("u8")
        }
        Type::Integer(from, to, _)
            if i64::from(*to) - i64::from(*from) <= 65536 && i64::from(*from) >= 0 =>
        {
            Some("u16")
        }
        Type::Integer(from, to, _) if i64::from(*to) - i64::from(*from) <= 255 => Some("i8"),
        Type::Integer(from, to, _) if i64::from(*to) - i64::from(*from) <= 65536 => Some("i16"),
        _ => None,
    }
}

/// Use this to map a loft type to the Rust type used in generated code.
/// The context controls whether the type appears as an owned value, argument, variable, or reference.
///
/// # Panics
/// When the rust type cannot be determined.
#[must_use]
pub fn rust_type(tp: &Type, context: &Context) -> String {
    if context == &Context::Reference {
        let mut result = String::new();
        result += "&";
        result += &rust_type(tp, &Context::Argument);
        return result;
    }
    if let Type::RefVar(in_tp) = tp {
        return format!("&mut {}", rust_type(in_tp, &Context::Variable));
    }
    match tp {
        // Narrow integer subtypes use their precise Rust type only in the function-return
        // context.  In variable and argument contexts `i32` is used instead to avoid
        // cascading type-mismatch errors when the variable is passed to a template
        // operation (e.g. `set_short`) that expects `i32`.  The `return` site adds an
        // explicit `as u16` / `as u8` cast (see `narrow_int_cast`).
        Type::Integer(from, to, _)
            if context == &Context::Result
                && i64::from(*to) - i64::from(*from) <= 255
                && i64::from(*from) >= 0 =>
        {
            "u8"
        }
        Type::Integer(from, to, _)
            if context == &Context::Result
                && i64::from(*to) - i64::from(*from) <= 65536
                && i64::from(*from) >= 0 =>
        {
            "u16"
        }
        Type::Integer(from, to, _)
            if context == &Context::Result && i64::from(*to) - i64::from(*from) <= 255 =>
        {
            "i8"
        }
        Type::Integer(from, to, _)
            if context == &Context::Result && i64::from(*to) - i64::from(*from) <= 65536 =>
        {
            "i16"
        }
        Type::Enum(_, false, _) => "u8",
        Type::Integer(_, _, _) | Type::Character | Type::Null => "i32",
        // null is represented as the null sentinel of the target type
        Type::Text(_) if context == &Context::Variable => "String",
        Type::Text(_) if context == &Context::Argument => "&str",
        Type::Text(_) => "Str",
        Type::Long => "i64",
        Type::Boolean => "bool",
        Type::Float => "f64",
        Type::Single => "f32",
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Index(_, _, _)
        // N8b.1: generator variables are stored as DbRef (index into native coroutine table).
        | Type::Iterator(_, _) => "DbRef",
        Type::Routine(_) => "u32",
        // C39/A5.6: fn-ref carries d_nr + closure DbRef as a tuple.
        Type::Function(_, _, _) => "(u32, DbRef)",
        Type::Unknown(_) => "??",
        Type::Keys => "&[Key]",
        Type::Void => "()",
        // N8a.1: emit the correct Rust tuple type, e.g. (i32, i64) for (integer, long).
        Type::Tuple(elems) => {
            let parts: Vec<String> = elems.iter().map(|e| rust_type(e, context)).collect();
            return format!("({})", parts.join(", "));
        }
        Type::Rewritten(inner) => return rust_type(inner, context),
        _ => panic!("Incorrect type {tp:?}"),
    }
    .to_string()
}

/// Return the Rust literal for the "null" default of a loft type, used when a function
/// body is empty (an explicit stub) but the declared return type is non-void.
pub(super) fn default_native_value(tp: &Type) -> String {
    match tp {
        Type::Float => "0.0_f64".into(),
        Type::Single => "0.0_f32".into(),
        Type::Long => "0_i64".into(),
        Type::Boolean => "false".into(),
        Type::Text(_) => "Str::new(loft::state::STRING_NULL)".into(),
        Type::Routine(_) => "0_u32".into(),
        Type::Function(_, _, _) => {
            "(0_u32, DbRef { store_nr: u16::MAX, rec: 0, pos: 0 })".into()
        }
        Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Sorted(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Index(_, _, _)
        // N8b.1: exhausted / uninitialized generator variable.
        | Type::Iterator(_, _) => "DbRef { store_nr: u16::MAX, rec: 0, pos: 8 }".into(),
        // N8a.1: a tuple null is the zero-default for each element type.
        Type::Tuple(elems) => {
            let parts: Vec<String> = elems.iter().map(default_native_value).collect();
            format!("({})", parts.join(", "))
        }
        _ => "0".into(), // Integer, Character, Enum(u8), etc.
    }
}

impl Output<'_> {
    /// Use this before emitting indented output lines.
    /// # Errors
    /// When the output cannot be written
    pub fn indent(&self, w: &mut dyn Write) -> std::io::Result<()> {
        for _i in 0..=self.indent {
            write!(w, "  ")?;
        }
        Ok(())
    }

    /// Use this to reset the emission state when starting a new function.
    pub fn start_fn(&mut self, def_nr: u32) {
        self.def_nr = def_nr;
        self.indent = 0;
        self.declared.clear();
        self.next_format_count = 0;
    }

    /// Emit the common Rust file header (attributes, imports, `mod external`).
    fn emit_file_header(w: &mut dyn Write, data: &Data, wasm_browser: bool) -> std::io::Result<()> {
        writeln!(
            w,
            "\
#![allow(unused_imports)]
#![allow(unused_parens)]
#![allow(unused_variables)]
#![allow(unreachable_code)]
#![allow(unused_mut)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(redundant_semicolons)]
#![allow(unused_assignments)]
#![allow(unused_labels)]
#![allow(unused_braces)]
#![allow(clippy::double_parens)]
#![allow(clippy::unused_unit)]
#![allow(unused_unsafe)]

extern crate loft;"
        )?;
        if wasm_browser {
            // declare host-imported functions for browser WASM.
            writeln!(w, "#[link(wasm_import_module = \"loft_io\")]")?;
            writeln!(w, "unsafe extern \"C\" {{")?;
            writeln!(
                w,
                "    safe fn loft_host_print(ptr: *const u8, len: usize);"
            )?;
            writeln!(w, "}}")?;
            // W1.1 step 6: emit WASM import declarations for all #native functions.
            // Each native symbol gets declared as an imported extern "C" function so
            // the generated code can call it directly (unqualified).
            writeln!(w, "#[link(wasm_import_module = \"loft_gl\")]")?;
            writeln!(w, "unsafe extern \"C\" {{")?;
            let mut declared_natives = std::collections::HashSet::new();
            for d_nr in 0..data.definitions() {
                let def = data.def(d_nr);
                if def.native.is_empty() || declared_natives.contains(&def.native) {
                    continue;
                }
                declared_natives.insert(def.native.clone());
                // Build the C-ABI signature from loft parameter types.
                use std::fmt::Write as _;
                let mut params = String::new();
                for attr in &def.attributes {
                    if attr.name.starts_with("__") {
                        continue;
                    }
                    if !params.is_empty() {
                        params.push_str(", ");
                    }
                    let name = sanitize(&attr.name);
                    match &attr.typedef {
                        Type::Text(_) => params.push_str("ptr: *const u8, len: usize"),
                        Type::Vector(elem_tp, _) => {
                            let elem = Self::vector_elem_rust_type(elem_tp);
                            let _ = write!(params, "ptr: *const {elem}, count: u32");
                        }
                        Type::Long => {
                            let _ = write!(params, "{name}: i64");
                        }
                        Type::Float => {
                            let _ = write!(params, "{name}: f64");
                        }
                        Type::Single => {
                            let _ = write!(params, "{name}: f32");
                        }
                        Type::Boolean => {
                            let _ = write!(params, "{name}: bool");
                        }
                        _ => {
                            let _ = write!(params, "{name}: i32");
                        }
                    }
                }
                let ret = match &def.returned {
                    Type::Void => String::new(),
                    Type::Integer(_, _, _) | Type::Character => " -> i32".to_string(),
                    Type::Long => " -> i64".to_string(),
                    Type::Float => " -> f64".to_string(),
                    Type::Single => " -> f32".to_string(),
                    Type::Boolean => " -> bool".to_string(),
                    _ => " -> i32".to_string(),
                };
                writeln!(w, "    safe fn {}({params}){ret};", def.native)?;
            }
            writeln!(w, "}}")?;
        } else {
            // Emit extern crate declarations for native packages.
            for (crate_name, _) in &data.native_packages {
                let ident = crate_name.replace('-', "_");
                writeln!(w, "extern crate {ident};")?;
            }
        }
        writeln!(w, "use loft::database::Stores;")?;
        writeln!(w, "use loft::keys::{{DbRef, Str, Key, Content}};")?;
        writeln!(w, "use loft::ops;")?;
        writeln!(w, "use loft::vector;")?;
        writeln!(w, "use loft::codegen_runtime;")?;
        writeln!(w, "use loft::codegen_runtime::*;")?;
        // The `external::` namespace is used by stdlib #rust templates for rand/random ops.
        // Use codegen_runtime wrappers so no cfg(feature) is needed in generated files.
        writeln!(
            w,
            "mod external {{
    pub fn rand_seed(seed: i64) {{ loft::codegen_runtime::cr_rand_seed(seed); }}
    pub fn rand_int(lo: i32, hi: i32) -> i32 {{ loft::codegen_runtime::cr_rand_int(lo, hi) }}
}}\n"
        )
    }

    /// Use this as the main entry point for native Rust code generation.
    ///
    /// # Errors
    /// Returns an error if any write action to `w` fails.
    pub fn output_native(
        &mut self,
        w: &mut dyn Write,
        from: u32,
        till: u32,
    ) -> std::io::Result<()> {
        Self::emit_file_header(w, self.data, self.wasm_browser)?;
        writeln!(w, "fn init(db: &mut Stores) {{")?;
        self.output_init(w, from, till)?;
        writeln!(w, "    db.finish();\n}}\n")?;
        self.output_functions(w, from, till, None)?;
        self.emit_main_bootstrap(w, till)
    }

    /// Like `output_native`, but only emits functions reachable from `entry_defs`.
    /// Stdlib functions outside `[from, till)` are included if they are transitively
    /// called.  Use this for per-test files so they are self-contained without
    /// emitting the entire stdlib.
    ///
    /// # Errors
    /// Returns an error if any write action to `w` fails.
    pub fn output_native_reachable(
        &mut self,
        w: &mut dyn Write,
        _from: u32,
        till: u32,
        entry_defs: &[u32],
    ) -> std::io::Result<()> {
        let reachable = reachable_functions(self.data, entry_defs);
        self.reachable.clone_from(&reachable);
        Self::emit_file_header(w, self.data, self.wasm_browser)?;
        writeln!(w, "fn init(db: &mut Stores) {{")?;
        // Register ALL types (0..till) so runtime type IDs match compile-time IDs.
        self.output_init(w, 0, till)?;
        writeln!(w, "    db.finish();\n}}\n")?;
        // Emit only reachable functions across the full definition range.
        self.output_functions(w, 0, till, Some(&reachable))?;
        // Emit a Rust entry point that bootstraps the loft `main` function, if present.
        if (0..till).any(|d| self.data.def(d).name == "n_main") {
            if self.wasm_browser {
                // exported cdylib entry point for browser WASM.
                writeln!(
                    w,
                    "\n#[unsafe(no_mangle)]\npub extern \"C\" fn loft_start() {{\n    let mut stores = Stores::new();\n    init(&mut stores);\n    n_main(&mut stores);\n}}"
                )?;
            } else {
                writeln!(
                    w,
                    "\nfn main() {{\n    let mut stores = Stores::new();\n    init(&mut stores);\n    n_main(&mut stores);\n}}"
                )?;
            }
        }
        Ok(())
    }

    /// Emit a Rust `fn main()` bootstrap if the program defines a loft `main` function.
    fn emit_main_bootstrap(&self, w: &mut dyn Write, till: u32) -> std::io::Result<()> {
        let main_nr = self.data.def_nr("n_main");
        if main_nr < till {
            writeln!(
                w,
                "\nfn main() {{\n    let mut stores = Stores::new();\n    init(&mut stores);\n    n_main(&mut stores);\n}}"
            )?;
        }
        Ok(())
    }

    /// Use this to emit only the `init` body that registers all types.
    /// Sorting by `known_type` ensures the runtime recreates type IDs in the same order
    /// as the compile-time database, keeping field indices consistent.
    fn output_init(&mut self, w: &mut dyn Write, from: u32, till: u32) -> std::io::Result<()> {
        let mut type_defs: Vec<(u16, u32)> = Vec::new();
        for dnr in from..till {
            self.start_fn(dnr);
            let def = self.data.def(dnr);
            let type_id = def.known_type;
            let is_enum_value_with_attrs =
                def.def_type == DefType::EnumValue && !def.attributes.is_empty();
            if type_id != u16::MAX
                && (matches!(def.def_type, DefType::Struct)
                    || def.def_type == DefType::Enum
                    || def.def_type == DefType::Vector
                    || is_enum_value_with_attrs)
            {
                type_defs.push((type_id, dnr));
            }
        }
        type_defs.sort_by_key(|(type_id, _)| *type_id);

        // Collect bare Byte/Short types that were registered by ensure_io_type() during
        // file I/O parsing.  These have no corresponding loft definition, so output_init
        // would otherwise skip them entirely, causing runtime type IDs to shift.
        let def_type_id_set: HashSet<u16> = type_defs.iter().map(|&(tid, _)| tid).collect();
        let mut bare_io: Vec<(u16, i32, bool, bool)> = Vec::new(); // (type_id, min, nullable, is_short)
        for (idx, tp) in self.stores.types.iter().enumerate() {
            let tid = idx as u16;
            if def_type_id_set.contains(&tid) {
                continue;
            }
            match &tp.parts {
                crate::database::Parts::Byte(min, nullable) => {
                    bare_io.push((tid, *min, *nullable, false));
                }
                crate::database::Parts::Short(min, nullable) => {
                    bare_io.push((tid, *min, *nullable, true));
                }
                _ => {}
            }
        }
        bare_io.sort_by_key(|&(tid, _, _, _)| tid);
        let mut bare_idx = 0;

        // Build a map from known_type → dnr for dependency resolution.
        let type_id_to_dnr: HashMap<u16, u32> =
            type_defs.iter().map(|&(tid, dnr)| (tid, dnr)).collect();

        // For each struct / enum-value, collect the content type IDs of its
        // sorted / index / hash / vector fields so we can emit them first.
        let mut deps: HashMap<u16, Vec<u16>> = HashMap::new();
        for &(type_id, dnr) in &type_defs {
            let def = self.data.def(dnr);
            let is_container = matches!(def.def_type, DefType::Struct)
                || (def.def_type == DefType::EnumValue && !def.attributes.is_empty());
            if !is_container {
                continue;
            }
            let mut d: Vec<u16> = Vec::new();
            for a in &def.attributes.clone() {
                let c_nr = match &a.typedef {
                    Type::Sorted(c_nr, _, _) | Type::Hash(c_nr, _, _) | Type::Index(c_nr, _, _) => {
                        // Guard matches the Vector convention: skip unresolved (u32::MAX) content types.
                        (*c_nr != u32::MAX).then_some(*c_nr)
                    }
                    Type::Vector(c_type, _) => {
                        let n = self.data.type_def_nr(c_type);
                        (n != u32::MAX).then_some(n)
                    }
                    _ => None,
                };
                if let Some(c_nr) = c_nr {
                    let c_tp = self.data.def(c_nr).known_type;
                    if c_tp != u16::MAX && type_id_to_dnr.contains_key(&c_tp) {
                        d.push(c_tp);
                    }
                }
            }
            if !d.is_empty() {
                deps.insert(type_id, d);
            }
        }

        // Emit type definitions in topological order: dependencies first.
        // Bare byte/short IO types are interleaved at their correct positions.
        let mut emitted: HashSet<u16> = HashSet::new();
        for &(type_id, dnr) in &type_defs {
            // Emit bare byte/short types that must precede this definition.
            while bare_idx < bare_io.len() && bare_io[bare_idx].0 < type_id {
                let (tid, min, nullable, is_short) = bare_io[bare_idx];
                if is_short {
                    writeln!(w, "    db.short({min}, {nullable}); // type {tid}")?;
                } else {
                    writeln!(w, "    db.byte({min}, {nullable}); // type {tid}")?;
                }
                bare_idx += 1;
            }
            self.emit_def_ordered(w, type_id, dnr, &type_id_to_dnr, &deps, &mut emitted)?;
        }
        // Emit any remaining bare byte/short types after all defs.
        while bare_idx < bare_io.len() {
            let (tid, min, nullable, is_short) = bare_io[bare_idx];
            if is_short {
                writeln!(w, "    db.short({min}, {nullable}); // type {tid}")?;
            } else {
                writeln!(w, "    db.byte({min}, {nullable}); // type {tid}")?;
            }
            bare_idx += 1;
        }
        Ok(())
    }

    /// Recursively emit `type_id` (def `dnr`) and its content-type dependencies
    /// before emitting the type itself, so that `db.sorted(c_tp, ...)` etc. always
    /// find the content type already registered.
    fn emit_def_ordered(
        &mut self,
        w: &mut dyn Write,
        type_id: u16,
        dnr: u32,
        type_id_to_dnr: &HashMap<u16, u32>,
        deps: &HashMap<u16, Vec<u16>>,
        emitted: &mut HashSet<u16>,
    ) -> std::io::Result<()> {
        if emitted.contains(&type_id) {
            return Ok(());
        }
        // Mark as emitted before recursing to prevent infinite loops on cycles.
        emitted.insert(type_id);
        // Emit all content-type dependencies first.
        if let Some(d) = deps.get(&type_id) {
            for &dep_tp in d {
                if let (false, Some(&dep_dnr)) =
                    (emitted.contains(&dep_tp), type_id_to_dnr.get(&dep_tp))
                {
                    self.emit_def_ordered(w, dep_tp, dep_dnr, type_id_to_dnr, deps, emitted)?;
                }
            }
        }
        if dnr == u32::MAX {
            eprintln!(
                "codegen warning: skipping type_id={type_id} — definition number is unresolved (u32::MAX)"
            );
            return Ok(());
        }
        let def = self.data.def(dnr);
        if matches!(def.def_type, DefType::Struct) {
            self.output_struct(w, dnr, 0)?;
        } else if def.def_type == DefType::EnumValue && !def.attributes.is_empty() {
            // Determine the 1-based position in the parent enum's attributes.
            let parent_nr = def.parent;
            if parent_nr == u32::MAX {
                eprintln!(
                    "codegen warning: EnumValue '{}' (dnr={dnr}) has no parent — skipping",
                    def.name
                );
                return Ok(());
            }
            let parent = self.data.def(parent_nr);
            let enum_value = parent
                .attributes
                .iter()
                .enumerate()
                .find(|(_, a)| a.name == def.name)
                .map_or(0, |(i, _)| i32::try_from(i).unwrap_or(0) + 1);
            self.output_struct(w, dnr, enum_value)?;
        } else if def.def_type == DefType::Enum {
            output_enum(w, dnr, self.data)?;
        } else if def.def_type == DefType::Vector {
            // Determine the content type's known_type for the db.vector() call.
            // Prefer def.parent (set by check_vector/vector_def), but fall back to
            // extracting the content type from the definition's returned Type when
            // parent is unresolved (u32::MAX) due to cross-source scoping.
            let content_known = if def.parent != u32::MAX {
                self.data.def(def.parent).known_type
            } else if let Type::Vector(ref c_type, _) = def.returned {
                let c_dnr = self.data.type_def_nr(c_type);
                if c_dnr == u32::MAX {
                    u16::MAX
                } else {
                    self.data.def(c_dnr).known_type
                }
            } else {
                u16::MAX
            };
            if content_known != u16::MAX {
                writeln!(w, "    db.vector({content_known});")?;
            }
        }
        Ok(())
    }

    /// Use this to emit all function bodies for the given definition range.
    /// When `reachable` is Some, only functions in the set are emitted.
    fn output_functions(
        &mut self,
        w: &mut dyn Write,
        from: u32,
        till: u32,
        reachable: Option<&HashSet<u32>>,
    ) -> std::io::Result<()> {
        for dnr in from..till {
            if !matches!(self.data.def(dnr).def_type, DefType::Function) {
                continue;
            }
            if let Some(r) = reachable
                && !r.contains(&dnr)
            {
                continue;
            }
            self.output_function(w, dnr)?;
        }
        Ok(())
    }

    /// Use this to emit a single struct field into the db-builder output.
    /// Dispatches on the field's `typedef` to produce the correct `db.*` call.
    fn emit_field(
        &self,
        w: &mut dyn Write,
        field_name: &str,
        typedef: &Type,
        nullable: bool,
        known_type: u16,
    ) -> std::io::Result<()> {
        if let Type::Vector(c, _) = typedef {
            let c_def = self.data.type_def_nr(c);
            if c_def != u32::MAX {
                let content = self.data.def(c_def).known_type;
                emit_db_field(w, "s", field_name, "vec", &format!("db.vector({content})"))?;
            }
            return Ok(());
        }
        if let Type::Integer(min, _, _) = typedef {
            let field_size = typedef.size(nullable);
            if field_size == 1 {
                emit_db_field(
                    w,
                    "s",
                    field_name,
                    "byte",
                    &format!("db.byte({min}, {nullable})"),
                )?;
            } else if field_size == 2 {
                emit_db_field(
                    w,
                    "s",
                    field_name,
                    "short",
                    &format!("db.short({min}, {nullable})"),
                )?;
            } else {
                writeln!(w, "    db.field(s, \"{field_name}\", 0);")?;
            }
            return Ok(());
        }
        if let Type::Sorted(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type;
            let keys_str = keys
                .iter()
                .map(|(k, asc)| format!("(\"{k}\".to_string(), {asc})"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                "s",
                field_name,
                "sorted",
                &format!("db.sorted({c_tp}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        if let Type::Hash(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type;
            let keys_str = keys
                .iter()
                .map(|k| format!("\"{k}\".to_string()"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                "s",
                field_name,
                "hash",
                &format!("db.hash({c_tp}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        if let Type::Index(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type;
            let keys_str = keys
                .iter()
                .map(|(k, asc)| format!("(\"{k}\".to_string(), {asc})"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                "s",
                field_name,
                "index",
                &format!("db.index({c_tp}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        if known_type != u16::MAX {
            writeln!(w, "    db.field(s, \"{field_name}\", {known_type});")?;
        }
        Ok(())
    }

    /// Use this to register one struct or enum-value type in the runtime database.
    /// The runtime field layout must be byte-for-byte identical to the compile-time layout,
    /// so field order and builder calls here must match what the compiler produced.
    ///
    /// `enum_value` is 0 for plain structs and the 1-based variant index for enum-value structs.
    fn output_struct(
        &self,
        w: &mut dyn Write,
        def_nr: u32,
        enum_value: i32,
    ) -> std::io::Result<()> {
        let def = self.data.def(def_nr);
        writeln!(
            w,
            "    let s = db.structure(\"{}\", {enum_value}); // {}",
            def.name, def.known_type
        )?;
        // For EnumValue types, the compile-time DB may have an implicit "enum" discriminator
        // field at position 0 (added when a "byte" type already existed from another struct).
        // If the compile-time type has "enum" at position 0, we must emit it here so that
        // field indices match (content field is at index 1, not 0).
        if enum_value > 0
            && def.known_type != u16::MAX
            && self.stores.position(def.known_type, "enum") == 0
        {
            writeln!(w, "    let byte_enum = db.byte(0, false);")?;
            writeln!(w, "    db.field(s, \"enum\", byte_enum);")?;
        }
        for a in &def.attributes {
            let td_nr = self.data.type_def_nr(&a.typedef);
            let field_type_id = self.data.def(td_nr).known_type;
            assert_ne!(def_nr, u32::MAX, "Unknown def_nr for {:?}", a.typedef);
            self.emit_field(w, &a.name, &a.typedef, a.nullable, field_type_id)?;
        }
        Ok(())
    }

    /// Use this to emit one loft function as a Rust function.
    /// Every loft function receives `stores: &mut Stores` as its first implicit argument.
    fn output_function(&mut self, w: &mut dyn Write, def_nr: u32) -> std::io::Result<()> {
        // Functions implemented in codegen_runtime (imported via `use loft::codegen_runtime::*`).
        // Emitting a stub would shadow the real implementation.
        const CODEGEN_RUNTIME_FNS: &[&str] = &[
            "n_now",
            "n_ticks",
            "n_get_store_lock",
            "n_set_store_lock",
            "n_rand",
            "n_rand_indices",
            "n_parallel_for_native",
            "n_parallel_get_int",
            "n_parallel_get_long",
            "n_parallel_get_float",
            "n_parallel_get_bool",
            "n_parallel_for_ref_native",
            "n_parallel_get_ref",
            "n_path_sep",
            "n_stack_trace",
        ];
        self.start_fn(def_nr);
        let def = self.data.def(def_nr);
        // Skip Op functions with no callable body.
        if def.name.starts_with("Op") && def.code == Value::Null {
            return Ok(());
        }
        // Skip functions implemented in codegen_runtime.
        if def.code == Value::Null && CODEGEN_RUNTIME_FNS.contains(&def.name.as_str()) {
            return Ok(());
        }
        // N8b.1: generator functions (returning iterator<T>) are emitted as state machines.
        if matches!(def.returned, Type::Iterator(_, _)) {
            return self.output_coroutine(w, def_nr);
        }
        // n_assert needs generic Display parameters to accept both Str and &str.
        if def.name == "n_assert" && def.code == Value::Null {
            writeln!(
                w,
                "fn n_assert<M: std::fmt::Display, F: std::fmt::Display>(_s: &mut Stores, test: bool, msg: M, file: F, line: i32) {{"
            )?;
            writeln!(
                w,
                "  if !test {{ panic!(\"{{}}:{{}} {{}}\", file, line, msg); }}"
            )?;
            writeln!(w, "}}\n")?;
            return Ok(());
        }
        write!(w, "fn {}(stores: &mut Stores", def.name)?;
        for a in &def.attributes {
            let tp = rust_type(&a.typedef, &Context::Argument);
            write!(w, ", mut var_{}: {tp}", sanitize(&a.name),)?;
        }
        write!(w, ") ")?;
        if def.returned != Type::Void {
            write!(w, "-> {} ", rust_type(&def.returned, &Context::Result))?;
        }
        // Mark argument variables as already declared so Set won't re-declare them.
        for arg_nr in def.variables.arguments() {
            self.declared.insert(arg_nr);
        }
        // Determine the user-visible loft name for the shadow call stack.
        let loft_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
        let loft_file = &def.position.file;
        let loft_line = def.position.line;
        // Only instrument user-defined functions (Block body, n_ prefix).
        let instrument = matches!(&def.code, Value::Block(_)) && def.name.starts_with("n_");
        let returns_text = matches!(def.returned, Type::Text(_));
        if let Value::Block(bl) = &def.code {
            // An empty-body loft function (explicit stub) has no operators and result Void,
            // but the function signature may still declare a non-void return type.
            // Rust requires an explicit return value in that case, so emit a null default.
            let block_empty = bl.operators.iter().all(|v| matches!(v, Value::Line(_)));
            if block_empty && def.returned != Type::Void {
                writeln!(w, "{{")?;
                writeln!(w, "  {}", default_native_value(&def.returned))?;
                writeln!(w, "}}")?;
            } else if instrument {
                // Emit shadow call stack instrumentation before the block body.
                // The CallGuard drop ensures cr_call_pop on all exit paths (including early return).
                // We emit the push/guard as a prefix inside the block's opening `{`.
                let escaped_file = loft_file.replace('\\', "\\\\");
                self.call_stack_prefix = Some(format!(
                    "  cr_call_push(\"{loft_name}\", \"{escaped_file}\", {loft_line});\n  \
                     let _call_guard = codegen_runtime::CallGuard;"
                ));
                self.output_block(w, bl, returns_text)?;
                self.call_stack_prefix = None;
            } else {
                self.output_block(w, bl, returns_text)?;
            }
        } else if def.code == Value::Null {
            // Native-only function with no loft body.
            // PKG.4: check if this function has a native symbol from a package manifest.
            let user_name = def.name.strip_prefix("n_").unwrap_or(&def.name);
            if let Some(rust_symbol) = self.data.native_symbols.get(user_name) {
                // Emit a call to the native Rust function with type marshalling.
                self.output_native_api_call(w, def_nr, rust_symbol)?;
            } else if !def.native.is_empty() {
                // #native "symbol" — emit direct call with type marshalling.
                if self.wasm_browser {
                    // call the imported function directly (unqualified).
                    // The function is declared in the preamble via
                    // #[link(wasm_import_module = "loft_gl")].
                    self.output_native_direct_call(w, def_nr, &def.native)?;
                } else if let Some(krate) = self.data.native_symbol_crates.get(&def.native) {
                    let qualified = format!("{}::{}", krate, def.native);
                    self.output_native_direct_call(w, def_nr, &qualified)?;
                } else {
                    writeln!(w, "{{")?;
                    if def.returned != Type::Void {
                        writeln!(w, "  todo!(\"native function {}\")", def.name)?;
                    }
                    writeln!(w, "}}")?;
                }
            } else {
                // Internal i_ functions have implementations in codegen_runtime.rs;
                // all others get a todo!() stub.
                writeln!(w, "{{")?;
                if def.name == "i_parse_errors" {
                    writeln!(w, "  loft::codegen_runtime::i_parse_errors(stores)")?;
                } else if def.name == "i_parse_error_push" {
                    writeln!(
                        w,
                        "  loft::codegen_runtime::i_parse_error_push(stores, var_msg)"
                    )?;
                } else if def.returned != Type::Void {
                    writeln!(w, "  todo!(\"native function {}\")", def.name)?;
                }
                writeln!(w, "}}")?;
            }
        } else {
            writeln!(w, "{{")?;
            self.output_code_inner(w, &def.code)?;
            writeln!(w, "\n}}")?;
        }
        writeln!(w, "\n")
    }

    /// PKG.4: emit a call to an external native Rust function from a package.
    /// The generated code calls `rust_symbol(stores, arg1, arg2, ...)` and
    /// returns the result.
    fn output_native_api_call(
        &self,
        w: &mut dyn Write,
        d_nr: u32,
        rust_symbol: &str,
    ) -> std::io::Result<()> {
        let def = self.data.def(d_nr);
        writeln!(w, "{{")?;
        write!(w, "  {rust_symbol}(stores")?;
        for attr in &def.attributes {
            if attr.name.starts_with("__") {
                continue;
            }
            write!(w, ", var_{}", sanitize(&attr.name))?;
        }
        write!(w, ")")?;
        writeln!(w, "\n}}")
    }

    /// Emit a direct call to a native `extern "C"` function with automatic
    /// type marshalling derived from the loft function signature.
    ///
    /// Conversions:
    /// - `text` (`&str`) → `ptr, len` (two C args)
    /// - `vector<T>` → `(*const ELEM_TYPE, count: u32)` pair via direct store access
    /// - `text` → `(ptr, len)` pointer pair
    /// - scalars pass through with casts where needed
    ///
    /// `vector<T>` args never use `LoftStore`/`LoftRef`.  Instead the codegen
    /// extracts the raw element pointer and count from the store's memory buffer
    /// directly.  This avoids the E0308 "two different loft_ffi" error that arises
    /// when loft and the native package are compiled as separate Cargo projects.
    ///
    /// Native functions that take `vector<T>` args must declare their C signature
    /// with `(*const ELEM_TYPE, count: u32)` pairs in place of each vector argument
    /// (no `LoftStore` or `LoftRef` involved).
    ///
    /// The return value is converted back to the loft type.
    fn output_native_direct_call(
        &self,
        w: &mut dyn Write,
        d_nr: u32,
        qualified_symbol: &str,
    ) -> std::io::Result<()> {
        let def = self.data.def(d_nr);
        writeln!(w, "{{")?;

        // Pre-declare per-vector extraction variables before the call expression
        // so that raw pointers are stable for the duration of the unsafe block.
        for attr in &def.attributes {
            if attr.name.starts_with("__") {
                continue;
            }
            if let Type::Vector(elem_tp, _) = &attr.typedef {
                let var = sanitize(&attr.name);
                let elem = Self::vector_elem_rust_type(elem_tp);
                writeln!(
                    w,
                    "  let _vr_{var} = loft::keys::store(&var_{var}, &stores.allocations).get_int(var_{var}.rec, var_{var}.pos) as u32;"
                )?;
                writeln!(
                    w,
                    "  let _vc_{var} = if _vr_{var} == 0 {{ 0u32 }} else {{ loft::keys::store(&var_{var}, &stores.allocations).get_int(_vr_{var}, 4) as u32 }};"
                )?;
                writeln!(
                    w,
                    "  let _vp_{var}: *const {elem} = if _vr_{var} == 0 {{ std::ptr::null() }} else {{ loft::keys::store(&var_{var}, &stores.allocations).addr::<{elem}>(_vr_{var}, 8) as *const {elem} }};"
                )?;
            }
        }

        let needs_ret_cast = matches!(&def.returned, Type::Integer(_, _, _));
        if needs_ret_cast {
            write!(w, "  (unsafe {{ {qualified_symbol}(")?;
        } else {
            write!(w, "  unsafe {{ {qualified_symbol}(")?;
        }
        let mut first = true;
        for attr in &def.attributes {
            if attr.name.starts_with("__") {
                continue;
            }
            let var = sanitize(&attr.name);
            match &attr.typedef {
                Type::Text(_) => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var}.as_ptr(), var_{var}.len()")?;
                }
                Type::Vector(_, _) => {
                    // Pass as (*const ELEM_TYPE, count: u32) — no LoftStore/LoftRef.
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "_vp_{var}, _vc_{var}")?;
                }
                Type::Integer(_, _, _) | Type::Character => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var} as _")?;
                }
                Type::Float => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var}")?;
                }
                Type::Boolean => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var}")?;
                }
                _ => {
                    if !first {
                        write!(w, ", ")?;
                    }
                    first = false;
                    write!(w, "var_{var}")?;
                }
            }
        }
        if needs_ret_cast {
            write!(w, ") }}) as i32")?;
        } else {
            write!(w, ") }}")?;
        }
        writeln!(w, "\n}}")
    }

    /// Map a loft vector element type to the Rust primitive type used for the
    /// raw-pointer calling convention.
    ///
    /// Native functions that accept `vector<T>` args receive a `*const ELEM_TYPE`
    /// pointer.  This function returns the Rust type name for each loft element type.
    fn vector_elem_rust_type(tp: &Type) -> &'static str {
        match tp {
            Type::Single => "f32",
            Type::Float => "f64",
            Type::Long => "i64",
            Type::Boolean => "u8",
            Type::Character => "u32",
            // Integer: loft uses i32 as the canonical runtime integer type.
            Type::Integer(_, _, _) => "i32",
            // Fallback for struct/enum elements: opaque bytes.
            _ => "u8",
        }
    }
}

fn emit_db_field(
    w: &mut dyn Write,
    struct_var: &str,
    field_name: &str,
    prefix: &str,
    builder: &str,
) -> std::io::Result<()> {
    let var = format!("{prefix}_{}", sanitize(field_name));
    writeln!(w, "    let {var} = {builder};")?;
    writeln!(w, "    db.field({struct_var}, \"{field_name}\", {var});")?;
    Ok(())
}

/// Use this to register an enum in the runtime database.
/// Plain tag variants are registered with `u16::MAX`; struct-enum variants use the variant
/// struct's `known_type` so that `ShowDb` can dispatch to the variant's fields.
fn output_enum(w: &mut dyn Write, d_nr: u32, data: &Data) -> std::io::Result<()> {
    let def = data.def(d_nr);
    writeln!(w, "    let e = db.enumerate(\"{}\");", def.name)?;
    for a in &def.attributes {
        let variant_type = if matches!(a.typedef, Type::Enum(_, true, _)) {
            // Find the EnumValue definition whose parent is this enum and name matches.
            (0..data.definitions())
                .find(|&v| {
                    let v_def = data.def(v);
                    v_def.def_type == DefType::EnumValue
                        && v_def.parent == d_nr
                        && v_def.name == a.name
                })
                .map_or(u16::MAX, |v| data.def(v).known_type)
        } else {
            u16::MAX
        };
        if variant_type == u16::MAX {
            writeln!(w, "    db.value(e, \"{}\", u16::MAX);", a.name)?;
        } else {
            writeln!(w, "    db.value(e, \"{}\", {variant_type}_u16);", a.name)?;
        }
    }
    Ok(())
}
