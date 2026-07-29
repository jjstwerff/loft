// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector
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
use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    /// Last opcode dispatched on this thread.  (pc, op_name_ptr, op_name_len).
    /// Updated by [`set_context`] from the interpreter's inner loop.
    static LAST_CTX: Cell<Ctx> = const { Cell::new(Ctx::EMPTY) };

    /// Plan-07 phase 1 step 1.20 / phase 3 — pc → source-position table
    /// snapshot for the running interpreter.  `State::execute_argv`
    /// publishes a clone here on entry so the panic hook (a process-wide
    /// non-allocating-restricted callback) can resolve the offending
    /// pc to `file:line:col` when a Rust panic fires inside a loft
    /// runtime fault (panic builtin, arithmetic overflow, future
    /// div-by-zero kinds).  `None` outside the execute window.
    static SOURCE_SPANS: std::cell::RefCell<Option<std::sync::Arc<std::collections::BTreeMap<u32, crate::lexer::Position>>>> = const { std::cell::RefCell::new(None) };
}

// On non-Unix platforms the signal-handler consumer is compiled
// out, so the fields look unread to rustc.  The thread-local
// `set_context` writes still happen (and the tests read them),
// so `#[allow(dead_code)]` is correct here for both targets.
#[derive(Clone, Copy)]
#[allow(dead_code)]
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
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

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

/// Read the last-dispatched opcode context on this thread.
/// Returns (pc, op_code, fn_d_nr).  Used by debug sentinels (e.g.
/// the `put_stack`/`get_stack` DbRef-bounds check) to report which
/// op was executing when a stack value turned out to be corrupt.
#[must_use]
#[allow(dead_code)]
pub fn last_context() -> (u32, u8, u32) {
    LAST_CTX.with(|c| {
        let ctx = c.get();
        (ctx.pc, ctx.op_code, ctx.fn_d_nr)
    })
}

/// Plan-07 phase 1 step 1.20 / phase 3 — publish a snapshot of the
/// current `State::source_spans` so the panic hook can look up
/// `at file:line:col` for the offending pc.  Pass `None` to clear.
pub fn set_source_spans(
    spans: Option<std::sync::Arc<std::collections::BTreeMap<u32, crate::lexer::Position>>>,
) {
    SOURCE_SPANS.with(|s| {
        *s.borrow_mut() = spans;
    });
}

/// Plan-07 phase 1 step 1.20 / phase 3 — look up the source position
/// for `pc` in the current thread's published source-span snapshot.
/// Returns the most recent `Position` recorded at or before `pc`, or
/// `None` if no snapshot is active or no entry precedes `pc`.
#[must_use]
pub fn source_loc_for_pc(pc: u32) -> Option<crate::lexer::Position> {
    SOURCE_SPANS.with(|s| {
        let borrow = s.borrow();
        borrow
            .as_ref()
            .and_then(|m| m.range(..=pc).next_back().map(|(_, p)| p.clone()))
    })
}

thread_local! {
    /// The loft source position the compiler is currently working on.
    ///
    /// A signal handler cannot allocate, so the SIGSEGV path above resolves its
    /// position from a pc.  A PANIC handler runs on the normal stack and may
    /// allocate freely, which is what lets this one carry a real `Position`.
    static COMPILE_POS: RefCell<Option<crate::lexer::Position>> = const { RefCell::new(None) };
}

/// Publish the loft source position the compiler is currently working on, so an
/// internal panic can point at the USER's program instead of at a compiler
/// source file (loft#665 piece 3).
///
/// Called at two chokepoints — per source line while parsing, and per definition
/// while generating code — rather than at the thousands of individual `assert!` /
/// `unwrap` sites, which is the only reason this is one mechanism instead of a
/// spray that would be silently incomplete the moment anyone adds an assert.
pub fn note_compile_pos(pos: &crate::lexer::Position) {
    COMPILE_POS.with(|p| {
        if let Ok(mut slot) = p.try_borrow_mut() {
            *slot = Some(pos.clone());
        }
    });
}

/// Forget the current compile position — call once compilation is done, so a
/// RUNTIME panic is not misattributed to the last line that was compiled.
pub fn clear_compile_pos() {
    COMPILE_POS.with(|p| {
        if let Ok(mut slot) = p.try_borrow_mut() {
            *slot = None;
        }
    });
}

#[must_use]
pub fn compile_pos() -> Option<crate::lexer::Position> {
    COMPILE_POS.with(|p| p.try_borrow().ok().and_then(|b| b.clone()))
}

