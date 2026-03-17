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

### N10 — Fix remaining native codegen failures

**Current state** (after N1–N7): 51 compile, 45 pass, 6 fail, 34 skip of 85 files.

The 6 runtime failures and 34 compile failures have distinct root causes.  Each
sub-step below fixes one root cause and is independently testable.

---

#### N10a — Fix `output_init` to register ALL intermediate types

**Problem:** `output_init` (generation.rs:273–318) skips intermediate type
registrations.  The compile-time type IDs are sequential across ALL definitions
with `known_type != u16::MAX`, but `output_init` only emits types matching:
`DefType::Struct || DefType::Enum || DefType::Vector || (EnumValue with attrs)`.

This skips:
- Plain `EnumValue` variants without attributes (like `Start`, `Ongoing`)
- `DefType::Type` entries (byte/short field types created by `db.byte()`)
- Anonymous vector types created as struct fields

**Symptoms:**
- `enums_types`: "index out of bounds: the len is 20 but the index is 20"
- `enums_enum_field`: "Unknown record 1150964204" (garbage from wrong type layout)

**Root cause detail:** The compile-time `fill_database` (`src/typedef.rs:135–232`)
assigns `known_type` via `database.structure()`, `database.enumerate()`, etc. to
every definition in order.  The runtime must register types in exactly the same
order.  When `output_init` skips a type, all subsequent type IDs shift down.

**Fix (generation.rs `output_init`):**
1. Collect ALL definitions with `known_type != u16::MAX` into `type_defs` — remove
   the `def_type` filter at line 281–285.
