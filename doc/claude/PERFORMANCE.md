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

- [OPTIMISATIONS.md](OPTIMISATIONS.md) — runtime optimisation opportunities audit
- [PLANNING.md](PLANNING.md) — priority-ordered enhancement backlog
- [INTERNALS.md](INTERNALS.md) — `src/fill.rs`, `src/state/`, `src/generation/`
- [NATIVE.md](NATIVE.md) — native code generation design and known issues
- [doc/00-performance.html](../00-performance.html) — rendered benchmark page with bar charts
