// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use crate::data::{Context, Data, DefType, IntegerSpec, Type, Value};
use crate::database::Stores;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
mod calls;
mod coroutine;
mod dispatch;
mod emit;
pub(crate) mod ops;
mod pre_eval;
mod text;

/// Entry produced by `collect_pre_evals`: `(temp_var, type_str, expr_code, def_nr, stores_fn)`.
type PreEvalEntry = (String, String, String, u32, bool);

/// Rust source spliced into the native `main` bootstrap just before its
/// closing brace.  Gated on `LOFT_NATIVE_LEAK_CHECK` so normal native
/// runs stay silent, but tests (and a curious developer) can opt in to
/// the same store-leak report the interpreter prints unconditionally —
/// giving leak regressions a guard on `--native`, not just `--interpret`.
const NATIVE_LEAK_CHECK_TAIL: &str = "    if std::env::var(\"LOFT_NATIVE_LEAK_CHECK\").is_ok() {\n        let stores: &Stores = unsafe { &*cell.get() };\n        let leaks = stores.collect_store_leaks();\n        if !leaks.is_empty() {\n            let count = leaks.len();\n            let preview = if count <= 5 { leaks.join(\", \") } else { format!(\"{} ... and {} more\", leaks[..5].join(\", \"), count - 5) };\n            eprintln!(\"Warning: {count} stores not freed at program exit: {preview}\");\n        }\n    }\n";

/// Walk the Value IR tree and collect all function definition numbers
/// referenced by `Value::Call(def_nr, _)` nodes.
/// Detect a T-parameterized method stub: name shape
/// `t_<digits><identifier>_<method>` where `<identifier>` is the
/// generic type variable's name.  These stubs are synthesized by
/// `parse_function`'s I7/I8.1 path for every interface method on a
/// bound type parameter (`fn foo<T: Bound>(...)`).  At call sites
/// they are substituted with the concrete impl via
/// `re_resolve_call`, so the stub body is never entered at runtime.
///
/// The discriminator from a regular `t_<LEN><Type>_<method>` (e.g.
/// `t_4text_starts_with`) is that `<Type>` here is a known concrete
/// type defined in the program; for T-stubs it's a generic type
/// variable name.  We can't distinguish those at this layer without
/// access to the Data table, so we accept the false-positive
/// possibility (a real builtin matching the same shape but missing
/// from `codegen_runtime.rs`) — those would emit `todo!()` instead
/// of `compile_error!()`, which still aborts at runtime if reached
/// (just at the call site rather than at compile time).
///
/// Pragmatic bar: any `t_<digits><alpha-prefix>_*` whose body is
/// empty + no `#rust` + no `#native` is treated as a T-stub.  The
/// caller already checks the empty-body branch.
/// @P321a — `#native` symbols implemented in `src/codegen_runtime.rs` (reusing
/// loft's zero-dep pure-Rust `crate::sha256` / `crate::base64`) rather than a
/// package native crate.  Routed to `loft::codegen_runtime::<symbol>` so
/// `--native` and WASM link them directly (and DCE drops them when unused).
fn is_crypto_runtime_symbol(symbol: &str) -> bool {
    matches!(
        symbol,
        "n_sha256"
            | "n_hmac_sha256"
            | "n_hmac_sha256_raw"
            | "n_base64_encode"
            | "n_base64_decode"
            | "n_base64url_encode"
    )
}

fn is_t_param_stub(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("t_") else {
        return false;
    };
    // Parse the leading length digits.
    let len_end = rest.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(0);
    if len_end == 0 {
        return false;
    }
    let Ok(type_len) = rest[..len_end].parse::<usize>() else {
        return false;
    };
    let after_len = &rest[len_end..];
    if after_len.len() < type_len + 1 {
        return false;
    }
    // The next `type_len` chars are the type name; then `_` then method.
    let type_name = &after_len[..type_len];
    let after_type = &after_len[type_len..];
    if !after_type.starts_with('_') {
        return false;
    }
    // Heuristic: a generic type variable is a single ASCII identifier
    // (mostly UPPERCASE single letter or short PascalCase).  Concrete
    // builtins use lowercase type names (`text`, `integer`, `single`,
    // `float`, `boolean`, `character`, `enum`, `function`, plus narrow
    // variants and user struct/enum names which are CamelCase).  Treat
    // a type-name component starting with an uppercase letter as a
    // potential generic-T stub.  This catches `T`, `T_p205`, `U`,
    // `MyTrait` etc.
    //
    // The known concrete CamelCase types (`JsonValue`, `JsonField`,
    // `RegexCapture`, etc.) are wired in `codegen_runtime.rs` already
    // and never reach this branch (the `is_codegen_runtime_fn` check
    // earlier returns true for them).  Any concrete CamelCase type
    // that ISN'T wired AND has an empty body would be incorrectly
    // labeled "T-stub" — but in that case the right behaviour is
    // identical (emit `todo!()` instead of compile_error since the
    // function is genuinely unimplemented).
    type_name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
}

