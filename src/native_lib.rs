// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN11 Arc N / N2 — auto-generate a native cdylib from a library's functions.
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
use crate::generation::{Output, returns_owned_string, rust_type};
use std::collections::{BTreeSet, HashSet};

/// @PLN11 Arc N / N2 (lean interface) — generate the loft-source **interface** a
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
            .filter(|a| !a.hidden && !is_text_work_buffer(&a.typedef))
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
    let decl_dups = crate::generation::duplicate_fn_names(data);
    for d in fns {
        let def = data.def(d);
        let name = def.name().strip_prefix("n_").unwrap_or(def.name());
        let params: Vec<String> = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && !is_text_work_buffer(&a.typedef))
            .map(|a| format!("{}: {}", a.name, a.typedef.name(data)))
            .collect();
        let ret = def.returned();
        let ret_clause = if matches!(ret, Type::Void | Type::Null) {
            String::new()
        } else {
            format!(" -> {} not null", ret.name(data))
        };
        let _ = writeln!(src, "pub fn {name}({}){ret_clause};", params.join(", "));
        let _ = writeln!(
            src,
            "#native \"loft_shared_{}\"",
            crate::generation::disambiguated_fn_ident(&decl_dups, def)
        );
    }
    src
}

/// @PLN11 Arc N / N3 — mark a library's functions for native dispatch.  Of the
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
    mark_exports(data, &exportable);
    exportable
}

/// @PLN11 Arc N / N3 (Step 2) — set `def.native = "loft_shared_<name>"` on each
/// function in `export` (which `byte_code` then routes through `OpStaticCall`).
/// Split out from [`mark_native_exports`] so the **build-before-mark** flow can
/// mark *only after* the cdylib compiles — a build failure simply never marks, so
/// the library interprets (Step 2's invariant: a library that can't compile native
/// silently interprets, no `exit`, no `OpStaticCall` to an unbuilt symbol).
pub fn mark_exports(data: &mut Data, export: &HashSet<u32>) {
    let dups = crate::generation::duplicate_fn_names(data);
    for &d in export {
        // The disambiguated ident (#305) — must equal the cdylib wrapper's
        // exported symbol, or `wire_shared_native_fns`' dlsym misses it.
        let sym = format!(
            "loft_shared_{}",
            crate::generation::disambiguated_fn_ident(&dups, data.def(d))
        );
        data.def_mut(d).native = sym;
    }
}

/// @PLN11 Arc N / N3 (Step 2) — the cdylib **export set** for the library at
/// `pkg_dir`, computed **without marking** (`&Data`, not `&mut`): the library's
/// top-level, user-named, `pub` functions (the dispatch-target invariant — see
/// [`mark_library_native`]) intersected with the shared-store-dispatchable gate.
/// The build-before-mark flow builds the cdylib from this set, then calls
/// [`mark_exports`] only on success.
#[must_use]
pub fn library_export_set(data: &Data, pkg_dir: &str) -> HashSet<u32> {
    let candidates: HashSet<u32> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type(), DefType::Function)
                && def.pub_visible
                && !is_synthetic_name(def.name())
                && def.position().file.starts_with(pkg_dir)
        })
        .collect();
    crate::native_gate::shared_store_dispatchable(data)
        .intersection(&candidates)
        .copied()
        .collect()
}

/// @PLN11 Arc N / N3 — mark a `use`d library's **public API** functions native.
///
/// **Invariant:** a dispatch target is a function the consuming script can directly
/// *name and `Call`* — a top-level, user-named, `pub` function owned by the package
/// at `pkg_dir` (by `def.position().file` prefix — the same ownership guard the
/// manifest path uses).  [`mark_native_exports`] then marks the
/// shared-store-dispatchable subset.
///
/// The candidate filter must exclude **synthetic** functions, not just lambdas: a
/// `pub fn`'s parse sprays `pub_visible` over every def it creates (`parser/mod.rs`),
/// so a nested lambda (`__lambda_N`) is also `pub_visible` — but it is a *fn-ref*
/// target the script cannot name, never a direct-`Call` dispatch target.  The
/// `__`-prefix is the codebase's synthetic-name convention (same marker
/// `native_gate` uses for synthetic params), so excluding it covers the whole class
/// of compiler-generated functions, not the lambda instance.  (Private helpers are
/// already excluded by `pub_visible`; they ride into the cdylib as reachable deps.)
pub fn mark_library_native(data: &mut Data, pkg_dir: &str) -> HashSet<u32> {
    let export = library_export_set(data, pkg_dir);
    mark_exports(data, &export);
    export
}