/// Render an internal compiler panic as a loft diagnostic pointing at the user's
/// source, then defer to the previous hook for the Rust-side detail.
///
/// A compiler-internal invariant that breaks is still something the USER
/// triggered with a specific program, and until now they were shown only
/// `thread 'main' panicked at src/state/codegen.rs:2955` naming a generated
/// symbol and an argument count they never wrote (loft#662).  Nothing in that
/// says which line of their program to look at, or that a bug report is wanted.
///
/// Wrapping the hook rather than replacing it keeps the Rust location and
/// backtrace, which are what make the report actionable.
pub fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(pos) = compile_pos() {
            let detail = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "internal error".to_string());
            let at = info
                .location()
                .map_or_else(String::new, |l| format!(" [{}:{}]", l.file(), l.line()));
            let msg = format!(
                "internal compiler error — please report this program at \
                 https://github.com/loft-lang/loft/issues\n  {detail}{at}"
            );
            let mut diags = crate::diagnostics::Diagnostics::new();
            diags.add_at(
                crate::diagnostics::Level::Fatal,
                &msg,
                &pos.file,
                pos.line,
                pos.pos,
            );
            let mode = crate::diagnostic_render::ErrorMode::from_cli_and_env(None);
            if matches!(mode, crate::diagnostic_render::ErrorMode::Pretty) {
                let loader = crate::diagnostic_render::FileSourceLoader::new();
                eprint!(
                    "{}",
                    crate::diagnostic_render::render_pretty_all(
                        &diags,
                        &loader,
                        crate::diagnostic_render::ColorMode::Auto,
                    )
                );
            } else {
                for entry in diags.entries() {
                    eprintln!("{}", entry.to_string_compact());
                }
            }
        }
        prev(info);
    }));
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
    // Plan-07 phase 3 — try to resolve the offending pc to a loft
    // source position.  This is technically not async-signal-safe
    // (`RefCell::try_borrow` reads a counter that another borrow
    // could be mutating mid-signal), but in practice the
    // SOURCE_SPANS RefCell is mutated only once per `execute_argv`
    // entry — a microsecond window — and `try_borrow` returns Err
    // (we then skip the source-loc print) instead of panicking on
    // a conflict.  Worst case: the crash report omits the source
    // line, which is the same as the pre-phase-3 behaviour.
    let source_loc = SOURCE_SPANS.with(|s| {
        s.try_borrow().ok().and_then(|borrow| {
            borrow
                .as_ref()
                .and_then(|m| m.range(..=ctx.pc).next_back().map(|(_, p)| p.clone()))
        })
    });
    let sig_name = match sig {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        _ => "signal",
    };
    let program = PROGRAM.get().copied().unwrap_or("loft");
    // Build message into a fixed-size buffer, async-signal-safe.
    let mut buf = [0u8; 768];
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
        // Plan-07 phase 3 — emit `at file:line:col` when the source
        // span lookup succeeded.  Truncate file path to fit; the
        // user can still grep for it.
        if let Some(pos) = source_loc.as_ref() {
            let _ = w.str("  at:      ");
            let _ = w.str(&pos.file);
            let _ = w.str(":");
            let _ = w.u32(pos.line);
            let _ = w.str(":");
            let _ = w.u32(pos.pos);
            let _ = w.str("\n");
        }
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

// `Writer` and its methods are pure Rust — available on every
// platform so the `#[cfg(test)]` unit tests compile uniformly.
// Only the signal-handler path that invokes it is `#[cfg(unix)]`,
// so on non-unix non-test builds (e.g. WASM release) it looks dead.
#[allow(dead_code)]
struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

#[allow(dead_code)]
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

    /// Plan-07 phase 3 — source-position lookup for the panic hook +
    /// signal handler.  Verifies the snapshot-based lookup returns the
    /// most-recent entry at-or-before a given pc, matching the
    /// `range(..=pc).next_back()` semantics.
    #[test]
    fn source_loc_lookup_returns_most_recent_at_or_before() {
        use crate::lexer::Position;
        use std::collections::BTreeMap;
        use std::sync::Arc;
        let mut spans = BTreeMap::new();
        spans.insert(
            5,
            Position {
                file: "a.loft".to_string(),
                line: 10,
                pos: 1,
            },
        );
        spans.insert(
            20,
            Position {
                file: "a.loft".to_string(),
                line: 20,
                pos: 1,
            },
        );
        spans.insert(
            50,
            Position {
                file: "a.loft".to_string(),
                line: 30,
                pos: 1,
            },
        );
        set_source_spans(Some(Arc::new(spans)));

        // Exact hit returns the entry's position.
        assert_eq!(source_loc_for_pc(5).unwrap().line, 10);
        assert_eq!(source_loc_for_pc(20).unwrap().line, 20);
        // Between entries returns the floor (most recent at-or-before).
        assert_eq!(source_loc_for_pc(7).unwrap().line, 10);
        assert_eq!(source_loc_for_pc(45).unwrap().line, 20);
        // Past the last entry returns the last entry.
        assert_eq!(source_loc_for_pc(100).unwrap().line, 30);
        // Before the first entry returns None.
        assert!(source_loc_for_pc(0).is_none());
        assert!(source_loc_for_pc(4).is_none());

        // Cleanup so other tests don't see this snapshot.
        set_source_spans(None);
    }

    #[test]
    fn source_loc_lookup_returns_none_when_no_snapshot() {
        // Ensure the thread-local is unset at the start.
        set_source_spans(None);
        assert!(source_loc_for_pc(42).is_none());
    }

    /// loft#665 piece 3 — the compile position an internal panic reports.  It must
    /// be publishable, readable, and above all CLEARABLE: a runtime panic that
    /// still saw a stale compile position would point the user at whatever line
    /// happened to compile last, which is worse than saying nothing.
    #[test]
    fn compile_pos_round_trips_and_clears() {
        clear_compile_pos();
        assert!(compile_pos().is_none(), "starts unset");

        let pos = crate::lexer::Position {
            file: "prog.loft".to_string(),
            line: 12,
            pos: 5,
        };
        note_compile_pos(&pos);
        let got = compile_pos().expect("published position is readable");
        assert_eq!((got.file.as_str(), got.line, got.pos), ("prog.loft", 12, 5));

        // A later position replaces the earlier one — the report wants where the
        // compiler IS, not where it started.
        let later = crate::lexer::Position {
            file: "prog.loft".to_string(),
            line: 30,
            pos: 1,
        };
        note_compile_pos(&later);
        assert_eq!(compile_pos().unwrap().line, 30);

        // Handing off to the runtime drops it, so a runtime panic is not
        // misattributed to the last line compiled.
        clear_compile_pos();
        assert!(
            compile_pos().is_none(),
            "cleared at the compile/run boundary"
        );
    }
}
