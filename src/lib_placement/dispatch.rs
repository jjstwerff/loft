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
//! # What this routes
//!
//! Parameters and returns may be integer-family, boolean, `single` or text, plus
//! a void return. A function outside that is simply **not marked**, so it runs
//! in-process — byte-identically, which is the same fallback a library that
//! cannot compile native already takes. Nothing becomes a call that fails later.
//! Structs, vectors and references need the arena and are still outside.
//!
//! # A text return rides the protocol it already has
//!
//! A text return does not come back on the stack. It travels the interpreter's
//! destination-buffer protocol: codegen emits `n_set_bridge_dest` immediately
//! before the call, stashing the caller's work-buffer record, and the callee
//! writes into that record and pushes nothing.
//!
//! Nothing had to be added for a placed call to use it. That routing keys on
//! [`crate::state::codegen::is_cdylib_text_call`] — a non-empty `def.native`
//! symbol plus a text return — which is exactly the shape [`mark_exports`]
//! leaves behind, so the two calls emit already. What remains is this side of
//! the contract: take the stashed destination, write the worker's answer into
//! it, and push nothing. Approximating it, which is why arc A refused text
//! returns rather than guessing, would have returned a wrong value.

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
    Single,
    Text,
    /// The hidden `&text` work buffer a text-returning loft function carries.
    ///
    /// Not a value on the wire — the worker never sees it. It is a slot the
    /// CALLER allocated and passed for the answer to be written into, and it is
    /// listed among the parameters because the emitted code pushes it like any
    /// other argument: a dispatcher that skipped it would leave it on the stack
    /// and every later frame would read one cell off.
    WorkBuf,
}

/// Classify a loft type for the wire, or refuse it.
fn kind_of(ty: &Type, returning: bool) -> Option<Kind> {
    match ty {
        Type::Void | Type::Null if returning => Some(Kind::Void),
        Type::Integer(_) => Some(Kind::Int),
        Type::Boolean => Some(Kind::Bool),
        Type::Single => Some(Kind::Single),
        Type::Text(_) => Some(Kind::Text),
        _ => None,
    }
}

