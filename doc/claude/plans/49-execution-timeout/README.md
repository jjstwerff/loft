<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 49 — Execution timeout: guaranteed termination + on-timeout diagnostics

## Status

Open — design ready, no code.  Motivated by repeated hang-debugging pain: this
project has hit several hangs (@P356/#9 — OOB index → astronomical loop; the
`rand` OOB hang; `native_oob_null`), and the only recourse was an external
`timeout N ./loft …` which `SIGKILL`s with **zero** context.  Worse, the
in-process cargo harness (`tests/wrap.rs`) runs scripts in the same process, so
one hanging script wedges the whole `cargo test` / `find_problems.sh` run —
fatal in autonomous/CI mode.

## Goal

A built-in execution deadline that (1) is **guaranteed to terminate the task —
even when execution is stuck inside arbitrary Rust/native code, a blocking
syscall, or a deadlock** — and (2) dumps **where loft is** (fn + source line +
call stack, plus the recent trace on the interpreter) on the way down.  Both
backends.

## Termination guarantee — the hard requirement (read this first)

A **cooperative** deadline check (poll a deadline in the bytecode dispatch loop,
or at codegen loop back-edges) gives the *richest* diagnostics but **cannot
guarantee termination**: it only runs *between* loft operations.  If control is
stuck inside a Rust routine — a `#native` cdylib fn spinning, a blocking
`ws_recv`/`tcp_accept`, a deadlock, or a tight Rust loop with no back-edge
instrumented — the cooperative check never runs and the task never stops.  So the
cooperative check is the **diagnostic layer, not the guarantee.**

The guarantee needs a killer that does **not** depend on the executing thread
cooperating.  Two mechanisms, in increasing robustness (the user OK'd
OS-dependent code):

1. **Watchdog thread → `process::abort()` (the in-process guarantee).**  A
   background thread sleeps until the deadline (+ a short grace) and then calls
   `std::process::abort()` / `libc::_exit(code)`.  This terminates the **whole
   process** via `SIGABRT`/`exit` regardless of what the main thread is doing —
   spinning in native code, blocked in a syscall, holding a lock — because it
   acts process-wide and needs no cooperation from the hung thread.  The OS
   preempts the busy thread to schedule the watchdog (true even on a saturated
   single core).  **This alone guarantees the loft binary stops.**  Cost: the
   whole process dies (correct for `loft run` / one program per process); the
   guaranteed-path dump is limited to a *shared breadcrumb* (see below), since
   the watchdog cannot safely read the hung thread's thread-local call stack.
2. **Subprocess isolation + external `SIGKILL` (the gold standard).**  Run the
   loft work in a **child process**; a parent/monitor enforces the deadline by
   `SIGTERM` (grace for a self-dump) → `SIGKILL`.  `SIGKILL` cannot be caught,
   blocked, or ignored, so the child *always* dies even if its own watchdog
   thread wedged, its heap is corrupted, or it's in uninterruptible `D` state.
   The OS reclaims everything.  **Required for the in-process cargo harness**
   (`tests/wrap.rs`): a watchdog-`abort` there would kill the *entire* `cargo
   test` process, not just the one hung script — so harness isolation must run
   each script in a child (or shell out to the `loft` binary, which then
   self-watchdogs).

