<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 03 — State reset + bytecode append

**Status: open.**

## Revised design (2026-06-07) — supersedes the bytecode-append plan below

Two findings collapse this phase well below its original MH estimate:

1. **Appendability already exists.** `compile::byte_code_from(state, data,
   start_d_nr, …)` emits bytecode only for definitions `>= start_d_nr` and skips
   the one-time init — so a new statement's code is appended with no
   `Arc<Vec<u8>>` → `Vec<u8>` refactor.  The whole "Option A/B" section below is
   moot.

2. **Variables live on the stack; keep them there.**  The REPL session is one
   long-lived, growing frame.  Its variable table is shaped exactly like a
   normal function's (`vars: Function` — names + types at fixed slots from the
   frame base), so the **existing expression codegen + execution read the slots
   for free** — no session struct, no field-type inference, no `s.x` identifier
   rewriting.

   - A reference to a prior variable compiles to an ordinary load from its slot.
   - **Defining a new variable is just the normal expression result** landing in
     the next free slot; the parser allocates that slot and records the
     name→slot/type in the persistent `vars`.
   - The values persist on the stack across inputs; `reset_for_repl` resets only
     `code_pos` / `call_stack`, never the variable region.

So the work is: (a) a persistent function-shaped `vars` + preserved stack frame,
seeded into the parse of each input; (b) `byte_code_from` to emit the new
statement's code; (c) `reset_for_repl`; (d) execute from the new code offset.
The load-bearing claim to spike first: a statement compiled against a
pre-seeded, function-shaped `vars` reads/writes the right slots and the values
survive a reset-and-re-enter.

**Slice A — DONE (2026-06-07).** The parse → compile → execute pipeline composes
end-to-end: `parse_statement` → `compile::byte_code` → `execute_argv` on the
phase-02 wrapper.  A definition from one input is callable from a later input
(functions persist in `Data`); bare expressions and stdlib calls run.
`tests/repl_eval.rs`.  Slice A uses a fresh `State` + full compile per input
(correct, not yet optimized — a fresh State sidesteps the @P381 CONST_STORE
re-lock).

**Slice B — DONE for integers (2026-06-07).** `ReplSession` (`src/repl.rs`)
gives cross-input *integer-variable* persistence: a variable bound in one input
is visible to the next (`x = 1` then `x + 1`), depends on earlier ones
(`b = a * 2`), and rebinds (`n = n + 100`).  `tests/repl_session.rs`.

Mechanism (integer scope): the session keeps the stdlib-loaded parser + the
accumulated *bindings* as source.  A binding is recorded but **not executed** —
an unused binding's slot is elided by the allocator, so compiling it would panic;
its value is realised when a later input *observes* it.  An observing input
(expression / call) is wrapped in one shared-scope fn over all bindings, run with
a fresh `State` per input (sidesteps the @P381 CONST_STORE re-lock).  `scopes::check`
runs between parse and `byte_code` (else locals get no slot).  Correct for pure
integer arithmetic (re-running deterministic bindings yields the same value).

**Error recovery — FIXED (2026-06-07).** Root cause: the lexer's `restart` /
`parse_string` reset the cursor but never cleared its `diagnostics`, and
`Diagnostics::level` is monotonic — so after a parse error, every later
`parse_str` re-`fill`ed the lexer's *stale* errors into the parser, and the
session rejected clean input (a typo poisoned the session).  Fix: `parse_str`
now clears the lexer diagnostics at its start (`Lexer::clear_diagnostics` +
`Diagnostics::clear`).  A standalone string parse no longer inherits prior
errors; benefits any repeated `parse_str` user, not just the REPL.
`tests/repl_session.rs::parse_error_leaves_session_usable` passes.

**Remaining toward the general model.** Non-integer variable types; eliminating
the re-run via the true stack-resident model (persistent `State` + `byte_code_from`
+ `reset_for_repl` preserving `stack_pos` + resume-execution); surfacing the
evaluated result for display (phase 04).

---

## Original design (bytecode-append — NOT pursued; kept for context)

## Goal

Make `State` re-runnable across REPL inputs:

- Bytecode is **appendable**: each new statement adds bytes to the
  existing `bytecode` buffer; previously-emitted code remains
  valid (offsets and jumps stay correct).
