// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the `#c` direct C-ABI caller (@PLN24 arc B).

//! @PLN24 arc B — the interpreter's caller for a `#c` binding.
//!
//! The `--native` backend (arc C) hands the declared signature to rustc, which
//! emits a typed `extern "C"` and gets the ABI right by construction. The
//! interpreter has no compiler at the call site, so it does what the plan's
//! architecture probe validated: resolve the symbol, then call it through a
//! **fixed set of per-arity trampolines** — `extern "C" fn(u64, …) -> u64` —
//! with every integer-class argument collapsed into one `u64` slot.
//!
//! The probe (`tests/fixtures/c_abi/`) is what makes that sound, and it is also
//! what bounds it:
//!
//! - **Arguments cross correctly**, at every arity, including across the
//!   register/stack boundary. A `u64` slot carries an `int`, a `long`, a
//!   pointer or a `char *` intact, because the callee reads the low bits it
//!   wants.
//! - **Returns do not.** A 32-bit C return leaves the upper half of the
//!   register unspecified; on x86-64 it zero-extends, so `-1` read back as a
//!   `u64` is 4294967295 — a plausible large positive, not a crash. Every
//!   return here is therefore truncated to its **declared** width and
//!   re-extended by [`narrow_return`]. That is the one place this file differs
//!   from a naive transmute-and-call, and it is the whole reason the
//!   declaration carries a signature.
//!
//! The trampolines cover arity 0..=12. Beyond that the binding is refused
//! rather than truncated — a wrong arity is silent (the probe called an arity-1
//! symbol through an arity-3 trampoline and got the right answer), so nothing
//! downstream would catch it.

#![cfg(feature = "native-extensions")]

use crate::c_signature::{CSignature, CType};

/// Call `f` with `args`, at the arity `args.len()` names. `None` when the arity
/// is past the ladder.
///
/// One transmute target per rung, written out rather than generated: every
/// integer-class C type — `int`, `long`, a pointer, `char *`, an enum —
/// occupies one `u64`, so arity is the only dimension. Thirteen function types,
/// no combinatorial explosion, and no libffi.
///
/// # Safety
/// `f` must be a C function whose parameters are all integer-class and whose
/// arity equals `args.len()`. Nothing checks this at runtime — the probe called
/// an arity-1 symbol through an arity-3 trampoline and got the RIGHT answer, so
/// there is no runtime signal to check against. The declaration is the only
/// authority, which is why it is checked when it is parsed.
unsafe fn call_at_arity(f: *const (), args: &[u64]) -> Option<u64> {
    macro_rules! at {
        ($ty:ty, $($i:literal),*) => {{
            let g: $ty = unsafe { std::mem::transmute(f) };
            g($(args[$i]),*)
        }};
    }
    Some(match args.len() {
        0 => {
            let g: extern "C" fn() -> u64 = unsafe { std::mem::transmute(f) };
            g()
        }
        1 => at!(extern "C" fn(u64) -> u64, 0),
        2 => at!(extern "C" fn(u64, u64) -> u64, 0, 1),
        3 => at!(extern "C" fn(u64, u64, u64) -> u64, 0, 1, 2),
        4 => at!(extern "C" fn(u64, u64, u64, u64) -> u64, 0, 1, 2, 3),
        5 => at!(extern "C" fn(u64, u64, u64, u64, u64) -> u64, 0, 1, 2, 3, 4),
        6 => at!(
            extern "C" fn(u64, u64, u64, u64, u64, u64) -> u64,
            0,
            1,
            2,
            3,
            4,
            5
        ),
        // 6 and 7 straddle the SysV boundary: the first six integer arguments
        // travel in registers and the seventh onward on the stack. The probe
        // pins both rungs for exactly that reason.
        7 => at!(
            extern "C" fn(u64, u64, u64, u64, u64, u64, u64) -> u64,
            0,
            1,
            2,
            3,
            4,
            5,
            6
        ),
        8 => at!(
            extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64,
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7
        ),
        9 => at!(
            extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64, u64) -> u64,
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8
        ),
        10 => at!(
            extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) -> u64,
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9
        ),
        11 => at!(
            extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) -> u64,
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9,
            10
        ),
        12 => at!(
            extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) -> u64,
            0,
            1,
            2,
            3,
            4,
            5,
            6,
            7,
            8,
            9,
            10,
            11
        ),
        _ => return None,
    })
}

