---
render_with_liquid: false
---
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Stack Trace Design

> **Status: planned — not yet implemented.**

`stack_trace()` returns a snapshot of the current call stack as a
`vector<StackFrame>`, giving loft programs structured access to function names,
source locations, and live argument values at the point of the call.

---

## Contents

- [Goals](#goals)
- [Exposed Types](#exposed-types)
- [API](#api)
- [Usage Examples](#usage-examples)
- [Runtime Design](#runtime-design)
- [Safety Concerns and Mitigations](#safety-concerns-and-mitigations)
- [Implementation Phases](#implementation-phases)
  - [Phase 1 — Shadow call-frame vector](#phase-1--shadow-call-frame-vector-srcstatemodrs)
  - [Phase 2 — Type declarations](#phase-2--type-declarations-default04_stacktracelor)
  - [Phase 3 — Materialisation](#phase-3--materialisation-srcstatemodrs-or-srcfillrs)
  - [Phase 4 — Call-site line numbers](#phase-4--call-site-line-numbers-srcstatecodegen.rs)
  - [Phase 5 (optional) — Debug symbol table](#phase-5-optional--debug-symbol-table-srcdatars-srcstatecodegen.rs)
  - [Phase 6 (optional) — `stack_trace_full()` and local variables](#phase-6-optional--stack_trace_full-and-local-variables)
- [Known Limitations](#known-limitations)
- [Non-Goals](#non-goals)
- [See also](#see-also)

---

## Goals

- Let a loft program inspect its own call stack at runtime without leaving the language.
- Provide structured data (names, file, line, typed argument values) rather than a raw
  text dump.
- Keep the hot path (normal execution) to a single push/pop per call frame.
- Fit into the existing native-function and opcode infrastructure without changing the
  calling convention for ordinary functions.
- (Optional, debug builds only) Expose live local variables alongside parameters via
  `stack_trace_full()`.

---

## Exposed Types

All types are declared `pub` in a new default library file `default/04_stacktrace.loft`,
loaded after `03_text.loft`.

### `ArgValue` — typed union of inspectable argument values

```loft
pub enum ArgValue {
    NullVal,
    BoolVal   { b: boolean },
    IntVal    { n: integer },
    LongVal   { n: long },
    FloatVal  { f: float },
    SingleVal { f: single },
    CharVal   { c: character },
    TextVal   { t: text },
    RefVal    { store: integer, rec: integer, pos: integer },
    FnVal     { d_nr: integer },
    OtherVal  { description: text },
}
```

| Variant | Used for |
|---|---|
| `NullVal` | Any null value regardless of type |
| `BoolVal` | `boolean` argument |
| `IntVal` | `integer` and range-constrained integer (`u8`, `u16`, …) |
| `LongVal` | `long` |
| `FloatVal` | `float` |
| `SingleVal` | `single` |
| `CharVal` | `character` |
| `TextVal` | `text` — content heap-allocated independently of the stack frame |
| `RefVal` | `reference<T>`, struct-enum, `vector<T>`, collection — raw DbRef triple; valid at snapshot time only (see [SC-ST-6](#sc-st-6--refval-coordinates-silently-dangle-after-the-source-frames-stores-are-freed)) |
| `FnVal` | `fn(T) -> R` — definition number |
| `OtherVal` | Iterator state or any type without a direct scalar representation; `description` holds the loft type name (e.g. `"iterator<Item, integer>"`) |

`RefVal` exposes the raw `(store, rec, pos)` triple from the DbRef. The coordinates are
valid only at the instant `stack_trace()` is called; they may point to a reallocated or
freed record if the trace is retained after the source frame has returned (see
[Known Limitations](#known-limitations)).

### `ArgInfo` — one function argument

```loft
pub struct ArgInfo {
    pub name:      text,     // parameter name as declared in the function signature
    pub type_name: text,     // loft type as text: "integer", "text", "vector<Foo>", …
    pub value:     ArgValue, // inspectable value at the time of stack_trace()
}
```

### `VarInfo` — one local variable (Phase 6, debug builds only)

```loft
pub struct VarInfo {
    pub name:      text,     // variable name as declared in the source
    pub type_name: text,     // loft type as text
    pub value:     ArgValue, // inspectable value at the time of stack_trace_full()
}
```

### `StackFrame` — one call frame

```loft
pub struct StackFrame {
    pub function:  text,            // bare function name, e.g. "compute_score"
    pub file:      text,            // source file path of the function definition
    pub line:      integer,         // 1-based source line of the call site
    pub arguments: vector<ArgInfo>, // one entry per declared parameter, in declaration order
    pub variables: vector<VarInfo>, // live local variables — empty unless stack_trace_full()
}
```

`line` is the line of the **call site** (the instruction that invoked this function),
not the line of the function declaration. For the innermost frame (the frame that called
`stack_trace()` or `stack_trace_full()`) it is the line of that call expression.

`variables` is always present as a field (an empty vector costs one word). It is
populated only when `stack_trace_full()` is used **and** the function was compiled with
debug symbols (Phases 5–6). When `stack_trace()` is called, `variables` is empty for
every frame regardless of build mode.

---

## API

```loft
// Phases 1–4: always available
pub fn stack_trace() -> vector<StackFrame>;

// Phases 5–6: debug builds only
pub fn stack_trace_full() -> vector<StackFrame>;
```

Both functions return the call stack as a vector of frames ordered **outermost first**:
index 0 is the entry point (`main` or test runner); the last element is the direct
caller. The vector is fully materialised at the moment of the call; mutations to the
live stack after the call do not affect it.

`stack_trace()` leaves `variables` empty in every frame.

`stack_trace_full()` also populates `variables` for each frame with every local variable
that is live at the call site according to the compiler's debug symbol table. If debug
symbols are absent (non-debug build), it aborts with:

```
stack_trace_full() requires debug symbols; rebuild with the debug_symbols flag
```

---

## Usage Examples

### Print a stack trace on error

```loft
fn assert_positive(n: integer) {
    if n <= 0 {
        for frame in stack_trace() {
            println("{frame.file}:{frame.line}  {frame.function}");
            for arg in frame.arguments {
                println("  {arg.name}: {arg.type_name} = {inspect_arg(arg.value)}");
            }
        }
        assert(false, "n must be positive");
    }
}

fn inspect_arg(v: ArgValue) -> text {
    match v {
        NullVal                       => "null",
        BoolVal   { b }               => "{b}",
        IntVal    { n }               => "{n}",
        LongVal   { n }               => "{n}",
        FloatVal  { f }               => "{f}",
        SingleVal { f }               => "{f}",
        CharVal   { c }               => "'{c}'",
        TextVal   { t }               => "\"{t}\"",
        RefVal    { store, rec, pos } => "ref({store},{rec},{pos})",
        FnVal     { d_nr }            => "fn#{d_nr}",
        OtherVal  { description }     => "<{description}>",
    }
}
```

### Capture a snapshot for logging

```loft
fn risky_operation(data: reference<Record>, threshold: integer) {
    trace = stack_trace();
    log_info("entering risky_operation, depth={len(trace)}");
    // ...
}
```

### Inspect argument types at a given depth

```loft
fn debug_caller() {
    frames = stack_trace();
    if len(frames) >= 2 {
        caller = frames[len(frames) - 2];
        println("called from {caller.function} with {len(caller.arguments)} args");
        for arg in caller.arguments {
            println("  {arg.name}: {arg.type_name}");
        }
    }
}
```

### Inspect local variables (debug builds only)

```loft
fn diagnose() {
    frames = stack_trace_full();
    for frame in frames {
        println("{frame.function}:");
        for arg in frame.arguments {
            println("  param {arg.name} = {inspect_arg(arg.value)}");
        }
        for v in frame.variables {
            println("  local {v.name} = {inspect_arg(v.value)}");
        }
    }
}
```

---

## Runtime Design

### Why a dedicated `call_stack` vector is needed

The existing bytecode stack stores return addresses inline — a `u32` code position
written by `fn_call` at `stack_pos + args_size`. Walking backwards from the current
frame to reconstruct earlier frames requires knowing each frame's `args_size`, which is
only available from the bytecode stream, not from the stack itself. Reconstructing it at
trace time would require parsing each function's entry sequence, which is expensive and
fragile.

The clean solution is a **shadow call-frame vector** on `State` that `fn_call` pushes
to and `fn_return` pops from. The overhead is one `Vec::push` + one `Vec::pop` per
call, negligible compared with the function dispatch itself.

### `CallFrame` — internal shadow frame (Rust)

```rust
struct CallFrame {
    d_nr:       u32,  // definition number of the called function
    call_pos:   u32,  // bytecode position of the call instruction (line-number lookup)
    args_base:  u32,  // absolute stack position of the first argument byte
    args_size:  u16,  // total byte size of all parameters (sum of size_of each type)
    local_size: u16,  // total local-variable space reservation (needed by Phase 6)
}
```

`args_base` is the absolute position on the bytecode stack at which the first parameter
byte sits. Parameters are laid out contiguously in declaration order from that position.
The return address word is written immediately above `args_base + args_size`.
`local_size` is the maximum bytes reserved for local variables beyond the parameter
region; it is needed by Phase 6 to bounds-check local variable reads.

`State` gains a field:

```rust
call_stack: Vec<CallFrame>,
```

### `fn_call` — extended signature

`fn_call` is extended with two new explicit parameters
(see [SC-ST-4](#sc-st-4--fn_calls-_size-is-not-args_size-args_base-would-be-wrong)):

```rust
pub fn fn_call(&mut self, d_nr: u32, args_size: u16, local_size: u16, to: i32) {
    // All argument values have already been pushed onto the stack.
    // args_base is the stack position of the first argument byte.
    let args_base = self.stack_pos - u32::from(args_size);
    self.call_stack.push(CallFrame {
        d_nr,
        call_pos: self.code_pos,
        args_base,
        args_size,
        local_size,
    });
    // Write the return address at stack_pos + args_size, then jump.
    self.put_stack(self.code_pos);
    self.code_pos = to as u32;
}
```

`fn_return` pops the top frame after restoring `code_pos`:

```rust
// At the end of fn_return, after restoring code_pos:
self.call_stack.pop();
```

Every call site in `fill.rs` that currently passes `_size` to the old `fn_call`
must be updated:
- `args_size` = sum of `size_of(param.typedef)` for each parameter in
  `data.definitions[d_nr].attributes`; this is known at the `OpCall` emission site in
  codegen and encoded as a dedicated operand word.
- `local_size` = the existing `_size` value, passed through unchanged.
- `d_nr` = the definition index encoded in the `OpCall` operand (already present).

### `fn_call_ref` — indirect calls

`fn_call_ref` handles calls through a `fn(T) -> R` value (indirect call). It already
carries `arg_size: u16` and reads `d_nr` from the function reference on the stack. It
must be updated to also pass `local_size`, which it computes from
`data.definitions[d_nr].local_size` (the maximum local reservation stored on
`Definition` at codegen time). `fn_call_ref` then delegates to `fn_call(d_nr,
arg_size, local_size, to)`, so the push/pop bookkeeping is centralised.

Static calls through the `library` table (native Rust functions) are **not** pushed —
they have no loft definition number and no loft source position.

### `OpStackTrace` and `OpStackTraceFull` — dedicated opcodes

The `library` table gives native functions access to only `(&mut Stores, &mut DbRef)`.
Stack trace materialisation needs `&State`, `&Data`, and `&call_stack`, which are not
available through that interface.

Both functions are therefore implemented as **dedicated opcodes**, following the same
pattern as `OpCallRef` (direct `&mut self` access, not routed through `library`):

```rust
// In fill.rs:
OpStackTrace => {
    let result = self.materialise_stack_trace(&data, false);
    self.put_stack(result);                   // pushes the DbRef of vector<StackFrame>
}

OpStackTraceFull => {
    let result = self.materialise_stack_trace(&data, true);
    self.put_stack(result);
}
```

The boolean `full` flag selects whether to populate the `variables` field of each frame.
`put_stack` writes the `DbRef` (12 bytes: store_nr, rec, pos) back onto the loft stack,
where the type system sees it as `vector<StackFrame>`.

### Materialising argument values

`materialise_stack_trace(&mut self, data: &Data, full: bool) -> DbRef` proceeds as
follows:

**Step 1 — Snapshot** (mandatory, see [SC-ST-3](#sc-st-3--re-entrant-stack_trace-corrupts-call_stack-iteration)):

```rust
fn materialise_stack_trace(&mut self, data: &Data, full: bool) -> DbRef {
    let frames = self.call_stack.clone();   // snapshot — must be first
    // ...
}
```

The live `self.call_stack` may be mutated by any loft call during materialisation
(e.g. store allocation). The snapshot isolates iteration from those mutations.

**Step 2** — Allocate a `vector<StackFrame>` store in `self.stores`.

**Step 3** — For each `CallFrame` in the snapshot, outermost first:

a. Look up `def = &data.definitions[frame.d_nr as usize]`.

b. Resolve the call-site line:
   `self.line_numbers.get(&frame.call_pos).copied().unwrap_or(0)`.
   Returns 0 if the call site has no source mapping (synthetic code, default parameter
   evaluation).

c. Resolve the file: `def.position.file.clone()` (source path of the function
   definition).

d. Allocate a `vector<ArgInfo>` store.

e. For each `param` in `def.attributes` (declaration order):
   - Compute `offset`: running sum of `size_of(prev.typedef)` for all prior parameters.
   - **Bounds-check** (see [SC-ST-5](#sc-st-5--no-bounds-check-on-argument-reads-metadata-mismatch-causes-ub)):
     if `offset + size_of(param.typedef) > usize::from(frame.args_size)`, push
     `OtherVal { description: "read-out-of-bounds" }` and continue.
   - Read raw bytes at absolute stack offset `frame.args_base as usize + offset`.
   - **For `Text`**: null check is `str.ptr == STRING_NULL.as_ptr()` (not `is_empty()`
     or a null-pointer guard — see [SC-ST-1](#sc-st-1--text-null-sentinel-is-strnewstring_null-not-a-null-pointer));
     if non-null, call `str.str().to_owned()` to produce an independently allocated
     `String` (see [SC-ST-2](#sc-st-2--str-may-borrow-a-live-strings-buffer-shallow-copy-produces-a-dangling-pointer)).
   - Classify into an `ArgValue` variant using the null sentinels in the table below.
   - Allocate an `ArgInfo` heap record; append to the argument vector.

f. If `full` and `debug_locals` is non-empty for this function, materialise live local
   variables (Phase 6 — see Phase 6 section).

g. Allocate a `StackFrame` heap record; set `variables` to the accumulated
   `vector<VarInfo>` (empty when not `full`); append to the result vector.

**Step 4** — Return the `DbRef` of the outer `vector<StackFrame>`.

#### Null sentinels and classification table

| `Type` | Stack read | Null sentinel | `ArgValue` variant |
|---|---|---|---|
| `Boolean` | `u8` at offset | byte `0` | `BoolVal` or `NullVal` |
| `Integer(min, max, _)` | `i32`/`i16`/`i8` (width from declared range) | `i32::MIN` / `i16::MIN` / `i8::MIN` | `IntVal` or `NullVal` |
| `Long` | `i64` at offset | `i64::MIN` | `LongVal` or `NullVal` |
| `Float` | `f64` at offset | NaN (`f64::is_nan()`) | `FloatVal` or `NullVal` |
| `Single` | `f32` at offset | NaN (`f32::is_nan()`) | `SingleVal` or `NullVal` |
| `Character` | `u32` at offset | `0` (NUL scalar) | `CharVal` or `NullVal` |
| `Text` | `Str { ptr: *const u8, len: u32 }` (12 bytes) | `ptr == STRING_NULL.as_ptr()` | heap-copy into `TextVal`; null → `NullVal` |
| `Reference`, `Vector`, collection | `DbRef` (12 bytes: `u32` store + `u32` rec + `u32` pos) | `rec == 0` | `RefVal{store,rec,pos}` or `NullVal` |
| `Function(_)` | `i32` d_nr at offset | — | `FnVal{d_nr}` |
| anything else | — | — | `OtherVal{description: type.to_string()}` |

**Note on `Boolean`:** loft booleans use `0` as both `false` and `null` — they are
indistinguishable. `BoolVal { b: false }` and `NullVal` for a boolean parameter map
to the same sentinel byte.

### Line number resolution

`State.line_numbers: HashMap<u32, u32>` maps bytecode positions to 1-based source line
numbers. `CallFrame.call_pos` is the bytecode position **of the call instruction**
itself (not the function entry), so `line_numbers[call_pos]` gives the call-site line.
If no entry exists (synthetic or inlined code), `line` is reported as `0`.

### File name resolution

`data.definitions[d_nr].position.file` holds the source file path of the **function
definition** (the file where `fn foo(...) { ... }` appears). This is the most useful
path for a developer reading a stack trace. Call-site file tracking (the file containing
the call expression) is not provided — `line` already pinpoints the call within the
function's file.

---

## Safety Concerns and Mitigations

### SC-ST-1 — Text null sentinel is `Str::new(STRING_NULL)`, not a null pointer

**Problem:** The loft text type on the stack is `Str { ptr: *const u8, len: u32 }`
(12 bytes), not a Rust `String`. The null sentinel is the static string
`STRING_NULL = "\0"`, so null text is `Str { ptr: STRING_NULL.as_ptr(), len: 1 }`.
The pointer is never truly null; `len` is 1 (the NUL byte itself). Checking `is_empty()`
(`len == 0`) or a null-pointer guard misidentifies all null text arguments as valid
`TextVal` entries containing garbage content.

**Mitigation:** Null text detection in `materialise_stack_trace` must compare
`str.ptr == STRING_NULL.as_ptr()`. No other check is correct.

---

### SC-ST-2 — `Str` may borrow a live `String`'s buffer; shallow copy produces a dangling pointer

**Problem:** Static string literals are `Str` pointing into `text_code` (static
lifetime — safe). Dynamically built strings use a Rust `String` on the loft stack; a
text parameter receives a `Str` borrowing that `String`'s heap buffer. If materialisation
copies only `(ptr, len)` into the `TextVal` record, the pointer becomes dangling the
moment `OpFreeText` frees the source `String` (which happens when the source frame
returns or the temporary is dropped).

**Mitigation:** Every non-null text argument and local variable is materialised into an
independently heap-allocated `String` via `str.str().to_owned()`. The `Str`-vs-`String`
distinction is irrelevant at the point of copy: `str.str()` yields a `&str` slice
regardless of the backing source, and `.to_owned()` always allocates a fresh independent
buffer. This cost is acceptable since `stack_trace()` is called only on diagnostic paths.

---

### SC-ST-3 — Re-entrant `stack_trace()` corrupts `call_stack` iteration

**Problem:** `materialise_stack_trace` needs to iterate `self.call_stack`. Any loft
function call during materialisation (e.g. a store allocation, a format conversion
callback) would `push` a new `CallFrame` onto `call_stack` while the iteration is live,
invalidating the iterator and producing wrong frame counts or a use-after-reallocate on
the `Vec` buffer.

**Mitigation:** The first statement of `materialise_stack_trace` must clone `call_stack`
into a local snapshot:

```rust
fn materialise_stack_trace(&mut self, data: &Data, full: bool) -> DbRef {
    let frames = self.call_stack.clone();   // snapshot — mandatory first step
    for frame in &frames { ... }
}
```

The live `self.call_stack` may then be freely mutated during materialisation without
affecting the iteration.

---

### SC-ST-4 — `fn_call`'s `_size` parameter is not `args_size`; `args_base` would be wrong

**Problem:** The current signature is `fn_call(&mut self, _size: u16, to: i32)` where
`_size` is the *maximum stack space needed by the new function* — the local-variable
reservation, not the argument region. Using it as `args_size` to compute
`args_base = stack_pos - _size` would yield the wrong base address, causing all
argument reads to land at incorrect memory positions.

**Mitigation:** `fn_call` is extended with explicit, clearly named parameters:

```rust
pub fn fn_call(&mut self, d_nr: u32, args_size: u16, local_size: u16, to: i32)
//                                   ^^^^^^^^^^^      ^^^^^^^^^^
//                        sum of param sizes      max local var space (existing _size)
```

Every `fn_call` call site in `fill.rs` must be audited. `args_size` is computed at
codegen time as the sum of `size_of(param.typedef)` over all parameters in
`data.definitions[d_nr].attributes`, and is encoded as a second operand word alongside
the existing `d_nr` and `local_size` in the `OpCall` instruction.

`fn_call_ref` already carries `arg_size: u16`; it maps directly to `args_size` in the
updated delegation.

---

### SC-ST-5 — No bounds check on argument reads; metadata mismatch causes UB memory access

**Problem:** If `data.definitions[d_nr].attributes` is out of sync with the actual
bytecode (first-pass vs. second-pass type mismatch, or a default-parameter count
difference), the computed `offset + param_size` may exceed `args_size`, reading bytes
from the return-address slot or local variables above — undefined behaviour in Rust.

**Mitigation:** Before reading each parameter's bytes, check:

```rust
if offset + param_size > usize::from(frame.args_size) {
    // Produce a safe sentinel instead of an OOB read.
    push ArgValue::OtherVal { description: "read-out-of-bounds".to_string() };
    continue;
}
```

This turns a potential UB memory access into a visible diagnostic value. A debug-build
`debug_assert!` may additionally panic to surface the metadata mismatch early during
development.

---

### SC-ST-6 — `RefVal` coordinates silently dangle after the source frame's stores are freed

**Problem:** After a function returns and its store allocations are freed, a `RefVal` in
a retained trace may contain coordinates that now belong to a freshly reallocated record
of an unrelated type. Since `RefVal` stores only integers (store, rec, pos), no memory
safety violation occurs — but the numbers silently describe the wrong object, producing
misleading diagnostics.

**Mitigation:** Document prominently that `RefVal` coordinates are a **point-in-time
snapshot**. Any code that retains a `vector<StackFrame>` across function returns must
treat `RefVal` entries as historical identifiers, not live pointers:

> `RefVal` coordinates are valid only at the instant `stack_trace()` is called.
> If the traced function's stores are freed or reallocated before the trace is read,
> the coordinates describe a different or invalid record. Never dereference them
> via native code after the source frame has returned.

---

### SC-ST-7 — Trace result in a worker's stores cannot safely cross thread boundaries

**Problem:** Each parallel worker (`par(...)` body) has its own `State` and `Stores`.
A `vector<StackFrame>` produced by `stack_trace()` inside a worker lives in the
worker's stores. If the worker appends it to a shared collection, the DbRef crosses
thread boundaries. The main thread's `Stores` does not own those records; freeing the
shared collection would not free the worker's records (leak), or the worker's stores
would be freed on worker exit while the main thread still holds the DbRefs
(use-after-free in Rust store memory).

**Mitigation:** The compiler must reject any assignment of a `stack_trace()` result
to a cross-thread shared variable inside a `par(...)` body — the same rule that governs
other store-owned values that must not cross thread boundaries. Until that check is
implemented, document the restriction explicitly:

> Do not assign the result of `stack_trace()` to a variable shared between the parallel
> worker and the enclosing scope. Use it only within the same thread — log it, assert
> on it, or discard it before the worker body ends.

---

### Summary of safety concerns

| ID | Concern | Severity | Resolution in this design |
|---|---|---|---|
| SC-ST-1 | Text null is `ptr == STRING_NULL.as_ptr()`, not a null pointer or `is_empty()` | High | Correct null check in materialisation table and mitigation section |
| SC-ST-2 | `Str` may borrow a `String` buffer; shallow copy produces dangling pointer | High | Always `str.str().to_owned()` into a fresh heap buffer |
| SC-ST-3 | Re-entrant loft calls during materialisation mutate `call_stack` under iteration | High | Mandatory `call_stack.clone()` snapshot as first step |
| SC-ST-4 | `_size` in `fn_call` is local-var space, not args size; `args_base` would be wrong | Medium | `fn_call` extended with explicit `d_nr`, `args_size`, `local_size` parameters |
| SC-ST-5 | No bounds guard on argument reads; metadata mismatch causes UB memory access | Medium | Per-parameter `offset + size <= args_size` guard; OOB → `OtherVal` |
| SC-ST-6 | `RefVal` coordinates dangle silently after source frame returns | Low | Documented; warning in Known Limitations |
| SC-ST-7 | Trace result in worker stores cannot be shared with main thread | Low | Documented; compiler enforcement deferred |

---

## Implementation Phases

Phases 1–4 implement `stack_trace()` with parameter inspection and are required for the
feature to ship. Phases 5–6 are **optional, debug-only** and add local-variable
inspection via `stack_trace_full()`. They require compiler changes and should be gated
behind a build-time `debug_symbols` feature flag to keep release builds unaffected.

---

### Phase 1 — Shadow call-frame vector (`src/state/mod.rs`)

Introduce the infrastructure that tracks call frames at runtime.

1. Define `CallFrame { d_nr, call_pos, args_base, args_size, local_size }` in
   `src/state/mod.rs` (private — not exposed to loft programs).

2. Add `call_stack: Vec<CallFrame>` to `State`. Initialise to `Vec::new()` in the
   `State` constructor; pre-allocate a small capacity (e.g. 64) to avoid reallocation
   on typical call depths.

3. Extend `fn_call` signature:

   ```rust
   pub fn fn_call(&mut self, d_nr: u32, args_size: u16, local_size: u16, to: i32) {
       let args_base = self.stack_pos - u32::from(args_size);
       self.call_stack.push(CallFrame {
           d_nr, call_pos: self.code_pos, args_base, args_size, local_size,
       });
       self.put_stack(self.code_pos);    // write return address
       self.code_pos = to as u32;        // jump
   }
   ```

4. Extend `fn_return`: add `self.call_stack.pop();` after restoring `code_pos`.

5. Update all `fn_call` call sites in `fill.rs`:
   - `OpCall` must encode `d_nr`, `args_size` (new), and `local_size` (existing `_size`)
     as separate operand words, or `args_size` must be derivable from the definition at
     dispatch time; choose the encoding that keeps `fill.rs` readable.
   - `fn_call_ref` reads `d_nr` from the stack and already has `arg_size: u16` — update
     it to also read `local_size` from `data.definitions[d_nr].local_size` and delegate
     to the updated `fn_call`.

#### Tests — Phase 1

| Test | What it verifies |
|---|---|
| `call_stack_depth` | `call_stack.len()` equals the expected nesting depth inside a known call chain |
| `call_stack_pop` | depth returns to 0 after all functions return |
| `call_stack_d_nr` | top frame's `d_nr` matches the currently executing function's definition index |
| `call_stack_args_base` | `args_base` for a two-parameter function points to the first parameter byte |
| `call_stack_args_size` | `args_size` matches the sum of `size_of` for all declared parameter types |

---

### Phase 2 — Type declarations (`default/04_stacktrace.loft`)

Introduce the loft-visible types and function declarations.

1. Create `default/04_stacktrace.loft` with `pub` declarations for:
   `ArgValue`, `ArgInfo`, `VarInfo`, `StackFrame`, `stack_trace()`,
   `stack_trace_full()`.

2. In `src/main.rs`, add `default/04_stacktrace.loft` to the default load sequence
   after `03_text.loft`.

3. Bind `stack_trace()` to `OpStackTrace` and `stack_trace_full()` to
   `OpStackTraceFull` using whatever mechanism the runtime uses to associate a loft
   declaration with a dedicated opcode (e.g. a `#opcode "OpStackTrace"` annotation or
   a compile-time special-case in the compiler).

4. Both opcodes must be defined in the `Op` enum in `src/data.rs` and handled in
   `fill.rs` (or `src/state/mod.rs`) with direct `&mut self` access.

#### Tests — Phase 2

| Test | What it verifies |
|---|---|
| `types_declared` | `ArgValue`, `ArgInfo`, `VarInfo`, `StackFrame` are resolvable by name in a loft program |
| `stack_trace_callable` | `stack_trace()` compiles without error and the return type is `vector<StackFrame>` |
| `stack_trace_full_callable` | `stack_trace_full()` compiles without error and the return type is `vector<StackFrame>` |

---

### Phase 3 — Materialisation (`src/state/mod.rs` or `src/fill.rs`)

Implement the opcode handlers and `materialise_stack_trace`.

```rust
OpStackTrace => {
    let result = self.materialise_stack_trace(&data, false);
    self.put_stack(result);
}

OpStackTraceFull => {
    if cfg!(not(feature = "debug_symbols")) {
        self.runtime_error("stack_trace_full() requires debug symbols; \
                            rebuild with the debug_symbols flag");
    }
    let result = self.materialise_stack_trace(&data, true);
    self.put_stack(result);
}
```

`materialise_stack_trace` follows the algorithm in the [Runtime Design](#runtime-design)
section. Key implementation notes:

- `frame.args_base` is an **absolute** stack position. Reading parameter bytes:
  `self.stack[frame.args_base as usize + offset .. ]`.
- The stack is the contiguous `Vec<u8>` (or equivalent) in `State`; reads are raw byte
  copies cast to the appropriate Rust type.
- Heap records (`ArgInfo`, `StackFrame`, `VarInfo`) are allocated via `self.stores`
  into the current thread's default store.
- `def.name` gives the bare function name (already stripped of the `n_` internal
  prefix); use it directly for `StackFrame.function`.

#### Tests — Phase 3

| Test | What it verifies |
|---|---|
| `trace_function_name` | innermost frame has the correct function name |
| `trace_file_line` | `file` and `line` match the source of the `stack_trace()` call |
| `trace_arg_int` | integer argument appears as `IntVal{n}` with the correct value |
| `trace_arg_null_int` | null integer (`i32::MIN`) appears as `NullVal` |
| `trace_arg_text` | text argument appears as `TextVal{t}` with the correct content |
| `trace_arg_null_text` | null text (`STRING_NULL` sentinel) appears as `NullVal` |
| `trace_arg_ref` | `reference<T>` argument appears as `RefVal{store, rec, pos}` |
| `trace_arg_null_ref` | null reference (`rec == 0`) appears as `NullVal` |
| `trace_depth` | `len(stack_trace())` equals the actual call depth |
| `trace_ordering` | frame 0 is the entry point; last frame is the direct caller |
| `trace_no_args` | function with no parameters produces an empty `arguments` vector |
| `trace_multi_arg` | three-parameter function shows all three in declaration order |
| `trace_text_independent` | mutating the original text after `stack_trace()` does not change the captured `TextVal` |
| `trace_reentrant_safe` | calling a loft function during a format operation on the trace result does not corrupt the frame list |
| `trace_variables_empty` | `variables` field is an empty vector for every frame when using `stack_trace()` |

---

### Phase 4 — Call-site line numbers (`src/state/codegen.rs`)

Ensure that every call instruction in the bytecode has a corresponding entry in
`line_numbers` so that `CallFrame.call_pos` resolves to a non-zero line.

The affected opcodes are: `OpCall`, `OpCallRef`, `OpMethod`, `OpCallBuiltin` — any
opcode that transfers control to a new function frame. For each emission site in
`codegen.rs`, verify that the current source position is recorded in `line_numbers`
immediately before the call opcode is emitted:

```rust
// In gen_call (or equivalent):
self.line_numbers.insert(self.code_pos, parser_position.line);
self.emit(OpCall { d_nr, args_size, local_size, to });
```

If `parser_position.line` is 0 (synthetic code: default-parameter evaluation,
`#iterator` wrappers, compiler-generated dispatch), no entry is written and the
trace will show `line: 0` for that frame.

#### Tests — Phase 4

| Test | What it verifies |
|---|---|
| `line_number_at_call` | `frame.line` reported by `stack_trace()` matches the source line of the call expression |
| `line_number_nested` | each frame in a three-deep call chain reports its own call-site line, not the entry-point line |
| `line_number_synthetic` | a frame whose call was synthesised by the compiler reports `line == 0` |

---

### Phase 5 (optional) — Debug symbol table (`src/data.rs`, `src/state/codegen.rs`)

> **Prerequisite:** Phases 1–4 complete. This phase introduces the compiler-side
> metadata needed by Phase 6. It has no visible effect at the loft level on its own.
> All changes are conditioned on `#[cfg(feature = "debug_symbols")]`.

#### Blockers resolved by this phase

| Blocker | Description |
|---|---|
| B1 — metadata discarded | `Variable` in `variables/` is dropped at end of compilation; nothing survives into `Definition` |
| B2 — IR live ranges | `first_def` / `last_use` on `Variable` are IR sequence numbers, not bytecode positions |
| B3 — slot reuse | Compiler reuses stack slots for dead variables; a slot may be physically occupied but logically dead |
| B4 — work variables | Compiler-generated temporaries must be excluded from the debug table |

#### `LocalVarMeta` — per-variable debug record

Add to `src/data.rs`:

```rust
#[cfg(feature = "debug_symbols")]
pub struct LocalVarMeta {
    pub name:               String,  // user-visible variable name
    pub type_def:           u32,     // index into data.types for the declared type
    pub stack_pos:          u16,     // byte offset from args_base (not from frame start)
    pub live_from_code_pos: u32,     // inclusive bytecode pos where slot first holds a value
    pub live_to_code_pos:   u32,     // exclusive bytecode pos after which slot may be reused
}
```

`stack_pos` is measured from `args_base` (same reference as parameters), making bounds
checks uniform between Phase 3 and Phase 6. Local variables are laid out above the
parameter region: `stack_pos >= args_size`.

Add to `Definition`:

```rust
#[cfg(feature = "debug_symbols")]
pub debug_locals: Vec<LocalVarMeta>,
// In non-debug builds, the field is absent; no allocation, no binary size impact.
```

#### Codegen population

In `src/state/codegen.rs`, at the end of code generation for each function (debug builds
only):

1. **IR-to-bytecode position table**: during code generation, maintain a
   `ir_seq_to_code_pos: Vec<u32>` vector that records the bytecode position at which
   each IR node was emitted. The IR sequence number for a node is its index in the
   function's IR sequence. This table is local to the codegen pass and discarded
   afterward; only the translated positions are kept in `LocalVarMeta`.

2. **Populate `debug_locals`**: after generating all IR nodes for a function, iterate
   `variables::Function.variables` and, for each `Variable` that passes the filter rules,
   construct a `LocalVarMeta`:
   - `name`: from `variable.name`
   - `type_def`: from `variable.type_def`
   - `stack_pos`: from `variable.stack_pos`
   - `live_from_code_pos`: `ir_seq_to_code_pos[variable.first_def]`
   - `live_to_code_pos`: `ir_seq_to_code_pos[variable.last_use] + instruction_size`

3. **Filter rules** (exclude from `debug_locals`):
   - `variable.argument == true`: already exposed as `ArgInfo` parameters.
   - `variable.name.starts_with('_')`: compiler-generated temporaries.
   - `variable.uses == 0`: defined but never read (compiler already warns; no live range
     to record meaningfully).
   - `variable.first_def == variable.last_use`: single-use temporaries that the compiler
     could not eliminate (treat as synthetic).

#### Tests — Phase 5

| Test | What it verifies |
|---|---|
| `debug_locals_count` | `debug_locals` in a function with two user-visible locals contains exactly two entries |
| `debug_locals_names` | each entry's `name` matches the source identifier |
| `debug_locals_no_args` | parameter names do not appear in `debug_locals` |
| `debug_locals_no_temps` | compiler temporaries (names starting with `_`) are absent |
| `debug_locals_live_range` | `live_from_code_pos < live_to_code_pos`; positions are within the function's bytecode span |
| `debug_locals_release_empty` | in a non-debug build, `debug_locals` is absent (field does not exist on `Definition`) |

---

### Phase 6 (optional) — `stack_trace_full()` and local variables

> **Prerequisite:** Phase 5. Adds the loft-visible portion of local-variable inspection.

`StackFrame.variables` is already declared (Phase 2). `stack_trace_full()` is already
declared and bound to `OpStackTraceFull` (Phase 2). This phase implements the
materialisation of `variables`.

#### Materialising local variables

Inside `materialise_stack_trace` when `full == true`, after completing step (e) for a
frame, execute step (f):

f. Iterate `def.debug_locals`. For each `LocalVarMeta meta`:

   1. **Live-range filter**: include the variable only if
      `meta.live_from_code_pos <= frame.call_pos < meta.live_to_code_pos`.
      Variables outside their live range have logically undefined stack content
      (the slot may have been reused); skip them.

   2. **Slot bounds-check**: verify
      `usize::from(meta.stack_pos) + size_of(meta.type_def)
       <= usize::from(frame.args_size) + usize::from(frame.local_size)`.
      If the check fails, produce `OtherVal { description: "read-out-of-bounds" }`
      and continue.

   3. **Read and classify**: apply the same null-sentinel and deep-copy rules as for
      parameters (SC-ST-1 through SC-ST-5). The absolute stack offset is
      `frame.args_base as usize + usize::from(meta.stack_pos)`.
      Use `data.types[meta.type_def as usize]` to determine the `Type` variant for
      classification.

   4. **Append** a `VarInfo` record (name, type_name, value) to the frame's
      `variables` vector.

#### Safety note — slot reuse and false values

Even with live-range filtering, a slot that was reused may retain bytes from a prior
value if the new occupant has not yet been assigned. The live-range filter protects
against reading stale values of the *current* variable after its last use, but cannot
protect against bytes that another variable wrote into the same slot before
`live_from_code_pos`. In practice this is not a problem because the live range begins
at the first assignment (`generate_set`), so the slot is guaranteed initialised by the
compiler at `live_from_code_pos`.

#### Tests — Phase 6

| Test | What it verifies |
|---|---|
| `full_trace_variables_present` | `stack_trace_full()` returns frames with non-empty `variables` for a function that has two user-visible locals |
| `full_trace_variable_name` | `VarInfo.name` matches the declared identifier |
| `full_trace_variable_value` | `VarInfo.value` matches the runtime value of the local at the call site |
| `full_trace_dead_slot_absent` | a variable past its `live_to_code_pos` does not appear in `variables` |
| `full_trace_args_not_in_vars` | parameters appear only in `arguments`, not duplicated in `variables` |
| `full_trace_no_debug_error` | calling `stack_trace_full()` in a non-debug build produces the documented runtime error message |
| `full_trace_null_local` | a null local variable appears as `NullVal` |
| `full_trace_text_local` | a text local is deep-copied; mutating the original after the call does not change the `VarInfo` |

---

## Known Limitations

| ID | Limitation | Workaround |
|---|---|---|
| ST-1 | `RefVal` exposes raw DbRef coordinates; dereferencing into a struct dump requires native code | Format the value as `{v:j}` (JSON) before calling `stack_trace()` and capture it as a text argument |
| ST-2 | `RefVal` coordinates are a point-in-time snapshot — they may describe a reallocated or freed record if the trace is retained after the traced frame returns | Read `RefVal` entries immediately; do not cache a `vector<StackFrame>` across scope exits of the traced functions |
| ST-3 | Static native calls (via `library`) do not appear as frames | By design: native functions have no loft definition number or source location |
| ST-4 | `line` is `0` for compiler-synthesised call sites (default-parameter evaluation, `#iterator` wrappers) | Unavoidable without synthetic source positions; the function name is still available |
| ST-5 | `stack_trace()` inside a `par(...)` worker returns only the current worker's frames; the result must not be shared with the enclosing scope | Log or assert within the worker body; do not return the trace to the caller |
| ST-6 | `stack_trace_full()` requires a debug build; using it in a release build aborts at runtime | Build with `--features debug_symbols`; use `stack_trace()` for production diagnostic hooks |

---

## Non-Goals

- **Modifying the call stack** — `stack_trace()` is a read-only snapshot.
- **Resumable exceptions** — this design does not introduce exception unwinding.
- **Full heap dump** — `RefVal` gives the DbRef coordinates; dereferencing into a full
  struct dump is out of scope.
- **Source text retrieval** — line numbers are provided; returning the source text of
  that line is not.
- **Performance tracing / profiling** — use the logger and `par(...)` timing instead.
- **Local variable inspection in release builds** — `stack_trace()` exposes parameters
  only; local variables require `stack_trace_full()` with debug symbols (Phases 5–6),
  which is intentionally unavailable in release builds.

---

## See also

- [INTERMEDIATE.md](INTERMEDIATE.md) — `State` layout, `fn_call`/`fn_return`, stack
  frame layout, `line_numbers`, `fn_positions`, `text_positions` invariant; `Str` vs
  `String` text representations; `STRING_NULL` sentinel
- [INTERNALS.md](INTERNALS.md) — native function registry, `library` call convention,
  `OpCallRef` as a precedent for opcodes with direct `&State` access
- [LOGGER.md](LOGGER.md) — runtime logging (complement to stack traces for diagnostics)
- [LOFT.md](LOFT.md) — enum and struct syntax used by `ArgValue` / `StackFrame`
- [THREADING.md](THREADING.md) — parallel execution model (ST-5 limitation context)
- [SLOTS.md](SLOTS.md) — stack slot assignment (B3 blocker context for Phase 5)