- State has a **`reset_for_repl()`** method that clears
  per-execution state (stack, code-pos, call-stack) without
  losing `database`, `bytecode`, `const_refs`, or
  `string_from_const_store`.
- Worker / par execution paths keep working — the bytecode shared
  to par workers must still be safely accessible after appends.

## Design

### Bytecode appendability

Today: `state.bytecode: Arc<Vec<u8>>` is built once via
`compile::byte_code()` and is immutable.  Par workers
`Arc::clone()` it for read-only access.

REPL: each `parse_statement` produces an additional bytecode
segment for the new `__repl_N` synthetic fn (and possibly bytecode
for new top-level fns if the input defined any).

Two implementation options:

#### Option A — Single growable Vec (no Arc)

Replace `Arc<Vec<u8>>` with `Vec<u8>` (or `Arc<Mutex<Vec<u8>>>`).
`compile::byte_code()` becomes incremental: each call appends to
the existing buffer.

**Pro**: minimal touch; the existing bytecode emission machinery
already pushes to a `Vec<u8>` internally.
**Con**: par workers currently `Arc::clone()` the bytecode for
zero-copy sharing.  Switching to `Mutex<Vec<u8>>` adds lock
overhead per opcode read; switching to `Vec<u8>` requires the
worker to clone the whole buffer.  REPL is single-threaded, so
appears acceptable, but par calls inside REPL would need the
worker to snapshot the buffer at par-start.

#### Option B — Per-statement segments

Replace the single buffer with `Vec<Arc<[u8]>>` segments, indexed
by entry-point d_nr.  Each statement emits its own segment.
`code_pos` becomes `(segment_id, offset)` — invasive for every
opcode emitter.

**Pro**: zero-copy par sharing per segment; segments never grow.
**Con**: substantial refactor of every jump / call / opcode handler.

### Recommendation

**Option A with per-call snapshot for par.**  The REPL's primary
use-case is single-threaded interactive eval; par calls inside the
REPL are rare and can tolerate a buffer clone at par-start.

Concrete change:
- `State::bytecode: Vec<u8>` (no Arc).
- `compile::byte_code()` appends; existing callers (file mode)
  call once and never append.
- Par-worker dispatch (`src/parallel.rs::WorkerProgram`) clones
  the buffer at construction.  Single allocation per par call,
  bounded by program size — not per-row.

### `State::reset_for_repl()`

```rust
impl State {
    pub fn reset_for_repl(&mut self) {
        // Clear per-execution state.
        self.stack_pos = 0;
        self.code_pos = 0;
        self.def_pos = 0;
        self.call_stack.clear();
        self.eval_stack.clear();
        // Preserve: database, bytecode, const_refs,
        // string_from_const_store, fn_positions, types.
    }
}
```

The REPL calls `reset_for_repl()` after each input runs to
completion (success or runtime error).  Database isn't reset —
the user's defined values stay alive.

### Const-refs across statements

`State::const_refs` is a `Vec<DbRef>` (each entry an interned
literal).  Each new statement may add entries.  `reset_for_repl`
preserves the existing entries; `compile::byte_code()` appends
new ones.

`string_from_const_store` follows the same rule.

### Fn-position registry

`State::fn_positions: Vec<u32>` maps `d_nr → bytecode offset`.
Today filled once via `compile::byte_code()`.  REPL: extend with
new entries for each `__repl_N` synthetic and any user-defined fn
in the input.

### Worker bytecode safety

`WorkerProgram::bytecode` (`src/parallel.rs`) currently
`Arc::clone()`s the State's bytecode.  After option A, the worker
gets `state.bytecode.clone()` — a fresh `Vec<u8>` per par call.
Worker bytecode lives for the duration of the call; no append
during execution.

## Implementation outline

