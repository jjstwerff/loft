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

use crate::data::{Context, Data, DefType, Type};
use crate::database::Stores;
use crate::generation::{Output, rust_type};
use std::collections::{BTreeSet, HashSet};

/// @PLAN54 Arc N / N2 (lean interface) — generate the loft-source **interface** a
/// script adopts to call a native library: the public type definitions the
/// exported functions reference (transitively) plus, per exported function, a
/// `#native "loft_shared_…"` forward declaration.
///
/// This is the lean half of "an interpreted script calling a compiled library":
/// the script parses only this interface (type layouts + signatures + dispatch
/// symbols), **never the library bodies**, and gets the library's types defined
/// in the library's own order — so type ids align by construction rather than by
/// the caller redefining the types identically.  (A binary schema load — the D2a
/// cache — is the robust successor that also covers non-public ordering; this
/// source form covers the common case where the public types are the only ones.)
#[must_use]
pub fn generate_interface(data: &Data, export_set: &HashSet<u32>) -> String {
    use std::fmt::Write as _;
    // Referenced struct/enum defs, transitively, kept in definition order (BTreeSet
    // on `d_nr`) so the script registers them in the library's order.
    let mut types: BTreeSet<u32> = BTreeSet::new();
    for &d in export_set {
        let def = data.def(d);
        for a in def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && !a.name.starts_with("__"))
        {
            collect_type_defs(data, &a.typedef, &mut types);
        }
        collect_type_defs(data, def.returned(), &mut types);
    }

    let mut src = String::new();
    for &t in &types {
        emit_type_def(data, t, &mut src);
        src.push('\n');
    }
    let mut fns: Vec<u32> = export_set.iter().copied().collect();
    fns.sort_unstable();
    for d in fns {
        let def = data.def(d);
        let name = def.name().strip_prefix("n_").unwrap_or(def.name());
        let params: Vec<String> = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && !is_text_work_buffer(&a.typedef) && !a.name.starts_with("__"))
            .map(|a| format!("{}: {}", a.name, a.typedef.name(data)))
            .collect();
        let ret = def.returned();
        let ret_clause = if matches!(ret, Type::Void | Type::Null) {
            String::new()
        } else {
            format!(" -> {} not null", ret.name(data))
        };
        let _ = writeln!(src, "pub fn {name}({}){ret_clause};", params.join(", "));
        let _ = writeln!(src, "#native \"loft_shared_{}\"", def.name());
    }
    src
}

/// @PLAN54 Arc N / N3 — mark a library's functions for native dispatch.  Of the
/// `candidates` (a library's functions — e.g. all functions from a `use`d
/// library's source), the **shared-store-dispatchable** subset has its
/// `def.native` set to the bridge symbol `loft_shared_<name>`.  Returns that
/// subset (the cdylib export set).
///
/// This is the hook that turns a *normal* library function (a body, no
/// hand-written `#native`) into a native-dispatched one: once `def.native` is set,
/// `byte_code` routes every call to it through `OpStaticCall` (codegen.rs), the
/// stub is registered by `register_native_stubs`, and `wire_shared_native_fns`
/// wires the real bridge after the cdylib loads.  **Call before `byte_code`.**
pub fn mark_native_exports(data: &mut Data, candidates: &HashSet<u32>) -> HashSet<u32> {
    let exportable: HashSet<u32> = crate::native_gate::shared_store_dispatchable(data)
        .intersection(candidates)
        .copied()
        .collect();
    for &d in &exportable {
        let sym = format!("loft_shared_{}", data.def(d).name());
        data.def_mut(d).native = sym;
    }
    exportable
}

/// Add to `types` (transitively, in definition order) the struct/enum defs that
/// loft type `t` references.
fn collect_type_defs(data: &Data, t: &Type, types: &mut BTreeSet<u32>) {
    // The struct/enum `def_nr` this type references, if any.  Reference / Enum /
    // Sorted / Index / Hash / Spacial all carry a leading element-struct `def_nr`;
    // a Vector recurses into its element type; everything else is a leaf.
    let d = match t {
        Type::Reference(d, _)
        | Type::Enum(d, _, _)
        | Type::Sorted(d, _, _)
        | Type::Index(d, _, _)
        | Type::Hash(d, _, _)
        | Type::Spacial(d, _, _) => *d,
        Type::Vector(elm, _) => {
            collect_type_defs(data, elm, types);
            return;
        }
        _ => return,
    };
    if !types.insert(d) {
        return; // already collected (also breaks cycles)
    }
    let def = data.def(d);
    for a in def.attributes() {
        collect_type_defs(data, &a.typedef, types);
    }
    // enum variants carry their own fields (DefType::EnumValue children)
    for v in data.children_of(d) {
        for a in data.def(v).attributes() {
            collect_type_defs(data, &a.typedef, types);
        }
    }
}

