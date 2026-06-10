// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Native extension loader.
//!
//! Package native crates (cdylib) export `loft_register_v1`, a C-ABI function
//! that registers all native symbols with the interpreter via a callback.
//! Only C primitives cross the boundary — no Rust types are shared.
//!
//! See `EXTERNAL_LIBS.md` for the full design.

/// Load all pending native extension libraries.
#[cfg(feature = "native-extensions")]
use std::collections::HashMap;
use std::sync::Mutex;

/// Wrapper for `*const ()` that is Send — function pointers from cdylibs are
/// valid for the process lifetime (the Library handle is leaked).
#[cfg(feature = "native-extensions")]
#[derive(Clone, Copy)]
struct FnPtr(*const ());
#[cfg(feature = "native-extensions")]
unsafe impl Send for FnPtr {}

/// Global registry of native function pointers loaded from cdylibs.
#[cfg(feature = "native-extensions")]
static NATIVE_REGISTRY: Mutex<Option<HashMap<String, FnPtr>>> = Mutex::new(None);

/// Plan-25: registry of generated marshal bridges (`loft_ffi::LoftBridgeFn`),
/// keyed by loft symbol.  Populated from a cdylib's `loft_register_bridges_v1`
/// if it exports one.  A symbol present here is dispatched through its bridge
/// (`dispatch_via_bridge`); a symbol absent here falls back to the legacy
/// raw-ptr `dispatch_call` arms.  Both coexist during the F3→F4 transition.
#[cfg(feature = "native-extensions")]
static BRIDGE_REGISTRY: Mutex<Option<HashMap<String, FnPtr>>> = Mutex::new(None);

/// Look up a generated bridge for `sym`, if the owning cdylib registered one.
#[cfg(feature = "native-extensions")]
fn get_bridge(sym: &str) -> Option<*const ()> {
    BRIDGE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(sym))
        .map(|p| p.0)
}

/// The C-ABI registration callback type.
#[cfg(feature = "native-extensions")]
type RegisterFn =
    unsafe extern "C" fn(unsafe extern "C" fn(*const u8, usize, *const (), *mut ()), *mut ());

/// The registration callback: called once per symbol by the cdylib.
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn collect(
    name_ptr: *const u8,
    name_len: usize,
    fn_ptr: *const (),
    ctx: *mut (),
) {
    let collected = unsafe { &mut *ctx.cast::<Vec<(String, *const ())>>() };
    let name = std::str::from_utf8(unsafe { std::slice::from_raw_parts(name_ptr, name_len) })
        .unwrap_or("<invalid>");
    collected.push((name.to_string(), fn_ptr));
}

#[cfg(feature = "native-extensions")]
pub fn load_all(_state: &mut crate::state::State, paths: Vec<String>) {
    for path in paths {
        load_one(&path);
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn load_all(_state: &mut crate::state::State, _paths: Vec<String>) {}

/// Loaded libraries kept alive for the process lifetime, each paired with
/// whether it exported `loft_register_v1` (the registration protocol).  Used by
/// `try_dlsym` to look up symbols from previously loaded cdylibs *and* report
/// whether the resolving library opted into registration — so the "unregistered
/// symbol" guard fires only for a library that chose the protocol, not for a
/// (legitimately) zero-registration cdylib loaded after one that did.
#[cfg(feature = "native-extensions")]
static LOADED_LIBS: Mutex<Vec<(libloading::Library, bool)>> = Mutex::new(Vec::new());

/// Load a single native extension shared library.
///
/// If the library exports `loft_register_v1`, calls it to collect all symbols.
/// Otherwise, the library is kept loaded and individual symbols will be
/// resolved on demand via `try_dlsym` during `wire_native_fns`.
#[cfg(feature = "native-extensions")]
fn load_one(path: &str) {
    use libloading::Library;
    use std::collections::HashSet;

    static LOAD_LOCK: Mutex<Option<HashSet<String>>> = Mutex::new(None);

    let canonical = std::fs::canonicalize(path)
        .unwrap_or_else(|_| std::path::PathBuf::from(path))
        .to_string_lossy()
        .into_owned();

    let mut guard = LOAD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let loaded = guard.get_or_insert_with(HashSet::new);
    if loaded.contains(&canonical) {
        return;
    }

    let lib = match unsafe { Library::new(path) } {
        Ok(l) => l,
        Err(e) => {
            eprintln!("loft: cannot load native extension '{path}': {e}");
            return;
        }
    };

    // Try the registration protocol first.  Records whether this library opted
    // into it — `try_dlsym` reports the flag so the unregistered-symbol guard
    // only fires for a library that chose the protocol (issue #119), not for a
    // zero-registration cdylib that legitimately relies on dlsym.
    let mut uses_v1 = false;
    if let Ok(register_sym) = unsafe { lib.get::<RegisterFn>(b"loft_register_v1\0") } {
        uses_v1 = true;
        let register_sym = *register_sym;
        let mut collected: Vec<(String, *const ())> = Vec::new();
        unsafe {
            register_sym(collect, std::ptr::addr_of_mut!(collected).cast::<()>());
        }
        let mut reg_guard = NATIVE_REGISTRY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let registry = reg_guard.get_or_insert_with(HashMap::new);
        for (name, ptr) in collected {
            registry.insert(name, FnPtr(ptr));
        }
    }
    // Plan-25: collect generated marshal bridges, if the cdylib exports them.
    // Symbols with a bridge dispatch through `dispatch_via_bridge`; the rest
    // keep using the legacy raw-ptr arms (a library may export both during the
    // transition, or only `loft_register_v1`).
    if let Ok(reg_sym) = unsafe { lib.get::<RegisterFn>(b"loft_register_bridges_v1\0") } {
        let reg_sym = *reg_sym;
        let mut collected: Vec<(String, *const ())> = Vec::new();
        unsafe {
            reg_sym(collect, std::ptr::addr_of_mut!(collected).cast::<()>());
        }
        let mut bridge_guard = BRIDGE_REGISTRY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let registry = bridge_guard.get_or_insert_with(HashMap::new);
        for (name, ptr) in collected {
            registry.insert(name, FnPtr(ptr));
        }
    }
    // Either way, keep the library loaded for potential dlsym lookups.
    loaded.insert(canonical);
    LOADED_LIBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((lib, uses_v1));
}

/// Try to resolve a symbol by name from any loaded cdylib.  Returns the symbol
/// pointer paired with whether the resolving library opted into
/// `loft_register_v1`.  Called by `wire_native_fns` as a fallback when the symbol
/// wasn't provided via the registry. This enables zero-registration cdylibs:
/// just export `#[unsafe(no_mangle)] pub extern "C" fn n_my_func(...)`.
#[cfg(feature = "native-extensions")]
fn try_dlsym(name: &str) -> Option<(*const (), bool)> {
    let libs = LOADED_LIBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut sym_name = name.to_string();
    sym_name.push('\0');
    for (lib, uses_v1) in libs.iter() {
        if let Ok(sym) = unsafe { lib.get::<*const ()>(sym_name.as_bytes()) } {
            return Some((*sym, *uses_v1));
        }
    }
    None
}

// ── Auto-marshal: wire cdylib functions via type-driven dispatch ────────

/// Argument type tag for auto-marshalling.
#[cfg(feature = "native-extensions")]
#[derive(Clone, Copy, Debug, PartialEq)]
enum ArgT {
    I32,
    I64,
    F32,
    F64,
    Bool,
    Text,
    Ref, // DbRef — struct reference (rec/pos point to the struct)
    Vec, // DbRef — vector reference (indirect: dereference rec/pos to get data record)
}

/// Mirror of `loft_ffi::LoftStr` — `#[repr(C)]` so layout matches.
#[cfg(feature = "native-extensions")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LoftStr {
    ptr: *const u8,
    len: usize,
}

/// Mirror of `loft_ffi::LoftRef` — `#[repr(C)]` so layout matches.
#[cfg(feature = "native-extensions")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LoftRef {
    store_nr: u16,
    rec: u32,
    pos: u32,
}

/// Mirror of `loft_ffi::LoftStoreCtx` — `#[repr(C)]` so layout matches.
#[cfg(feature = "native-extensions")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LoftStoreCtx {
    _opaque: *mut (),
}

/// Mirror of `loft_ffi::LoftStore` — `#[repr(C)]` so layout matches.
#[cfg(feature = "native-extensions")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LoftStore {
    ptr: *mut u8,
    size: u32,
    ctx: LoftStoreCtx,
    claim_fn: Option<unsafe extern "C" fn(LoftStoreCtx, u32) -> u32>,
    reload_fn: Option<unsafe extern "C" fn(LoftStoreCtx, *mut *mut u8, *mut u32)>,
    resize_fn: Option<unsafe extern "C" fn(LoftStoreCtx, u32, u32) -> u32>,
}

/// Compact native signature: parameter types + return type.
#[cfg(feature = "native-extensions")]
#[derive(Clone, Debug)]
struct NativeSig {
    params: Vec<ArgT>,
    ret: Option<ArgT>,
}

/// Side table: library index → (native symbol name, signature).
/// Populated by `wire_native_fns`, read by the generic dispatcher.
#[cfg(feature = "native-extensions")]
static NATIVE_SIGS: Mutex<Option<HashMap<u16, (String, NativeSig)>>> = Mutex::new(None);

/// #303 — wire-time signature of a shared-store bridge function, derived from
/// the ONE marshallability judgment (`native_gate::classify_bridge_attr`).
/// The interpreter call site pushes EVERY attribute, so `pops` has one entry
/// per attribute (declaration order) and drives the dispatcher's stack pops;
/// the generated bridge reads only the marshalled subset (compacted order), so
/// `forward` marks which popped slots become `LibArg`s.  `text_workbuf` is the
/// `pops` index of the `text_return` work buffer: in a non-dest call
/// (expression context) the dispatcher reuses that caller-cleared cell as
/// `bridge_text_dest` and returns a `Str` over it.
#[cfg(feature = "native-extensions")]
#[derive(Clone)]
struct SharedSig {
    pops: Vec<ArgT>,
    forward: Vec<bool>,
    text_workbuf: Option<usize>,
    ret: Option<ArgT>,
}

/// @PLN11 Arc N — side table for the **shared-store** bridge
/// (`native_lib::generate_shared_cdylib_lib_rs`): library index → (bridge fn ptr,
/// signature).  Populated by `wire_shared_native_fns`, read by
/// `shared_store_dispatch`.  Separate from `NATIVE_SIGS` because the ABI differs
/// (a `*mut Stores` + `LibArg` bridge, not the `LoftStore`/raw-ptr marshalling).
#[cfg(feature = "native-extensions")]
static SHARED_SIGS: Mutex<Option<HashMap<u16, (FnPtr, SharedSig)>>> = Mutex::new(None);

