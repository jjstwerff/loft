// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Minimal crash reporter: installs a SIGSEGV / SIGABRT / SIGBUS
// signal handler that prints the last-executed opcode, bytecode
// position, and function name from a thread-local context marker
// updated by the interpreter's execute loop.
//
// Goal: when the interpreter crashes inside an opcode that corrupts
// memory (heap overflow, stack overflow, use-after-free), the
// glibc / kernel error arrives OUT of Rust's panic path — `set_hook`
// only catches panics, not signals.  Without a native handler the
// process just aborts with no context.
//
// Usage: call [`install`] once near program start (or at test
// harness init).  The interpreter's execute loop calls
// [`set_context`] before each opcode dispatch to publish the
// current PC / function / op-name.  On a crash the handler reads
// that context and prints a one-line diagnostic to stderr before
// the default handler runs.
//
// Design notes:
//
// - Async-signal safety: signal handlers are extremely restricted —
//   we cannot call `format!`, `println!`, or allocate.  We format
//   directly into a thread-local `[u8; N]` buffer and write it with
//   `libc::write(STDERR_FILENO, ...)`.  Everything is
//   reentrant/async-signal-safe.
// - Thread-local context: each thread publishes its own context.
//   On crash we read the local thread's context only; worker threads
//   get their own trace.
// - No allocation: the buffer is fixed-size; the fields are all
//   fixed-width types (u32 pc, u16 op code) or a `&'static str`
//   (the op name, which is a compile-time constant from the
//   interpreter's opcode table).

#![allow(clippy::module_name_repetitions)]

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    /// Last opcode dispatched on this thread.  (pc, op_name_ptr, op_name_len).
    /// Updated by [`set_context`] from the interpreter's inner loop.
    static LAST_CTX: Cell<Ctx> = const { Cell::new(Ctx::EMPTY) };
}

#[derive(Clone, Copy)]
struct Ctx {
    pc: u32,
    fn_d_nr: u32,
    op_code: u8,
    op_name: &'static str,
    fn_name: &'static str,
}

impl Ctx {
    const EMPTY: Ctx = Ctx {
        pc: u32::MAX,
        fn_d_nr: u32::MAX,
        op_code: 0,
        op_name: "",
        fn_name: "",
    };
}

/// Used by the installer to ensure we only install once per process.
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// Holds the program name for the diagnostic prefix.
static PROGRAM: OnceLock<&'static str> = OnceLock::new();

/// Update the per-thread context just before an opcode dispatches.
///
/// Call this AT MOST ONCE per opcode.  The inner loop overhead is
/// a single thread-local store (one atomic-less write on most
/// platforms) so the hot path stays cheap.
#[inline]
pub fn set_context(
    pc: u32,
    op_code: u8,
    op_name: &'static str,
    fn_d_nr: u32,
    fn_name: &'static str,
) {
    LAST_CTX.with(|c| {
        c.set(Ctx {
            pc,
            fn_d_nr,
            op_code,
            op_name,
            fn_name,
        });
    });
}

/// Install signal handlers for SIGSEGV / SIGABRT / SIGBUS.
///
/// No-op on non-Unix platforms and when called more than once.
pub fn install(program: &'static str) {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = PROGRAM.set(program);
    #[cfg(unix)]
    unsafe {
        for &sig in &[libc::SIGSEGV, libc::SIGABRT, libc::SIGBUS] {
            let mut act: libc::sigaction = std::mem::zeroed();
            act.sa_sigaction = handler as *const () as libc::sighandler_t;
            // SA_SIGINFO for the siginfo/ucontext args we ignore here; SA_RESETHAND so
            // the default handler runs after we print (produces the core dump).
            act.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND;
            libc::sigemptyset(&raw mut act.sa_mask);
            libc::sigaction(sig, &raw const act, std::ptr::null_mut());
        }
    }
}