/// Emit the loft-source definition of struct/enum `d_nr` into `src`.
fn emit_type_def(data: &Data, d_nr: u32, src: &mut String) {
    use std::fmt::Write as _;
    let def = data.def(d_nr);
    match def.def_type() {
        DefType::Struct => {
            let _ = writeln!(src, "struct {} {{", def.name());
            for a in def.attributes() {
                let _ = writeln!(src, "    {}: {},", a.name, a.typedef.name(data));
            }
            src.push_str("}\n");
        }
        DefType::Enum => {
            let variants: Vec<u32> = data.children_of(d_nr).collect();
            let parts: Vec<String> = variants
                .iter()
                .map(|&v| {
                    let vd = data.def(v);
                    let vname = vd.name().rsplit('.').next().unwrap_or(vd.name());
                    // Each variant carries an auto-added `enum` discriminant field
                    // (definitions.rs) — skip it; emit only the user-declared fields.
                    let fields: Vec<String> = vd
                        .attributes()
                        .iter()
                        .filter(|a| a.name != "enum")
                        .map(|a| format!("{}: {}", a.name, a.typedef.name(data)))
                        .collect();
                    if fields.is_empty() {
                        vname.to_string()
                    } else {
                        format!("{vname} {{ {} }}", fields.join(", "))
                    }
                })
                .collect();
            let _ = writeln!(src, "enum {} {{ {} }}", def.name(), parts.join(", "));
        }
        _ => {}
    }
}

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
    /// or a boolean as `0/1`.  Junk for non-scalar slots.
    pub scalar: i64,
    /// Reference payload: the raw stack `DbRef` for a `vector`/`reference` slot
    /// (passed through unchanged — the `--native` body expects the same indirect
    /// form the interpreter holds).  Junk for non-ref slots.
    pub dbref: crate::keys::DbRef,
    /// Text payload pointer: for a `text` arg, the UTF-8 bytes (borrowed from the
    /// caller's store for the call's duration — `--native` takes `&str`).  Null
    /// for non-text slots.
    pub text_ptr: *const u8,
    /// Text payload length (bytes), paired with `text_ptr`.
    pub text_len: usize,
}

impl LibArg {
    /// All-zero slot — the spread base so each `LibArg` literal sets only the one
    /// field its type uses (`LibArg { scalar: x, ..LibArg::ZERO }`, etc.).
    pub const ZERO: LibArg = LibArg {
        scalar: 0,
        dbref: crate::keys::DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        },
        text_ptr: std::ptr::null(),
        text_len: 0,
    };
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
        let _ = out.output_native_library(&mut buf, 0, data.definitions(), entry);
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
            let _ = write!(fwd, ", {var}");
        } else if is_text_work_buffer(&a.typedef) {
            // text_return work buffer (`&mut String`) — own a LOCAL String, pass
            // `&mut`.  The returned `Str` points into it; the return handler copies
            // the bytes into the shared store's scratch before this frame drops.
            let _ = writeln!(body, "    let mut {var}: String = String::new();");
            let _ = write!(fwd, ", &mut {var}");
        } else {
            let ty = rust_type(&a.typedef, &Context::Argument);
            let read = bridge_read(&a.typedef, &format!("a[{slot}]"));
            let _ = writeln!(body, "    let {var}: {ty} = {read};");
            slot += 1;
            let _ = write!(fwd, ", {var}");
        }
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
        // Text arg → `&str` borrowed from the slot's (store-backed) bytes.
        Type::Text(_) => format!(
            "unsafe {{ std::str::from_utf8_unchecked(std::slice::from_raw_parts({slot}.text_ptr, {slot}.text_len)) }}"
        ),
        // Plain (tag-only) enum → a `u8` tag riding in the scalar slot.
        Type::Enum(_, false, _) => format!("{slot}.scalar as u8"),
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
        // Plain (tag-only) enum returns a `u8` tag → widen into the scalar slot.
        Type::Integer(_) | Type::Character | Type::Boolean | Type::Enum(_, false, _) => {
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
        // Text return: the inner fn returns a `Str` pointing into a local work
        // `String` (about to drop).  Copy the bytes into the shared store's scratch
        // (stable, outlives this frame) and point `ret` at that — mirroring the
        // legacy `bridge_push_str`.  The dispatcher reads ptr+len back onto the stack.
        Type::Text(_) => format!(
            "let __r = ({expr});\n    \
             let __t = __r.str();\n    \
             let __st: &mut Stores = unsafe {{ &mut *cell.get() }};\n    \
             __st.scratch.clear();\n    \
             __st.scratch.push(__t.to_string());\n    \
             let __s = &__st.scratch[0];\n    \
             unsafe {{ (*ret).text_ptr = __s.as_ptr(); (*ret).text_len = __s.len(); }}"
        ),
        _ => "compile_error!(\"unsupported shared-store return type\");".to_string(),
    }
}