/// Compute the argument type list and return type from a definition's signature.
/// Returns `None` if the signature contains types that can't be auto-marshalled
/// (e.g. struct references, vectors).
#[cfg(feature = "native-extensions")]
fn compute_sig(data: &crate::data::Data, d_nr: u32) -> Option<NativeSig> {
    use crate::data::Type;
    let def = data.def(d_nr);
    let mut params = Vec::new();
    for attr in &def.attributes {
        let t = match &attr.typedef {
            // @P370: a plain loft `integer` is 64-bit (8-byte slot) — it must
            // marshal as I64.  Only an EXPLICIT narrow integer (`u8/i8/u16/i16/
            // i32`, which carry `forced_size`) or a `Character` (4-byte
            // codepoint) is ≤4 bytes → I32.  Auto-converting `integer` to i32
            // truncated i64 values (e.g. an `i64::MIN` null sentinel → 0) and
            // diverged from `--native` (which uses the lib's real i64 ABI).
            Type::Integer(s) if s.forced_size.is_none() => ArgT::I64,
            Type::Integer(_) | Type::Character => ArgT::I32,
            Type::Float => ArgT::F64,
            Type::Single => ArgT::F32,
            Type::Boolean => ArgT::Bool,
            Type::Text(_) => ArgT::Text,
            Type::Enum(_, false, _) => ArgT::I32, // simple enum tag
            Type::Reference(_, _)
            | Type::Enum(_, true, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Spacial(_, _, _) => ArgT::Ref,
            Type::Vector(_, _) => ArgT::Vec,
            _ => return None,
        };
        params.push(t);
    }
    let ret = match &def.returned {
        Type::Void | Type::Null => None,
        // @P370: plain loft `integer` is 64-bit → I64; only explicit narrow
        // ints (`forced_size`) and `Character` are ≤4 bytes → I32.
        Type::Integer(s) if s.forced_size.is_none() => Some(ArgT::I64),
        Type::Integer(_) | Type::Character => Some(ArgT::I32),
        Type::Float => Some(ArgT::F64),
        Type::Single => Some(ArgT::F32),
        Type::Boolean => Some(ArgT::Bool),
        Type::Text(_) => Some(ArgT::Text),
        Type::Enum(_, false, _) => Some(ArgT::I32),
        Type::Reference(_, _)
        | Type::Enum(_, true, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Spacial(_, _, _) => Some(ArgT::Ref),
        Type::Vector(_, _) => Some(ArgT::Vec),
        _ => return None,
    };
    Some(NativeSig { params, ret })
}

/// The `ArgT` mapping for one marshalled loft type (shared-bridge param or
/// return).  `None` = not marshallable.
#[cfg(feature = "native-extensions")]
fn marshal_arg_t(t: &crate::data::Type) -> Option<ArgT> {
    use crate::data::Type;
    Some(match t {
        // @P370: plain loft `integer` is 64-bit → I64; only explicit narrow
        // ints (`forced_size`) and `Character` are ≤4 bytes → I32.
        Type::Integer(s) if s.forced_size.is_none() => ArgT::I64,
        Type::Integer(_) | Type::Character => ArgT::I32,
        Type::Float => ArgT::F64,
        Type::Single => ArgT::F32,
        Type::Boolean => ArgT::Bool,
        Type::Text(_) => ArgT::Text,
        Type::Enum(_, false, _) => ArgT::I32,
        Type::Reference(_, _)
        | Type::Enum(_, true, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Spacial(_, _, _) => ArgT::Ref,
        Type::Vector(_, _) => ArgT::Vec,
        _ => return None,
    })
}

/// #303 — build a [`SharedSig`] from a definition, via the ONE marshallability
/// judgment (`native_gate::classify_bridge_attr`).  Returns `None` exactly when
/// the gate would not have marked the function — so a marked function always
/// wires (the divergence that left panicking stubs behind emitted dispatches).
#[cfg(feature = "native-extensions")]
fn compute_shared_sig(data: &crate::data::Data, d_nr: u32) -> Option<SharedSig> {
    use crate::data::Type;
    use crate::native_gate::{BridgeAttrKind, classify_bridge_attr};
    let def = data.def(d_nr);
    let ret_text = matches!(def.returned(), Type::Text(_));
    let mut pops = Vec::new();
    let mut forward = Vec::new();
    let mut text_workbuf = None;
    for attr in &def.attributes {
        match classify_bridge_attr(attr, ret_text)? {
            BridgeAttrKind::Marshal => {
                pops.push(marshal_arg_t(&attr.typedef)?);
                forward.push(true);
            }
            BridgeAttrKind::WorkText => {
                // The call site pushes the work buffer's `DbRef` (a CreateStack
                // cell) — popped like any ref, never forwarded.
                if text_workbuf.is_none() {
                    text_workbuf = Some(pops.len());
                }
                pops.push(ArgT::Ref);
                forward.push(false);
            }
            BridgeAttrKind::HiddenDest => {
                // The call site pushes the caller-allocated placeholder; the
                // bridge allocates its own dest — popped, never forwarded.
                pops.push(ArgT::Vec);
                forward.push(false);
            }
        }
    }
    let ret = match def.returned() {
        Type::Void | Type::Null => None,
        t => Some(marshal_arg_t(t)?),
    };
    Some(SharedSig {
        pops,
        forward,
        text_workbuf,
        ret,
    })
}

/// Set of symbols that were registered as stubs (not hand-written glue).
/// Only these should be replaced by auto-marshalled wrappers.
static STUB_SYMBOLS: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

/// Record which symbols are stubs (called from `register_native_stubs`).
pub fn set_stub_symbols(syms: std::collections::HashSet<String>) {
    *STUB_SYMBOLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(syms);
}

/// After `load_all()` has populated `NATIVE_REGISTRY`, iterate all `#native`
/// definitions and replace the panic stubs with auto-marshalled wrappers.
///
/// For symbols not found in the registry (i.e. the cdylib didn't use
/// `loft_register_v1`), falls back to direct `dlsym` lookup — enabling
/// zero-registration cdylibs that just export `extern "C" fn n_*()`.
///
/// Functions already registered by `native::init()` are skipped — their
/// stubs were never created by `register_native_stubs`.
///
/// # Panics
/// Panics if a symbol is found via dlsym but the library used `loft_register_v1`
/// (indicating a registration bug).
#[cfg(feature = "native-extensions")]
pub fn wire_native_fns(state: &mut crate::state::State, data: &crate::data::Data) {
    // Phase 1: resolve any missing symbols via dlsym.
    {
        let stub_guard = STUB_SYMBOLS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stub_syms = stub_guard.as_ref();
        let reg_guard = NATIVE_REGISTRY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut to_resolve: Vec<String> = Vec::new();
        for d_nr in 0..data.definitions() {
            let def = data.def(d_nr);
            if def.native.is_empty() {
                continue;
            }
            let sym = &def.native;
            // @PLN11 Arc N: shared-store bridges are wired by
            // `wire_shared_native_fns` (a different ABI) — skip them here.
            if sym.starts_with("loft_shared_") {
                continue;
            }
            if let Some(stubs) = stub_syms
                && !stubs.contains(sym)
            {
                continue;
            }
            let found = reg_guard.as_ref().is_some_and(|r| r.contains_key(sym));
            if !found {
                to_resolve.push(sym.clone());
            }
        }
        drop(reg_guard);
        drop(stub_guard);

        // Resolve via dlsym (no locks held).  The guard is now keyed on the
        // *resolving library's* own `uses_v1` flag (returned by `try_dlsym`), not
        // a global "registry non-empty" proxy — so a zero-registration cdylib
        // loaded after one that used the protocol no longer false-positives, while
        // issue #119 (a v1 library's unregistered-but-dlsym-found symbol) still
        // panics.
        for sym in to_resolve {
            if let Some((ptr, lib_uses_v1)) = try_dlsym(&sym) {
                // The resolving library used loft_register_v1 but didn't register
                // this symbol — this is a registration bug.
                assert!(
                    !lib_uses_v1,
                    "native symbol '{sym}' was not registered via loft_register_v1 \
                     but was found via dlsym. This is a registration bug — \
                     add reg!(b\"{sym}\", <fn>) to loft_register_v1.",
                );
                let mut rg = NATIVE_REGISTRY
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                rg.get_or_insert_with(HashMap::new).insert(sym, FnPtr(ptr));
            }
        }
    }

    // Phase 2: wire auto-marshalled dispatchers.
    let guard = NATIVE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let registry = match guard.as_ref() {
        Some(r) => r,
        None => return,
    };

    let mut sigs = NATIVE_SIGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sig_table = sigs.get_or_insert_with(HashMap::new);

    let stub_guard = STUB_SYMBOLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let stub_syms = stub_guard.as_ref();

    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.native.is_empty() {
            continue;
        }
        let sym = &def.native;

        // @PLN11 Arc N: shared-store bridges use a different dispatcher — skip.
        if sym.starts_with("loft_shared_") {
            continue;
        }

        // Only replace stubs — skip hand-written glue from native::init().
        if let Some(stubs) = stub_syms
            && !stubs.contains(sym)
        {
            continue;
        }

        if !registry.contains_key(sym) {
            continue;
        }

        // Only wire if we can auto-marshal the signature.
        let sig = match compute_sig(data, d_nr) {
            Some(s) => s,
            None => continue,
        };

        // Get the library index for this symbol.
        let lib_idx = match state.library_names.get(sym) {
            Some(&idx) => idx,
            None => continue,
        };

        // Store the signature for runtime dispatch.
        sig_table.insert(lib_idx, (sym.clone(), sig));

        // Replace the stub with the generic auto-marshal dispatcher.
        state.replace_static_fn(sym, native_auto_dispatch);
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn wire_native_fns(_state: &mut crate::state::State, _data: &crate::data::Data) {}

/// @PLN11 Arc N — wire the **shared-store** bridge dispatchers.  For every
/// `#native "loft_shared_…"` definition (the marker for an auto-generated
/// shared-store bridge), resolve the bridge symbol from the loaded cdylibs via
/// dlsym, record `(bridge_ptr, signature)` in `SHARED_SIGS`, and replace the stub
/// `OpStaticCall` target with `shared_store_dispatch`.  Call this *after*
/// `load_all` (and, if also using legacy `#native`, after `wire_native_fns` —
/// they handle disjoint symbol sets, `wire_native_fns` skips `loft_shared_…`).
#[cfg(feature = "native-extensions")]
pub fn wire_shared_native_fns(state: &mut crate::state::State, data: &crate::data::Data) {
    // Phase 1: collect resolvable shared bridges (no lock held).
    let mut wired: Vec<(String, u16, *const (), SharedSig)> = Vec::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        let sym = &def.native;
        if !sym.starts_with("loft_shared_") {
            continue;
        }
        // #303 — a `loft_shared_*`-marked def means codegen already emitted
        // `OpStaticCall` dispatches for it.  A wiring failure here leaves the
        // panicking stub behind those dispatches, so every skip must be LOUD:
        // the program will panic at the first call with a message that doesn't
        // name the function.
        let unwired = |reason: &str| {
            eprintln!(
                "loft: auto-native fn `{}` ({sym}) is marked for cdylib dispatch but \
                 could not be wired ({reason}) — calling it will panic",
                def.original_name(),
            );
        };
        let Some((ptr, _uses_v1)) = try_dlsym(sym) else {
            unwired("bridge symbol not found in any loaded cdylib");
            continue;
        };
        let Some(sig) = compute_shared_sig(data, d_nr) else {
            unwired("signature not bridge-marshallable (gate/wire divergence)");
            continue;
        };
        let Some(&lib_idx) = state.library_names.get(sym) else {
            unwired("no stub slot registered for the symbol");
            continue;
        };
        wired.push((sym.clone(), lib_idx, ptr, sig));
    }

    // Phase 2: record signatures, then replace the stub dispatch targets.
    {
        let mut guard = SHARED_SIGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let table = guard.get_or_insert_with(HashMap::new);
        for (_, lib_idx, ptr, sig) in &wired {
            table.insert(*lib_idx, (FnPtr(*ptr), sig.clone()));
        }
    }
    for (sym, ..) in &wired {
        state.replace_static_fn(sym, shared_store_dispatch);
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn wire_shared_native_fns(_state: &mut crate::state::State, _data: &crate::data::Data) {}

/// @PLN11 Arc N — the shared-store bridge dispatcher.  Invoked via `OpStaticCall`
/// for a function whose `#native` symbol is an auto-generated `loft_shared_…`
/// bridge.  Reads the call's args off the interpreter stack into `LibArg` slots
/// (scalars by value; `vector`/`reference` as the **raw** stack `DbRef`, no
/// deref — the `--native` body expects the same indirect form), passes the
/// caller's `*mut Stores` directly (zero-marshalling shared store), calls the
/// bridge, and writes the return back onto the stack.
#[cfg(feature = "native-extensions")]
fn shared_store_dispatch(stores: &mut crate::database::Stores, stack: &mut crate::keys::DbRef) {
    use crate::keys::DbRef;
    use crate::native_lib::LibArg;

    crate::state::SHARED_DISPATCH_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let lib_idx = CURRENT_LIB_IDX.with(std::cell::Cell::get);
    let (bridge_ptr, sig) = {
        let guard = SHARED_SIGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let table = guard.as_ref().expect("SHARED_SIGS not initialized");
        match table.get(&lib_idx) {
            Some((p, s)) => (p.0, s.clone()),
            None => panic!("no shared signature for lib_idx {lib_idx}"),
        }
    };

    // Pop ONE stack slot per attribute (the call site pushes every attribute,
    // including hidden dests and the text work buffer) — in reverse (stack is
    // LIFO), then restore declaration order.
    let mut popped: Vec<LibArg> = Vec::with_capacity(sig.pops.len());
    for &t in sig.pops.iter().rev() {
        let slot = match t {
            ArgT::I32 | ArgT::I64 => LibArg {
                scalar: *stores.get::<i64>(stack),
                ..LibArg::ZERO
            },
            ArgT::F64 => LibArg {
                scalar: (*stores.get::<f64>(stack)).to_bits() as i64,
                ..LibArg::ZERO
            },
            ArgT::F32 => LibArg {
                scalar: i64::from((*stores.get::<f64>(stack) as f32).to_bits()),
                ..LibArg::ZERO
            },
            ArgT::Bool => LibArg {
                scalar: i64::from(*stores.get::<bool>(stack)),
                ..LibArg::ZERO
            },
            // The raw stack DbRef, passed UNCHANGED (the --native body consumes
            // the indirect-header form, not the dereferenced direct record).
            ArgT::Ref | ArgT::Vec => LibArg {
                dbref: *stores.get::<DbRef>(stack),
                ..LibArg::ZERO
            },
            // Text arg → `&str` for the body: the store-backed bytes (borrowed for
            // the call's duration, valid because the store is shared and live).
            ArgT::Text => {
                let s = *stores.get::<crate::keys::Str>(stack);
                LibArg {
                    text_ptr: s.ptr,
                    text_len: s.len as usize,
                    ..LibArg::ZERO
                }
            }
        };
        popped.push(slot);
    }
    popped.reverse();
    // Forward only the marshalled slots (compacted order — the bridge reads
    // `a[0..n]` skipping work buffers and hidden dests it satisfies locally).
    let args: Vec<LibArg> = popped
        .iter()
        .zip(&sig.forward)
        .filter(|(_, f)| **f)
        .map(|(a, _)| *a)
        .collect();

    // @PLN10 — dest-mode detection: the interpreter caller routes a text-returning
    // auto-native call through `gen_cdylib_text_dest_call`, which set a per-call
    // `bridge_text_dest` before this dispatch.  The bridge consumes it (writes the
    // result into that caller-owned record) and signals dest-mode by leaving `ret`
    // text null — so we must push NOTHING here, matching the codegen's dest-mode
    // stack accounting.  Captured before the call because the bridge `take()`s it.
    let text_dest_mode = stores.bridge_text_dest.is_some();
    // #303 — expression-context text return: no external dest was stashed, so
    // reuse the call's own work buffer (a caller-cleared CreateStack cell whose
    // DbRef the call site pushed) as the destination; after the call a `Str`
    // over its content is pushed as the result, mirroring the interpreted ABI.
    let self_dest: Option<DbRef> = if !text_dest_mode && matches!(sig.ret, Some(ArgT::Text)) {
        sig.text_workbuf.map(|i| {
            let d = popped[i].dbref;
            stores.bridge_text_dest = Some(d);
            d
        })
    } else {
        None
    };

    let mut ret = LibArg::ZERO;
    let stores_ptr: *mut crate::database::Stores = stores;
    // SAFETY: `bridge_ptr` is a `loft_shared_…` export of an auto-generated cdylib
    // that links this exact `LibArg` / `Stores` / `DbRef` from libloft, so the ABI
    // matches.  `stores_ptr` is borrowed from the live `&mut Stores`; the bridge
    // uses it (and only it) for the duration of the call.
    let bridge: unsafe extern "C" fn(
        *mut crate::database::Stores,
        *const LibArg,
        usize,
        *mut LibArg,
    ) = unsafe { std::mem::transmute(bridge_ptr) };
    unsafe {
        bridge(
            stores_ptr,
            args.as_ptr(),
            args.len(),
            std::ptr::from_mut(&mut ret),
        )
    };

    match sig.ret {
        None => {}
        Some(ArgT::I32 | ArgT::I64) => stores.put::<i64>(stack, ret.scalar),
        Some(ArgT::F64) => stores.put::<f64>(stack, f64::from_bits(ret.scalar as u64)),
        Some(ArgT::F32) => stores.put::<f64>(stack, f64::from(f32::from_bits(ret.scalar as u32))),
        Some(ArgT::Bool) => stores.put(stack, ret.scalar != 0),
        Some(ArgT::Ref | ArgT::Vec) => stores.put::<DbRef>(stack, ret.dbref),
        // @PLN10 — dest-passing: in dest-mode the bridge wrote the result into the
        // caller-owned `bridge_text_dest` record and left `ret` text null; the value
        // lives in its destination, so push NOTHING (matches the dest-mode codegen).
        Some(ArgT::Text) if text_dest_mode => {}
        // #303 — expression context: the bridge wrote the result into the
        // self-stashed work buffer; push a `Str` over its content (the cell
        // outlives the expression — it is the call's own CreateStack cell),
        // matching the +16-byte result the call-site codegen accounts.
        Some(ArgT::Text) => {
            if let Some(d) = self_dest {
                let s: &String = stores.store(&d).addr::<String>(d.rec, d.pos);
                let result = crate::keys::Str {
                    ptr: s.as_ptr(),
                    len: s.len() as u32,
                };
                stores.put(stack, result);
            } else {
                // A text return with neither an external dest nor a work
                // buffer (e.g. a constant-text body) — there is no caller cell
                // to carry the bytes; degrade to an empty `Str` and flag in dev.
                debug_assert!(
                    ret.text_ptr.is_null(),
                    "auto-native text return reached the dispatcher without any \
                     dest cell — @PLN10 dest-passing coverage gap"
                );
                stores.put(stack, crate::keys::Str::new(""));
            }
        }
    }
}

/// Generic auto-marshal dispatcher. Called via `OpStaticCall` for all
/// auto-wired native functions. Reads signature from `NATIVE_SIGS` using
/// the library index stored in the bytecode (passed via the stack frame).
///
/// Since `Call = fn(&mut Stores, &mut DbRef)` doesn't receive the library
/// index, we use CURRENT_LIB_IDX (set by a patched static_call).
///
/// Actually — `State::static_call()` doesn't pass the library index to
/// the Call function. We need a different mechanism.
///
/// Solution: use a thread-local that `static_call` sets before invoking.
#[cfg(feature = "native-extensions")]
fn native_auto_dispatch(stores: &mut crate::database::Stores, stack: &mut crate::keys::DbRef) {
    use crate::keys::Str;

    // Read the current library index from the thread-local.
    let lib_idx = CURRENT_LIB_IDX.with(std::cell::Cell::get);

    // P245: clone the (sym, sig) entry out of NATIVE_SIGS and DROP the
    // mutex guard before invoking the native function.  The native fn
    // may block (e.g. `n_tcp_accept` waits in `listener.accept()`) —
    // holding the guard across the call serialises every parallel arm
    // that calls into native code, manifesting as a hang on the
    // sibling worker.  The Mutex is now used only for the table
    // lookup itself, never for the call.
    let (sym, sig) = {
        let guard = NATIVE_SIGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sig_table = guard.as_ref().expect("NATIVE_SIGS not initialized");
        match sig_table.get(&lib_idx) {
            Some((s, sg)) => (s.clone(), sg.clone()),
            None => panic!("no signature for lib_idx {lib_idx}"),
        }
    };

    // Plan-25 F3: dispatch through the generated bridge if the owning cdylib
    // registered one for this symbol; otherwise fall through to the legacy
    // raw-ptr arms below.
    if let Some(bridge_ptr) = get_bridge(&sym) {
        dispatch_via_bridge(stores, stack, bridge_ptr, &sig);
        return;
    }

    let fp = unsafe { get_native_fn_raw(&sym) }.unwrap_or_else(|| {
        panic!("native symbol '{sym}' not loaded");
    });

    let mut args: Vec<ArgVal> = Vec::with_capacity(sig.params.len());
    // Pop in reverse order (stack is LIFO).
    for &t in sig.params.iter().rev() {
        let val = match t {
            // Post-2c: loft `integer` is i64 on stack (8 bytes) but the
            // external C ABI may still declare `i32` — narrow here for
            // the call, then widen back when writing the return value.
            // Type::Character also maps to ArgT::I32 but is not currently
            // used by any cdylib (the existing arm stays safe under this
            // change because no Character parameter reaches here).
            ArgT::I32 => ArgVal::I32(*stores.get::<i64>(stack) as i32),
            ArgT::I64 => ArgVal::I64(*stores.get::<i64>(stack)),
            ArgT::F32 => ArgVal::F32(*stores.get::<f64>(stack) as f32),
            ArgT::F64 => ArgVal::F64(*stores.get::<f64>(stack)),
            ArgT::Bool => ArgVal::Bool(*stores.get::<bool>(stack)),
            ArgT::Text => {
                let s = *stores.get::<Str>(stack);
                ArgVal::Text(s.str().as_ptr(), s.str().len())
            }
            ArgT::Ref => {
                let r = *stores.get::<crate::keys::DbRef>(stack);
                ArgVal::Ref(r.store_nr, r.rec, r.pos)
            }
            ArgT::Vec => {
                // Vector refs are indirect: the DbRef on the stack points to
                // a location that *contains* the vector data record number.
                // Dereference to get the actual vector record, matching how
                // vector::length_vector works (get_int(rec, pos) -> vec_rec).
                let r = *stores.get::<crate::keys::DbRef>(stack);
                if r.rec == 0 || r.pos == 0 {
                    ArgVal::Ref(r.store_nr, 0, 0)
                } else {
                    let vec_rec = stores.store(&r).get_u32_raw(r.rec, r.pos);
                    ArgVal::Ref(r.store_nr, vec_rec, 0)
                }
            }
        };
        args.push(val);
    }
    args.reverse(); // Now in declaration order.

    // Dispatch based on exact signature pattern.
    dispatch_call(stores, stack, fp, &args, &sig.params, sig.ret);
}

/// Plan-25 F3: dispatch a native call through its generated `LoftBridgeFn`.
///
/// Marshals the stack args into `[LoftValue]` at FULL width (the bridge casts
/// each to the impl's real param type), builds the `LoftStore` from the first
/// ref arg's store (or store 0), calls the bridge, and writes the tagged
/// return back to the stack.  Replaces the ~98-arm `dispatch_call` for any
/// symbol whose cdylib was built with `#[loft_native]`.
#[cfg(feature = "native-extensions")]
fn dispatch_via_bridge(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    bridge_ptr: *const (),
    sig: &NativeSig,
) {
    use crate::keys::Str;
    use loft_ffi::{LoftRef as FfiRef, LoftStr as FfiStr, LoftTag, LoftValue};

    // Build args at full width (pop in reverse — LIFO — then restore order).
    let mut args: Vec<LoftValue> = Vec::with_capacity(sig.params.len());
    for &t in sig.params.iter().rev() {
        let v = match t {
            // No pre-narrowing: pass the whole i64 cell; the bridge casts to
            // the impl's real width (i32/u16/…) per its Rust signature.
            ArgT::I32 | ArgT::I64 => LoftValue::int(*stores.get::<i64>(stack)),
            ArgT::F32 | ArgT::F64 => LoftValue::float(*stores.get::<f64>(stack)),
            ArgT::Bool => LoftValue::boolean(*stores.get::<bool>(stack)),
            ArgT::Text => {
                let s = *stores.get::<Str>(stack);
                LoftValue::text(FfiStr {
                    ptr: s.str().as_ptr(),
                    len: s.str().len(),
                })
            }
            ArgT::Ref => {
                let r = *stores.get::<crate::keys::DbRef>(stack);
                LoftValue::reference(FfiRef {
                    store_nr: r.store_nr,
                    rec: r.rec,
                    pos: r.pos,
                })
            }
            ArgT::Vec => {
                // Same indirect-vector deref as the legacy marshal.
                let r = *stores.get::<crate::keys::DbRef>(stack);
                let rec = if r.rec == 0 || r.pos == 0 {
                    0
                } else {
                    stores.store(&r).get_u32_raw(r.rec, r.pos)
                };
                LoftValue::reference(FfiRef {
                    store_nr: r.store_nr,
                    rec,
                    pos: 0,
                })
            }
        };
        args.push(v);
    }
    args.reverse();

    // The store the bridge sees: the first ref arg's store (so ref/vector
    // params and allocating returns resolve correctly), else store 0.
    let store_nr = args
        .iter()
        .find_map(|v| (v.tag == LoftTag::Ref).then(|| v.as_ref().store_nr))
        .unwrap_or(0);
    let ls = make_loft_store(stores, store_nr);

    // CURRENT_STORES must be live for the bridge's ffi_claim/resize callbacks.
    struct StoresGuard;
    impl Drop for StoresGuard {
        fn drop(&mut self) {
            CURRENT_STORES.with(|c| c.set(std::ptr::null_mut()));
        }
    }
    let stores_ptr: *mut crate::database::Stores = stores;
    CURRENT_STORES.with(|c| c.set(stores_ptr));
    let _guard = StoresGuard;

    let mut ret = LoftValue::VOID;
    // SAFETY: `bridge_ptr` is a `loft_ffi::LoftBridgeFn` registered by the
    // cdylib; the local `LoftStore` mirror is `#[repr(C)]`-identical to
    // `loft_ffi::LoftStore`, so the ABI matches.
    let bridge: unsafe extern "C" fn(LoftStore, *const LoftValue, usize, *mut LoftValue) =
        unsafe { std::mem::transmute(bridge_ptr) };
    unsafe { bridge(ls, args.as_ptr(), args.len(), std::ptr::from_mut(&mut ret)) };

    // Write the tagged return to the stack (mirrors dispatch_call's returns).
    match ret.tag {
        LoftTag::Void => {}
        // The bridge already widened i32 returns (i32::MIN → i64::MIN).
        LoftTag::I64 => stores.put::<i64>(stack, ret.as_i64()),
        LoftTag::Bool => stores.put(stack, ret.as_bool()),
        LoftTag::F64 => stores.put(stack, ret.as_f64()),
        LoftTag::Text => bridge_push_str(stores, stack, ret.as_text()),
        LoftTag::Ref => bridge_push_ref(stores, stack, ret.as_ref()),
    }
}

/// Copy a returned `loft_ffi::LoftStr` into the stack (mirror of
/// `dispatch_call`'s nested `push_loft_str`, for the bridge path).
#[cfg(feature = "native-extensions")]
fn bridge_push_str(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    s: loft_ffi::LoftStr,
) {
    bridge_text_result(stores, stack, s.ptr, s.len);
}

/// Materialise a foreign `LoftStr` text return into the interpreter.
///
/// @PLN10 N2b — when `stores.bridge_text_dest` is set (by `n_set_bridge_dest`,
/// emitted immediately before this cdylib call by `gen_cdylib_text_dest_call`),
/// write the bytes into that caller-owned work-buffer record and push NOTHING —
/// the record IS the result (read by the chokepoint's `Var(w)`), so the
/// never-cleared `stores.scratch` is bypassed.  Otherwise fall back to the legacy
/// scratch-backed `Str` (the value-position case the chokepoint hasn't wrapped).
#[cfg(feature = "native-extensions")]
fn bridge_text_result(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    ptr: *const u8,
    len: usize,
) {
    use crate::keys::Str;
    let text: &str = if !ptr.is_null() && len > 0 {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
    } else {
        ""
    };
    if let Some(dest) = stores.bridge_text_dest.take() {
        if !text.is_empty() {
            stores
                .store_mut(&dest)
                .addr_mut::<String>(dest.rec, dest.pos)
                .push_str(text);
        }
        return;
    }
    // @PLN10 D/G2 — no dest set ⇒ this cdylib text call was NOT routed through
    // `n_set_bridge_dest` (an uncovered value position).  Dest-passing covers
    // every position in the corpus (whole-suite `=panic` == 0), so this is dead;
    // degrade gracefully to an empty `Str` rather than re-introduce
    // `stores.scratch` (the field is being retired), and flag the coverage gap
    // loudly in dev builds.
    debug_assert!(
        false,
        "cdylib text return reached the bridge without a dest (uncovered value \
         position) — @PLN10 N2b coverage gap"
    );
    stores.put(stack, Str::new(""));
}

/// Wrap a returned `loft_ffi::LoftRef` (direct vector record) in the indirect
/// header layout the interpreter expects, and push it (mirror of
/// `dispatch_call`'s nested `push_loft_ref`, for the bridge path).
#[cfg(feature = "native-extensions")]
fn bridge_push_ref(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    r: loft_ffi::LoftRef,
) {
    let base = crate::keys::DbRef {
        store_nr: r.store_nr,
        rec: 0,
        pos: 0,
    };
    let header = stores.claim(&base, 1);
    stores.store_mut(&base).set_u32_raw(header.rec, 4, r.rec);
    let dbref = crate::keys::DbRef {
        store_nr: r.store_nr,
        rec: header.rec,
        pos: 4,
    };
    stores.put(stack, dbref);
}

// Thread-local: current library index being dispatched.
// Set by `State::static_call()` before invoking the Call function.
std::thread_local! {
    static CURRENT_LIB_IDX: std::cell::Cell<u16> = const { std::cell::Cell::new(0) };
}

// ── Store callback infrastructure for FFI allocation ─────────────────────

// Thread-local raw pointer to the interpreter's Stores during a native call.
// Set before calling a native function, cleared after it returns.
#[cfg(feature = "native-extensions")]
std::thread_local! {
    pub(crate) static CURRENT_STORES: std::cell::Cell<*mut crate::database::Stores> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// C-ABI callback: allocate `words` 8-byte words in the store identified by ctx.
/// Returns the new record number. May reallocate the store buffer.
/// Returns 0 if the allocation panics (caught to prevent UB at the C-ABI boundary).
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn ffi_claim(ctx: LoftStoreCtx, words: u32) -> u32 {
    let store_nr = ctx._opaque as usize as u16;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CURRENT_STORES.with(|c| {
            let stores = unsafe { &mut *c.get() };
            let store = &mut stores.allocations[store_nr as usize];
            store.claim(words)
        })
    }))
    .unwrap_or(0)
}

/// C-ABI callback: resize record `rec` to `words` 8-byte words.
/// Returns the (possibly new) record number. May reallocate the store buffer.
/// Returns `rec` unchanged if the resize panics.
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn ffi_resize(ctx: LoftStoreCtx, rec: u32, words: u32) -> u32 {
    let store_nr = ctx._opaque as usize as u16;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CURRENT_STORES.with(|c| {
            let stores = unsafe { &mut *c.get() };
            let store = &mut stores.allocations[store_nr as usize];
            store.resize(rec, words)
        })
    }))
    .unwrap_or(rec)
}

