// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 08-S2 — live function dispatch: a compiled binary that can push
//! individual functions to the interpreter at runtime.
//!
//! The heart-of-the-engine tier flip.  Under `LOFT_LIVE_FLIP=1` a `--native`
//! binary bootstraps a parked interpreter [`State`] from the SAME sources the
//! binary was generated from (the loft driver hands the paths down via
//! `LOFT_LIVE_SRC` / `LOFT_LIVE_STDLIB` / `LOFT_LIVE_LIBS`), and the program
//! world — the `Stores` every generated fn runs over — is the BOOTSTRAP's
//! world, taken out of the parked State.  Every generated user fn opens with
//! a one-atomic-load entry check; flipping a fn routes its calls into the
//! interpreter over those same stores.
//!
//! **The sharing model is a swap, not an alias**: the parked State keeps a
//! placeholder `Stores`; each dispatched call swaps the program world into
//! `state.database`, runs [`State::reenter`]/[`State::reenter_ret`] (the 02
//! frame contract), and swaps it back out.  Single-threaded by construction —
//! the check fires inside the callee on the thread that bootstrapped; worker
//! threads fall through to the compiled body.
//!
//! Failure posture: every bootstrap problem WARNS and falls back to fully
//! compiled execution — live mode is an instrument, never a halt.

use std::cell::{RefCell, UnsafeCell};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::database::Stores;
use crate::keys::DbRef;
use crate::state::State;

/// Set once at a successful bootstrap; the first word of every entry check.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// `LOFT_DISPATCH_DEBUG=1` — one stderr line per dispatched call (the S2
/// observable: a flip must be visible ONLY here and in timing).
static DEBUG: AtomicBool = AtomicBool::new(false);
/// Per generated-fn-index flip switches (index = position in the emitted
/// `LOFT_LIVE_FNS` table).  Settable only for resolved fns.
static FLIPS: OnceLock<Box<[AtomicBool]>> = OnceLock::new();
/// Which indices resolved to a bootstrap (d_nr, code_position) — a flip on an
/// unresolved fn would dispatch into nothing, so `set_flip` refuses it.
static RESOLVED: OnceLock<Box<[bool]>> = OnceLock::new();
/// Total dispatched calls (all fns) — the S2 sentinel's counter.
static DISPATCHED: AtomicU64 = AtomicU64::new(0);

/// The parked interpreter.  The `Parser` (owning `Data`) and the `State` are
/// boxed BEFORE the raw `data_ptr`/`parallel_ctx` pointers are wired, so the
/// addresses those pointers capture stay stable for the program's lifetime.
struct Live {
    /// Never read after bootstrap, but load-bearing: it OWNS the `Data` the
    /// parked State's `data_ptr`/`parallel_ctx` raw pointers reference.
    _parser: Box<crate::parser::Parser>,
    state: Box<State>,
    /// Per generated fn index: (d_nr, code_position) in the bootstrap world.
    fns: Vec<(u32, u32)>,
    names: &'static [&'static str],
}

thread_local! {
    static LIVE: RefCell<Option<Live>> = const { RefCell::new(None) };
}

/// The entry check every generated user fn opens with.  Cold path cost when
/// live mode is off: one relaxed atomic load.
#[inline]
pub fn live_flipped(idx: usize) -> bool {
    if !ENABLED.load(Ordering::Relaxed) {
        return false;
    }
    if !FLIPS
        .get()
        .is_some_and(|f| f.get(idx).is_some_and(|b| b.load(Ordering::Relaxed)))
    {
        return false;
    }
    // Worker threads have no parked State — they run the compiled body.
    LIVE.with(|l| l.borrow().is_some())
}

/// True after a successful bootstrap — generated `main` uses this to skip
/// `init(&cell)` (the bootstrap world is already fully seeded by the parse).
#[inline]
#[must_use]
pub fn live_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Generated `main` calls this instead of `Stores::new()`.  Under
/// `LOFT_LIVE_FLIP=1` the returned stores are the bootstrap world (the parse
/// seeded types, CONST_STORE, const vectors — `init` must be skipped);
/// otherwise a plain `Stores::new()` (caller runs `init` as always).
#[must_use]
pub fn boot_stores(fn_names: &'static [&'static str]) -> Stores {
    if std::env::var("LOFT_LIVE_FLIP").is_ok_and(|v| v == "1") {
        match bootstrap(fn_names) {
            Ok(stores) => return stores,
            Err(msg) => eprintln!("loft-live: disabled — {msg}"),
        }
    }
    Stores::new()
}

