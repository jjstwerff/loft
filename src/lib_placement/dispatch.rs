// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN119 arc A — routing a placed library's calls from the interpreter to its worker.

//! What makes placement *policy*: a consumer writes `use maths;` and calls
//! `add(2, 3)`, and whether that runs here or in a worker process is decided by
//! the library's manifest, not by the calling source.
//!
//! The route reuses the mechanism a native library already travels. Marking a
//! function with a `def.native` symbol makes `byte_code` emit an `OpStaticCall`
//! and registers a stub; [`install`] then replaces that stub with
//! [`placed_dispatch`], which reads the arguments off the interpreter stack,
//! hands them to the worker over the wire, and writes the answer back. The
//! interpreter never learns that anything unusual happened, which is the point.
//!
//! # What arc A routes
//!
//! Parameters may be integer-family, boolean, or text; returns may be void,
//! integer-family, or boolean. A function outside that is simply **not marked**,
//! so it runs in-process — byte-identically, which is the same fallback a
//! library that cannot compile native already takes. Nothing becomes a call that
//! fails later.
//!
//! `single` and text RETURNS are deliberately excluded rather than approximated.
//! A text return travels the interpreter's dest-buffer protocol
//! (`n_set_bridge_dest`), which codegen emits only for a cdylib call, and
//! guessing at it would produce a wrong value rather than a refusal. Both, plus
//! structs, vectors and references, are the boundary marshal of arc B.

use super::wire::Worker;
use crate::data::{Data, DefType, Type};
use crate::database::Stores;
use crate::host::Value;
use crate::keys::DbRef;
use crate::state::State;
use std::collections::HashMap;
use std::sync::Mutex;

/// The wire shape of one parameter or return.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Void,
    Int,
    Bool,
    Text,
}

/// Classify a loft type for the wire, or refuse it.
fn kind_of(ty: &Type, returning: bool) -> Option<Kind> {
    match ty {
        Type::Void | Type::Null if returning => Some(Kind::Void),
        Type::Integer(_) => Some(Kind::Int),
        Type::Boolean => Some(Kind::Bool),
        // A text return needs the dest-buffer protocol; see the module header.
        Type::Text(_) if !returning => Some(Kind::Text),
        _ => None,
    }
}

/// One placed function: which worker serves it, what it is called there, and
/// the shape of its frame.
struct Placed {
    worker: usize,
    func: String,
    params: Vec<Kind>,
    ret: Kind,
}

/// Every worker this process started, and the call table the dispatcher reads.
///
/// One lock covers both because a call must hold the worker for its whole
/// round trip: the wire has a single request slot, so two threads calling the
/// same placed library concurrently would interleave two frames in one buffer.
/// Serialising them is therefore correctness, not caution — and it is the
/// reason a placed library is not yet a good fit for a hot `par` arm.
static PLACEMENT: Mutex<Option<Registry>> = Mutex::new(None);

struct Registry {
    workers: Vec<Worker>,
    calls: HashMap<u16, Placed>,
}

/// The functions of the library at `pkg_dir` that arc A can route, marked with
/// their dispatch symbol.
///
/// Mirrors `native_lib::library_export_set` — a top-level, user-named, `pub`
/// function whose source file sits under the package — and applies the wire's
/// own type gate instead of the shared-store one. **Call before `byte_code`.**
///
/// Returns the (definition, symbol, name) of each marked function.
pub fn mark_exports(data: &mut Data, pkg_dir: &str) -> Vec<(u32, String, String)> {
    let dups = crate::generation::duplicate_fn_names(data);
    let mut marked = Vec::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if !matches!(def.def_type(), DefType::Function)
            || !def.pub_visible
            || !def.position().file.starts_with(pkg_dir)
            || !def.native().is_empty()
        {
            continue;
        }
        let name = def.original_name().clone();
        if name.starts_with('_') {
            continue;
        }
        let params: Vec<Type> = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden)
            .map(|a| a.typedef.clone())
            .collect();
        if kind_of(&def.returned, true).is_none()
            || params.iter().any(|t| kind_of(t, false).is_none())
        {
            continue;
        }
        let sym = format!(
            "loft_placed_{}",
            crate::generation::disambiguated_fn_ident(&dups, data.def(d))
        );
        data.def_mut(d).native.clone_from(&sym);
        marked.push((d, sym, name));
    }
    marked
}

