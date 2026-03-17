// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Native Rust Code Generation

Plan for making the existing Rust code generation backend (`src/generation.rs`) produce
compilable, runnable code. The generated code must produce the same results as the
bytecode interpreter for every loft program.

---

## Goals

### Primary goal
Make `src/generation.rs` produce Rust source files that compile and run correctly —
producing identical output to the bytecode interpreter for every loft program.

### Interpreter safety invariant
The bytecode interpreter is the production execution engine.  **Every step in this
plan must leave it fully functional.**  Concretely:

1. **`cargo test` must pass after every commit.** All 400+ existing tests exercise
   the bytecode interpreter.  A red test means the interpreter is broken.
2. **Never modify `src/fill.rs` or `src/state/` for native codegen purposes.**
   These files are the interpreter core.  Native codegen is a parallel backend,
   not a replacement.
3. **`default/01_code.loft` templates are shared.**  The `#rust` annotations are
   read by both `src/create.rs` (bytecode → fill.rs) and `src/generation.rs`
   (native codegen).  Any template change must be validated against both paths:
   - `create.rs` applies `stores.` → `s.database.` before writing fill.rs
   - `generation.rs` must apply `s.database.` → `stores.` (the inverse) when
     emitting native code
   - Templates that already say `s.database.*` pass through create.rs unchanged
     and **must not be changed to `stores.*`** — that would break fill.rs
4. **New files only.**  Steps that add code (N3: `codegen_runtime.rs`, N6: compile
   test, N7: CLI flag) create new files or add `pub mod` lines.  They do not
   modify interpreter logic.
5. **Test both backends after template changes.**  After any change to
   `default/01_code.loft`:
   - Run `cargo test` (validates bytecode interpreter)
   - Run `make gtest` or equivalent (regenerates fill.rs; confirms templates
     still produce valid operator code)

### Verification checklist (run after every N-step)
```bash
cargo test                       # all interpreter tests pass
cargo clippy -- -D warnings      # no new warnings
cargo fmt -- --check             # formatted
```

---

## Current State

`src/generation.rs` already translates the loft IR tree into Rust source files. The test
framework (`tests/testing.rs`) writes 87 files to `tests/generated/` on every test run.
**None of these files currently compile** — there are ~1500 errors across 6 root causes.

### Error Breakdown

| # | Root Cause | Errors | Affected Files |
|---|-----------|--------|----------------|
| 1 | `external::op_*` in `#rust` templates — module doesn't exist in generated code | ~490 | 41 |
| 2 | `u32::from(i32)` — const params emitted as `i32` but templates wrap in `u32::from()` | ~476 | 50+ |
| 3 | `n_assert` and stdlib functions not in scope — test files only get user definitions | ~92 | 40+ |
| 4 | `s.database.*` leaked from bytecode templates — no `s` variable in generated code | ~53 | 10 |
| 5 | Database ops (`OpNewRecord`, `OpDatabase`, `OpFreeRef`, etc.) have no `#rust` body | ~260 | 41 |
| 6 | `Value::Iter` / `Value::Keys` not handled in `output_code_inner()` | ~11 | 5 |

### Architecture

The generated code uses these loft library types (already public):
- `loft::database::Stores` — runtime data store
- `loft::keys::{DbRef, Str, Key, Content}` — reference and string types
- `loft::ops` — pure scalar operations (arithmetic, conversions)
- `loft::vector` — vector operations

Each generated file contains:
1. An `init(db: &mut Stores)` function that registers all type schemas
2. Rust functions for each loft function, receiving `stores: &mut Stores` as first arg
3. A `#[test]` wrapper that calls `init()` then the test function

---

## Steps

### N1 — Fix `#rust` templates for generated code

Three search-and-replace fixes in `default/01_code.loft`, batchable into a single commit:

