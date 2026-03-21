// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use crate::data::{Block, Context, Data, DefType, Definition, Type, Value};
use crate::database::Stores;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;


/// Walk the Value IR tree and collect all function definition numbers
/// referenced by `Value::Call(def_nr, _)` nodes.
fn collect_calls(val: &Value, calls: &mut HashSet<u32>) {
    match val {
        Value::Call(d, args) => {
            calls.insert(*d);
            for a in args {
                collect_calls(a, calls);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                collect_calls(op, calls);
            }
        }
        Value::If(test, t, f) => {
            collect_calls(test, calls);
            collect_calls(t, calls);
            collect_calls(f, calls);
        }
        Value::Set(_, v) | Value::Return(v) | Value::Drop(v) => collect_calls(v, calls),
        Value::Insert(ops) => {
            for op in ops {
                collect_calls(op, calls);
            }
        }
        Value::Iter(_, create, next, extra) => {
            collect_calls(create, calls);
            collect_calls(next, calls);
            collect_calls(extra, calls);
        }
        _ => {}
    }
}

/// Compute the set of function definitions reachable from `entry_defs` via
/// transitive calls.  Returns the full reachable set including `entry_defs`.
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
        collect_calls(&def.code, &mut calls);
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
        Type::Integer(from, to)
            if i64::from(*to) - i64::from(*from) <= 255 && i64::from(*from) >= 0 =>
        {
            Some("u8")
        }
        Type::Integer(from, to)
            if i64::from(*to) - i64::from(*from) <= 65536 && i64::from(*from) >= 0 =>
        {
            Some("u16")
        }
        Type::Integer(from, to) if i64::from(*to) - i64::from(*from) <= 255 => Some("i8"),
        Type::Integer(from, to) if i64::from(*to) - i64::from(*from) <= 65536 => Some("i16"),
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
        Type::Integer(from, to)
            if context == &Context::Result
                && i64::from(*to) - i64::from(*from) <= 255
                && i64::from(*from) >= 0 =>
        {
            "u8"
        }
        Type::Integer(from, to)
            if context == &Context::Result
                && i64::from(*to) - i64::from(*from) <= 65536
                && i64::from(*from) >= 0 =>
        {
            "u16"
        }
        Type::Integer(from, to)
            if context == &Context::Result
                && i64::from(*to) - i64::from(*from) <= 255 =>
        {
            "i8"
        }
        Type::Integer(from, to)
            if context == &Context::Result
                && i64::from(*to) - i64::from(*from) <= 65536 =>
        {
            "i16"
        }
        Type::Enum(_, false, _) => "u8",
        Type::Integer(_, _) | Type::Character => "i32",
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
        | Type::Index(_, _, _) => "DbRef",
        Type::Routine(_) | Type::Function(_, _) => "u32",
        Type::Unknown(_) => "??",
        Type::Iterator(_, _) => "Iterator",
        Type::Keys => "&[Key]",
        _ => panic!("Incorrect type {tp:?}"),
    }
    .to_string()
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
#![allow(clippy::double_parens)]
#![allow(clippy::unused_unit)]

extern crate loft;"
        )?;
        writeln!(w, "use loft::database::Stores;")?;
        writeln!(w, "use loft::keys::{{DbRef, Str, Key, Content}};")?;
        writeln!(w, "use loft::ops;")?;
        writeln!(w, "use loft::vector;")?;
        writeln!(w, "use loft::codegen_runtime::*;\n")?;
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
#![allow(clippy::double_parens)]
#![allow(clippy::unused_unit)]

