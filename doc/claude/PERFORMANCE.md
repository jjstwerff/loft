
# Performance Analysis

This document records current benchmark results, a root-cause analysis of every
performance gap relative to CPython and hand-written Rust, and a detailed implementation
design for each planned improvement.

---

## Contents

- [Benchmark results](#benchmark-results)
- [How the interpreter executes](#how-the-interpreter-executes)
- [Interpreter vs Python](#interpreter-vs-python)
- [Native vs Rust](#native-vs-rust)
- [wasm vs native](#wasm-vs-native)
- [Design: P1 — Superinstruction merging](#design-p1--superinstruction-merging)
- [Design: P2 — Reduce store indirection on the stack](#design-p2--reduce-store-indirection-on-the-stack)
- [Design: P3 — Confirm integer paths carry no long sentinel](#design-p3--confirm-integer-paths-carry-no-long-sentinel)
- [Design: N1 — Direct-emit local collections in native codegen](#design-n1--direct-emit-local-collections-in-native-codegen)
- [Design: N2 — Omit stores parameter from pure native functions](#design-n2--omit-stores-parameter-from-pure-native-functions)
- [Design: N3 — Remove long null-sentinel from generated code](#design-n3--remove-long-null-sentinel-from-generated-code)
- [Design: W1 — wasm string representation](#design-w1--wasm-string-representation)
- [Improvement priority order](#improvement-priority-order)
- [See also](#see-also)

---

## Benchmark results

All times are wall-clock milliseconds, best of one warm run, single core,
Linux x86-64. Run `bench/run_bench.sh` from the project root to reproduce.

| # | Benchmark | Python | interp | native | wasm | Rust | interp/Py | native/Rust |
|---|-----------|-------:|-------:|-------:|-----:|-----:|----------:|------------:|
| 01 | fibonacci (recursive, n=38)      | 3395 | 4819  | 169 | 257 |  92 | 1.42× | 1.84× |
| 02 | sum loop (10 M integers)         |   66 |  584  |  15 |  21 |   8 | 8.85× | 1.88× |
| 03 | prime sieve (n=100 000)          |   49 |  141  |   4 |   6 |   4 | 2.88× | 1.00× |
| 04 | Collatz lengths (1 .. 1 M)       | 7393 | 14379 | 334 | 599 | 149 | 1.94× | 2.24× |
| 05 | Mandelbrot (200×200, 256 iter)   |  135 |  344  |   7 |  10 |   6 | 2.55× | 1.17× |
| 06 | Newton sqrt (1 M calls)          | 1481 | 3437  | 159 | 159 | 152 | 2.32× | 1.05× |
| 07 | string build (500 K appends)     |   70 |   61  |  33 |  68 |  23 | **0.87×** | 1.43× |
| 08 | word frequency (hash map)        |   46 |  169  |  32 |  60 |   2 | 3.67× | 16.0× |
| 09 | dot product (5 M floats)         |  158 |  428  |  36 |  86 |   3 | 2.71× | 12.0× |
| 10 | insertion sort (3 000 integers)  |  131 |  291  |  29 |  56 |   4 | 2.22× | 7.25× |

Ratios below 2× are expected for an interpreter that has not been tuned yet.
Ratios above 5× in native mode signal a structural problem.

---

## How the interpreter executes

Understanding the interpreter's execution model is prerequisite to every performance design
below.

### Dispatch loop (`src/state/mod.rs`)

The main execution loop fetches one opcode byte per cycle and calls the corresponding
function from a 240-entry array of function pointers (`src/fill.rs`):

```rust
while self.code_pos < bytecode_len {
    let op = *self.code::<u8>();          // fetch byte, advance code_pos
    OPERATORS[op as usize](self);          // indirect call through fn-pointer array
    if self.code_pos == u32::MAX { break; }
}
```

Each element of `OPERATORS` is a standalone Rust function taking `&mut State`.
The array currently has **240 entries** (opcodes 0–239); opcodes 240–255 are unused.

There is no `match` at the top level — dispatch is already a hardware indirect branch.
The cost per cycle is: one array index, one indirect branch (potentially mispredicted),
one function-call ABI round-trip, plus the function body itself.

### Stack and variable access (`src/state/mod.rs`)

The execution stack is **not** a `Vec` per call frame. It is a single flat region of
memory inside a `Stores` record, addressed by two fields:

```rust
pub stack_cur: DbRef,   // (store_nr, rec, pos) — the allocated record
pub stack_pos: u32,     // current offset within that record
```

Every `get_stack<T>` and `put_stack<T>` call does:

```rust
pub fn get_stack<T>(&mut self) -> &T {
    self.stack_pos -= size_of::<T>() as u32;
    self.database
        .store(&self.stack_cur)              // lookup by store_nr
        .addr::<T>(self.stack_cur.rec,
                   self.stack_cur.pos + self.stack_pos)
}
pub fn put_stack<T>(&mut self, val: T) {
    let m = self.database
        .store_mut(&self.stack_cur)          // lookup by store_nr (mutable)
        .addr_mut::<T>(self.stack_cur.rec,
                       self.stack_cur.pos + self.stack_pos);
    *m = val;
    self.stack_pos += size_of::<T>() as u32;
}
```

`database.store(&self.stack_cur)` resolves `store_nr` to a `Store` via an indexed
allocation table. This adds one indirection beyond a raw pointer dereference on every
single push and pop, including every arithmetic intermediate value.

### Function calls

`fn_call` pushes the return address (4 bytes) onto the stack and jumps
`code_pos` to the callee. The callee's local variables live above the caller's on the
same flat stack record — there is no frame allocation or deallocation. Return pops
`code_pos` back from the stack.

The overhead per call is: one `put_stack` (store indirection + write), one `code_pos`
update, and the reverse on return. For a million recursive calls this adds up, but the
store-indirection cost on the many arithmetic operations inside the call body dominates.

---

## Interpreter vs Python

### Summary table

| Group | Benchmarks | Typical ratio | Primary cost |
|---|---|---|---|
| Tight integer loops | 02, 04 | 2–9× | Dispatch overhead per opcode |
| Recursive compute | 01, 06 | 1.4–2.3× | Dispatch × call depth |
| Float loops | 05, 09 | 2.5–2.7× | Same dispatch; FPU hides some |
| Collection-heavy | 08, 10 | 2.2–3.7× | Store indirection on collection access |
| String building | 07 | **0.87×** | loft format-strings beat CPython object churn |

### Root causes (interpreter)

**1. Indirect branch + ABI round-trip per opcode**

The tight inner loop of sum-loop (02) is:

```
var_int  [slot]      → push integer from slot
const_int [1]        → push constant 1
add_int              → pop two, push sum
put_int  [slot]      → pop, store to slot
goto_false [offset]  → pop condition, maybe branch
```

That is 5 `OPERATORS[op](self)` calls per loop iteration, each with a function-call
ABI round-trip (save/restore registers, align stack). CPython's C implementation
executes an equivalent loop body in a single compiled C frame with no function calls.

**2. Store indirection on every push/pop**

Each `get_stack` and `put_stack` resolves `store_nr → Store → raw pointer` before
reading or writing. For sum-loop: 5 opcodes × ~2 stack ops each = ~10 store-indirection
lookups per loop iteration. This competes with CPython which uses a direct C stack
pointer with no extra indirection.

**3. `long` null-sentinel checks**

`long` arithmetic opcodes in `fill.rs` each check whether the operand equals `i64::MIN`
before performing the operation. Collatz (04) uses `long` throughout; this is roughly
one extra conditional branch per arithmetic operation.

**4. Near parity and one win**

String building (07) runs faster in loft (61 ms) than CPython (70 ms) because loft's
format-string concatenation avoids CPython's per-character `PyUnicodeObject` allocation.
This shows the interpreter's overhead is not universal — I/O-bound and allocation-heavy
workloads can favour loft.

---

## Native vs Rust

### Summary table

| Group | Benchmarks | Typical ratio | Primary cost |
|---|---|---|---|
| Pure float compute | 05, 06 | 1.0–1.2× | Near parity — good target |
| Recursive integer | 01, 02, 04 | 1.8–2.2× | `stores` parameter + call overhead |
| Data structures | 08, 09, 10 | 7–16× | `codegen_runtime` vs direct Rust |

### Root causes (native)

**1. Every generated function takes `stores: &mut Stores`**

`src/generation/` emits all loft functions with this signature:

```rust
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 { … }
```

Even functions that never read or write a store carry this parameter. For recursive
Fibonacci (01, 1.84× gap) with ~39 million recursive calls, `rustc -O` cannot inline
across the `&mut Stores` borrow boundary because `Stores` is a large external type.
The parameter forces a register save/restore on every call frame.

**2. `codegen_runtime` helpers for collection operations**

All vector and hash operations in generated code go through functions in
`src/codegen_runtime.rs`. Each helper:
- takes `stores: &mut Stores`
- decodes a `DbRef` (store_nr, rec, pos) to get to the raw data
- performs bounds and null-sentinel checks
- calls into the underlying `vector::` or `hash::` module

Examples: `OpSortVector(stores, data, db_tp)`, `OpInsertVector(stores, data, …)`,
`OpIterate(stores, …)`, `OpHashRemove(stores, …)`, `OpAppendCopy(stores, …)`.

Hand-written Rust uses `vec.sort()`, `vec.push()`, `map.get()` — zero indirection.
The gaps are word frequency (16×), dot product (12×), insertion sort (7.25×).

**3. `long` null-sentinel in generated code**

Generated code for `long` arithmetic emits the same `i64::MIN` check as the interpreter:

```rust
if v1 == i64::MIN || v2 == i64::MIN { i64::MIN } else { v1 + v2 }
```

For Collatz (04, 2.24×) this appears in every loop iteration. Hand-written Rust uses
plain arithmetic with no sentinel.

**4. Float near-parity — the target model**

Newton sqrt (06, 1.05×) and Mandelbrot (05, 1.17×) show what the native pipeline
achieves when there are no stores or collections: `rustc -O` sees clean arithmetic and
produces essentially the same machine code as hand-written Rust. This is the quality
target for integer and collection paths after P1–N2 are implemented.

---

## wasm vs native

| Benchmark | native | wasm | ratio | Note |
|---|---:|---:|---:|---|
| fibonacci       | 169 | 257 | 1.52× | Expected wasm overhead |
| sum loop        |  15 |  21 | 1.40× | Expected |
| sieve           |   4 |   6 | 1.50× | Expected |
| Collatz         | 334 | 599 | 1.79× | `long` sentinel amplified by wasm i64 cost |
| Mandelbrot      |   7 |  10 | 1.43× | Expected |
| Newton sqrt     | 159 | 159 | **1.00×** | FPU bound; wasm matches native |
| string build    |  33 |  68 | 2.06× | wasm memory model for strings |
| word frequency  |  32 |  60 | 1.88× | Hash indirection in wasm linear memory |
| dot product     |  36 |  86 | 2.39× | wasm f64 array layout |
| insertion sort  |  29 |  56 | 1.93× | wasm indirect memory for vector ops |

The 1.4–1.8× overhead on compute-bound benchmarks is structural wasm cost (linear memory
model, function-call overhead through wasm module boundary). FPU-bound Newton sqrt
achieves exact parity because the bottleneck is the FPU, not memory access.
The 2× gaps on data structures and strings are design-level issues addressed by W1.

---

## Design: P1 — Superinstruction merging

**Affected benchmarks:** 02 (8.85×), 04 (1.94×), 03 (2.88×), all tight loops
**Expected gain:** 2–4× on integer loops (reduces dispatch cycles by 60–80% in hot paths)
**Cost:** Medium — peephole pass + new opcode entries + new function bodies

### Background

**Blocked:** The opcode table now has 254/256 entries — only 2 slots remain (255–256),
which is not enough for even one superinstruction sequence.  O1 is deferred indefinitely
until opcode space is freed (e.g. by a two-byte escape prefix or a dedicated
superinstruction dispatch table).  The hot-pattern analysis below is preserved for
reference when that redesign is undertaken.

### Hot patterns

Profile of a tight integer loop in loft bytecode:

```
var_int   [slot_a]   ; load variable a
var_int   [slot_b]   ; load variable b
add_int              ; a + b
put_int   [slot_c]   ; store to c
```

```
var_int   [slot_i]   ; load loop counter
const_int [limit]    ; load constant upper bound
cmp_lt_int           ; i < limit?
goto_false [offset]  ; exit loop if false
```

```
var_int   [slot_i]   ; load counter
const_int [1]        ; load 1
add_int              ; i + 1
put_int   [slot_i]   ; i = i + 1
```

The 16 available slots cover the following four superinstructions:

| # | Name | Pattern | Operands | Cycles saved |
|---|---|---|---|---|
| 240 | `si_load2_add_store` | `var_int var_int add_int put_int` | a, b, c (3 × u16) | 3 of 4 |
| 241 | `si_load_const_add_store` | `var_int const_int add_int put_int` | a, k, c | 3 of 4 |
| 242 | `si_load_const_cmp_lt_branch` | `var_int const_int cmp_lt_int goto_false` | a, k, offset | 3 of 4 |
| 243 | `si_load2_cmp_lt_branch` | `var_int var_int cmp_lt_int goto_false` | a, b, offset | 3 of 4 |
| 244 | `si_load_const_mul_store` | `var_int const_int mul_int put_int` | a, k, c | 3 of 4 |
| 245 | `si_load2_mul_store` | `var_int var_int mul_int put_int` | a, b, c | 3 of 4 |

Six superinstructions leave 10 slots for future use. Extend to more patterns if profiling
shows additional high-frequency sequences.

### Peephole pass design

**Location:** `src/compile.rs`, after `state.def_code(d_nr, data)`.

The pass operates on the already-emitted bytecode for one function at a time.
It scans from the start of the function's bytecode region and replaces matching windows
in-place. In-place replacement is safe because superinstruction operand encodings are
designed to be at most as wide as the replaced sequence.

```rust
fn peephole(bytecode: &mut Vec<u8>, start: usize) {
    let mut pc = start;
    while pc < bytecode.len() {
        // Peek at next 4 opcodes (each opcode byte is followed by operand bytes).
        // Parse a window: opcode, then however many operand bytes its encoding needs.
        if let Some((si, new_len)) = match_superinstruction(bytecode, pc) {
            rewrite(bytecode, pc, si, new_len);
            // Do not advance pc — try to match again from same position.
        } else {
            pc += instruction_len(bytecode, pc);
        }
    }
}
```

`match_superinstruction` returns `Some((si_opcode_byte, total_bytes_used))` when a
known pattern matches. `rewrite` overwrites the window starting at `pc` with the new
opcode and its merged operands, then fills the remaining bytes with a new `nop` opcode
(or shrinks the Vec if relocation is acceptable — see below).

### Operand encoding

The canonical form for `si_load_const_add_store` (pattern: `var_int a; const_int k;
add_int; put_int c`):

```
[245] [a_lo] [a_hi] [k_b0] [k_b1] [k_b2] [k_b3] [c_lo] [c_hi]
```
- `a` and `c` are u16 slot offsets (same as `var_int` / `put_int`)
- `k` is a i32 constant (same as `const_int`)
- Total: 9 bytes, same as the original 4-instruction sequence:
  `var_int`(3) + `const_int`(5) + `add_int`(1) + `put_int`(3) = 12 bytes → savings 3 bytes

Because the replacement is always ≤ the original sequence length, the bytecode can be
rewritten in-place; excess bytes become `nop` (opcode 0 if `goto` is not 0, or a
dedicated `nop` opcode). This avoids having to relocate any branch targets.

**Alternative: shrink and relocate.** After peephole, walk the bytecode a second time
and update all `goto` / `goto_false` / `goto_word` / `call` target offsets. This
removes `nop` padding but is more complex. Defer until profiling shows the padding
matters.

### Superinstruction bodies (`fill.rs`)

Example for `si_load_const_add_store`:

```rust
fn si_load_const_add_store(s: &mut State) {
    let slot_a = *s.code::<u16>();
    let k      = *s.code::<i32>();
    let slot_c = *s.code::<u16>();
    let a = *s.get_var::<i32>(slot_a);
    let result = ops::op_add_int(a, k);
    s.put_var(slot_c, result);
}
```

This body does no intermediate stack push/pop — it reads both inputs directly from
variables or the constant, computes the result, and writes it directly to a variable.
The store-indirection lookups drop from 5 (`var_int` get + `const_int` push + `add_int`
get×2 + push + `put_int` get + store) to 2 (`get_var` + `put_var`).

### Registration

Add to the end of `OPERATORS` in `fill.rs`:

```rust
pub const OPERATORS: &[fn(&mut State); 246] = &[
    // … existing 240 …
    si_load2_add_store,        // 240
    si_load_const_add_store,   // 241
    si_load_const_cmp_lt_branch, // 242
    si_load2_cmp_lt_branch,    // 243
    si_load_const_mul_store,   // 244
    si_load2_mul_store,        // 245
];
```

### Prerequisite check

Before implementing, confirm that `instruction_len(bytecode, pc)` can be computed from
opcode tables alone (without executing the instruction). Since every opcode's operand
width is fixed and determined by the opcode byte, this is straightforward to add as a
companion to the OPERATORS array (a `static OPCODE_LEN: &[u8; 256]` table).

---

## Design: P2 — Reduce store indirection on the stack

**Affected benchmarks:** 01 (1.42×), 02 (8.85×), 04 (1.94×), 05 (2.55×), 06 (2.32×)
**Expected gain:** 20–50% across all interpreter benchmarks
**Cost:** High — touches `State`, `Store`, and the entire stack-access API

### Background

Every `get_stack<T>` and `put_stack<T>` call currently goes through:

```
database.store(&self.stack_cur)          // HashMap/vec lookup by store_nr
  .addr::<T>(self.stack_cur.rec,         // compute raw pointer from record
             self.stack_cur.pos + self.stack_pos)
```

The `database.store()` lookup is at minimum an array index into `allocations`, but the
raw pointer to the record's memory changes whenever the underlying `Store` reallocates.
This means the pointer cannot be cached across calls.

### Proposed change: cache the raw stack pointer

Add a `stack_base: *mut u8` field to `State` that is refreshed once per function call
(when `stack_pos` changes structurally, not on every push/pop):

```rust
pub struct State {
    // … existing fields …
    stack_base: *mut u8,   // raw pointer to start of stack record
}
```

After every `fn_call` and `op_return`, refresh:

```rust
fn refresh_stack_ptr(&mut self) {
    self.stack_base = self.database
        .store_mut(&self.stack_cur)
        .record_ptr_mut(self.stack_cur.rec, self.stack_cur.pos);
}
```

Then `get_stack` and `put_stack` become pointer arithmetic with no extra lookup:

```rust
pub fn get_stack<T>(&mut self) -> &T {
    self.stack_pos -= size_of::<T>() as u32;
    unsafe { &*(self.stack_base.add(self.stack_pos as usize) as *const T) }
}
pub fn put_stack<T>(&mut self, val: T) {
    unsafe {
        *(self.stack_base.add(self.stack_pos as usize) as *mut T) = val;
    }
    self.stack_pos += size_of::<T>() as u32;
}
```

`get_var` and `put_var` become similarly simple: `stack_base - slot_offset`.

### Safety requirement

`stack_base` must be **invalidated** whenever the underlying store could reallocate:
- When a new record is allocated (`OpNewRecord`, `OpDatabase`)
- When a vector grows (`OpInsertVector`, `OpAppendCopy`)

In those cases, `execute()` must call `refresh_stack_ptr()` after the operation.
The simplest approach: make `OPERATORS` entries that allocate call `refresh_stack_ptr`
unconditionally at their end. Add a helper flag to `State`:

```rust
pub stack_dirty: bool,  // set by any allocation op; checked at top of loop
```

```rust
while self.code_pos < bytecode_len {
    let op = *self.code::<u8>();
    OPERATORS[op as usize](self);
    if self.stack_dirty {
        self.refresh_stack_ptr();
        self.stack_dirty = false;
    }
    if self.code_pos == u32::MAX { break; }
}
```

This adds one branch per loop iteration (cheaply predicted) and eliminates the
`database.store()` lookup on every arithmetic push/pop.

### Risk

The `Store` backing the stack record must not move between `refresh_stack_ptr` and
the next push/pop. This holds as long as no allocation occurs on the stack store itself
between refreshes. The stack store (`stack_cur`) is never modified by collection
operations — those use different stores — so the invariant is maintainable.

### Alternative (lower risk, lower reward)

If the raw-pointer approach is too risky, a smaller improvement: cache
`&self.database.allocations[stack_store_nr]` as a field. This saves the `HashMap`
or `Vec` index lookup but still requires the `rec + pos` offset calculation. Estimated
gain: 10–20% vs 20–50% for the full approach.

---

## Design: P3 — Confirm integer paths carry no long sentinel

**Affected benchmarks:** 02, 10 (minor — already separated by opcode)
**Expected gain:** 2–5% on pure integer benchmarks
**Cost:** Low — mostly verification + one test

### Background

`integer` (i32) and `long` (i64) already have separate opcode variants in `fill.rs`
(`add_int` vs `add_long`). The question is whether any `integer` path inadvertently
checks `i64::MIN`.

### Design

1. **Grep audit:** Search `fill.rs` for `i64::MIN` and `i32::MIN`. Confirm they appear
   only in `*_long` functions, never in `*_int` functions.

2. **Compile-time enforcement:** Add a `static_assertions` check in `fill.rs` or a
   test that ensures the `op_add_int`, `op_mul_int`, `op_sub_int` functions in
   `src/ops.rs` contain no branch comparing to `i64::MIN`:

   ```rust
   #[test]
   fn integer_ops_have_no_long_sentinel_checks() {
       // Read ops.rs source, assert no "i64::MIN" appears in *_int functions.
       // Achievable via include_str! + string search.
   }
   ```

3. **If violations exist:** Separate the dispatch table into `op_add_int(a: i32, b: i32)
   -> i32` (no sentinel) vs `op_add_long(a: i64, b: i64) -> i64` (sentinel). The
   `integer` opcode calls the `i32` variant exclusively.

This is a verification task that may yield no changes if the separation is already clean.

---

## Design: N1 — Direct-emit local collections in native codegen

**Affected benchmarks:** 08 (16×), 09 (12×), 10 (7.25×)
**Expected gain:** 5–15× on data-structure benchmarks; closes the native/Rust gap
**Cost:** High — new analysis pass, new emit path, extended type system in codegen

### Background

All vector and hash collection access in generated Rust currently goes through
`codegen_runtime` helpers that take `stores: &mut Stores` and decode `DbRef` pointers.
For a local `vector<integer>` used only within one function, the correct Rust type is
`Vec<i32>` — no stores, no DbRef, no bounds-check beyond Rust's built-in `panic`.

### Escape analysis pass

A new pre-pass over the IR (run once per function definition, before native code
generation) marks each local variable with one of:

```
Local      — declared in this function, never assigned to a store field
             and never passed by reference to another function
Escaping   — passed by &ref to another function, assigned to a struct field,
             or stored in a Store
External   — parameter or return value
```

Only `Local` variables qualify for direct emit. The analysis is conservative: if in
doubt, mark `Escaping`.

**Rules for `Local`:**
- `Value::Var(v)` where `v` is declared in the current function body → start as `Local`
- `Value::Call(_, args)` where arg is `Value::Ref(v)` → mark `v` as `Escaping`
- `Value::Store(field, v)` → mark `v` as `Escaping`
- `Value::Assign(dest, v)` where `dest` is a struct field → mark `v` as `Escaping`

### Direct-emit type mapping

For a `Local` variable of loft type `vector<T>`, generate Rust type:

| loft type | Rust direct type |
|---|---|
| `vector<integer>` | `Vec<i32>` |
| `vector<long>` | `Vec<i64>` |
| `vector<float>` | `Vec<f64>` |
| `vector<text>` | `Vec<String>` |
| `index<integer, T>` (local hash) | `HashMap<i32, T>` |
| `index<text, T>` (local hash) | `HashMap<String, T>` |

### Operation mapping

When emitting operations on a `Local` variable, bypass codegen_runtime:

| loft operation | current emit | direct emit |
|---|---|---|
| `v[i]` (get) | `vector::get_vector(&v, size, i, &allocations)` | `v[i as usize]` |
| `v[i] = x` (set) | `vector::set_vector(&mut v, size, i, x, &mut alloc)` | `v[i as usize] = x` |
| `v.length` | `OpSizeofRef(stores, v)` | `v.len() as i32` |
| `v.append(x)` | `OpAppendCopy(stores, v, 1, tp)` | `v.push(x)` |
| `v.sort()` | `OpSortVector(stores, v, tp)` | `v.sort()` |
| `h[k]` (get) | hash::find + store decode | `h.get(&k).copied()` |
| `h[k] = v` | hash operations through stores | `h.insert(k, v)` |

### Declaration site

For a `Local` vector, emit its declaration as a `Vec`:

```rust
let mut var_counts: Vec<i32> = Vec::new();
```

instead of the current:

```rust
let mut var_counts: DbRef = stores.null();
```

Its `drop` at end of scope is automatic — no `OpFreeRef` call needed.

### Interaction with function calls

If a `Local` vector must be passed to a function that expects `DbRef`, it is not
`Local` by the escape analysis above — it has `Escaping` status and uses the existing
store-backed path. This ensures correctness without special cases.

### Changes to `src/generation/`

1. Add `fn escape_analysis(def_nr: u32, data: &Data) -> HashMap<u16, Locality>`.
2. In `Output::output_code_inner`, check `locality[var]` before emitting any
   collection operation.
3. Add a `direct_emit_vec_op` and `direct_emit_hash_op` path alongside the existing
   `codegen_runtime` call emitter.

### Verification strategy

Add a new benchmark test (`tests/bench/`) that asserts the generated Rust for
`09_matrix_mul.loft` contains `Vec<f64>` and no `OpAppendCopy`. Run `make ci` to
ensure the native pipeline produces correct output for all 10 benchmarks.

---

## Design: N2 — Omit stores parameter from pure native functions

**Affected benchmarks:** 01 (1.84×), 06 (2.32×)
**Expected gain:** 10–30% on recursive compute benchmarks
**Cost:** High — purity analysis, two function signatures, call-site dispatch

### Background

Every generated function is currently emitted as:

```rust
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 {
    if n <= 1 { return n; }
    n_fibonacci(stores, n - 1) + n_fibonacci(stores, n - 2)
}
```

The `stores: &mut Stores` parameter is an 8-byte pointer that must be saved and
restored across recursive calls. `rustc -O` cannot eliminate it because `Stores` is an
externally-defined large struct. For Fibonacci this adds roughly one register
save/restore pair per call — measured cost is 1.84× vs hand-written Rust.

### Purity definition

A function is **pure** for native codegen purposes if:
1. It does not read or write any `Store`
2. It does not call any non-pure function
3. It has no `Format`, `IO`, `HashFind`, `NewRecord`, `FreeRef`, or similar operations
   in its IR

Purity is determined by a recursive scan of `def.code: Value` before `generation/`
runs.

### Pure function signature

```rust
fn n_fibonacci_pure(n: i32) -> i32 {
    if n <= 1 { return n; }
    n_fibonacci_pure(n - 1) + n_fibonacci_pure(n - 2)
}
```

`rustc -O` can now inline or tail-call-optimise this freely.

### Entry-point wrapper

The non-pure `n_fibonacci` wrapper (called from stores-using code) delegates:

```rust
fn n_fibonacci(stores: &mut Stores, n: i32) -> i32 {
    n_fibonacci_pure(n)
}
```

This keeps the call interface uniform while giving `rustc` the pure inner function
to optimise.

### Purity analysis implementation

Add `fn is_pure(def_nr: u32, data: &Data, cache: &mut HashMap<u32, bool>) -> bool`
to `src/generation/`. Scan `data.def(def_nr).code` recursively:

```rust
fn is_pure(v: &Value, data: &Data, cache: &mut HashMap<u32, bool>) -> bool {
    match v {
        Value::Call(d_nr, args) => {
            let def = data.def(*d_nr);
            if def.name.starts_with("Op") { return false; }  // codegen_runtime op
            if def.rust.contains("stores") { return false; } // uses stores in template
            let callee_pure = *cache.entry(*d_nr).or_insert_with(|| {
                is_pure(&def.code, data, cache)
            });
            callee_pure && args.iter().all(|a| is_pure(a, data, cache))
        }
        Value::Block(vs) => vs.iter().all(|v| is_pure(v, data, cache)),
        Value::If(c, t, f) => is_pure(c, data, cache) && is_pure(t, data, cache)
                               && is_pure(f, data, cache),
        // Literals, variables, arithmetic — always pure
        Value::Int(_) | Value::Float(_) | Value::Text(_) | Value::Boolean(_)
        | Value::Var(_) | Value::Assign(_, _) => true,
        // Anything involving stores or IO
        Value::Ref(_) | Value::Store(_, _) | Value::Format(_) => false,
        _ => false,  // conservative: unknown nodes are not pure
    }
}
```

Memoise results to avoid exponential recursion on call graphs.

### Changes to `src/generation/`

1. Add `fn is_pure` (above).
2. In `output_native_reachable`, for each pure function, emit both `n_foo_pure`
   (no `stores`) and `n_foo` (wrapper with `stores`).
3. In `output_function`, when emitting a call to a pure function from within another
   pure function, emit `n_foo_pure(…)` directly.

---

## Design: N3 — Remove long null-sentinel from generated code

**Affected benchmarks:** 04 (2.24×)
**Expected gain:** 1.3–1.5× on Collatz and any `long`-heavy generated code
**Cost:** Low — localised change in `src/generation/` + `src/ops.rs`

### Background

The current generated code for `long` arithmetic, e.g. addition, is:

```rust
// ops::op_add_long as emitted today
if v1 == i64::MIN || v2 == i64::MIN { i64::MIN } else { v1 + v2 }
```

For Collatz, this pattern appears in every loop body. The two comparisons and the
conditional branch prevent `rustc -O` from auto-vectorising or pipelining the arithmetic.

### Strategy: sentinel checks only at store boundaries

`i64::MIN` as null means "this field was never written". This matters only when:
- Reading a `long` field from a `Store` record that may never have been assigned
- Writing a `long` field and wanting to clear it (set to null)

Within a function body, a `long` local variable that has been assigned is never null
during arithmetic. Generated code has definite assignment for every local variable at
its first use (guaranteed by the compiler's scope analysis).

### Design

1. **New template in `src/ops.rs`:**

   ```rust
   #[inline(always)]
   pub fn op_add_long_nn(v1: i64, v2: i64) -> i64 { v1 + v2 }  // nn = non-null
   #[inline(always)]
   pub fn op_mul_long_nn(v1: i64, v2: i64) -> i64 { v1 * v2 }
   // … etc. for all long arithmetic ops
   ```

2. **In `src/generation/`:**

   For a `long` binary operation where both operands are local variables (determined by
   the same escape analysis pass from N1, applied to `long` variables), emit
   `op_add_long_nn` instead of `op_add_long`.

   For a `long` field read from a store or a function parameter annotated as nullable,
   continue to use `op_add_long` with the sentinel check.

3. **Conservative fallback:** If there is any doubt about nullability (e.g. the value
   comes from a function call that returns `long`), use the sentinel version. Only
   local-variable-to-local-variable arithmetic with definite assignment uses `_nn`.

### Changes

- `src/ops.rs`: add `_nn` variants for `add`, `sub`, `mul`, `div`, `mod`, `neg`,
  comparison ops.
- `src/generation/`: in the long-arithmetic emit path, check nullability of both
  operands before choosing variant.
- `default/01_code.loft`: add `#rust` templates for the `_nn` ops if needed by codegen.

---

## Design: W1 — wasm string representation

**Affected benchmark:** 07 (2.06× wasm vs native)
**Expected gain:** Reduce the gap to <1.3×
**Cost:** Medium — wasm-target conditional compilation

### Background

The wasm build compiles the same `src/` Rust code as the native build, which means
string operations use Rust `String` — heap-allocated via Rust's allocator inside wasm
linear memory. Each dynamic string operation (append, concatenate, slice) involves a
call to the wasm allocator, which is slower than native `malloc` because it must
operate within the linear memory model with `memory.grow` for expansion.

### Design

Use `wasm-bindgen`'s or `wasm-pack`'s built-in string handling or, for the wasip2
target, use the native `String` representation but optimise the critical format-string
path:

1. **Pre-allocate the result buffer.** In the generated `format!` equivalent for string
   building, compute an estimated capacity from the number of append operations (if
   statically known) and `String::with_capacity(n * avg_element_len)` before the loop.

2. **Avoid intermediate allocations.** Replace `text + other_text` (which allocates
   a new `String`) with `text.push_str(&other_text)` (appends in place). The loft
   compiler already emits format-string concatenation this way in the interpreter; verify
   that `src/generation/` does the same for native/wasm.

3. **Profile first.** Run `bench/run_bench.sh` with wasm and capture a perf trace
   via `wasmtime --profile`. If the 2× gap is allocator overhead, the capacity
   pre-allocation above will close most of it. If it is wasm function-call overhead
   on string operations, a different approach is needed.

This item is lower priority than P1 and N1 because the absolute time difference is
small (35 ms) and the affected benchmark (string building) is already fast in both
modes.

---

## Improvement priority order

| Priority | Item | Target benchmarks | Expected gain | Cost |
|---|---|---|---|---|
| 1 | P1 — Superinstructions | 02, 03, 04, all tight loops | 2–4× on integer loops | Medium |
| 2 | N1 — Direct collection emit | 08, 09, 10 | 5–15× data-struct native | High |
| 3 | P2 — Stack raw pointer cache | all interpreter | 20–50% across interpreter | High |
| 4 | N2 — Pure function stores omit | 01, 06 native | 10–30% recursive native | High |
| 5 | N3 — Long sentinel in codegen | 04 native | ~1.5× Collatz native | Low |
| 6 | P3 — Verify integer sentinel | 02, 10 | 2–5% (verification) | Low |
| 7 | W1 — wasm string path | 07 wasm | <1.3× gap | Medium |

Items 1–3 should be scheduled after the 0.8.3 language-syntax milestone. P1 is the
highest-impact single change because it benefits every tight loop in the interpreter
without touching the memory model.

---

## See also

- Optimisations section below — runtime optimisation opportunities audit
- [PLANNING.md](PLANNING.md) — priority-ordered enhancement backlog
- [INTERNALS.md](INTERNALS.md) — `src/fill.rs`, `src/state/`, `src/generation/`
- [NATIVE.md](NATIVE.md) — native code generation design and known issues
- [doc/00-performance.html](../00-performance.html) — rendered benchmark page with bar charts

---

This document audits the interpreter runtime for concrete performance improvements,
weighing impact against implementation cost and maintainability.

## Contents
- [Open opportunities](#open-opportunities)
- [Not worth changing](#not-worth-changing)
- [Open — recommended priority order](#open--recommended-priority-order)

Completed optimisations (debug_assert, clone removal, Arc bytecode sharing, LLRB free-list)
are recorded in CHANGELOG.md.

---

## Open opportunities

### 1. `Stores::types` and `Stores::names` cloned for every worker

**File:** `database.rs:1541-1561`

`clone_for_worker` copies:

- `types: self.types.clone()` — `Vec<Type>`, read-only after compilation
- `names: self.names.clone()` — `HashMap<String, u16>`, read-only after compilation

Both are pure metadata that no worker modifies.  Wrapping them in
`Arc<Vec<Type>>` and `Arc<HashMap<String, u16>>` would reduce the per-worker
clone to two atomic-ref-count increments.

For a program with 200 types and a 500-entry name map the savings are small in
absolute bytes, but the pattern becomes significant if the type system grows or
if hundreds of parallel calls are made.

**Impact:** Low-Medium — mainly prevents future scaling problems
**Cost:** Medium — field types change throughout `database.rs`; some methods need `Arc::make_mut` if mutation is ever needed before `clone_for_worker` is called
**Verdict:** Defer until parallel usage grows; note the shape of the fix here

---

## Not worth changing

| Pattern | Reason |
|---|---|
| `State` HashMap fields (`stack`, `vars`, `calls`, `types`, `line_numbers`) | Only accessed in debug/dump functions, not in the hot execute loop |
| `WorkerProgram` channel + batching in `parallel.rs` | `Vec::with_capacity(end-start)` is already exact; no reallocation |
| `calc.rs` BTreeMap for struct layout | Compile-time only; immeasurable runtime effect |
| `library_names: HashMap<String, u16>` | Queried during compilation, not execution; worker states leave it empty |
| Function pointer dispatch table in `fill.rs` | Already optimal for an interpreter; JIT is the next step |

---

## Open — recommended priority order

| # | Change | File(s) | Effort | Impact |
|---|--------|---------|--------|--------|
| 1 | `Arc` for `Stores::types` / `names` | `database.rs` | Medium | Low–Med |
| 2 | O8.1b: packed bytes in bytecode | `vector.rs`, `state/mod.rs` | Medium | High |
| 3 | O8.3: zero-fill struct defaults | `parser/objects.rs` | Small | Low–Med |

---

## O1 Superinstruction Peephole — Design Notes (deferred)

The infrastructure for superinstructions is in place but the peephole rewriting
pass is deferred to a future release.  This section documents the design for
the implementor.

### What exists

- **Opcodes registered** in `default/01_code.loft`: `OpSiLoad2AddStore`,
  `OpSiLoadConstAddStore`, `OpSiLoadConstCmpBranch`, `OpSiLoad2CmpBranch`,
  `OpSiLoadConstMulStore`, `OpSiLoad2MulStore`, `OpNop`.
- **State stubs** in `src/state/mod.rs`: delegation methods that call `nop()`.
  Replace these with the real implementations below.
- **`fill.rs` auto-generated** with the opcodes in the OPERATORS array.
- **`build_opcode_len_table()`** in `src/compile.rs`: computes instruction
  byte-lengths from operator definitions — survives renumbering.
- **`opcode_by_name()`** in `src/compile.rs`: resolves opcode numbers by name.
- **`fill_rs_up_to_date`** CI test: asserts `src/fill.rs` matches the generated
  version — prevents drift when `01_code.loft` changes.

### The stack-relative operand problem

`get_var(pos)` computes `stack_base + stack_pos - pos`.  Each `VarInt` pushes
4 bytes, advancing `stack_pos`.  The superinstruction runs without intermediate
pushes, so the second operand sees the wrong `stack_pos`.

**Arithmetic for `VarInt(a) VarInt(b) AddInt PutInt(c)` at initial SP:**

| Instruction | stack_pos | Address accessed |
|-------------|-----------|-----------------|
| VarInt(a) | SP | base + SP - a |
| VarInt(b) | SP+4 | base + SP + 4 - b |
| AddInt | SP+8→SP+4 | (pops 2, pushes 1) |
| PutInt(c) | SP+4→SP | base + SP + 4 - c |

The superinstruction at SP (no pushes):
- `get_var(a)`: base + SP - a ✓
- `get_var(b)`: base + SP - b ✗ (should be base + SP + 4 - b)
- `put_var(c)`: base + SP + 4 - c ✓ (put_var adds sizeof(T) internally)

**Fix:** adjust `b' = b - 4` in the peephole rewriter.  Then `base + SP - (b-4) = base + SP + 4 - b`. ✓

**Guard:** skip the pattern when `b < 4` (would underflow).

### Real implementations for State methods

Replace the `nop()` stubs with:

```rust
pub fn si_load2_add_store(&mut self) {
    let a = *self.code::<u16>();
    let b = *self.code::<u16>();  // pre-adjusted: b' = b - 4
    let c = *self.code::<u16>();
    let va = *self.get_var::<i32>(a);
    let vb = *self.get_var::<i32>(b);
    self.put_var(c, crate::ops::op_add_int(va, vb));
}
// Same pattern for si_load2_mul_store.
// For const variants: k is a literal (no adjustment).
// For cmp+branch: si_load2_cmp_branch reads i16 offset, branches if va >= vb.
```

### Peephole rewriter

Add `PeepholeCtx` to `src/compile.rs` that:
1. Builds opcode-length table via `build_opcode_len_table(data)`
2. Resolves opcodes by name via `opcode_by_name(data, name)`
3. Scans each function's bytecode as a sliding 4-instruction window
4. Matches patterns with exact length guards (l0==3, l1==3, l2==1, l3==3)
5. Rewrites in-place with adjusted operands, fills excess bytes with OpNop
6. **Skips default library functions** (`data.def(d_nr).position.file.starts_with("default/")`)

### Known issue: default library corruption

Running the peephole on default library functions causes `issue_84` tests
(recursive merge sort) to fail with "Unknown record" errors.  Root cause:
the default library uses patterns where the VarInt operands interact with
store-relative addressing in ways the simple b-4 adjustment doesn't cover
(possibly involving RefVar parameters or OpCreateStack pushes between the
matched instructions).

**Mitigation:** skip default library functions.  They're already fast
(hand-optimised `#rust` templates).  Only user functions benefit from
superinstructions.

### Adjustments per pattern

| Pattern | a | b/k | c/off | Super size |
|---------|---|-----|-------|------------|
| `VarInt VarInt {Add\|Mul}Int PutInt` | a | b-4 | c | 7 bytes |
| `VarInt ConstInt {Add\|Mul}Int PutInt` | a | k | c | 9 bytes |
| `VarInt VarInt LtInt GotoFalse` | a | b-4 | i16 offset | 7 bytes |
| `VarInt ConstInt LtInt GotoFalse` | a | k | i16 offset | 9 bytes |

Branch offset for cmp patterns: original `goto_false` offset is i8 relative
to `pc3+2`.  Super offset is i16 relative to `pc+7` (or `pc+9` for const).
Compute: `new_off = (pc3 + 2 + old_off) - (pc + super_size)`.

---

## See also
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark results, root-cause analysis, and detailed designs for O1–O7 (superinstructions, stack pointer cache, native collection emit, purity analysis)
- [PLANNING.md](PLANNING.md) — Priority-ordered backlog
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/store.rs`, `src/state/` implementation details

### 2. O8: Constant data initialisation (delivered 2026-04-02)

**Files:** `src/const_eval.rs`, `src/vector.rs`, `src/fill.rs`, `src/parser/vectors.rs`

Three optimisations delivered:

- **O8.1a** `OpPreAllocVector`: pre-allocates vector capacity for known-size
  literals, eliminating all `store.resize()` calls.  One new opcode (replaced
  unused `OpNop` slot).
- **O8.5** Constant comprehension unrolling: `[for i in 0..N { expr(i) }]`
  unrolled at compile time when bounds and body are const-evaluable.  10k limit.
- **`const_eval()`** module: compile-time constant folder for arithmetic, casts,
  comparisons, boolean ops across all numeric types.

**Impact:** For a 20-element constant vector, eliminates 1-2 resize allocations.
For constant comprehensions, eliminates the entire runtime loop.

Full design: the Constant Data section below.

---

---


# String Buffer Allocation and Optimization Opportunities

## Text type duality

Loft has two runtime representations for text:

| Type | Size | Heap? | Where used |
|---|---|---|---|
| `Str` | 16 bytes (ptr + len + pad) | No — borrows existing buffer | Arguments, temporaries on eval stack |
| `String` | 24 bytes (ptr + capacity + len) | Yes — owns heap buffer | Local variables, work texts |

The split is the primary optimization: text arguments are zero-copy
references into the caller's (or constant pool's) memory.

---

## Allocation lifecycle of a local text variable

```
OpText          →  24B String written to stack, zero heap (String::new())
OpAppendText    →  first append allocates heap buffer
OpClearText     →  .clear() — content gone, heap buffer preserved
OpAppendText    →  reuses existing buffer if it fits
OpFreeText      →  .shrink_to(0) — deallocates heap buffer
```

Key insight: `String::new()` is free (no heap allocation).  The real
cost is the **first `OpAppendText`** which triggers a heap allocation.
Subsequent reassignments via `OpClearText` + `OpAppendText` often
reuse the existing buffer.

---

## Where copies actually happen

| Situation | What happens | Heap alloc? |
|---|---|---|
| Text argument passing | 16B Str reference pushed | **No** |
| `OpVarText` (read local) | Create 16B Str view of 24B String | **No** |
| `OpArgText` (read param) | Read existing 16B Str | **No** |
| **`x = "hello"` (first)** | OpText + OpAppendText | **Yes** — one alloc |
| **`x = y` (text copy)** | OpText + OpVarText + OpAppendText | **Yes** — copy into new buffer |
| **`x = y + z` (concat)** | Work text + 2× OpAppendText | **Yes** — work text buffer grows |
| `x = func()` (text return) | Destination passing via RefVar(Text) | **No extra** — writes into x directly |
| `x = "new"` (reassign) | OpClearText + OpAppendText | **Usually no** — reuses buffer |
| Work text reuse | OpClearText | **No** — keeps capacity |

### Destination passing (already optimized)

Text-returning functions use `RefVar(Text)`: the caller's String
buffer is passed as an implicit parameter, and the callee writes
directly into it.  No intermediate copy.

```
fn greet(name: text) -> text {
  "hello " + name       // writes directly into caller's buffer
}
result = greet("world"); // result's String IS the buffer
```

This is implemented in `codegen.rs:gen_text_dest_call` (~line 1858)
and `text_return()` in `control.rs`.

---

## Current efficiency assessment

The design is already quite efficient:

1. **Arguments**: Zero-copy Str references — best possible.
2. **Work texts**: Allocated once per function, reused across
   statements.  `.clear()` preserves capacity.
3. **Destination passing**: Text-returning functions avoid
   intermediate buffers entirely.
4. **Reassignment**: `.clear()` + append reuses the heap buffer.
5. **String::new()**: Zero-cost until first content — no
   speculative allocation.

The remaining overhead is **one heap allocation per mutable text
variable** on first content assignment.  This is inherent to the
owned-buffer design.

---

## Optimization opportunities

### O-S1. `String::clone()` for `x = y` — **Low value**

Currently `x = y` emits OpText (empty String) + OpVarText (read y) +
OpAppendText (copy into x).  This does: allocate empty → reallocate
to fit → copy.

A dedicated `OpCloneText` could do `String::clone()` directly: one
allocation at the correct size, one memcpy.  Saves one reallocation.

**Impact:** Marginal — `String::clone()` vs empty + append is ~10%
difference in microbenchmarks.  Not worth a new opcode.

### O-S2. Pre-sized allocation for known lengths — **Low value**

For `x = "long literal string"`, the compiler knows the length at
compile time.  `String::with_capacity(len)` would avoid the realloc
on first append.

**Impact:** Negligible — short strings (< 16 chars) are the common
case, and the allocator typically over-provisions anyway.

### O-S3. Copy-on-write (Cow) for read-only variables — **Medium value, high complexity**

If a text variable is assigned once and only read thereafter, it
could stay as a borrowed `Str` instead of copying into an owned
`String`.  This requires:
- Mutation analysis in the parser (which variables are never mutated?)
- A third text representation: `Cow<'a, str>` or similar
- Fallback path for variables that are later mutated

This is analogous to the auto-const analysis for struct parameters.
The compiler already knows (via `find_written_vars`) which variables
are mutated.

**Impact:** Eliminates heap allocation for read-only text variables.
Significant for programs that pass text through multiple layers
without modifying it.  But the P115 auto-promotion mechanism shows
that mutation detection is feasible — we could do the inverse: keep
as Str until first mutation, then promote.

**Risk:** Lifetime management.  The borrowed Str points into the
caller's memory.  If the caller's String is freed or reallocated
while the callee still holds a Str, we get use-after-free.  This
is safe today because Str arguments have function-call lifetime.
Extending to local variables requires proving the source outlives
the borrower.

### O-S4. Small-string optimization (SSO) — **High value, high complexity**

Store strings ≤ 22 bytes inline in the 24-byte stack slot instead
of heap-allocating.  This eliminates heap allocation for the vast
majority of strings in typical programs (names, labels, short
messages).

Requires replacing `String` with a custom `SmallString` type that
stores either inline data or a heap pointer.  Every text operation
(`OpAppendText`, `OpClearText`, `OpVarText`, `OpFreeText`) needs
to handle both representations.

**Impact:** High — eliminates ~80% of text heap allocations in
typical programs.  But the implementation cost is substantial.

---

## Recommendation

The current design is already well-optimized for the common cases.
The `Str`/`String` split, destination passing, and work-text reuse
handle the important paths.

**No immediate action needed.**  If profiling reveals text allocation
as a bottleneck, O-S3 (copy-on-write for read-only variables) is the
most impactful optimization that integrates with the existing
architecture.  O-S4 (SSO) delivers the highest raw improvement but
requires a custom string type that touches every text operation.

---


# Struct Passing, Copies, and Optimization Opportunities

## Loft parameter semantics

Loft passes ALL struct parameters by reference (shared DbRef).  There
is no implicit copy on function calls.  Mutation is the default:

```loft
fn modify(s: Point) { s.x = 99.0; }
fn main() {
  p = Point { x: 1.0, y: 2.0 };
  modify(p);
  // p.x is now 99.0 — caller's struct was mutated
}
```

The three parameter modes:

| Syntax | Semantics | Store locked? |
|---|---|---|
| `param: T` | Mutable reference — callee can mutate caller's data | No |
| `param: &T` | Mutable reference — same as above, explicit | No |
| `param: const T` | Read-only reference — store locked, writes panic | Yes |

---

## Where copies actually happen

Copies are NOT on parameter passing.  They happen on **first local
variable assignment** and **return values**.

### Copy landscape

| Situation | What happens | Cost |
|---|---|---|
| Parameter passing | DbRef shared (12 bytes) | **Zero** |
| Vector element `v[i]` | DbRef pointer arithmetic | **Zero** |
| For-loop iteration | DbRef per element | **Zero** |
| Local reassignment `x = y` | DbRef overwrite | **Zero** |
| **First assignment `x = func()`** | OpCopyRecord deep copy | **Expensive** |
| **First assignment `x = y`** (same struct type) | OpCopyRecord deep copy | **Expensive** |
| **Return values** | copy_block (byte copy) | **Moderate** |
| Const lock check | Bool assert per write | Negligible |

### When OpCopyRecord fires

Only three cases in `gen_set_first_at_tos` (codegen.rs):

1. **`x = func_returning_struct()`** — function return assigned to new
   local variable.  Emits OpConvRefFromNull + OpDatabase + OpCopyRecord.
   Deep copies all fields including nested vectors, text, sub-structs.

2. **`x = y`** where both are same struct type and x is uninitialized —
   same deep copy to give x its own independent store.

3. **Tuple destructuring** `(a, b) = expr` where an element is a
   Reference — deep copy for the extracted element.

### What OpCopyRecord costs

Runtime at `state/io.rs:932`:
```
copy_block(&data, &to, size)     — raw byte copy of struct fields
copy_claims(&data, &to, tp)      — deep copy of nested structures
```

For `Mat4` (16 × f64 + vector wrapper): ~128 bytes + vector record.
For `Scene` with meshes/materials/nodes: hundreds of bytes + all vectors.

### Return value copy (latent issue)

`state/mod.rs:1032` copies return values with `copy_block` only — no
`copy_claims`.  This is a shallow byte copy.  If a returned struct
contains owned nested references (vectors, text), the returned DbRef
shares them with the callee's about-to-be-freed store.  **Potential
use-after-free for complex return types.**

---

## Optimization 1: Move semantics for return values

### Problem

`br_mvp = rect_mvp(proj, x, y, w, h)` — called 60×/frame in Brick Buster.
Each call: callee constructs Mat4, returns it, caller OpCopyRecord deep
copies it into `br_mvp`'s store.  The callee's original is immediately
freed.  The copy is wasted — the data could transfer ownership.

### Fix: return slot pre-allocation (destination passing)

The caller pre-allocates the destination store and passes a DbRef to the
callee.  The callee writes directly into it.  No copy on return.

```
Before:                              After:
  callee: build Mat4 in local store    callee: build Mat4 in caller's store
  return: copy_block to caller         return: nothing (already there)
  caller: OpCopyRecord to br_mvp      caller: nothing (already in br_mvp)
```

This pattern already exists for text-returning functions
(`try_text_dest_pass` in codegen.rs).  Extending it to struct returns
is the natural next step.

### Implementation

**File:** `src/state/codegen.rs`

1. When generating a function call whose return type is `Reference`:
   - If the result is assigned to a local variable (`x = func(...)`),
     pass `x`'s store DbRef as a hidden first parameter
   - The callee writes into that store instead of its own local
   - Return is a no-op (data already in the right place)

2. Requires the callee to be aware of the destination.  Two options:
   - **Implicit:** codegen detects struct construction and redirects writes
   - **Explicit:** new `__dest` hidden parameter (like text_return)

### Impact

| Function | Calls/frame | Bytes saved per call |
|---|---|---|
| `rect_mvp()` | 60 | ~128 bytes + vector overhead |
| `mat4_mul()` | 60 | ~128 bytes |
| `mat4_perspective()` | 1 | ~128 bytes |
| `mat4_look_at()` | 1 | ~128 bytes |

**~15 KB/frame** eliminated in Brick Buster.  Proportionally more in the
renderer (PBR pass constructs Mat4 per node).

---

## Optimization 2: Last-use move (elide copy when source dies)

### Problem

```loft
a = Point { x: 1.0, y: 2.0 };
b = a;       // OpCopyRecord — deep copy
// a is never used again
```

The copy is unnecessary — `a`'s store could be transferred to `b`.

### Fix: last-use analysis

If `x = y` and `y` is never read again after this point (last use),
transfer `y`'s DbRef to `x` and null out `y`.  No copy needed.

The variable liveness analysis in `src/variables/` already tracks
`first_def` and `last_use`.  If `last_use(y) == current_statement`,
it's safe to move.

### Implementation

**File:** `src/state/codegen.rs`, in `gen_set_first_at_tos`

Before emitting OpCopyRecord for `x = y`:
```rust
if let Value::Var(src) = value
    && stack.function.last_use(*src) == current_def_position
{
    // Move: transfer src's DbRef to x, no copy
    let src_pos = stack.position - stack.function.stack(*src);
    stack.add_op("OpVarRef", self);
    self.code_add(src_pos);
    // Mark src as moved — OpFreeRef will skip it
    return;
}
```

### Impact

Eliminates copies for temporary struct results that are immediately
assigned and never reused.  Common in builder patterns:

```loft
m = mat4_translate(1.0, 2.0, 3.0);      // result → m (move, no copy)
mvp = mat4_mul(proj, mat4_mul(view, m)); // inner result → temp (move)
```

---

## Optimization 3: Auto-const inference (safety, not performance)

### Purpose

Not a performance optimization (parameters aren't copied).  Instead:
auto-lock stores for provably unwritten parameters to catch accidental
mutation bugs at runtime.

### When to auto-lock

A struct parameter can be auto-locked when:
- Never directly written (`param.field = x`)
- Never appended to (`param.vec += [x]`)
- Never passed as `&T` to another function
- **Never passed as plain `T` to a non-const function** (conservative —
  callee might mutate through the shared reference)

### Implementation

1. Add `auto_const: bool` to Variable
2. Run `find_written_vars()` at end of first pass
3. Add escape analysis: check if param is passed to any function call
   where the receiving parameter is not `const`
4. Lock store at function entry for auto-const params

### Compiler warning

When inference succeeds:
```
Warning: parameter 's' is never mutated — consider adding 'const'
```

---

## Test cases

### Test 1: mutation through plain parameter (current behavior, correct)

```loft
struct S { x: integer not null }
fn modify(s: S) { s.x = 99; }
fn main() {
  p = S { x: 1 };
  modify(p);
  assert(p.x == 99, "mutation visible to caller");
}
```

### Test 2: const parameter locks store

```loft
struct S { x: integer not null }
fn read(s: const S) -> integer { s.x }
fn main() {
  p = S { x: 1 };
  assert(read(p) == 1, "const read works");
}
```

### Test 3: const prevents mutation via &T (runtime panic)

```loft
struct S { x: integer not null }
fn mutate_ref(m: &S) { m.x = 99; }
fn bad(s: const S) { mutate_ref(s); }
fn main() { bad(S { x: 1 }); }
// Panics: "Write to locked store"
```

### Test 4: escape to non-const blocks auto-lock

```loft
struct S { x: integer not null }
fn helper(s: S) { s.x = 42; }
fn caller(s: S) {
  helper(s);  // s escapes to mutable function — cannot auto-lock
}
```

### Test 5: return value copy (current behavior)

```loft
struct Point { x: float not null, y: float not null }
fn make() -> Point { Point { x: 1.0, y: 2.0 } }
fn main() {
  p = make();       // OpCopyRecord fires here
  q = p;            // OpCopyRecord fires here
  q.x = 99.0;
  assert(p.x == 1.0, "p isolated from q after copy");
}
```

### Test 6: move optimization target

```loft
fn make() -> Point { Point { x: 1.0, y: 2.0 } }
fn main() {
  p = make();       // Could be a move (no copy) if dest-passing works
  println("{p.x}");
}
```

---

## Priority order

| # | Optimization | Impact | Effort | Risk |
|---|---|---|---|---|
| 1 | Return slot / destination passing | ~15 KB/frame in games | M | Low — text_return already does this |
| 2 | Last-use move for `x = y` | Eliminates temp copies | S | Low — liveness data available |
| 3 | Auto-const inference | Safety, not perf | M | Medium — needs escape analysis |

---

## Related

- [PERFORMANCE.md](PERFORMANCE.md) — benchmark data and optimization plan
- Optimisations section below — planned interpreter optimizations
- [SLOTS.md](SLOTS.md) — stack slot assignment design

---


# Block Copy Efficiency: Analysis and Recommendations

## What's actually expensive

Block copy has two phases:

| Phase | Function | Cost | What it does |
|---|---|---|---|
| 1. `copy_block` | `structures.rs:777` | O(struct_size) memcpy | Raw byte copy of struct fields |
| 2. `copy_claims` | `allocation.rs:642` | O(total_owned_data) + allocations | Deep copy of ALL owned sub-structures |

Phase 1 is cheap — typically 4-128 bytes of memcpy.

Phase 2 is the real cost.  For each field, `copy_claims` recursively:
- **Text fields:** re-interns the string (allocate store record + memcpy)
- **Vectors:** allocates new vector record, copies header + all elements,
  then recursively copies each element's owned data
- **Nested structs:** recurses into each struct field
- **Arrays/Hash/Index:** O(n) allocations, each with recursive traversal

A `Mat4` with a `vector<float>` costs: 1 store allocation + vector
allocation + 16 floats copied.  A `Scene` with meshes/materials/nodes:
dozens of allocations, hundreds of bytes.

## Where deep copies happen today

Deep copies (OpCopyRecord) fire in exactly three codegen paths, all in
`gen_set_first_at_tos` (codegen.rs:931-983):

```
x = func()      →  gen_set_first_ref_copy      →  OpCopyRecord
x = y            →  gen_set_first_ref_var_copy  →  OpCopyRecord
(a, b) = expr    →  gen_set_first_ref_tuple_copy →  OpCopyRecord
```

Each emits: `OpConvRefFromNull` → `OpDatabase` → `OpCopyRecord`.

Return values themselves are cheap (12-byte DbRef shallow copy in
`copy_result`).  The deep copy only fires at first assignment.

## Optimization candidates

### O-B1. Last-use move — **IMPLEMENT THIS**

**Pattern:**
```loft
temp = compute();
result = temp;     // temp never used again → move DbRef, skip copy
```

**Detection:** At the `x = y` codegen site (`gen_set_first_ref_var_copy`),
check `stack.function.last_use(src) <= stack.function.first_def(v)`.
If true, the source variable is never read after this assignment.
Transfer the DbRef instead of deep copying.

**Implementation:**
```rust
// In gen_set_first_ref_var_copy, before emitting OpCopyRecord:
if stack.function.last_use(src) <= stack.function.first_def(v) {
    // Move: just copy the DbRef (12 bytes), skip deep copy.
    // Mark src as moved so OpFreeRef skips it.
    stack.add_op("OpVarRef", self);
    let src_pos = stack.position - stack.function.stack(src);
    self.code_add(src_pos);
    stack.function.set_skip_free(src);
    return;
}
```

**Impact:** Eliminates ALL deep copies for temporary-to-final patterns.
Common in math-heavy code:
```loft
m = mat4_identity();        // allocate + build
mvp = mat4_mul(proj, m);    // m's last use → move, not copy
```

**Complexity:** S — 5-10 lines in one function.  No new opcodes,
no ABI changes.  Uses existing `last_use`, `first_def`, `skip_free`.

**Risk:** Low.  The liveness analysis is already computed and used
for slot assignment.  `skip_free` already exists for other purposes.
Must verify that `last_use` accounts for implicit frees (OpFreeRef)
— if it does, the check is safe.

---

### O-B2. Last-use move for function returns — **IMPLEMENT THIS**

**Pattern:**
```loft
result = make_point();   // function returns struct, immediately assigned
// make_point's return value is never aliased
```

Currently: function returns DbRef (shallow), then `gen_set_first_ref_copy`
deep copies it.  The return value's store is freed immediately after.

**Detection:** The RHS is `Value::Call(OpCopyRecord, args)` where args[0]
is a function call.  The intermediate DbRef from the function call is
always a temporary — it has no variable, so it's always "last use".

**Implementation:** In `gen_set_first_ref_copy`, when the inner call
is a user function (not OpCopyRecord itself), the return value is a
one-shot temporary.  Skip the OpCopyRecord and directly adopt the
returned DbRef.

However, there's a subtlety: the returned DbRef points into the
callee's store, which may be freed.  Need to verify store lifetime.
If the callee's store outlives the return (it should — stores are
not freed until explicit OpFreeRef), this is safe.

**Impact:** Eliminates deep copies for `x = func()` patterns.

**Complexity:** M — needs careful store lifetime analysis.

---

### O-B3. Destination passing for struct returns — **DEFER**

**Pattern:** Extend the text `RefVar(Text)` destination-passing
mechanism to struct-returning functions.  Caller pre-allocates the
destination store, callee writes directly into it.

**Why defer:** Requires ABI changes (hidden parameter), callee
rewriting to use the destination store for all field writes, and
interaction with existing OpCopyRecord codegen.  The last-use move
(O-B1) handles the most common cases with much less complexity.

**When to revisit:** If profiling shows that `x = func()` copies
remain a bottleneck after O-B1/O-B2, destination passing is the
next step.

---

### O-B4. Shallow copy for immutable borrows — **DEFER**

**Pattern:** `x = y` where x is never mutated — share the DbRef
instead of deep copying.

**Why defer:** Requires copy-on-write or reference counting to
prevent double-free and aliasing bugs.  The auto-const analysis
(CONST_REF.md O3) would need to run first to identify which
variables are immutable.  High complexity for moderate gain.

---

## Status after O-B1

**O-B1 is implemented** (codegen.rs `gen_set_first_ref_var_copy`).
When `x = y` and y has `uses == 1` (only read here), not an argument,
not captured: emits OpVarRef + skip_free instead of the full deep copy.

### Remaining deep copy sites

| Site | Codegen function | Pattern | Frequency |
|---|---|---|---|
| 1 | `gen_set_first_ref_copy` | `x = func()` | **Very common** — every struct-returning call |
| 2 | `gen_set_first_ref_var_copy` | `x = y` (uses > 1) | Rare — O-B1 handles uses == 1 |
| 3 | `gen_set_first_ref_tuple_copy` | `(a, b) = expr` | Rare — tuple destructuring |

**Site 1 is the dominant remaining cost.** Every `m = mat4_mul(a, b)`
allocates a fresh store, deep copies, and **leaks the callee's store**.

### Store leak on struct returns

When a function returns a struct, the callee's store is kept alive
(scopes.rs `in_ret` check skips OpFreeRef).  After the caller deep
copies from it, nobody frees it.  This is a latent store leak that
grows linearly with struct-returning calls.

## Recommendation

### O-B2: Return store adoption — **IMPLEMENT NEXT**

For `x = func()`, the source is always a temporary on the eval stack
(no variable holds it).  Instead of allocating a NEW store + deep
copy, adopt the returned DbRef directly.  This fixes BOTH the copy
cost AND the store leak.

**Safety concern:** The returned DbRef might point to a parameter's
store (e.g. `fn identity(p: Point) -> Point { p }`).  Adopting it
would cause the caller to free a shared store.

**Safe implementation:** After the function call + OpCopyRecord runs,
free the source store.  The deep copy is still performed, but the
leak is fixed.  Then separately optimize the copy away for provably
fresh returns (callee constructs a new struct, never returns a param).

**Detecting fresh returns:** A function whose return type has empty
dependencies (`dep.is_empty()`) and whose return expression is a
struct constructor or a call to another struct-returning function
(not a Var pointing to a parameter) is safe to adopt.

**Complexity:** M — two-phase: (1) fix the leak (S), (2) skip copy
for fresh returns (M, needs callee analysis).

### O-B3 and O-B4 — **DEFER**

Destination passing (O-B3) is the clean long-term solution but
requires ABI changes.  Shallow copy for immutables (O-B4) needs
copy-on-write.  Both deferred until the simpler O-B2 is in place.

### Expected savings after O-B1 + O-B2

| Pattern | Current cost | After O-B1+O-B2 |
|---|---|---|
| `m = mat4_identity(); mvp = m` | Deep copy | 12B DbRef move (O-B1) |
| `result = temp_struct` where temp dies | Deep copy | 12B DbRef move (O-B1) |
| `x = func()` (func builds new struct) | Deep copy + store leak | 12B DbRef adopt (O-B2) |
| `x = func()` (func returns param) | Deep copy + store leak | Deep copy, no leak (O-B2 phase 1) |
| Loop: `acc = transform(acc)` | Deep copy per iter | 12B move per iter (O-B1) |

For Brick Buster (60 rect_mvp/frame): ~15 KB deep copies + 60 leaked
stores/frame → 720B moves + 0 leaks.

---

## Implementation status

| Optimisation | Status | Issue |
|---|---|---|
| O-B1: last-use move `x = y` | **Done** | — |
| O-B2: adoption for no-ref-param functions | **Done** (codegen branch for `n_*` functions) | — |
| O-B2: deep copy for ref-param functions | **Partially done** (`gen_set_first_ref_call_copy`) | P116 |
| Store leak fix (callee store after copy) | **Partial** — O-B2 adoption fixes no-ref-param case | P117 |
| Threading regression | **Blocked** — needs investigation | P118 |

### Known issues found during optimisation

- **P116**: `x = func(s)` where func has Reference params aliases
  the store.  Codegen branch added but needs regression testing.
- **P117**: Store leak for struct-returning functions.  Fixed for
  no-ref-param functions by O-B2 adoption.  Remaining: ref-param case.
- **P118**: `22-threading.loft` panics "Incomplete record" after
  P64/P66 checked arithmetic changes.  Not yet diagnosed.

---

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
- Optimisations section below — Runtime optimisation audit
- [PERFORMANCE.md](PERFORMANCE.md) — Benchmark data and root-cause analysis
- [INTERMEDIATE.md](INTERMEDIATE.md) — Bytecode layout and State stack model
- [DATABASE.md](DATABASE.md) — Store allocator and `copy_block` API