/// C-ABI callback: refresh ptr and size after a potential reallocation.
/// No-op if the reload panics (ptr/size remain unchanged).
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn ffi_reload(ctx: LoftStoreCtx, out_ptr: *mut *mut u8, out_size: *mut u32) {
    let store_nr = ctx._opaque as usize as u16;
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CURRENT_STORES.with(|c| {
            let stores = unsafe { &*c.get() };
            let store = &stores.allocations[store_nr as usize];
            unsafe {
                *out_ptr = store.base_ptr();
                *out_size = store.capacity_words();
            }
        });
    }));
}

/// Set the current library index for auto-dispatch. Called from `State::static_call()`.
pub fn set_current_lib_idx(idx: u16) {
    CURRENT_LIB_IDX.with(|c| c.set(idx));
}

/// Look up a raw function pointer by symbol name.
#[cfg(feature = "native-extensions")]
unsafe fn get_native_fn_raw(name: &str) -> Option<*const ()> {
    let guard = NATIVE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.as_ref()?.get(name).map(|fp| fp.0)
}

/// Build a LoftStore handle from the store that a LoftRef points to.
/// Includes allocation callbacks so native code can create records and vectors.
#[cfg(feature = "native-extensions")]
fn make_loft_store(stores: &crate::database::Stores, store_nr: u16) -> LoftStore {
    let store = stores.store(&crate::keys::DbRef {
        store_nr,
        rec: 0,
        pos: 0,
    });
    LoftStore {
        ptr: store.base_ptr(),
        size: store.capacity_words(),
        ctx: LoftStoreCtx {
            _opaque: store_nr as usize as *mut (),
        },
        claim_fn: Some(ffi_claim),
        reload_fn: Some(ffi_reload),
        resize_fn: Some(ffi_resize),
    }
}