2. Sort by `known_type` (already done at line 290).
3. For each type, dispatch on `def_type`:
   - `Struct` → `db.structure(name, 0)` + fields (existing code)
   - `EnumValue` with attrs → `db.structure(name, enum_value)` + fields (existing)
   - `EnumValue` without attrs → skip (no runtime registration needed — the parent
     Enum's `db.value()` already created the slot)
   - `Enum` → `db.enumerate(name)` + `db.value()` per variant (existing)
   - `Vector` → `db.vector(content_type)` (existing)
   - `Type` → check if it's a byte/short type; emit `db.byte(min, nullable)` or
     `db.short(min, nullable)` or skip (field-types are registered implicitly by
     their parent struct's `db.field()` call)

The key insight: `DefType::Type` entries with `known_type != u16::MAX` represent
standalone byte/short types (like the text type = 5).  They must be registered
with `db.byte()` or `db.short()` so their type ID is consumed.  Compare with
`typedef.rs:173–195` which handles `Parts::Byte` and `Parts::Short`.

**Files:** `src/generation.rs` (`output_init`, lines 273–318)
**Test:** `enums_types` and `enums_enum_field` pass
**Verify:** `grep -c 'db\.' tests/generated/enums_types.rs` registration count
matches compile-time types: `cargo test --test expressions -- enums_types` then
count db.structure + db.enumerate + db.vector + db.byte calls in the generated file

---

#### N10b — Fix `output_set` for DbRef deep copy

**Problem:** `Set(var_b, Var(var_a))` where both are `Type::Reference` emits
`var_b = var_a` — a pointer copy.  Both variables then share the same database
record.  Modifying one modifies the other.

**Symptom:** `objects_independent_strings`: "hello world" instead of "hello" —
modifying `b.name` also changes `a.name` because they share the same record.

**Root cause detail:** The bytecode codegen (`src/state/codegen.rs:405–423`)
detects same-type reference assignment in `generate_set` and synthesises a
`Value::Call(OpCopyRecord, [Var(src), Var(dst), Int(tp_nr)])`.  The `generation.rs`
`output_set` does not perform this synthesis — it emits a plain `var_b = var_a`.

**Fix (generation.rs `output_set`, after line 997):**
After emitting the assignment, check if:
1. Variable type is `Type::Reference(d_nr, _)`
2. RHS is `Value::Var(src_var)` where src_var has the same reference type
3. RHS is NOT `Value::Null`

If all three hold, emit an `OpCopyRecord` call:
```rust
// In output_set, after the regular assignment emission:
if let Type::Reference(d_nr, _) = variables.tp(var) {
    if let Value::Var(src) = to {
        if let Type::Reference(_, _) = variables.tp(*src) {
            let tp_nr = self.data.def(*d_nr).known_type;
            writeln!(w)?;
            self.indent(w)?;
            write!(w, "OpCopyRecord(stores, var_{src_name}, var_{name}, {tp_nr}_i32)")?;
        }
    }
}
```

The `tp_nr` comes from `data.def(d_nr).known_type` where `d_nr` is the struct
definition number from the `Type::Reference(d_nr, _)`.

**Files:** `src/generation.rs` (`output_set`, lines 967–1014)
**Test:** `objects_independent_strings` passes

---

#### N10c — Fix `OpFormatDatabase` for struct-enum variants

**Problem:** `OpFormatDatabase` outputs only the enum type name (e.g. "Call")
instead of the full struct representation ("Call {function:\"foo\",parameters:2}").

**Symptom:** `enums_define_enum`: 'Call != "Call {function:\"foo\",parameters:2}"'

**Root cause detail:** `ShowDb::write` (`src/database/format.rs:295–349`) handles
struct-enum variants by reading the discriminator byte from the record to determine
the variant, then dispatching to `write_struct()` for the variant's fields.  This
works correctly — the issue is in how `output_call` passes the type to
`OpFormatDatabase`.

The bytecode interpreter's `format_db` (`src/state/io.rs:301–317`) reads `db_tp`
from bytecode and passes it as `known_type` to `ShowDb`.  The `known_type` must
be the PARENT enum type (e.g. the `Val` enum containing `A` and `B` variants),
not a specific variant.  `ShowDb` then reads the discriminator to pick the variant.

Check what the generated code passes — if `output_call`'s `OpFormatDatabase`
handler passes the variant type instead of the parent enum type, the format will
only show the variant name without struct fields.

**Fix (src/generation.rs or src/codegen_runtime.rs):**
1. In `output_call`'s `OpFormatDatabase` handler, verify the `tp_val` argument
   is the parent enum's `known_type`, not a variant's.
2. If the IR passes the wrong type, fix the `output_call` handler to look up
   the parent enum type from the definition.
3. If the IR passes the correct type but `ShowDb` doesn't recurse into variant
   fields, the bug is in `ShowDb::write` — check `Parts::Enum` handling at
   format.rs:328–349.

**Debug approach:** Compare the `db_tp` value passed by the bytecode interpreter
vs the generated code by adding a `eprintln!("OpFormatDatabase db_tp={db_tp}")` in
both `codegen_runtime::OpFormatDatabase` and `State::format_db`.

**Files:** `src/codegen_runtime.rs` and/or `src/generation.rs`
**Test:** `enums_define_enum` and `enums_general_json` pass

---

#### N10d — Fix null DbRef handling in vector operations

**Problem:** `vectors_fill_result` panics with "Unknown record 2147483648" (`u32::MAX`).

**Symptom:** `vectors_fill_result`: "Unknown record 2147483648"

**Root cause detail:** `stores.null()` (`src/database/allocation.rs:103–105`) calls
`self.database(u32::MAX)` which allocates a store but returns `DbRef { store_nr, rec: 0, pos: 0 }`.
The `store_nr` is a real store index (not 0).  The null DbRef is passed to
`n_fill(stores, var_result)` by value.  Inside `n_fill`:
1. `vector::clear_vector(&var_result, &mut stores.allocations)` is called
2. `var_result.rec == 0` but `store_nr` points to a real store
3. `clear_vector` tries to access the store and hits an invalid record

The bytecode interpreter avoids this because the variable sits on the stack and
`OpDatabase` modifies it in-place before `clear_vector` runs.  In generated code,
`OpDatabase` returns a new DbRef (assigned to `var_result`), but `clear_vector`
runs BEFORE `OpDatabase` in the generated sequence.

**Fix (src/codegen_runtime.rs and/or src/generation.rs):**

Option A — Guard `clear_vector` calls:
In generated code, add a null check before `clear_vector`:
```rust
if var_result.rec != 0 { vector::clear_vector(&var_result, &mut stores.allocations); }
```
This requires detecting `OpClearVector` in `output_call` and wrapping it.

Option B — Fix `stores.null()` return value:
Return `DbRef { store_nr: u16::MAX, rec: 0, pos: 0 }` as the sentinel.
The `u16::MAX` store_nr is already used by `OpNullRefSentinel` and guards in
`Stores::free/valid` already check for it.  However, this changes `Stores::null()`
behaviour which could affect the interpreter.

Option C — Reorder in generated code:
Ensure `OpDatabase` runs before `clear_vector`.  Check the IR ordering and whether
`output_code_inner` preserves statement order correctly.

**Recommended:** Option A — minimal, codegen-only change, no interpreter impact.

**Files:** `src/generation.rs` (`output_call` for `OpClearVector`)
**Test:** `vectors_fill_result` passes

---

#### N10e — Fix remaining 34 compile failures

After N10a–N10d fix the 6 runtime failures, the 34 compile failures remain.

| Category | Count | Sub-step |
|----------|-------|----------|
| Mismatched types (`()` for missing else) | 16 | N10e-1 |
| `if`/`else` incompatible types | 4 | N10e-1 |
| `OpIterate` / `OpStep` / `Keys` not found | 3 | N10e-2 |
| `OpFormatFloat` / `OpFormatStackLong` | 2 | N10e-3 |
| Empty pre-eval (`let _pre = ;`) | 2 | N10e-5 |
| `crate::state::STRING_NULL` reference | 2 | N10e-4 |
| Double borrow of `stores` | 1 | N10e-5 |
| Wrong argument count for `OpGetRecord` | 1 | N10e-5 |
| `prefix _pre14 is unknown` | 1 | N10e-5 |

---

**N10e-1: Fix `output_if` for missing else branches (fixes ~20 files)**

**Location:** `src/generation.rs` `output_if` (lines 828–862) and
`output_code_inner` (line 747: `Value::Null => write!(w, "()")`)

**Problem:** When `false_v` is `Value::Null`, the if-expression emits `()` for the
else branch.  If the true branch produces a value (e.g. `i32`, `&str`), Rust
reports "mismatched types: expected i32, found ()".

**Current code path:** `output_if` at line 856 calls `output_code_inner(w, false_v)`
which hits `Value::Null => write!(w, "()")` at line 747.

**Fix approach:** `output_if` does not receive type information.  The type must be
inferred from the true branch.  Two options:

Option A (simpler): Add a helper `fn infer_if_type(&self, true_v: &Value) -> Option<Type>`
that inspects the true branch to determine its result type.  Then in `output_if`,
when `false_v` is `Value::Null` and `infer_if_type` returns a non-void type, emit
a typed null instead of `()`:

```rust
// In output_if, when false_v is Value::Null and true branch returns a value:
match inferred_type {
    Type::Integer(_, _) => write!(w, "{{ i32::MIN }}")?,
    Type::Long => write!(w, "{{ i64::MIN }}")?,
    Type::Float => write!(w, "{{ f64::NAN }}")?,
    Type::Single => write!(w, "{{ f32::NAN }}")?,
    Type::Boolean => write!(w, "{{ false }}")?,
    Type::Text(_) => write!(w, "{{ \"\" }}")?,
    Type::Reference(_, _) => write!(w, "{{ stores.null() }}")?,
    Type::Enum(_, false, _) => write!(w, "{{ 255_u8 }}")?,
    _ => write!(w, "{{ () }}")?,
}
```

Option B: Track the expected result type through the `output_code_inner` recursion
by adding a `result_type: Option<&Type>` parameter.  More invasive but cleaner.

**Recommended:** Option A — `infer_if_type` can inspect:
- `Value::Call(d, _)` → `data.def(d).returned`
- `Value::Var(v)` → `variables.tp(v)`
- `Value::Int(_)` → `Type::Integer(...)`
- `Value::Block(bl)` → `bl.result`

**Files:** `src/generation.rs`
**Test:** 20 files that currently fail with "mismatched types" or "if/else incompatible"

---

**N10e-2: Add `OpIterate`/`OpStep` + `Value::Iter` handler (fixes 3 files)**

**Problem:** Iterator operations are complex bytecode sequences.  The generated
code currently falls through to debug output for `Value::Iter`.

**Reference implementation:**
- `iterate()`: `src/state/io.rs:373–446` — reads `on: u8` (flags), `arg: u16`
  (field ref), `keys: Vec<Key>`, `from_key`/`till_key`, stack values `from`/`till`,
  then dispatches on collection type (1=index/tree, 2=sorted/vector, 3=ordered)
  to compute `(start, finish)` position markers.
- `step()`: `src/state/io.rs:473–570` — reads current position from state variable,
  advances to next element via `tree::next()`/`vector::vector_step()`, signals
  loop end with `u32::MAX` sentinel.

**Codegen_runtime signatures:**
```rust
/// Returns (start_pos, finish_pos) for the iteration range.
pub fn OpIterate(
    stores: &Stores,
    data: DbRef,       // collection reference
    on: u8,            // flags: bits 0-5=type, bit 6=reverse, bit 7=exclusive
    arg: u16,          // field type reference
    keys: &[Key],      // sort/index key definitions
    from: &[Content],  // start key values
    till: &[Content],  // end key values
) -> (u32, u32)

/// Advances iterator; returns next element DbRef or None if done.
pub fn OpStep(
    stores: &Stores,
    cur: &mut u32,     // current position (mutated in-place)
    finish: u32,       // end sentinel from OpIterate
    data: DbRef,       // collection reference
    on: u8,            // same flags as OpIterate
    arg: u16,          // field type reference
) -> DbRef             // next element (rec=0 when done)
```

**Value::Iter handler in `output_code_inner`:**
`Value::Iter(var_nr, create, step, extra_init)` should emit:
```rust
{
    <extra_init>;
    let (mut _iter_pos, _iter_end) = { <create> };
    loop {
        let var_<name> = { <step> };
        if var_<name>.rec == 0 { break; }
        // loop body follows in the enclosing Block
    }
}
```

The `create` sub-expression is a `Value::Call(OpIterate, ...)`.
The `step` sub-expression is a `Value::Call(OpStep, ...)`.
The loop body is NOT inside the Iter — it follows in the parent Block.

**Files:** `src/generation.rs` (`output_code_inner`), `src/codegen_runtime.rs`
**Test:** 3 files with iterator operations compile and pass

---

**N10e-3: Add `OpFormatFloat`/`OpFormatStackLong` handlers (fixes 2 files)**

**Problem:** Format operations for float and long values are not handled in
`output_call`, so they're emitted as function calls to non-existent functions.

**Reference implementation:** `src/ops.rs:518–586`
```rust
pub fn format_long(s: &mut String, val: i64, radix: u8, width: i32, token: u8, plus: bool, note: bool)
pub fn format_float(s: &mut String, val: f64, width: i32, precision: i32)
pub fn format_single(s: &mut String, val: f32, width: i32, precision: i32)
```

These are already public in `loft::ops`.  The bytecode versions
(`src/state/text.rs:351–391`) read parameters from bytecode + stack and call
these `ops` functions.

**Fix:** Add special-case handlers in `output_call` that emit direct calls to
`ops::format_long` / `ops::format_float`:

```rust
"OpFormatLong" | "OpFormatStackLong" => {
    // Already handled by self.format_long(w, vals) — verify it works
}
"OpFormatFloat" | "OpFormatStackFloat" => {
    if let [ref work_var, ref val, ref width, ref precision] = vals[..] {
        write!(w, "ops::format_float(&mut ")?;
        // emit work_var as mutable String ref
        // emit val, width, precision
        write!(w, ")")?;
    }
    return Ok(());
}
```

Check whether `OpFormatLong` is already handled (line 1028: `"OpFormatLong" => return self.format_long(w, vals)`).  If so, only `OpFormatFloat` /
`OpFormatStackFloat` need new handlers.

**Files:** `src/generation.rs` (`output_call`)
**Test:** 2 files with float/long formatting compile

---

**N10e-4: Fix `crate::state::STRING_NULL` reference (fixes 2 files)**

**Problem:** The `#rust` template for `OpConvBoolFromText` contains:
```
@v1 != crate::state::STRING_NULL
```
In the bytecode interpreter (`fill.rs`), this resolves because `crate` = the
`loft` crate.  In generated standalone `.rs` files, `crate` refers to the
generated file itself — not the `loft` crate.

**`STRING_NULL` definition:** `src/state/mod.rs:24`:
```rust
pub const STRING_NULL: &str = "\0";
```

**Fix:** In `output_call_template` (generation.rs, after the `s.database.` → `stores.`
substitution at line 1102), add:
```rust
res = res.replace("crate::state::", "loft::state::");
```

This handles any `crate::` reference in templates that should point to the `loft`
crate in generated code.

**Files:** `src/generation.rs` (`output_call_template`, ~line 1103)
**Test:** 2 files with `crate::state::` references compile

---

**N10e-5: Fix empty pre-eval, prefix, and argument count issues (fixes 3 files)**

**Problem 1 — Empty pre-eval:** `collect_pre_evals` (`src/generation.rs:601–655`)
can produce a pre-eval binding where the expression buffer is empty:
`let _pre19 = ;` — a syntax error.

**Root cause:** `rewrite_code` (line 659) calls `generate_expr_buf(arg)` which
for certain `Value::Null` or void expressions returns an empty string.

**Fix:** In `output_code_with_subst` or `rewrite_code`, skip emitting a pre-eval
binding when the expression is empty or when `generate_expr_buf` returns `"()"`.

**Problem 2 — Prefix `_pre14`:** Rust edition 2021+ treats `_pre14` as a prefix
token (like `b"..."` or `r"..."`), causing parse errors in some contexts.

**Fix:** Change the pre-eval naming from `_pre{counter}` to `_pre_{counter}`
(underscore separator).  In `collect_pre_evals_inner` at lines 615 and 640:
```rust
let name = format!("_pre_{}", self.counter);
```

**Problem 3 — Wrong argument count for `OpGetRecord`:** The generated code
passes inline key values as separate arguments, but the `codegen_runtime`
function expects a `&[Content]` slice.

**Fix:** In `output_call`, add a handler for `OpGetRecord` that collects
the key arguments into a `vec![...]` literal before calling the runtime function.

**Files:** `src/generation.rs`
**Test:** 3 remaining files compile

---

## Dependency Graph

```
N1–N7 (done) ── 51 compile, 45 pass

N10a (output_init types) ──── fixes enums_types, enums_enum_field
N10b (DbRef deep copy) ────── fixes objects_independent_strings
N10c (FormatDatabase enum) ── fixes enums_define_enum, enums_general_json
N10d (null DbRef guard) ───── fixes vectors_fill_result
N10e-1 (output_if typed null) ── fixes 20 compile failures
N10e-2 (OpIterate/OpStep) ───── fixes 3 compile failures
N10e-3 (OpFormatFloat/Long) ─── fixes 2 compile failures
N10e-4 (crate::state:: fix) ─── fixes 2 compile failures
N10e-5 (pre-eval/prefix) ────── fixes 3 compile failures
                                ── all 85 files compile and pass
```

N10a–N10d fix the 6 runtime failures (independent of each other).
N10e-1 is the highest-impact compile fix (20 files).
N10e-2–N10e-5 fix the remaining 10 compile failures.

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