| Step | Files | Effort |
|------|-------|--------|
| 1. `State::bytecode` field type change `Arc<Vec<u8>>` → `Vec<u8>` | `src/state/mod.rs` | XS |
| 2. Update every read site (`*self.code::<T>()` etc.) | `src/state/mod.rs`, `src/state/text.rs`, `src/state/io.rs`, `src/codegen_runtime.rs` | S |
| 3. `compile::byte_code()` appends | `src/compile.rs` | XS |
| 4. `WorkerProgram::new` clones bytecode | `src/parallel.rs` | XS |
| 5. `State::reset_for_repl` + tests | `src/state/mod.rs` | XS |
| 6. Const-ref / fn-position append paths | `src/state/mod.rs`, `src/compile.rs` | S |
| 7. Round-trip test: parse statement, execute, parse another, execute, verify state | `tests/repl_state.rs` (new) | S |

## Tests

### Single statement → execute → reset → another statement

```rust
#[test]
fn repl_state_round_trip() {
    let mut p = Parser::new();
    p.parse_dir("default", true, false).unwrap();
    let mut state = State::new(p.database.clone());

    // First statement: x = 42
    let r1 = p.parse_statement("x = 42");
    let entry1 = match r1 { ParseResult::Ready { entry_def_nr } => entry_def_nr, _ => panic!() };
    compile::byte_code(&mut state, &p.data);
    state.execute_at_def(entry1, &p.data);
    state.reset_for_repl();

    // Second statement: y = x + 1
    let r2 = p.parse_statement("y = x + 1");
    let entry2 = match r2 { ParseResult::Ready { entry_def_nr } => entry_def_nr, _ => panic!() };
    compile::byte_code(&mut state, &p.data);   // appends
    state.execute_at_def(entry2, &p.data);

    // Verify __repl_session.y == 43.
    let session = state.database.lookup_repl_session();
    assert_eq!(session.y, 43);
}
```

### Par-call inside REPL session

A regression test that defines a fn using `par(...)` then invokes
it from a REPL input, verifying the worker bytecode clone path
works.

## Acceptance criteria

1. Each `parse_statement` + `compile::byte_code` + `execute`
   cycle leaves `state.database` mutated as expected, with all
   prior state intact.
2. `state.reset_for_repl()` returns the State to a "ready for
   next call" shape with stack / code-pos / call-stack zeroed
   and database / bytecode / const-refs preserved.
3. Par calls from REPL inputs succeed (worker clones bytecode at
   par-start; no segfault from buffer-relocation under append).
4. File-mode execution (`cargo run --bin loft -- file.loft`)
   keeps working unchanged — `compile::byte_code` is called once
   and no `reset_for_repl` happens.
5. Full test suite green.

## Effort

**MH (~3–4 days).**  Step 2 (every bytecode read site) is the bulk —
changing `Arc<Vec<u8>>` to `Vec<u8>` ripples across the runtime.
Step 6 (const-ref append) needs care: the const-ref allocator
shouldn't reset between statements but new entries must register
correctly.

## Risk

- **Worker bytecode invalidation under append.**  If a par call
  is in flight and the main thread appends bytecode, the worker's
  `Arc<...>` was a snapshot — but if we switch to `Vec<u8>` the
  worker holds its own clone, immune to the append.  Verify via
  a stress test (par call in flight while another thread appends).
- **Memory growth** — never-resetting bytecode grows with every
  REPL input.  Mitigation: phase 03 doesn't garbage-collect
  bytecode; if memory becomes a problem, a `:reset` REPL command
  (phase 04) wipes the buffer.
- **Stack-position drift** — `reset_for_repl` zeros `stack_pos`
  but the database may still hold DbRefs into freed stack slots.
  Mitigation: after reset, verify no DbRef in the database has
  `store_nr == 0` (the stack store) before next call.  Same
  invariant the test runner uses today.

## Out of scope

- **GC of orphan bytecode segments** — when a `__repl_N`'s
  symbolic name is reassigned, the old segment is unreachable but
  the bytes stay.  Acceptable given session lifetimes.
- **Bytecode persistence across REPL launches** — start fresh each
  time.  Phase 06 may add session save/load.

## See also

- [00-baseline.md](00-baseline.md) — bytecode + state survey.
- [02-statement-parser.md](02-statement-parser.md) — produces the
  IR this phase compiles + executes.
- [04-repl-shell.md](04-repl-shell.md) — drives this phase from
  user input.
- `src/state/mod.rs` — `State` struct + bytecode field.
- `src/compile.rs::byte_code` — current single-shot compiler.
- `src/parallel.rs::WorkerProgram` — par worker bytecode handle.