/// Find the first Ref argument and return its store_nr.
#[cfg(feature = "native-extensions")]
fn first_ref_store(args: &[ArgVal]) -> u16 {
    for a in args {
        if let ArgVal::Ref(s, _, _) = a {
            return *s;
        }
    }
    0
}

/// Dispatch a C-ABI call based on argument types and return type.
/// This matches on common signature patterns and calls with the right cast.
#[cfg(feature = "native-extensions")]
#[allow(clippy::too_many_lines)]
fn dispatch_call(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    fp: *const (),
    args: &[ArgVal],
    params: &[ArgT],
    ret: Option<ArgT>,
) {
    // Normalize Vec → Ref for dispatch matching. The vector dereference
    // already happened during argument extraction, so Vec and Ref have
    // identical calling conventions at this point.
    let norm_params: Vec<ArgT> = params
        .iter()
        .map(|t| if *t == ArgT::Vec { ArgT::Ref } else { *t })
        .collect();
    let params = &norm_params[..];
    let ret = ret.map(|t| if t == ArgT::Vec { ArgT::Ref } else { t });

    // Set up thread-local so FFI callbacks can reach the stores.
    // Uses a guard to ensure cleanup even if a native call panics.
    struct StoresGuard;
    impl Drop for StoresGuard {
        fn drop(&mut self) {
            CURRENT_STORES.with(|c| c.set(std::ptr::null_mut()));
        }
    }
    let stores_ptr: *mut crate::database::Stores = stores;
    CURRENT_STORES.with(|c| c.set(stores_ptr));
    let _guard = StoresGuard;

    // Helper macros to extract typed args.
    macro_rules! i32_arg {
        ($idx:expr) => {
            match &args[$idx] {
                ArgVal::I32(v) => *v,
                _ => unreachable!(),
            }
        };
    }
    macro_rules! i64_arg {
        ($idx:expr) => {
            match &args[$idx] {
                ArgVal::I64(v) => *v,
                _ => unreachable!(),
            }
        };
    }
    macro_rules! f64_arg {
        ($idx:expr) => {
            match &args[$idx] {
                ArgVal::F64(v) => *v,
                _ => unreachable!(),
            }
        };
    }
    macro_rules! f32_arg {
        ($idx:expr) => {
            match &args[$idx] {
                ArgVal::F32(v) => *v,
                _ => unreachable!(),
            }
        };
    }
    macro_rules! ref_arg {
        ($idx:expr) => {
            match &args[$idx] {
                ArgVal::Ref(s, r, p) => LoftRef {
                    store_nr: *s,
                    rec: *r,
                    pos: *p,
                },
                _ => unreachable!(),
            }
        };
    }
    macro_rules! text_arg {
        ($idx:expr) => {
            match &args[$idx] {
                ArgVal::Text(p, l) => (*p, *l),
                _ => unreachable!(),
            }
        };
    }

    // Helper: copy a LoftStr into stores.scratch and push onto the stack.
    #[inline]
    fn push_loft_str(
        stores: &mut crate::database::Stores,
        stack: &mut crate::keys::DbRef,
        s: LoftStr,
    ) {
        // @PLN10 N2b — shares the bridge dest-passing path (see `bridge_text_result`).
        bridge_text_result(stores, stack, s.ptr, s.len);
    }

    // Helper: widen an i32 native return to i64, preserving the null
    // sentinel.  Cdylibs predate the i64 migration and return `i32::MIN`
    // to mean "null"; without this preservation, `i64::from(i32::MIN)`
    // is just `-2147483648`, a regular non-null integer.  The loft-side
    // `if !x { … }` check then misses the failure (e.g.
    // `gl_load_font` errors silently flow through, font_idx stays
    // negative, every text texture renders empty — Brick Buster's
    // entire HUD/title/score disappears).  Map i32::MIN → i64::MIN so
    // the sentinel survives the boundary.
    #[inline]
    fn widen_int(v: i32) -> i64 {
        if v == i32::MIN {
            i64::MIN
        } else {
            i64::from(v)
        }
    }

    // Helper: convert a LoftRef from a native cdylib back to DbRef and push
    // onto the stack.  Native cdylibs return a direct vector reference
    // (rec=data_rec, pos=8) but the interpreter expects an indirect layout
    // where a header record at pos contains a pointer to the data record.
    // Allocate a header record in the same store and wrap the reference.
    fn push_loft_ref(
        stores: &mut crate::database::Stores,
        stack: &mut crate::keys::DbRef,
        r: LoftRef,
    ) {
        let base = crate::keys::DbRef {
            store_nr: r.store_nr,
            rec: 0,
            pos: 0,
        };
        let header = stores.claim(&base, 1);
        stores.store_mut(&base).set_u32_raw(header.rec, 4, r.rec);
        let dbref = crate::keys::DbRef {
            store_nr: r.store_nr,
            rec: header.rec,
            pos: 4,
        };
        stores.put(stack, dbref);
    }

    match (params, ret) {
        // () -> void
        (&[], None) => {
            let f: extern "C" fn() = unsafe { std::mem::transmute(fp) };
            f();
        }
        // () -> i32
        (&[], Some(ArgT::I32)) => {
            let f: extern "C" fn() -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f()));
        }
        // () -> bool
        (&[], Some(ArgT::Bool)) => {
            let f: extern "C" fn() -> bool = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f());
        }
        // () -> text (LoftStr return)
        (&[], Some(ArgT::Text)) => {
            let f: extern "C" fn() -> LoftStr = unsafe { std::mem::transmute(fp) };
            push_loft_str(stores, stack, f());
        }
        // (i32) -> void
        (&[ArgT::I32], None) => {
            let f: extern "C" fn(i32) = unsafe { std::mem::transmute(fp) };
            f(i32_arg!(0));
        }
        // (i32) -> i32
        (&[ArgT::I32], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f(i32_arg!(0))));
        }
        // (i32) -> bool
        (&[ArgT::I32], Some(ArgT::Bool)) => {
            let f: extern "C" fn(i32) -> bool = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i32_arg!(0)));
        }
        // (i64) -> void
        (&[ArgT::I64], None) => {
            let f: extern "C" fn(i64) = unsafe { std::mem::transmute(fp) };
            f(i64_arg!(0));
        }
        // (i32, i32) -> i32
        (&[ArgT::I32, ArgT::I32], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f(i32_arg!(0), i32_arg!(1))));
        }
        // (i32, f64) -> i32
        (&[ArgT::I32, ArgT::F64], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32, f64) -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f(i32_arg!(0), f64_arg!(1))));
        }
        // (i32, f64) -> f64  (e.g. gl_font_ascent: i32, f32→f32 widened to f64)
        (&[ArgT::I32, ArgT::F64], Some(ArgT::F64)) => {
            let f: extern "C" fn(i32, f64) -> f64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i32_arg!(0), f64_arg!(1)));
        }
        // (i32, i32) -> void
        (&[ArgT::I32, ArgT::I32], None) => {
            let f: extern "C" fn(i32, i32) = unsafe { std::mem::transmute(fp) };
            f(i32_arg!(0), i32_arg!(1));
        }
        // (i32, text) -> bool
        (&[ArgT::I32, ArgT::Text], Some(ArgT::Bool)) => {
            let f: extern "C" fn(i32, *const u8, usize) -> bool =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put(stack, f(i32_arg!(0), p, l));
        }
        // (i32, text) -> void  (e.g. tcp_respond: integer→u16 cast in cdylib)
        (&[ArgT::I32, ArgT::Text], None) => {
            let f: extern "C" fn(u16, *const u8, usize) = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(i32_arg!(0) as u16, p, l);
        }
        // (i32, text, text) -> void  (e.g. tcp_respond_typed:
        // status, body, content_type — TTT v3's HTML/CSS/JS/loft-mime
        // server responses)
        (&[ArgT::I32, ArgT::Text, ArgT::Text], None) => {
            let f: extern "C" fn(u16, *const u8, usize, *const u8, usize) =
                unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(1);
            let (p1, l1) = text_arg!(2);
            f(i32_arg!(0) as u16, p0, l0, p1, l1);
        }
        // (text) -> i32
        (&[ArgT::Text], Some(ArgT::I32)) => {
            let f: extern "C" fn(*const u8, usize) -> i32 = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            stores.put::<i64>(stack, widen_int(f(p, l)));
        }
        // (text, text) -> i32
        (&[ArgT::Text, ArgT::Text], Some(ArgT::I32)) => {
            let f: extern "C" fn(*const u8, usize, *const u8, usize) -> i32 =
                unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(0);
            let (p1, l1) = text_arg!(1);
            stores.put::<i64>(stack, widen_int(f(p0, l0, p1, l1)));
        }
        // (i32, i32, text) -> bool  (e.g. gl_create_window: u32,u32,text→bool)
        (&[ArgT::I32, ArgT::I32, ArgT::Text], Some(ArgT::Bool)) => {
            let f: extern "C" fn(u32, u32, *const u8, usize) -> bool =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(2);
            stores.put(stack, f(i32_arg!(0) as u32, i32_arg!(1) as u32, p, l));
        }
        // (i32, text, f64) -> f64  (e.g. gl_measure_text: i32, text, f32→f32)
        (&[ArgT::I32, ArgT::Text, ArgT::F64], Some(ArgT::F64)) => {
            let f: extern "C" fn(i32, *const u8, usize, f64) -> f64 =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put(stack, f(i32_arg!(0), p, l, f64_arg!(2)));
        }
        // (f64) -> f64
        (&[ArgT::F64], Some(ArgT::F64)) => {
            let f: extern "C" fn(f64) -> f64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(f64_arg!(0)));
        }
        // (f64, f64) -> f64
        (&[ArgT::F64, ArgT::F64], Some(ArgT::F64)) => {
            let f: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(f64_arg!(0), f64_arg!(1)));
        }
        // (f64) -> i32
        (&[ArgT::F64], Some(ArgT::I32)) => {
            let f: extern "C" fn(f64) -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f(f64_arg!(0))));
        }
        // (i32, i32, i32) -> i32
        (&[ArgT::I32, ArgT::I32, ArgT::I32], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32, i32, i32) -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f(i32_arg!(0), i32_arg!(1), i32_arg!(2))));
        }
        // (i32, i32, i32) -> void
        (&[ArgT::I32, ArgT::I32, ArgT::I32], None) => {
            let f: extern "C" fn(i32, i32, i32) = unsafe { std::mem::transmute(fp) };
            f(i32_arg!(0), i32_arg!(1), i32_arg!(2));
        }
        // (i64) -> i64
        (&[ArgT::I64], Some(ArgT::I64)) => {
            let f: extern "C" fn(i64) -> i64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i64_arg!(0)));
        }
        // (text) -> bool
        (&[ArgT::Text], Some(ArgT::Bool)) => {
            let f: extern "C" fn(*const u8, usize) -> bool = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            stores.put(stack, f(p, l));
        }
        // (text) -> void
        (&[ArgT::Text], None) => {
            let f: extern "C" fn(*const u8, usize) = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            f(p, l);
        }
        // (text) -> text (LoftStr return)
        (&[ArgT::Text], Some(ArgT::Text)) => {
            let f: extern "C" fn(*const u8, usize) -> LoftStr = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            push_loft_str(stores, stack, f(p, l));
        }
        // (text, text) -> text (LoftStr return)
        (&[ArgT::Text, ArgT::Text], Some(ArgT::Text)) => {
            let f: extern "C" fn(*const u8, usize, *const u8, usize) -> LoftStr =
                unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(0);
            let (p1, l1) = text_arg!(1);
            push_loft_str(stores, stack, f(p0, l0, p1, l1));
        }
        // (text, text, text, text) -> i32
        (&[ArgT::Text, ArgT::Text, ArgT::Text, ArgT::Text], Some(ArgT::I32)) => {
            let f: extern "C" fn(
                *const u8,
                usize,
                *const u8,
                usize,
                *const u8,
                usize,
                *const u8,
                usize,
            ) -> i32 = unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(0);
            let (p1, l1) = text_arg!(1);
            let (p2, l2) = text_arg!(2);
            let (p3, l3) = text_arg!(3);
            // @P362: widen the i32 C return to the 8-byte loft `integer`
            // slot (put::<i64> + widen_int), exactly like every other
            // I32-return arm.  A bare `stores.put` infers `i32` and pushes
            // only 4 bytes, leaving `stack.pos` 4 short — the frame then
            // misaligns and the caller's next ref slot reads garbage
            // (n_http_do("GET",url,"","") → HttpResponse store panic).
            stores.put::<i64>(stack, widen_int(f(p0, l0, p1, l1, p2, l2, p3, l3)));
        }
        // (text, text) -> bool
        (&[ArgT::Text, ArgT::Text], Some(ArgT::Bool)) => {
            let f: extern "C" fn(*const u8, usize, *const u8, usize) -> bool =
                unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(0);
            let (p1, l1) = text_arg!(1);
            stores.put(stack, f(p0, l0, p1, l1));
        }
        // (i32) -> f64
        (&[ArgT::I32], Some(ArgT::F64)) => {
            let f: extern "C" fn(i32) -> f64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i32_arg!(0)));
        }
        // (i32) -> i64
        (&[ArgT::I32], Some(ArgT::I64)) => {
            let f: extern "C" fn(i32) -> i64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i32_arg!(0)));
        }
        // (i32, i32) -> bool
        (&[ArgT::I32, ArgT::I32], Some(ArgT::Bool)) => {
            let f: extern "C" fn(i32, i32) -> bool = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i32_arg!(0), i32_arg!(1)));
        }
        // (i32, i32, i32, i32) -> i32
        (&[ArgT::I32, ArgT::I32, ArgT::I32, ArgT::I32], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32, i32, i32, i32) -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(
                stack,
                widen_int(f(i32_arg!(0), i32_arg!(1), i32_arg!(2), i32_arg!(3))),
            );
        }
        // (i32, text) -> i32
        (&[ArgT::I32, ArgT::Text], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32, *const u8, usize) -> i32 = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put::<i64>(stack, widen_int(f(i32_arg!(0), p, l)));
        }
        // () -> i64
        (&[], Some(ArgT::I64)) => {
            let f: extern "C" fn() -> i64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f());
        }
        // () -> f64
        (&[], Some(ArgT::F64)) => {
            let f: extern "C" fn() -> f64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f());
        }
        // ── Ref patterns (LoftStore prepended as first C-ABI arg) ────
        // (Ref) -> I32
        (&[ArgT::Ref], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef) -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f(ls, ref_arg!(0))));
        }
        // (Ref) -> Bool
        (&[ArgT::Ref], Some(ArgT::Bool)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef) -> bool = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(ls, ref_arg!(0)));
        }
        // (Ref) -> Ref
        (&[ArgT::Ref], Some(ArgT::Ref)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef) -> LoftRef =
                unsafe { std::mem::transmute(fp) };
            push_loft_ref(stores, stack, f(ls, ref_arg!(0)));
        }
        // (Ref) -> void
        (&[ArgT::Ref], None) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef) = unsafe { std::mem::transmute(fp) };
            f(ls, ref_arg!(0));
        }
        // (Ref) -> Text
        (&[ArgT::Ref], Some(ArgT::Text)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef) -> LoftStr =
                unsafe { std::mem::transmute(fp) };
            push_loft_str(stores, stack, f(ls, ref_arg!(0)));
        }
        // (Text, Ref) -> Bool
        (&[ArgT::Text, ArgT::Ref], Some(ArgT::Bool)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, *const u8, usize, LoftRef) -> bool =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            stores.put(stack, f(ls, p, l, ref_arg!(1)));
        }
        // (Text, Ref) -> I32
        (&[ArgT::Text, ArgT::Ref], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, *const u8, usize, LoftRef) -> i32 =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            stores.put::<i64>(stack, widen_int(f(ls, p, l, ref_arg!(1))));
        }
        // (I32, Ref) -> I32
        (&[ArgT::I32, ArgT::Ref], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i32, LoftRef) -> i32 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f(ls, i32_arg!(0), ref_arg!(1))));
        }
        // (I32, Ref) -> Bool
        (&[ArgT::I32, ArgT::Ref], Some(ArgT::Bool)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i32, LoftRef) -> bool =
                unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(ls, i32_arg!(0), ref_arg!(1)));
        }
        // (Ref, I32) -> I32
        (&[ArgT::Ref, ArgT::I32], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32) -> i32 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, widen_int(f(ls, ref_arg!(0), i32_arg!(1))));
        }
        // (Ref, I32) -> void
        (&[ArgT::Ref, ArgT::I32], None) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32) = unsafe { std::mem::transmute(fp) };
            f(ls, ref_arg!(0), i32_arg!(1));
        }
        // (Ref, Text) -> Bool
        (&[ArgT::Ref, ArgT::Text], Some(ArgT::Bool)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, *const u8, usize) -> bool =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put(stack, f(ls, ref_arg!(0), p, l));
        }
        // (Ref, Ref) -> Bool
        (&[ArgT::Ref, ArgT::Ref], Some(ArgT::Bool)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, LoftRef) -> bool =
                unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(ls, ref_arg!(0), ref_arg!(1)));
        }
        // ── Scalar→Ref patterns (allocate a fresh store for the result) ──
        // (I32) -> Ref  (e.g. rand_indices)
        (&[ArgT::I32], Some(ArgT::Ref)) => {
            let result_db = stores.null();
            let ls = make_loft_store(stores, result_db.store_nr);
            let f: extern "C" fn(LoftStore, i32) -> LoftRef = unsafe { std::mem::transmute(fp) };
            push_loft_ref(stores, stack, f(ls, i32_arg!(0)));
        }
        // () -> Ref
        (&[], Some(ArgT::Ref)) => {
            let result_db = stores.null();
            let ls = make_loft_store(stores, result_db.store_nr);
            let f: extern "C" fn(LoftStore) -> LoftRef = unsafe { std::mem::transmute(fp) };
            push_loft_ref(stores, stack, f(ls));
        }
        // (I32, I32) -> Ref
        (&[ArgT::I32, ArgT::I32], Some(ArgT::Ref)) => {
            let result_db = stores.null();
            let ls = make_loft_store(stores, result_db.store_nr);
            let f: extern "C" fn(LoftStore, i32, i32) -> LoftRef =
                unsafe { std::mem::transmute(fp) };
            push_loft_ref(stores, stack, f(ls, i32_arg!(0), i32_arg!(1)));
        }
        // (Text) -> Ref
        (&[ArgT::Text], Some(ArgT::Ref)) => {
            let result_db = stores.null();
            let ls = make_loft_store(stores, result_db.store_nr);
            let f: extern "C" fn(LoftStore, *const u8, usize) -> LoftRef =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            push_loft_ref(stores, stack, f(ls, p, l));
        }
        // (I32, Ref, I32) -> I32  (e.g. scalar + vector + scalar)
        (&[ArgT::I32, ArgT::Ref, ArgT::I32], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i32, LoftRef, i32) -> i32 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(
                stack,
                widen_int(f(ls, i32_arg!(0), ref_arg!(1), i32_arg!(2))),
            );
        }
        // (I32, Ref, I32) -> void
        (&[ArgT::I32, ArgT::Ref, ArgT::I32], None) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i32, LoftRef, i32) = unsafe { std::mem::transmute(fp) };
            f(ls, i32_arg!(0), ref_arg!(1), i32_arg!(2));
        }
        // ── F32 patterns ─────────────────────────────────────────────
        // (F32) -> F32
        (&[ArgT::F32], Some(ArgT::F32)) => {
            let f: extern "C" fn(f32) -> f32 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f64::from(f(f32_arg!(0))));
        }
        // (F32, F32) -> F32
        (&[ArgT::F32, ArgT::F32], Some(ArgT::F32)) => {
            let f: extern "C" fn(f32, f32) -> f32 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f64::from(f(f32_arg!(0), f32_arg!(1))));
        }
        // (Bool) -> void  (e.g. gl_depth_mask)
        (&[ArgT::Bool], None) => {
            let f: extern "C" fn(bool) = unsafe { std::mem::transmute(fp) };
            if let ArgVal::Bool(v) = &args[0] {
                f(*v);
            }
        }
        // (F64) -> void  (e.g. gl_line_width, gl_point_size)
        (&[ArgT::F64], None) => {
            let f: extern "C" fn(f64) = unsafe { std::mem::transmute(fp) };
            f(f64_arg!(0));
        }
        // (I32, I32, I32, I32) -> void  (e.g. gl_viewport)
        (&[ArgT::I32, ArgT::I32, ArgT::I32, ArgT::I32], None) => {
            let f: extern "C" fn(i32, i32, i32, i32) = unsafe { std::mem::transmute(fp) };
            f(i32_arg!(0), i32_arg!(1), i32_arg!(2), i32_arg!(3));
        }
        // (I32, Text, Ref) -> void  (e.g. gl_set_uniform_mat4: program, name, vec_ref)
        (&[ArgT::I32, ArgT::Text, ArgT::Ref], None) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i32, *const u8, usize, LoftRef) =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(ls, i32_arg!(0), p, l, ref_arg!(2));
        }
        // (I32, Text, F64) -> void  (e.g. gl_set_uniform_float)
        (&[ArgT::I32, ArgT::Text, ArgT::F64], None) => {
            let f: extern "C" fn(i32, *const u8, usize, f64) = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(i32_arg!(0), p, l, f64_arg!(2));
        }
        // (I32, Text, I32) -> void  (e.g. gl_set_uniform_int)
        (&[ArgT::I32, ArgT::Text, ArgT::I32], None) => {
            let f: extern "C" fn(i32, *const u8, usize, i32) = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(i32_arg!(0), p, l, i32_arg!(2));
        }
        // (I32, Text, F64, F64, F64) -> void  (e.g. gl_set_uniform_vec3)
        (&[ArgT::I32, ArgT::Text, ArgT::F64, ArgT::F64, ArgT::F64], None) => {
            let f: extern "C" fn(i32, *const u8, usize, f64, f64, f64) =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(i32_arg!(0), p, l, f64_arg!(2), f64_arg!(3), f64_arg!(4));
        }
        // ── Graphics/canvas patterns ──────────────────────────────────
        // (Ref, I32, I32) -> I32  (e.g. get_pixel)
        (&[ArgT::Ref, ArgT::I32, ArgT::I32], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, i32) -> i32 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(
                stack,
                widen_int(f(ls, ref_arg!(0), i32_arg!(1), i32_arg!(2))),
            );
        }
        // (Ref, I32, I32, I32) -> void  (e.g. set_pixel, blend_pixel)
        (&[ArgT::Ref, ArgT::I32, ArgT::I32, ArgT::I32], None) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, i32, i32) =
                unsafe { std::mem::transmute(fp) };
            f(ls, ref_arg!(0), i32_arg!(1), i32_arg!(2), i32_arg!(3));
        }
        // (Ref, I32, I32, I32, I32) -> void  (e.g. hline, vline, draw_circle)
        (&[ArgT::Ref, ArgT::I32, ArgT::I32, ArgT::I32, ArgT::I32], None) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, i32, i32, i32) =
                unsafe { std::mem::transmute(fp) };
            f(
                ls,
                ref_arg!(0),
                i32_arg!(1),
                i32_arg!(2),
                i32_arg!(3),
                i32_arg!(4),
            );
        }
        // (Ref, I32, I32, I32, I32, I32) -> void  (e.g. fill_rect, draw_line)
        (
            &[
                ArgT::Ref,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
            ],
            None,
        ) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, i32, i32, i32, i32) =
                unsafe { std::mem::transmute(fp) };
            f(
                ls,
                ref_arg!(0),
                i32_arg!(1),
                i32_arg!(2),
                i32_arg!(3),
                i32_arg!(4),
                i32_arg!(5),
            );
        }
        // (Ref, I32, I32, I32, I32, I32, I32) -> void  (e.g. fill_triangle)
        (
            &[
                ArgT::Ref,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
            ],
            None,
        ) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, i32, i32, i32, i32, i32) =
                unsafe { std::mem::transmute(fp) };
            f(
                ls,
                ref_arg!(0),
                i32_arg!(1),
                i32_arg!(2),
                i32_arg!(3),
                i32_arg!(4),
                i32_arg!(5),
                i32_arg!(6),
            );
        }
        // (Ref, I32, I32, I32, I32, I32, I32, I32) -> void
        (
            &[
                ArgT::Ref,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
            ],
            None,
        ) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, i32, i32, i32, i32, i32, i32) =
                unsafe { std::mem::transmute(fp) };
            f(
                ls,
                ref_arg!(0),
                i32_arg!(1),
                i32_arg!(2),
                i32_arg!(3),
                i32_arg!(4),
                i32_arg!(5),
                i32_arg!(6),
                i32_arg!(7),
            );
        }
        // (Ref, I32, I32, I32, I32, I32, I32, I32, I32, I32) -> void  (e.g. draw_bezier)
        (
            &[
                ArgT::Ref,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
            ],
            None,
        ) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, i32, i32, i32, i32, i32, i32, i32, i32) =
                unsafe { std::mem::transmute(fp) };
            f(
                ls,
                ref_arg!(0),
                i32_arg!(1),
                i32_arg!(2),
                i32_arg!(3),
                i32_arg!(4),
                i32_arg!(5),
                i32_arg!(6),
                i32_arg!(7),
                i32_arg!(8),
                i32_arg!(9),
            );
        }
        // (Text, I32, I32, Ref) -> Bool  (e.g. save_png_raw)
        (&[ArgT::Text, ArgT::I32, ArgT::I32, ArgT::Ref], Some(ArgT::Bool)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, *const u8, usize, i32, i32, LoftRef) -> bool =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            stores.put(stack, f(ls, p, l, i32_arg!(1), i32_arg!(2), ref_arg!(3)));
        }
        // (Ref, Text) -> Bool  — already covered at line ~1003
        // (Ref, I32, I32) -> I32  (e.g. gl_upload_canvas)
        // Already covered by (Ref, I32, I32) -> I32 above
        //
        // (I32, Text, F64, Ref) -> I32  (e.g. rasterize_text_into)
        (&[ArgT::I32, ArgT::Text, ArgT::F64, ArgT::Ref], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i32, *const u8, usize, f64, LoftRef) -> i32 =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put::<i64>(
                stack,
                widen_int(f(ls, i32_arg!(0), p, l, f64_arg!(2), ref_arg!(3))),
            );
        }
        // (Ref, I32, Text, F64, I32, I32, I32) -> void  (e.g. draw_text)
        (
            &[
                ArgT::Ref,
                ArgT::I32,
                ArgT::Text,
                ArgT::F64,
                ArgT::I32,
                ArgT::I32,
                ArgT::I32,
            ],
            None,
        ) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, *const u8, usize, f64, i32, i32, i32) =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(2);
            f(
                ls,
                ref_arg!(0),
                i32_arg!(1),
                p,
                l,
                f64_arg!(3),
                i32_arg!(4),
                i32_arg!(5),
                i32_arg!(6),
            );
        }
        // (Ref, I32, F64) -> I32  (e.g. audio_play_raw: vector<single>, sample_rate, volume)
        (&[ArgT::Ref, ArgT::I32, ArgT::F64], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i32, f64) -> i32 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(
                stack,
                widen_int(f(ls, ref_arg!(0), i32_arg!(1), f64_arg!(2))),
            );
        }
        // ── @PLAN48 P1b: I64 arms — a plain loft `integer` now marshals as I64
        // (was I32).  These mirror the I32 twins above; an I64 return uses
        // `put::<i64>` directly (no `widen_int` — the value is already i64 and
        // a native i64 fn returns the i64::MIN null sentinel verbatim).  Until
        // FFI.1/FFI.3 generate these per-library (lib_plans/05-game-infra), they
        // are hand-written.  Shapes from the graphics impls + external `rand`.
        // (i64) -> bool
        (&[ArgT::I64], Some(ArgT::Bool)) => {
            let f: extern "C" fn(i64) -> bool = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i64_arg!(0)));
        }
        // (i64, i64) -> void
        (&[ArgT::I64, ArgT::I64], None) => {
            let f: extern "C" fn(i64, i64) = unsafe { std::mem::transmute(fp) };
            f(i64_arg!(0), i64_arg!(1));
        }
        // (i64, i64) -> i64
        (&[ArgT::I64, ArgT::I64], Some(ArgT::I64)) => {
            let f: extern "C" fn(i64, i64) -> i64 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(i64_arg!(0), i64_arg!(1)));
        }
        // (i64, i64, i64) -> void
        (&[ArgT::I64, ArgT::I64, ArgT::I64], None) => {
            let f: extern "C" fn(i64, i64, i64) = unsafe { std::mem::transmute(fp) };
            f(i64_arg!(0), i64_arg!(1), i64_arg!(2));
        }
        // (i64, i64, i64) -> i64
        (&[ArgT::I64, ArgT::I64, ArgT::I64], Some(ArgT::I64)) => {
            let f: extern "C" fn(i64, i64, i64) -> i64 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(i64_arg!(0), i64_arg!(1), i64_arg!(2)));
        }
        // (i64, i64, i64, i64) -> void
        (&[ArgT::I64, ArgT::I64, ArgT::I64, ArgT::I64], None) => {
            let f: extern "C" fn(i64, i64, i64, i64) = unsafe { std::mem::transmute(fp) };
            f(i64_arg!(0), i64_arg!(1), i64_arg!(2), i64_arg!(3));
        }
        // (text) -> i64
        (&[ArgT::Text], Some(ArgT::I64)) => {
            let f: extern "C" fn(*const u8, usize) -> i64 = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            stores.put::<i64>(stack, f(p, l));
        }
        // (text, text) -> i64
        (&[ArgT::Text, ArgT::Text], Some(ArgT::I64)) => {
            let f: extern "C" fn(*const u8, usize, *const u8, usize) -> i64 =
                unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(0);
            let (p1, l1) = text_arg!(1);
            stores.put::<i64>(stack, f(p0, l0, p1, l1));
        }
        // (i64, f64) -> i64
        (&[ArgT::I64, ArgT::F64], Some(ArgT::I64)) => {
            let f: extern "C" fn(i64, f64) -> i64 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(i64_arg!(0), f64_arg!(1)));
        }
        // (i64, f64) -> void
        (&[ArgT::I64, ArgT::F64], None) => {
            let f: extern "C" fn(i64, f64) = unsafe { std::mem::transmute(fp) };
            f(i64_arg!(0), f64_arg!(1));
        }
        // (i64, f64) -> f64
        (&[ArgT::I64, ArgT::F64], Some(ArgT::F64)) => {
            let f: extern "C" fn(i64, f64) -> f64 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i64_arg!(0), f64_arg!(1)));
        }
        // (i64, text, f64) -> void
        (&[ArgT::I64, ArgT::Text, ArgT::F64], None) => {
            let f: extern "C" fn(i64, *const u8, usize, f64) = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(i64_arg!(0), p, l, f64_arg!(2));
        }
        // (i64, text, i64) -> void
        (&[ArgT::I64, ArgT::Text, ArgT::I64], None) => {
            let f: extern "C" fn(i64, *const u8, usize, i64) = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(i64_arg!(0), p, l, i64_arg!(2));
        }
        // (i64, text, f64) -> f64
        (&[ArgT::I64, ArgT::Text, ArgT::F64], Some(ArgT::F64)) => {
            let f: extern "C" fn(i64, *const u8, usize, f64) -> f64 =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put(stack, f(i64_arg!(0), p, l, f64_arg!(2)));
        }
        // (i64, text, f64, f64, f64) -> void
        (&[ArgT::I64, ArgT::Text, ArgT::F64, ArgT::F64, ArgT::F64], None) => {
            let f: extern "C" fn(i64, *const u8, usize, f64, f64, f64) =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(i64_arg!(0), p, l, f64_arg!(2), f64_arg!(3), f64_arg!(4));
        }
        // (i64, text, ref) -> void  (e.g. gl_set_mat4)
        (&[ArgT::I64, ArgT::Text, ArgT::Ref], None) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i64, *const u8, usize, LoftRef) =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(ls, i64_arg!(0), p, l, ref_arg!(2));
        }
        // (ref, i64) -> i64  (e.g. gl_upload_vertices)
        (&[ArgT::Ref, ArgT::I64], Some(ArgT::I64)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i64) -> i64 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(ls, ref_arg!(0), i64_arg!(1)));
        }
        // (ref, i64, i64) -> i64  (e.g. gl_upload_canvas)
        (&[ArgT::Ref, ArgT::I64, ArgT::I64], Some(ArgT::I64)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i64, i64) -> i64 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(ls, ref_arg!(0), i64_arg!(1), i64_arg!(2)));
        }
        // (ref, i64, f64) -> i64  (e.g. audio_play_raw)
        (&[ArgT::Ref, ArgT::I64, ArgT::F64], Some(ArgT::I64)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef, i64, f64) -> i64 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(ls, ref_arg!(0), i64_arg!(1), f64_arg!(2)));
        }
        // (text, i64, i64, ref) -> bool  (e.g. save_png)
        (&[ArgT::Text, ArgT::I64, ArgT::I64, ArgT::Ref], Some(ArgT::Bool)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, *const u8, usize, i64, i64, LoftRef) -> bool =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            stores.put(stack, f(ls, p, l, i64_arg!(1), i64_arg!(2), ref_arg!(3)));
        }
        // (i64, text, f64, ref) -> i64  (e.g. rasterize_text_into)
        (&[ArgT::I64, ArgT::Text, ArgT::F64, ArgT::Ref], Some(ArgT::I64)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i64, *const u8, usize, f64, LoftRef) -> i64 =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put::<i64>(stack, f(ls, i64_arg!(0), p, l, f64_arg!(2), ref_arg!(3)));
        }
        // (i64) -> ref  (e.g. rand_indices)
        (&[ArgT::I64], Some(ArgT::Ref)) => {
            let result_db = stores.null();
            let ls = make_loft_store(stores, result_db.store_nr);
            let f: extern "C" fn(LoftStore, i64) -> LoftRef = unsafe { std::mem::transmute(fp) };
            push_loft_ref(stores, stack, f(ls, i64_arg!(0)));
        }
        // (ref) -> i64  (e.g. ext_vec_sum: vector<integer> -> integer)
        (&[ArgT::Ref], Some(ArgT::I64)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, LoftRef) -> i64 = unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(ls, ref_arg!(0)));
        }
        // (i64, ref) -> i64  (e.g. ext_offset_sum: offset, vector<integer>)
        (&[ArgT::I64, ArgT::Ref], Some(ArgT::I64)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i64, LoftRef) -> i64 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(ls, i64_arg!(0), ref_arg!(1)));
        }
        // (i64, ref, i64) -> i64  (e.g. ext_sandwich_sum: a, vector<integer>, b)
        (&[ArgT::I64, ArgT::Ref, ArgT::I64], Some(ArgT::I64)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i64, LoftRef, i64) -> i64 =
                unsafe { std::mem::transmute(fp) };
            stores.put::<i64>(stack, f(ls, i64_arg!(0), ref_arg!(1), i64_arg!(2)));
        }
        // ── @PLAN48 P3: I64 twins of the text-bearing I32 arms above.  After
        // @P370 a plain loft `integer` marshals as I64, so lib/web + lib/server
        // handle/status/index params that sit next to a `text` argument now
        // surface here instead of the I32 twins.  Same bodies, i64 ABI.
        // (i64, text) -> bool  (e.g. ws_client_send / ws_send: handle, msg)
        (&[ArgT::I64, ArgT::Text], Some(ArgT::Bool)) => {
            let f: extern "C" fn(i64, *const u8, usize) -> bool =
                unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put(stack, f(i64_arg!(0), p, l));
        }
        // (i64, text) -> i64  (e.g. byte_at: idx, text; ws_broadcast: handle, msg)
        (&[ArgT::I64, ArgT::Text], Some(ArgT::I64)) => {
            let f: extern "C" fn(i64, *const u8, usize) -> i64 = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put::<i64>(stack, f(i64_arg!(0), p, l));
        }
        // (i64, text) -> void  (e.g. tcp_respond: status, body — cdylib casts
        // the i64 status to u16 itself, so the loft `integer` decl stays clean)
        (&[ArgT::I64, ArgT::Text], None) => {
            let f: extern "C" fn(i64, *const u8, usize) = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            f(i64_arg!(0), p, l);
        }
        // (i64, text, text) -> void  (e.g. tcp_respond_typed: status, body,
        // content_type — TTT v3's HTML/CSS/JS/loft-mime server responses)
        (&[ArgT::I64, ArgT::Text, ArgT::Text], None) => {
            let f: extern "C" fn(i64, *const u8, usize, *const u8, usize) =
                unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(1);
            let (p1, l1) = text_arg!(2);
            f(i64_arg!(0), p0, l0, p1, l1);
        }
        // (text, text, text, text) -> i64  (e.g. http_do: method, url, body,
        // headers -> status; the loft `integer` return marshals as I64 now)
        (&[ArgT::Text, ArgT::Text, ArgT::Text, ArgT::Text], Some(ArgT::I64)) => {
            let f: extern "C" fn(
                *const u8,
                usize,
                *const u8,
                usize,
                *const u8,
                usize,
                *const u8,
                usize,
            ) -> i64 = unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(0);
            let (p1, l1) = text_arg!(1);
            let (p2, l2) = text_arg!(2);
            let (p3, l3) = text_arg!(3);
            stores.put::<i64>(stack, f(p0, l0, p1, l1, p2, l2, p3, l3));
        }
        _ => {
            let sig_str: Vec<String> = params.iter().map(|t| format!("{t:?}")).collect();
            panic!(
                "auto-marshal: unsupported signature ({}) -> {:?}",
                sig_str.join(", "),
                ret
            );
        }
    }
    // _guard's Drop clears CURRENT_STORES automatically, even on panic.
}