/// Is `t` a `text_return` work buffer — a `&mut String` the inner fn appends the
/// result into (loft IR type `RefVar(Text)`)?  The bridge owns a local `String`
/// for each and passes `&mut`.  Shared with `native_gate` (the gate admits exactly
/// these as the one allowed `__`-prefixed param, and only for a text-returning fn).
pub(crate) fn is_text_work_buffer(t: &Type) -> bool {
    matches!(t, Type::RefVar(inner) if matches!(**inner, Type::Text(_)))
}

/// @PLAN54 Arc N / N3 — locate the running build's `libloft.rlib` + its sibling
/// `deps/` directory, for linking an auto-generated cdylib against the **same**
/// libloft this process links (so `Stores`/`DbRef`/`LibArg` are ABI-identical).
///
/// Works in both contexts the cdylib must build in: a real `cargo run --bin loft`
/// (an unhashed `target/<prof>/libloft.rlib`, or `deps/`) and an integration test
/// (a hashed `libloft-<hash>.rlib` in the test binary's `deps/`).  Returns the
/// chosen rlib path and the `deps/` dir to add to the link search path.
#[must_use]
pub fn find_loft_rlib() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    // Search the exe dir and its `deps/` for `libloft.rlib` (unhashed) or the
    // newest `libloft-<hash>.rlib` (hashed, as cargo emits for dependencies).
    for dir in [exe_dir.clone(), exe_dir.join("deps")] {
        if !dir.is_dir() {
            continue;
        }
        let exact = dir.join("libloft.rlib");
        if exact.exists() {
            return Some((exact, dir));
        }
        let hashed = std::fs::read_dir(&dir)
            .ok()?
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("libloft-") && has_rlib_ext(&n)
            })
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
            .map(|e| e.path());
        if let Some(rlib) = hashed {
            return Some((rlib, dir));
        }
    }
    None
}

/// Does filename `n` have a (case-insensitive) `.rlib` extension?
fn has_rlib_ext(n: &str) -> bool {
    std::path::Path::new(n)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("rlib"))
}

/// `--extern name=path` flags for the optional feature-dep rlibs (random/png/…) in
/// `deps` that the generated stdlib code may reference — every `libX-<hash>.rlib`
/// except `libloft*` (which is passed explicitly as `--extern loft=`).
fn extra_externs(deps: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(deps) else {
        return out;
    };
    for e in entries.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !n.starts_with("lib") || !has_rlib_ext(&n) || n.starts_with("libloft") {
            continue;
        }
        if let Some(stem) = n
            .strip_prefix("lib")
            .and_then(|s| s.rsplit_once('-'))
            .map(|x| x.0)
        {
            out.push((stem.to_string(), e.path()));
        }
    }
    out
}

/// Platform cdylib filename for `stem` (`lib<stem>.so` / `.dylib` / `<stem>.dll`).
#[must_use]
pub fn platform_cdylib_name(stem: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{stem}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

/// @PLAN54 Arc N / N3 — generate **and compile** the shared-store cdylib for
/// `export_set` into `out_dir`, returning the built cdylib path.  This is the
/// production build step `use <lib>` runs after `byte_code`: it locates the
/// running build's `libloft.rlib` ([`find_loft_rlib`]), writes `lib.rs`
/// ([`generate_shared_cdylib_lib_rs`]), and invokes `rustc` with the `--native`
/// flag set (cdylib, edition 2024, `--extern loft=` + feature-dep externs).
///
/// # Errors
/// Returns a message if the rlib can't be found, the source can't be written,
/// `rustc` can't be launched, or compilation fails (the message includes the
/// `rustc` stderr tail and the kept `lib.rs` path for inspection).
pub fn build_shared_cdylib(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    out_dir: &std::path::Path,
    stem: &str,
) -> Result<std::path::PathBuf, String> {
    let (rlib, deps) = find_loft_rlib().ok_or("libloft.rlib not found for this build")?;
    let src = generate_shared_cdylib_lib_rs(data, stores, export_set);
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let rs = out_dir.join(format!("{stem}.rs"));
    std::fs::write(&rs, &src).map_err(|e| format!("write {}: {e}", rs.display()))?;
    let so = out_dir.join(platform_cdylib_name(stem));

    let mut cmd = std::process::Command::new("rustc");
    cmd.arg("--edition=2024")
        .arg("-C")
        .arg("debuginfo=0")
        .arg("-C")
        .arg("opt-level=0")
        .arg("--crate-type")
        .arg("cdylib")
        .arg("-o")
        .arg(&so)
        .arg(&rs)
        .arg("--extern")
        .arg(format!("loft={}", rlib.display()))
        .arg("-L")
        .arg(&deps);
    for (name, path) in extra_externs(&deps) {
        cmd.arg("--extern")
            .arg(format!("{name}={}", path.display()));
    }
    let output = cmd
        .output()
        .map_err(|e| format!("launch rustc: {e} (is the Rust toolchain installed?)"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(30).collect();
        let tail: String = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(format!(
            "cdylib compile failed (source kept at {}):\n{tail}",
            rs.display()
        ));
    }
    Ok(so)
}