/// The frame shape of one call: every attribute the emitted code pushes, in
/// declaration order, hidden work buffers included.
///
/// The two lists a signature produces are different questions and must not be
/// conflated: what the WORKER is sent (user parameters) and what the emitted
/// code PUSHES (those plus the compiler's hidden ones).
fn frame_kinds(def: &crate::data::Definition) -> Option<Vec<Kind>> {
    def.attributes()
        .iter()
        .map(|a| {
            if crate::native_lib::is_text_work_buffer(&a.typedef) {
                Some(Kind::WorkBuf)
            } else if a.hidden {
                // Some other compiler-inserted parameter. Its layout is not
                // known here, so refuse the function rather than pop a frame
                // shape that is a guess.
                None
            } else {
                kind_of(&a.typedef, false)
            }
        })
        .collect()
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
        // Both questions must be answerable: what the worker is sent, and what
        // the emitted code pushes. A signature that fails either runs in-process.
        let Some(frame) = frame_kinds(def) else {
            continue;
        };
        let Some(ret) = kind_of(&def.returned, true) else {
            continue;
        };
        // A text answer needs somewhere in the CALLER to live, and the caller
        // only ever offers one of two: a destination record (where the result is
        // assigned to a text variable) or the hidden work buffer a promoted text
        // return carries. A function whose text return was never promoted — the
        // usual reason being that it returns a constant, `fn version() -> text {
        // "1.0" }` — offers neither, and a `Str` over the worker's own answer
        // would point at a String freed the moment this call returns.
        //
        // So it is not placed, and runs in-process instead. That is the same
        // fallback every other signature the wire cannot carry takes, and by the
        // invariant it is the same program. Making it uniform means promoting
        // such a return to a retbuf (@PLN104's transform, which today only
        // considers the main program's own definitions).
        if ret == Kind::Text && !frame.contains(&Kind::WorkBuf) {
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
            let (Some(params), Some(ret)) = (frame_kinds(def), kind_of(&def.returned, true)) else {
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

    // Pop in reverse (the stack is LIFO), then restore declaration order. Every
    // cell the emitted code pushed is popped here, hidden ones included — a cell
    // left behind does not fail here, it shifts every later frame by one.
    let mut args: Vec<Value> = Vec::with_capacity(placed.params.len());
    let mut work_buf: Option<DbRef> = None;
    for k in placed.params.iter().rev() {
        match k {
            // The whole cell, unnarrowed: the callee's declared width is what
            // decides the value, exactly as the native bridge treats it.
            Kind::Int => args.push(Value::Int(*stores.get::<i64>(stack))),
            Kind::Bool => args.push(Value::Bool(*stores.get::<bool>(stack))),
            // `single` is a 4-byte cell, so it pops as `f32` — reading it as an
            // `f64` would take the next argument's bytes with it.
            Kind::Single => args.push(Value::Float(f64::from(*stores.get::<f32>(stack)))),
            Kind::Text => {
                args.push(Value::Text(
                    stores.get::<crate::keys::Str>(stack).str().to_string(),
                ));
            }
            Kind::Void => args.push(Value::Void),
            // Popped, never sent: it is the caller's answer slot, not an input.
            Kind::WorkBuf => work_buf = Some(*stores.get::<DbRef>(stack)),
        }
    }
    args.reverse();

    let ret = placed.ret;
    // Claimed before the crossing, so it is claimed on EVERY exit below —
    // including a fault, where a destination left set would redirect the next
    // text call in the program into this call's buffer.
    let text_dest = if ret == Kind::Text {
        stores.bridge_text_dest.take()
    } else {
        None
    };
    // Checked before the crossing, while the signature is still in hand: a text
    // answer with nowhere to go is a routing bug, and refusing it here is what
    // arc A chose over approximating the protocol and returning a wrong value.
    assert!(
        ret != Kind::Text || text_dest.is_some() || work_buf.is_some(),
        "placed text call '{}' has nowhere to put its answer — neither a stashed \
         destination nor a work buffer reached the dispatcher",
        placed.func
    );
    let out = reg.workers[placed.worker].call(&placed.func, &args);
    // The lock is held across the crossing on purpose (see `PLACEMENT`), but a
    // fault must not poison it while a guard is live.
    drop(guard);

    match out {
        Ok(v) => match (ret, v) {
            (Kind::Void, _) => {}
            (Kind::Int, Value::Int(i)) => stores.put::<i64>(stack, i),
            (Kind::Bool, Value::Bool(b)) => stores.put(stack, b),
            (Kind::Single, Value::Float(f)) => stores.put::<f32>(stack, f as f32),
            // A text return has two call shapes and the call site chose which
            // one, so this reads the choice rather than assuming it — the same
            // two branches the cdylib bridge (`bridge_text_result`) has.
            //
            //   * Destination-passing, emitted where the result is assigned to a
            //     text variable: the answer goes straight into that variable's
            //     record and NOTHING is pushed.
            //   * Otherwise the ordinary loft convention, which every other
            //     position uses: the answer goes into the hidden work buffer the
            //     caller passed, and a `Str` over it is pushed as the result.
            (Kind::Text, Value::Text(s)) => {
                let into = text_dest.or(work_buf).expect("checked before the crossing");
                let buf = stores
                    .store_mut(&into)
                    .addr_mut::<String>(into.rec, into.pos);
                // Append: the call site cleared the buffer beforehand, exactly
                // as it does for an in-process text return.
                buf.push_str(&s);
                if text_dest.is_none() {
                    // `Str` borrows the buffer's bytes; the buffer is the
                    // caller's own variable and outlives this result.
                    let result = crate::keys::Str::new(buf.as_str());
                    stores.put(stack, result);
                }
            }
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