/// Bring a raw return register back to a loft `integer`.
///
/// The register holds whatever the callee left there, and for a return narrower
/// than 64 bits the upper half is **not defined by the ABI** — x86-64 happens to
/// zero-extend, which turns a negative `int` into a large positive rather than
/// into anything that looks wrong. Truncating to the declared width and
/// re-extending by the declared signedness is the fix, and it only exists
/// because the declaration says what the width is.
#[must_use]
pub fn narrow_return(raw: u64, ret: &CType) -> i64 {
    match *ret {
        CType::Int {
            bits: 8,
            signed: true,
        } => i64::from(raw as u8 as i8),
        CType::Int {
            bits: 8,
            signed: false,
        } => i64::from(raw as u8),
        CType::Int {
            bits: 16,
            signed: true,
        } => i64::from(raw as u16 as i16),
        CType::Int {
            bits: 16,
            signed: false,
        } => i64::from(raw as u16),
        CType::Int {
            bits: 32,
            signed: true,
        } => i64::from(raw as u32 as i32),
        CType::Int {
            bits: 32,
            signed: false,
        } => i64::from(raw as u32),
        // 64-bit, and every pointer: the whole register is the value.
        _ => raw as i64,
    }
}

/// Resolve a C symbol for the running process.
///
/// Searches the cdylibs loft has already loaded, then the process itself —
/// which is what makes a libc binding work with nothing declared and nothing
/// installed. A library that must be `dlopen`ed first is arc D's manifest work.
#[must_use]
pub fn resolve(symbol: &str) -> Option<*const ()> {
    if let Some((p, _)) = crate::extensions::try_dlsym_pub(symbol) {
        return Some(p);
    }
    #[cfg(unix)]
    {
        use libloading::os::unix::Library;
        // `Library::this()` is the process handle: symbols already linked in
        // (libc) and anything loaded with global visibility.
        let this = Library::this();
        let mut name = symbol.to_string();
        name.push('\0');
        if let Ok(sym) = unsafe { this.get::<*const ()>(name.as_bytes()) } {
            return Some(*sym);
        }
    }
    None
}

/// Everything the interpreter needs to make one `#c` call, resolved once at
/// wiring time rather than per call.
#[derive(Clone)]
pub struct CBinding {
    pub sig: CSignature,
    /// The loft parameter types, in declaration order — what decides how each
    /// argument is popped off the stack. Kept beside the C signature rather
    /// than re-derived, because the two together are the mapping and either
    /// alone is half of it.
    pub loft_params: Vec<crate::data::Type>,
    pub void_return: bool,
    /// A `char *` return bound to loft `text`, which comes back through the
    /// destination record rather than the value stack (@PLN24 arc D).
    pub text_return: bool,
}

/// Side table: library index -> the binding that index calls. Populated by
/// [`register`], read by [`dispatch`].
static C_BINDINGS: std::sync::Mutex<Option<std::collections::HashMap<u16, CBinding>>> =
    std::sync::Mutex::new(None);

/// Wire every `#c` declaration into the interpreter's static-call table.
///
/// The call site needs no change: a body-less definition with no `#native`
/// symbol already resolves through `library_names` under its OWN name
/// (`state/codegen.rs`, the `lib_lookup` fallback), so registering under that
/// name is what routes the call here.
pub fn register(state: &mut crate::state::State, data: &crate::data::Data) {
    let target = crate::c_signature::CTarget::host();
    // @PLN24 arc D — open what the program declared, BEFORE any symbol is
    // looked up: `resolve` searches loaded libraries first, so a declared
    // library has to be loaded by the time a call happens. A failure is left
    // for the call site to report, where the binding that needed it can be
    // named.
    for (lib, dir) in &data.c_libraries {
        crate::extensions::load_c_library(lib, dir);
    }
    let mut table = std::collections::HashMap::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.c_sig.is_empty() || *def.code() != crate::data::Value::Null {
            continue;
        }
        let Some(Ok(sig)) = crate::c_signature::of(data, d_nr, target) else {
            continue; // reported at the declaration
        };
        let loft_params: Vec<crate::data::Type> = def
            .attributes()
            .iter()
            .filter(|a| !a.name.starts_with("__") && !a.name.starts_with('#'))
            .map(|a| a.typedef.clone())
            .collect();
        let binding = CBinding {
            sig,
            loft_params,
            void_return: matches!(def.returned(), crate::data::Type::Void),
            text_return: crate::state::codegen::is_c_text_call(def),
        };
        state.static_fn(def.name(), dispatch);
        if let Some(&idx) = state.library_names.get(def.name()) {
            table.insert(idx, binding);
        }
    }
    *C_BINDINGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(table);
}