#[cfg(feature = "native-extensions")]
#[derive(Clone)]
#[allow(dead_code)]
enum ArgVal {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Bool(bool),
    Text(*const u8, usize),
    Ref(u16, u32, u32), // store_nr, rec, pos
}

// ── Auto-build ──────────────────────────────────────────────────────────

/// Auto-build a package's native crate if the shared library OR
/// rlib is missing.
///
/// Both artifacts are produced by the same `cargo build` invocation
/// (every `lib/*/native/Cargo.toml` declares
/// `crate-type = ["cdylib", "rlib"]`).  The cdylib is loaded by the
/// runtime extension loader; the rlib is consumed by `--native`
/// codegen via `--extern <crate>=<path>` (see
/// `src/main.rs::add_native_extern_flags`).
///
/// Pre-2026-05-12 this function checked ONLY the cdylib before
/// returning early, which let a parallel test see "cdylib exists"
/// → skip the build → then fail later when `add_native_extern_flags`
/// looked for the rlib that hadn't been written yet.  On Windows
/// (where file-system flush ordering between cdylib and rlib differs
/// from ext4/APFS, and file-locks held by another concurrent cargo
/// invocation block reads even when the file is on disk) this race
/// surfaced as `error[E0463]: can't find crate for <name>` in
/// `tests/codegen_emitter.rs::p244_text_native_wrapper_compiles_under_native`
/// and any other test that ran in parallel with a fresh build.
///
/// Fix: also require the rlib to exist before returning early.  If
/// either artifact is missing, run cargo (which is itself
/// fcntl-locked per target dir, so concurrent invocations serialize
/// and only one actually rebuilds).
/// @PLAN12 Phase 6b — compute the target dir for a package's
/// native cdylib/rlib build.
///
/// When `pkg_dir` is under `~/.loft/registry/` (i.e., a
/// `loft install`-extracted chunk), returns the redirected
/// `~/.loft/build-cache/<pkg>-<ver>/` path; otherwise returns
/// `<pkg_dir>/native/target` (the in-tree default).
///
/// Used by [`auto_build_native`] (to set `CARGO_TARGET_DIR` and
/// to read the freshly-built artifact) AND by
/// `native_utils::add_native_extern_flags` (to find the rlib at
/// link time).  Single source of truth so the cdylib/rlib are
/// always at the same root.
#[must_use]
pub fn native_target_root(pkg_dir: &std::path::Path) -> std::path::PathBuf {
    // Without the registry feature there is no `~/.loft/registry/` to
    // redirect away from — every install is in-tree, so the package
    // dir's own `native/target/` is the target root.
    #[cfg(not(feature = "registry"))]
    {
        pkg_dir.join("native").join("target")
    }
    #[cfg(feature = "registry")]
    {
        let registry_cache = crate::registry_index::cache_dir();
        let registry_cache_canon = std::fs::canonicalize(&registry_cache).ok();
        let pkg_canon = std::fs::canonicalize(pkg_dir).ok();
        let use_redirected = match (&registry_cache_canon, &pkg_canon) {
            (Some(rc), Some(pc)) => pc.starts_with(rc),
            _ => false,
        };
        if use_redirected {
            let stem_dir = pkg_dir.file_name().map_or_else(
                || "native-pkg".to_string(),
                |s| s.to_string_lossy().into_owned(),
            );
            registry_cache
                .parent()
                .map_or_else(
                    || std::path::PathBuf::from("."),
                    std::path::Path::to_path_buf,
                )
                .join("build-cache")
                .join(stem_dir)
        } else {
            pkg_dir.join("native").join("target")
        }
    }
}

