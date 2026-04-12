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

/// Loaded libraries kept alive for the process lifetime.
/// Used by `try_dlsym` to look up symbols from previously loaded cdylibs.
#[cfg(feature = "native-extensions")]
static LOADED_LIBS: Mutex<Vec<libloading::Library>> = Mutex::new(Vec::new());

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

    // Try the registration protocol first.
    if let Ok(register_sym) = unsafe { lib.get::<RegisterFn>(b"loft_register_v1\0") } {
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
    // Either way, keep the library loaded for potential dlsym lookups.
    loaded.insert(canonical);
    LOADED_LIBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(lib);
}

/// Try to resolve a symbol by name from any loaded cdylib.
/// Called by `wire_native_fns` as a fallback when the symbol wasn't
/// provided via `loft_register_v1`. This enables zero-registration cdylibs:
/// just export `#[unsafe(no_mangle)] pub extern "C" fn n_my_func(...)`.
#[cfg(feature = "native-extensions")]
fn try_dlsym(name: &str) -> Option<*const ()> {
    let libs = LOADED_LIBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut sym_name = name.to_string();
    sym_name.push('\0');
    for lib in libs.iter() {
        if let Ok(sym) = unsafe { lib.get::<*const ()>(sym_name.as_bytes()) } {
            return Some(*sym);
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
            Type::Integer(_, _, _) | Type::Character => ArgT::I32,
            Type::Long => ArgT::I64,
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
        Type::Integer(_, _, _) | Type::Character => Some(ArgT::I32),
        Type::Long => Some(ArgT::I64),
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
/// (indicating a registration bug — issue #119).
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
            if let Some(stubs) = stub_syms
                && !stubs.contains(sym) {
                    continue;
                }
            let found = reg_guard.as_ref().is_some_and(|r| r.contains_key(sym));
            if !found {
                to_resolve.push(sym.clone());
            }
        }
        drop(reg_guard);
        drop(stub_guard);

        // Check if any library used loft_register_v1 (i.e. registry is non-empty).
        // If so, dlsym fallback is a registration bug — the library chose
        // the registration protocol, so all its symbols should be registered.
        let has_v1 = {
            let rg = NATIVE_REGISTRY
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            rg.as_ref().is_some_and(|r| !r.is_empty())
        };

        // Resolve via dlsym (no locks held).
        for sym in to_resolve {
            if let Some(ptr) = try_dlsym(&sym) {
                // The library used loft_register_v1 but didn't register
                // this symbol — this is a registration bug (issue #119).
                assert!(
                    !has_v1,
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

        // Only replace stubs — skip hand-written glue from native::init().
        if let Some(stubs) = stub_syms
            && !stubs.contains(sym) {
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

    let guard = NATIVE_SIGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sig_table = guard.as_ref().expect("NATIVE_SIGS not initialized");
    let (sym, sig) = match sig_table.get(&lib_idx) {
        Some(entry) => entry,
        None => panic!("no signature for lib_idx {lib_idx}"),
    };

    let fp = unsafe { get_native_fn_raw(sym) }.unwrap_or_else(|| {
        panic!("native symbol '{sym}' not loaded");
    });

    let mut args: Vec<ArgVal> = Vec::with_capacity(sig.params.len());
    // Pop in reverse order (stack is LIFO).
    for &t in sig.params.iter().rev() {
        let val = match t {
            ArgT::I32 => ArgVal::I32(*stores.get::<i32>(stack)),
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
                    let vec_rec = stores.store(&r).get_int(r.rec, r.pos) as u32;
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
    use crate::keys::Str;

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
        if !s.ptr.is_null() && s.len > 0 {
            let text =
                unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(s.ptr, s.len)) };
            stores.scratch.clear();
            stores.scratch.push(text.to_string());
            stores.put(stack, Str::new(&stores.scratch[0]));
        } else {
            stores.put(stack, Str::new(""));
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
        stores.store_mut(&base).set_int(header.rec, 4, r.rec as i32);
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
            stores.put(stack, f());
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
            stores.put(stack, f(i32_arg!(0)));
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
            stores.put(stack, f(i32_arg!(0), i32_arg!(1)));
        }
        // (i32, f64) -> i32
        (&[ArgT::I32, ArgT::F64], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32, f64) -> i32 = unsafe { std::mem::transmute(fp) };
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
        // (text) -> i32
        (&[ArgT::Text], Some(ArgT::I32)) => {
            let f: extern "C" fn(*const u8, usize) -> i32 = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(0);
            stores.put(stack, f(p, l));
        }
        // (text, text) -> i32
        (&[ArgT::Text, ArgT::Text], Some(ArgT::I32)) => {
            let f: extern "C" fn(*const u8, usize, *const u8, usize) -> i32 =
                unsafe { std::mem::transmute(fp) };
            let (p0, l0) = text_arg!(0);
            let (p1, l1) = text_arg!(1);
            stores.put(stack, f(p0, l0, p1, l1));
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
            stores.put(stack, f(f64_arg!(0)));
        }
        // (i32, i32, i32) -> i32
        (&[ArgT::I32, ArgT::I32, ArgT::I32], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32, i32, i32) -> i32 = unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(i32_arg!(0), i32_arg!(1), i32_arg!(2)));
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
            stores.put(stack, f(p0, l0, p1, l1, p2, l2, p3, l3));
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
            stores.put(stack, f(i32_arg!(0), i32_arg!(1), i32_arg!(2), i32_arg!(3)));
        }
        // (i32, text) -> i32
        (&[ArgT::I32, ArgT::Text], Some(ArgT::I32)) => {
            let f: extern "C" fn(i32, *const u8, usize) -> i32 = unsafe { std::mem::transmute(fp) };
            let (p, l) = text_arg!(1);
            stores.put(stack, f(i32_arg!(0), p, l));
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
            stores.put(stack, f(ls, ref_arg!(0)));
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
            stores.put(stack, f(ls, p, l, ref_arg!(1)));
        }
        // (I32, Ref) -> I32
        (&[ArgT::I32, ArgT::Ref], Some(ArgT::I32)) => {
            let ls = make_loft_store(stores, first_ref_store(args));
            let f: extern "C" fn(LoftStore, i32, LoftRef) -> i32 =
                unsafe { std::mem::transmute(fp) };
            stores.put(stack, f(ls, i32_arg!(0), ref_arg!(1)));
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
            stores.put(stack, f(ls, ref_arg!(0), i32_arg!(1)));
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
            stores.put(stack, f(ls, i32_arg!(0), ref_arg!(1), i32_arg!(2)));
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
            stores.put(stack, f(ls, ref_arg!(0), i32_arg!(1), i32_arg!(2)));
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
            stores.put(stack, f(ls, i32_arg!(0), p, l, f64_arg!(2), ref_arg!(3)));
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
            stores.put(stack, f(ls, ref_arg!(0), i32_arg!(1), f64_arg!(2)));
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

/// Auto-build a package's native crate if the shared library is missing.
#[must_use]
pub fn auto_build_native(pkg_dir: &str, stem: &str) -> Option<String> {
    let cargo_toml = format!("{pkg_dir}/native/Cargo.toml");
    if !std::path::Path::new(&cargo_toml).exists() {
        return None;
    }
    let lib_name = platform_lib_name(stem);
    let release_path = format!("{pkg_dir}/native/target/release/{lib_name}");
    if std::path::Path::new(&release_path).exists() {
        return Some(release_path);
    }
    let debug_path = format!("{pkg_dir}/native/target/debug/{lib_name}");
    if std::path::Path::new(&debug_path).exists() {
        return Some(debug_path);
    }
    let built_path = release_path;
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "--manifest-path", &cargo_toml])
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status();
    match status {
        Ok(s) if s.success() && std::path::Path::new(&built_path).exists() => Some(built_path),
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
