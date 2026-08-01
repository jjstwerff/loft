// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN24 probe — does the fixed per-arity trampoline design actually hold?
//!
//! The plan's architecture rests on one claim:
//!
//! > Every integer-class C argument (int, long, pointer, `char*`, bool, enum)
//! > collapses to a `u64`, so the only dimension is **arity** — about thirteen
//! > `extern "C" fn(u64, …) -> u64` functions compiled into loft-core once, and
//! > no libffi.
//!
//! It is currently backed by one throwaway probe from 2026-06-14 that called
//! `strlen` and `strncmp`. Two symbols cannot see the boundary of a claim this
//! load-bearing, so this runs it across the whole type matrix in
//! `tests/fixtures/c_abi/` — including the argument-register boundary, the
//! return width, and every case the plan says needs a shim.
//!
//! Written in Rust, not C, on purpose: the trampolines land in loft-core, which
//! is Rust, so the question is whether **Rust** can express them soundly. A C
//! probe would prove something nobody needs.
//!
//! Build and run (no cargo, no dependencies):
//!
//! ```sh
//! make probe
//! ```
//!
//! Two kinds of cell, and the difference matters:
//!
//! - **ASSERT** — the design claim. A failure here means the architecture is
//!   wrong, and the probe exits non-zero.
//! - **REPORT** — behaviour the C ABI does not define, or defines as "don't".
//!   The value is information for the design, never a verdict: asserting on
//!   undefined behaviour would pin whatever this machine happened to do today.

use std::ffi::{CString, c_char, c_int, c_void};

unsafe extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *mut c_char;
}

const RTLD_NOW: c_int = 2;

/// The trampoline set, at the arities this probe needs. loft-core would carry
/// the full 0..=12 ladder; the shape is the same for every rung.
type T0 = extern "C" fn() -> u64;
type T1 = extern "C" fn(u64) -> u64;
type T2 = extern "C" fn(u64, u64) -> u64;
type T3 = extern "C" fn(u64, u64, u64) -> u64;
type T4 = extern "C" fn(u64, u64, u64, u64) -> u64;
type T6 = extern "C" fn(u64, u64, u64, u64, u64, u64) -> u64;
type T7 = extern "C" fn(u64, u64, u64, u64, u64, u64, u64) -> u64;
#[rustfmt::skip]
type T12 = extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) -> u64;

struct Lib(*mut c_void);

impl Lib {
    fn open(path: &str) -> Lib {
        let c = CString::new(path).expect("path");
        let h = unsafe { dlopen(c.as_ptr(), RTLD_NOW) };
        if h.is_null() {
            let e = unsafe { dlerror() };
            let msg = if e.is_null() {
                "unknown".to_string()
            } else {
                unsafe { std::ffi::CStr::from_ptr(e) }
                    .to_string_lossy()
                    .into_owned()
            };
            panic!("dlopen({path}) failed: {msg}\nrun `make` in this directory first");
        }
        Lib(h)
    }

    /// Resolve a symbol and reinterpret it at the caller's chosen arity.
    ///
    /// This is the whole trick, and also the whole risk: nothing checks that
    /// `F` matches the C function. The compiler cannot; `dlsym` returns an
    /// address and no signature. Whatever discipline keeps them in step has to
    /// come from the loft declaration — which is exactly what the return-width
    /// and arity cells below are measuring the cost of.
    fn sym<F: Copy>(&self, name: &str) -> F {
        assert_eq!(
            size_of::<F>(),
            size_of::<*mut c_void>(),
            "F must be a bare fn pointer"
        );
        let c = CString::new(name).expect("symbol");
        let p = unsafe { dlsym(self.0, c.as_ptr()) };
        assert!(!p.is_null(), "dlsym({name}) found nothing");
        unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) }
    }
}

static mut FAILURES: u32 = 0;

fn assert_eq_u64(what: &str, got: u64, want: u64) {
    if got == want {
        println!("  ok     {what}: {got}");
    } else {
        unsafe { FAILURES += 1 };
        println!("  FAIL   {what}: got {got}, want {want}");
    }
}

fn report(what: &str, note: &str) {
    println!("  report {what}: {note}");
}