**Layering — the watchdog is always SLOWER than the diagnostic path (key
principle).**  Full diagnostics are *preferred*, so the cooperative diagnostic
deadline fires **first, at the requested `T`**, and the guaranteed watchdog
hard-kill is set **strictly later, at `T + grace`**.  Whenever execution can
reach a checkpoint (loop back-edge / fn-entry / dispatch step), the cooperative
layer raises a typed `Timeout`, dumps the rich diagnostic (full stack +
`crash_tail`), and exits cleanly *before* the watchdog ever fires — so in the
common case the watchdog is a no-op and we always get the best diagnostic.  Only
when execution is genuinely stuck in Rust (the cooperative check can't run) does
the clock reach `T + grace` and the watchdog hard-kills with the shared
breadcrumb.  Net: **best-diagnostic-wins by construction; guaranteed-hard-kill
as the strictly-later backstop.**

## Sub-arcs

| Item | What | Status |
|---|---|---|
| **T1** — watchdog hard-kill (THE GUARANTEE) | watchdog thread; fires at **`T + grace`** (always later than T2), dumps the shared breadcrumb, then `process::abort()`/`_exit` — terminates even when stuck in Rust/native/syscall | Open |
| **T2** — cooperative rich diagnostic (PREFERRED, fires first at `T`) | deadline check at every loft "checkpoint" — runtime AND parse-time — raises a typed `Timeout` and dumps the rich context: (a) interpreter dispatch-loop entry; (b) native fn-entry / loop back-edge; (c) **parse-time entry points** — `Lexer::next`, `Lexer::cont`, `Lexer::recover_to` (the recovery loop in particular: 2026-05-27 we hit a 7-min infinite recovery loop on a malformed script).  Each checkpoint updates the shared breadcrumb (phase=parse/run, file, line) so T1's hard-kill has actionable context even if T2 misses. | Open |
| **T3** — CLI / env + `--tests` default | `--timeout <secs>` + `LOFT_TIMEOUT`; **default ON under `--tests`/`loft test`**; the two-phase grace (graceful → hard) | Open |
| **T4** — subprocess isolation for the harness | `tests/wrap.rs` / `tests/native.rs` run each script in a child process (or shell to `loft`) so a hard-kill localizes to one test, not the whole suite | Open |

## Diagnostics: what each path can report

| Path | Available info |
|---|---|
| **Cooperative (T2), interpreter** | fn + `file:line`, `pc`, full **call stack** (`stack_trace`/`StackFrame`, `src/native.rs:1786`), and the **last N execution steps** (`crash_tail:N`, `src/log_config.rs:346`) |
| **Cooperative (T2), native** | fn + `file:line` + full **call stack** — native already maintains a runtime stack (`CALL_STACK`: thread-local `Vec<(name,file,line)>` via `cr_call_push`/`CallGuard`, `src/codegen_runtime.rs:3796`).  The check runs on the executing thread, so it can read it.  No per-op trace (`crash_tail` is interpreter-only). |
| **Cooperative (T2), parse-time** | current source `file:line` + the parser's current expectation token (the "Expect …" message that recovery is trying to satisfy).  The lexer is the active stack frame; no call stack to dump, but the breadcrumb has enough to localize the recovery loop. |
| **Watchdog hard-kill (T1)** | a **shared breadcrumb** only — the watchdog runs on another thread and can't read the thread-local `CALL_STACK`.  Codegen/interpreter/lexer store the current `(phase, file, line)` into **shared atomics** at every checkpoint; the watchdog prints "last at phase=X file:line before hard-kill".  `phase ∈ {parse, run-interpret, run-native}`.  If the hang is *inside* a native fn, the breadcrumb is its loft call site — which is exactly the useful fact ("hung in/after `ws_recv` at file:line"). |

The shared-breadcrumb store (two atomics, updated at fn-entry + line boundaries)
is cheap and is what makes the *guaranteed* path still informative.  `SIGALRM`/
signal-handler dumping is rejected: a signal interrupting the `RefCell` borrow of
`CALL_STACK` is UB, and async-signal-safe formatting is fragile.

## Native specifics (T1 + T2 on `--native`)

- T2 cooperative checks: codegen injects `cr_check_deadline()` at **fn-entry**
  (next to `cr_call_push` — catches runaway recursion) and **loop back-edges**
  (catches infinite loops); on hit it reads the thread-local `CALL_STACK`, dumps
  the full stack, and exits non-zero.  A watchdog bumps a global `NOW_TICKS:
  AtomicU64` every ~10 ms so the per-iteration check is a relaxed load + compare
  vs a `DEADLINE_TICK` (`u64::MAX` when no timeout → branch-predicted not-taken,
  ~free).
- T1 watchdog hard-kill: the SAME watchdog thread, after the grace, calls
  `process::abort()` — guaranteeing termination of a native binary even when the
  cooperative checks can't fire (stuck in a vendored cdylib's Rust loop, a
  blocking socket read, etc.).

## Phase ordering

1. **T1 + T3 + breadcrumb skeleton** — the guarantee first (watchdog thread +
   `abort` + shared breadcrumb), wired via `--timeout` / `LOFT_TIMEOUT`.
   Breadcrumb is set at three coarse points: parse entry (`<file>` + line
   updates as lexer advances), interpreter dispatch (current fn + file:line),
   native fn-entry (current fn + file:line via `cr_call_push`).  Even before
   rich diagnostics exist, this stops the bleeding: no run can hang forever,
   and the breadcrumb localises parse-time vs runtime hangs.  Wire it for both
   backends (interpreter run + generated-binary main).
2. **T2** — layer the cooperative rich diagnostic on top.  Runtime
   (interpreter dispatch loop, native fn-entry/back-edge checks) AND
   parse-time (`Lexer::next`/`recover_to` deadline check raising a typed
   `Timeout` instead of looping forever).
3. **T4** — subprocess isolation for the cargo harness (pairs with @P369; lets a
   hard-kill localize to one test instead of the suite).

## Open questions

1. **Default timeout values.**  `--tests` default (30 s/test? per-file?); no
   default for a plain `loft run` (real programs run long) unless `--timeout`.
2. **Grace margin** `grace` (settled principle: watchdog fires at `T + grace`,
   always strictly later than the cooperative `T`).  Only the exact value is
   open — long enough for the graceful dump to complete, short enough to stay
   snappy (≈1–2 s, or scale with `T`).
3. **`process::abort()` vs `_exit(code)`** — `abort` gives a core dump (debug
   value) but a noisy `SIGABRT`; `_exit` is clean.  Pick per context (dev vs CI).
4. **`par(...)` / worker threads.**  Each worker has its own `CALL_STACK`; the
   cooperative check must cover worker loops, and the watchdog's `abort` kills
   all workers (fine — the whole process is the task).
5. **Windows.**  `abort`/threads are portable; the subprocess-`SIGKILL` backstop
   needs the Windows equivalent (`TerminateProcess` / Job Objects) in T4.

## See also

- [PROBLEMS.md](../../PROBLEMS.md) — the hang P-issues that motivated this (@P356/#9, etc.).
- `src/state/mod.rs` (dispatch loops, T2), `src/log_config.rs` (`crash_tail`),
  `src/native.rs` (`stack_trace`/`StackFrame`), `src/codegen_runtime.rs`
  (`CALL_STACK`/`cr_call_push`, T1/T2), `src/main.rs` + `src/test_runner.rs` (T3/T4).
- [STACKTRACE.md](../../STACKTRACE.md) — the `stack_trace()` surface reused for the dump.
- [TESTING.md](../../TESTING.md) — `LOFT_LOG`/`crash_tail` presets the dump builds on.
