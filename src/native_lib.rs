// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLAN54 Arc N / N2 — auto-generate a native cdylib from a library's
//! scalar-dispatchable functions (the first dispatch slice).
//!
//! In the native-library execution model (C71) a stable library compiles to a
//! cdylib and the interpreter dispatches calls to it (the stdlib `#native` path,
//! `OpStaticCall` → dlsym).  This module generates that cdylib's `lib.rs` for the
//! **scalar-dispatchable** subset (`native_gate::scalar_dispatchable`): functions
//! whose params + return are scalars, so no store reference crosses the boundary
//! and the export wrapper is trivial (no `&UnsafeCell<Stores>` ↔ `LoftStore`
//! bridge — that comes in the store-touching slice).
//!
//! The source is the `--native` program for the export set + its transitive
//! dependencies (`Output::output_native_reachable` — header + `init` + only the
//! reachable functions) plus, per exported function `n_<name>`, an export wrapper:
//!
//! ```ignore
//! #[unsafe(no_mangle)]
//! pub extern "C" fn loft_n_<name>(a0: i64, …) -> <ret> {
//!     let cell = std::cell::UnsafeCell::new(Stores::new());
//!     init(&cell);
//!     n_<name>(&cell, a0, …)
//! }
//! ```
//!
//! The per-call cell is safe: a scalar-return function cannot leak a store
//! reference out, so any internal store use is contained and dropped with the
//! cell.  (`init` populates the type schema + const vectors the body may read.)

use crate::data::{Context, Data};
use crate::database::Stores;
use crate::generation::{Output, rust_type};
use std::collections::HashSet;

/// Generate the cdylib `lib.rs` source for `data`: the `--native` program for the
/// functions in `export_set` **and their transitive dependencies**, plus a
/// `#[no_mangle] pub extern "C"` export wrapper for each `export_set` function
/// (the symbols the interpreter will dispatch to).
///
/// `export_set` is the **library's own public, scalar-dispatchable functions** —
/// the cdylib's exported API, not the whole stdlib.  It serves as both the
/// reachability roots (so only those functions + their deps are emitted) and the
/// set to wrap.  The caller computes it as `library_pub_fns ∩ scalar_dispatchable`
/// (operator definitions and stdlib internals are deps, inlined/emitted as needed,
/// never wrapped).
#[must_use]
pub fn generate_cdylib_lib_rs(data: &Data, stores: &Stores, export_set: &HashSet<u32>) -> String {
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
        // Emit only the exported functions + their transitive deps (header +
        // init + the reachable subset) — exactly what a `--native` binary emits
        // from `n_main`, so unreachable operator stubs never surface.  The
        // entry set is the functions we export.
        let entry: Vec<u32> = export_set.iter().copied().collect();
        let _ = out.output_native_reachable(&mut buf, 0, data.definitions(), &entry);
    }
    let mut src = String::from_utf8(buf).unwrap_or_default();
    for &d in export_set {
        src.push('\n');
        src.push_str(&export_wrapper(data, d));
    }
    src
}

/// The `#[no_mangle] pub extern "C"` export wrapper for scalar-dispatchable
/// function `d_nr` (whose `--native` inner function is its name, e.g. `n_double`).
/// The export symbol is `loft_<name>` (distinct from the inner to avoid a clash).
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