/// Make one `#c` call: pop the loft arguments, marshal them into `u64` slots,
/// call through the trampoline, bring the return back at its declared width.
///
/// Arguments pop in REVERSE declaration order (the stack is LIFO), which is why
/// the slots are built backwards and reversed — the same shape every native
/// handler in `native.rs` uses.
fn dispatch(stores: &mut crate::database::Stores, stack: &mut crate::keys::DbRef) {
    use crate::data::Type;
    use crate::keys::{DbRef, Str};

    let idx = crate::extensions::current_lib_idx();
    let binding = {
        let guard = C_BINDINGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match guard.as_ref().and_then(|t| t.get(&idx)) {
            Some(b) => b.clone(),
            None => panic!("no `#c` binding registered for library index {idx}"),
        }
    };

    // The NUL-terminated copies a C string argument needs. Held here so they
    // outlive the call: loft text is UTF-8 plus a LENGTH, and C reads to the
    // first NUL, so the copy is what makes the two the same thing.
    let mut owned: Vec<Vec<u8>> = Vec::new();
    let mut slots: Vec<u64> = Vec::with_capacity(binding.sig.params.len());
    for tp in binding.loft_params.iter().rev() {
        match tp.base() {
            Type::Text(_) => {
                let s = *stores.get::<Str>(stack);
                let bytes = s.str().as_bytes();
                let mut b = Vec::with_capacity(bytes.len() + 1);
                b.extend_from_slice(bytes);
                b.push(0u8);
                slots.push(b.as_ptr() as u64);
                owned.push(b);
            }
            Type::Vector(_, _) => {
                // A loft vector is an OUTER record whose word at (rec, pos)
                // names the data record; the elements start at byte 8 and the
                // count sits at byte 4. Pushed as count-then-pointer because
                // the slots are reversed below.
                let r = *stores.get::<DbRef>(stack);
                let data_rec = if r.rec == 0 || r.pos == 0 {
                    0
                } else {
                    stores.store(&r).get_u32_raw(r.rec, r.pos)
                };
                let (ptr, count) = if data_rec == 0 {
                    (0u64, 0u64)
                } else {
                    let st = stores.store(&r);
                    let count = u64::from(st.get_u32_raw(data_rec, 4));
                    // Valid for the duration of the call only — the arena may
                    // move on the next claim, which is the contract the
                    // declaration's mapping states.
                    let p = std::ptr::from_ref(st.addr::<u8>(data_rec, 8)) as u64;
                    (p, count)
                };
                slots.push(count);
                slots.push(ptr);
            }
            _ => slots.push(*stores.get::<i64>(stack) as u64),
        }
    }
    slots.reverse();

    let Some(f) = resolve(&binding.sig.symbol) else {
        panic!(
            "`#c` symbol '{}' not found — it is not in this process and no loaded library \
             exports it. Declare the library it comes from, or check the spelling",
            binding.sig.symbol
        );
    };
    let Some(raw) = (unsafe { call_at_arity(f, &slots) }) else {
        panic!(
            "`#c` symbol '{}' needs {} arguments; the caller covers 0..={}. Wrap it \
             in an ANSI-C shim with fewer parameters",
            binding.sig.symbol,
            slots.len(),
            crate::c_signature::MAX_C_ARITY
        );
    };

    if binding.void_return {
        return;
    }
    if binding.text_return {
        // The dest was stashed by `n_set_bridge_dest`, which the call site emits
        // immediately before this one (`gen_cdylib_text_dest_call`). Write into
        // it and push nothing — the codegen's stack accounting expects exactly
        // that, the same contract the cdylib bridge honours.
        let Some(dest) = stores.bridge_text_dest.take() else {
            panic!(
                "`#c` text binding '{}' was called with no destination — a text return \
                 reaches C only through `gen_cdylib_text_dest_call`",
                binding.sig.symbol
            );
        };
        let s = c_text(raw);
        stores
            .store_mut(&dest)
            .addr_mut::<String>(dest.rec, dest.pos)
            .push_str(&s);
        return;
    }
    // The declared width is what makes this right; read raw, a negative `int`
    // would arrive as a large positive.
    stores.put(stack, narrow_return(raw, &binding.sig.ret));
}

