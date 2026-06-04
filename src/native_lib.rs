// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 Arc N / N2 — auto-generate a native cdylib from a library's functions.
//!
//! In the native-library execution model (C71) a stable library compiles to a
//! cdylib and the interpreter dispatches calls to it (the stdlib `#native` path,
//! `OpStaticCall` → dlsym).  This module generates that cdylib's `lib.rs`.  Two
//! dispatch ABIs, by what crosses the boundary:
//!
//! **Scalar slice** (`generate_cdylib_lib_rs`, `scalar_dispatchable`): params +
//! return are scalars, so no store reference crosses.  The export wrapper
//! `loft_n_<name>(scalars) -> ret` stands up a **per-call** `UnsafeCell<Stores>` +
//! `init()` and forwards — safe because a scalar-return body cannot leak a store
//! ref out (any internal allocation is contained and dropped with the cell).  This
//! is ABI-identical to a hand-written scalar `#native` symbol, so it reuses the
//! existing dispatch wholesale.
//!
//! **Store-touching slice** (`generate_shared_cdylib_lib_rs`,
//! `shared_store_dispatchable`): a non-scalar value (`vector`/`reference`) crosses
//! the boundary.  Because an auto-generated cdylib **links libloft**, its `Stores`
//! / `DbRef` are the *same Rust types* as the interpreter's — so the bridge
//! **shares the caller's real `Stores` by pointer** (zero-marshalling, C71's
//! promise) rather than going through the `LoftStore` FFI handle (that handle
//! exists for hand-written cdylibs that don't link `Stores`).  The bridge wrapper
//! `loft_shared_n_<name>(stores: *mut Stores, args, n, ret)` casts `*mut Stores` →
//! `&UnsafeCell<Stores>` (`UnsafeCell` is `repr(transparent)`) and forwards — **no
//! per-call cell, no `init`, no marshalling** (the caller's store is live; `DbRef`
//! args are already valid in it).  Args/return are passed through the uniform
//! [`LibArg`] slot.

use crate::data::{Context, Data, Type};
use crate::database::Stores;
use crate::generation::{Output, rust_type};
use std::collections::HashSet;

/// @PLAN54 Arc N — a uniform 16/24-byte argument/return slot for the
/// **shared-store** native-library bridge ([`generate_shared_cdylib_lib_rs`]).
///
/// The bridge knows each slot's type from the function signature, so no tag is
/// needed: a scalar reads/writes `.scalar` (an `i64`, or float bits, or `0/1` for
/// bool), a `vector`/`reference` reads/writes `.dbref`.  `#[repr(C)]` so the
/// interpreter and the generated cdylib — **both linking this exact type from
/// libloft** — agree on layout with no marshalling.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LibArg {
    /// Scalar payload: an `i64`, `f64::to_bits() as i64`, `f32::to_bits() as i64`,
    /// or a boolean as `0/1`.  Junk for ref/vector slots.
    pub scalar: i64,
    /// Reference payload: the raw stack `DbRef` for a `vector`/`reference` slot
    /// (passed through unchanged — the `--native` body expects the same indirect
    /// form the interpreter holds).  Junk for scalar slots.
    pub dbref: crate::keys::DbRef,
}

/// Build the shared `--native` program (header + `init` + only the functions
/// reachable from `entry` + their deps) for `data`.  Both cdylib generators
/// append their export wrappers to this.
fn emit_program(data: &Data, stores: &Stores, entry: &[u32]) -> String {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut out = Output {
            data,
            stores,
            counter: 0,
            indent: 0,
            def_nr: 0,
            declared: HashSet::new(),
            reachable: HashSet::new(),
            loop_stack: Vec::new(),
            next_format_count: 0,
            yield_collect: false,
            yield_collect_text: false,
            fn_ref_context: false,
            i32_literal_context: false,
            tuple_text_to_string: false,
            coroutine_persistent_vars: HashSet::new(),
            call_stack_prefix: None,
            wasm_browser: false,
        };
        // Only the exported functions + their transitive deps (header + init + the
        // reachable subset) — exactly what a `--native` binary emits from `n_main`,
        // so unreachable operator stubs never surface.
        let _ = out.output_native_reachable(&mut buf, 0, data.definitions(), entry);
    }
    String::from_utf8(buf).unwrap_or_default()
}