/// Start a worker for each placed library and point its marked functions at it.
///
/// Call after `byte_code`, which is what registers the stubs this replaces.
///
/// # Errors
/// The first library whose worker will not start, named. A placed library that
/// cannot run is not degraded to in-process silently: its whole reason for the
/// declaration is isolation, and quietly withdrawing that would leave no trace
/// in any output.
pub fn install(
    state: &mut State,
    data: &Data,
    libs: &[(String, String)],
    stdlib_dir: &std::path::Path,
) -> Result<usize, String> {
    let mut reg = Registry {
        workers: Vec::new(),
        calls: HashMap::new(),
    };
    let dups = crate::generation::duplicate_fn_names(data);
    let mut wired = 0usize;
    for (name, pkg_dir) in libs {
        let worker = Worker::spawn(name, std::path::Path::new(pkg_dir), stdlib_dir)
            .map_err(|e| e.to_string())?;
        let w_idx = reg.workers.len();
        reg.workers.push(worker);

        for d in 0..data.definitions() {
            let def = data.def(d);
            if !matches!(def.def_type(), DefType::Function)
                || !def.position().file.starts_with(pkg_dir.as_str())
                || !def.native().starts_with("loft_placed_")
            {
                continue;
            }
            let sym = format!(
                "loft_placed_{}",
                crate::generation::disambiguated_fn_ident(&dups, def)
            );
            let Some(&lib_idx) = state.library_names.get(&sym) else {
                continue;
            };
            let params: Vec<Kind> = def
                .attributes()
                .iter()
                .filter(|a| !a.hidden)
                .filter_map(|a| kind_of(&a.typedef, false))
                .collect();
            let Some(ret) = kind_of(&def.returned, true) else {
                continue;
            };
            reg.calls.insert(
                lib_idx,
                Placed {
                    worker: w_idx,
                    func: def.original_name().clone(),
                    params,
                    ret,
                },
            );
            state.replace_static_fn(&sym, placed_dispatch);
            wired += 1;
        }
    }
    *PLACEMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reg);
    Ok(wired)
}

/// Shut every worker down. Called at the end of a run so a worker never
/// outlives the process that started it.
pub fn shutdown() {
    if let Ok(mut g) = PLACEMENT.lock() {
        g.take();
    }
}

/// The interpreter's entry into a placed call: read the frame off the stack,
/// cross, write the answer back.
fn placed_dispatch(stores: &mut Stores, stack: &mut DbRef) {
    let lib_idx = crate::extensions::current_lib_idx();
    let mut guard = PLACEMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(reg) = guard.as_mut() else {
        panic!("placed call with no placement registry — install() did not run");
    };
    let Some(placed) = reg.calls.get(&lib_idx) else {
        panic!("no placed signature for library index {lib_idx}");
    };

    // Pop in reverse (the stack is LIFO), then restore declaration order.
    let mut args: Vec<Value> = Vec::with_capacity(placed.params.len());
    for k in placed.params.iter().rev() {
        args.push(match k {
            // The whole cell, unnarrowed: the callee's declared width is what
            // decides the value, exactly as the native bridge treats it.
            Kind::Int => Value::Int(*stores.get::<i64>(stack)),
            Kind::Bool => Value::Bool(*stores.get::<bool>(stack)),
            Kind::Text => Value::Text(stores.get::<crate::keys::Str>(stack).str().to_string()),
            Kind::Void => Value::Void,
        });
    }
    args.reverse();

    let ret = placed.ret;
    let out = reg.workers[placed.worker].call(&placed.func, &args);
    // The lock is held across the crossing on purpose (see `PLACEMENT`), but a
    // fault must not poison it while a guard is live.
    drop(guard);

    match out {
        Ok(v) => match (ret, v) {
            (Kind::Void, _) => {}
            (Kind::Int, Value::Int(i)) => stores.put::<i64>(stack, i),
            (Kind::Bool, Value::Bool(b)) => stores.put(stack, b),
            (k, v) => panic!("placed call returned {v:?} where the signature declares {k:?}"),
        },
        // A fault inside a placed library surfaces as the caller's runtime
        // error, which is what makes its error behaviour match an in-process
        // call rather than a transport failure. The message is the library's
        // own; the position is the library's, not the caller's, so none is
        // claimed here rather than pointing at the wrong file.
        Err(e) => {
            stores.runtime_error = Some(Box::new(crate::runtime_error::RuntimeError::user_panic(
                e,
                String::new(),
                0,
            )));
            stores.had_fatal = true;
        }
    }
}