extern crate loft;"
        )?;
        writeln!(w, "use loft::database::Stores;")?;
        writeln!(w, "use loft::keys::{{DbRef, Str, Key, Content}};")?;
        writeln!(w, "use loft::ops;")?;
        writeln!(w, "use loft::vector;")?;
        writeln!(w, "use loft::codegen_runtime::*;\n")?;
        writeln!(w, "fn init(db: &mut Stores) {{")?;
        // Register ALL types (0..till) so runtime type IDs match compile-time IDs.
        self.output_init(w, 0, till)?;
        writeln!(w, "    db.finish();\n}}\n")?;
        // Emit only reachable functions across the full definition range.
        self.output_functions(w, 0, till, Some(&reachable))?;
        // Emit a Rust `main` that bootstraps the loft `main` function, if present.
        if (0..till).any(|d| self.data.def(d).name == "n_main") {
            writeln!(
                w,
                "\nfn main() {{\n    let mut stores = Stores::new();\n    init(&mut stores);\n    n_main(&mut stores);\n}}"
            )?;
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
                        Some(*c_nr)
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
        let mut emitted: HashSet<u16> = HashSet::new();
        for &(type_id, dnr) in &type_defs {
            self.emit_def_ordered(w, type_id, dnr, &type_id_to_dnr, &deps, &mut emitted)?;
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
        let def = self.data.def(dnr);
        if matches!(def.def_type, DefType::Struct) {
            self.output_struct(w, dnr, 0)?;
        } else if def.def_type == DefType::EnumValue && !def.attributes.is_empty() {
            // Determine the 1-based position in the parent enum's attributes.
            let parent_nr = def.parent;
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
            writeln!(
                w,
                "    db.vector({});",
                self.data.def(def.parent).known_type
            )?;
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
        if let Type::Integer(min, _) = typedef {
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
        self.start_fn(def_nr);
        let def = self.data.def(def_nr);
        // Skip Op functions with no callable body.
        if def.name.starts_with("Op") && def.code == Value::Null {
            return Ok(());
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
        let returns_text = matches!(def.returned, Type::Text(_));
        if let Value::Block(bl) = &def.code {
            self.output_block(w, bl, returns_text)?;
        } else if def.code == Value::Null {
            // Native-only function with no loft body — emit a todo!() stub.
            writeln!(w, "{{")?;
            if def.returned != Type::Void {
                writeln!(w, "  todo!(\"native function {}\")", def.name)?;
            }
            writeln!(w, "}}")?;
        } else {
            writeln!(w, "{{")?;
            self.output_code_inner(w, &def.code)?;
            writeln!(w, "\n}}")?;
        }
        writeln!(w, "\n")
    }

    /// Use this instead of emitting an argument block when the block exists only to pass a
    /// local text variable by mutable reference. Returns the variable index so the call site
    /// can emit `&mut var_<name>` without generating a spurious empty block expression.
    fn create_stack_var(&self, v: &Value) -> Option<u16> {
        // Direct OpCreateStack call on a text variable: `fn f(x: &text)` called as `f(txt)`.
        // The parser wraps the argument as Value::Call("OpCreateStack", [Value::Var(n)]).
        // output_call emits nothing for OpCreateStack, so we must intercept here and emit
        // `&mut var_<name>` instead.
        if let Value::Call(d_nr, args) = v
            && self.data.def(*d_nr).name == "OpCreateStack"
            && let [Value::Var(nr)] = args.as_slice()
            && matches!(self.data.def(self.def_nr).variables.tp(*nr), Type::Text(_))
        {
            return Some(*nr);
        }
        let Value::Block(bl) = v else { return None };
        // Handle DbRef-stack refs: Type::Reference with OpCreateStack ops.
        if let Type::Reference(_, vars) = &bl.result {
            let [vr] = vars.as_slice() else { return None };
            let only_create_stack = bl
                .operators
                .iter()
                .filter(|op| !matches!(op, Value::Line(_)))
                .all(|op| matches!(op, Value::Call(d_nr, _) if self.data.def(*d_nr).name == "OpCreateStack"));
            return only_create_stack.then_some(*vr);
        }
        None
    }

    /// Fix the "hoisted return value" pattern inserted by `scopes::free_vars`.
    ///
    /// When a function returns early (`return expr`) and has local text/ref variables
    /// that need cleanup, `scopes::free_vars` transforms the return into:
    ///   `[expr, OpFreeText(v)…, Return(Null)]`
    /// so the interpreter can push `expr` onto the stack before freeing locals and returning.
    ///
    /// In native Rust code, `OpFreeText` is a no-op (Rust drops automatically), so the
    /// pattern degenerates to `expr; return ();` which drops the return value and fails to
    /// compile when the function return type is not void.
    ///
    /// This method detects the pattern in a slice of block operators and returns a patched
    /// copy where `Return(Null)` is replaced by `Return(expr)` and `expr` is removed from
    /// its earlier position.
    fn patch_hoisted_returns<'a>(&self, ops: &'a [Value]) -> std::borrow::Cow<'a, [Value]> {
        let fn_returned = &self.data.def(self.def_nr).returned;
        if matches!(fn_returned, Type::Void) {
            return std::borrow::Cow::Borrowed(ops);
        }
        // Quick check: is there any Return(Null) at all?
        if !ops
            .iter()
            .any(|op| matches!(op, Value::Return(v) if **v == Value::Null))
        {
            return std::borrow::Cow::Borrowed(ops);
        }
        let is_free_op = |op: &Value| {
            if let Value::Call(d, _) = op {
                let name = &self.data.def(*d).name;
                name == "OpFreeText" || name == "OpFreeRef"
            } else {
                false
            }
        };
        let mut result: Vec<Value> = ops.to_vec();
        // Process all Return(Null) occurrences (usually just one).
        let mut search_from = 0;
        while let Some(ret_pos) = result[search_from..]
            .iter()
            .position(|op| matches!(op, Value::Return(v) if **v == Value::Null))
            .map(|p| p + search_from)
        {
            // Find the nearest preceding expression that is not a free-op, Line, or Return.
            let expr_pos = result[..ret_pos]
                .iter()
                .rposition(|op| !matches!(op, Value::Line(_)) && !is_free_op(op));
            if let Some(idx) = expr_pos {
                let expr = result.remove(idx);
                // ret_pos shifted by -1 because we removed one element before it.
                let actual_ret = ret_pos - 1;
                result[actual_ret] = Value::Return(Box::new(expr));
                search_from = actual_ret + 1;
            } else {
                search_from = ret_pos + 1;
            }
        }
        std::borrow::Cow::Owned(result)
    }

    /// Use this to detect sub-expressions that would cause a double-borrow of `stores`
    /// if left inline and must therefore be hoisted into `let _preN` bindings.
    /// Returns true if the named native Op function uses `stores` in its special-case emit code.
    /// These functions need pre-eval treatment to avoid double-borrow of `stores` when they
    /// appear as arguments inside other stores-using calls.
    fn op_uses_stores(name: &str) -> bool {
        matches!(
            name,
            "OpNewRecord"
                | "OpFinishRecord"
                | "OpGetRecord"
                | "OpIterate"
                | "OpDatabase"
                | "OpCopyRecord"
                | "OpSizeofRef"
                | "OpStep"
                | "OpRemove"
                | "OpHashRemove"
                | "OpAppendCopy"
                | "OpFormatDatabase"
                | "OpFormatStackDatabase"
        )
    }

    fn needs_pre_eval(&self, v: &Value) -> bool {
        match v {
            Value::Call(d_nr, vals) => {
                let def = self.data.def(*d_nr);
                // User-defined functions (rust template is empty AND have loft code body)
                // always need pre-eval to avoid double-borrow.
                if def.rust.is_empty() && def.code != Value::Null {
                    true
                } else if def.rust.contains("stores") {
                    // Template fns that use `stores` can cause double-borrow when nested
                    // inside another stores-using call; treat them as needing pre-eval.
                    true
                } else if def.rust.is_empty() && Self::op_uses_stores(&def.name) {
                    // Native Op functions whose special-case emit code passes `stores`
                    // also cause double-borrow when nested inside other stores-using calls.
                    true
                } else if def.rust.is_empty()
                    && def.code == Value::Null
                    && !def.name.starts_with("Op")
                {
                    // User-fn stubs (no rust template, no loft body, not a built-in Op)
                    // are emitted as todo!() but still take `&mut Stores` — pre-eval
                    // them to avoid double-borrow when they appear as nested arguments.
                    true
                } else {
                    vals.iter().any(|a| self.needs_pre_eval(a))
                }
            }
            Value::Block(_) => true,
            Value::If(test, t, f) => {
                self.needs_pre_eval(test) || self.needs_pre_eval(t) || self.needs_pre_eval(f)
            }
            Value::Drop(v) => self.needs_pre_eval(v),
            _ => false,
        }
    }

    /// Use this when you need the generated text of an expression for substitution or comparison,
    /// rather than writing it directly to the output stream.
    fn generate_expr_buf(&mut self, v: &Value) -> std::io::Result<String> {
        let mut buf = std::io::BufWriter::new(Vec::new());
        self.output_code_inner(&mut buf, v)?;
        Ok(String::from_utf8(buf.into_inner()?).unwrap())
    }

    /// Use this to identify all sub-expressions in `v` that must be hoisted before the enclosing
    /// expression to prevent simultaneous `&mut Stores` borrows.
    /// Returns `(var_name, expr_code)` pairs ordered innermost-first so each pre-eval
    /// can safely reference earlier ones.
    fn collect_pre_evals(&mut self, v: &Value) -> std::io::Result<Vec<(String, String, u32, bool)>> {
        let mut result = Vec::new();
        self.collect_pre_evals_inner(v, &mut result)?;
        Ok(result)
    }

    /// Use this as the recursive worker for `collect_pre_evals`.
    /// Splitting from the wrapper keeps the result `Vec` allocated once, and the pre-eval
    ///  counter is globally unique within a block.
    fn collect_pre_evals_inner(
        &mut self,
        v: &Value,
        result: &mut Vec<(String, String, u32, bool)>,
    ) -> std::io::Result<()> {
        // Recurse into wrapper nodes so nested Call nodes inside Set/Drop/If are found.
        if let Value::Set(_, rhs) = v {
            return self.collect_pre_evals_inner(rhs, result);
        }
        if let Value::Drop(inner) = v {
            return self.collect_pre_evals_inner(inner, result);
        }
        if let Value::If(test, true_v, false_v) = v {
            self.collect_pre_evals_inner(test, result)?;
            self.collect_pre_evals_inner(true_v, result)?;
            return self.collect_pre_evals_inner(false_v, result);
        }
        if let Value::Call(d_nr, vals) = v {
            let def_fn = self.data.def(*d_nr);
            if def_fn.rust.is_empty() {
                // User-defined function: pre-eval any Block or nested user-fn arguments
                // (both cause double-borrow of stores if left inline).
                for arg in vals {
                    let needs_pre = self.create_stack_var(arg).is_none()
                        && (matches!(arg, Value::Block(_)) || self.needs_pre_eval(arg));
                    if needs_pre {
                        let name = format!("_pre_{}", self.counter);
                        self.counter += 1;
                        self.rewrite_code(result, arg, name, false)?;
                    } else {
                        self.collect_pre_evals_inner(arg, result)?;
                    }
                }
            } else {
                // Template function: pre-eval Block args (they may use stores) and,
                // when multiple user-fn args exist, pre-eval those too to avoid
                // double-borrow of stores.
                let block_count = vals.iter().filter(|a| matches!(a, Value::Block(_))).count();
                let user_fn_count = vals.iter().filter(|a| self.needs_pre_eval(a)).count();
                // Pre-eval when the template uses stores itself (any user-fn arg causes conflict)
                // or when multiple user-fn args exist (they'd conflict with each other).
                let template_uses_stores = def_fn.rust.contains("stores");
                // Also pre-eval any arg whose template placeholder appears more than once
                // (e.g., `#rust"!@v1.is_nan() && ... @v1 ..."` expands @v1 twice, causing
                // double-borrow when @v1 is a user-fn call returning stores-backed data).
                let has_dup_param = def_fn
                    .attributes
                    .iter()
                    .enumerate()
                    .any(|(i, a)| {
                        let placeholder = format!("@{}", a.name);
                        i < vals.len()
                            && def_fn.rust.matches(placeholder.as_str()).count() > 1
                            && self.needs_pre_eval(&vals[i])
                    });
                let needs_pre_eval_args = block_count > 0
                    || user_fn_count > 1
                    || (template_uses_stores && user_fn_count > 0)
                    || has_dup_param;
                if needs_pre_eval_args {
                    for (arg_idx, arg) in vals.iter().enumerate() {
                        let is_block = matches!(arg, Value::Block(_));
                        let is_multi_user_fn = user_fn_count > 1 && self.needs_pre_eval(arg);
                        let is_stores_conflict =
                            template_uses_stores && self.needs_pre_eval(arg);
                        let is_dup = if arg_idx < def_fn.attributes.len() {
                            let placeholder =
                                format!("@{}", def_fn.attributes[arg_idx].name);
                            def_fn.rust.matches(placeholder.as_str()).count() > 1
                                && self.needs_pre_eval(arg)
                        } else {
                            false
                        };
                        if is_block || is_multi_user_fn || is_stores_conflict || is_dup {
                            let name = format!("_pre_{}", self.counter);
                            self.counter += 1;
                            self.rewrite_code(result, arg, name, is_dup)?;
                        } else {
                            self.collect_pre_evals_inner(arg, result)?;
                        }
                    }
                } else {
                    for arg in vals {
                        self.collect_pre_evals_inner(arg, result)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Use this to register one pre-eval binding: generate the expression text with inner
    /// pre-evals already substituted, then push `(name, code)` onto `result`.
    fn rewrite_code(
        &mut self,
        result: &mut Vec<(String, String, u32, bool)>,
        arg: &Value,
        name: String,
        replace_all: bool,
    ) -> std::io::Result<()> {
        // Collect inner pre-evals first, so the pre-eval code itself
        // is free of double borrows.
        let decl_clone = self.declared.clone();
        let start_idx = result.len();
        self.collect_pre_evals_inner(arg, result)?;
        // Propagate replace_all flag: if this pre-eval is a dup-param (replace_all=true),
        // all its inner pre-evals must also use replace_all so that progressive substitution
        // correctly transforms all N occurrences of the dup arg in the outer expression.
        if replace_all {
            for entry in &mut result[start_idx..] {
                entry.3 = true;
            }
        }
        let inner_pre_evals = result[start_idx..].to_vec();
        // Save counter state before generating the expression text;
        // output_block will restore to this value before output_code_with_subst
        // so the block inner pre-eval names (_pre_N) match in both passes.
        let counter_before_gen = self.counter;
        let raw_code = self.generate_expr_buf(arg)?;
        let substituted = if inner_pre_evals.is_empty() {
            raw_code
        } else {
            let mut s = raw_code;
            for (pre_name, pre_code, _, inner_replace_all) in &inner_pre_evals {
                if *inner_replace_all {
                    // Dup-param inner pre-eval: the arg code appears multiple times
                    // in the binding code (template expanded @v1 twice), replace all.
                    s = s.replace(pre_code.as_str(), pre_name.as_str());
                } else {
                    // Normal inner pre-eval: appears once, use replace-first.
                    s = s.replacen(pre_code.as_str(), pre_name.as_str(), 1);
                }
            }
            s
        };
        if !substituted.is_empty() && substituted != "()" {
            result.push((name, substituted, counter_before_gen, replace_all));
        }
        self.declared = decl_clone;
        Ok(())
    }

    /// Use this instead of `output_code_inner` when `pre_evals` is non-empty.
    /// Without substitution the same expression would be emitted twice, causing a second
    /// mutable borrow of `stores`.
    fn output_code_with_subst(
        &mut self,
        w: &mut dyn Write,
        v: &Value,
        pre_evals: &[(String, String, u32, bool)],
    ) -> std::io::Result<()> {
        if pre_evals.is_empty() {
            self.output_code_inner(w, v)?;
            return Ok(());
        }
        // For If expressions, apply substitution structurally rather than via string
        // replacement on the full generated text.  String-level substitution on the full
        // if-else tree corrupts the `let _pre_N = …;` declarations that inner Block
        // branches emit for their own operators: those declarations contain the same
        // raw code strings (e.g. `get_int_8_code`) as the outer pre-evals, so a
        // replacen call intended for the outer condition accidentally replaces the inner
        // declaration, making the inner variable a stale alias of the outer pre-eval.
        //
        // The structural fix: apply substitution only to the *condition* part of the If
        // (and recursively to any else-if conditions); emit Block branches directly via
        // `output_code_inner`, which calls `output_block` and manages their own
        // pre-evals internally.
        if let Value::If(test, true_v, false_v) = v {
            // Check exact match first: if this entire If expression equals a pre-eval
            // binding, emit the name.  Save/restore counter and declared so the check
            // pass does not corrupt state for the real structural emission below.
            let saved_counter = self.counter;
            let saved_declared = self.declared.clone();
            let mut check_buf = std::io::BufWriter::new(Vec::new());
            self.output_code_inner(&mut check_buf, v)?;
            let full_code = String::from_utf8(check_buf.into_inner()?).unwrap();
            self.counter = saved_counter;
            self.declared = saved_declared;
            for (name, pre_code, _, _) in pre_evals {
                if full_code == *pre_code {
                    write!(w, "{name}")?;
                    return Ok(());
                }
            }
            return self.output_if_with_subst(w, test, true_v, false_v, pre_evals);
        }
        let mut buf_check = std::io::BufWriter::new(Vec::new());
        self.output_code_inner(&mut buf_check, v)?;
        let code = String::from_utf8(buf_check.into_inner()?).unwrap();
        for (name, pre_code, _, _) in pre_evals {
            if code == *pre_code {
                write!(w, "{name}")?;
                return Ok(());
            }
        }
        let mut result = code;
        for (name, pre_code, _, replace_all) in pre_evals {
            if *replace_all {
                // Dup-param: the same arg code appears multiple times in a template
                // expansion; replace all occurrences so the pre-eval is used everywhere.
                result = result.replace(pre_code.as_str(), name.as_str());
            } else {
                // Normal pre-eval: use replace-first so that identical code strings
                // inside nested block pre-eval declarations are NOT substituted.
                // Multiple pre-evals with the same binding code (one per usage site)
                // are generated by the caller and each replaces exactly one occurrence.
                result = result.replacen(pre_code.as_str(), name.as_str(), 1);
            }
        }
        write!(w, "{result}")?;
        Ok(())
    }

    /// Use this to emit an `if`/`else` expression with pre-eval substitution applied
    /// structurally: the condition receives substitution, Block branches are emitted
    /// directly (they handle their own pre-evals via `output_block`), and non-Block
    /// branches (else-if chains) receive substitution recursively.
    fn output_if_with_subst(
        &mut self,
        w: &mut dyn Write,
        test: &Value,
        true_v: &Value,
        false_v: &Value,
        pre_evals: &[(String, String, u32, bool)],
    ) -> std::io::Result<()> {
        write!(w, "if ")?;
        let b_true = matches!(*true_v, Value::Block(_));
        let b_false = matches!(*false_v, Value::Block(_));
        // Condition: apply substitution (this is exactly what the pre-evals are for).
        self.output_code_with_subst(w, test, pre_evals)?;
        if b_true {
            write!(w, " ")?;
        } else {
            write!(w, " {{")?;
        }
        self.indent += u32::from(!b_true);
        if b_true {
            // Block branch: manages its own pre-evals, no outer substitution needed.
            self.output_code_inner(w, true_v)?;
        } else {
            self.output_code_with_subst(w, true_v, pre_evals)?;
        }
        self.indent -= u32::from(!b_true);
        if let Value::Block(_) = *true_v {
            write!(w, " else ")?;
        } else {
            write!(w, "}} else ")?;
        }
        if !b_false {
            write!(w, "{{")?;
        }
        self.indent += u32::from(!b_false);
        if matches!(false_v, Value::Null) && let Some(tp) = self.infer_type(true_v) {
            Self::write_typed_null(w, &tp)?;
        } else if b_false {
            // Block branch: manages its own pre-evals, no outer substitution needed.
            self.output_code_inner(w, false_v)?;
        } else {
            // Non-block false branch (else-if chain or leaf): apply substitution.
            self.output_code_with_subst(w, false_v, pre_evals)?;
        }
        self.indent -= u32::from(!b_false);
        if !b_false {
            write!(w, "}}")?;
        }
        Ok(())
    }

    /// Use this to determine whether a value produces no Rust result (type `()`).
    /// Needed by `output_block` to find the last non-void expression that should be the
    /// block's return value.
    fn is_void_value(&self, v: &Value) -> bool {
        match v {
            Value::Null | Value::Drop(_) | Value::Set(_, _) => true,
            Value::If(_, _, false_v) => matches!(**false_v, Value::Null),
            Value::Call(d_nr, _) => {
                let def = self.data.def(*d_nr);
                matches!(def.returned, Type::Void)
            }
            Value::Block(bl) => matches!(bl.result, Type::Void),
            _ => false,
        }
    }

    /// Central recursive dispatch from a `Value` node to its Rust representation.
    /// All emit functions ultimately call this; complex variants are delegated to
    /// dedicated helpers to keep each match arm concise.
    fn output_code_inner(&mut self, w: &mut dyn Write, code: &Value) -> std::io::Result<()> {
        match code {
            Value::Text(txt) => {
                // Use debug format to produce a properly escaped Rust string literal.
                write!(w, "{txt:?}")?;
            }
            Value::Long(v) => write!(w, "{v}_i64")?,
            Value::Int(v) => write!(w, "{v}_i32")?,
            Value::Enum(v, _) => write!(w, "{v}_u8")?,
            Value::Boolean(v) => write!(w, "{v}")?,
            Value::Float(v) => write!(w, "{v}_f64")?,
            Value::Single(v) => write!(w, "{v}_f32")?,
            Value::Null => write!(w, "()")?,
            Value::Line(_) => {}
            Value::Break(_) => write!(w, "break")?,
            Value::Continue(_) => write!(w, "continue")?,
            Value::Drop(v) => self.output_code_inner(w, v)?,
            Value::Insert(ops) => {
                for (vnr, v) in ops.iter().enumerate() {
                    self.indent(w)?;
                    self.indent += 1;
                    self.output_code_inner(w, v)?;
                    self.indent -= 1;
                    if vnr < ops.len() - 1 {
                        writeln!(w, ";")?;
                    } else {
                        writeln!(w)?;
                    }
                }
            }
            Value::Block(bl) => self.output_block(w, bl, false)?,
            Value::Loop(lp) => {
                writeln!(w, "loop {{ //{}_{}", lp.name, lp.scope)?;
                for v in &lp.operators {
                    self.indent(w)?;
                    self.indent += 1;
                    self.output_code_inner(w, v)?;
                    self.indent -= 1;
                    writeln!(w, ";")?;
                }
                self.indent(w)?;
                write!(w, "}} /*{}_{}*/", lp.name, lp.scope)?;
            }
            Value::Set(var, to) => self.output_set(w, *var, to)?,
            Value::Var(var) => {
                let variables = &self.data.def(self.def_nr).variables;
                let text_var = if matches!(variables.tp(*var), Type::Text(_)) {
                    // Text params are `&str` — already a reference, don't add `&`
                    // Text locals are `String` — add `&` to coerce to `&str`
                    if variables.is_argument(*var) { "" } else { "&" }
                } else {
                    ""
                };
                write!(
                    w,
                    "{text_var}var_{}",
                    sanitize(self.data.def(self.def_nr).variables.name(*var))
                )?;
            }
            Value::If(test, true_v, false_v) => self.output_if(w, test, true_v, false_v)?,
            Value::Call(def_nr, vals) => {
                self.output_call(w, *def_nr, vals)?;
            }
            Value::Return(val) => {
                let returned = &self.data.def(self.def_nr).returned;
                let returns_text = matches!(returned, Type::Text(_));
                let narrow = narrow_int_cast(returned);
                write!(w, "return ")?;
                if returns_text {
                    write!(w, "Str::new(")?;
                } else if narrow.is_some() {
                    write!(w, "(")?;
                }
                self.output_code_inner(w, val)?;
                if returns_text {
                    write!(w, ")")?;
                } else if let Some(cast) = narrow {
                    write!(w, ") as {cast}")?;
                }
            }
            Value::Keys(keys) => {
                write!(w, "&[")?;
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        write!(w, ", ")?;
                    }
                    write!(
                        w,
                        "Key {{ type_nr: {}, position: {} }}",
                        k.type_nr, k.position
                    )?;
                }
                write!(w, "]")?;
            }
            _ => write!(w, "{code:?}")?,
        }
        Ok(())
    }

    /// Use this to emit an `if/else` expression. Handles whether branches are bare
    /// blocks (no extra braces needed) or single expressions (braces required).
    /// Infer the result type of an expression for generating typed null defaults.
    fn infer_type(&self, v: &Value) -> Option<Type> {
        match v {
            Value::Int(_) => Some(Type::Integer(i32::MIN + 1, i32::MAX as u32)),
            Value::Long(_) => Some(Type::Long),
            Value::Float(_) => Some(Type::Float),
            Value::Single(_) => Some(Type::Single),
            Value::Boolean(_) => Some(Type::Boolean),
            Value::Text(_) => Some(Type::Text(Vec::new())),
            Value::Enum(_, tp) => Some(Type::Enum(u32::from(*tp), false, Vec::new())),
            Value::Var(nr) => Some(self.data.def(self.def_nr).variables.tp(*nr).clone()),
            Value::Call(d, _) => {
                let ret = &self.data.def(*d).returned;
                (*ret != Type::Void).then(|| ret.clone())
            }
            Value::Block(bl) => (bl.result != Type::Void).then(|| bl.result.clone()),
            Value::If(_, t, _) => self.infer_type(t),
            _ => None,
        }
    }

    /// Emit a typed null sentinel for the given type.
    fn write_typed_null(w: &mut dyn Write, tp: &Type) -> std::io::Result<()> {
        match tp {
            Type::Integer(_, _) | Type::Character => write!(w, "i32::MIN"),
            Type::Long => write!(w, "i64::MIN"),
            Type::Float => write!(w, "f64::NAN"),
            Type::Single => write!(w, "f32::NAN"),
            Type::Boolean => write!(w, "false"),
            Type::Text(_) => write!(w, "\"\""),
            Type::Enum(_, false, _) => write!(w, "255_u8"),
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _) => {
                write!(w, "DbRef {{ store_nr: 0, rec: 0, pos: 0 }}")
            }
            _ => write!(w, "()"),
        }
    }

    fn output_if(
        &mut self,
        w: &mut dyn Write,
        test: &Value,
        true_v: &Value,
        false_v: &Value,
    ) -> std::io::Result<()> {
        write!(w, "if ")?;
        let b_true = matches!(*true_v, Value::Block(_));
        let b_false = matches!(*false_v, Value::Block(_));
        self.output_code_inner(w, test)?;
        if b_true {
            write!(w, " ")?;
        } else {
            write!(w, " {{")?;
        }
        self.indent += u32::from(!b_true);
        self.output_code_inner(w, true_v)?;
        self.indent -= u32::from(!b_true);
        if let Value::Block(_) = *true_v {
            write!(w, " else ")?;
        } else {
            write!(w, "}} else ")?;
        }
        if !b_false {
            write!(w, "{{")?;
        }
        self.indent += u32::from(!b_false);
        // When the else branch is Null and the true branch returns a value,
        // emit a typed null sentinel instead of () to match the true branch type.
        if matches!(false_v, Value::Null)
            && let Some(tp) = self.infer_type(true_v)
        {
            Self::write_typed_null(w, &tp)?;
        } else {
            self.output_code_inner(w, false_v)?;
        }
        self.indent -= u32::from(!b_false);
        if !b_false {
            write!(w, "}}")?;
        }
        Ok(())
    }

    /// Use this to emit a scoped sequence of operators with an optional return value.
    /// This is the most involved emitter because blocks must handle three interacting concerns:
    /// 1. **Pre-evaluation hoisting** — sub-expressions that would double-borrow `stores`
    ///    are lifted into `let _preN` bindings before the enclosing expression.
    /// 2. **Return-value tracking** — when void operators trail the last non-void expression,
    ///    that expression is captured into `let _ret` first, then yielded at the end.
    /// 3. **String conversion** — a text-typed block may receive a `Str` from a field read;
    ///    `.to_string()` converts it to an owned `String`.
    fn output_block(
        &mut self,
        w: &mut dyn Write,
        bl: &Block,
        wrap_text: bool,
    ) -> std::io::Result<()> {
        writeln!(
            w,
            "{{ //{}_{}: {}",
            bl.name,
            bl.scope,
            bl.result
                .show(self.data, &self.data.def(self.def_nr).variables)
        )?;
        let is_void_block = matches!(bl.result, Type::Void);
        let is_text_result = wrap_text && matches!(bl.result, Type::Text(_));
        // Fix "hoisted return value" pattern from scopes::free_vars before iterating.
        // This replaces [expr, OpFreeText…, Return(Null)] with [OpFreeText…, Return(expr)]
        // so native code emits `return expr` rather than a dropped `expr` + `return ()`.
        let patched_ops;
        let operators: &[Value] = if is_void_block {
            patched_ops = self.patch_hoisted_returns(&bl.operators);
            &patched_ops
        } else {
            &bl.operators
        };
        // When the block expects a non-void result but trailing operator(s) are
        // void (drops, if-without-else, etc.), find the last non-void operator
        // and capture its value before the trailing void ops run.
        let last_op_idx = operators.len().saturating_sub(1);
        let return_idx = if is_void_block || operators.is_empty() {
            None
        } else {
            operators.iter().rposition(|v| !self.is_void_value(v))
        };
        let has_trailing_void = return_idx.is_some_and(|i| i < last_op_idx);
        for (vnr, v) in operators.iter().enumerate() {
            if matches!(v, Value::Line(_)) {
                continue;
            }
            // Collect pre-evaluations needed for this operator (to avoid double
            // mutable borrow of stores when user-defined functions are nested).
            // NOTE: indent is incremented here to match the level used in
            // output_code_with_subst below, so multi-line block pre_codes match.
            self.indent += 1;
            let pre_evals = self.collect_pre_evals(v)?;
            self.indent -= 1;
            let counter_after_collect = self.counter;
            for (name, code, _, _) in &pre_evals {
                self.indent(w)?;
                writeln!(w, "let {name} = {code};")?;
            }
            // Restore counter to the value it had when the pre-eval code was generated
            // so that output_code_with_subst regenerates the same inner _pre_N names
            // as those stored in the pre-eval strings (counter desync fix).
            let restore_counter = pre_evals
                .iter()
                .map(|(_, _, c, _)| *c)
                .max()
                .unwrap_or(self.counter);
            self.counter = restore_counter;
            self.indent(w)?;
            if has_trailing_void && return_idx == Some(vnr) {
                write!(w, "let _ret = ")?;
                self.indent += 1;
                self.output_code_with_subst(w, v, &pre_evals)?;
                self.indent -= 1;
                writeln!(w, ";")?;
            } else {
                let is_return_expr =
                    !is_void_block && !has_trailing_void && return_idx == Some(vnr);
                let wrap_result = is_return_expr && is_text_result;
                let narrow_cast =
                    if is_return_expr { narrow_int_cast(&bl.result) } else { None };
                if wrap_result {
                    write!(w, "Str::new(")?;
                } else if narrow_cast.is_some() {
                    write!(w, "(")?;
                }
                self.indent += 1;
                self.output_code_with_subst(w, v, &pre_evals)?;
                self.indent -= 1;
                if wrap_result {
                    write!(w, ")")?;
                } else if let Some(cast) = narrow_cast {
                    write!(w, ") as {cast}")?;
                }
                if is_return_expr {
                    writeln!(w)?;
                } else {
                    writeln!(w, ";")?;
                }
            }
            // Restore counter to the state after collect_pre_evals so the next
            // operator gets fresh, non-conflicting pre-eval names.
            self.counter = counter_after_collect;
        }
        if has_trailing_void {
            self.indent(w)?;
            if is_text_result {
                writeln!(w, "Str::new(_ret)")?;
            } else if let Some(cast) = narrow_int_cast(&bl.result) {
                writeln!(w, "_ret as {cast}")?;
            } else {
                writeln!(w, "_ret")?;
            }
        }
        self.indent(w)?;
        write!(
            w,
            "}} /*{}_{}: {}*/",
            bl.name,
            bl.scope,
            bl.result
                .show(self.data, &self.data.def(self.def_nr).variables)
        )?;
        Ok(())
    }

    /// Use this to emit a variable assignment.
    /// On first use it emits `let mut var_<name>: <type> = ...` and tracks the variable
    /// in `declared`. On later uses it emits plain `var_<name> = ...`.
    ///
    /// Two special cases:
    /// - Text variables assigned from a block are pre-declared as `String::new()` before the
    ///   block opens, so that a `drop(@var)` inside the block (e.g., on `break`) can still
    ///   reference the variable even though `let` has not been reached.
    /// - `DbRef` variables assigned `Null` call `stores.null()` rather than emitting `()`.
    fn output_set(&mut self, w: &mut dyn Write, var: u16, to: &Value) -> std::io::Result<()> {
        let variables = &self.data.def(self.def_nr).variables;
        if variables.is_argument(var)
            && let Type::RefVar(inner) = variables.tp(var)
        {
            if to != &Value::Null && matches!(**inner, Type::Text(_)) {
                let name = sanitize(variables.name(var));
                write!(w, "*var_{name} = ")?;
                self.output_code_inner(w, to)?;
                write!(w, ".to_string()")?;
            }
            return Ok(());
        }
        let needs_to_string = matches!(variables.tp(var), Type::Text(_));
        let name = sanitize(variables.name(var));
        // When assigning a reference to a reference variable, a pointer copy is not
        // sufficient — emit an OpCopyRecord call after the assignment for a deep copy.
        let copy_record: Option<(String, u32)> = match (variables.tp(var), to) {
            (Type::Reference(d_nr, _), Value::Var(src))
                if matches!(variables.tp(*src), Type::Reference(_, _)) =>
            {
                Some((sanitize(variables.name(*src)), *d_nr))
            }
            _ => None,
        };
        // For text/reference block assignments, pre-declare the variable so that
        // any drop(@var) inside the block (e.g., on break) can reference it.
        if !self.declared.contains(&var) && matches!(to, Value::Block(_)) {
            let var_tp = variables.tp(var);
            if matches!(var_tp, Type::Text(_)) {
                self.declared.insert(var);
                write!(w, "let mut var_{name} = ")?;
                self.output_code_inner(w, to)?;
                if needs_to_string {
                    write!(w, ".to_string()")?;
                }
                return Ok(());
            }
        }
        if self.declared.contains(&var) {
            write!(w, "var_{name} = ")?;
        } else {
            self.declared.insert(var);
            let var_tp = variables.tp(var);
            let tp_str = rust_type(var_tp, &Context::Variable);
            write!(w, "let mut var_{name}: {tp_str} = ")?;
        }
        if matches!(to, Value::Null) && rust_type(variables.tp(var), &Context::Variable) == "DbRef"
        {
            write!(w, "stores.null()")?;
        } else {
            self.output_code_inner(w, to)?;
            if needs_to_string {
                write!(w, ".to_string()")?;
            } else if to != &Value::Null && narrow_int_cast(variables.tp(var)).is_some() {
                // Variable is a narrow integer type (stored as i32), but the RHS expression
                // (a function returning u16 or an iterator block returning as u16) produces
                // the narrow type. Add an explicit `as i32` cast.
                write!(w, " as i32")?;
            }
        }
        if let Some((src_name, d_nr)) = copy_record {
            let tp_nr = self.data.def(d_nr).known_type;
            writeln!(w, ";")?;
            self.indent(w)?;
            write!(
                w,
                "OpCopyRecord(stores, var_{src_name}, var_{name}, {tp_nr}_i32)"
            )?;
        }
        Ok(())
    }

    /// Use this to dispatch a `Value::Call` to either the user-function or template emitter.
    /// Certain built-in text operations are intercepted here because their generated Rust
    /// differs structurally from both a regular call and a template substitution.
    #[allow(clippy::too_many_lines)] // large opcode dispatch — splitting would lose context
    fn output_call(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
        vals: &[Value],
    ) -> std::io::Result<()> {
        let def_fn = self.data.def(def_nr);
        let name: &str = &def_fn.name;
        match name {
            "OpFormatLong" | "OpFormatStackLong" => {
                return self.format_long(w, vals, name == "OpFormatStackLong");
            }
            "OpFormatFloat" | "OpFormatStackFloat" => {
                return self.format_float(w, vals, name == "OpFormatStackFloat");
            }
            "OpFormatSingle" | "OpFormatStackSingle" => {
                return self.format_single(w, vals, name == "OpFormatStackSingle");
            }
            "OpFormatText" | "OpFormatStackText" => return self.format_text(w, vals),
            "OpAppendText" => return self.append_text(w, vals),
            "OpAppendStackText" => {
                write!(w, "*")?;
                return self.append_text(w, vals);
            }
            "OpAppendCharacter" | "OpAppendStackCharacter" => {
                return self.append_character(w, vals);
            }
            "OpClearStackText" | "OpClearText" => return self.clear_stack_text(w, vals),
            "OpClearVector" => return self.clear_vector(w, vals),
            "OpFreeText" | "OpCreateStack" | "OpNullRefSentinel" => return Ok(()),
            "OpCopyRecord" => {
                // Deep copy: copy_block + copy_claims
                if let [ref src, ref dst, ref tp_val] = vals[..] {
                    write!(w, "OpCopyRecord(stores, ")?;
                    self.output_code_inner(w, src)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, dst)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, tp_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpConvTextFromNull" => {
                write!(w, "\"\"")?;
                return Ok(());
            }
            "OpConvRefFromNull" => {
                write!(w, "DbRef {{ store_nr: 0, rec: 0, pos: 0 }}")?;
                return Ok(());
            }
            "OpGetTextSub" => {
                // text[from..till] → &str slice
                if let [ref text_val, ref from_val, ref till_val] = vals[..] {
                    write!(w, "OpGetTextSub(")?;
                    self.output_code_inner(w, text_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, from_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, till_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpSizeofRef" => {
                if let [ref val] = vals[..] {
                    write!(w, "OpSizeofRef(stores, ")?;
                    self.output_code_inner(w, val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpDatabase" => {
                // OpDatabase modifies its DbRef argument in-place; emit as reassignment.
                if let [ref var_val, ref tp_val] = vals[..] {
                    self.output_code_inner(w, var_val)?;
                    write!(w, " = OpDatabase(stores, ")?;
                    self.output_code_inner(w, var_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, tp_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpFormatDatabase" | "OpFormatStackDatabase" => {
                // OpFormatDatabase takes a &mut String as the output buffer.
                if let [ref work_val, ref record_val, ref tp_val, ref fmt_val] = vals[..] {
                    write!(w, "OpFormatDatabase(stores, &mut ")?;
                    // work_val is Var(nr) — strip the leading & that output_code_inner adds
                    if let Value::Var(nr) = work_val {
                        let variables = &self.data.def(self.def_nr).variables;
                        write!(w, "var_{}", sanitize(variables.name(*nr)))?;
                    } else {
                        self.output_code_inner(w, work_val)?;
                    }
                    write!(w, ", ")?;
                    self.output_code_inner(w, record_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, tp_val)?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, fmt_val)?;
                    write!(w, ")")?;
                }
                return Ok(());
            }
            "OpGetRecord" => {
                // vals: [data, db_tp, count, key1, key2, …]
                // Emit: OpGetRecord(stores, data, db_tp, &[Content::…, …])
                if vals.len() >= 3
                    && let (Value::Int(db_tp), Value::Int(_count)) = (&vals[1], &vals[2])
                {
                    let db_tp = *db_tp;
                    let key_types: Vec<i8> = self
                        .stores
                        .types
                        .get(usize::try_from(db_tp).unwrap_or(0))
                        .map(|t| t.keys.iter().map(|k| k.type_nr).collect())
                        .unwrap_or_default();
                    let key_vals = &vals[3..];
                    write!(w, "OpGetRecord(stores, ")?;
                    self.output_code_inner(w, &vals[0])?;
                    write!(w, ", {db_tp}_i32, &[")?;
                    for (i, key_val) in key_vals.iter().enumerate() {
                        if i > 0 {
                            write!(w, ", ")?;
                        }
                        let type_nr = key_types.get(i).copied().unwrap_or(1);
                        self.emit_content(w, key_val, type_nr)?;
                    }
                    write!(w, "])")?;
                    return Ok(());
                }
            }
            "OpIterate" => {
                // vals: [data, on, arg, Keys(keys), from_count, from_vals…, till_count, till_vals…]
                // Emit: OpIterate(stores, data, on, arg, &[Key{…}], &[Content::…], &[Content::…])
                if vals.len() >= 4
                    && let Value::Keys(keys) = &vals[3]
                {
                    let keys = keys.clone();
                    let rest = &vals[4..];
                    let from_count = if let Some(Value::Int(n)) = rest.first() {
                        usize::try_from(*n).unwrap_or(0)
                    } else {
                        0
                    };
                    let till_start = 1 + from_count;
                    let till_count = if let Some(Value::Int(n)) = rest.get(till_start) {
                        usize::try_from(*n).unwrap_or(0)
                    } else {
                        0
                    };
                    let from_vals = rest.get(1..till_start).unwrap_or(&[]);
                    let till_vals = rest
                        .get(till_start + 1..till_start + 1 + till_count)
                        .unwrap_or(&[]);
                    write!(w, "OpIterate(stores, ")?;
                    self.output_code_inner(w, &vals[0])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[2])?;
                    write!(w, ", &[")?;
                    for (i, k) in keys.iter().enumerate() {
                        if i > 0 {
                            write!(w, ", ")?;
                        }
                        write!(
                            w,
                            "Key {{ type_nr: {}, position: {} }}",
                            k.type_nr, k.position
                        )?;
                    }
                    write!(w, "], &[")?;
                    for (i, v) in from_vals.iter().enumerate() {
                        if i > 0 {
                            write!(w, ", ")?;
                        }
                        let type_nr = keys.get(i).map_or(1, |k| k.type_nr);
                        self.emit_content(w, v, type_nr)?;
                    }
                    write!(w, "], &[")?;
                    for (i, v) in till_vals.iter().enumerate() {
                        if i > 0 {
                            write!(w, ", ")?;
                        }
                        let type_nr = keys.get(i).map_or(1, |k| k.type_nr);
                        self.emit_content(w, v, type_nr)?;
                    }
                    write!(w, "])")?;
                    return Ok(());
                }
            }
            "OpStep" => {
                // vals: [iter_var, data, on, arg]
                // Emit: OpStep(stores, &mut var_iter, data, on, arg)
                if vals.len() == 4 {
                    write!(w, "OpStep(stores, &mut ")?;
                    if let Value::Var(v) = &vals[0] {
                        let name = sanitize(self.data.def(self.def_nr).variables.name(*v));
                        write!(w, "var_{name}")?;
                    } else {
                        self.output_code_inner(w, &vals[0])?;
                    }
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[2])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[3])?;
                    write!(w, ")")?;
                    return Ok(());
                }
            }
            "OpRemove" => {
                // vals: [state_var, data, on, tp/arg]
                // Emit: OpRemove(stores, &mut var_state, data, on, arg)
                // The state may be i32 (plain vector) or i64 (sorted/tree iterator).
                if vals.len() == 4 {
                    write!(w, "OpRemove(stores, &mut ")?;
                    if let Value::Var(v) = &vals[0] {
                        let name =
                            sanitize(self.data.def(self.def_nr).variables.name(*v));
                        write!(w, "var_{name}")?;
                    } else {
                        self.output_code_inner(w, &vals[0])?;
                    }
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[1])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[2])?;
                    write!(w, ", ")?;
                    self.output_code_inner(w, &vals[3])?;
                    write!(w, ")")?;
                    return Ok(());
                }
            }
            _ => {}
        }
        if def_fn.rust.is_empty() {
            self.output_call_user_fn(w, def_fn, vals)
        } else {
            self.output_call_template(w, def_fn, vals)
        }
    }

    /// Use this to emit `OpClearVector` with a null-record guard.
    /// `stores.null()` returns a `DbRef` whose `store_nr` is valid but `rec == 0`;
    /// calling `vector::clear_vector` on it panics.  The guard skips the call when
    /// the vector has not yet been allocated.
    fn clear_vector(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [Value::Var(nr)] = vals {
            let v_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            write!(
                w,
                "if var_{v_nr}.rec != 0 {{ vector::clear_vector(&var_{v_nr}, &mut stores.allocations); }}"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit a single key value as a typed `Content::…` constructor.
    /// `type_nr` is from a `Key` struct; sign indicates sort direction (ignored here),
    /// absolute value indicates the data type:
    /// 1 = integer, 2 = long, 3 = f32, 4 = f64, 5 = bool, 6 = text, 7 = byte.
    fn emit_content(&mut self, w: &mut dyn Write, v: &Value, type_nr: i8) -> std::io::Result<()> {
        let expr = self.generate_expr_buf(v)?;
        match type_nr.unsigned_abs() {
            1 | 5 | 7 => write!(w, "Content::Long({expr} as i64)"),
            2 => write!(w, "Content::Long({expr})"),
            3 => write!(w, "Content::Single({expr})"),
            4 => write!(w, "Content::Float({expr})"),
            6 => write!(w, "Content::Str(Str::new(&*({expr})))"),
            _ => write!(w, "Content::Long(0)"),
        }
    }

    /// Use this to emit `OpClearStackText` as a `.clear()` call on the target string variable.
    fn clear_stack_text(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [Value::Var(nr)] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            write!(w, "var_{s_nr}.clear()")?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpAppendCharacter` with a null-character guard,
    /// because loft represents characters as integers and zero means no character.
    fn append_character(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [Value::Var(nr), val] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            write!(
                w,
                "{{let c = {val_expr}; if c != 0 {{ var_{s_nr}.push(ops::to_char(c)); }} }}"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpAppendText` as a `+=` on the target string variable.
    fn append_text(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [Value::Var(nr), val] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            write!(w, "var_{s_nr} += &*({val_expr})")?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatText`/`OpFormatStackText` as a call to `ops::format_text`.
    fn format_text(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [
            Value::Var(nr),
            val,
            width,
            Value::Int(dir),
            Value::Int(token),
        ] = vals
        {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            // User-defined functions returning text produce `Str`; `format_text` expects `&str`.
            // Wrap with `&*` to deref `Str` → `&str` (deref coercion via Deref<Target=str>).
            // Also wrap template functions whose rust string contains `Str::new(` (e.g.
            // OpCastTextFromEnum), since those also produce `Str` rather than `&str`.
            let val_str = if let Value::Call(d, _) = val
                && matches!(self.data.def(*d).returned, Type::Text(_))
                && (self.data.def(*d).rust.is_empty()
                    || self.data.def(*d).rust.contains("Str::new("))
            {
                format!("&*({val_expr})")
            } else {
                val_expr
            };
            let width_expr = self.generate_expr_buf(width)?;
            write!(
                w,
                "ops::format_text(&mut var_{s_nr}, {val_str}, {width_expr}, {dir}, {token})"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatLong` as a call to `ops::format_long`.
    fn format_long(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [
            Value::Var(nr),
            val,
            Value::Int(radix),
            width,
            Value::Int(token),
            Value::Boolean(plus),
            Value::Boolean(note),
        ] = vals
        {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_long({prefix}var_{s_nr}, {val_expr}, {radix} as u8, {width_expr}, {token} as u8, {plus}, {note})"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    fn format_float(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [Value::Var(nr), val, width, prec] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prec_expr = self.generate_expr_buf(prec)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_float({prefix}var_{s_nr}, {val_expr}, {width_expr}, {prec_expr})"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatSingle`/`OpFormatStackSingle` as a call to `ops::format_single`.
    fn format_single(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [Value::Var(nr), val, width, prec] = vals {
            let s_nr = sanitize(self.data.def(self.def_nr).variables.name(*nr));
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prec_expr = self.generate_expr_buf(prec)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_single({prefix}var_{s_nr}, {val_expr}, {width_expr}, {prec_expr})"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit a call to a user-defined loft function as `fn_name(stores, arg0, …)`.
    fn output_call_user_fn(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        vals: &[Value],
    ) -> std::io::Result<()> {
        write!(w, "{}(stores", def_fn.name)?;
        for v in vals {
            write!(w, ", ")?;
            if let Some(vr) = self.create_stack_var(v) {
                let name = sanitize(self.data.def(self.def_nr).variables.name(vr));
                write!(w, "&mut var_{name}")?;
            } else {
                self.output_code_inner(w, v)?;
            }
        }
        write!(w, ")")
    }

    /// Use this to inline a `#rust` template operator by substituting `@param` placeholders
    /// with generated argument expressions.
    fn output_call_template(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        vals: &[Value],
    ) -> std::io::Result<()> {
        let mut res = def_fn.rust.clone();
        for (a_nr, a) in def_fn.attributes.iter().enumerate() {
            let name = "@".to_string() + &a.name;
            if a_nr < vals.len() {
                // For enum-typed parameters, Value::Null means the null enum byte (255).
                if matches!(a.typedef, Type::Enum(_, _, _)) && matches!(vals[a_nr], Value::Null) {
                    res = res.replace(&name, "(255u8)");
                    continue;
                }
                // For character-typed parameters, Value::Int means a character code point.
                if matches!(a.typedef, Type::Character)
                    && let Value::Int(n) = vals[a_nr]
                {
                    let with = format!("char::from_u32({n}_u32).unwrap_or('\\0')");
                    res = res.replace(&name, &format!("({with})"));
                    continue;
                }
                // For character-typed parameters, a variable holding an i32 char needs
                // ops::to_char() because the template expects a `char`, not `i32`.
                if matches!(a.typedef, Type::Character)
                    && let Value::Var(n) = vals[a_nr]
                    && matches!(self.data.def(self.def_nr).variables.tp(n), Type::Character)
                {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    res = res.replace(&name, &format!("(ops::to_char({inner}))"));
                    continue;
                }
                // For character-typed parameters, a call returning character yields `i32`
                // (due to the `as u32 as i32` auto-cast), so wrap with ops::to_char().
                if matches!(a.typedef, Type::Character)
                    && let Value::Call(d, _) = &vals[a_nr]
                    && matches!(self.data.def(*d).returned, Type::Character)
                {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    res = res.replace(&name, &format!("(ops::to_char({inner}))"));
                    continue;
                }
                // Text-typed parameters: a user-defined function returning text produces `Str`,
                // but templates expect `&str`. Deref `Str` → `&str` with `&*`.
                if matches!(a.typedef, Type::Text(_))
                    && let Value::Call(d, _) = &vals[a_nr]
                    && matches!(self.data.def(*d).returned, Type::Text(_))
                    && self.data.def(*d).rust.is_empty()
                {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    res = res.replace(&name, &format!("(&*({inner}))"));
                    continue;
                }
                let mut with = self.generate_expr_buf(&vals[a_nr])?;
                // Integer parameter receiving a char value needs explicit cast.
                if matches!(a.typedef, Type::Integer(_, _)) {
                    let val_is_char = match &vals[a_nr] {
                        Value::Var(n) => {
                            matches!(self.data.def(self.def_nr).variables.tp(*n), Type::Character)
                        }
                        Value::Call(d, _) => {
                            matches!(self.data.def(*d).returned, Type::Character)
                        }
                        _ => false,
                    };
                    if val_is_char {
                        with += " as u32 as i32";
                    }
                }
                // Templates use u32::from(@name) for field offsets; that was written for u16
                // parameters (fill.rs).  Native codegen emits i32 literals, so substitute the
                // entire u32::from(@name) pattern with (@value) as u32 to stay type-correct.
                let u32_from_pat = format!("u32::from({name})");
                if res.contains(&u32_from_pat) {
                    res = res.replace(&u32_from_pat, &format!("({with}) as u32"));
                } else {
                    // When the template parameter expects a narrow unsigned integer (u8/u16),
                    // native codegen emits i32 literals.  Add a cast so the types match.
                    // Use Context::Result to get the precise narrow type (e.g. u16) since
                    // Context::Variable returns i32 for narrow integers.
                    let tp_str = rust_type(&a.typedef, &Context::Result);
                    if matches!(tp_str.as_str(), "u8" | "u16") {
                        let typed_with = if with.ends_with("_i32") {
                            format!("{}_{tp_str}", &with[..with.len() - 4])
                        } else {
                            format!("({with}) as {tp_str}")
                        };
                        res = res.replace(&name, &format!("({typed_with})"));
                    } else {
                        res = res.replace(&name, &format!("({with})"));
                    }
                }
            } else {
                println!(
                    "Problem def_fn {def_fn} attributes {:?} vals {vals:?}",
                    def_fn.attributes
                );
                break;
            }
        }
        // Templates use `s.database.` and `s.` for bytecode interpreter (State).
        // In generated native code, `stores` is the direct Stores reference.
        res = res.replace("s.database.", "stores.");
        res = res.replace("s.db_from_text(", "db_from_text(stores, ");
        res = res.replace("crate::state::", "loft::state::");
        // loft represents `character` as `i32`; template functions that return `char`
        // (like `ops::text_character`) need an explicit cast at the call site.
        if matches!(def_fn.returned, Type::Character) {
            write!(w, "({res}) as u32 as i32")
        } else {
            write!(w, "{res}")
        }
    }
}

/// Use this to emit the `db.field` registration for one struct field using a typed builder call.
/// Keeping the two-line pattern in one place prevents the builder name and field name
/// from diverging across the six field-type variants in `output_struct`.
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
