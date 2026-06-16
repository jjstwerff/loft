<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 3 — pc → source-line map

Status: shipped (2026-05-03).  Core machinery (pc → Position table,
publish-to-thread-local snapshot, panic-hook source-loc print) had
already landed during @PLAN09 work.  Phase 3's missing piece — the
SIGSEGV / SIGABRT / SIGBUS handler emitting the source location —
landed in this session along with regression tests covering the
lookup semantics.

The plan's 3a (compact `SourceSpanTable` + `PositionInterner`) was
skipped — `BTreeMap<u32, Position>` + `Arc<...>` snapshot is fine
for the panic-only access pattern, and the per-op overhead the
plan was guarding against (3e) doesn't apply: nothing reads the
map per-op.  Lookup happens only on a fault.

3d (LOFT_LOG=crash_tail prefix) was deferred — low value (the
trace already prints op names + pcs) and would require either
per-op formatting cost or significant log-routing surgery.

## Goal

At runtime, given a bytecode `pc`, the interpreter can answer "which
loft source line caused this opcode to be emitted?".  The map feeds
phase 4's `RuntimeError` (so the message says
`game.loft:88:14`, not `pc 0x12ab`) and `src/crash_report.rs`
(so a SIGSEGV / SIGABRT prints the originating loft source line
along with the op-name it already shows).

## Input from phase 1

Phase 1e populates `Definition.source_spans: HashMap<u32 /* pc */,
Position>` for every fault-prone IR site.  Phase 3 turns that into a
fast lookup and threads it through the runtime crash paths.

## Steps

### 3a — Compact representation

`HashMap` is wrong for runtime lookup: a panicking opcode is rare
but when it fires we want the lookup in microseconds and we want
the data to live alongside the bytecode for cache locality.

Replacement structure on `Definition`:

```rust
pub struct SourceSpanTable {
    pcs: Vec<u32>,           // sorted ascending; one entry per fault-prone op
    spans: Vec<PositionId>,  // parallel array; PositionId is an interned u32
}

impl SourceSpanTable {
    /// Return the span of the op whose pc is the largest pc ≤ `pc`.
    /// Used by the crash printer — we only know the panicking op's
    /// pc, not which fault-prone *range* it falls within.
    pub fn lookup(&self, pc: u32) -> Option<PositionId> { … }
}
```

`PositionId` indexes into a process-wide `PositionInterner` — most
ops on the same source line share a position, so de-duplication
saves memory and bytes-on-cache.

Building the table: phase 1e already records `(pc, Position)` as
codegen visits `Value::Span`; phase 3a sorts by pc at the end of
`generate` and converts to the parallel-arrays form.

### 3b — Lookup helper on `Data`

```rust
impl Data {
    pub fn source_at_pc(&self, pc: u32) -> Option<Position> {
        let fn_d_nr = self.fn_d_nr_for_pos(pc)?;
        let def = self.def(fn_d_nr);
        let id = def.source_spans.lookup(pc)?;
        Some(self.position_interner.get(id).clone())
    }
}
```

`fn_d_nr_for_pos` already exists for the debug-assertions
infinite-loop trap (`src/state/mod.rs:1591`).  Phase 3 lifts it out
of the `cfg(debug_assertions)` block and makes it production code.

### 3c — Wire into crash printer (signal handler path)

`src/crash_report.rs::set_context` receives `(pc, op, op_name,
fn_d_nr, fn_name)` per opcode.  The signal handler is
async-signal-safe — it cannot call back into `Data` to look up a
position.

Solution: phase 3c extends the published `Ctx` with an optional
pre-formatted source location string, computed once per opcode by
the dispatch loop **outside** the signal handler:

```rust
struct Ctx {
    pc: u32,
    fn_d_nr: u32,
    op_code: u8,
    op_name: &'static str,
    fn_name: &'static str,
    source_loc: [u8; 96],   // "file.loft:88:14\0" or "" if no span
    source_loc_len: u8,
}
```

In `state/mod.rs:1559` the dispatch loop already calls
`set_context(...)` once per op; phase 3c adds a span lookup right
before the call:

```rust
let span = data.source_at_pc(op_pos_rt);
let loc_buf = format_into(&mut buf, span);  // writes "file:line:col"
crate::crash_report::set_context(op_pos_rt, op, op_name, fn_d_nr, fn_name, &buf[..len]);
```

The lookup is an O(log N) binary search per op — measured cost in
3e.  The signal handler then only has to copy the pre-formatted
buffer; no `Data` access.

### 3d — Wire into normal panic path

When a Rust `panic!` fires in `fill.rs` or anywhere in the runtime,
the panic hook (installed by `main.rs`) reads `LAST_CTX` and
appends:

```
loft: panic at game.loft:88:14
      in fn n_run_battle (op #42 OpDivInt at pc 0x12ab)
```

This is the human-readable path.  Source-line resolution is allowed
to allocate here because we are inside Rust's panic infrastructure,
not a signal handler.

The hook is installed in `src/main.rs` and re-uses
`crash_report::LAST_CTX` plus a global `Arc<Data>` set by the
runtime at startup (already needed for the existing
infinite-loop printer).

### 3e — Cost measurement