/// Async-signal-safe handler.  Reads the thread-local context and
/// writes a one-line diagnostic to stderr; the default handler
/// then takes over (which produces a core dump if `ulimit -c` is
/// set).
#[cfg(unix)]
extern "C" fn handler(sig: libc::c_int, _info: *mut libc::siginfo_t, _ucontext: *mut libc::c_void) {
    // Read the context.  If the interpreter wasn't running, EMPTY
    // fields produce a "no context" message — still useful to
    // confirm the signal fired.
    let ctx = LAST_CTX.with(Cell::get);
    let sig_name = match sig {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        _ => "signal",
    };
    let program = PROGRAM.get().copied().unwrap_or("loft");
    // Build message into a fixed-size buffer, async-signal-safe.
    let mut buf = [0u8; 512];
    let mut w = Writer::new(&mut buf);
    let _ = w.str("\n=== loft crash (");
    let _ = w.str(program);
    let _ = w.str(") ");
    let _ = w.str(sig_name);
    let _ = w.str(" caught ===\n  last op:  ");
    if ctx.op_name.is_empty() {
        let _ = w.str("(none — crash outside interpreter)\n");
    } else {
        let _ = w.str(ctx.op_name);
        let _ = w.str(" (op=");
        let _ = w.u32(u32::from(ctx.op_code));
        let _ = w.str(")\n  pc:       ");
        let _ = w.u32(ctx.pc);
        let _ = w.str("\n  fn:       ");
        let _ = w.str(if ctx.fn_name.is_empty() {
            "(?)"
        } else {
            ctx.fn_name
        });
        let _ = w.str(" (d_nr=");
        let _ = w.u32(ctx.fn_d_nr);
        let _ = w.str(")\n");
    }
    let _ = w.str("===\n");
    let bytes = w.as_bytes();
    unsafe {
        let _ = libc::write(
            libc::STDERR_FILENO,
            bytes.as_ptr().cast::<libc::c_void>(),
            bytes.len(),
        );
    }
    // SA_RESETHAND → the default handler fires next, producing the
    // core dump and terminating the process.
}

#[cfg(unix)]
struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

#[cfg(unix)]
impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Writer { buf, pos: 0 }
    }
    fn str(&mut self, s: &str) -> Result<(), ()> {
        for &b in s.as_bytes() {
            if self.pos >= self.buf.len() {
                return Err(());
            }
            self.buf[self.pos] = b;
            self.pos += 1;
        }
        Ok(())
    }
    fn u32(&mut self, mut n: u32) -> Result<(), ()> {
        if n == 0 {
            return self.str("0");
        }
        let mut digits = [0u8; 10];
        let mut i = 0;
        while n > 0 {
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            i += 1;
        }
        while i > 0 {
            i -= 1;
            if self.pos >= self.buf.len() {
                return Err(());
            }
            self.buf[self.pos] = digits[i];
            self.pos += 1;
        }
        Ok(())
    }
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.pos]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_basic() {
        let mut buf = [0u8; 64];
        let mut w = Writer::new(&mut buf);
        w.str("pc=").unwrap();
        w.u32(42).unwrap();
        w.str(" op=").unwrap();
        w.str("OpReturn").unwrap();
        assert_eq!(w.as_bytes(), b"pc=42 op=OpReturn");
    }

    #[test]
    fn writer_zero() {
        let mut buf = [0u8; 8];
        let mut w = Writer::new(&mut buf);
        w.u32(0).unwrap();
        assert_eq!(w.as_bytes(), b"0");
    }

    #[test]
    fn writer_max() {
        let mut buf = [0u8; 16];
        let mut w = Writer::new(&mut buf);
        w.u32(u32::MAX).unwrap();
        assert_eq!(w.as_bytes(), b"4294967295");
    }

    #[test]
    fn context_updates() {
        set_context(10, 7, "OpVarInt", 42, "main");
        LAST_CTX.with(|c| {
            let ctx = c.get();
            assert_eq!(ctx.pc, 10);
            assert_eq!(ctx.op_code, 7);
            assert_eq!(ctx.op_name, "OpVarInt");
            assert_eq!(ctx.fn_d_nr, 42);
            assert_eq!(ctx.fn_name, "main");
        });
    }

    #[test]
    fn install_is_idempotent() {
        // Calling twice should not panic or misbehave.
        install("test");
        install("test");
    }
}