/// Generate the cdylib `lib.rs` source for `data`'s **scalar** export set: the
/// `--native` program for the functions in `export_set` **and their transitive
/// dependencies**, plus a `#[no_mangle] pub extern "C"` export wrapper for each
/// `export_set` function (the symbols the interpreter will dispatch to).
///
/// `export_set` is the **library's own public, scalar-dispatchable functions** —
/// the cdylib's exported API, not the whole stdlib.  It serves as both the
/// reachability roots (so only those functions + their deps are emitted) and the
/// set to wrap.  The caller computes it as `library_pub_fns ∩ scalar_dispatchable`
/// (operator definitions and stdlib internals are deps, inlined/emitted as needed,
/// never wrapped).
#[must_use]
pub fn generate_cdylib_lib_rs(data: &Data, stores: &Stores, export_set: &HashSet<u32>) -> String {
    let entry: Vec<u32> = export_set.iter().copied().collect();
    let mut src = emit_program(data, stores, &entry);
    for &d in export_set {
        src.push('\n');
        src.push_str(&export_wrapper(data, d));
    }
    src
}

/// Generate the cdylib `lib.rs` source for `data`'s **shared-store** export set:
/// the `--native` program for `export_set` + deps, plus a uniform shared-store
/// bridge ([`LibArg`] ABI) per exported function.  Use this when a non-scalar
/// value (`vector`/`reference`) crosses the boundary; `export_set` is computed as
/// `library_pub_fns ∩ shared_store_dispatchable`.
#[must_use]
pub fn generate_shared_cdylib_lib_rs(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
) -> String {
    let entry: Vec<u32> = export_set.iter().copied().collect();
    let mut src = emit_program(data, stores, &entry);
    for &d in export_set {
        src.push('\n');
        src.push_str(&shared_bridge_wrapper(data, d));
    }
    src
}

/// The `#[no_mangle] pub extern "C"` **scalar** export wrapper for
/// scalar-dispatchable function `d_nr` (inner `--native` fn = its name, e.g.
/// `n_double`).  The export symbol is `loft_<name>` (distinct from the inner).
fn export_wrapper(data: &Data, d_nr: u32) -> String {
    let def = data.def(d_nr);
    let inner = def.name(); // e.g. "n_double"
    let ret = rust_type(def.returned(), &Context::Result);
    // Positional params (`a0`, `a1`, …) so wrapper arg names never clash with
    // anything; types match the inner function's argument-context signature.
    let params: Vec<(String, String)> = def
        .attributes()
        .iter()
        .filter(|a| !a.name.starts_with("__"))
        .enumerate()
        .map(|(i, a)| (format!("a{i}"), rust_type(&a.typedef, &Context::Argument)))
        .collect();
    let sig: Vec<String> = params.iter().map(|(n, t)| format!("{n}: {t}")).collect();
    let mut fwd = String::new();
    for (n, _) in &params {
        fwd.push_str(", ");
        fwd.push_str(n);
    }
    format!(
        "#[unsafe(no_mangle)]\n\
         pub extern \"C\" fn loft_{inner}({}) -> {ret} {{\n    \
         let cell = std::cell::UnsafeCell::new(Stores::new());\n    \
         init(&cell);\n    \
         {inner}(&cell{fwd})\n\
         }}\n",
        sig.join(", "),
    )
}