pub fn auto_build_native(pkg_dir: &str, stem: &str) -> Option<String> {
    use std::path::PathBuf;
    // P244-windows fix #2 (2026-05-12): use PathBuf::join, not
    // `format!("{pkg_dir}/...")`.  When `pkg_dir` arrives as a
    // canonicalized Windows extended-length path (e.g.
    // `\\?\D:\a\loft\loft\lib\server`), concatenating with `/native/...`
    // produces a malformed path: the `\\?\` verbatim prefix bypasses
    // Rust's path normalization, so the mixed-separator suffix
    // doesn't resolve and `Path::exists()` returns false even when
    // the file is on disk.  PathBuf::join handles each component
    // through proper Path semantics on every platform.
    let pkg = PathBuf::from(pkg_dir);
    let cargo_toml = pkg.join("native").join("Cargo.toml");
    if !cargo_toml.exists() {
        return None;
    }
    let lib_name = platform_lib_name(stem);
    let rlib_name = format!("lib{stem}.rlib");

    // @PLAN12 Phase 6b — target root via the shared helper (redirects
    // chunk-resident installs to ~/.loft/build-cache/<pkg>-<ver>/;
    // keeps in-tree target/ for the monorepo's lib/<pkg>/native/).
    let target_root = native_target_root(&pkg);
    let in_tree_target = pkg.join("native").join("target");
    let use_redirected_target = target_root != in_tree_target;

    // Existing-cache check: look in the redirected target first
    // (where future builds land), then the legacy in-tree target/
    // (so existing builds from older loft binaries are still
    // reused).
    let mut search_roots: Vec<PathBuf> = Vec::new();
    search_roots.push(target_root.clone());
    if use_redirected_target {
        search_roots.push(in_tree_target.clone());
    }
    // @PLN11 Arc N / N0 — reuse a cached artifact only if it was built by THIS
    // loft build (its fingerprint sidecar matches).  A loft rebuild flips the
    // fingerprint, so a stale package rlib / cdylib is rebuilt instead of being
    // silently linked against a changed loft ABI — the automatic replacement for
    // `make rebuild-native-cdylibs`.
    let fp = crate::cache::loft_build_fingerprint();
    let find_existing = || {
        for root in &search_roots {
            for profile in ["release", "debug"] {
                let dir = root.join(profile);
                let lib = dir.join(&lib_name);
                let rlib = dir.join(&rlib_name);
                if lib.exists()
                    && rlib.exists()
                    && crate::cache::native_artifact_fingerprint_matches(&dir, fp)
                {
                    return Some(lib.to_string_lossy().to_string());
                }
            }
        }
        None
    };
    if let Some(p) = find_existing() {
        return Some(p);
    }

    // @P388: serialise on-demand native builds ACROSS PROCESSES.  Parallel test
    // binaries (nextest runs one process per test) and parallel end-user `loft`
    // invocations otherwise each run an unlocked `cargo build` that re-resolves
    // the dependency tree concurrently, racing cargo's shared registry index +
    // package cache → transient "required to be available in rlib format" errors
    // and non-deterministic version picks.  A single cross-process advisory file
    // lock serialises the builds.  (The CI pre-build already serialises CI; this
    // covers the latent end-user-parallelism half of @P388.)  The `--locked`
    // route is unavailable: the repo deliberately gitignores every `Cargo.lock`,
    // and these crates.io-sourced native deps would drift against the actively-
    // versioned loft-ffi crates.  Best-effort: if the lock can't be opened/taken
    // we proceed unserialised — no worse than before.  `File::lock` blocks until
    // acquired and releases when `_build_lock` drops at function exit (so a crash
    // mid-build can't strand the lock).
    let _build_lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(std::env::temp_dir().join("loft-native-build.lock"))
        .ok();
    if let Some(f) = &_build_lock {
        let _ = f.lock();
    }
    // Re-check under the lock: a process we waited on may have just produced the
    // artifact, in which case we must NOT rebuild.
    if let Some(p) = find_existing() {
        return Some(p);
    }

    // Build.  When redirecting, pass `CARGO_TARGET_DIR` so cargo
    // writes outside the install dir.
    let mut cmd = std::process::Command::new("cargo");
    cmd.args(["build", "--release", "--manifest-path"])
        .arg(&cargo_toml)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    // #274 — build the package crate with the SAME RUSTFLAGS loft's own rlibs
    // used (captured at loft build time), so a shared transitive dep like
    // `libloading` gets a matching SVH.  Otherwise loft's `-g` copy and the
    // package's plain copy share a `StableCrateId` but differ in SVH and rustc
    // aborts at the generated program's link step.  Force it (overriding any
    // ambient value) and clear `CARGO_ENCODED_RUSTFLAGS` so cargo doesn't see
    // both forms at once.
    cmd.env("RUSTFLAGS", env!("LOFT_BUILD_RUSTFLAGS"))
        .env_remove("CARGO_ENCODED_RUSTFLAGS");
    if use_redirected_target {
        if let Some(parent) = target_root.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        cmd.env("CARGO_TARGET_DIR", &target_root);
    }
    let built_path = target_root.join("release").join(&lib_name);
    let status = cmd.status();
    match status {
        Ok(s) if s.success() => {
            // @PLN11 Arc N / N0 — stamp the build fingerprint on ANY successful
            // cargo build: the rlib is produced even for rlib-only packages whose
            // cdylib `built_path` is absent, so a later loft change still
            // invalidates it (see `find_existing` / `add_native_extern_flags`).
            crate::cache::write_native_artifact_fingerprint(&target_root.join("release"), fp);
            built_path
                .exists()
                .then(|| built_path.to_string_lossy().to_string())
        }
        _ => None,
    }
}