The dispatch loop runs the lookup once per op.  Phase 0d's
`bench/01_classic` runs ~3e8 ops; if the lookup costs 10 ns it adds
3 seconds to the bench.  Mitigations to evaluate in 3e:

1. **Skip when no span exists for this fn.**  A definition with
   `source_spans.pcs.is_empty()` (e.g. native fns) writes an empty
   `source_loc` once and reuses it.  Branch is predictable.
2. **Cache the last hit.**  90 % of consecutive ops share a
   source line; remember the last `(pc_lo, pc_hi, PositionId)`
   span and skip the binary search on a hit.
3. **Compile-time toggle.**  Behind `--release` we can elide the
   per-op span publish when `LOFT_ERRORS=compact` (phase 2's env);
   the source-line lookup runs only when a panic actually fires.
   The crash printer then walks `Data` itself (not signal-safe, but
   it's already inside Rust's panic hook).

The default lands on (1) + (2).  (3) is a fallback if bench drift
exceeds 5 %.

### 3f — Tests

- `tests/error_messages.rs` adds a runtime-error subset (cases
  17-25 from phase 0, the `runtime_*` ones).  They still panic in
  phase 3 — phase 4 turns them into `RuntimeError`.  But the panic
  message now includes `at <file>:<line>:<col>`; the goldens lock
  that in.
- `tests/crash_report.rs` (existing) gains an assertion that the
  published `Ctx.source_loc` is non-empty after any opcode dispatch
  in a non-empty user fn.
- `make bench` re-run; comparison appended to `0d-bench.txt` under
  "phase 3".  Bound: ≤ 5 % regression on `bench/01_classic` and
  `bench/11_par`.

## Atomic landing sequence

| # | Step | Test |
|---|---|---|
| 3.1 | Add `PositionInterner` (intern `Position` → `PositionId`, lookup back) | Unit test: intern same Position twice, assert same id; `get(id)` returns equal Position; intern N distinct, assert N ids |
| 3.2 | Add `SourceSpanTable { pcs: Vec<u32>, spans: Vec<PositionId> }` with `lookup(pc) -> largest pc ≤` semantics | Unit test: build from `[(5, A), (10, B), (15, C)]`; assert `lookup(5) == A`, `lookup(7) == A`, `lookup(10) == B`, `lookup(20) == C`, `lookup(0) == None` |
| 3.3 | Build `SourceSpanTable` from `Definition.source_spans` at end of codegen | Unit test: hand-crafted `Definition` with two spans, codegen + build, assert table contents match insertion |
| 3.4 | Lift `fn_d_nr_for_pos` out of `cfg(debug_assertions)` and add `Data::source_at_pc(pc)` | Unit test: synthetic `Data` with two fns at known pc ranges, assert pc in fn1's range returns fn1's span and not fn2's |
| 3.5 | Extend `crash_report::Ctx` with `source_loc: [u8; 96]` + `source_loc_len: u8`; update `set_context` setter signature | Unit test: `set_context(...)` writes loc, `LAST_CTX.get()` returns it byte-for-byte |
| 3.6 | Add `LAST_HIT_SPAN` per-thread cache; format source-loc string in dispatch loop before `set_context` | Unit test runs a tight loop over a single fault-prone op, asserts cache hit rate ≥ 90 % via a debug counter |
| 3.7 | Install Rust panic hook in `main.rs` that reads `LAST_CTX` and prints `loft: panic at file:line:col in fn …` | Integration test: trigger `panic!` from a known fault site, capture stderr, assert it contains `at file:line:col` |
| 3.8 | Update `LOFT_LOG=crash_tail:N` formatter to prefix each line with the source loc | Integration test: enable `crash_tail:5`, force a div-by-zero (will panic until phase 4), assert tail lines start with `file:line:col` |
| 3.9 | Re-run `make bench`; append to `0d-bench.txt` heading "phase 3" | Bench delta ≤ 5 % vs phase 1 gates merge |

## Acceptance

- `Data::source_at_pc(pc)` returns the originating `Position` for
  every fault-prone opcode.
- SIGSEGV / SIGABRT printer includes `at file:line:col`.
- Rust panics in `fill.rs` print `at file:line:col`.
- `LOFT_LOG=crash_tail:N` output gains a leading source-line line.
- `make bench` ≤ 5 % regression vs phase 1.
- `make ci` green.

## Risks

| Risk | Mitigation |
|---|---|
| Per-op span lookup dominates the dispatch loop | 3e mitigation 1 + 2 (skip empty fns, cache last hit).  Fallback: compile-time toggle. |
| Signal handler reads partially-written `Ctx` | `Ctx` already uses `Cell<>` (single-writer per thread) and the source-loc bytes are fixed-size; the worst case is a torn formatted string, which is harmless and self-evidently truncated. |
| `PositionInterner` becomes a contention point under threads | Interning happens at codegen time (single-threaded); runtime is read-only.  No contention. |
| Native-codegen path (`--native`) bypasses the dispatch loop | Phase 3 covers interpreter only.  `--native` source-line mapping is plan-`NATIVE_DEBUG.md`'s job (DWARF / source maps).  Phase 7's CHANGELOG notes the limitation. |
