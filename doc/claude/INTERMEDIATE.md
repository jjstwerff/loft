
# loft Intermediate Language (IR) Reference

## Overview

The compiler pipeline is:
```
.loft source -> Parser (Value tree IR) -> State.byte_code() -> bytecode Vec<u8> -> fill::OPERATORS[opcode](&mut State) at runtime
```

The intermediate representation is the `Value` enum tree defined in `src/data.rs`.
Bytecode generation is done in `src/state/codegen.rs` via `State`.
The 233 operator functions are in `src/fill.rs`.
Variable/scope tracking during parsing is in `src/variables/` via `Function`.

---

## Contents
- [Value Enum (IR Nodes) — `src/data.rs`](#value-enum-ir-nodes--srcdatars)
- [Type Enum — `src/data.rs`](#type-enum--srcdatars)
- [AST-Level Operators (Call node op_nr)](#ast-level-operators-call-node-op_nr)
- [Bytecode State — `src/state/`](#bytecode-state--srcstate)
- [Variable Tracking — `src/variables/`](#variable-tracking--srcvariablesrs)
- [233 Bytecode Operators — `src/fill.rs`](#233-bytecode-operators--srcfillrs)
- [DbRef](#dbref)
- [Key Patterns](#key-patterns)
- [Debug Tools (`src/state/debug.rs`, debug builds only)](#debug-tools-srcstatedebugrs-debug-builds-only)

---

## Value Enum (IR Nodes) — `src/data.rs`

```rust
pub enum Value {
    Null,
    Line(u32),                         // Source line annotation
    Int(i32),                          // Integer literal
    Long(i64),                         // 64-bit integer literal
    Single(f32),                       // 32-bit float literal
    Float(f64),                        // 64-bit float literal
    Boolean(bool),                     // Boolean literal
    Enum(u8, u16),                     // Enum variant index + database type id
    Text(String),                      // Text literal
    Call(u32, Vec<Value>),             // Operator/function call: (op_nr, args)
    CallRef(u16, Vec<Value>),          // Call via fn-ref variable: (var_nr, args)
    Block(Box<Block>),                 // Scoped statement sequence
    Insert(Vec<Value>),                // Inline statements (no new scope)
    Var(u16),                          // Read variable at stack position n
    Set(u16, Box<Value>),             // Write variable at stack position n
    Return(Box<Value>),                // Return from function
    Break(u16),                        // Break out of n-th enclosing loop
    Continue(u16),                     // Continue n-th enclosing loop
    If(Box<Value>, Box<Value>, Box<Value>), // cond, then-branch, else-branch
    Loop(Box<Block>),                  // Infinite loop (exit via Break)
    Drop(Box<Value>),                  // Evaluate and discard return value
    Iter(u16, Box<Value>, Box<Value>), // for-loop: (var_nr, init, step)
    Keys(Vec<Key>),                    // Key descriptor for sorted/hash/index
}
```

### Special Var(0)

`Var(0)` is used as a **placeholder** in struct field default expressions to mean
"the current record being initialized." It is replaced with the actual record reference
at object initialization time by `Parser::replace_record_ref()` in `src/parser/expressions.rs`.
In `$` expressions inside struct field defaults, `$` maps to `Value::Var(0)`.

### Block

```rust
pub struct Block {
    pub name: &'static str,   // Debug label
    pub operators: Vec<Value>, // Ordered IR nodes
    pub result: Type,          // Return type
    pub scope: u16,            // Scope nesting level
}
```

---

## Type Enum — `src/data.rs`

```rust
pub enum Type {
    Unknown(u32),         // Forward reference placeholder (linked type id or 0)
    Null,                 // No type / null literal
    Void,                 // Function return: no value
    Integer(i32, u32),    // Range-constrained integer (min, max)
    Boolean,
    Long,                 // i64
    Float,                // f64
    Single,               // f32
    Character,            // Single unicode codepoint
    Text(Vec<u16>),       // Text + dependency variable list (for lifetime tracking)
    Keys,                 // Key spec for collection types
    Enum(u32, bool, Vec<u16>),               // def_nr, is_ref, deps
    Reference(u32, Vec<u16>),                // struct def_nr + deps (nullable)
    RefVar(Box<Type>),                       // Mutable reference argument (&T)
    Vector(Box<Type>, Vec<u16>),             // Dynamic array + deps
    Sorted(u32, Vec<(String, bool)>, Vec<u16>), // Ordered set: def_nr, [(field, asc)]
    Index(u32, Vec<(String, bool)>, Vec<u16>),  // Index: def_nr, [(field, asc)]
    Spacial(u32, Vec<String>, Vec<u16>),        // Spatial index: def_nr, [fields]
    Hash(u32, Vec<String>, Vec<u16>),           // Hash table: def_nr, [fields]
    Routine(u32),                            // Dynamic routine reference
    Iterator(Box<Type>, Box<Type>),          // (yield_type, internal_state_type)
    Function(Vec<Type>, Box<Type>),          // First-class fn: arg types + return type
                                             // Stored as i32 d_nr at runtime (same as integer)
                                             // Variables of this type are callable: f(args)
    Rewritten(Box<Type>),                    // After append rewrite (Text/structs)
}
```

### Integer Storage Size

`Integer(min, max)` is stored compactly based on range:
- range < 256  → 1 byte
- range < 65536 → 2 bytes
- otherwise    → 4 bytes

Nullable integers with range exactly 256 or 65536 also use 1 or 2 bytes respectively.

### Dependency Lists (`Vec<u16>`)

Types carrying `Vec<u16>` track ownership for scope-based freeing.
See `src/data.rs` (Type enum doc) and `src/scopes.rs` (module doc) for
the full semantics.  `Type::RefVar(Box<Type>)` means "stack reference"
— a DbRef into another variable's stack slot; `depend()` delegates to
the inner type.

---

## AST-Level Operators (Call node op_nr)

These are the operator names used in `Call(op_nr, args)` at the AST level,
as listed in `data.rs`:

```
OpAdd   OpMin   OpMul   OpDiv   OpRem   OpPow
OpNot   OpLand  OpLor   OpEor   OpSLeft OpSRight
OpEq    OpNe    OpLt    OpLe    OpGt    OpGe
OpAppend  OpConv  OpCast
```

The actual numeric `op_nr` values are resolved via the operator registry in
`src/parser/mod.rs` during parsing.

---

## Bytecode State — `src/state/`

```rust
pub struct State {
    bytecode: Vec<u8>,          // Main bytecode stream
    text_code: Vec<u8>,         // String constant pool
    stack_cur: DbRef,           // Current stack frame (a DB record in store 1000)
    pub stack_pos: u32,         // Current stack pointer
    pub code_pos: u32,          // Current position in bytecode
    pub database: Stores,       // All data stores
    pub arguments: u16,         // Stack size of function arguments
    pub stack: HashMap<u32, u16>, // code_pos -> stack level (for scoping)
    pub vars: HashMap<u32, u16>,  // code_pos -> variable stack position
    pub calls: HashMap<u32, Vec<u32>>, // code_pos -> called def_nrs
    pub types: HashMap<u32, u16>,      // code_pos -> type id
    pub library: Vec<Call>,            // Extern Rust function table
    pub library_names: HashMap<String, u16>,
    text_positions: BTreeSet<u32>,   // debug: set of absolute positions of live Strings
    line_numbers: HashMap<u32, u32>,
    fn_positions: Vec<u32>,          // d_nr → bytecode entry point (for OpCallRef)
}

pub type Call = fn(&mut Stores, &mut DbRef);
```

The stack is stored as a database record in store 1000 (index 0 in `Stores::allocations`).
`stack_pos` starts at 4 (offset past the 4-byte record header slot reserved for the return address
of `main`). `execute()` immediately pushes `u32::MAX` as the sentinel return address, so on entry
to `main` the effective `stack_pos = 8`.

### Stack Frame Layout

For a function `fn foo(a: T1, b: T2) -> R { ... }`:

```
absolute offset from stack_cur.pos:
  [caller's stack ...]
  [a]        ← size_of(T1) bytes, compile-time position 0
  [b]        ← size_of(T2) bytes, compile-time position size(T1)
  [ret-addr] ← 4 bytes (u32 code_pos), compile-time position = args_size = size(T1)+size(T2)
  [locals]   ← compile-time positions start at args_size+4
```

`State::arguments` records `args_size` after scanning a function's parameter list.

### Variable Position Encoding (`var[N]` in bytecode dumps)

Each variable is accessed via `get_var(encoded_pos)` / `put_var(encoded_pos, val)`.

- **Compile-time alloc position `N`**: the value of `Stack::position` at the moment the variable
  was first pushed/allocated onto the compile-time stack. For a function argument the first bytes
  are pushed at N=0; for locals, N=args_size+4 or higher.
- **`var[N]` in bytecode dumps**: the dump shows `var[N]` where `N = stack_at_instruction - encoded_pos`. `N` is the compile-time allocation position of the variable.
- **Actual encoded value**: `encoded_pos = stack_at_instruction - N`. Stored as a u16 in the bytecode stream after the opcode byte.

Runtime formula for `get_var(encoded_pos)`:
```
absolute_address = stack_cur.pos + runtime_stack_pos - encoded_pos
                 = stack_cur.pos + (Z + compile_stack_pos) - (compile_stack_pos - N)
                 = stack_cur.pos + Z + N
```
where `Z` = absolute stack position at the start of the function's arguments (runtime `stack_pos`
before any args were pushed).

So `var[N]` always resolves to `stack_cur.pos + Z + N`, the variable's fixed absolute address for
the current call frame — regardless of how much has been pushed onto the stack since.

`put_var(encoded_pos, val)` writes to `stack_cur.pos + runtime_stack_pos + size_of::<T>() - encoded_pos`. This differs from `get_var` by `size_of::<T>()`, so callers must account for the size offset.

### `OpDatabase` vs `OpConvRefFromNull`

- `OpConvRefFromNull` → `Stores::null()` → allocates a fresh store with `rec=0`. This store is
  used as the "home" for the struct allocation in `OpDatabase`.
- `OpDatabase(var, db_tp)` → reads the existing DbRef from `var`, calls `clear`+`claim`+`set_default_value` on it, then writes `{store_nr, rec=1, pos=8}` back to `var`. The store it operates on is the one allocated by `ConvRefFromNull`.

### Store LIFO Invariant

`Stores::database()` allocates store `max` and increments `max`. `Stores::free()` just decrements `max` by 1 without verifying `al == max-1`. This means stores **must be freed in exact LIFO (reverse-allocation) order**; out-of-order frees corrupt `max` and will cause subsequent allocations to overwrite a still-live store.

### `text_positions` (debug-only `String` liveness tracker)

Text variables on the stack are stored as Rust `String` objects (`size_of::<String>() = 24 bytes`
on 64-bit). In debug builds, `State` tracks which stack positions hold live `String`s:

- `OpText` (`state.text()`) → inserts `stack_cur.pos + stack_pos` into `text_positions` before advancing `stack_pos`.
- `OpFreeText(pos)` (`state.free_text()`) → computes `stack_cur.pos + stack_pos - pos`, calls `shrink_to(0)` on the `String`, and removes the absolute position from `text_positions`. Asserts it was present (double-free detection).
- `OpFreeStack(value, discard)` (`state.free_stack()`) → after decrementing `stack_pos` by `discard`, asserts that `text_positions` has **no entries** in the discarded range. Violation = "Not freed texts" panic.

**Consequence**: every `String` allocated by `OpText` in a block must be freed by `OpFreeText` before the block's `OpFreeStack` runs — except the block's *return* variable, which must be allocated **outside** the block's stack range (i.e. at an enclosing scope) so its position falls below the discard range.

### `Store::addr` layout

```rust
pub fn addr<T>(&self, rec: u32, fld: u32) -> &T {
    debug_assert!(rec * 8 + fld + size_of::<T>() <= store.size * 8, "out of bounds");
    unsafe { self.ptr.offset(rec as isize * 8 + fld as isize).cast::<T>() … }
}
```

Address = `ptr + rec * 8 + fld`. The factor-of-8 comes from the 8-byte record header
(the size of a single-word `u64` slot reserved per record). For the stack store,
`stack_cur.rec` is whatever `claim(1000)` returned, and `stack_cur.pos = 8` (fixed
by `Stores::database()`). Therefore:

```
absolute byte offset into store's backing buffer
  = stack_cur.rec * 8 + stack_cur.pos + runtime_stack_pos
  = stack_cur.rec * 8 + 8 + runtime_stack_pos
```

In debug builds `addr` and `addr_mut` assert `rec * 8 + fld + size_of::<T>() <= store.size * 8`.  Without this check, a garbage `DbRef` (e.g. from `format_stack_float`'s off-by-4 bug) silently reads invalid memory and causes SIGSEGV rather than a panic with a useful location.

A `DbRef` created by `OpCreateStack` has `{store_nr=stack_cur.store_nr,
rec=stack_cur.rec, pos=stack_cur.pos + var.absolute_position}`. When
`addr::<String>(r.rec, r.pos)` is called on it, the actual byte offset is
`r.rec * 8 + r.pos = stack_cur.rec * 8 + stack_cur.pos + var.absolute_position`.
This is the same formula as `get_var`, so both paths address the same memory.

### Stack text operators: `string_mut` vs `string_ref_mut` vs `GetStackText`

Three closely related helpers deal with text buffers on the stack:

| Helper | Finds the `String` via… | Used by |
|---|---|---|
| `string_mut(pos)` | `stack_cur.pos + stack_pos - pos` (direct — variable IS a `String`) | `OpText`, `OpAppendText`, `OpFormatFloat`, `OpFreeText` |
| `string_ref_mut(pos)` | reads a `DbRef` at `stack_cur.pos + stack_pos - pos`, then follows `(r.rec, r.pos)` | `OpAppendStackText`, `OpFormatStackFloat`, `OpFormatStackInt`, … |
| `GetStackText` | pops a `DbRef` `r` from stack, calls `database.store(&r).addr::<String>(r.rec, r.pos)` | `OpGetStackText` (return of a `RefVar(Text)` function) |

`string_ref_mut` uses `store_mut(&self.stack_cur)` regardless of the DbRef's
`store_nr`, because the DbRef was always created via `OpCreateStack` from the same
stack frame.  `GetStackText` uses `database.store(&r)` which selects the store via
`r.store_nr` — correct only if `r.store_nr == stack_cur.store_nr`, which holds when
the DbRef was created by `OpCreateStack` in the same thread.

**The "Stack" format ops** (`OpAppendStackText`, `OpFormatStackFloat`, etc.) differ
from their plain counterparts (`OpAppendText`, `OpFormatFloat`) in that they expect the
target text buffer to be a `RefVar(Text)` — i.e. a pointer to a `String` stored
elsewhere on the stack — rather than directly being the `String`.  This indirection is
what allows a text-returning function to write into a caller-supplied buffer.

### `generate_var` — reading stack variables by type

`generate_var` (state.rs) emits different opcode sequences depending on the variable's type:

| Type | Opcodes emitted | Result on stack |
|---|---|---|
| `Integer` | `OpVarInt(pos)` | the `i32` value |
| `Long` | `OpVarLong(pos)` | the `i64` value |
| `Float` | `OpVarFloat(pos)` | the `f64` value |
| `Text` (owned) | `OpVarText(pos)` or `OpArgText(pos)` | a `Str` (ptr+len) |
| `Vector` | `OpVarVector(pos)` | the 12-byte `DbRef` pointing to the container record |
| `Reference` / `Enum(true)` | `OpVarRef(pos)` | the 12-byte `DbRef` pointing to the struct record |
| `RefVar(Integer)` | `OpVarRef(pos)` → `OpGetInt(0)` | the referenced `i32` |
| `RefVar(Text)` | `OpVarRef(pos)` → `OpGetStackText` | dereferences the `DbRef` to a `String *` |
| `RefVar(Vector)` | `OpVarRef(pos)` → `OpGetStackRef(0)` | dereferences the `DbRef` to another `DbRef` |

**`RefVar(Vector)` note:** `OpVarRef(pos)` pushes the `DbRef` of the `OpCreateStack`
temp record. `OpGetStackRef(0)` then reads the 12-byte `DbRef` stored at offset 0 in
that temp record, which is the **caller's vector `DbRef`** — the `OpCreateStack`
instruction stores it there when the call is set up. This is the correct result for
`v += extra` (Issue 4 fix): `assign_refvar_vector` in `parser.rs` emits
`OpAppendVector(Var(v_nr), rhs, rec_tp)`; `generate_var` for `Var(v_nr)` uses the
`OpVarRef + OpGetStackRef(0)` path to supply the caller's vector container `DbRef`
to `vector_append`, which modifies the caller's vector in place.

---

## Variable Tracking — `src/variables/`

```rust
pub struct Function {
    pub name: String,
    pub file: String,
    steps: Vec<u8>,          // Byte steps for each variable on stack
    unique: u16,             // Unique name counter
    current_loop: u16,       // Current loop nesting (MAX = top-level)
    loops: Vec<Iterator>,    // Active for-loop contexts
    variables: Vec<Variable>,
    work_text: u16,          // Work variable for text operations
    work_ref: u16,           // Work variable for ref operations
    work_texts: BTreeSet<u16>,
    work_refs: BTreeSet<u16>,
    names: HashMap<String, u16>,  // name -> last variable index
    pub done: bool,
    pub logging: bool,
}

pub struct Variable {
    name: String,
    type_def: Type,
    source: (u32, u32),   // (line, col) of declaration
    scope: u16,           // 0 = function arguments
    stack_pos: u16,       // Position on stack frame (u16::MAX = unassigned)
    first_def: u32,       // Bytecode-sequence position of first assignment (u32::MAX = never)
    last_use: u32,        // Bytecode-sequence position of last read (0 = never)
    uses: u16,            // Reference count
    argument: bool,
    defined: bool,
}
```

Variables are referenced in IR by their `stack_pos` (`u16`).
Scope 0 is always function arguments.
The same variable name may have multiple `Variable` instances across scopes.

### Live Intervals (`first_def` / `last_use`)

`compute_intervals(function, ir)` in `variables/` walks the entire IR tree in sequential
order (assigning each node a monotonically-increasing sequence number `seq`) and fills:

- `first_def` — the sequence number of the `Value::Set(v, …)` node that first defines `v`.
  Critically, the *value expression* inside `Set` is visited **before** `first_def` is
  recorded, so a block-return temporary that lives only inside the RHS expression does not
  create a false overlap with `v`.
- `last_use` — the maximum sequence number of any `Value::Var(v)` reference.

`validate_slots(function)` uses these intervals to detect conflicting stack-slot assignments:
two variables with the same `stack_pos` overlap if their live intervals intersect
(`left.first_def <= right.last_use && right.first_def <= left.last_use`).
Same-name + same-slot pairs are exempt (they represent the sequential-block reuse pattern).
Only owning types (Text, Reference, Vector, owned Enum) are checked; primitive scalars can
safely share slots.

Called (in debug builds) from `state.rs` after `byte_code()` completes, immediately before
`state.execute()`.

---

## 233 Bytecode Operators — `src/fill.rs`

Operators are indexed by their position in the `OPERATORS` array. The array is generated by `src/create.rs::generate_code` from the `#rust "..."` annotations on `Op`-prefixed definitions in the default library. Operator names in the array follow the convention `Op<CamelCase>` → `op_<snake_case>` (e.g. `OpAddInt` → `add_int`). The exception is `OpReturn` → `op_return` (to avoid the Rust keyword).

The index of each operator equals its `op_code` field in `Data::Definition`, which is emitted as a single byte in the bytecode stream and used as an index into `fill::OPERATORS` at runtime.

Categories:

### Control Flow (0–6)
`goto`, `goto_word`, `goto_false`, `goto_false_word`, `call`, `op_return`, `free_stack`

### Boolean (7–12)
`const_true`, `const_false`, `cast_text_from_bool`, `var_bool`, `put_bool`, `not`

### Integer (13–56)
- Constants: `const_int` (4-byte), `const_short` (2-byte), `const_tiny` (1-byte)
- Var/Put: `var_int`, `var_character`, `put_int`, `put_character`
- Conversions: `conv_int_from_null`, `conv_character_from_null`, `cast_int_from_text`, `cast_long_from_text`, `cast_single_from_text`, `cast_float_from_text`, `conv_long_from_int`, `conv_float_from_int`, `conv_single_from_int`, `conv_bool_from_int`
- Math: `abs_int`, `min_single_int`
- Arithmetic: `add_int`, `min_int`, `mul_int`, `div_int`, `rem_int`
- Bitwise: `land_int`, `lor_int`, `eor_int`, `s_left_int`, `s_right_int`
- Comparison: `eq_int`, `ne_int`, `lt_int`, `le_int`

### Long (57–81)
Similar structure to Integer but for `i64`.
Extra: `format_long`, `format_stack_long`

### Single / Float (82–131)
Similar arithmetic. Math functions are merged: `math_func_single` / `math_func_float` dispatch 10 unary ops (cos/sin/tan/acos/asin/atan/ceil/floor/round/sqrt) via a 1-byte fn_id; `math_func2_single` / `math_func2_float` dispatch 2 binary ops (atan2/log). Separate entries for pow, pi, e.
Extra: `format_single`, `format_stack_single`, `format_float`, `format_stack_float`

### Text (152–175)
`var_text`, `arg_text`, `const_text`, `conv_text_from_null`, `length_text`, `length_character`, `conv_bool_from_text`, `text`, `append_text`, `get_text_sub`, `text_character`, `conv_bool_from_character`, `clear_text`, `free_text`, `eq_text`, `ne_text`, `lt_text`, `le_text`, `format_text`, `format_stack_text`, `append_character`, `text_compare`, `cast_character_from_int`, `conv_int_from_character`

### Enum (176–184)
`var_enum`, `const_enum`, `put_enum`, `conv_bool_from_enum`, `cast_text_from_enum`, `cast_enum_from_text`, `conv_int_from_enum`, `cast_enum_from_int`, `conv_enum_from_null`

### Database / Struct (185–215)
- Record ops: `database` (allocate), `format_database`, `format_stack_database`
- Ref: `conv_bool_from_ref`, `conv_ref_from_null`, `free_ref`, `var_ref`, `put_ref`, `eq_ref`, `ne_ref`, `get_ref`, `set_ref`
- Field access: `get_field`, `get_int`, `get_character`, `get_long`, `get_single`, `get_float`, `get_byte`, `get_enum`, `set_enum`, `get_short`, `get_text`, `set_int`, `set_character`, `set_long`, `set_single`, `set_float`, `set_byte`, `set_short`, `set_text`

### Vector (216–228)
`var_vector`, `length_vector`, `clear_vector`, `get_vector`, `vector_ref`, `cast_vector_from_text`, `remove_vector`, `insert_vector`, `new_record`, `finish_record`, `append_vector`, `get_record`, `validate`

### Collections (229–241)
`hash_add`, `hash_find`, `hash_remove`, `eq_bool`, `ne_bool`, `panic`, `print`, `iterate`, `step`, `remove`, `clear`, `append_copy`, `copy_record`

### Static / Stack (242–247)
`static_call`, `create_stack`, `get_stack_text`, `get_stack_ref`, `set_stack_ref`, `append_stack_text`, `append_stack_character`, `clear_stack_text`

### Callable fn-refs (op_code 232)
`call_ref` — `OpCallRef(fn_var_dist: u16, arg_size: u16)`. Reads the `d_nr` stored at the fn-ref variable's stack position (via `get_var(fn_var_dist)`), looks it up in `State::fn_positions`, and dispatches via `fn_call`. Declared in `default/02_images.loft` to avoid renumbering the file-I/O operators in `01_code.loft`.

`fn_positions: Vec<u32>` on `State` — maps each definition index to its bytecode entry point. Populated at the start of each `execute_argv` / `execute_log` call from `data.definitions`.

---

## DbRef

Universal pointer `(store_nr: u16, rec: u32, pos: u32)` — see [DATABASE.md](DATABASE.md) for the full definition and key/compare API. Used for stack frames (store 1000), struct instances, and vector elements.

---

## Key Patterns

### Object Initialization Order

`object_init()` in `src/parser/expressions.rs` fills unspecified struct fields in **definition order**.
Fields provided in the object literal are set first; then for each missing field,
the stored default expression is emitted, with `Var(0)` replaced by the actual record ref.

### Struct Default Expressions

Stored in `Definition.attributes[n].value` as a `Value` tree.
The `$` token in field defaults maps to `Value::Var(0)` (the current record placeholder).
`Parser::replace_record_ref()` substitutes `Var(0)` → actual record `Value` recursively
over `Call`, `If`, `Block`, and leaf nodes.

### Iterator Protocol

`Iter(var_nr, init, step)`:
- `init` evaluates to the iterator state and stores it in `var_nr`
- `step` advances the iterator and yields the next value (or signals done)
- The outer `Loop(Block)` with `Break` exits when done

---

## Debug Tools (`src/state/debug.rs`, debug builds only)

Two free functions in `src/state/codegen.rs` are compiled only in debug builds
(`#[cfg(debug_assertions)]`).

### `ir_contains_var(value, v) -> bool`

Recursively checks whether a `Value` tree contains any `Var(v)` node. Handles all
`Value` variants: `Call` args, `Set`/`Return`/`Drop` inner, `If` branches,
`Block`/`Loop` operators, `Insert` items, and `Iter` create/next/extra nodes.

Used in `generate_set` at the top of the first-assignment path (`pos == u16::MAX`) to
detect self-reference bugs — a variable appearing in its own first-assignment expression
always indicates a parser bug (storage not yet allocated), and panics with a clear message
naming the function and the broken IR.

### `print_ir(value, data, vars, depth)`

Pretty-prints a `Value` IR tree to stderr in loft-like syntax. Handles all `Value`
variants with appropriate indentation. Called from `def_code` when the `LOFT_IR`
environment variable is set.

**Usage:**
```bash
LOFT_IR=n_test    cargo test my_test -- --nocapture  # one function by name substring
LOFT_IR=*         cargo test my_test -- --nocapture  # all user functions
LOFT_IR=          cargo test my_test -- --nocapture  # same as *
```

Output format:
```
=== IR: n_test ===
{  // block
  d = t_5Color_double(c)
  ...
}
===
```

The `LOFT_IR` gate additionally checks the `logging` flag on the function definition
(true for non-default-library functions) to suppress output for built-in operators.

---

## See also
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
- [DATABASE.md](DATABASE.md) — Store allocator, Stores schema, DbRef, vector/tree/hash implementations
- [INTERNALS.md](INTERNALS.md) — calc.rs, stack.rs, create.rs, native.rs, ops.rs, parallel.rs
- [DESIGN.md](DESIGN.md) — Algorithm catalog including bytecode dispatch, store layout, and collection complexity
