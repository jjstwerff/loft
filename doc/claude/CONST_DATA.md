# Constant Data Initialisation Optimisation

Design for bulk initialisation of constant data structures, reducing bytecode
size and interpreter dispatch overhead for vector literals, struct defaults,
and repeated-element patterns.

---

## Contents
- [Motivation](#motivation)
- [Current behaviour](#current-behaviour)
- [Constant folding](#constant-folding)
- [Proposed changes](#proposed-changes)
  - [O8.1 Bulk primitive vector literals](#o81-bulk-primitive-vector-literals)
  - [O8.2 Bulk struct vector literals](#o82-bulk-struct-vector-literals)
  - [O8.3 Zero-fill struct defaults](#o83-zero-fill-struct-defaults)
  - [O8.4 Const text table](#o84-const-text-table)
  - [O8.5 Constant range comprehensions](#o85-constant-range-comprehensions)
- [Out of scope](#out-of-scope)
- [Implementation order](#implementation-order)

---

## Motivation

A 20-element integer vector literal `[1, 2, ..., 20]` currently emits 60
bytecodes (3 per element: `OpNewRecord` + `OpSetInt` + `OpFinishRecord`)
and performs 20 store-allocation checks plus multiple vector resizes.  Native
codegen produces 60 individual store writes.

For data-heavy programs (lookup tables, configuration, test fixtures), this
overhead dominates both compilation size and startup time.  The store already
has `copy_block()` and `zero_fill()` primitives that can transfer arbitrary
byte ranges in a single call.

---

## Current behaviour

### Primitive vector literal: `[1, 2, 3, 4, 5]`

Parser IR (per element):
```
OpNewRecord(vec, type_nr, u16::MAX)   // allocate element slot
OpSetInt(elm, field_offset, value)    // write the integer
OpFinishRecord(vec, elm, type_nr, u16::MAX)  // increment length
```

Interpreter: 3 dispatches per element.  `OpNewRecord` calls `vector_new()`
which checks capacity and may call `store.resize()`.

Native: 3 function calls per element.  No batching.

### Struct literal: `Point { x: 1.0, y: 2.0 }`

Parser IR (per field):
```
OpSetFloat(ref, field_offset, value)
```

After all explicit fields, `object_init()` fills omitted fields with zero
or default values — one `OpSetInt`/`OpSetFloat`/etc. per omitted field.

### Repeated element: `[Struct { ... }; 100]`

Already optimised: `OpAppendCopy` copies one initialised element N times
using `copy_block()`.  Only the first element is constructed field-by-field.

---

## Constant folding

All O8 phases share a prerequisite: the ability to evaluate pure expressions
at compile time.  `[2*3, 4+1, 10/2]` should be treated as `[6, 5, 5]` and
become eligible for bulk init, not just bare literals like `[6, 5, 5]`.

### What qualifies as a constant expression

An expression is **const-evaluable** when it contains only:

| Node type | Example | Const? |
|---|---|---|
| Integer / long / float / single literal | `42`, `3.14`, `100l` | Yes |
| Boolean literal | `true`, `false` | Yes |
| Character literal | `'A'` | Yes |
| Text literal (no interpolation) | `"hello"` | Yes — but only for text table (O8.4) |
| Arithmetic on const operands | `2 * 3`, `n + 1` where `n` is const | Yes |
| Comparison on const operands | `x > 0` where `x` is const | Yes |
| Unary ops on const operands | `-x`, `!b` where operand is const | Yes |
| `as` cast between numeric types | `42 as long`, `3.14 as integer` | Yes |
| File-scope `UPPER_CASE` constants | `PI`, `MAX_SIZE` | Yes |
| Conditional with const condition | `if true { 1 } else { 2 }` → `1` | Yes |
| Null literal | `null` | Yes (folds to sentinel) |
| Function calls | `sqrt(2.0)` | **No** — side effects not provable |
| Variable references | `x` (local mutable) | **No** |
| Field access | `p.x` | **No** |
| Format strings | `"val={x}"` | **No** — depends on runtime values |

### Implementation: `const_eval()`

Add a function `const_eval(val: &Value, data: &Data) -> Option<Value>` in
`src/parser/expressions.rs` (or a new `src/const_eval.rs`):

```rust
/// Evaluate a pure expression at compile time.
/// Returns Some(literal) when fully evaluable, None otherwise.
/// Conservative: unknown patterns return None → runtime fallback.
///
/// Safety invariants (see §Safety S5):
///  - Integer arithmetic uses wrapping_{add,sub,mul} to match interpreter overflow
///  - Division/modulo by zero → None (runtime returns null)
///  - Division/modulo of i32::MIN by -1 → None (wrapping_div panics in debug)
///  - Float NaN propagation: Rust f64 ops handle this naturally
///  - No recursion depth limit needed: IR tree depth is bounded by parser
pub fn const_eval(val: &Value, data: &Data) -> Option<Value> {
    match val {
        Value::Int(_) | Value::Long(_) | Value::Float(_)
        | Value::Single(_) | Value::Boolean(_) => Some(val.clone()),
        Value::Call(op, args) => {
            let folded: Option<Vec<Value>> = args.iter()
                .map(|a| const_eval(a, data))
                .collect();
            let args = folded?;
            let name = &data.def(*op).name;
            match (name.as_str(), args.as_slice()) {
                // --- integer ---
                ("OpAddInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a.wrapping_add(*b))),
                ("OpMinInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a.wrapping_sub(*b))),
                ("OpMulInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a.wrapping_mul(*b))),
                ("OpDivInt", [Value::Int(a), Value::Int(b)])
                    if *b != 0 && !(*a == i32::MIN && *b == -1) =>
                    Some(Value::Int(a / b)),
                ("OpModInt", [Value::Int(a), Value::Int(b)])
                    if *b != 0 && !(*a == i32::MIN && *b == -1) =>
                    Some(Value::Int(a % b)),
                ("OpMinSingleInt", [Value::Int(a)]) =>
                    Some(Value::Int(a.wrapping_neg())),
                // --- long ---
                ("OpAddLong", [Value::Long(a), Value::Long(b)]) =>
                    Some(Value::Long(a.wrapping_add(*b))),
                ("OpMinLong", [Value::Long(a), Value::Long(b)]) =>
                    Some(Value::Long(a.wrapping_sub(*b))),
                ("OpMulLong", [Value::Long(a), Value::Long(b)]) =>
                    Some(Value::Long(a.wrapping_mul(*b))),
                ("OpDivLong", [Value::Long(a), Value::Long(b)])
                    if *b != 0 && !(*a == i64::MIN && *b == -1) =>
                    Some(Value::Long(a / b)),
                // --- float ---
                ("OpAddFloat", [Value::Float(a), Value::Float(b)]) =>
                    Some(Value::Float(a + b)),
                ("OpMinFloat", [Value::Float(a), Value::Float(b)]) =>
                    Some(Value::Float(a - b)),
                ("OpMulFloat", [Value::Float(a), Value::Float(b)]) =>
                    Some(Value::Float(a * b)),
                ("OpDivFloat", [Value::Float(a), Value::Float(b)]) =>
                    Some(Value::Float(a / b)),  // NaN/Inf handled by IEEE 754
                // --- single ---
                ("OpAddSingle", [Value::Single(a), Value::Single(b)]) =>
                    Some(Value::Single(a + b)),
                ("OpMinSingle", [Value::Single(a), Value::Single(b)]) =>
                    Some(Value::Single(a - b)),
                ("OpMulSingle", [Value::Single(a), Value::Single(b)]) =>
                    Some(Value::Single(a * b)),
                ("OpDivSingle", [Value::Single(a), Value::Single(b)]) =>
                    Some(Value::Single(a / b)),
                // --- comparison (integer) ---
                ("OpEqInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Boolean(*a == *b)),
                ("OpNeInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Boolean(*a != *b)),
                ("OpLtInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Boolean(*a < *b)),
                ("OpLeInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Boolean(*a <= *b)),
                // --- bitwise ---
                ("OpAndInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a & b)),
                ("OpOrInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a | b)),
                ("OpXorInt", [Value::Int(a), Value::Int(b)]) =>
                    Some(Value::Int(a ^ b)),
                // --- casts ---
                ("OpConvLongFromInt", [Value::Int(a)]) =>
                    Some(Value::Long(i64::from(*a))),
                ("OpConvFloatFromInt", [Value::Int(a)]) =>
                    Some(Value::Float(*a as f64)),
                ("OpConvIntFromLong", [Value::Long(a)]) =>
                    Some(Value::Int(*a as i32)),
                ("OpConvIntFromFloat", [Value::Float(a)]) if a.is_finite() =>
                    Some(Value::Int(*a as i32)),
                // --- boolean ---
                ("OpNot", [Value::Boolean(a)]) =>
                    Some(Value::Boolean(!a)),
                ("OpAndBool", [Value::Boolean(a), Value::Boolean(b)]) =>
                    Some(Value::Boolean(*a && *b)),
                ("OpOrBool", [Value::Boolean(a), Value::Boolean(b)]) =>
                    Some(Value::Boolean(*a || *b)),
                _ => None,
            }
        }
        Value::If(cond, then_val, else_val) => {
            if let Some(Value::Boolean(c)) = const_eval(cond, data) {
                const_eval(if c { then_val } else { else_val }, data)
            } else {
                None
            }
        }
        _ => None,
    }
}
```

The function returns `Some(literal)` when the expression can be fully
evaluated, or `None` when it cannot.  It is conservative: any unknown
pattern returns `None` and falls back to runtime evaluation.

Key safety properties:
- `wrapping_*` for integer arithmetic matches interpreter overflow semantics
- Division by zero → `None` (runtime returns null via sentinel)
- `i32::MIN / -1` → `None` (would panic in Rust debug, wraps in release)
- Float division by zero → `Inf`/`NaN` via IEEE 754 (same as runtime)
- `as i32` cast on non-finite float → `None` (avoids undefined truncation)

### Where it plugs in

| Phase | Call site | Effect |
|---|---|---|
| O8.1 | `build_vector_list()` after collecting items | Fold each element; if all fold → bulk init |
| O8.2 | Same, for struct field values | Fold each field; if all fold → packed record |
| O8.3 | `object_init()` for default expressions | Fold default; if folds to zero → skip emit |
| O8.5 | `parse_vector_for()` for `[for i in 0..N { expr(i) }]` | Fold body for each i; if all fold → bulk init |
| General | Any `Value::Call` during second pass | Opportunistic: replace with literal when possible |

### Null sentinel folding

Null sentinels differ by type:

| Type | Null sentinel | Byte representation |
|---|---|---|
| `integer` | `i32::MIN` (`-2147483648`) | `0x00000080` (little-endian) |
| `integer not null` | N/A (0 is valid) | — |
| `long` | `i64::MIN` | `0x0000000000000080` |
| `float` | `NaN` | `0x000000000000F87F` |
| `single` | `NaN (f32)` | `0x0000C07F` |
| `boolean` | `false` | `0x00` |
| `character` | `'\0'` | `0x00000000` |

When folding `null` in a typed context, produce the correct sentinel value
so it can be packed into the bulk data buffer.

### File-scope constants

Loft `UPPER_CASE` constants at file scope are already evaluated once:

```loft
PI = 3.14159265358979;
SCALE = 100;
data = [PI * SCALE, PI * SCALE * 2];  // should fold to [314.159..., 628.318...]
```

`const_eval` resolves `PI` and `SCALE` by looking up their `Value::Set`
initialiser in the IR.  Only constants that are themselves const-evaluable
qualify; a constant initialised from a function call does not.

---

## Proposed changes

### O8.1 — Bulk primitive vector literals

**Applies to:** `vector<integer>`, `vector<long>`, `vector<float>`,
`vector<single>` where ALL elements are const-evaluable (see
[Constant folding](#constant-folding) — includes literals, arithmetic
on literals, file-scope constants, and casts).

**New opcode:** `OpInitVector(vec, count: const u16, elem_size: const u16)`

The opcode reads `count * elem_size` bytes of packed constant data from
the code stream immediately following the operands, then:

1. Allocates a vector record of `(count * elem_size + 8 + 7) / 8` words
2. Writes `count` into the length field (offset 4)
3. Copies the constant bytes into offsets `8..8 + count * elem_size`

**Opcode definition** (`default/01_code.loft`):
```loft
fn OpInitVector(r: vector, count: const u16, elem_size: const u16);
```

The `#rust` body calls a new `vector::init_vector_bulk()` function in
`src/vector.rs` that reads constant data from the code stream.

**Parser detection** (`src/parser/vectors.rs`):
In `build_vector_list()`, after collecting all elements, call
`const_eval()` on each item.  If every element folds to a primitive
literal, pack the folded values into a byte buffer and emit
`OpInitVector` + the raw bytes instead of the per-element loop.

Examples that qualify:
```loft
[1, 2, 3, 4, 5]              // bare literals
[2*3, 4+1, 10/2]             // arithmetic folds to [6, 5, 5]
[PI, PI*2, PI*3]              // constant references fold
[1 as long, 2 as long]       // casts fold
[0; 1000]                     // already optimised via OpAppendCopy
```

**Interpreter** (`src/fill.rs`):
```rust
fn init_vector(s: &mut State) {
    let count = *s.code::<u16>() as u32;
    let elem_size = *s.code::<u16>() as u32;
    // S4: overflow check before allocation
    let total = u64::from(count) * u64::from(elem_size);
    assert!(total <= MAX_STORE_WORDS as u64 * 8, "OpInitVector: {count}×{elem_size} exceeds store limit");
    let total = total as u32;
    // S3: bounds-checked read from code stream
    let src = s.code_ptr(total);
    let db = *s.get_stack::<DbRef>();
    let store = keys::mut_store(&db, &mut s.database.allocations);
    let vec_rec = store.claim((total + 8 + 7) / 8);
    store.set_int(db.rec, db.pos, vec_rec as i32);
    store.set_int(vec_rec, 4, count as i32);
    // S2: data is already at native alignment (8-byte word boundary + 8-byte header)
    store.copy_from_code(vec_rec, 8, src, total);
}
```

**Parser packing** (`src/parser/vectors.rs`):
```rust
// S1: pack in native byte order to match store.set_int / store.set_float
fn pack_const_vector(values: &[Value]) -> Vec<u8> {
    let mut buf = Vec::new();
    for v in values {
        match v {
            Value::Int(n)    => buf.extend_from_slice(&n.to_ne_bytes()),
            Value::Long(n)   => buf.extend_from_slice(&n.to_ne_bytes()),
            Value::Float(n)  => buf.extend_from_slice(&n.to_ne_bytes()),
            Value::Single(n) => buf.extend_from_slice(&n.to_ne_bytes()),
            _ => unreachable!("non-primitive in const vector"),
        }
    }
    buf
}
```

**Native codegen** (`src/generation/`):
Emit a `static INIT_DATA: [u8; N] = [...]` array (bytes in native order)
and a single `store.copy_block_from_slice(vec_rec, 8, &INIT_DATA)` call.

**Bytecode reduction:** `3 * N` opcodes → 1 opcode + `N * elem_size` raw bytes.
For 100 integers: 300 dispatches → 1 dispatch + 400 bytes inline data.

**State method** (`src/state/mod.rs`):
Add `code_ptr(len: u32) -> *const u8` that returns a pointer to the current
code position and advances past `len` bytes.  Panics in debug if
`code_pos + len > code.len()` (S3).  Used only by `OpInitVector`.

---

### O8.2 — Bulk struct vector literals

**Applies to:** `vector<Struct>` where ALL elements are struct literals with
ALL fields being const-evaluable (integers, floats, booleans, characters;
no text, no nested structs, no reference fields).

**Approach:** Extend O8.1 to structs.  Each struct element is a fixed-size
byte record.  Pack all N records contiguously and use the same
`OpInitVector` opcode with `elem_size = struct_record_size`.

**Parser detection:** In `build_vector_list()`, for each struct element:
1. Call `const_eval()` on every field value
2. If all fields fold, write the folded values at the correct byte offsets
   (from `calc::calculate_positions`)
3. If all elements fold, emit `OpInitVector` with the packed records

```loft
struct Point { x: float not null, y: float not null }
data = [
  Point { x: 1.0, y: 2.0 },
  Point { x: 3.0, y: 4.0 },
  Point { x: 5.0 + 0.5, y: 6.0 * 2.0 },  // folds to { x: 5.5, y: 12.0 }
];
// → single OpInitVector with 3 × 16 = 48 bytes of packed data
```

**Limitation:** Struct elements with text or reference fields fall back to
per-element initialisation.  This is the common case for real-world structs,
so the benefit is primarily for numeric-heavy structs (points, colours,
coordinates, pixel data).

---

### O8.3 — Zero-fill struct defaults

**Applies to:** Any struct construction where omitted fields use the default
value (null sentinel for the type).

**Current:** `object_init()` emits one `OpSetInt(ref, offset, 0)` per omitted
integer field, one `OpSetFloat(ref, offset, NaN)` per omitted float, etc.

**Optimisation:** The store's `zero_fill(rec)` already zeroes an entire
record.  Use it as a first step, then patch only non-zero sentinels.

**Approach:**
1. After `OpDatabase` allocates the record, emit `OpZeroFill(ref)` once
2. Only emit explicit `OpSetX` for fields with non-zero null sentinels:
   - `integer` (nullable): `i32::MIN` is `0x00000080`, not zero → explicit
   - `long` (nullable): `i64::MIN` → explicit
   - `float`: NaN → explicit
   - `single`: NaN → explicit
   - Fields with `default(expr)` or `= expr` → explicit
3. Fields that ARE zero after `zero_fill` (skip `OpSetX`):
   - `boolean` null = `0` ✓
   - `character` null = `0` ✓
   - `vector`/`sorted`/`hash`/`index` null = `0` ✓
   - `reference` null = `0` ✓
   - `text` null = `0` ✓
   - `integer not null` default = `0` ✓

**Benefit:** A struct with 5 boolean fields, 3 vector fields, and 2
integer fields reduces from 10 `OpSetInt(0)` calls to 1 `OpZeroFill` +
2 `OpSetInt(i32::MIN)`.  Structs with mostly non-numeric fields benefit
most.

**Risk:** Low — `zero_fill` is already used by the store for freed records.
See S6 in the safety section for the full null-sentinel analysis.

---

### O8.4 — Const text table

**Applies to:** Repeated text literals across a program.

**Current:** Each text literal `"hello"` in a format string or assignment
generates an inline `OpText` with the UTF-8 bytes embedded in the bytecode.
If `"hello"` appears 10 times, the bytes are duplicated 10 times.

**Approach:** Deduplicate text constants into a string table at compile time.
Each unique string gets an index.  `OpConstText(index)` looks up the string
from the table instead of reading inline bytes.

**Benefit:** Reduces bytecode size for programs with repeated string literals
(logging format strings, error messages, enum-to-string tables).

**Cost:** Adds an indirection.  Only beneficial when the same string appears
multiple times.  Not worth it for strings that appear once.

**Verdict:** Low priority — most loft programs use format interpolation, not
repeated literals.  Defer unless bytecode size becomes a bottleneck.

---

### O8.5 — Constant range comprehensions

**Applies to:** `[for i in A..B { expr(i) }]` where `A` and `B` are
const-evaluable integers and `expr(i)` is const-evaluable for every `i`
in the range.

**Current:** A comprehension always generates a runtime loop: init counter,
test bound, evaluate body, append element, increment, branch back.  For
`[for i in 0..100 { i * i }]` this is 100 loop iterations at runtime.

**Optimisation:** At compile time, unroll the loop:
1. Evaluate `A` and `B` via `const_eval` to get concrete integer bounds
2. For each `i` in `A..B`, substitute `i` into the body and call `const_eval`
3. If every iteration folds to a constant, pack the results and emit
   `OpInitVector`

```loft
squares = [for i in 0..10 { i * i }];
// Compiler unrolls: const_eval(0*0)=0, const_eval(1*1)=1, ..., const_eval(9*9)=81
// → OpInitVector with 10 × 4 = 40 bytes: [0, 1, 4, 9, 16, 25, 36, 49, 64, 81]
```

**Filtered comprehensions:** `[for i in A..B if pred(i) { expr(i) }]`
also qualifies when `pred(i)` is const-evaluable.  The compiler evaluates
the predicate for each `i` and only includes elements where it is true:

```loft
evens = [for i in 0..20 if i % 2 == 0 { i }];
// Compiler: const_eval(0%2==0)=true, const_eval(1%2==0)=false, ...
// → OpInitVector with 10 × 4 = 40 bytes: [0, 2, 4, 6, 8, 10, 12, 14, 16, 18]
```

**Size limit (S7):** Do not unroll ranges larger than 10,000 elements.
This is a hard limit enforced in the parser, not configurable.  Ranges
above the limit silently fall back to runtime loops — no error, no
performance regression, just no optimisation.  The limit prevents
adversarial programs from exhausting compiler memory.

```rust
const MAX_CONST_UNROLL: u32 = 10_000;

// In parse_vector_for, before attempting const fold:
let range_size = (end - start) as u32;
if range_size > MAX_CONST_UNROLL {
    // Fall back to runtime loop — range too large for compile-time unroll
    return normal_loop_path();
}
```

**Where it plugs in:** In `parse_vector_for()` (or `build_comprehension_code`),
before emitting the loop IR:
1. Check if range bounds are const-evaluable
2. If so, try to fold the body for each iteration
3. If all fold, emit `OpInitVector` instead of the loop
4. Otherwise fall back to the normal loop path

**Nested comprehensions:** Not supported for const folding.  Only simple
`for i in A..B` with a non-loop body qualifies.

**Dependencies:** O8.1 (provides `OpInitVector`), `const_eval()`.

---

## Safety analysis

### S1 — Endianness: native byte order only

`store.set_int()` writes via `*addr_mut::<i32>() = val`, which uses the host's
native byte order.  `OpInitVector` must pack constant bytes in the **same
native byte order** — i.e. `val.to_ne_bytes()`, not `to_le_bytes()` or
`to_be_bytes()`.

**Risk:** If the packing uses the wrong byte order, every element reads as
garbage.  All current platforms (x86-64, aarch64) are little-endian, so the
bug would only surface on a big-endian target.

**Mitigation:** Use `i32::to_ne_bytes()` / `f64::to_ne_bytes()` in the
packing loop.  Add a test that round-trips a known value through pack →
`OpInitVector` → `get_vector` → compare.

### S2 — Alignment: store uses 8-byte-word addressing

The store's `ptr` is `*mut u8` but `addr_mut::<T>` casts to `*mut T` via
`ptr.offset(...).cast::<T>()`.  This is safe because all records are
allocated at 8-byte word boundaries (`claim` returns word indices, addresses
are `rec * 8 + fld`).  Field offsets are computed by `calc.rs` to respect
alignment.

`OpInitVector` bulk-copies bytes starting at offset 8 (past the length
header).  Elements are at `8 + i * elem_size`.  For 4-byte integers this
is always 4-byte aligned.  For 8-byte longs/floats this is always 8-byte
aligned (because the header is 8 bytes).

**Risk:** None for primitive vectors — alignment is inherent.  For O8.2
(struct vectors), the struct record size must be a multiple of the largest
field alignment (guaranteed by `calc::calculate_positions`).

### S3 — Buffer overflow in code stream

`OpInitVector` reads `count * elem_size` bytes from the bytecode stream.
If the bytecode is malformed (count or elem_size is wrong), the read could
overrun the code buffer.

**Mitigation:** `State::code_ptr(len)` must bounds-check against the code
stream size.  In debug builds, `debug_assert!(self.code_pos + len <= self.code.len())`.
In release builds the code stream is compiler-generated and cannot be
malformed unless the compiler has a bug — same trust model as existing
opcodes that read `code::<u16>()` etc.

### S4 — Store allocation overflow

`store.claim((total + 8 + 7) / 8)` can overflow if `count * elem_size`
exceeds `u32::MAX - 15`.  For `u16` count and `u16` elem_size, the maximum
`total` is `65535 * 65535 = 4,294,836,225` which exceeds `u32::MAX`.

**Mitigation:** Check `(count as u64) * (elem_size as u64) <= MAX_STORE_WORDS * 8`
before the allocation.  If exceeded, panic with a clear message (same as
the existing `MAX_STORE_WORDS` guard in `store.rs`).

### S5 — `const_eval` correctness

If `const_eval` produces a wrong value, the bulk-initialised vector silently
contains incorrect data — with no runtime check.

**Mitigations:**
1. `const_eval` is conservative: any unrecognised pattern returns `None` and
   falls back to runtime.  Wrong results can only come from incorrectly
   implemented operator cases.
2. Use `wrapping_add`/`wrapping_sub`/`wrapping_mul` for integer arithmetic
   to match the interpreter's overflow semantics.  Loft integers wrap on
   overflow — they do not trap.
3. Division by zero: `const_eval` must return `None` (not fold), matching
   the runtime behaviour of returning null.  The design already shows
   `if *b != 0` guard.
4. Float NaN propagation: `NaN + x = NaN`, `NaN * x = NaN` etc. must be
   preserved.  Rust's `f64` arithmetic already handles this.
5. Integer null sentinel: `i32::MIN` is the null sentinel.  Folding
   `i32::MIN + 1` should produce `-2147483647`, not null.  `wrapping_add`
   does the right thing.  Folding `-2147483647 - 1` wraps to `i32::MIN`
   which IS the null sentinel — this matches runtime behaviour.
6. **Test strategy:** For each operator in `const_eval`, add a test that
   compares `const_eval(expr)` against `state.execute(expr)` for the same
   inputs.  Any divergence is a bug.

### S6 — O8.3 zero-fill assumes null sentinels are zero

`zero_fill` writes all-zero bytes.  This is correct for:
- `integer` null = `0` (which IS `i32::MIN`? **No** — `i32::MIN` is
  `0x80000000`, not zero!)

**Correction:** The O8.3 design is partially wrong.  `integer` null
sentinel is `i32::MIN` (`-2147483648` = `0x00000080` in LE), not `0`.
Zero-fill produces `0` which is a valid non-null integer.

For nullable integer fields, `zero_fill` produces the wrong default.
Only `not null` integer fields (where `0` is the intended default) benefit.

**Revised O8.3 rule:** Use `zero_fill` only when ALL omitted fields have
a zero-byte null sentinel:
- `boolean` null = `false` = `0` ✓
- `character` null = `'\0'` = `0` ✓
- `vector`/`sorted`/`hash`/`index` null = `0` (null pointer) ✓
- `reference` null = `0` ✓
- `integer` null = `i32::MIN` = `0x00000080` ✗
- `long` null = `i64::MIN` ✗
- `float` null = `NaN` ✗
- `single` null = `NaN` ✗
- `text` null = null pointer = `0` ✓

So `zero_fill` is safe when the struct has no nullable numeric fields.
Otherwise, emit explicit `OpSetInt(i32::MIN)` / `OpSetFloat(NaN)` for
those fields after the zero-fill.

### S7 — O8.5 compile-time resource exhaustion

Unrolling `[for i in 0..1000000 { i }]` at compile time produces a 4 MB
byte buffer and a 4 MB bytecode segment.  Without a size limit, an
adversarial program can exhaust compiler memory.

**Mitigation:** The design specifies a 10,000-element threshold.  This
should be enforced as a hard limit in the parser, not configurable.
Ranges above the limit silently fall back to runtime loops — no error,
no performance regression, just no optimisation.

### S8 — Parallel execution

`OpInitVector` writes to a store via `keys::mut_store()`.  In parallel
`for` loops, each worker has its own store set.  The bulk init is safe
because store writes are worker-local.

If a parallel worker constructs a constant vector, the `OpInitVector`
runs on the worker's private store — same as the current per-element
path.  No new concurrency risk.

### S9 — Native codegen: static data in generated Rust

O8.1 native codegen emits `static INIT_DATA: [u8; N] = [...]`.  Rust
statics are immutable and thread-safe.  The `copy_block_from_slice` call
copies from the static into the mutable store.

**Risk:** None — Rust's type system ensures the static is never mutated.

---

## Out of scope

| Pattern | Why |
|---|---|
| Sorted/index/hash bulk init | Insertion requires key ordering / hashing per element |
| Runtime-dependent comprehensions | Body depends on variables, function calls, or I/O |
| Mutable default sharing (copy-on-write) | Would require reference counting; complexity not justified |
| JIT compilation | Separate design; this document covers interpreter + native AOT only |
| Cross-function inlining for const eval | Calling `fn square(x: integer) -> integer { x*x }` is not const; only operator intrinsics are folded |

---

## Implementation order

| Phase | Item | Status | Effort | Impact |
|---|---|---|---|---|
| 0 | **`const_eval()`** | **Done** | Small | — |
| O8.1a | **Pre-allocate vector capacity** | **Done** | Small | Medium |
| O8.5 | **Constant range comprehensions** | **Done** | Medium | Medium |
| O8.1b | Packed bytes in bytecode | Not started | Medium | High |
| O8.3 | Zero-fill struct defaults | Not started | Small | Low-Medium |
| O8.2 | Bulk struct vectors | Not started | Medium | Medium |

### Delivered

- **`const_eval()`** — 130-line module with 10 unit tests.  Folds
  arithmetic, casts, comparisons, boolean ops across all numeric types.
- **O8.1a** — `OpPreAllocVector(vec, capacity, elem_size)` eliminates
  all `store.resize()` calls for known-size vector literals.
- **O8.5** — `[for i in 0..N { expr(i) }]` unrolled at compile time when
  bounds and body are const-evaluable.  Filtered comprehensions also
  supported.  10,000-element safety limit.

### Remaining

- **O8.1b** — embed packed constant bytes in bytecode for one-memcpy
  init.  Needs `Value::Bytes` IR variant and `State::code_ptr()`.
  Would reduce 3N → 1 ops (currently 3N+1 with pre-alloc).
- **O8.3** — `OpZeroFill` after `OpDatabase` to skip per-field zero
  writes.  Low-medium value since most fields are explicitly set.
- **O8.2** — pack numeric struct records for bulk init.  Needs
  `const_eval` on struct field values + field offset layout.

---

## LLVM overlap analysis

The native backend compiles generated Rust through `rustc` → LLVM.  With
`--native-release` (`-O`), LLVM applies constant folding, inlining, and
dead-code elimination.  This section evaluates which O8 optimisations
overlap with what LLVM already does, and which remain uniquely valuable.

### What LLVM already optimises

**Arithmetic on literal arguments:**
The generated code emits `ops::op_mul_int(2_i32, 3_i32)`.  With `-O`,
LLVM inlines `op_mul_int` (it's `#[inline]`), sees both arguments are
constants, evaluates the null-sentinel checks (`v1 != i32::MIN`), folds
the arithmetic, and replaces the call with a constant `6_i32`.

This means `const_eval` for **simple arithmetic** (`2*3`, `4+1`) is
**redundant in the native-release path** — LLVM already does it.

**Dead branch elimination:**
`if true { 1 } else { 2 }` — LLVM eliminates the dead branch after
constant propagation.  `const_eval` for conditionals is also redundant
in native-release.

### What LLVM cannot optimise

**Per-element vector construction:**
The generated code calls `OpNewRecord` / `OpFinishRecord` per element.
These are in the `codegen_runtime` module, compiled into `libloft.rlib`.
Without LTO, LLVM treats them as **opaque extern calls with side effects**.
Even with LTO, these functions contain:
- `vector_new()` → capacity check → possible `store.resize()`
- `vector_finish()` → length increment
- Bounds validation in `store.set_int()`

LLVM cannot:
- Batch 20 separate `store.set_int()` calls into one `memcpy`
- Pre-allocate the vector to the known final size (avoiding resizes)
- Eliminate per-element capacity checks
- Merge 20 `OpNewRecord`+`OpFinishRecord` pairs into a single allocation

**This is the core value of O8.1:** it replaces N opaque runtime calls
with one bulk allocation + one `memcpy`.  LLVM cannot derive this
transformation because it cannot see that 20 consecutive `OpNewRecord`
calls target the same vector with known-size elements.

**Comprehension unrolling (O8.5):**
The native codegen does NOT emit a Rust `for` loop for loft comprehensions.
It emits a loft-level loop with `OpStep`/`OpIterate` runtime calls.  LLVM
cannot unroll or eliminate these because they're opaque function calls with
mutable store references.

### Summary per phase

| Phase | Interpreter value | Native-debug value | Native-release value |
|---|---|---|---|
| **`const_eval`** | High — reduces bytecodes | Medium — fewer runtime calls | **Low** — LLVM already folds arithmetic |
| **O8.1** bulk vectors | High — 1 vs 3N dispatches | High — 1 vs 3N calls | **High** — 1 memcpy vs 3N opaque calls |
| **O8.2** bulk struct vectors | High | High | **High** — same as O8.1 |
| **O8.3** zero-fill defaults | Medium — fewer opcodes | Medium — fewer calls | **Medium** — LLVM can't merge set_int calls |
| **O8.4** text table | Low — smaller bytecode | Low | **None** — text literals are Rust `&str` in native |
| **O8.5** const comprehensions | High — eliminates loop | High — eliminates loop | **High** — eliminates opaque loop |

### Revised recommendations

1. **O8.1 (bulk vectors) is valuable across ALL backends.**  The
   per-element `OpNewRecord`/`OpFinishRecord` overhead cannot be
   eliminated by LLVM.  This is the highest-priority item.

2. **`const_eval` is still worthwhile** even though LLVM handles
   arithmetic, because:
   - It benefits the interpreter (the default execution mode)
   - It's the prerequisite for O8.1 detection (identifying which vectors
     are all-constant)
   - It enables O8.5 (comprehension unrolling) which LLVM cannot do
   - Cost is small (~80 lines of Rust)

3. **O8.4 (text table) has NO native value** — the native codegen emits
   Rust string literals (`"hello"`) which are deduplicated by the Rust
   compiler and linker automatically.  Only the interpreter benefits.
   **Deprioritise or drop.**

4. **O8.3 (zero-fill) has moderate native value** — even with `-O`, LLVM
   cannot merge multiple `stores.store_mut(&db).set_int(...)` calls into
   a `memset` because each goes through a bounds-checked method with a
   mutable borrow cycle.

5. **O8.5 (const comprehensions) has high native value** — the loop uses
   opaque runtime dispatch that LLVM cannot unroll or vectorise.

---

## See also
- [OPTIMISATIONS.md](OPTIMISATIONS.md) — Runtime optimisation audit
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark data and root-cause analysis
- [INTERMEDIATE.md](INTERMEDIATE.md) — Bytecode layout and State stack model
- [DATABASE.md](DATABASE.md) — Store allocator and `copy_block` API
