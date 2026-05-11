<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4 — Typed runtime errors

Status: in-progress (foundation + 2/9 sites shipped 2026-05-11).
Steps 4.1, 4.2, 4.11 (panic builtin), 4.13 (assert) landed in this
session.  Remaining: 4.3-4.10 (div, mod, vector/text index, null
deref, narrow cast), 4.12 (stack overflow), 4.14 (backtrace
capture), 4.15 (renderer frame list), 4.16 (bench gate).

Foundation shape:

- `RuntimeError { kind, position, op_pc, message }` and
  `RuntimeErrorKind` enum live in `src/runtime_error.rs`.
- `Stores::runtime_error: Option<Box<RuntimeError>>` field carries
  the error across the State/native boundary (natives only see
  `&mut Stores`, not `&mut State`).
- `State::execute_argv`'s dispatch loop checks
  `database.runtime_error.is_some()` after each op and short-
  circuits via `code_pos = u32::MAX`; `State::resume` mirrors
  the check for post-yield panic / assert.
- `main.rs` renders captured errors through the existing phase-2
  `render_entry_pretty` and exits 1 via the existing
  `had_fatal` check (production-mode path unchanged: logs +
  continues; only DEV mode now produces a typed error).
- The in-process test harness (`tests/testing.rs`) re-raises
  `runtime_error` as a Rust panic after `state.execute(...)`
  returns so `#[should_panic]` fixtures keep firing.

Site conversions shipped:

- `n_panic` (`src/native.rs`): replaces Rust panic with
  `RuntimeError::user_panic`.
- `n_assert` (`src/native.rs`): replaces Rust panic on failed
  assertion with `RuntimeError::assertion_failed`.

End-to-end output now reads:
```
error: panic: boom!
  --> /tmp/p_panic.loft:2:1
  |
2 |   panic("boom!");
  | ^
```

instead of the legacy Rust panic frame.

## Goal

Replace the implicit "panic = bug; sentinel = user error" coin-flip
with an explicit `RuntimeError { kind, position, op_pc }` raised at
a small set of well-known fault sites.  Today these sites either:

- Return `i64::MIN` (the null sentinel) — a silent wrong answer
  unless the result is consumed by `??`.  See
  `src/ops.rs:305::op_div_long` returning `i64::MIN` on `v2 == 0`.
- Panic with no source-level context — see the implicit panics
  inside C-coercion paths.

After phase 4, every fault attributable to user code is a
`RuntimeError` with a source position and a stable kind.  Anything
*not* attributable to user code stays a hard panic and is treated
as an interpreter bug.

## Decision 04.A — sentinel, panic, or RuntimeError per site

Every fault site picks one of three behaviours:

| Behaviour | When | Example |
|---|---|---|
| **Sentinel (`null`)** | Op feeds the LHS of `??` (the C54.G-hybrid nullable opcode is emitted) | `attack / armour ?? 0` |
| **`RuntimeError`** | Op stands alone or feeds something other than `??` | `let dmg = attack / armour` (armour=0) |
| **Panic** | Interpreter invariant broken, never user-induced | bytecode pc out of range |

Codegen already chooses between non-nullable and nullable opcodes
based on whether `??` is the next operation (see `OpDivIntNullable`
at `src/fill.rs:517`).  Phase 4 keeps that decision but changes the
non-nullable failure mode from "sentinel + downstream silent
wrong-answer" to "raise `RuntimeError`".

## Steps

### 4a — `RuntimeError` type + propagation

```rust
#[derive(Debug)]
pub struct RuntimeError {
    pub kind: RuntimeErrorKind,
    pub position: Option<Position>,    // resolved via Data::source_at_pc
    pub op_pc: u32,
    pub fn_d_nr: u32,
    pub message: String,
}

#[derive(Debug, Clone)]
pub enum RuntimeErrorKind {
    DivideByZero,                       // `/`, `%`
    IndexOutOfBounds { idx: i64, len: u32 },
    NegativeIndex { idx: i64 },
    NullDereference,                    // null DbRef field/method access
    NarrowCastOverflow { value: i64, target: &'static str },
    StackOverflow,                      // recursion depth
    UserPanic { message: String },      // the `panic("…")` builtin
    AssertionFailed { message: String },
}
```

Propagation strategy: the interpreter's main loop (`src/state/mod.rs:1549`)
gets a per-thread `Cell<Option<RuntimeError>>` field on `State`.  A
fault-site opcode sets it and writes `u32::MAX` to `self.code_pos`,
which the existing loop terminator (`if self.code_pos == u32::MAX
{ break; }` at line 1607) already honours.