**1a. `external::` → `ops::`**
`#rust` templates use `external::op_add_int(...)` etc. The `external` module doesn't exist
in generated code — only `ops` is imported. Two renames needed:
- `external::op_min_single_int(@v1)` → `ops::op_negate_int(@v1)`
- `external::op_min_single_long(@v1)` → `ops::op_negate_long(@v1)`

All other `external::op_*` names match `ops::op_*` exactly.

**1b. `u32::from(@fld)` → `((@fld) as u32)`**
Const parameters are emitted as `i32` literals but templates wrap in `u32::from()`.
Rust has no `u32: From<i32>` impl. Field offsets are always non-negative, so `as u32` is safe.

**1c. `s.database.*` → `stores.*` in generation.rs (NOT in templates)**
Some templates reference `s.database.allocations`, `s.database.enum_val()`, etc.
In generated code there is no `s` — only `stores: &mut Stores`.  However, these
patterns **must stay unchanged in `default/01_code.loft`** because `create.rs` needs
them for fill.rs (the bytecode interpreter).  The fix goes in `src/generation.rs`:
add `res = res.replace("s.database.", "stores.");` in the template substitution path
(the inverse of what create.rs does with `stores.` → `s.database.`).

**Files:** `src/generation.rs` (template substitution), possibly `src/database/mod.rs` (make methods pub)
**Verify:** `grep -c 'external::\|u32::from\|s\.database' tests/generated/*.rs` returns 0
**Eliminates:** ~1019 errors
**Interpreter safety:** Templates unchanged; fill.rs unaffected

---

### N2 — Include stdlib in each generated test file

**Problem:** `tests/testing.rs` calls `output_native(w, start, def_nr)` for test files,
where `start` skips all default-library definitions. Standard library functions like
`n_assert` are in `[0, start)` and only written to `tests/generated/default.rs`. Individual
test files cannot find them.

**Fix:** Change `tests/testing.rs` to pass `(0, def_nr)` instead of `(start, def_nr)` to
`output_native()` for test files. Each file becomes self-contained.

**Files:** `tests/testing.rs` lines 232–235
**Verify:** `grep -c 'fn n_assert' tests/generated/expressions_*.rs` finds definitions
**Eliminates:** ~92 errors; ~41 simple files compile after N1+N2

---

### N3 — Add `codegen_runtime` module for database operations

**Problem:** Database operations are bytecode opcodes with no `#rust` template. The code
generator emits them as function calls (`OpNewRecord(...)`, `OpDatabase(...)`) but no such
functions exist in generated code. These can't be simple templates because they involve
complex multi-step interactions with `Stores`.

**Fix:** Create `src/codegen_runtime.rs` with wrapper functions that replicate what the
bytecode interpreter does for each operation:

| Function | Reference in | Purpose |
|----------|-------------|---------|
| `op_database(stores, tp) -> DbRef` | `src/state/io.rs` | Allocate database root record |
| `op_new_record(stores, parent, tp, fld) -> DbRef` | `src/state/io.rs` | Create struct element |
| `op_finish_record(stores, parent, rec, tp, fld)` | `src/state/io.rs` | Finalize record (insert into collection) |
| `op_free_ref(stores, v)` | `src/fill.rs` | Free a reference |
| `op_get_record(stores, db, tp, keys) -> DbRef` | `src/state/io.rs` | Look up record in collection |
| `op_format_database(stores, ...) -> String` | `src/state/debug.rs` | Format record for display |
| `op_conv_text_from_null() -> Str` | `src/fill.rs` | Null text constant |

Register the module: add `pub mod codegen_runtime;` to `src/lib.rs`.
Add `use loft::codegen_runtime::*;` to the generated preamble in `src/generation.rs`.
Update `output_call()` in `generation.rs` to emit these function names for the
corresponding `Op*` definitions.

**Files:** new `src/codegen_runtime.rs`, `src/lib.rs`, `src/generation.rs`
**Eliminates:** ~260 errors

---

### N4 — Handle `Value::Iter` and `Value::Keys` in code generation