/// Parse + byte-code the program this binary was generated from, park the
/// State, and hand its fully-seeded world out to the compiled code.
fn bootstrap(fn_names: &'static [&'static str]) -> Result<Stores, String> {
    let src = std::env::var("LOFT_LIVE_SRC")
        .map_err(|_| "LOFT_LIVE_SRC not set (run through the loft driver)".to_string())?;
    let stdlib = std::env::var("LOFT_LIVE_STDLIB").unwrap_or_else(|_| "default".to_string());
    let mut p = Box::new(crate::parser::Parser::new());
    p.parse_dir(&stdlib, true, false)
        .map_err(|e| format!("stdlib `{stdlib}`: {e}"))?;
    if let Ok(libs) = std::env::var("LOFT_LIVE_LIBS") {
        p.lib_dirs = libs
            .split(':')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    p.parse(&src, false);
    if p.diagnostics.level() >= crate::diagnostics::Level::Error {
        return Err(format!("`{src}` does not parse clean"));
    }
    crate::scopes::check(&mut p.data);
    let mut state = Box::new(State::new(p.database.clone()));
    crate::compile::byte_code(&mut state, &mut p.data);

    // The prologue `execute_argv` would have done, minus running `main` —
    // `reenter` needs fn_positions (Call ops), and natives that spawn interp
    // workers need data_ptr / parallel_ctx.  Wired AFTER boxing: the raw
    // pointers capture the boxes' stable addresses.
    state.fn_positions = p.data.definitions.iter().map(|d| d.code_position).collect();
    let data_ptr = std::ptr::from_ref(&p.data);
    state.data_ptr = data_ptr;
    let stk_lib_nr = state
        .library_names
        .get("n_stack_trace")
        .copied()
        .unwrap_or(u16::MAX);
    state.database.parallel_ctx = Some(Box::new(crate::database::ParallelCtx {
        bytecode: &raw const state.bytecode,
        library: &raw const state.library,
        data: data_ptr,
        stack_trace_lib_nr: stk_lib_nr,
    }));
    crate::crash_report::set_source_spans(Some(std::sync::Arc::new(state.source_spans.clone())));

    // Resolve the generated fn table against the bootstrap world.
    let mut fns = Vec::with_capacity(fn_names.len());
    let mut resolved = Vec::with_capacity(fn_names.len());
    for name in fn_names {
        let d_nr = p.data.def_nr(name);
        if d_nr == u32::MAX {
            fns.push((u32::MAX, 0));
            resolved.push(false);
        } else {
            fns.push((d_nr, p.data.def(d_nr).code_position));
            resolved.push(true);
        }
    }
    let _ = RESOLVED.set(resolved.into_boxed_slice());
    let _ = FLIPS.set(
        (0..fn_names.len())
            .map(|_| AtomicBool::new(false))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    DEBUG.store(
        std::env::var("LOFT_DISPATCH_DEBUG").is_ok_and(|v| v == "1"),
        Ordering::Relaxed,
    );

    // The handover: the program runs over the BOOTSTRAP's world.  The parked
    // State keeps a placeholder; every dispatch swaps the world back in.
    let stores = std::mem::take(&mut state.database);
    LIVE.with(|l| {
        *l.borrow_mut() = Some(Live {
            _parser: p,
            state,
            fns,
            names: fn_names,
        });
    });
    ENABLED.store(true, Ordering::Relaxed);

    // Startup flips: LOFT_FLIP_FNS="tick,reply" (loft names, no n_ prefix).
    if let Ok(list) = std::env::var("LOFT_FLIP_FNS") {
        for name in list.split(',').filter(|s| !s.is_empty()) {
            if !set_flip(name, true) {
                eprintln!("loft-live: LOFT_FLIP_FNS names unknown fn `{name}` — ignored");
            }
        }
    }
    Ok(stores)
}

/// Flip one fn (loft name, without the `n_` prefix) to the interpreter (`on`)
/// or back to compiled (`!on`).  Returns false when the name is unknown or
/// unresolved — a refused flip, never a halt.
pub fn set_flip(name: &str, on: bool) -> bool {
    let (Some(flips), Some(resolved)) = (FLIPS.get(), RESOLVED.get()) else {
        return false;
    };
    let full = format!("n_{name}");
    let found = LIVE.with(|l| {
        l.borrow().as_ref().and_then(|live| {
            live.names
                .iter()
                .position(|n| **n == full)
                .filter(|&i| resolved[i])
        })
    });
    let Some(idx) = found else { return false };
    flips[idx].store(on, Ordering::Relaxed);
    if DEBUG.load(Ordering::Relaxed) {
        eprintln!(
            "live-flip: {full} -> {}",
            if on { "interp" } else { "compiled" }
        );
    }
    true
}

/// Total dispatched calls so far (the S2 sentinel's counter).
#[must_use]
pub fn dispatch_count() -> u64 {
    DISPATCHED.load(Ordering::Relaxed)
}

/// The dispatch chokepoint: swap the program world into the parked State,
/// re-enter the interpreter, swap it back out.  Every `live_call_*` routes
/// through here — the sentinel and the sharing model live in ONE place.
fn dispatch<R>(
    cell: &UnsafeCell<Stores>,
    idx: usize,
    run: impl FnOnce(&mut State, u32, u32) -> R,
) -> R {
    LIVE.with(|l| {
        let mut borrow = l.borrow_mut();
        let live = borrow
            .as_mut()
            .expect("live_flipped() gates dispatch on a parked State");
        let (d_nr, pos) = live.fns[idx];
        let n = DISPATCHED.fetch_add(1, Ordering::Relaxed) + 1;
        if DEBUG.load(Ordering::Relaxed) {
            eprintln!("live-dispatch: {} #{n}", live.names[idx]);
        }
        // The swap: generated callers up the stack hold `&mut Stores` derived
        // from this same UnsafeCell — the cell's ADDRESS stays put, only the
        // value moves, and no native frame touches it until we return.
        let in_cell = unsafe { &mut *cell.get() };
        std::mem::swap(&mut live.state.database, in_cell);
        let r = run(&mut live.state, d_nr, pos);
        std::mem::swap(&mut live.state.database, in_cell);
        r
    })
}

/// Dispatch a `Void` fn.  `push` lands the args via [`State::put_stack`] in
/// declared order — the 02 probe's contract (and the generated signature's).
pub fn live_call_void(cell: &UnsafeCell<Stores>, idx: usize, push: impl FnOnce(&mut State)) {
    dispatch(cell, idx, |st, d_nr, pos| st.reenter(d_nr, pos, push));
}

/// Dispatch an `integer`-returning fn.
pub fn live_call_i64(cell: &UnsafeCell<Stores>, idx: usize, push: impl FnOnce(&mut State)) -> i64 {
    dispatch(cell, idx, |st, d_nr, pos| {
        st.reenter_ret::<i64>(d_nr, pos, push)
    })
}

/// Dispatch a `float`-returning fn.
pub fn live_call_f64(cell: &UnsafeCell<Stores>, idx: usize, push: impl FnOnce(&mut State)) -> f64 {
    dispatch(cell, idx, |st, d_nr, pos| {
        st.reenter_ret::<f64>(d_nr, pos, push)
    })
}

/// Dispatch a `boolean`-returning fn (storage byte: 0/1/255).
pub fn live_call_u8(cell: &UnsafeCell<Stores>, idx: usize, push: impl FnOnce(&mut State)) -> u8 {
    dispatch(cell, idx, |st, d_nr, pos| {
        st.reenter_ret::<u8>(d_nr, pos, push)
    })
}

/// Dispatch a record/vector/hash-returning fn (a `DbRef` into the shared world).
pub fn live_call_ref(
    cell: &UnsafeCell<Stores>,
    idx: usize,
    push: impl FnOnce(&mut State),
) -> DbRef {
    dispatch(cell, idx, |st, d_nr, pos| {
        st.reenter_ret::<DbRef>(d_nr, pos, push)
    })
}

// ── loft-visible surface (lib/engine_host: `live_flip(name, on)`) ──────────

/// Typed twin for generated code: `live_flip(name: text, on: boolean) -> boolean`.
pub fn n_live_flip(_cell: &UnsafeCell<Stores>, name: &str, on: u8) -> u8 {
    u8::from(set_flip(name, on == 1))
}

/// Interp stack native: under the interpreter everything is already
/// interpreted, so a flip is a no-op `false` — the differential stays clean.
pub fn n_live_flip_stack(stores: &mut Stores, stack: &mut DbRef) {
    let _on = *stores.get::<u8>(stack);
    let _name = stores.get::<crate::keys::Str>(stack).str().to_owned();
    stores.put(stack, false);
}