/// Resolve the platform-correct shared-library filename from a stem.
#[must_use]
pub fn platform_lib_name(stem: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(windows) {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
    }
}

/// Public API for generated native code that needs to call a cdylib
/// function with a `LoftStore` handle.
///
/// @PLAN12 phase 3.5a (2026-05-24) — added so the `--native` codegen
/// can drive store-allocating cdylib calls (e.g. `n_rand_indices`,
/// `n_load_png`).  The interpreter's `dispatch_call` does the same
/// thing inline (`make_loft_store` + `CURRENT_STORES` setup at
/// `src/extensions.rs:570` and `:632`); this module exposes the same
/// machinery as a `pub` API constructing `loft_ffi::LoftStore`
/// directly (the interpreter still uses the local `#[repr(C)]`
/// mirrors at lines 155-192 for its internal dispatch).
///
/// Usage from generated code:
/// ```ignore
/// let _guard = loft::native_call::enter(stores);
/// let ls = loft::native_call::build_store(stores, store_nr);
/// let r = unsafe { external_crate::n_some_fn(ls, arg1, arg2) };
/// // _guard's Drop clears CURRENT_STORES.
/// ```
#[cfg(feature = "native-extensions")]
#[allow(dead_code)] // Consumed by generated native code (separate compilation unit),
// not by the loft binary itself.  Clippy can't see those callers.
pub mod native_call {
    use crate::database::Stores;