After the loop exits, `execute()` checks the cell.  If non-empty,
it bubbles up.  Top-level (`main.rs`) prints via the phase-2
renderer.  Inner `execute_at_*` callers propagate the error — most
are already infallible by signature; phase 4 widens them to
`Result<…, RuntimeError>` only where they're called from native
context (`#rust ""` annotations).

### 4b — Site list (every fault becomes typed)

Audit from phase 0's `0a-sites.md`.  Concrete site set:

| Site | File | Today | After phase 4 |
|---|---|---|---|
| Integer `/` | `src/ops.rs:305` | returns `i64::MIN` on v2=0 | `RuntimeError::DivideByZero` (non-nullable opcode); sentinel kept for nullable variant |
| Integer `%` | `src/ops.rs:315` | as above | as above |
| Float `/` | `src/fill.rs:886` | returns `f64::INFINITY`/`NaN` (Rust default) | `RuntimeError::DivideByZero` if v2=0 (non-nullable); IEEE behaviour kept on nullable |
| Vector index `v[i]` | `src/fill.rs::vec_get_*` | unchecked read or panic | `RuntimeError::IndexOutOfBounds` / `NegativeIndex` |
| Text index `t[i]` | `src/fill.rs::text_at` | as above | as above |
| Field access on null | `src/fill.rs::field_read_*` | unchecked deref → SIGSEGV | `RuntimeError::NullDereference` |
| Narrow cast | `src/native_utils.rs::narrow_int_cast` | wrap or sentinel | `RuntimeError::NarrowCastOverflow` (non-nullable variant) |
| `panic("msg")` builtin | `src/fill.rs:1675` | `panic!` | `RuntimeError::UserPanic` |
| Stack-overflow trap | `src/state/mod.rs` recursion guard | `panic!("Too many operations")` | `RuntimeError::StackOverflow` |
| Assertion (test stdlib) | `default/01_code.loft::assert` | `panic("…")` | `RuntimeError::AssertionFailed` (subkind of UserPanic) |

Out of scope (stays a panic — interpreter bug if it ever fires):
- Bytecode `pc` out of range
- Stack underflow
- Type-tag mismatch on a `Value` that codegen should have resolved
- `unreachable!` in second-pass walkers

### 4c — Per-site refactor pattern

Each site converts in three lines:

```rust
// Before:
fn div_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    let new_value = ops::op_div_int(v_v1, v_v2);    // returns i64::MIN on v2=0
    s.put_stack(new_value);
}

// After:
fn div_int(s: &mut State) {
    let v_v2 = *s.get_stack::<i64>();
    let v_v1 = *s.get_stack::<i64>();
    if v_v2 == 0 {
        s.raise(RuntimeErrorKind::DivideByZero);
        return;
    }
    s.put_stack(ops::op_div_int_checked(v_v1, v_v2));
}
```

`State::raise` populates the error cell, sets `code_pos = u32::MAX`,
and (importantly) does *not* write a result to the stack — the
caller's stack is in an indeterminate state, but execution is about
to end so it does not matter.

The nullable opcode (`OpDivIntNullable`) is unchanged — codegen
selects it when `??` is next, and the sentinel + `??` flow keeps
working.  Phase 4 only changes non-nullable behaviour.

### 4d — Stack-trace capture

`RuntimeError::position` covers the originating site.  But for a
user debugging a deep call chain, "divide by zero at
math.loft:12:8" is less useful than "divide by zero at
math.loft:12:8, called from game.loft:88:14, called from
main.loft:5:3".

Phase 4d snapshots `State::call_stack` into the error and exposes
it through the renderer:

```rust
pub struct RuntimeError {
    …
    pub backtrace: Vec<StackFrame>,    // most-recent-first
}

pub struct StackFrame {
    pub fn_d_nr: u32,
    pub fn_name: String,
    pub op_pc: u32,
    pub source: Option<Position>,      // resolved at capture time
}
```

The renderer prints up to 8 frames; user can set `LOFT_BT=full` to
get all of them.  The data is the same shape as
[STACKTRACE.md](../../STACKTRACE.md)'s user-callable
`stack_trace()` — phase 4d intentionally aligns with that design so
both sources reuse one type.

### 4e — Tests

- The 9 runtime cases from phase 0 (cases 17-25) are converted to
  `RuntimeError`.  The `.expect` files swap from "panic at …" to
  "error: divide by zero …".  Goldens regenerated.
- New `tests/runtime_errors.rs` for each kind: assert the error
  fires, the kind matches, the position matches, the backtrace
  contains the expected frames.
- `tests/issues.rs` and other suites that check for specific
  panic messages: re-baseline.  This is the only set of test
  changes outside `tests/error_messages/` — record in 4e's PR.
