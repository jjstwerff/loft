// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN84 ZT-C step 1 / probe P2 (R1) — the load-bearing headless-asyncify
// proof, hand-written so it isolates the asyncify suspend/resume mechanism
// from ALL loft codegen + the WS bridge.  It is the minimal analogue of
// what `web::yield_frame()` will compile to under `--html`:
//
//   * one host import `loft_web.ws_yield()` — the dedicated suspend point
//     (design option 2);
//   * a synchronous counting loop that calls `ws_yield()` each iteration;
//   * output via `loft_io.loft_host_print` (same import the real bundle
//     uses), so the Node driver can SEE the resumes interleave.
//
// Built with the SAME toolchain as a real `--html` bundle:
//   rustc --target wasm32-unknown-unknown --crate-type cdylib -O
//   wasm-opt -O1 --asyncify \
//     --pass-arg=asyncify-imports@loft_gl.loft_gl_swap_buffers,loft_web.ws_yield
//
// PASS = the driver prints frames 0..N interleaved with its own
// `[frame K]` resume markers, then `loft_start: OK` — proving the WASM
// returns to the JS event loop and resumes WITHOUT any GL window.

#![no_std]
#![allow(internal_features)]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}

#[link(wasm_import_module = "loft_io")]
unsafe extern "C" {
    safe fn loft_host_print(ptr: *const u8, len: usize);
}

// The dedicated asyncify suspend import (design option 2).  Calling this
// once per loop iteration is what `web::yield_frame()` lowers to; wasm-opt
// instruments it as an unwind/rewind point via the `--pass-arg`.
#[link(wasm_import_module = "loft_web")]
unsafe extern "C" {
    safe fn ws_yield();
}

// A tiny fixed scratch buffer in linear memory for formatting.  Static mut
// is fine: single-threaded, one writer.
static mut BUF: [u8; 32] = [0; 32];

fn print_line(prefix: &[u8], n: u32) {
    // Format "<prefix><n>\n" into BUF and hand it to the host.
    unsafe {
        let buf = &mut *core::ptr::addr_of_mut!(BUF);
        let mut i = 0usize;
        for &b in prefix {
            buf[i] = b;
            i += 1;
        }
        // decimal of n
        if n == 0 {
            buf[i] = b'0';
            i += 1;
        } else {
            let start = i;
            let mut v = n;
            while v > 0 {
                buf[i] = b'0' + (v % 10) as u8;
                v /= 10;
                i += 1;
            }
            // reverse the digits in place
            let end = i - 1;
            let mut a = start;
            let mut b2 = end;
            while a < b2 {
                buf.swap(a, b2);
                a += 1;
                b2 -= 1;
            }
        }
        buf[i] = b'\n';
        i += 1;
        loft_host_print(buf.as_ptr(), i);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loft_start() {
    let mut i: u32 = 0;
    while i < 5 {
        print_line(b"frame=", i);
        // Yield to the host between iterations: under asyncify this unwinds
        // the wasm stack back to JS and the driver resumes on the next
        // setImmediate.  Without the suspend point this is a no-op import.
        ws_yield();
        i += 1;
    }
    print_line(b"done=", i);
}