/// Is `stored_name` (an `n_<name>` definition name) a compiler-generated
/// (synthetic) function — i.e. its user-facing name starts with `__` (lambdas
/// `__lambda_N`, and any future synthetic kind)?  Such functions are never a
/// script-callable public API, so they are not auto-native dispatch targets.
fn is_synthetic_name(stored_name: &str) -> bool {
    stored_name
        .strip_prefix("n_")
        .unwrap_or(stored_name)
        .starts_with("__")
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

/// @PLN11 Arc N — a uniform 16/24-byte argument/return slot for the
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
        let mut out = Output::new(data, stores);
        // @PLN26 phase 2 — a cdylib that calls a `[native] crate` package emits
        // that package's fns as C-ABI `extern "C"` decls naming `loft_ffi::Loft*`,
        // its `.so` linked (sealing the package's whole Rust crate graph), exactly
        // as the executable native path does.  The SAME `native_cabi_enabled()`
        // gate drives the matching link flags in `build_shared_cdylib`, so codegen
        // and link never disagree.  No-op when the library uses no native package.
        out.native_cabi = native_cabi_link_enabled();
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
    let dups = crate::generation::duplicate_fn_names(data);
    for &d in export_set {
        src.push('\n');
        src.push_str(&export_wrapper(data, &dups, d));
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
    let dups = crate::generation::duplicate_fn_names(data);
    for &d in export_set {
        src.push('\n');
        src.push_str(&shared_bridge_wrapper(data, &dups, d));
    }
    src
}

/// The `#[no_mangle] pub extern "C"` **scalar** export wrapper for
/// scalar-dispatchable function `d_nr` (inner `--native` fn = its name, e.g.
/// `n_double`).  The export symbol is `loft_<name>` (distinct from the inner).
fn export_wrapper(data: &Data, dups: &HashSet<String>, d_nr: u32) -> String {
    let def = data.def(d_nr);
    // The emitted program disambiguates colliding fn names (#305) — the
    // wrapper must call (and be named after) the SAME identifier.
    let inner = crate::generation::disambiguated_fn_ident(dups, def); // e.g. "n_double"
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
fn shared_bridge_wrapper(data: &Data, dups: &HashSet<String>, d_nr: u32) -> String {
    use std::fmt::Write as _;
    let def = data.def(d_nr);
    // Same identifier the emitted program uses for this fn (#305).
    let inner = crate::generation::disambiguated_fn_ident(dups, def); // e.g. "n_vec_sum"

    let mut body = String::new();
    let mut fwd = String::new();
    let mut slot = 0usize; // next public-arg LibArg slot
    let ret_text = matches!(def.returned(), Type::Text(_));
    for (i, a) in def.attributes().iter().enumerate() {
        let var = format!("p{i}");
        // #303 — the ONE marshallability judgment (shared with the gate and the
        // wire-time signature builder).  The gate guarantees `Some` for every
        // attribute of a marked function; a `None` here means the judgment
        // drifted between marking and generation — fail loudly, never emit a
        // bridge whose ABI the dispatcher would disagree with.
        let kind = crate::native_gate::classify_bridge_attr(a, ret_text).unwrap_or_else(|| {
            panic!(
                "shared bridge for {}: attribute '{}' is not bridge-classifiable \
                 but the function was marked dispatchable (gate/generator divergence)",
                def.name(),
                a.name,
            )
        });
        match kind {
            crate::native_gate::BridgeAttrKind::HiddenDest => {
                // ref_return destination.  A body-bearing caller pre-allocates
                // one and the dispatcher forwards its slot (#311) — write the
                // result into THAT record (the caller's frame owns and frees
                // it; a bridge-local allocation orphaned it, one leaked store
                // per call).  Fallbacks allocate, mirroring a `--native` caller
                // (`null_named` + `OpDatabase(<type_id>)`): a no-body `#native`
                // decl caller forwards no slot (`{slot} >= n`), and a null
                // incoming ref means no usable record arrived.
                let tname = hidden_dest_type_name(data, &a.typedef).unwrap_or_else(|| {
                    panic!(
                        "shared bridge for {}: hidden dest '{}' has no shared-store type name",
                        def.name(),
                        a.name
                    )
                });
                let _ = writeln!(
                    body,
                    "    let mut {var}: DbRef = if {slot} < n {{ a[{slot}].dbref }} else {{ DbRef {{ store_nr: 0, rec: 0, pos: 0 }} }};"
                );
                let _ = writeln!(body, "    if {var}.rec == 0 && {var}.pos == 0 {{");
                let _ = writeln!(
                    body,
                    "        let _tid{slot} = unsafe {{ (&*cell.get()) }}.name({tname:?});"
                );
                let _ = writeln!(
                    body,
                    "        assert!(_tid{slot} != u16::MAX, \"shared bridge: type {tname} not registered in the caller store\");"
                );
                let _ = writeln!(
                    body,
                    "        {var} = unsafe {{ (&mut *cell.get()).null_named(\"__shared_dest\") }};"
                );
                let _ = writeln!(
                    body,
                    "        {var} = OpDatabase(cell, {var}, i32::from(_tid{slot}));"
                );
                let _ = writeln!(body, "    }}");
                slot += 1;
                let _ = write!(fwd, ", {var}");
            }
            crate::native_gate::BridgeAttrKind::WorkText => {
                // text_return work buffer (`&mut String`) — own a LOCAL String, pass
                // `&mut`.  The returned `Str` points into it; `bridge_write_ret` copies
                // the bytes into the caller-owned `bridge_text_dest` record (@PLN10
                // dest-passing) before this frame drops.
                let _ = writeln!(body, "    let mut {var}: String = String::new();");
                let _ = write!(fwd, ", &mut {var}");
            }
            crate::native_gate::BridgeAttrKind::Marshal => {
                let ty = rust_type(&a.typedef, &Context::Argument);
                let read = bridge_read(&a.typedef, &format!("a[{slot}]"));
                let _ = writeln!(body, "    let {var}: {ty} = {read};");
                slot += 1;
                let _ = write!(fwd, ", {var}");
            }
        }
    }

    let call = format!("{inner}(cell{fwd})");
    let ret_stmt = bridge_write_ret(def.returned(), &call, returns_owned_string(def));

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
/// The SHARED-STORE type NAME for a hidden destination attr — resolved at
/// BRIDGE RUNTIME via `Stores::name` in the caller's store, because the
/// library's compile-time type IDS live in a different id space than the
/// caller's (@PLAN59: a lib-side constant id produced `claim(size=0)` /
/// "Incomplete record" aborts once struct dests became universal).
fn hidden_dest_type_name(data: &Data, t: &Type) -> Option<String> {
    match t {
        Type::Vector(elm, _) => Some(format!("main_vector<{}>", elm.name(data))),
        Type::Reference(td, _) | Type::Enum(td, true, _) => Some(data.def(*td).name().to_string()),
        _ => None,
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
/// into the `ret` [`LibArg`] slot.  `inner_owned` is [`returns_owned_string`] for
/// the inner fn: text producers split into those returning an owned `String`
/// (nwb / FFI-direct / curated) and those returning a buffer-backed `Str`, and the
/// bridge must borrow `&str` from the right one (`String` has no `.str()`).
fn bridge_write_ret(t: &Type, expr: &str, inner_owned: bool) -> String {
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
        // `String` (about to drop).  @PLN10 — destination-passing, not `scratch`:
        // the interpreter caller routes this call through `gen_cdylib_text_dest_call`
        // (`is_cdylib_text_call` ⇒ true for an auto-native text fn), which stashed a
        // per-call `bridge_text_dest` on the shared store.  Write the bytes into that
        // caller-owned record (line-lifetime, distinct per call — no re-entrancy,
        // no never-cleared global buffer) and signal dest-mode by leaving `ret` text
        // null so the dispatcher pushes nothing (mirrors the cdylib `bridge_text_result`).
        Type::Text(_) => {
            // Borrow `&str` from the inner result: an owned `String` (nwb /
            // FFI-direct) vs a buffer-backed `Str` (which has `.str()`).
            let borrow = if inner_owned {
                "let __t: &str = __r.as_str();"
            } else {
                "let __t = __r.str();"
            };
            format!(
                "let __r = ({expr});\n    \
                 {borrow}\n    \
                 let __st: &mut Stores = unsafe {{ &mut *cell.get() }};\n    \
                 if let Some(__d) = __st.bridge_text_dest.take() {{\n    \
                     if !__t.is_empty() {{\n    \
                         __st.store_mut(&__d).addr_mut::<String>(__d.rec, __d.pos).push_str(__t);\n    \
                     }}\n    \
                 }}\n    \
                 unsafe {{ (*ret).text_ptr = std::ptr::null(); (*ret).text_len = 0; }}"
            )
        }
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

/// Candidate `(rlib-lookup-dir, deps-dir)` pairs for a loft binary at `exe_dir`,
/// most authoritative first.  The dev/test layout looks beside the exe (`deps/`,
/// then the uplifted `<profile>/`); the INSTALLED layout (`<prefix>/bin/loft`)
/// keeps `libloft.rlib` + its deps under `<prefix>/share/loft/` — where `make
/// install` puts them, NOT next to the binary.  Without the share candidates the
/// installed loft can fingerprint its rlib (`cache::loft_rlib_path` has the same
/// fallback) but cannot LOCATE it to link a library's cdylib → "libloft.rlib not
/// found for this build".  Keep aligned with `cache::loft_rlib_path` (#304).
fn rlib_search_dirs(exe_dir: &std::path::Path) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    // Dependency rlibs live in `<profile>/deps/`.  Real binary run: `exe_dir` is
    // `<profile>`, so `deps/` is its child.  Integration test: `exe_dir` already IS
    // `.../deps`.  Either way the returned link-search dir is that `deps/`.
    let dev_deps = if exe_dir.file_name().is_some_and(|n| n == "deps") {
        exe_dir.to_path_buf()
    } else {
        exe_dir.join("deps")
    };
    let mut out = vec![
        (dev_deps.clone(), dev_deps.clone()),
        (exe_dir.to_path_buf(), dev_deps.clone()),
    ];
    if exe_dir.file_name().is_some_and(|n| n == "bin")
        && let Some(prefix) = exe_dir.parent()
    {
        let share = prefix.join("share").join("loft");
        out.push((share.join("deps"), share.join("deps")));
        out.push((share.clone(), share.join("deps")));
    }
    out
}

/// @PLN11 Arc N / N3 — locate the running build's `libloft.rlib` + its sibling
/// `deps/` directory, for linking an auto-generated cdylib against the **same**
/// libloft this process links (so `Stores`/`DbRef`/`LibArg` are ABI-identical).
///
/// Works in both contexts the cdylib must build in: a real `cargo run --bin loft`
/// (an unhashed `target/<prof>/libloft.rlib`, or `deps/`) and an integration test
/// (a hashed `libloft-<hash>.rlib` in the test binary's `deps/`).  Returns the
/// chosen rlib path and the `deps/` dir to add to the link search path.
///
/// The deps-first preference must stay aligned with `cache::rlib_candidates`
/// (#304): the fingerprint that validates a built cdylib has to hash the same
/// rlib this function links, or a cdylib gets validated against one loft and
/// built against another.
#[must_use]
pub fn find_loft_rlib() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    // Find `libloft.rlib` (unhashed) or the newest hashed `libloft-<hash>.rlib`,
    // returning the matching `deps/` dir as the link-search path.
    for (dir, deps_dir) in &rlib_search_dirs(&exe_dir) {
        if !dir.is_dir() {
            continue;
        }
        let exact = dir.join("libloft.rlib");
        if exact.exists() {
            return Some((exact, deps_dir.clone()));
        }
        let hashed = std::fs::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("libloft-") && has_rlib_ext(&n)
            })
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
            .map(|e| e.path());
        if let Some(rlib) = hashed {
            return Some((rlib, deps_dir.clone()));
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

/// Pick the `loft_ffi` rlib in `deps` that `libloft` was built against.
///
/// loft's `deps/` can hold several `libloft_ffi-<hash>.rlib` with the same
/// StableCrateId but different SVH — e.g. one from `cargo build` (the loft binary)
/// and one from `cargo test` (the suite).  `libloft` links exactly ONE of them; if
/// the cdylib's `--extern loft_ffi=` names a DIFFERENT copy, the link carries two
/// `loft_ffi` and rustc aborts with "found crates (`loft_ffi` and `loft_ffi`) with
/// colliding StableCrateId values" — the zero-trust native blocker, and the same
/// failure behind p171/p310/imaging.
///
/// rmeta records the dependency by SVH, not by the filename hash, so we can't
/// string-match it.  Instead pick the candidate whose mtime is closest to
/// `libloft`'s: a crate and the dependency it links are produced by one cargo
/// invocation and share a build time, while a stray copy from another build sits far
/// off.  (A heuristic — a precise SVH match is the follow-up hardening.)
pub fn loft_ffi_for_libloft(
    libloft: &std::path::Path,
    deps: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let anchor = libloft.metadata().and_then(|m| m.modified()).ok()?;
    let mut best: Option<(std::path::PathBuf, std::time::Duration)> = None;
    for e in std::fs::read_dir(deps).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !n.starts_with("libloft_ffi-") || !has_rlib_ext(&n) {
            continue;
        }
        let Ok(mtime) = e.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        // Symmetric gap so clock direction does not matter.
        let gap = mtime
            .duration_since(anchor)
            .or_else(|_| anchor.duration_since(mtime))
            .unwrap_or_default();
        if best.as_ref().is_none_or(|(_, b)| gap < *b) {
            best = Some((e.path(), gap));
        }
    }
    best.map(|(p, _)| p)
}

/// On Windows MSVC, the build-script output dirs holding native import libraries
/// (e.g. `windows.0.48.5.lib` from `windows-sys`) must be passed to a hand-driven
/// `rustc` as `-L` paths — cargo adds them via `cargo:rustc-link-search` but we
/// don't, so the cdylib link fails `LNK1181: cannot open input file …`.  Mirrors
/// the `--native` test runner's `find_native_lib_dirs`.  Empty (a no-op) off Windows.
#[cfg(not(windows))]
fn native_lib_search_dirs(_rlib: &std::path::Path) -> Vec<std::path::PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn native_lib_search_dirs(rlib: &std::path::Path) -> Vec<std::path::PathBuf> {
    // `rlib` is `target/<profile>/libloft.rlib` or `target/<profile>/deps/libloft-*.rlib`;
    // walk up to the profile dir, then scan `build/<crate>-<hash>/`.
    let Some(profile_dir) = rlib.parent().and_then(|p| {
        if p.file_name().is_some_and(|n| n == "deps") {
            p.parent()
        } else {
            Some(p)
        }
    }) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(profile_dir.join("build")) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let build_entry = entry.path();
        // `out/` and its immediate subdirs (some crates emit into `out/<target>/`).
        let out = build_entry.join("out");
        if out.is_dir() {
            dirs.push(out.clone());
            if let Ok(subs) = std::fs::read_dir(&out) {
                dirs.extend(
                    subs.filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_dir()),
                );
            }
        }
        // `cargo:rustc-link-search` directives cached in `build/<crate>-<hash>/output`
        // (e.g. `windows_x86_64_msvc` ships its `.lib` inside the registry package).
        if let Ok(content) = std::fs::read_to_string(build_entry.join("output")) {
            for line in content.lines() {
                if let Some(p) = line
                    .strip_prefix("cargo:rustc-link-search=native=")
                    .or_else(|| line.strip_prefix("cargo:rustc-link-search="))
                {
                    let p = std::path::PathBuf::from(p);
                    if p.is_dir() && !dirs.contains(&p) {
                        dirs.push(p);
                    }
                }
            }
        }
    }
    dirs
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

/// @PLN26 — whether a cdylib links `[native] crate` packages by C-ABI (their
/// `.so`) rather than as Rust rlibs.  The library-crate twin of
/// `native_utils::native_cabi_enabled` (the binary-crate copy the executable path
/// reads): both read the ONE env gate, so the cdylib's codegen + link agree with
/// the executable's on a given host.  `LOFT_NATIVE_CABI=0` forces the legacy rlib
/// link (the escape hatch); the C-ABI path is the default on every host.
fn native_cabi_link_enabled() -> bool {
    !matches!(std::env::var("LOFT_NATIVE_CABI").ok().as_deref(), Some("0"))
}

/// @PLN26 phase 2 — the rustc link flags for ONE native package's resolved cdylib
/// `.so`: `-L native=<dir>` + `-l dylib=<name>` + an RPATH (the build/prebuilt
/// dir and `$ORIGIN` for an installed binary shipping the `.so` beside it).  On
/// Windows a DLL links through its import library and there is no RPATH, so this
/// bridges `<stem>.dll.lib` → `<stem>.lib` and the loader finds the staged DLL
/// beside the binary instead.
///
/// Returns empty when the `.so` can't be resolved ([`crate::extensions::resolve_native_lib`]
/// already printed why) — the link then fails loudly on the undefined symbol
/// rather than silently mis-linking.  The `.so` seals the package's whole Rust
/// crate graph (its own `loft_ffi` + `loft_register_v1` included), so no
/// `-L dependency=` / per-crate pinning is needed and two packages no longer
/// collide on `loft_register_v1`.
///
/// Mirrors the C-ABI branch of `native_utils::add_native_extern_flags` (the
/// executable path); kept separate because that helper lives in the binary crate
/// and this builder lives in the library crate.
fn native_pkg_cabi_link_args(crate_name: &str, pkg_dir: &str) -> Vec<String> {
    // The cdylib is named after the `[library] native` stem, which can differ
    // from the crate name; read it from the manifest (the SAME stem the
    // interpreter resolves with), falling back to the crate name when absent.
    let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
        .and_then(|m| m.native)
        .unwrap_or_else(|| crate_name.replace('-', "_"));
    let Some(so) = crate::extensions::resolve_native_lib(pkg_dir, &stem) else {
        return Vec::new();
    };
    let so_path = std::path::PathBuf::from(&so);
    let Some(so_dir) = so_path.parent() else {
        return Vec::new();
    };
    // `-l dylib=<name>` derived from the RESOLVED file (strip `lib` prefix +
    // extension) so a prebuilt or non-`lib<stem>` cdylib links.
    let libname = so_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map_or(stem.as_str(), |s| s.strip_prefix("lib").unwrap_or(s));
    let mut args = vec![
        "-L".to_string(),
        format!("native={}", so_dir.display()),
        "-l".to_string(),
        format!("dylib={libname}"),
    ];
    if cfg!(windows) {
        // Naming bridge: a Rust cdylib's import lib is `<stem>.dll.lib`, but
        // `-l dylib=<stem>` makes MSVC link.exe open `<stem>.lib`.  Copy
        // `<stem>.dll.lib` → `<stem>.lib` beside it (identical content) so the
        // `-l dylib=` resolves.
        let dll_lib = so_dir.join(format!("{libname}.dll.lib"));
        let plain_lib = so_dir.join(format!("{libname}.lib"));
        if dll_lib.exists() && !plain_lib.exists() {
            let _ = std::fs::copy(&dll_lib, &plain_lib);
        }
        // Disallow-the-unverifiable-loudly: with NEITHER import-lib name present
        // the link dies on an opaque `LNK1181`, so name it rather than mis-link.
        if !plain_lib.exists() && !dll_lib.exists() {
            eprintln!(
                "loft: native package `{crate_name}` cdylib at {} has no import \
                 library (`{libname}.dll.lib` / `{libname}.lib`) — Windows links a \
                 DLL through its import lib (@PLN26 phase 4); rebuild the package's \
                 cdylib with a toolchain that emits one.",
                so_dir.display()
            );
        }
    } else {
        // Two RPATH entries: the build/prebuilt dir (run-from-build-tree) AND
        // `$ORIGIN` (an installed binary shipping the `.so` beside it).  `$ORIGIN`
        // is literal; the dynamic loader expands it at run time.
        args.push(format!("-Clink-arg=-Wl,-rpath,{}", so_dir.display()));
        args.push("-Clink-arg=-Wl,-rpath,$ORIGIN".to_string());
    }
    args
}

/// @PLN11 Arc N / N3 — generate **and compile** the shared-store cdylib for
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
    // Same pre-check as `auto_build_native` / the driver's native path: a
    // live rustc differing from loft's build rustc cannot link the
    // SVH-locked rlib below (E0514) — fail with the reason instead of a
    // compiler spew (callers fall back to interpreting the library).
    if let Some(reason) = crate::cache::rustc_mismatch() {
        return Err(format!("{reason} — rebuild loft to restore native"));
    }
    let (rlib, deps) = find_loft_rlib().ok_or("libloft.rlib not found for this build")?;
    let src = generate_shared_cdylib_lib_rs(data, stores, export_set);
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let rs = out_dir.join(format!("{stem}.rs"));
    std::fs::write(&rs, &src).map_err(|e| format!("write {}: {e}", rs.display()))?;
    let so = out_dir.join(platform_cdylib_name(stem));
    // Compile to a temp name, rename into place on success: a concurrent
    // reader (fast path of `cached_or_build_shared_cdylib`) then always sees
    // either the old complete `.so` or the new complete one, never a torn
    // file mid-write ("file too short" on dlopen).
    let tmp_so = out_dir.join(format!("{stem}.building"));

    // Pass rustc args via an `@argfile`: the `--extern <crate>=<path>` list + `-L`
    // search paths routinely exceed Windows' ~32 KB `CreateProcessW` command-line
    // limit (os error 206: "The filename or extension is too long"), and the
    // argfile form is cross-platform — the same fix the `--native` test runner
    // already uses (PLAN49).
    let mut args: Vec<String> = vec![
        "--edition=2024".to_string(),
        "-C".to_string(),
        "debuginfo=0".to_string(),
        "-C".to_string(),
        "opt-level=2".to_string(),
        "--crate-type".to_string(),
        "cdylib".to_string(),
        "-o".to_string(),
        tmp_so.display().to_string(),
        rs.display().to_string(),
        "--extern".to_string(),
        format!("loft={}", rlib.display()),
        "-L".to_string(),
        deps.display().to_string(),
    ];
    for (name, path) in extra_externs(&deps) {
        args.push("--extern".to_string());
        args.push(format!("{name}={}", path.display()));
    }
    // @PLN26 phase 2 — a shared-store library cdylib that USES a `[native] crate`
    // package links that package by C-ABI, exactly as the executable native path
    // does: its `.so` (sealing the package's whole Rust crate graph — its own
    // `loft_ffi` + `loft_register_v1` included) via `-L native`/`-l dylib`/RPATH,
    // and its fns as `extern "C"` decls naming `loft_ffi::Loft*` (codegen gated on
    // the SAME `native_cabi_link_enabled()` consulted by `emit_program`, so codegen
    // and link can't disagree).  The sealed `.so` is why two native packages no
    // longer collide on `loft_register_v1` — the old 2-package limit is lifted.
    if !data.native_packages.is_empty() {
        // The legacy rlib link (LOFT_NATIVE_CABI=0) CAN'T link a native package
        // into a cdylib: pulling the package rlib in brings a SECOND `loft_ffi`
        // rlib (duplicate `loft_register_v1`, two `StableCrateId`s) — unlinkable.
        // Refuse THAT combo loudly and fall back to interpret (codegen emitted
        // `extern crate <pkg>` for it, which has no matching `--extern` anyway).
        if !native_cabi_link_enabled() {
            let names: Vec<&str> = data
                .native_packages
                .iter()
                .map(|(c, _)| c.as_str())
                .collect();
            eprintln!(
                "loft: a shared-store cdylib using `[native] crate` package(s) {names:?} \
                 needs the C-ABI link path, but LOFT_NATIVE_CABI=0 forces the legacy rlib \
                 link (which cannot take two `loft_ffi` rlibs into one cdylib); interpreting \
                 it instead.  Unset LOFT_NATIVE_CABI to use the C-ABI path."
            );
            return Err(
                "shared-store cdylib + native package needs the C-ABI path (LOFT_NATIVE_CABI=0 set)"
                    .to_string(),
            );
        }
        // The C-ABI decls name `loft_ffi::LoftStore`/`LoftRef`/`LoftStr`; loft's
        // own `loft_ffi` rlib must be on the command (the package `.so` is C-ABI,
        // so this is the only `loft_ffi` the cdylib's Rust code names).  Mirrors
        // the standalone native compile in main.rs.
        // Pick the `loft_ffi` `libloft` was built against, NOT the first one in dir
        // order: with two copies present, naming the wrong one puts a second
        // `loft_ffi` in the link → "colliding StableCrateId" (see `loft_ffi_for_libloft`).
        if let Some(ffi) = loft_ffi_for_libloft(&rlib, &deps) {
            args.push("--extern".to_string());
            args.push(format!("loft_ffi={}", ffi.display()));
        }
        // Link each native package's resolved cdylib `.so` (the same `-L native`
        // / `-l dylib` / RPATH the executable path emits) so the cdylib's
        // `extern "C"` calls resolve at link time and the `.so` loads at run time.
        for (crate_name, pkg_dir) in &data.native_packages {
            args.extend(native_pkg_cabi_link_args(crate_name, pkg_dir));
        }
    }
    // Windows MSVC: add the build-script `-L` dirs holding native import libs
    // (`windows.0.48.5.lib` etc.) or the link fails LNK1181.  No-op off Windows.
    for dir in native_lib_search_dirs(&rlib) {
        args.push("-L".to_string());
        args.push(dir.display().to_string());
    }
    // One arg per line; quote any containing whitespace (rustc's argfile parser is
    // whitespace-separated, newline-separated is a strict subset).
    let argfile = out_dir.join(format!("{stem}.args"));
    let contents = args
        .iter()
        .map(|s| {
            if s.contains(char::is_whitespace) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&argfile, contents).map_err(|e| format!("write {}: {e}", argfile.display()))?;
    let output = std::process::Command::new("rustc")
        .arg(format!("@{}", argfile.display()))
        .output()
        .map_err(|e| format!("launch rustc: {e} (is the Rust toolchain installed?)"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let hint = toolchain_failure_hint(&stderr)
            .map(|h| format!("{h}\n\n"))
            .unwrap_or_default();
        let tail: Vec<&str> = stderr.lines().rev().take(30).collect();
        let tail: String = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(format!(
            "{hint}cdylib compile failed (source kept at {}):\n{tail}",
            rs.display()
        ));
    }
    std::fs::rename(&tmp_so, &so)
        .map_err(|e| format!("install {} -> {}: {e}", tmp_so.display(), so.display()))?;
    Ok(so)
}

/// Detect ENVIRONMENT failures that masquerade as compiler/linker errors.
///
/// A `rustc`/`cc`/`ld` invocation that dies from a full temp dir or OOM emits a
/// cryptic crash, not a clear diagnosis: a SIGBUS from `ld` writing to a full
/// tmpfs (the common case — the linker mmaps object files into `TMPDIR`, and a
/// write the tmpfs can no longer back faults with a Bus error) reads like a
/// linker bug or a stale-artifact problem, when it is really "disk is full".
/// Scan a failed invocation's captured stderr for those signatures and return a
/// one-line, actionable hint to surface ABOVE the raw output; return `None` for
/// a genuine compile error so the real diagnostics show through untouched.
#[must_use]
/// Is a working `rustc` on `PATH`?  Cached for the process.
///
/// This is the line between a legitimate interpret-fallback and a real failure: with
/// NO toolchain, loft cannot build native at all, so interpreting a library is the
/// only option and is graceful.  With a toolchain PRESENT, a native build that fails
/// is a genuine error — silently interpreting it would hand back a partly-interpreted
/// binary (or one whose `#native` functions panic at runtime) while the user asked for
/// native.  Callers gate the fallback on this.
#[must_use]
pub fn rustc_available() -> bool {
    use std::sync::OnceLock;
    static AVAIL: OnceLock<bool> = OnceLock::new();
    *AVAIL.get_or_init(|| {
        std::process::Command::new("rustc")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

pub fn toolchain_failure_hint(stderr: &str) -> Option<String> {
    let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let env_fault = |what: &str| {
        Some(format!(
            "NOTE: the toolchain {what} — this is an ENVIRONMENT failure, not a \
             code error. The build temp dir is almost certainly full (or out of \
             memory). Free space on TMPDIR ({tmp}), or set TMPDIR to a larger \
             filesystem, then retry."
        ))
    };
    if stderr.contains("No space left on device") || stderr.contains("ENOSPC") {
        return env_fault("ran out of disk space");
    }
    if stderr.contains("Bus error")
        || stderr.contains("signal: 7")
        || stderr.contains("signal 7")
        || stderr.contains("SIGBUS")
    {
        return env_fault("crashed with a Bus error (SIGBUS), typically a full temp dir");
    }
    if stderr.contains("Cannot allocate memory")
        || stderr.contains("signal: 9")
        || stderr.contains("SIGKILL")
    {
        return env_fault("was killed (out of memory)");
    }
    // The known `loft_ffi` duplicate-crate collision: two copies of loft-ffi reach the
    // same link carrying the same StableCrateId, which rustc refuses.  Name it
    // explicitly so a consumer does not read the raw rustc dump as a bug in their own
    // code or in the library — it is neither, and loft falls back to interpreting.
    if stderr.contains("StableCrateId") {
        return Some(
            "NOTE: this is the known loft_ffi duplicate-crate collision — two copies of \
             loft-ffi reach the same link with the same StableCrateId, which rustc \
             refuses.  It is a BUILD/TOOLCHAIN limitation, NOT a bug in your code or in \
             this library.  To clear it, rebuild the library's cdylib against the CURRENT \
             loft — `make rebuild-native-cdylibs` in the loft tree, or `cargo build \
             --release` in the library's native/ dir — then re-run.  If it persists after \
             a clean rebuild, it is the tracked StableCrateId limitation, not your build."
                .to_string(),
        );
    }
    None
}

/// Derive the auto-native cdylib crate stem from a package directory.
///
/// The stem becomes the cdylib's crate name (rustc derives the crate name from the
/// source-file stem when no `--crate-name` is given), so every character must be
/// valid in a Rust identifier.  A registry dir carries a dotted version
/// (`glb-0.1.0`): rustc maps `-`→`_` itself but rejects the surviving `.`, so map
/// **every** non-`[A-Za-z0-9_]` char to `_` — enforcing the full invalid-character
/// class, not just `-`/`.`.  The constant `loft_auto_` prefix guarantees the leading
/// character is alphabetic (a crate name may not start with a digit).  (#294)
#[must_use]
pub fn auto_cdylib_stem(pkg_dir: &str) -> String {
    let raw = std::path::Path::new(pkg_dir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("lib");
    format!(
        "loft_auto_{}",
        raw.replace(|c: char| !(c.is_ascii_alphanumeric() || c == '_'), "_")
    )
}

/// @PLN11 Arc N / N3 (Step 4 — **dev-interpret-on-edit**) — decide how the library
/// at `pkg_dir` should run this invocation, and build its cdylib only when warranted.
///
/// Returns:
/// - `Ok(Some(so))` — dispatch native against the cdylib at `so` (fresh cached, or
///   just built);
/// - `Ok(None)` — **interpret this run** (the library is being actively edited);
/// - `Err(_)` — a build was attempted and failed (the caller interprets + warns).
///
/// The policy reconciles "a library is always native" with "no `rustc` per save":
/// 1. **Fresh artifact** (`.so` exists, fingerprint matches, source not newer) →
///    native, no work.
/// 2. **No artifact yet, or `loft` itself changed** (fingerprint mismatch) → **build
///    eagerly** → native.  First use of a library, and a deployed/stable dep, pay the
///    one-time build and run native from the start (this is also why the parity tests,
///    which wipe `native-auto/` then run once, still dispatch native).
/// 3. **Stale artifact** (an `.so` exists but the source is newer — the library was
///    edited) → consult the last-run source hash:
///    - *changed since the previous run* → the edit loop is live → **interpret this
///      run** (`Ok(None)`); record the new hash.  No `rustc`.
///    - *unchanged since the previous run* → editing has settled → **rebuild** → native.
///
/// So an edit-run-edit-run loop interprets each time (no `rustc`); the first run after
/// you stop editing rebuilds and the library is native again.  (Polish: do that
/// rebuild in the background so even the settling run never blocks — see the plan's
/// Step 4 option 3.)
///
/// # Errors
/// Propagates a build failure from [`build_shared_cdylib`].
pub fn cached_or_build_shared_cdylib(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    pkg_dir: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    let stem = auto_cdylib_stem(pkg_dir);
    let out_dir = std::path::Path::new(pkg_dir).join("native-auto");
    let so = out_dir.join(platform_cdylib_name(&stem));
    let fp = crate::cache::loft_build_fingerprint();

    // 1. Fresh artifact → native, no hashing.
    if so.exists()
        && crate::cache::native_artifact_fingerprint_matches(&out_dir, fp)
        && !source_newer_than(pkg_dir, &so)
    {
        return Ok(Some(so));
    }

    // Concurrent `loft` processes (parallel tests are the common case)
    // routinely load the same library at once; an unserialized double-build
    // rewrites the generated `.rs` under a running rustc and tears the `.so`
    // (mixed-object "dangerous relocation" link errors, "file too short" on
    // load).  Take an exclusive advisory lock for the check+build, then
    // RE-CHECK freshness: the waiter adopts the winner's artifact instead of
    // rebuilding over it.  Released when `lock` drops, on every return path.
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let lock_path = out_dir.join(".build.lock");
    let lock = std::fs::File::create(&lock_path)
        .map_err(|e| format!("create {}: {e}", lock_path.display()))?;
    lock.lock()
        .map_err(|e| format!("lock {}: {e}", lock_path.display()))?;
    if so.exists()
        && crate::cache::native_artifact_fingerprint_matches(&out_dir, fp)
        && !source_newer_than(pkg_dir, &so)
    {
        return Ok(Some(so));
    }

    let build = |out_dir: &std::path::Path| -> Result<Option<std::path::PathBuf>, String> {
        let built = build_shared_cdylib(data, stores, export_set, out_dir, &stem)?;
        crate::cache::write_native_artifact_fingerprint(out_dir, fp);
        Ok(Some(built))
    };

    // 2. No artifact yet, or `loft` changed → build eagerly (native from the start).
    if !so.exists() || !crate::cache::native_artifact_fingerprint_matches(&out_dir, fp) {
        crate::cache::write_run_source_hash(&out_dir, source_content_hash(pkg_dir));
        return build(&out_dir);
    }

    // 3. Stale artifact (the library was edited) → dev-interpret-on-edit.
    let cur = source_content_hash(pkg_dir);
    let stable = crate::cache::read_run_source_hash(&out_dir) == Some(cur);
    crate::cache::write_run_source_hash(&out_dir, cur);
    if stable {
        build(&out_dir) // editing settled → rebuild, native again
    } else {
        Ok(None) // still being edited → interpret this run, no rustc
    }
}

/// @PLN11 Arc N / N3 (Step 4) — a content hash of every `.loft` / `loft.toml` source
/// under `pkg_dir` (build/artifact dirs skipped, files visited in sorted order for a
/// stable digest).  Used to detect "did this library's source change since the last
/// run?" — content-based, not mtime, so it is deterministic (testable) and a no-op
/// touch doesn't trigger a rebuild.
fn source_content_hash(pkg_dir: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(pkg_dir)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            if name == "native-auto" || name == "native" || name == "target" {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            let is_src = p
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("loft"))
                || name == "loft.toml";
            if is_src {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut h = Sha256::new();
    for f in &files {
        // The path relative to the package keeps the digest stable across machines
        // while still reacting to renames; content reacts to edits.
        let rel = f.strip_prefix(pkg_dir).unwrap_or(f);
        h.update(rel.to_string_lossy().as_bytes());
        if let Ok(bytes) = std::fs::read(f) {
            h.update(&bytes);
        }
    }
    let digest: [u8; 32] = h.finalize().into();
    u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]))
}

/// Is any `.loft` / `loft.toml` source under `pkg_dir` newer than the artifact at
/// `artifact`?  (Build/artifact dirs are skipped.)  A missing/unreadable artifact
/// mtime counts as stale.
fn source_newer_than(pkg_dir: &str, artifact: &std::path::Path) -> bool {
    let Ok(art_mtime) = artifact.metadata().and_then(|m| m.modified()) else {
        return true;
    };
    let mut newest: Option<std::time::SystemTime> = None;
    let mut stack = vec![std::path::PathBuf::from(pkg_dir)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            if name == "native-auto" || name == "native" || name == "target" {
                continue; // build/artifact dirs
            }
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            let is_src = p
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("loft"))
                || name == "loft.toml";
            if is_src
                && let Ok(mt) = e.metadata().and_then(|m| m.modified())
                && newest.is_none_or(|n| mt > n)
            {
                newest = Some(mt);
            }
        }
    }
    newest.is_some_and(|src| src > art_mtime)
}

#[cfg(test)]
mod toolchain_hint_tests {
    use super::toolchain_failure_hint;

    #[test]
    fn flags_sigbus_as_environment_failure() {
        // The real shape that misled diagnosis: ld dies with a Bus error when
        // TMPDIR's tmpfs is full.  Must be named an ENVIRONMENT failure.
        let stderr = "collect2: fatal error: ld terminated with signal 7 [Bus error], core dumped\n\
                      error: aborting due to 1 previous error";
        let hint = toolchain_failure_hint(stderr).expect("SIGBUS must produce a hint");
        assert!(hint.contains("ENVIRONMENT"), "hint: {hint}");
        assert!(
            hint.contains("TMPDIR"),
            "hint should point at TMPDIR: {hint}"
        );
    }

    #[test]
    fn flags_enospc() {
        assert!(toolchain_failure_hint("error: No space left on device (os error 28)").is_some());
    }

    #[test]
    fn genuine_compile_error_gets_no_hint() {
        // A real type error must pass through untouched so its diagnostics show.
        let stderr = "error[E0308]: mismatched types\n  expected `i32`, found `&str`";
        assert!(toolchain_failure_hint(stderr).is_none());
    }
}

#[cfg(test)]
mod rlib_search_tests {
    use super::rlib_search_dirs;
    use std::path::{Path, PathBuf};

    /// #398 follow-up — the INSTALLED layout (`<prefix>/bin/loft`) must search
    /// `<prefix>/share/loft/` for libloft.rlib, else a normal library's cdylib link
    /// fails "libloft.rlib not found for this build" (the bin/ dir holds no rlib).
    #[test]
    fn installed_bin_layout_searches_share_loft() {
        let dirs = rlib_search_dirs(Path::new("/usr/local/bin"));
        let share = PathBuf::from("/usr/local/share/loft");
        assert!(
            dirs.iter()
                .any(|(d, deps)| *d == share && *deps == share.join("deps")),
            "install layout must search <prefix>/share/loft with its deps/: {dirs:?}"
        );
        assert!(
            dirs.iter().any(|(d, _)| *d == share.join("deps")),
            "install layout must also search <prefix>/share/loft/deps: {dirs:?}"
        );
    }

    /// The dev/test layout (exe in `<profile>/`) keeps the existing behaviour:
    /// `deps/` first, then the uplifted `<profile>/`, and NO share/ candidate.
    #[test]
    fn dev_layout_unchanged_no_share() {
        let dirs = rlib_search_dirs(Path::new("target/release"));
        assert_eq!(dirs[0].0, PathBuf::from("target/release/deps"));
        assert_eq!(dirs[1].0, PathBuf::from("target/release"));
        assert!(
            !dirs
                .iter()
                .any(|(d, _)| d.to_string_lossy().contains("share/loft")),
            "a dev exe must not gain a share/ candidate: {dirs:?}"
        );
        // every dev candidate links the same deps/ search dir
        assert!(
            dirs.iter()
                .all(|(_, deps)| deps.as_path() == std::path::Path::new("target/release/deps"))
        );
    }

    /// Integration-test layout: exe already in `.../deps` → deps dir is itself.
    #[test]
    fn deps_dir_exe_uses_itself() {
        let dirs = rlib_search_dirs(Path::new("target/release/deps"));
        assert_eq!(dirs[0].0, PathBuf::from("target/release/deps"));
        assert_eq!(dirs[0].1, PathBuf::from("target/release/deps"));
    }
}