- `make bench` re-run.  The fault-check branch is added to every
  div / index / cast — needs measurement.  Bound: ≤ 3 %
  regression.  If breached, the check moves to a cold-out-of-line
  path (`#[cold]` + outlined fault-fn).

## Atomic landing sequence

Each row converts one fault site (or one infrastructure piece).
The site conversions can land in any order — they don't depend on
each other beyond the infrastructure (4.1).

| # | Step | Test |
|---|---|---|
| 4.1 | Add `RuntimeError`, `RuntimeErrorKind`; add `Cell<Option<RuntimeError>>` field to `State`; add `State::raise(kind)` helper that populates the cell and sets `code_pos = u32::MAX` | Unit test: `state.raise(DivideByZero)`; assert `state.error.is_some()`, kind matches, `code_pos == u32::MAX` |
| 4.2 | `execute()` checks the error cell after the dispatch loop and returns it; widen the few `execute_at_*` callers that propagate to native | Unit test: hand-build a tiny bytecode that calls `raise`, assert `execute()` returns the error |
| 4.3 | Convert `div_int` (`fill.rs:482`) to raise on `v2 == 0`; the nullable variant unchanged | Fixture case 17 (`runtime_div_by_zero_int.loft`): assert `kind == DivideByZero`, position = expected line/col |
| 4.4 | Convert `rem_int` (`fill.rs:489`) | Fixture case 19 (`runtime_mod_by_zero.loft`) |
| 4.5 | Convert `div_float` and `div_single` | Fixture case 18 (`runtime_div_by_zero_float.loft`) |
| 4.6 | Convert vector-index ops (positive OOB) | Fixture case 20: `kind == IndexOutOfBounds { idx, len }`, both fields assert |
| 4.7 | Convert vector-index ops (negative idx) | Fixture case 22: `kind == NegativeIndex { idx }` |
| 4.8 | Convert text-index ops | Fixture case 21 |
| 4.9 | Convert null-DbRef field/method access | Fixture case 23: `kind == NullDereference`, position points at the `.` token |
| 4.10 | Convert `narrow_int_cast` overflow (non-nullable variant) | Fixture case 24: `kind == NarrowCastOverflow { value, target }` |
| 4.11 | Convert `panic("…")` builtin (`fill.rs:1675`) → `UserPanic { message }` | Fixture case 25: kind matches, message is the user's literal |
| 4.12 | Convert recursion / stack-overflow trap (`state/mod.rs:1605`) → `StackOverflow` | Fixture cases 26, 27 |
| 4.13 | Convert `assert(...)` stdlib (`default/01_code.loft`) → `AssertionFailed` | New fixture: failed assertion produces typed kind |
| 4.14 | Capture `State::call_stack` into `RuntimeError.backtrace` at `raise` time (resolve each frame's source via phase 3's `Data::source_at_pc`) | Multi-frame fixture: `main → battle → divide_by_zero`, assert backtrace has 3 frames in order with correct positions |
| 4.15 | Render `RuntimeError` through phase-2's pretty renderer (kind label + position + caret + top-3 frames + "(use `LOFT_BT=full` for more)" if more) | Golden output for cases 17-27; `LOFT_BT=full` golden for case 26 |
| 4.16 | Re-run `make bench`; outline fault-check branches with `#[cold]` if hot | Bench delta ≤ 3 % vs phase 3 gates merge |

## Acceptance

- All 9 phase-0 runtime cases now emit `RuntimeError`-rendered
  output, not panics.
- `tests/runtime_errors.rs` covers every kind in
  `RuntimeErrorKind`.
- The only `panic!` reachable from valid loft code is the
  `panic("…")` builtin (which becomes `RuntimeError::UserPanic`).
- `make bench` ≤ 3 % regression vs phase 3.
- `make ci` green.
- `doc/claude/PROBLEMS.md` entries about silent div-by-zero
  sentinels are closed (or moved to "design — null sentinel kept
  for `??` only").

## Risks

| Risk | Mitigation |
|---|---|
| Per-op fault check slows hot loops | Use `#[cold]` outlined fault paths and `#[inline(always)]` happy-path checks.  Measure on `bench/01_classic`. |
| Existing tests that depended on `i64::MIN` propagating silently break | These tests were relying on undocumented behaviour; convert them to `??`-explicit.  Audit done in phase 0 already (case 17-25 in baseline).  Any *other* test that breaks here means a real semantic change — investigate per-test, do not paper over. |
| `RuntimeError` propagation requires touching every native fn | Most natives never raise; only the documented site-list does.  Other natives keep their `fn (&mut State)` signature.  The error cell is on `State` so propagation is implicit through control flow. |
| Backtrace capture allocates per error | Errors are rare; allocation is acceptable.  Cap at the most recent 32 frames to bound memory if recursion is deep. |