/// Bring a C `char *` return back as loft text.
///
/// Three decisions, and both backends make the same three (`--native` emits the
/// twin of this in `output_c_direct_call`) — a text return that differed between
/// them would be exactly the divergence arc C was built to stop:
///
/// - **NULL is loft null.** text-null is CONTENT-based (`STRING_NULL`), so the
///   record carries it and `??` / `!` / `match` read it as null.
/// - **The bytes end at the first NUL**, because that is what `char *` means. A
///   loft text carries a length and can hold an interior NUL; a C string cannot,
///   so the crossing truncates there rather than inventing a length.
/// - **Invalid UTF-8 is replaced, not refused** — loft text is UTF-8, and a
///   locale-encoded byte from C must not take the program down (C80).
///
/// The pointer is COPIED and never freed. See the return check in
/// `c_signature::check` for why borrowed is the only safe default.
#[must_use]
fn c_text(raw: u64) -> String {
    if raw == 0 {
        return crate::state::STRING_NULL.to_string();
    }
    let p = raw as *const std::ffi::c_char;
    // SAFETY: the declaration says this symbol returns `char *`, and that
    // declaration is the sole authority (there is no runtime signal to check it
    // against — the plan's central measurement). A non-NUL pointer from a
    // correctly declared symbol points at a NUL-terminated string.
    let bytes = unsafe { std::ffi::CStr::from_ptr(p) }.to_bytes();
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe's central finding, as a unit: a 32-bit C return read raw is a
    /// plausible large POSITIVE, and only the declared width recovers it.
    #[test]
    fn a_narrow_return_is_recovered_by_its_declared_width() {
        // What the register actually holds after `int f() { return -1; }` on
        // x86-64: the low half is 0xFFFFFFFF, the upper half zero-extended.
        let raw = 0x0000_0000_FFFF_FFFF_u64;
        assert_eq!(
            raw as i64, 4_294_967_295,
            "read raw, it is a large positive"
        );
        assert_eq!(
            narrow_return(
                raw,
                &CType::Int {
                    bits: 32,
                    signed: true
                }
            ),
            -1,
            "read at its declared width, it is -1"
        );
        assert_eq!(
            narrow_return(
                raw,
                &CType::Int {
                    bits: 32,
                    signed: false
                }
            ),
            4_294_967_295,
            "and an unsigned declaration means the large positive was right"
        );
    }

    #[test]
    fn every_narrow_width_round_trips_its_sign() {
        for (bits, neg) in [(8u8, -5i64), (16, -300), (32, -70000)] {
            let signed = CType::Int { bits, signed: true };
            let raw = neg as u64;
            assert_eq!(narrow_return(raw, &signed), neg, "{bits}-bit signed");
        }
        assert_eq!(
            narrow_return(
                u64::MAX,
                &CType::Int {
                    bits: 64,
                    signed: true
                }
            ),
            -1
        );
    }

    /// libc is linked into this test binary, so resolution is checkable with
    /// nothing installed — and a symbol that does not exist must answer None
    /// rather than a stale or null pointer.
    #[test]
    fn resolves_a_process_symbol_and_refuses_a_missing_one() {
        assert!(resolve("strlen").is_some(), "libc is in the process");
        assert!(resolve("loft_definitely_not_a_symbol_9z").is_none());
    }

    /// The trampolines, exercised against libc directly — the same thing the
    /// interpreter will do, without the interpreter in the way.
    #[test]
    fn the_trampolines_call_libc() {
        let strlen = resolve("strlen").expect("strlen");
        let s = c"hello";
        let n = unsafe { call_at_arity(strlen, &[s.as_ptr() as u64]) }.expect("arity 1");
        assert_eq!(n, 5);

        let atoi = resolve("atoi").expect("atoi");
        let neg = c"-1";
        let raw = unsafe { call_at_arity(atoi, &[neg.as_ptr() as u64]) }.expect("arity 1");
        assert_eq!(
            narrow_return(
                raw,
                &CType::Int {
                    bits: 32,
                    signed: true
                }
            ),
            -1,
            "atoi returns a 32-bit int; raw it reads {raw}"
        );

        let abs = resolve("abs").expect("abs");
        let raw = unsafe { call_at_arity(abs, &[(-7i64) as u64]) }.expect("arity 1");
        assert_eq!(
            narrow_return(
                raw,
                &CType::Int {
                    bits: 32,
                    signed: true
                }
            ),
            7
        );
    }

    /// Beyond the ladder the answer is None, never a truncated call: a wrong
    /// arity is SILENT (the probe got the right answer from an arity-3 call to
    /// an arity-1 symbol), so nothing downstream would catch it.
    #[test]
    fn an_arity_past_the_ladder_is_refused_not_truncated() {
        let f = resolve("abs").expect("abs");
        let too_many = vec![0u64; crate::c_signature::MAX_C_ARITY + 1];
        assert!(unsafe { call_at_arity(f, &too_many) }.is_none());
        assert!(unsafe { call_at_arity(f, &[0u64; crate::c_signature::MAX_C_ARITY]) }.is_some());
    }
}