fn collect_calls(val: &Value, data: &Data, calls: &mut HashSet<u32>) {
    match val {
        Value::Call(d, args) => {
            calls.insert(*d);
            // n_parallel_for / n_parallel_queue pass a worker function as
            // args[4]: an integer literal that the codegen emitter
            // (src/generation/ops/parallel.rs) resolves into a closure body
            // calling the worker by name.  Detect it here so the worker
            // is included in the reachable set — without this, the
            // closure refers to a fn that never gets emitted and rustc
            // fails with "cannot find function" (E0425).
            if matches!(
                data.def(*d).name.as_str(),
                "n_parallel_for"
                    | "n_parallel_for_light"
                    | "n_parallel_queue"
                    | "n_parallel_queue_text"
                    | "n_parallel_queue_ref"
                    | "n_parallel_queue_narrow"
                    | "n_parallel_queue_fn"
            ) && args.len() >= 5
                && let Value::Int(fn_d_nr) = &args[4]
                && *fn_d_nr >= 0
            {
                calls.insert((*fn_d_nr).cast_unsigned());
            }
            // ARC.md A5b — par_fold uses a different arg layout than
            // the for/queue family: the worker fn d_nr is at args[2]
            // (after input + init).  Same reason for the insert: the
            // ParallelFoldEmitter generates `worker_name(cell, acc, row)`
            // and the worker must be in the reachable set.
            if data.def(*d).name.as_str() == "n_parallel_fold"
                && args.len() >= 4
                && let Value::Int(fn_d_nr) = &args[2]
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
        Value::Span(b) => collect_calls(&b.1, data, calls),
        _ => {}
    }
}

/// Recursively collect all `Int` literals from a value tree that may represent
/// fn-ref constants (e.g. inside `if`/`block` branches of a function-typed `Set`).
fn collect_int_fn_refs(val: &Value, calls: &mut HashSet<u32>) {
    match val {
        Value::Int(n) if *n >= 0 => {
            calls.insert((*n).cast_unsigned());
        }
        // FnRef(d_nr, clos_var, _) is used for closure fn-refs.
        Value::FnRef(d_nr, _, _) if *d_nr >= 0 => {
            calls.insert((*d_nr).cast_unsigned());
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
        // Span wraps most operators for parser diagnostics — recurse
        // through it so `Span(Int(d_nr))` fn-ref literals at call
        // sites get added to the reachable set (without this, the
        // CallRef match-arm dispatch is missing candidates and panics
        // at runtime with `invalid fn-ref: <n>`).
        Value::Span(b) => collect_int_fn_refs(&b.1, calls),
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
            // @P299 — a fn-ref stored into a struct FIELD lowers to
            // `OpSetInt4(target, pos, Int(<lambda d_nr>))`: the lambda's d_nr
            // rides as a plain Int (capturing closures wrap this in a
            // `fn_ref_field_set` block, non-capturing emit it bare — see
            // `parser/mod.rs::set_field_check`).  The FnRef-literal walk below
            // can't see it, so a closure called ONLY through a struct field is
            // pruned as unreachable and dropped from the native fn-ref dispatch
            // candidate set (`emit.rs::output_call_ref`) → `invalid fn-ref`
            // panic.  Recover it: if `OpSetInt4` writes a literal that is a
            // valid Function definition, mark it reachable.  This
            // over-approximates (a plain int field equal to a fn d_nr would
            // mark that fn reachable too) but reachability over-approximation
            // is correctness-safe — it only ever emits an unused candidate.
            if callee.name == "OpSetInt4"
                && let Some(arg2) = args.get(2)
                && let Value::Int(dn) = arg2.unspan()
                && *dn >= 0
                && (*dn as u32) < data.definitions()
                && matches!(data.def(*dn as u32).def_type, DefType::Function)
                && !data.def(*dn as u32).name.starts_with("Op")
            {
                calls.insert(*dn as u32);
            }
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
        Value::FnRef(d_nr, _, _) if *d_nr >= 0 => {
            calls.insert((*d_nr).cast_unsigned());
        }
        // Span wraps most operators for parser diagnostics — recurse so
        // Set / Call args that arrive as Span(...) still trigger the
        // fn-ref-literal walk.
        Value::Span(b) => collect_fn_ref_literals(&b.1, data, variables, calls),
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
#[allow(clippy::struct_excessive_bools)]
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
    /// When set alongside `yield_collect`, the eager-collect buffer is
    /// `Vec<String>` (text-yielding generator) and the `Value::Yield`
    /// emission wraps the value in `.to_string()` instead of `as i64`.
    pub yield_collect_text: bool,
    /// P224: when emitting a coroutine state-machine method body
    /// (`emit_next_i64` / `emit_next_text`), each `var_nr` listed here
    /// is a function-local that lives as a struct field on the
    /// generator (so its value persists across `next_*` calls).
    /// `Value::Var(v)` for these emits `self.var_<name>` rather than
    /// `var_<name>`; `Value::Set(v, _)` emits `self.var_<name> = …;`.
    /// Empty outside coroutine bodies.
    pub coroutine_persistent_vars: HashSet<u16>,
    /// When true, `Value::Int` emits a `(d_nr_u32, null_DbRef)` tuple
    /// instead of `d_nr_i32`.  Set during fn-ref variable assignment so
    /// if-else branches produce the correct tuple type.
    pub fn_ref_context: bool,
    /// When true, `Value::Int` emits `{v}_i32` instead of the post-2c
    /// default `{v}_i64`.  Set when emitting a tp-number / field-index /
    /// flag-enum slot (where the runtime signature is still i32) so
    /// compile-time constants land at the expected width.  Cleared on
    /// entry to every recursive `output_code_inner` that isn't
    /// explicitly inside such a slot.
    pub i32_literal_context: bool,
    /// When true, `Value::Text` literals inside a `Value::Tuple` emit
    /// with a trailing `.to_string()` so the element fits a
    /// `String`-typed tuple slot.  Set by `set_var` (and similar
    /// destination-aware paths) when assigning a `Value::Tuple` to a
    /// `Type::Tuple(...)` variable that has at least one `Type::Text`
    /// element.  Cleared after the assignment so argument-context
    /// tuples (which need `&str`) keep the default emit.
    pub tuple_text_to_string: bool,
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
        Type::Integer(s) if s.range() - 1 <= 255 && i64::from(s.min) >= 0 => Some("u8"),
        Type::Integer(s) if s.range() - 1 <= 65536 && i64::from(s.min) >= 0 => Some("u16"),
        Type::Integer(s) if s.range() - 1 <= 255 => Some("i8"),
        Type::Integer(s) if s.range() - 1 <= 65536 => Some("i16"),
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
        Type::Integer(s) if context == &Context::Result && s.range() - 1 <= 255 && s.min >= 0 => {
            "u8"
        }
        Type::Integer(s)
            if context == &Context::Result && s.range() - 1 <= 65536 && s.min >= 0 =>
        {
            "u16"
        }
        Type::Integer(s) if context == &Context::Result && s.range() - 1 <= 255 => "i8",
        Type::Integer(s) if context == &Context::Result && s.range() - 1 <= 65536 => "i16",
        Type::Enum(_, false, _) => "u8",
        Type::Character | Type::Null => "i32",
        Type::Integer(_) => "i64",
        // null is represented as the null sentinel of the target type
        Type::Text(_) if context == &Context::Variable => "String",
        Type::Text(_) if context == &Context::Argument => "&str",
        Type::Text(_) => "Str",
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
        // T1.8a: in Result context, recurse with Variable context for element
        // types so a tuple-of-text return signature is `(String, String)` —
        // matching the caller's owned-tuple slot.  Without this the signature
        // becomes `(Str, Str)` (Result-context Text → "Str") while the
        // caller's local declares `(String, String)` (Variable-context),
        // producing a type mismatch.  Argument/Variable contexts already
        // recurse with their own context (giving `(&str, &str)` or
        // `(String, String)` respectively), which matches the call site
        // and the variable slot.
        Type::Tuple(elems) => {
            let elem_context = if matches!(context, Context::Result) {
                &Context::Variable
            } else {
                context
            };
            let parts: Vec<String> = elems.iter().map(|e| rust_type(e, elem_context)).collect();
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
            // Tuple variables hold Variable-context element types
            // (`String` for `Text`, etc.), so the per-element default
            // must match.  Map `Type::Text` to `String::new()` here
            // — the bare `default_native_value(Text)` would return
            // `Str::new(...)` which is the Argument-context literal
            // and won't fit a `String`-typed tuple slot.
            let parts: Vec<String> = elems
                .iter()
                .map(|e| match e {
                    Type::Text(_) => "String::new()".to_string(),
                    other => default_native_value(other),
                })
                .collect();
            format!("({})", parts.join(", "))
        }
        _ => "0".into(), // Integer, Character, Enum(u8), etc.
    }
}

/// Which subset of a struct / enum-value's attributes to emit in the
/// current pass of `output_init`.  Phase 1 emits `Simple` fields so
/// bare Sorted/Hash/Index types registered later find their content
/// struct already populated.  Phase 2 emits `Collection` fields (which
/// reference those pre-created bare collections via `t{N}`) and
/// `EnumValues` (the `db.value` add-backs that close the enum ↔
/// variant mutual-recursion cycle).
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
enum FieldPhase {
    /// Emit ALL struct / enum-value fields in source order.  Collection
    /// fields (Sorted/Hash/Index/Vector) trigger inline `db.sorted /
    /// hash / index / vector` creation that dedups on name — the
    /// runtime id assigned at first creation matches the compile-time
    /// `known_type`, so subsequent references through raw literals
    /// (`OpNewRecord(parent_tp, field_index)`) stay correct.
    AllFields,
    /// Emit only the `db.value(...)` add-backs for enum values.
    EnumValues,
    /// (Historical — kept for potential partial emission.)
    Simple,
    Collection,
}

/// Return true when the given field type participates in Phase 2
/// (collection-typed fields that reference a bare Vector / Sorted /
/// Hash / Index created during `output_init`'s first pass).
fn is_collection_field(tp: &Type) -> bool {
    matches!(
        tp,
        Type::Vector(_, _) | Type::Sorted(_, _, _) | Type::Hash(_, _, _) | Type::Index(_, _, _)
    )
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
                        // Post-2c round 10c: wide Type::Integer (former Type::Long)
                        // passes as i64 — range > i32::MAX can't fit in i32.
                        Type::Integer(s) if s.is_wide() => {
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
                    // Post-2c round 10c: wide Type::Integer returns as i64.
                    Type::Integer(s) if s.is_wide() => " -> i64".to_string(),
                    Type::Integer(_) | Type::Character => " -> i32".to_string(),
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
        writeln!(w, "use loft::hash;")?;
        writeln!(w, "use loft::tree;")?;
        writeln!(w, "use loft::codegen_runtime;")?;
        writeln!(w, "use loft::codegen_runtime::*;")?;
        // The `external::` namespace is used by stdlib #rust templates for rand/random ops.
        // Use codegen_runtime wrappers so no cfg(feature) is needed in generated files.
        writeln!(
            w,
            "mod external {{
    pub fn rand_seed(seed: i64) {{ loft::codegen_runtime::cr_rand_seed(seed); }}
    pub fn rand_int(lo: i64, hi: i64) -> i64 {{ loft::codegen_runtime::cr_rand_int(lo, hi) }}
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
        // P269 — populate reachability from `n_main` so the unimplemented-
        // native check (`output_function`'s `todo!()`-emit branches) can
        // distinguish "reachable AND unimpl" (compile_error!, fail at
        // build time per the "fail at startup, not runtime" principle)
        // from "unreachable + unimpl" (harmless `todo!()` shim).  Without
        // this, the reachability set was empty under `--native` (only
        // populated by `output_native_reachable`), so EVERY unimpl native
        // would have to compile-error or NONE could.  Now the two emit
        // paths agree on what's actually live in the program.
        let main_nr = self.data.def_nr("n_main");
        if main_nr < self.data.definitions() {
            self.reachable = reachable_functions(self.data, &[main_nr]);
        } else {
            // No `n_main` (test-script style with `fn p203_*` as the
            // top-level fn).  Walk reachability from EVERY user-source
            // function so the compile_error check has accurate data.
            // Without this, the `is_empty()` fallback in
            // `output_function` treats every unimplemented stub as
            // reachable and emits compile_error, breaking test
            // scripts that don't define a main.
            //
            // "User source" = position.file outside `default/` and
            // `lib/`.  Imprecise (it includes any user file, not just
            // the entry script) but safe — the worst case is a few
            // false-positive reachable defs that emit compile_error
            // when they could've been todo!(), still better than the
            // pre-fix behaviour where EVERYTHING was treated as
            // reachable.
            let mut entries: Vec<u32> = Vec::new();
            for d in 0..self.data.definitions() {
                let def = self.data.def(d);
                if !matches!(def.def_type, DefType::Function) {
                    continue;
                }
                let pos_file = &def.position.file;
                // Match `/default/` and `/lib/` segments under EITHER path
                // separator — Windows uses `\` so a bare `/default/`
                // substring check misses `default\01_code.loft` and
                // misclassifies every stdlib fn as user source, which
                // pulls the entire stdlib into the reachable set and
                // raises P269 compile_error for unimplemented natives
                // (n_parallel_queue_fn / n_parallel_buf_get_fn / ...).
                // Linux/macOS only ever produce `/` separators so the
                // original check happened to work; Windows surfaced
                // the gap in PR #212 CI.
                let normalised: String = pos_file
                    .chars()
                    .map(|c| if c == '\\' { '/' } else { c })
                    .collect();
                if pos_file.is_empty()
                    || normalised.contains("/default/")
                    || normalised.contains("/lib/")
                {
                    continue;
                }
                entries.push(d);
            }
            if !entries.is_empty() {
                self.reachable = reachable_functions(self.data, &entries);
            }
        }
        Self::emit_file_header(w, self.data, self.wasm_browser)?;
        writeln!(w, "fn init(cell: &std::cell::UnsafeCell<Stores>) {{")?;
        writeln!(
            w,
            "    let db: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        self.output_init(w, from, till)?;
        writeln!(w, "    db.finish();")?;
        // Mirror `compile::build_const_vectors` so module-scope `const`
        // vectors (`const NUMS = [10, 20, 30]`) populate `db.const_refs`
        // before `n_main` runs.  Without this, `OpConstRef(<d_nr>)`
        // indexes into an empty Vec and panics.  @P275 fix.
        self.emit_const_vectors(w, till)?;
        writeln!(w, "}}\n")?;
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
        writeln!(w, "fn init(cell: &std::cell::UnsafeCell<Stores>) {{")?;
        writeln!(
            w,
            "    let db: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
        // Register ALL types (0..till) so runtime type IDs match compile-time IDs.
        self.output_init(w, 0, till)?;
        writeln!(w, "    db.finish();")?;
        // Initiative 03 Phase 3b: emit code to build CONST_STORE
        // vectors and populate `db.const_refs` — mirrors the
        // interpreter path in `compile::build_const_vectors`.
        self.emit_const_vectors(w, till)?;
        writeln!(w, "}}\n")?;
        // Emit only reachable functions across the full definition range.
        self.output_functions(w, 0, till, Some(&reachable))?;
        // Emit a Rust entry point that bootstraps the loft `main` function, if present.
        if (0..till).any(|d| self.data.def(d).name == "n_main") {
            if self.wasm_browser {
                // exported cdylib entry point for browser WASM.  WASM
                // doesn't have an argv, so user_args stays empty —
                // arguments() returns [].
                writeln!(
                    w,
                    "\n#[unsafe(no_mangle)]\npub extern \"C\" fn loft_start() {{\n    let cell = std::cell::UnsafeCell::new(Stores::new());\n    init(&cell);\n    n_main(&cell);\n}}"
                )?;
            } else {
                // Native binary: thread std::env::args() into Stores.user_args
                // so the loft `arguments()` builtin returns the program's
                // CLI args.  Skip [0] (binary path).  Mirrors what
                // src/main.rs does for the interpreter path
                // (state.database.user_args.clone_from(&user_args)).
                // @PLAN37 phase 10.3 fix.
                write!(
                    w,
                    "\nfn main() {{\n    let cell = std::cell::UnsafeCell::new(Stores::new());\n    {{ let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; stores.user_args = std::env::args().skip(1).collect(); }}\n    init(&cell);\n    n_main(&cell);\n"
                )?;
                w.write_all(NATIVE_LEAK_CHECK_TAIL.as_bytes())?;
                writeln!(w, "}}")?;
            }
        }
        Ok(())
    }

    /// Emit a Rust `fn main()` bootstrap if the program defines a loft `main` function.
    fn emit_main_bootstrap(&self, w: &mut dyn Write, till: u32) -> std::io::Result<()> {
        let main_nr = self.data.def_nr("n_main");
        if main_nr < till {
            write!(
                w,
                "\nfn main() {{\n    let cell = std::cell::UnsafeCell::new(Stores::new());\n    {{ let stores: &mut Stores = unsafe {{ &mut *cell.get() }}; stores.user_args = std::env::args().skip(1).collect(); }}\n    init(&cell);\n    n_main(&cell);\n"
            )?;
            w.write_all(NATIVE_LEAK_CHECK_TAIL.as_bytes())?;
            writeln!(w, "}}")?;
        }
        Ok(())
    }

    /// Use this to emit only the `init` body that registers all types.
    /// Sorting by `known_type` ensures the runtime recreates type IDs in the same order
    /// as the compile-time database, keeping field indices consistent.
    fn output_init(&mut self, w: &mut dyn Write, from: u32, till: u32) -> std::io::Result<()> {
        // Base types are pre-registered by `Stores::new()` with fixed indices
        // 0..=6 (integer, long, single, float, boolean, text, character — see
        // `src/database/mod.rs:Stores::new`).  Subsequent struct / vector /
        // hash / index fields reference these by `known_type`.  The emitter
        // binds each pre-registered id into a `t{N}` variable so field
        // references use the same `t{N}` form as types created below — the
        // `known_type → runtime id` identity is made explicit via scope.
        for n in 0..=6u16 {
            writeln!(w, "    let t{n}: u16 = {n};")?;
        }
        let _ = writeln!(
            w,
            "    let _ = (t0, t1, t2, t3, t4, t5, t6); // suppress unused-let warnings for unreferenced base types"
        );
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

        // Collect bare Byte/Short/Int types that were registered by
        // `database.byte` / `.short` / `.int` during type-field lowering
        // (e.g. narrow integer fields with `size(N)` annotations).  These
        // have no corresponding loft definition, so `output_init` would
        // otherwise skip them entirely, leaving a GAP in the runtime type
        // id sequence and shifting every type numbered after it.
        //
        // Enum values without attributes (plain tag variants) are also not
        // in `type_defs` — they don't get their own runtime type record,
        // but `Stores::enumerate` still advances the counter when the
        // parent enum is registered, so the plain-tag variants themselves
        // don't consume a slot.  Only Parts::{Byte, Short, Int} produce
        // standalone runtime types that need to be re-created here.
        let def_type_id_set: HashSet<u16> = type_defs.iter().map(|&(tid, _)| tid).collect();
        // @P296 — keyed-collection types reachable only as a struct/enum FIELD
        // are created inline during that container's field emission (so they
        // must NOT also be emitted in the bare_io stream — see the
        // Sorted/Hash/Index arm below).  But a keyed-collection LOCAL var
        // (`s: sorted<Item[k]> = []`) mints a keyed type via
        // `gen_set_first_keyed_null` that NO struct field references, so it
        // never gets created inline → its runtime type id is missing and
        // `content(tp)` returns u16::MAX → `set_default_value` panics on
        // `--native`.  Collect the field-referenced keyed type ids here so
        // the bare_io collection below can emit the local-only ones.
        let field_keyed: HashSet<u16> = {
            let mut set = HashSet::new();
            for tp in &self.stores.types {
                if let crate::database::Parts::Struct(fields)
                | crate::database::Parts::EnumValue(_, fields) = &tp.parts
                {
                    for f in fields {
                        if matches!(
                            self.stores.types[f.content as usize].parts,
                            crate::database::Parts::Sorted(_, _)
                                | crate::database::Parts::Hash(_, _)
                                | crate::database::Parts::Index(_, _, _)
                        ) {
                            set.insert(f.content);
                        }
                    }
                }
            }
            set
        };
        #[allow(dead_code)]
        enum BareIo {
            Byte(i32, bool),
            Short(i32, bool),
            ShortRaw(i32, bool),
            Int(i32, bool),
            Vector(u16),
            Sorted(u16, Vec<(u16, bool)>),
            Hash(u16, Vec<u16>),
            Index(u16, Vec<(u16, bool)>),
        }
        let mut bare_io: Vec<(u16, BareIo)> = Vec::new();
        for (idx, tp) in self.stores.types.iter().enumerate() {
            let tid = idx as u16;
            if def_type_id_set.contains(&tid) {
                continue;
            }
            match &tp.parts {
                crate::database::Parts::Byte(min, nullable) => {
                    bare_io.push((tid, BareIo::Byte(*min, *nullable)));
                }
                crate::database::Parts::Short(min, nullable) => {
                    bare_io.push((tid, BareIo::Short(*min, *nullable)));
                }
                crate::database::Parts::ShortRaw(min, nullable) => {
                    bare_io.push((tid, BareIo::ShortRaw(*min, *nullable)));
                }
                crate::database::Parts::Int(min, nullable) => {
                    bare_io.push((tid, BareIo::Int(*min, *nullable)));
                }
                crate::database::Parts::Vector(c) => {
                    bare_io.push((tid, BareIo::Vector(*c)));
                }
                // Sorted / Hash / Index that are a struct/enum FIELD are
                // created INLINE during that container's field emission
                // (via `emit_field` → `db.sorted / hash / index`), so they
                // must NOT be emitted here — doing so would swap the
                // container's field source-order and break baked-in
                // `OpNewRecord(parent_tp, field_index)` calls.  But a keyed
                // type minted only for a LOCAL var (@P296) is referenced by
                // no field, so it would otherwise leave a GAP in the runtime
                // type-id sequence (→ `content(tp)` u16::MAX → panic).  Emit
                // exactly those local-only keyed types here, at their tid
                // position; the bare_io arms below already know how.
                crate::database::Parts::Sorted(c, keys) if !field_keyed.contains(&tid) => {
                    bare_io.push((tid, BareIo::Sorted(*c, keys.clone())));
                }
                crate::database::Parts::Hash(c, keys) if !field_keyed.contains(&tid) => {
                    bare_io.push((tid, BareIo::Hash(*c, keys.clone())));
                }
                crate::database::Parts::Index(c, keys, _) if !field_keyed.contains(&tid) => {
                    bare_io.push((tid, BareIo::Index(*c, keys.clone())));
                }
                crate::database::Parts::Sorted(_, _)
                | crate::database::Parts::Hash(_, _)
                | crate::database::Parts::Index(_, _, _) => {}
                _ => {}
            }
        }
        bare_io.sort_by_key(|&(tid, _)| tid);
        let mut bare_idx = 0;
        // Resolve a struct's field name by field_nr — needed for
        // Sorted/Hash/Index key-string emission at bare-type level.
        let resolve_field_name = |c: u16, k: u16| -> String {
            if let crate::database::Parts::Struct(ref fields)
            | crate::database::Parts::EnumValue(_, ref fields) =
                self.stores.types[c as usize].parts
            {
                fields[k as usize].name.clone()
            } else {
                "?".to_string()
            }
        };

        // Build a map from known_type → dnr for dependency resolution.
        let type_id_to_dnr: HashMap<u16, u32> =
            type_defs.iter().map(|&(tid, dnr)| (tid, dnr)).collect();

        // For each struct / enum-value / enum, collect the known_type ids that
        // its emission will *reference* as `t{N}` let-bindings — so the
        // topological walk emits them first.  Previously these were raw u16
        // literals and forward references worked; with the Category D let-
        // binding scheme, every referenced id must already be in scope.
        //
        // Struct / EnumValue: content-type of each sorted / hash / index /
        // vector field is a dep.  Enum: each typed variant (EnumValue with
        // attributes) is a dep, since `db.value(enum, variant_name, t{N})`
        // must find the variant's `t{N}` binding in scope.
        let mut deps: HashMap<u16, Vec<u16>> = HashMap::new();
        for &(type_id, dnr) in &type_defs {
            let def = self.data.def(dnr);
            let is_container = matches!(def.def_type, DefType::Struct)
                || (def.def_type == DefType::EnumValue && !def.attributes.is_empty());
            let is_enum = def.def_type == DefType::Enum;
            if !is_container && !is_enum {
                continue;
            }
            let mut d: Vec<u16> = Vec::new();
            if is_container {
                for a in &def.attributes.clone() {
                    let c_nr = match &a.typedef {
                        Type::Sorted(c_nr, _, _)
                        | Type::Hash(c_nr, _, _)
                        | Type::Index(c_nr, _, _) => {
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
            } else {
                // is_enum: typed variants referenced by `db.value(enum, name, t{N})`.
                for a in &def.attributes.clone() {
                    if matches!(a.typedef, Type::Enum(_, true, _)) {
                        // Resolve the EnumValue def_nr whose parent matches.
                        let v_dnr = (0..self.data.definitions()).find(|&v| {
                            let vd = self.data.def(v);
                            vd.def_type == DefType::EnumValue
                                && vd.parent == dnr
                                && vd.name == a.name
                        });
                        if let Some(v_dnr) = v_dnr {
                            let v_tp = self.data.def(v_dnr).known_type;
                            if v_tp != u16::MAX && type_id_to_dnr.contains_key(&v_tp) {
                                d.push(v_tp);
                            }
                        }
                    }
                }
            }
            if !d.is_empty() {
                deps.insert(type_id, d);
            }
        }

        // Two-phase emission: (1) create all types in known_type order so every
        // cross-reference in phase 2 is a backward reference to an already-bound
        // `t{N}`; (2) populate struct / enum-value fields and enum values once
        // every type id is in scope.  This resolves mutual-recursion cycles
        // (e.g. JsonValue enum with JArray variant holding vector<JsonValue>)
        // that broke the previous single-pass topological approach.
        let _ = deps; // no longer used; kept only for future re-introduction
        let _ = type_id_to_dnr;

        // Single-pass emission in strict known_type order.
        //
        // For struct / enum-value types we emit `db.structure` + fields
        // immediately so that any subsequent bare Sorted/Hash/Index
        // (registered inline via the struct field emission) gets its
        // runtime id assigned at the exact moment that matches the
        // compile-time `known_type`.  For enums we only emit
        // `db.enumerate` here; the `db.value(enum, variant_tid)` calls
        // move to Phase 2 so that mutual-recursion cycles (enum →
        // typed variant → enum) break cleanly.
        //
        // `deps` / `type_id_to_dnr` are retired — known_type order is
        // sufficient because parse-time `fill_database` guarantees each
        // type's content dependencies already have a `known_type` by
        // the time the type itself is registered.
        // Track which type_ids have been fully emitted (creation +
        // fields) so the recursive walk below breaks cycles.  A type's
        // `db.structure` / `db.enumerate` call is emitted BEFORE
        // recursion, so by the time a field references it the `t{N}`
        // binding is already in scope — resolving the mutual-recursion
        // case (JsonValue enum ↔ JArray variant with
        // `items: vector<JsonValue>`).
        let mut emitted: HashSet<u16> = HashSet::new();
        let type_id_to_dnr_local = type_id_to_dnr;

        for &(type_id, dnr) in &type_defs {
            while bare_idx < bare_io.len() && bare_io[bare_idx].0 < type_id {
                let (tid, ref bio) = bare_io[bare_idx];
                match bio {
                    BareIo::Byte(min, nullable) => {
                        writeln!(w, "    let t{tid} = db.byte({min}, {nullable});")?;
                    }
                    BareIo::Short(min, nullable) => {
                        writeln!(w, "    let t{tid} = db.short({min}, {nullable});")?;
                    }
                    BareIo::ShortRaw(min, nullable) => {
                        writeln!(w, "    let t{tid} = db.short_raw({min}, {nullable});")?;
                    }
                    BareIo::Int(min, nullable) => {
                        writeln!(w, "    let t{tid} = db.int({min}, {nullable});")?;
                    }
                    BareIo::Vector(c) => {
                        let c_ref = type_id_ref(*c);
                        writeln!(w, "    let t{tid} = db.vector({c_ref});")?;
                    }
                    BareIo::Sorted(c, keys) => {
                        let c_ref = type_id_ref(*c);
                        let keys_str = keys
                            .iter()
                            .map(|&(k, asc)| {
                                format!("(\"{}\".to_string(), {asc})", resolve_field_name(*c, k))
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        writeln!(w, "    let t{tid} = db.sorted({c_ref}, &[{keys_str}]);")?;
                    }
                    BareIo::Hash(c, keys) => {
                        let c_ref = type_id_ref(*c);
                        let keys_str = keys
                            .iter()
                            .map(|&k| format!("\"{}\".to_string()", resolve_field_name(*c, k)))
                            .collect::<Vec<_>>()
                            .join(", ");
                        writeln!(w, "    let t{tid} = db.hash({c_ref}, &[{keys_str}]);")?;
                    }
                    BareIo::Index(c, keys) => {
                        let c_ref = type_id_ref(*c);
                        let keys_str = keys
                            .iter()
                            .map(|&(k, asc)| {
                                format!("(\"{}\".to_string(), {asc})", resolve_field_name(*c, k))
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        writeln!(w, "    let t{tid} = db.index({c_ref}, &[{keys_str}]);")?;
                    }
                }
                writeln!(w, "    let _ = t{tid}; // may be unused")?;
                bare_idx += 1;
            }
            self.emit_def_create_recurse_fields(
                w,
                type_id,
                dnr,
                &deps,
                &type_id_to_dnr_local,
                &mut emitted,
            )?;
        }
        while bare_idx < bare_io.len() {
            let (tid, ref bio) = bare_io[bare_idx];
            match bio {
                BareIo::Byte(min, nullable) => {
                    writeln!(w, "    let t{tid} = db.byte({min}, {nullable});")?;
                }
                BareIo::Short(min, nullable) => {
                    writeln!(w, "    let t{tid} = db.short({min}, {nullable});")?;
                }
                BareIo::ShortRaw(min, nullable) => {
                    writeln!(w, "    let t{tid} = db.short_raw({min}, {nullable});")?;
                }
                BareIo::Int(min, nullable) => {
                    writeln!(w, "    let t{tid} = db.int({min}, {nullable});")?;
                }
                BareIo::Vector(c) => {
                    let c_ref = type_id_ref(*c);
                    writeln!(w, "    let t{tid} = db.vector({c_ref});")?;
                }
                BareIo::Sorted(c, keys) => {
                    let c_ref = type_id_ref(*c);
                    let keys_str = keys
                        .iter()
                        .map(|&(k, asc)| {
                            format!("(\"{}\".to_string(), {asc})", resolve_field_name(*c, k))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(w, "    let t{tid} = db.sorted({c_ref}, &[{keys_str}]);")?;
                }
                BareIo::Hash(c, keys) => {
                    let c_ref = type_id_ref(*c);
                    let keys_str = keys
                        .iter()
                        .map(|&k| format!("\"{}\".to_string()", resolve_field_name(*c, k)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(w, "    let t{tid} = db.hash({c_ref}, &[{keys_str}]);")?;
                }
                BareIo::Index(c, keys) => {
                    let c_ref = type_id_ref(*c);
                    let keys_str = keys
                        .iter()
                        .map(|&(k, asc)| {
                            format!("(\"{}\".to_string(), {asc})", resolve_field_name(*c, k))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(w, "    let t{tid} = db.index({c_ref}, &[{keys_str}]);")?;
                }
            }
            writeln!(w, "    let _ = t{tid}; // may be unused")?;
            bare_idx += 1;
        }

        // Phase 2 — enum value add-backs (db.value) emitted after all
        // typed variant structs have been created, so the enum ↔
        // variant mutual-recursion cycle resolves without forward refs.
        for &(type_id, dnr) in &type_defs {
            if self.data.def(dnr).def_type == DefType::Enum {
                self.emit_type_fields_mode(w, type_id, dnr, FieldPhase::EnumValues)?;
            }
        }
        Ok(())
    }

    /// Initiative 03 Phase 3b: emit Rust code that rebuilds every
    /// `pub CONST: vector<T> = [...]` constant inside `init()`.
    /// Mirrors `compile::build_const_vectors` — for each DefType::
    /// Constant definition with vector content and literal values,
    /// allocates a fresh store, writes the elements, and records
    /// the DbRef in `db.const_refs[d_nr]`.  The `#rust"…"` template
    /// for `OpConstRef` is `s.const_ref_at(@d_nr as usize)`; native
    /// codegen translates `s.const_ref_at(` → `stores.const_ref_at_runtime(`
    /// (`src/generation/calls.rs`), so the emitted user functions
    /// find these DbRefs at call time.
    fn emit_const_vectors(&self, w: &mut dyn Write, till: u32) -> std::io::Result<()> {
        // Short-circuit if nothing references const_refs (avoids
        // emitting an unused `db.const_refs.resize(...)` that'd
        // produce a dead-code warning under `-D warnings`).
        let have_any = (0..till).any(|d| {
            self.data.def(d).def_type == DefType::Constant && self.data.def(d).const_ref.is_some()
        });
        if !have_any {
            return Ok(());
        }
        writeln!(
            w,
            "    // Initiative 03 Phase 3b — const_refs: mirror compile::build_const_vectors.",
        )?;
        writeln!(
            w,
            "    db.const_refs.resize({till}, loft::keys::DbRef {{ store_nr: u16::MAX, rec: 0, pos: 0 }});"
        )?;
        for d_nr in 0..till {
            let def = self.data.def(d_nr);
            if def.def_type != DefType::Constant {
                continue;
            }
            if def.const_ref.is_none() {
                continue;
            }
            let crate::data::Type::Vector(ref elem_tp_box, _) = def.returned else {
                continue;
            };
            let elem_tp = (**elem_tp_box).clone();
            let values = crate::compile::extract_literal_values_public(&def.code, self.data);
            if values.is_empty() {
                continue;
            }
            let vec_struct_name = format!("main_vector<{}>", elem_tp.name(self.data));
            let vec_struct_dnr = self.data.def_nr(&vec_struct_name);
            if vec_struct_dnr == u32::MAX {
                continue;
            }
            let vec_tp = self.data.def(vec_struct_dnr).known_type;
            if vec_tp == u16::MAX {
                continue;
            }
            let size = self.stores.size(vec_tp);
            writeln!(w, "    {{ // const d_nr={d_nr}")?;
            writeln!(w, "        let cv = db.database({size}_u32);")?;
            writeln!(
                w,
                "        db.store_mut(&cv).set_u32_raw(cv.rec, 4, {vec_tp}_u32);"
            )?;
            writeln!(w, "        db.set_default_value({vec_tp}, &cv);")?;
            writeln!(
                w,
                "        let cvr = loft::keys::DbRef {{ store_nr: cv.store_nr, rec: 1, pos: 8 }};"
            )?;
            for val in &values {
                writeln!(w, "        {{ let rec = db.record_new(&cvr, {vec_tp}, 0);")?;
                match val {
                    crate::data::Value::Int(v) => {
                        writeln!(
                            w,
                            "            db.store_mut(&rec).set_int(rec.rec, rec.pos, {v}_i64);"
                        )?;
                    }
                    crate::data::Value::Long(v) => {
                        writeln!(
                            w,
                            "            db.store_mut(&rec).set_long(rec.rec, rec.pos, {v}_i64);"
                        )?;
                    }
                    crate::data::Value::Float(v) => {
                        writeln!(
                            w,
                            "            db.store_mut(&rec).set_float(rec.rec, rec.pos, {v}_f64);"
                        )?;
                    }
                    crate::data::Value::Single(v) => {
                        writeln!(
                            w,
                            "            db.store_mut(&rec).set_single(rec.rec, rec.pos, {v}_f32);"
                        )?;
                    }
                    crate::data::Value::Text(v) => {
                        let esc = v.replace('\\', "\\\\").replace('"', "\\\"");
                        writeln!(
                            w,
                            "            {{ let store = db.store_mut(&rec); \
                             let s_pos = store.set_str(\"{esc}\"); \
                             store.set_u32_raw(rec.rec, rec.pos, s_pos); }}"
                        )?;
                    }
                    _ => {}
                }
                writeln!(w, "            db.record_finish(&cvr, &rec, {vec_tp}, 0);")?;
                writeln!(w, "        }}")?;
            }
            writeln!(w, "        db.allocations[cv.store_nr as usize].lock();")?;
            writeln!(
                w,
                "        db.allocations[cv.store_nr as usize].ref_count = u32::MAX / 2;"
            )?;
            writeln!(w, "        db.const_refs[{d_nr}] = cvr;")?;
            writeln!(w, "    }}")?;
        }
        Ok(())
    }

    /// Recursive single-pass emission: create the type's `t{N}`
    /// binding first so any subsequent inline collection-field emission
    /// referencing it as `t{N}` finds the binding in scope.  Then
    /// recurse into content-type dependencies (from `deps`) to satisfy
    /// forward references like `JObject { fields: vector<JsonField> }`
    /// where `JsonField` has a higher `known_type` than `JObject`.
    /// Finally, emit fields in source order — inline collection creates
    /// dedup on name and land at the correct runtime id.
    #[allow(clippy::only_used_in_recursion)]
    fn emit_def_create_recurse_fields(
        &mut self,
        w: &mut dyn Write,
        type_id: u16,
        dnr: u32,
        deps: &HashMap<u16, Vec<u16>>,
        type_id_to_dnr: &HashMap<u16, u32>,
        emitted: &mut HashSet<u16>,
    ) -> std::io::Result<()> {
        if !emitted.insert(type_id) {
            return Ok(());
        }
        // Emit type-creation call first so the `t{type_id}` binding is
        // available for any recursive emission that reads it as a
        // content type below.
        self.emit_type_creation(w, type_id, dnr)?;
        if dnr == u32::MAX {
            return Ok(());
        }
        let def = self.data.def(dnr);
        if matches!(def.def_type, DefType::Struct)
            || (def.def_type == DefType::EnumValue && !def.attributes.is_empty())
        {
            // Walk fields in source order, mirroring parse-time
            // `fill_database` exactly.  For each collection field
            // (`vector / sorted / hash / index<X>`), recurse into the
            // content type X *inline* — so X's `db.structure` call (and
            // any type X's own fields trigger) land at the same
            // runtime index as parse time.  Then emit the field itself,
            // which triggers inline `db.vector / sorted / hash / index`
            // creation at the next runtime id — matching parse-time.
            let enum_value = if def.def_type == DefType::EnumValue {
                let parent = self.data.def(def.parent);
                parent
                    .attributes
                    .iter()
                    .enumerate()
                    .find(|(_, a)| a.name == def.name)
                    .map_or(0, |(i, _)| i32::try_from(i).unwrap_or(0) + 1)
            } else {
                0
            };
            let s_var = format!("t{type_id}");
            if enum_value > 0
                && def.known_type != u16::MAX
                && self.stores.position(def.known_type, "enum") == 0
            {
                writeln!(w, "    let byte_enum = db.byte(0, false);")?;
                writeln!(w, "    db.field({s_var}, \"enum\", byte_enum);")?;
            }
            let attrs = def.attributes.clone();
            for a in &attrs {
                // Resolve field's content dep and recurse inline
                // before the field is emitted — parse-time
                // `fill_database` does the same via recursive content
                // resolution when a collection field first names a
                // forward-declared type.
                let dep_tp = match &a.typedef {
                    Type::Sorted(c_nr, _, _) | Type::Hash(c_nr, _, _) | Type::Index(c_nr, _, _) => {
                        (*c_nr != u32::MAX)
                            .then(|| self.data.def(*c_nr).known_type)
                            .filter(|t| *t != u16::MAX)
                    }
                    Type::Vector(c_type, _) => {
                        let n = self.data.type_def_nr(c_type);
                        (n != u32::MAX)
                            .then(|| self.data.def(n).known_type)
                            .filter(|t| *t != u16::MAX)
                    }
                    // Plan-06 phase 4d: tuple struct fields inline the
                    // synthetic `__tuple<…>` struct's bytes — emit its
                    // type-creation call before the parent's `db.field`
                    // so the forward reference (`db.field(t_parent, "v",
                    // t_synthetic_tuple)`) sees the synthetic binding
                    // already declared.
                    Type::Tuple(_) => {
                        let n = self.data.type_def_nr(&a.typedef);
                        (n != u32::MAX)
                            .then(|| self.data.def(n).known_type)
                            .filter(|t| *t != u16::MAX)
                    }
                    _ => None,
                };
                if let Some(dep_tp) = dep_tp
                    && !emitted.contains(&dep_tp)
                    && let Some(&dep_dnr) = type_id_to_dnr.get(&dep_tp)
                {
                    self.emit_def_create_recurse_fields(
                        w,
                        dep_tp,
                        dep_dnr,
                        deps,
                        type_id_to_dnr,
                        emitted,
                    )?;
                }
                let td_nr = self.data.type_def_nr(&a.typedef);
                let field_type_id = self.data.def(td_nr).known_type;
                let forced = self.data.forced_size(a.alias_d_nr);
                self.emit_field(
                    w,
                    &s_var,
                    &a.name,
                    &a.typedef,
                    a.nullable,
                    field_type_id,
                    forced,
                )?;
            }
        }
        Ok(())
    }

    /// Phase 1 — emit just the type-creation call for `dnr` (no fields,
    /// no enum values).  Captures the runtime id in a `let t{type_id}`
    /// binding so Phase 2 field/value emission can reference it.
    fn emit_type_creation(
        &mut self,
        w: &mut dyn Write,
        type_id: u16,
        dnr: u32,
    ) -> std::io::Result<()> {
        if dnr == u32::MAX {
            eprintln!(
                "codegen warning: skipping type_id={type_id} — definition number is unresolved (u32::MAX)"
            );
            return Ok(());
        }
        let def = self.data.def(dnr);
        if matches!(def.def_type, DefType::Struct) {
            writeln!(w, "    let t{type_id} = db.structure(\"{}\", 0);", def.name)?;
        } else if def.def_type == DefType::EnumValue && !def.attributes.is_empty() {
            let parent_nr = def.parent;
            if parent_nr == u32::MAX {
                return Ok(());
            }
            let parent = self.data.def(parent_nr);
            let enum_value = parent
                .attributes
                .iter()
                .enumerate()
                .find(|(_, a)| a.name == def.name)
                .map_or(0, |(i, _)| i32::try_from(i).unwrap_or(0) + 1);
            writeln!(
                w,
                "    let t{type_id} = db.structure(\"{}\", {enum_value});",
                def.name
            )?;
        } else if def.def_type == DefType::Enum {
            writeln!(w, "    let t{type_id} = db.enumerate(\"{}\");", def.name)?;
        } else if def.def_type == DefType::Vector {
            // prefer the actual registered Parts::Vector
            // content from `stores.types[type_id]` — that's what
            // `fill_database` stored, including narrow-integer content
            // via `narrow_vector_content`.  Falling back to
            // `def.parent.known_type` resolves any Type::Integer to the
            // plain `integer` slot (0) and breaks narrow vectors.
            let content_known = if let crate::database::Parts::Vector(c) =
                self.stores.types[type_id as usize].parts
            {
                c
            } else if def.parent != u32::MAX {
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
                let content_ref = type_id_ref(content_known);
                writeln!(w, "    let t{type_id} = db.vector({content_ref});")?;
                writeln!(w, "    let _ = t{type_id}; // may be unused")?;
            }
        }
        Ok(())
    }

    /// Populate struct / enum-value fields or enum values.  `mode` selects
    /// which kinds of fields to emit:
    ///   `FieldPhase::Simple`      — scalar / text / enum-typed fields only.
    ///   `FieldPhase::Collection`  — Vector / Sorted / Hash / Index fields
    ///                               (reference pre-created `t{N}` bare
    ///                               types emitted by Phase 1).
    ///   `FieldPhase::EnumValues`  — enum value add-backs (`db.value`).
    fn emit_type_fields_mode(
        &self,
        w: &mut dyn Write,
        type_id: u16,
        dnr: u32,
        mode: FieldPhase,
    ) -> std::io::Result<()> {
        if dnr == u32::MAX {
            return Ok(());
        }
        let def = self.data.def(dnr);
        if matches!(def.def_type, DefType::Struct)
            || (def.def_type == DefType::EnumValue && !def.attributes.is_empty())
        {
            if matches!(
                mode,
                FieldPhase::Simple | FieldPhase::Collection | FieldPhase::AllFields
            ) {
                let enum_value = if def.def_type == DefType::EnumValue {
                    let parent = self.data.def(def.parent);
                    parent
                        .attributes
                        .iter()
                        .enumerate()
                        .find(|(_, a)| a.name == def.name)
                        .map_or(0, |(i, _)| i32::try_from(i).unwrap_or(0) + 1)
                } else {
                    0
                };
                self.output_struct_fields_filtered(w, dnr, enum_value, type_id, mode)?;
            }
        } else if def.def_type == DefType::Enum && matches!(mode, FieldPhase::EnumValues) {
            output_enum_values(w, dnr, self.data, type_id)?;
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
    /// `s_var` is the Rust variable holding the parent struct's runtime id
    /// (e.g. `t59` for `known_type=59`).
    #[allow(clippy::too_many_arguments)]
    fn emit_field(
        &self,
        w: &mut dyn Write,
        s_var: &str,
        field_name: &str,
        typedef: &Type,
        nullable: bool,
        known_type: u16,
        forced_size: Option<u8>,
    ) -> std::io::Result<()> {
        if let Type::Vector(c, _) = typedef {
            // when the element `Type::Integer` carries a
            // `forced_size` annotation that `vector_narrow_width`
            // accepts (u8 / i8 / u16 / i16 / i32), look up the narrow
            // content type-nr that `fill_database` already registered
            // via `narrow_vector_content`.  Without this,
            // `data.type_def_nr(c)` resolves any `Type::Integer` to
            // the plain `integer` def-nr → wide `vector<integer>`.
            // The wrapper's `main_vector<T>` struct field would end up
            // with 8-byte stride even though `fill_database` narrowed
            // the actual runtime Parts, corrupting reads/writes.
            if let Type::Integer(spec) = &**c
                && let Some(n) = spec.vector_narrow_width()
            {
                let name = match n {
                    1 => {
                        if spec.min == 0 {
                            "byte".to_string()
                        } else {
                            format!("byte<{},false>", spec.min)
                        }
                    }
                    2 => format!("short_raw<{},false>", spec.min),
                    4 => format!("int<{},false>", spec.min),
                    _ => String::new(),
                };
                if !name.is_empty() {
                    let narrow = self.stores.name(&name);
                    if narrow != u16::MAX {
                        let content_ref = type_id_ref(narrow);
                        emit_db_field(
                            w,
                            s_var,
                            field_name,
                            "vec",
                            &format!("db.vector({content_ref})"),
                        )?;
                        return Ok(());
                    }
                }
            }
            // P214: vector<fn-ref> elements route through narrow_vector_content
            // to a 4-byte int (Parts::Int with size=4) — match the parser's
            // `vector_of` path so the emitted `db.vector(narrow_int)`
            // matches the runtime narrow Parts.  Without this, the
            // `type_def_nr(Type::Function)` path returns `i32` whose
            // `known_type` is `u16::MAX` → emits `db.vector(u16::MAX)`
            // which panics in `Stores::field`'s parent-tracking when the
            // wrapper struct is registered.
            if matches!(**c, Type::Function(_, _, _)) {
                let narrow = self.stores.name("int<0,false>");
                if narrow != u16::MAX {
                    let content_ref = type_id_ref(narrow);
                    emit_db_field(
                        w,
                        s_var,
                        field_name,
                        "vec",
                        &format!("db.vector({content_ref})"),
                    )?;
                    return Ok(());
                }
            }
            let c_def = self.data.type_def_nr(c);
            if c_def != u32::MAX {
                let content = self.data.def(c_def).known_type;
                let content_ref = type_id_ref(content);
                emit_db_field(
                    w,
                    s_var,
                    field_name,
                    "vec",
                    &format!("db.vector({content_ref})"),
                )?;
            }
            return Ok(());
        }
        if let Type::Integer(IntegerSpec { min, .. }) = typedef {
            // Post-2c: the field's size may come from the integer alias's
            // `size(N)` annotation (captured in `Attribute.alias_d_nr` →
            // `Data::forced_size`) OR from the `Type::Integer` range.
            // Mirrors `src/typedef.rs:354-373` exactly so the runtime
            // Parts matches the interpreter's (Byte/Short/Int/base).
            let field_size = forced_size.unwrap_or_else(|| typedef.size(nullable));
            debug_assert!(
                matches!(field_size, 1 | 2 | 4 | 8),
                "emit_field: unexpected integer field width \
                 field_size={field_size} for `{field_name}` — only 1/2/4/8 \
                 are supported by db.byte / db.short / db.int / db.field"
            );
            if field_size == 1 {
                emit_db_field(
                    w,
                    s_var,
                    field_name,
                    "byte",
                    &format!("db.byte({min}, {nullable})"),
                )?;
            } else if field_size == 2 {
                emit_db_field(
                    w,
                    s_var,
                    field_name,
                    "short",
                    &format!("db.short({min}, {nullable})"),
                )?;
            } else if field_size == 4 {
                emit_db_field(
                    w,
                    s_var,
                    field_name,
                    "int",
                    &format!("db.int({min}, {nullable})"),
                )?;
            } else {
                writeln!(w, "    db.field({s_var}, \"{field_name}\", 0);")?;
            }
            return Ok(());
        }
        if let Type::Sorted(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type;
            let c_ref = type_id_ref(c_tp);
            let keys_str = keys
                .iter()
                .map(|(k, asc)| format!("(\"{k}\".to_string(), {asc})"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                s_var,
                field_name,
                "sorted",
                &format!("db.sorted({c_ref}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        if let Type::Hash(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type;
            let c_ref = type_id_ref(c_tp);
            let keys_str = keys
                .iter()
                .map(|k| format!("\"{k}\".to_string()"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                s_var,
                field_name,
                "hash",
                &format!("db.hash({c_ref}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        if let Type::Index(c_nr, keys, _) = typedef {
            let c_tp = self.data.def(*c_nr).known_type;
            let c_ref = type_id_ref(c_tp);
            let keys_str = keys
                .iter()
                .map(|(k, asc)| format!("(\"{k}\".to_string(), {asc})"))
                .collect::<Vec<_>>()
                .join(", ");
            emit_db_field(
                w,
                s_var,
                field_name,
                "index",
                &format!("db.index({c_ref}, &[{keys_str}])"),
            )?;
            return Ok(());
        }
        if matches!(typedef, Type::Function(_, _, _)) {
            // Storage holds the 4-byte i32 d_nr; closure half is not
            // stored.  Mirrors the typedef.rs Function arm so native
            // and interpreter agree on layout.
            emit_db_field(w, s_var, field_name, "int", "db.int(0, false)")?;
            return Ok(());
        }
        // Plan-22 phase 02c (P258 native fix, 2026-05-12): auto-Reference
        // attribute — when the dep list is non-empty, use the 12-byte
        // Parts::DbRef storage shape (`db.dbref()`) instead of the
        // inline-struct-bytes path below (which uses the inner struct's
        // known_type).  Mirrors `src/typedef.rs::fill_database`'s
        // `Type::Reference(_, ref deps) if !deps.is_empty()` branch so
        // native + interp agree on layout.  Without this, native
        // computes the closure record's auto-Reference field as
        // inline-bytes (size = inner struct's size) but interp writes
        // 12-byte DbRef bytes — the resulting size mismatch causes
        // `claim_child_rec`'s byte-copy to truncate at native's
        // smaller size and the lambda body reads garbage instead of
        // the captured DbRef.
        if let Type::Reference(_, deps) = typedef
            && !deps.is_empty()
        {
            emit_db_field(w, s_var, field_name, "dbref", "db.dbref()")?;
            return Ok(());
        }
        if known_type != u16::MAX {
            let kt_ref = type_id_ref(known_type);
            writeln!(w, "    db.field({s_var}, \"{field_name}\", {kt_ref});")?;
        }
        Ok(())
    }

    /// Populate struct / enum-value fields, restricted to the given
    /// `phase`.  Runs once per struct per phase (Simple before any bare
    /// Sorted/Hash/Index types register, Collection after they do).
    fn output_struct_fields_filtered(
        &self,
        w: &mut dyn Write,
        def_nr: u32,
        enum_value: i32,
        type_id: u16,
        phase: FieldPhase,
    ) -> std::io::Result<()> {
        let def = self.data.def(def_nr);
        let s_var = format!("t{type_id}");
        // Implicit enum-discriminator byte (inserted when the runtime
        // already had a plain `byte` type at the position where the
        // variant's content fields should begin).  Emitted only in
        // Phase 1 so field indices line up before any collection
        // fields are added.
        if phase == FieldPhase::Simple
            && enum_value > 0
            && def.known_type != u16::MAX
            && self.stores.position(def.known_type, "enum") == 0
        {
            writeln!(w, "    let byte_enum = db.byte(0, false);")?;
            writeln!(w, "    db.field({s_var}, \"enum\", byte_enum);")?;
        }
        for a in &def.attributes {
            let is_coll = is_collection_field(&a.typedef);
            let emit = match phase {
                FieldPhase::AllFields => true,
                FieldPhase::Simple => !is_coll,
                FieldPhase::Collection => is_coll,
                FieldPhase::EnumValues => false,
            };
            if !emit {
                continue;
            }
            let td_nr = self.data.type_def_nr(&a.typedef);
            let field_type_id = self.data.def(td_nr).known_type;
            assert_ne!(def_nr, u32::MAX, "Unknown def_nr for {:?}", a.typedef);
            let forced = self.data.forced_size(a.alias_d_nr);
            self.emit_field(
                w,
                &s_var,
                &a.name,
                &a.typedef,
                a.nullable,
                field_type_id,
                forced,
            )?;
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
        // Skip functions implemented in codegen_runtime — emitting a stub
        // would shadow the real implementation.  Plan 09 phase 01
        // consolidated the hardcoded list into the registry in
        // `src/codegen_runtime.rs::CODEGEN_RUNTIME_FNS`.
        if def.code == Value::Null && crate::codegen_runtime::is_codegen_runtime_fn(&def.name) {
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
                "fn n_assert<M: std::fmt::Display, F: std::fmt::Display>(_cell: &std::cell::UnsafeCell<Stores>, test: bool, msg: M, file: F, line: i64) {{"
            )?;
            writeln!(
                w,
                "  if !test {{ panic!(\"{{}}:{{}} {{}}\", file, line, msg); }}"
            )?;
            writeln!(w, "}}\n")?;
            return Ok(());
        }
        // DX-source-map: emit a `// loft:<file>:<line>` comment
        // above each function so rustc errors at the function header
        // (e.g. wrong arg type, missing trait impl) map directly to
        // the .loft definition site.
        if !def.position.file.is_empty() {
            writeln!(w, "// loft:{}:{}", def.position.file, def.position.line)?;
        }
        write!(w, "fn {}(cell: &std::cell::UnsafeCell<Stores>", def.name)?;
        for a in &def.attributes {
            let tp = rust_type(&a.typedef, &Context::Argument);
            write!(w, ", mut var_{}: {tp}", sanitize(&a.name))?;
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
                writeln!(
                    w,
                    "  let _stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                )?;
                writeln!(w, "  {}", default_native_value(&def.returned))?;
                writeln!(w, "}}")?;
            } else if instrument {
                // Emit shadow call stack instrumentation before the block body.
                // The CallGuard drop ensures cr_call_pop on all exit paths (including early return).
                // We emit the push/guard as a prefix inside the block's opening `{`.
                // P199 — prepend the `&mut Stores` derivation from the
                // `&UnsafeCell<Stores>` parameter so templates and inner
                // emissions see `stores` as a regular `&mut Stores` binding.
                let escaped_file = loft_file.replace('\\', "\\\\");
                self.call_stack_prefix = Some(format!(
                    "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};\n  \
                     cr_call_push(\"{loft_name}\", \"{escaped_file}\", {loft_line});\n  \
                     let _call_guard = codegen_runtime::CallGuard;"
                ));
                self.output_block(w, bl, returns_text)?;
                self.call_stack_prefix = None;
            } else {
                // Non-instrumented user-fn (e.g. `t_…` methods) — still
                // needs the `&mut Stores` derivation from the UnsafeCell
                // parameter for templates / inner calls.
                self.call_stack_prefix =
                    Some("  let stores: &mut Stores = unsafe { &mut *cell.get() };".to_string());
                self.output_block(w, bl, returns_text)?;
                self.call_stack_prefix = None;
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
                if is_crypto_runtime_symbol(&def.native) {
                    // @P321a — crypto `#native` symbols (sha256 / hmac / base64,
                    // from lib/crypto) are implemented in `codegen_runtime`
                    // reusing the zero-dependency pure-Rust `crate::sha256` /
                    // `crate::base64`, so `--native` AND WASM link them without a
                    // package crate or external dep (and DCE drops them when
                    // unused).  Same text ABI as a package `#native` fn.
                    let qualified = format!("loft::codegen_runtime::{}", def.native);
                    self.output_native_direct_call(w, def_nr, &qualified)?;
                } else if self.wasm_browser {
                    // call the imported function directly (unqualified).
                    // The function is declared in the preamble via
                    // #[link(wasm_import_module = "loft_gl")].
                    self.output_native_direct_call(w, def_nr, &def.native)?;
                } else if let Some(krate) = self.data.native_symbol_crates.get(&def.native) {
                    let qualified = format!("{}::{}", krate, def.native);
                    self.output_native_direct_call(w, def_nr, &qualified)?;
                } else {
                    // P269: refuse to emit a runtime panic for a reachable
                    // unimplemented native — convert to a compile-time error
                    // per the "fail at startup, not runtime" principle.
                    // Unreachable defs keep the `todo!()` shim so unused
                    // declarations don't reject otherwise-valid programs.
                    let reachable = self.reachable.is_empty() || self.reachable.contains(&def_nr);
                    writeln!(w, "{{")?;
                    if def.returned != Type::Void {
                        if reachable {
                            writeln!(
                                w,
                                "  compile_error!(\"loft --native: native fn `{}` (#native \\\"{}\\\") has no implementation in any registered native crate; either run via --interpret or wire the symbol in a #native package or src/codegen_runtime.rs (P269)\")",
                                def.name, def.native
                            )?;
                        } else {
                            writeln!(w, "  todo!(\"native function {}\")", def.name)?;
                        }
                    }
                    writeln!(w, "}}")?;
                }
            } else {
                // Internal i_ functions have implementations in codegen_runtime.rs;
                // all others get a todo!() stub.
                writeln!(w, "{{")?;
                if def.name == "i_parse_errors" {
                    writeln!(
                        w,
                        "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                    )?;
                    writeln!(w, "  loft::codegen_runtime::i_parse_errors(stores)")?;
                } else if def.name == "i_parse_error_push" {
                    writeln!(
                        w,
                        "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                    )?;
                    writeln!(
                        w,
                        "  loft::codegen_runtime::i_parse_error_push(stores, var_msg)"
                    )?;
                } else if def.name == "n_json_errors" {
                    writeln!(
                        w,
                        "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                    )?;
                    writeln!(w, "  loft::codegen_runtime::i_json_errors(stores)")?;
                } else if def.returned != Type::Void {
                    // P269: same compile-time-error escalation as above for
                    // reachable unimplemented natives without a `#native`
                    // annotation.  Unreachable internal stubs (e.g. unused
                    // `i_*` helpers) keep the `todo!()` shim.
                    //
                    // Three categories of "abstract declarations never
                    // called at runtime" must NOT trigger compile_error
                    // even when the reachability walker counts them:
                    //
                    // 1. Functions with a `#rust"…"` annotation (e.g.
                    //    text methods like `starts_with` / `ends_with`
                    //    / `trim` in `default/03_text.loft`) inline the
                    //    Rust expression at every call site via the
                    //    dispatch in `src/generation/calls.rs`.
                    // 2. Interface method stubs (`__iface_<N>_<method>`)
                    //    are abstract declarations created by
                    //    `parse_interface`; bound-generic dispatch
                    //    substitutes the call with the concrete impl
                    //    via `re_resolve_call`.
                    // 3. T-parameterized stubs (`t_<LEN><Tname>_<method>`
                    //    where `<Tname>` is a generic type variable)
                    //    are synthesized at function-parse time
                    //    (parse_function I7/I8.1); same re_resolve_call
                    //    substitution applies.
                    //
                    // For all three, the function body is emitted but
                    // never entered at runtime.  Use `todo!()` (the
                    // safe placeholder that costs nothing unless
                    // someone takes the address of the fn) instead
                    // of `compile_error!()`.
                    let is_iface_stub = def.name.starts_with("__iface_");
                    let is_t_stub = is_t_param_stub(&def.name);
                    let has_custom_op_emitter = ops::has_custom_emitter(&def.name);
                    let reachable = (self.reachable.is_empty() || self.reachable.contains(&def_nr))
                        && def.rust.is_empty()
                        && !is_iface_stub
                        && !is_t_stub
                        && !has_custom_op_emitter;
                    if reachable {
                        writeln!(
                            w,
                            "  compile_error!(\"loft --native: built-in fn `{}` has no native implementation; wire it in src/codegen_runtime.rs or run via --interpret (P269)\")",
                            def.name
                        )?;
                    } else {
                        writeln!(w, "  todo!(\"native function {}\")", def.name)?;
                    }
                }
                writeln!(w, "}}")?;
            }
        } else {
            writeln!(w, "{{")?;
            writeln!(
                w,
                "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
            )?;
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
        writeln!(
            w,
            "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;
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
        writeln!(
            w,
            "  let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
        )?;

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
                    "  let _vr_{var} = loft::keys::store(&var_{var}, &stores.allocations).get_u32_raw(var_{var}.rec, var_{var}.pos);"
                )?;
                writeln!(
                    w,
                    "  let _vc_{var} = if _vr_{var} == 0 {{ 0u32 }} else {{ loft::keys::store(&var_{var}, &stores.allocations).get_u32_raw(_vr_{var}, 4) }};"
                )?;
                writeln!(
                    w,
                    "  let _vp_{var}: *const {elem} = if _vr_{var} == 0 {{ std::ptr::null() }} else {{ loft::keys::store(&var_{var}, &stores.allocations).addr::<{elem}>(_vr_{var}, 8) as *const {elem} }};"
                )?;
            }
        }

        let needs_ret_cast = matches!(&def.returned, Type::Integer(_));
        // P244: `text`-returning natives return `loft_ffi::LoftStr` from the
        // extern, but the wrapper's declared return type (per `rust_type` with
        // `Context::Result`) is `Str`.  Capture the LoftStr as a typed local
        // and copy its bytes into `stores.scratch`, then hand back a `Str`
        // borrowed from the long-lived scratch buffer (mirrors the P205
        // pattern used by text-returning loft functions).
        let needs_text_wrap = matches!(&def.returned, Type::Text(_));
        if needs_ret_cast {
            write!(w, "  (unsafe {{ {qualified_symbol}(")?;
        } else if needs_text_wrap {
            // No type annotation: lib/server (and other native sub-crates)
            // bring their own copy of `loft_ffi`; binding by inferred type
            // sidesteps the "multiple versions of crate loft_ffi" rustc
            // E0308.  Both copies are structurally identical (`#[repr(C)]
            // pub struct LoftStr { pub ptr: *const u8, pub len: usize }`),
            // so the field reads on the next lines work either way.
            write!(w, "  let _ls = unsafe {{ {qualified_symbol}(")?;
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
                Type::Integer(_) | Type::Character => {
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
            write!(w, ") }}) as i64")?;
        } else if needs_text_wrap {
            writeln!(w, ") }};")?;
            writeln!(
                w,
                "  let _bytes: Vec<u8> = if _ls.ptr.is_null() {{ Vec::new() }} else {{ unsafe {{ std::slice::from_raw_parts(_ls.ptr, _ls.len) }}.to_vec() }};"
            )?;
            writeln!(
                w,
                "  stores.scratch.push(unsafe {{ String::from_utf8_unchecked(_bytes) }});"
            )?;
            write!(w, "  Str::new(stores.scratch.last().unwrap())")?;
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
            Type::Boolean => "u8",
            Type::Character => "u32",
            // @P310: the FFI data-pointer element width must match the
            // vector's STORAGE STRIDE, not its value range.  A plain
            // `vector<integer>` carries no `forced_size`, so it is stored at
            // the wide 8-byte stride (`byte_width() == 8`, `Type::size == 8`,
            // `vector_append` strides 8) — even though its bounds are the
            // signed-32 template (`is_wide() == false`).  Keying off
            // `vector_narrow_width()` (the same predicate the vector storage +
            // `OpGetVector`/`OpSetVector` use, `data.rs::vector_narrow_width`)
            // gives the pointer the storage-matching width: plain integer →
            // `i64` (matches the cdylib `vec<i64>` wrappers + `Canvas` pixel
            // semantics), narrow aliases keep their forced width.  (i8/i16
            // narrow vectors map to the unsigned same-width name — signedness
            // is moot, no `#native` FFI takes a narrow-int vector today.)
            Type::Integer(s) => match s.vector_narrow_width() {
                Some(1) => "u8",
                Some(2) => "i16",
                Some(4) => "i32",
                _ => "i64",
            },
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

/// Render a compile-time `known_type` as a reference expression in the
/// generated `init()` body.  For real types (0..=u16::MAX-1) this is the
/// `t{N}` let-binding that `output_init` / `output_struct` emit at the
/// time the runtime id is assigned.  For the `u16::MAX` null sentinel
/// (used by `Type::Vector(Type::Unresolved, _)` etc.) we emit the raw
/// literal — there is no let-binding for it.
fn type_id_ref(known_type: u16) -> String {
    if known_type == u16::MAX {
        "u16::MAX".to_string()
    } else {
        format!("t{known_type}")
    }
}

/// Use this to register an enum in the runtime database.
/// Plain tag variants are registered with `u16::MAX`; struct-enum variants use the variant
/// struct's `known_type` so that `ShowDb` can dispatch to the variant's fields.
fn output_enum_values(
    w: &mut dyn Write,
    d_nr: u32,
    data: &Data,
    type_id: u16,
) -> std::io::Result<()> {
    let def = data.def(d_nr);
    let e_var = format!("t{type_id}");
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
            writeln!(w, "    db.value({e_var}, \"{}\", u16::MAX);", a.name)?;
        } else {
            writeln!(w, "    db.value({e_var}, \"{}\", t{variant_type});", a.name)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod p310_vector_elem_tests {
    use super::*;
    use crate::data::IntegerSpec;

    /// @P310 regression: the FFI element pointer width must track the vector's
    /// storage stride.  `vector_elem_rust_type` takes the ELEMENT type.  Plain
    /// `vector<integer>` (signed32 template, NOT `is_wide()`) is 8-byte-stride
    /// storage → must emit `i64`, not `i32`.
    #[test]
    fn vector_elem_rust_type_matches_storage_stride() {
        // Both plain-integer templates store at 8-byte stride → i64.
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::wide())),
            "i64"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::signed32())),
            "i64",
            "plain vector<integer> must emit *const i64 (storage is 8-byte stride) — @P310"
        );
        // Narrow aliases keep their forced storage width.
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::i32())),
            "i32"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::u16())),
            "i16"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::i16())),
            "i16"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::u8())),
            "u8"
        );
        assert_eq!(
            Output::vector_elem_rust_type(&Type::Integer(IntegerSpec::i8())),
            "u8"
        );
        // Non-integer element types unchanged.
        assert_eq!(Output::vector_elem_rust_type(&Type::Single), "f32");
        assert_eq!(Output::vector_elem_rust_type(&Type::Float), "f64");
    }
}
