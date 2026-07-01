// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F48 — loft CLI (--timeout execution watchdog)

//! @PLAN49 T1+T3 — process-wide execution timeout with a watchdog
//! thread and a shared breadcrumb.
//!
//! The watchdog is the **guaranteed-termination** layer: a background
//! thread sleeps until `T + grace` and then `process::abort()`s.  It
//! does not depend on the executing thread cooperating, so it fires
//! even when control is stuck inside arbitrary Rust/native code, a
//! blocking syscall, or a deadlock.
//!
//! The shared breadcrumb is the *only* diagnostic the watchdog can
//! report — it runs on a different thread and can't safely read the
//! main thread's call stack.  Parser, interpreter, and native codegen
//! call `checkpoint_*` at coarse-grained checkpoints (file entry, fn
//! entry, lexer recovery loop) so when the watchdog fires the printed
//! breadcrumb localises the hang as far as possible.
//!
//! ## Cost when disabled (no `--timeout` / `LOFT_TIMEOUT`)
//!
//! Every checkpoint starts with `if !ARMED.load(Relaxed) { return; }`.
//! That's **one relaxed atomic load + one branch-predicted not-taken
//! branch** — the optimizer keeps the load (it has acquire semantics
//! for thread visibility) but the branch is essentially free.  No
//! allocation, no mutex, no syscall: ~1–2 ns per call site.  Designed
//! to be sprinkled at fn-entry without measurable runtime cost.
//!
//! ## Cost when armed
//!
//! Each checkpoint takes one mutex `try_lock` + a few field writes
//! (~50–100 ns).  Frequencies are low: native fn-entry (once per
//! loft fn call), interpret execute_argv (once per program), parse
//! (once per file), lexer recovery (throttled to every 256
//! iterations).  Not in any hot opcode dispatch loop.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Master "is the timeout active?" flag.  Every checkpoint's first
/// instruction is an atomic load of this — when `false` the
/// breadcrumb update is skipped entirely.
static ARMED: AtomicBool = AtomicBool::new(false);

/// When was the timeout armed?  `None` once means timeout disabled —
/// `deadline_reached` always returns `false`, the watchdog never
/// fires.  Set once via `arm`.
static DEADLINE: OnceLock<Instant> = OnceLock::new();

/// The configured timeout in seconds, stored at `arm` so the cooperative
/// (T2) exit can report it.  `0` while disarmed.
static TIMEOUT_SECS: AtomicU64 = AtomicU64::new(0);

/// Shared breadcrumb — read by the watchdog before it aborts, written
/// by parse-time / runtime / codegen checkpoints.  Held under a
/// `Mutex` so the watchdog can `try_lock` and skip without blocking
/// if the main thread is mid-update (rare, and the watchdog is the
/// last word — losing one byte of breadcrumb is acceptable).
static BREADCRUMB: Mutex<Breadcrumb> = Mutex::new(Breadcrumb {
    phase: "",
    fn_name: "",
    file: String::new(),
    line: 0,
});

struct Breadcrumb {
    /// Coarse execution phase: "parse", "run-interpret", "run-native",
    /// or "" before any checkpoint fires.  Always a string literal.
    phase: &'static str,
    /// Most recent fn name we entered.  Always a string literal (lib
    /// names or codegen-emitted literals); no leak risk.  "" before
    /// any checkpoint fires.
    fn_name: &'static str,
    /// Source filename of whatever code is currently being processed.
    /// Owned because the parser passes a runtime `&str` (filename
    /// from argv); we copy on update so it stays valid.
    file: String,
    /// Line in `file` of the last advanced checkpoint.
    line: u32,
}

/// Arm the watchdog for `timeout_secs` total execution time.  The
/// graceful diagnostic deadline is at `timeout_secs`; the hard kill
/// is at `timeout_secs + grace_secs`.  Idempotent — second call is
/// a no-op (the first deadline + watchdog remain in effect).
///
/// Pass `0` to leave the timeout disarmed.
///
/// # Panics
/// Panics if the OS refuses to spawn the `loft-watchdog` thread.
pub fn arm(timeout_secs: u64, grace_secs: u64) {
    if timeout_secs == 0 {
        return;
    }
    if ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    TIMEOUT_SECS.store(timeout_secs, Ordering::SeqCst);
    let now = Instant::now();
    let deadline = now + Duration::from_secs(timeout_secs);
    let hard = deadline + Duration::from_secs(grace_secs);
    let _ = DEADLINE.set(deadline);
    std::thread::Builder::new()
        .name("loft-watchdog".to_string())
        .spawn(move || {
            // Single sleep, not a polling loop, so the watchdog itself
            // never wakes up unnecessarily before it has work to do.
            let now = Instant::now();
            if let Some(remaining) = hard.checked_duration_since(now) {
                std::thread::sleep(remaining);
            }
            print_breadcrumb_and_abort(timeout_secs, grace_secs);
        })
        .expect("loft: failed to spawn watchdog thread");
}