/// The `#[no_mangle] pub extern "C"` **shared-store** bridge for
/// shared-store-dispatchable function `d_nr`.  The export symbol is
/// `loft_shared_<name>`.  It casts the caller's `*mut Stores` to
/// `&UnsafeCell<Stores>` (no per-call cell — the caller's store is live), then,
/// in the inner fn's attribute order:
/// - a **visible** parameter is read from the next [`LibArg`] slot by its type;
/// - a **hidden** destination parameter (`Attribute::hidden`, appended by
///   `ref_return` for a non-scalar return) is **allocated here** in the shared
///   store (`null_named` + `OpDatabase(<type_id>)`), exactly as a `--native`
///   caller would — so the caller-side dispatcher only ever passes the public args.
///
/// Finally it forwards to the inner `--native` fn and writes the return.
fn shared_bridge_wrapper(data: &Data, d_nr: u32) -> String {
    use std::fmt::Write as _;
    let def = data.def(d_nr);
    let inner = def.name(); // e.g. "n_vec_sum"

    let mut body = String::new();
    let mut fwd = String::new();
    let mut slot = 0usize; // next public-arg LibArg slot
    for (i, a) in def.attributes().iter().enumerate() {
        let var = format!("p{i}");
        if a.hidden {
            // ref_return destination — allocate it in the SHARED store, mirroring
            // a `--native` caller (`stores.null_named` + `OpDatabase(<type_id>)`).
            let tid = hidden_dest_type_id(data, &a.typedef);
            let _ = writeln!(
                body,
                "    let mut {var}: DbRef = unsafe {{ (&mut *cell.get()).null_named(\"__shared_dest\") }};"
            );
            let _ = writeln!(body, "    {var} = OpDatabase(cell, {var}, {tid}i32);");
        } else {
            let ty = rust_type(&a.typedef, &Context::Argument);
            let read = bridge_read(&a.typedef, &format!("a[{slot}]"));
            let _ = writeln!(body, "    let {var}: {ty} = {read};");
            slot += 1;
        }
        let _ = write!(fwd, ", {var}");
    }

    let call = format!("{inner}(cell{fwd})");
    let ret_stmt = bridge_write_ret(def.returned(), &call);

    format!(
        "#[unsafe(no_mangle)]\n\
         pub extern \"C\" fn loft_shared_{inner}(\n    \
         stores: *mut Stores,\n    \
         args: *const loft::native_lib::LibArg,\n    \
         n: usize,\n    \
         ret: *mut loft::native_lib::LibArg,\n\
         ) {{\n    \
         let cell = unsafe {{ &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) }};\n    \
         let a = unsafe {{ std::slice::from_raw_parts(args, n) }};\n    \
         let _ = ({slot}, a);\n\
         {body}    \
         {ret_stmt}\n\
         }}\n",
    )
}

/// The schema type id for a hidden `ref_return` destination of loft type `t`,
/// for the `OpDatabase(cell, ref, <id>)` allocation.  Vectors resolve via the
/// `main_vector<elm>` schema name (the same key `--native`'s `output_alloc_heap`
/// uses).  Other aggregates (struct `reference`, data-`enum`) need their own
/// type id and are not yet handled (the gate excludes them).
fn hidden_dest_type_id(data: &Data, t: &Type) -> u16 {
    match t {
        Type::Vector(elm, _) => {
            let elm_name = elm.name(data);
            data.name_type(&format!("main_vector<{elm_name}>"), 0)
        }
        _ => u16::MAX,
    }
}

/// Rust expression reading an argument of loft type `t` out of `LibArg` slot
/// `slot` (e.g. `"a[0]"`), at the inner fn's argument-context type.
fn bridge_read(t: &Type, slot: &str) -> String {
    match t {
        Type::Integer(s) if s.forced_size.is_none() => format!("{slot}.scalar"),
        Type::Integer(_) => format!("{slot}.scalar as {}", rust_type(t, &Context::Argument)),
        Type::Character => format!("{slot}.scalar as i32"),
        Type::Boolean => format!("{slot}.scalar != 0"),
        Type::Float => format!("f64::from_bits({slot}.scalar as u64)"),
        Type::Single => format!("f32::from_bits({slot}.scalar as u32)"),
        Type::Vector(_, _)
        | Type::Reference(_, _)
        | Type::Enum(_, true, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Spacial(_, _, _) => format!("{slot}.dbref"),
        // Not bridge-able — the gate excludes these, so this is unreachable for a
        // shared-store-dispatchable function; emit a clearly-wrong token so a gate
        // bug surfaces as a compile error rather than silent corruption.
        _ => "compile_error!(\"unsupported shared-store arg type\")".to_string(),
    }
}

/// Rust statement writing the inner-fn result `expr` (of loft return type `t`)
/// into the `ret` [`LibArg`] slot.
fn bridge_write_ret(t: &Type, expr: &str) -> String {
    match t {
        Type::Void | Type::Null => format!("let _ = {expr};"),
        Type::Integer(_) | Type::Character | Type::Boolean => {
            format!("unsafe {{ (*ret).scalar = ({expr}) as i64; }}")
        }
        Type::Float => format!("unsafe {{ (*ret).scalar = ({expr}).to_bits() as i64; }}"),
        Type::Single => format!("unsafe {{ (*ret).scalar = ({expr}).to_bits() as i64; }}"),
        Type::Vector(_, _)
        | Type::Reference(_, _)
        | Type::Enum(_, true, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Spacial(_, _, _) => format!("unsafe {{ (*ret).dbref = ({expr}); }}"),
        _ => "compile_error!(\"unsupported shared-store return type\");".to_string(),
    }
}