    /// C-ABI callback equivalent to `super::ffi_claim` but typed
    /// against `loft_ffi::LoftStoreCtx` so the resulting
    /// `loft_ffi::LoftStore` can be passed to a generated-code
    /// callsite without `transmute`.
    unsafe extern "C" fn ffi_claim_pub(ctx: loft_ffi::LoftStoreCtx, words: u32) -> u32 {
        let store_nr = ctx._opaque as usize as u16;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::CURRENT_STORES.with(|c| {
                let stores = unsafe { &mut *c.get() };
                let store = &mut stores.allocations[store_nr as usize];
                store.claim(words)
            })
        }))
        .unwrap_or(0)
    }

    unsafe extern "C" fn ffi_resize_pub(ctx: loft_ffi::LoftStoreCtx, rec: u32, words: u32) -> u32 {
        let store_nr = ctx._opaque as usize as u16;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::CURRENT_STORES.with(|c| {
                let stores = unsafe { &mut *c.get() };
                let store = &mut stores.allocations[store_nr as usize];
                store.resize(rec, words)
            })
        }))
        .unwrap_or(rec)
    }

    unsafe extern "C" fn ffi_reload_pub(
        ctx: loft_ffi::LoftStoreCtx,
        out_ptr: *mut *mut u8,
        out_size: *mut u32,
    ) {
        let store_nr = ctx._opaque as usize as u16;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::CURRENT_STORES.with(|c| {
                let stores = unsafe { &*c.get() };
                let store = &stores.allocations[store_nr as usize];
                unsafe {
                    *out_ptr = store.base_ptr();
                    *out_size = store.capacity_words();
                }
            });
        }));
    }

    /// Construct a `loft_ffi::LoftStore` handle pointing at the
    /// store identified by `store_nr`.  Callbacks read from the
    /// `CURRENT_STORES` thread-local — call `enter()` before this
    /// to set it up.
    #[must_use]
    pub fn build_store(stores: &Stores, store_nr: u16) -> loft_ffi::LoftStore {
        let store = stores.store(&crate::keys::DbRef {
            store_nr,
            rec: 0,
            pos: 0,
        });
        loft_ffi::LoftStore {
            ptr: store.base_ptr(),
            size: store.capacity_words(),
            ctx: loft_ffi::LoftStoreCtx {
                _opaque: store_nr as usize as *mut (),
            },
            claim_fn: Some(ffi_claim_pub),
            reload_fn: Some(ffi_reload_pub),
            resize_fn: Some(ffi_resize_pub),
        }
    }

    /// RAII guard: sets `CURRENT_STORES` on entry, clears on drop.
    /// One per cdylib call; nested calls are NOT supported (would
    /// stomp the thread-local).  Native codegen emits one per
    /// call-site, scoped via `let _guard = native_call::enter(stores);`.
    pub struct StoresGuard {
        _priv: (),
    }

    /// Set the thread-local current stores pointer for the
    /// duration of the returned guard.  Generated code:
    /// `let _guard = native_call::enter(stores);` immediately
    /// before a cdylib call.
    pub fn enter(stores: &mut Stores) -> StoresGuard {
        let ptr = std::ptr::from_mut::<Stores>(stores);
        super::CURRENT_STORES.with(|c| c.set(ptr));
        StoresGuard { _priv: () }
    }

    impl Drop for StoresGuard {
        fn drop(&mut self) {
            super::CURRENT_STORES.with(|c| c.set(std::ptr::null_mut()));
        }
    }
}
