# Threading Interface

## Contents
- [Current State](#current-state)
- [`fn` Expression](#fn-expression)
- [`parallel_for` Call Rewriting](#parallel_for-call-rewriting)
- [Runtime](#runtime)
- [Compiler Validation Summary](#compiler-validation-summary)
- [`par(...)` Parallel For-Loop Syntax](#par-parallel-for-loop-syntax)
- [Deferred: Extra Worker Parameters](#deferred-extra-worker-parameters)
- [Deferred: Text and Reference Return Types](#deferred-text-and-reference-return-types)

---

## Current State

The public API for parallel execution is the `par(...)` for-loop clause.  The internal functions `parallel_for_int`, `parallel_for`, and `parallel_get_*` are declared without `pub` in `default/01_code.loft` and must not be called directly from user code.

Function references (`fn <name>`) are now fully first-class (T1-1 complete): they can be stored in variables of type `fn(T) -> R`, passed as parameters, and called directly via `f(args)`. See the [`fn` Expression](#fn-expression) section for details.

### `par(...)` Parallel For-Loop (public API)

See the [par(...) Parallel For-Loop Syntax](#par-parallel-for-loop-syntax) section below.

### Internal Primitives (not public)

#### `parallel_for_int`

```loft
fn parallel_for_int(func: text, input: reference,
                    element_size: integer, threads: integer) -> reference
```

Legacy internal: function name is a runtime string (no compiler check), return type is always `integer`, element size must be supplied manually.

#### `parallel_for` (compiler-checked, internal)

```loft
fn parallel_for(input: reference, element_size: integer, return_size: integer,
                threads: integer, func: integer) -> reference
```

Emitted by the compiler when rewriting `par(...)` clauses.  The user-facing form is the `par(...)` clause; this function is not callable directly.

Worker rules: see `par(...)` Parallel For-Loop Syntax below.

---

## `fn` Expression

A `fn <name>` expression in value position produces a `Type::Function(args, ret)` value.  The runtime representation is the definition number (`d_nr`) stored as an `i32`.

```loft
f = fn double_score;   // type: fn(const Score) -> integer
                       // runtime value: d_nr of double_score
```

**Compile-time resolution:**
- Tries `n_<name>` first (user function naming convention).
- Falls back to bare `<name>` (methods, operators).
- Emits a diagnostic error if neither resolves.
- The `Type::Function` carries full argument type and return type metadata.

**No new bytecode opcode** — compiles to `OpInt(d_nr)`.

**Callable fn-ref variables (T1-1, complete):** A local variable or parameter of type
`Type::Function` can be called directly: `f(args)`. `parse_call` detects the
`Type::Function` case and emits `Value::CallRef(var_nr, args)` instead of `Value::Call`.
At bytecode generation `generate_call_ref` emits `OpCallRef` (op_code 252). The runtime
looks up the entry point in `State::fn_positions` and dispatches via `fn_call`.

`fn(T) -> R` is also a valid parameter type, enabling higher-order functions:
```loft
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
```

---

## `parallel_for` Call Rewriting

The parser special-cases calls to `parallel_for` in `parse_call` (similar to `assert`).  After collecting the argument list, it calls `parse_parallel_for` which:

1. Verifies `types[0]` is `Type::Function(args, ret)` (produced by `fn <name>`).
2. Verifies `types[1]` is `Type::Vector(T, _)`.
3. Checks worker return type is a supported primitive.
4. Validates extra arg count == worker's extra param count.
5. Computes `element_size = database.size(T.known_type)` (actual inline storage size).
6. Computes `return_size` (1/4/8 bytes).
7. Emits `Value::Call(n_parallel_for_d_nr, [input, elem_size, return_size, threads, func])`.

The internal native function `n_parallel_for` (registered in `native.rs::FUNCTIONS`) has the loft declaration:

```loft
fn parallel_for(input: reference, element_size: integer, return_size: integer,
                threads: integer, func: integer) -> reference;
```

`input` is listed first so that `gather_key` in `generate_call` does not misread the integer `func` d_nr as a key count.

---

## Runtime

### `execute_at_raw` (state.rs)

```rust
pub fn execute_at_raw(&mut self, fn_pos: u32, arg: &DbRef, return_size: u32) -> u64
```

Sets up the same `[arg: DbRef][return-addr u32::MAX]` stack layout as `execute_at`.  After execution, pops the result using the correct width:

| return_size | pop method | Rust type |
|---|---|---|
| 8 | `get_stack::<u64>()` | `i64`/`f64` bit pattern |
| 4 | `get_stack::<u32>()` | `i32` bit pattern |
| 1 | `get_stack::<u8>()` | `bool` as 0/1 |

### `run_parallel_raw` (parallel.rs)

```rust
pub fn run_parallel_raw(
    stores, program, fn_pos, input, element_size, return_size, n_threads
) -> Vec<u64>
```

Generalisation of `run_parallel_int`.  Each worker calls `execute_at_raw` and stores the raw bits in a `u64`.  The main thread assembles results in order.

### `n_parallel_for` (native.rs)

Pops (reverse declaration order): `func`, `threads`, `return_size`, `element_size`, `input`.  Calls `run_parallel_raw`, then builds the result vector:

| return_size | store method |
|---|---|
| 8 | `set_long(rec, fld, bits as i64)` |
| 4 | `set_int(rec, fld, bits as i32)` |
| 1 | `set_byte(rec, fld, 0, bits as i32)` |

---

## Compiler Validation Summary

| Check | Location | Error |
|---|---|---|
| `fn <name>` names an existing function | `parse_fn_ref` | `"Unknown function '{name}'"` |
| `fn <name>` resolves to a `DefType::Function` | `parse_fn_ref` | `"'{name}' is not a function"` |
| First `parallel_for` arg is `Type::Function` | `parse_parallel_for` | `"first argument must be a function reference (use fn <name>)"` |
| Second arg is `Type::Vector` | `parse_parallel_for` | `"second argument must be a vector"` |
| Worker return type is primitive | `parse_parallel_for` | `"worker return type '…' must be integer, long, float, or boolean"` |
| Extra arg count matches worker | `parse_parallel_for` | `"wrong number of extra arguments"` |

---

## `par(...)` Parallel For-Loop Syntax

The `par(b=worker(a), N)` clause on a `for ... in` loop is a shorthand that runs the worker in parallel over the vector and iterates the results in order.

### Syntax

```loft
for a in <vector> par(b=<worker_call>, <threads>) {
    // body — b holds the worker result for element a
}
```

Two worker call forms are supported:

| Form | Example | Description |
|---|---|---|
| Form 1 | `func(a)` | Global/user function; `a` is the loop element |
| Form 2 | `a.method()` | Method on the element type |

Form 3 (`c.method(a)` — captured receiver) is detected but not yet implemented.

### Desugaring

```
par_len   = length(vector)
par_results = parallel_for(vector, elem_size, return_size, threads, fn_d_nr)
for b#index in 0..par_len {
    b = parallel_get_T(par_results, b#index)
    <body>
}
```

### Limitations

- Input must be a `vector<T>`; integer ranges (`1..10`) are not supported.
- Worker must return a primitive: `integer`, `long`, `float`, or `boolean`.
- Form 3 (captured receiver) requires IR-level wrapper synthesis (deferred).
- The worker function may not write to shared state.
- **Float/long accumulation**: using the result variable `b` in arithmetic with a pre-declared `float` or `long` variable can trigger a first-pass type-inference conflict (`Variable 'x' cannot change type from float to integer`).  Workaround: only use `b` in boolean comparisons (`if b > threshold { count += 1; }`) or cast to integer (`sum += b as integer`) inside the body.

### Element Size

Element size is computed from `self.database.size(element_type.known_type)` — the actual inline struct field size (e.g. 4 for `Score{value:integer}`, 8 for `Range{lo,hi:integer}`), NOT `size_of::<DbRef>()`.

### Multi-threading Safety

`Stores::clone_for_worker()` creates locked copies of all in-use stores for each worker thread. Freed store slots (`.free == true`) are replaced with fresh unlocked `Store::new(100)` instances so that `State::new_worker → Stores::database` can safely re-initialise them without hitting the "Write to locked store" debug assert.

### Example

```loft
fn double_score(r: const Score) -> integer { r.value * 2 }
fn get_value(self: const Score) -> integer { self.value }

fn main() {
    q = make_score_items();   // [10, 20, 30]

    // Form 1: global function
    sum = 0;
    for a in q.items par(b=double_score(a), 4) {
        sum += b;   // b = 20, 40, 60  → sum = 120
    }

    // Form 2: method
    total = 0;
    for a in q.items par(b=a.get_value(), 1) {
        total += b;  // b = 10, 20, 30  → total = 60
    }
}
```

---

## Deferred: Extra Worker Parameters

Passing additional parameters beyond the row reference to the worker is not yet implemented.  The validation infrastructure is already in place (`n_extra == n_worker_extra` check in `parse_parallel_for`); only the emission path is missing.

**Planned approach — extend `execute_at_raw`** (no IR wrapper synthesis required):

1. Add `extra_args: &[u64]` to `execute_at_raw` and push those values onto the call stack before the row ref, in declaration order.
2. `run_parallel_raw` receives the captured extra arg values as a `Vec<u64>` (read-only constants, cloned to every worker).
3. The compiler emits the extras as additional parameters in the `n_parallel_for` call.

Supported extra arg types:
- `integer`, `long`, `float`, `boolean` — fit directly in a `u64` slot.
- `const Struct` — 12-byte DbRef; pass as an `Option<DbRef>` context alongside the row ref rather than folding into `u64`.
- `text` — already readable from cloned stores via their DbRef; no special handling needed.

This is planned for A1.1 (0.8.2).

---

## Deferred: Value-Struct and Text/Reference Return Types

**Value-struct returns (no heap pointers) — A1.1:**
For worker return types where all fields are primitives (`integer`, `long`, `float`, `boolean`, `character`), the `Vec<u64>` result channel is replaced with a pre-allocated `Vec<u8>` output buffer of size `n_rows × result_byte_size`.  Workers write directly into non-overlapping per-row slices via `execute_at_struct(fn_pos, row_ref, out_slice: &mut [u8])`.  No store interaction needed.  Structs with `text` or `reference` fields use the approach below.

**Text and reference return types — A1.2:**
`text` and `reference` values are DbRefs that point into a specific store.  Worker stores are LIFO-locked snapshots; new allocations in a worker are invisible to the main thread after join, and ad-hoc merging is unsafe due to LIFO ordering constraints.

*Planned approach — dedicated result store:*
Before parallel dispatch the main thread calls `Stores::new_result_store()`, which allocates a fresh writable store not included in the workers' input snapshots.  `clone_for_worker` gives each worker mutable access to this result store, range-partitioned by row index.  Workers redirect text allocations to the result store.  After join, `Stores::adopt_result_store(idx)` incorporates the result store into the main store map.  Since the result store did not exist in any input snapshot, there are no LIFO conflicts.

This is planned for A1.2 (0.8.2), dependent on A1.1.

---

## See also
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/state/`, store cloning for workers
- [STDLIB.md](STDLIB.md) — `par(...)` parallel for-loop user-facing API
- [PLANNING.md](PLANNING.md) — A1 (parallel workers: extra args + text/ref returns)