**Problem:** `output_code_inner()` has no match arms for `Value::Iter` and `Value::Keys`.
They fall through to the `_ => write!(w, "{code:?}")` debug fallback.

**Fix:** Add match arms:
- `Value::Iter(var, create, next, extra_init)` — emit a Rust loop calling
  `codegen_runtime::op_iterate()` / `codegen_runtime::op_step()`
- `Value::Keys(keys)` — emit a key array literal `vec![Key { ... }, ...]`

Also add `op_iterate()` and `op_step()` to `codegen_runtime.rs`.

**Files:** `src/generation.rs`, `src/codegen_runtime.rs`
**Depends on:** N3
**Eliminates:** ~11 errors

---

### N5 — Skip or fix empty native function bodies

**Problem:** Functions like `OpConvTextFromNull`, `OpLengthCharacter`, operator functions
with a return type but no `#rust` body are emitted as `fn name() -> T {}` — missing the
return expression.

**Fix:** In `output_function()`, skip emitting functions that are:
- Operators with a `#rust` template (these are inlined at call sites, not called directly)
- Native functions with no IR body (registered via `FUNCTIONS` table in `native.rs`)

For operators that genuinely need a `#rust` template but don't have one, add the template
to `default/01_code.loft`.

**Files:** `src/generation.rs`, `default/01_code.loft`
**Eliminates:** remaining ~50 errors; all files compile

---

### N6 — Add compilation gate test

**Problem:** No CI protection against regressions in generated code quality.

**Fix:** Add a test that runs `rustc` on a representative generated file and asserts
it compiles without errors. This prevents future changes from breaking the code generator.

**Files:** new test in `tests/` or addition to `tests/testing.rs`

---

### N7 — Add `--native` CLI flag

**Problem:** No user-facing way to generate and run native code.

**Fix:** Add `--native <file.loft>` to `src/main.rs`:
1. Parse and compile the loft program (same as normal)
2. Generate a Rust source file via `Output::output_native()`
3. Compile with `rustc` (linking against the loft crate)
4. Run the resulting binary

**Files:** `src/main.rs`
**Depends on:** N1–N6

---

## Dependency Graph

```
N1 (template fixes) ── N2 (stdlib) ──── ~41 simple files compile
                        N3 (codegen_runtime) ── most files compile
                        N4 (Iter/Keys) ──────── iterator files compile
                        N5 (empty bodies) ────── all files compile
                        N6 (compile gate) ────── CI protection
                        N7 (--native flag) ───── user feature
```

N1 is search-and-replace in one file. N2 is a one-line change.
N3 is the largest new code (~200 lines).

---

## Critical Files

| File | Role |
|------|------|
| `default/01_code.loft` | All `#rust` templates (N1, N5) |
| `src/generation.rs` | Code emitter (N3–N5) |
| `tests/testing.rs:220–242` | Where generated files are written (N2) |
| `src/fill.rs` | Reference implementations for all 234 opcodes |
| `src/state/io.rs` | Reference for `OpDatabase`, `OpNewRecord`, etc. |
| `src/ops.rs` | Pure operations — already imported by generated code |
| `src/codegen_runtime.rs` | New runtime module (N3) |

---

## Verification

After each step:
1. `cargo test` — existing tests must still pass (bytecode interpreter unaffected)
2. Count remaining compilation errors:
   ```bash
   for f in tests/generated/*.rs; do
     rustc --edition 2024 --crate-type lib "$f" \
       -L target/debug/deps --extern loft=target/debug/libloft.rlib 2>&1
   done | grep "^error\[" | wc -l
   ```
3. After N6: CI gate prevents regressions

---

## See also
- [COMPILER.md](COMPILER.md) — Compiler pipeline: lexer, parser, IR, bytecode
- [INTERMEDIATE.md](INTERMEDIATE.md) — IR Value tree structure
- [DESIGN.md](DESIGN.md) — Algorithm analysis for major subsystems
- [DATABASE.md](DATABASE.md) — Runtime data store and type schema