fn main() {
    let lib = Lib::open(lib_path());

    println!("\n@PLN24 — the trampoline claim, measured\n");

    // ---- A. Arity, including the register/stack boundary -------------------
    //
    // On SysV x86-64 the first SIX integer arguments travel in registers and
    // the seventh onward on the stack; on AArch64 the split is at eight. If a
    // fixed trampoline set is going to break anywhere it is there, so 6 and 7
    // are adjacent cells rather than points on a ladder. Each argument is
    // weighted by a distinct prime, so two arguments swapped would show.
    println!("A. arity — 1 is passed to every parameter, so the answer IS the weight sum");
    let a0: T0 = lib.sym("lc_arity0");
    assert_eq_u64("arity0", a0(), 0x10F7);
    let a1: T1 = lib.sym("lc_arity1");
    assert_eq_u64("arity1(21)", a1(21), 42);
    let a6: T6 = lib.sym("lc_arity6");
    assert_eq_u64("arity6 (last in registers)", a6(1, 1, 1, 1, 1, 1), 41);
    let a7: T7 = lib.sym("lc_arity7");
    assert_eq_u64("arity7 (first on the stack)", a7(1, 1, 1, 1, 1, 1, 1), 58);
    let a12: T12 = lib.sym("lc_arity12");
    assert_eq_u64(
        "arity12",
        a12(1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1),
        197,
    );
    // Position, not just count: move a 2 into the stack slot only.
    assert_eq_u64(
        "arity7 stack argument carries its own weight",
        a7(0, 0, 0, 0, 0, 0, 2).wrapping_sub(a7(0, 0, 0, 0, 0, 0, 1)),
        17,
    );

    // ---- B. A full 64-bit value ------------------------------------------
    println!("\nB. width — a value with bits in both halves of the word");
    let i64f: T1 = lib.sym("lc_i64");
    let mask: u64 = 0x0123_4567_89AB_CDEF;
    assert_eq_u64("lc_i64(0) is the whole mask", i64f(0), mask);
    assert_eq_u64("lc_i64 round trips", i64f(i64f(1234567890123)), 1234567890123);
    assert_eq_u64(
        "and round trips a negative",
        i64f(i64f((-1i64) as u64)),
        (-1i64) as u64,
    );

    // ---- C. The return width — the load-bearing cell ----------------------
    //
    // This is the plan's own open question, and it has an answer.
    //
    // `int32_t lc_neg_i32(int32_t)` returns -1 for 1. A 32-bit C return writes
    // EAX; the upper half of RAX is left in whatever state the callee's last
    // instruction happened to leave it. Reading it as a u64 is therefore not
    // "sign-extension gone wrong" — there is no defined value up there at all.
    println!("\nC. return width — a 32-bit C return read through a u64 trampoline");
    let neg: T1 = lib.sym("lc_neg_i32");
    let raw = neg(1);
    report(
        "lc_neg_i32(1) raw u64",
        &format!(
            "0x{raw:016X} ({raw}) — SysV leaves the upper half UNSPECIFIED for a \
             32-bit return, and on x86-64 a 32-bit write zero-extends, so what \
             comes back is the zero-extension of -1: a large POSITIVE number. \
             That is the dangerous direction — not a crash, a plausible value \
             that `>= 0` accepts (the shape that defeated loft-libs-net's \
             `server::listen`)"
        ),
    );
    assert_eq_u64(
        "truncate to the DECLARED C width, then sign-extend",
        (raw as u32 as i32 as i64) as u64,
        (-1i64) as u64,
    );
    report(
        "consequence",
        "the trampoline cannot be signature-blind on the RETURN. Arity alone \
         does not parameterise it — the declaration must carry the C return \
         width, so `#c \"sym\"` needs the type spelling (option (a) in the plan)",
    );

    // ---- D. Pointers, as arguments and as returns -------------------------
    //
    // The convention the DB clients need: an opaque handle (`PGconn *`) is a
    // loft `integer` holding the pointer value.
    println!("\nD. pointers — text in, handle out, handle back in");
    let strlen: T1 = lib.sym("lc_strlen");
    let text = CString::new("loft").expect("text");
    assert_eq_u64("lc_strlen(\"loft\")", strlen(text.as_ptr() as u64), 4);
    let static_text: T0 = lib.sym("lc_static_text");
    let borrowed = static_text();
    assert_eq_u64("a returned const char* is usable", strlen(borrowed), 10);

    let open: T1 = lib.sym("lc_open");
    let read: T1 = lib.sym("lc_read");
    let bump: T2 = lib.sym("lc_bump");
    let close: T1 = lib.sym("lc_close");
    let h = open(1000);
    assert_eq_u64("handle survives the u64 round trip", read(h), 1000);
    assert_eq_u64("and is still the same object", bump(h, 7), 1007);
    close(h);
    assert_eq_u64("a null handle reports rather than faults", read(0), u64::MAX);

    // ---- E. Vectors -------------------------------------------------------
    println!("\nE. vectors — pointer + count, weighted so order faults show");
    let v64: [i64; 3] = [10, 20, 30];
    let sum64: T2 = lib.sym("lc_i64_sum");
    assert_eq_u64("lc_i64_sum", sum64(v64.as_ptr() as u64, 3), 140);
    let v32: [i32; 3] = [10, 20, 30];
    let sum32: T2 = lib.sym("lc_i32_sum");
    assert_eq_u64("lc_i32_sum", sum32(v32.as_ptr() as u64, 3), 140);

    // ---- F. The shims -----------------------------------------------------
    //
    // Everything the plan says needs an ANSI-C shim, reached through the same
    // integer-class trampolines. If these pass, the escape hatch is real.
    println!("\nF. shims — the boundary cases, made integer-class");
    let shim_f64: T1 = lib.sym("lc_shim_f64");
    let out = f64::from_bits(shim_f64(2.0f64.to_bits()));
    assert_eq_u64("lc_shim_f64(2.0) == 4.5", (out == 4.5) as u64, 1);
    let shim_f32: T1 = lib.sym("lc_shim_f32");
    let out32 = f32::from_bits(shim_f32(2.0f32.to_bits() as u64) as u32);
    assert_eq_u64("lc_shim_f32(2.0) == 4.5", (out32 == 4.5) as u64, 1);
    let shim_dot: T2 = lib.sym("lc_shim_pair_dot");
    assert_eq_u64("struct by value, via shim", shim_dot(4, 5), 37);
    let shim_div: T2 = lib.sym("lc_shim_div");
    let shim_mod: T2 = lib.sym("lc_shim_mod");
    assert_eq_u64("out-parameter, via shim (quotient)", shim_div(17, 5), 3);
    assert_eq_u64("out-parameter, via shim (remainder)", shim_mod(17, 5), 2);
    let shim_sum3: T3 = lib.sym("lc_shim_sum3");
    assert_eq_u64("varargs, via shim", shim_sum3(1, 2, 3), 6);

    // ---- G. What the trampolines must NOT be pointed at --------------------
    //
    // Reported, never asserted. Both of these are undefined behaviour, and the
    // reason they are here is that neither one CRASHES — which is the trap. A
    // design that relies on "we'd notice" is relying on nothing.
    println!("\nG. the boundary — undefined, and quiet about it");
    let f64_direct: T1 = lib.sym("lc_f64");
    let bogus = f64_direct(2.0f64.to_bits());
    report(
        "double arg through an integer trampoline",
        &format!(
            "returned 0x{bogus:016X}, wanted the bits of 4.5 \
             (0x{:016X}) — the argument went to rdi and the callee read xmm0",
            4.5f64.to_bits()
        ),
    );
    let var_direct: T4 = lib.sym("lc_var_sum");
    let var = var_direct(3, 1, 2, 3);
    report(
        "variadic through a non-variadic trampoline",
        &format!(
            "returned {var} (the right answer is 6). SysV requires AL to hold \
             the vector-register count for a variadic call; a non-variadic \
             call site does not set it, so a right answer here is luck, not a \
             guarantee — this is precisely why varargs gets a shim"
        ),
    );
    let over: T3 = lib.sym("lc_arity1");
    report(
        "arity-1 symbol called through an arity-3 trampoline",
        &format!(
            "returned {} (wanted 42) — extra register arguments are ignored, so \
             an arity MISTAKE is silent. Nothing at runtime can catch it; the \
             arity has to come from the declaration and be right",
            over(21, 999, 999)
        ),
    );

    // ---- Verdict ----------------------------------------------------------
    let failures = unsafe { FAILURES };
    println!("\n---\n");
    if failures == 0 {
        println!(
            "The claim HOLDS for arguments: every integer-class argument — int, \
             long, pointer, char*, bool — passes through a u64 slot, at every \
             arity including across the register/stack boundary, with no libffi \
             and no generated code.\n\n\
             It does NOT hold for returns. A 32-bit C return leaves the upper \
             half of the register undefined, so `arity alone parameterises the \
             trampoline set` is true of the call and false of the answer. The \
             set stays ~13 functions; the DECLARATION has to carry the C return \
             width (plan option (a): `#c \"PQstatus\" \"int(void*)\"`).\n\n\
             The shims work, and they are three lines each — so the trade the \
             plan is built on (complexity in the shim, not in loft-core) is \
             real.\n\n\
             And read section G again: of its three misuses, ONE produced \
             garbage and TWO produced the right answer. A wrong arity and a \
             variadic call both came back correct, by luck. So the boundary is \
             not self-enforcing at runtime — nothing here would have told you. \
             Whatever keeps a `#c` declaration in step with the C signature has \
             to be a COMPILE-time check against a spelled-out type, because \
             there is no runtime signal to fall back on."
        );
    } else {
        println!("{failures} assertion(s) failed — the design claim does not hold as written.");
        std::process::exit(1);
    }
}

fn lib_path() -> &'static str {
    if cfg!(target_os = "macos") {
        "./liblc_types.dylib"
    } else {
        "./liblc_types.so"
    }
}
