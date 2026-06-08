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

**Slice B — DONE (2026-06-07; persists any value type).** `ReplSession`
(`src/repl.rs`) gives cross-input *variable* persistence: a variable bound in one
input is visible to the next (`x = 1` then `x + 1`), depends on earlier ones
(`b = a * 2`), and rebinds (`n = n + 100`).  Built and first tested for integers,
but the mechanism is type-agnostic — text, struct, and vector bindings persist
too (verified in `tests/repl_session.rs`).

Mechanism: the session keeps the stdlib-loaded parser + the accumulated
*bindings* as source.  A binding is recorded but **not executed** — an unused
binding's slot is elided by the allocator, so compiling it would panic; its value
is realised when a later input *observes* it.  An observing input (expression /
call) is wrapped in one shared-scope fn over all bindings, run with a fresh
`State` per input (sidesteps the @P381 CONST_STORE re-lock).  `scopes::check` runs
between parse and `byte_code` (else locals get no slot).  Re-running the bindings
is correct as long as each RHS is deterministic and side-effect-free (any type):
re-running yields the same value.  A side effect in a binding's RHS would repeat
once per observation — the one real limitation, addressed by REPL.X.

**Error recovery — FIXED (2026-06-07).** Root cause: the lexer's `restart` /
`parse_string` reset the cursor but never cleared its `diagnostics`, and
`Diagnostics::level` is monotonic — so after a parse error, every later
`parse_str` re-`fill`ed the lexer's *stale* errors into the parser, and the
session rejected clean input (a typo poisoned the session).  Fix: `parse_str`
now clears the lexer diagnostics at its start (`Lexer::clear_diagnostics` +
`Diagnostics::clear`).  A standalone string parse no longer inherits prior
errors; benefits any repeated `parse_str` user, not just the REPL.
`tests/repl_session.rs::parse_error_leaves_session_usable` passes.

**Remaining toward the general model.** Eliminating the re-run via the true
stack-resident model (persistent `State` + `byte_code_from` + `reset_for_repl`
preserving `stack_pos` + resume-execution) — REPL.X.  (Result-for-display landed
in phase 04; cross-type persistence already works via the re-run model above.)

## REPL.X — eliminating the re-run (designed 2026-06-08, not yet built)

The re-run is structural: the session re-executes the accumulated bindings each
time it observes a value, so a binding whose RHS has a *side effect* repeats it.
A correct fix runs each line **once** and keeps the variable frame alive between
inputs.  Investigation found two viable approaches and one real hazard.

**Hazard (the reason this is not a quick edit).** A frame is not just bytes:
text/`DbRef` locals stored in it need lifetime handling.  The coroutine path
already proves this — `serialise_text_args` + `drop_text_locals_in_bytes` exist
to own text out of a saved frame and to free it without double-dropping.  Any
frame snapshot/restore for the REPL must reuse that handling, so the safe scope
to land *first* is integer-only locals (no text in the frame) — a constraint of
*this* preserved-frame approach, not of today's re-run model, which already
persists every type.

**Approach A — checkpoint / restore / resume (keeps all types).**  Persistent
`State`; the session is one growing `fn`.  After running through statement N,
checkpoint `(code_pos, stack-frame bytes, stack_pos, call_stack)`.  On input
N+1: append it, re-`byte_code` (codegen of the unchanged prefix 1..N is
deterministic, so the checkpoint `code_pos` stays valid), restore the frame, run
from the checkpoint → only N+1 executes.  Foundation: the coroutine stack-bytes
snapshot (`coroutine_create` copies from the stack store via `store.addr`).
Risk: prefix-stability + const-store/text-local corners.

**Approach B — function params + value capture (integer-scoped first).**  Each
input is a `fn f(<prior vars>) -> <new var> { … }`; the REPL stores variable
*values*, passes them as args, captures the return.  Each fn runs once → no
re-run.  Needs a typed-arg/return execute entry (push args, read the return) —
which also unblocks `:vars` and in-process result-as-`String`.  Marshalling
beyond integers (text `DbRef`, structs) is the follow-on.

**Recommendation.** Build B first for integers (bounded, reuses the
native-call arg/return marshalling, and dividends: `:vars` + result return),
then A for the all-types, no-recompile endpoint.  Either is a focused spike on
the execution core — land it deliberately, not bundled with unrelated work.

---

## Convergence — REPL.X, auto-resume, and persistence are one design

*(evaluated 2026-06-08)*

Three open REPL problems share a single fix: **make bindings store-resident
records instead of replayed source.**

- **REPL.X (no re-run):** if a binding's value lives in a store, observing it
  reads the store — the RHS never re-executes, so side effects don't repeat.
- **Auto-resume (REPL.S):** the session heap is then just stores, and stores
  already persist (below).
- **Exact restart:** because nothing re-executes on restore, every computed
  value returns *verbatim* — including non-deterministic ones (`random()`,
  `now()`).  Text-replay cannot do this: it re-runs the generators and draws new
  values.  (The generator's *forward* state is deliberately not restored — see
  "RNG" below.)

**Why not "mmap the stack".**  `State` (src/state/mod.rs:111) is not a flat
buffer: it holds `HashMap`s, `Arc`, `Vec<CallFrame>`, `coroutines: Vec<Box<…>>`,
and a raw `data_ptr: *const Data`.  Restoring that at a new base address would
dangle, and the stack is transient — nothing lives on it between inputs.

**Why the stores DO mmap.**  A `Store` is a word-addressed buffer whose pointers
are logical `DbRef{store_nr, rec, pos}` (src/keys.rs:202), not native addresses,
so the bytes are position-independent and survive mmap-restore at any base
(src/store.rs:23).  Stores are already mmap-backed with CRC + corruption
rejection (`file: Option<MmapStorage>`, src/store.rs:119).  And the save/load
already ships for the stdlib: `Bundle { data, types }` serialized into a store
(src/data_store.rs:406), "save the stdlib to a `.store` file, load it back
(mmap, no re-parse)" (src/ir_read.rs:1287), keyed + invalidated by a content
hash (src/cache.rs:181).  Session resume = that startup-cache mechanism applied
to the user's session store.

**RNG — values are stored, generator state is deliberately not.**  Drawn random
values sit in the session store like any value, so they restore exactly.  The
PCG generator state lives in the `random` cdylib (the single source of RNG state
for both backends — see the src/ops.rs comment), not in a store, and is **not**
snapshotted: restoring saved RNG state would make the stream reproducible from
the session image (predict/replay future `random()` outputs — a security
hazard).  On resume the generator continues fresh (re-seeded from entropy, as on
any launch); reproducible streams stay an explicit-seed opt-in (`random_seed`).
Declined — [DESIGN_DECISIONS.md § C72](../../DESIGN_DECISIONS.md#c72--repl-session-resume-does-not-persist-rng-generator-state).

**What the store-resident model still needs** (beyond the mmap, which is built):

1. A store-resident **binding environment** (name → `DbRef`/scalar); today the
   name→value map is regenerated by replaying `body`.
2. **Scalars at rest** (`x = 5`) boxed into the store (or a tiny text residue).
3. **Schema-version gating** — stamp the image with the cache key so a loft
   upgrade rejects a stale image and falls back to fresh (infra exists).
4. Not portable/shareable (endian + layout + binary specific) — fine for a local
   session, not for sharing.

Items 1–2 are exactly Approach A/B's hard part.  So **do not build the
store-resident model for resume alone — that is over-engineering.**  Ship
text-replay auto-resume first (portable, upgrade-proof, fault-tolerant).  Build
the store-resident image *when* you build REPL.X: then one design collapses all
three problems and reuses the startup cache.

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