/// True iff a deadline was armed and has been reached.  Cheap (one
/// atomic load + Instant compare) — callers can sprinkle this in
/// hot paths without observable cost when the timeout is disabled.
#[must_use]
#[inline]
pub fn deadline_reached() -> bool {
    if !ARMED.load(Ordering::Relaxed) {
        return false;
    }
    DEADLINE.get().is_some_and(|d| Instant::now() >= *d)
}

/// @PLAN49 T2 — cooperative graceful exit.  A `checkpoint_*` on the
/// executing thread observed the deadline has passed: print the (current,
/// thread-local-accurate) breadcrumb and exit cleanly with `124` (the GNU
/// `timeout` convention) — *before* the watchdog's `T + grace` hard-kill, so
/// in the common case the watchdog is a no-op and the user gets a clean,
/// localised timeout report.  When execution is genuinely wedged in Rust
/// (no checkpoint runs past `T`), only the watchdog fires — the guarantee.
#[cold]
fn graceful_exit() -> ! {
    let (phase, fn_name, file, line) = match BREADCRUMB.try_lock() {
        Ok(bc) => (bc.phase, bc.fn_name, bc.file.clone(), bc.line),
        Err(_) => ("", "", String::new(), 0),
    };
    eprintln!(
        "[timeout] deadline reached after {}s (graceful): phase={} fn={} file={}:{}",
        TIMEOUT_SECS.load(Ordering::Relaxed),
        if phase.is_empty() { "?" } else { phase },
        if fn_name.is_empty() { "?" } else { fn_name },
        if file.is_empty() { "?" } else { &file },
        line
    );
    std::process::exit(124);
}

/// Native / interpret function-entry checkpoint — combines phase +
/// fn name + (file, line) into ONE mutex acquisition.  All inputs are
/// `&'static str` (string literals from generated code or library
/// names), so we store them by pointer without leaking.
///
/// Hot path when disabled: one relaxed atomic load + branch, return.
#[inline]
pub fn checkpoint_fn(phase: &'static str, name: &'static str, file: &'static str, line: u32) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut bc) = BREADCRUMB.try_lock() {
        bc.phase = phase;
        bc.fn_name = name;
        if bc.file != file {
            bc.file.clear();
            bc.file.push_str(file);
        }
        bc.line = line;
    }
    if deadline_reached() {
        graceful_exit();
    }
}

/// Parser checkpoint — `file` is a runtime `&str` (filename from
/// argv / `use` resolution), so we copy into the breadcrumb's owned
/// `String`.  Called once per parse() entry plus once every N lexer
/// recovery iterations.
///
/// Hot path when disabled: one relaxed atomic load + branch, return.
#[inline]
pub fn checkpoint_parse(file: &str, line: u32) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut bc) = BREADCRUMB.try_lock() {
        bc.phase = "parse";
        bc.fn_name = "";
        if bc.file != file {
            bc.file.clear();
            bc.file.push_str(file);
        }
        bc.line = line;
    }
    if deadline_reached() {
        graceful_exit();
    }
}

fn print_breadcrumb_and_abort(timeout: u64, grace: u64) {
    // try_lock to avoid blocking on a poisoned / held mutex — if we
    // can't read the breadcrumb we still hard-kill (the GUARANTEE).
    let (phase, fn_name, file, line) = match BREADCRUMB.try_lock() {
        Ok(bc) => (bc.phase, bc.fn_name, bc.file.clone(), bc.line),
        Err(_) => ("", "", String::new(), 0),
    };
    eprintln!(
        "[timeout] hard-kill after {}s+{}s grace: \
         phase={} fn={} file={}:{}",
        timeout,
        grace,
        if phase.is_empty() { "?" } else { phase },
        if fn_name.is_empty() { "?" } else { fn_name },
        if file.is_empty() { "?" } else { &file },
        line
    );
    // `process::abort()` raises SIGABRT — useful for debugging
    // (core dump on `ulimit -c`).  Test/CI runs may prefer the
    // cleaner `_exit`; pick via env `LOFT_TIMEOUT_CLEAN_EXIT`.
    if std::env::var("LOFT_TIMEOUT_CLEAN_EXIT").is_ok() {
        std::process::exit(124); // GNU `timeout` convention
    } else {
        std::process::abort();
    }
}

/// Parse `LOFT_TIMEOUT=<seconds>` from the env.  Returns `0` if
/// unset / malformed.  Companion to the `--timeout` CLI flag (which
/// the main binary parses directly and passes to `arm`).
#[must_use]
pub fn env_timeout_secs() -> u64 {
    std::env::var("LOFT_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

/// Grace margin between the cooperative diagnostic deadline `T` and
/// the hard-kill `T + grace`.  Default 2 seconds — long enough for a
/// graceful T2 dump to complete, short enough to keep the user-visible
/// kill snappy.  Overridable via `LOFT_TIMEOUT_GRACE=<seconds>`.
#[must_use]
pub fn env_grace_secs() -> u64 {
    std::env::var("LOFT_TIMEOUT_GRACE")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(2)
}
