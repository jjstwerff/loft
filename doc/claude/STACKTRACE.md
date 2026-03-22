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

---

## Exposed Types

All types are declared `pub` in a new default library file `default/04_stacktrace.loft`.

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
| `RefVal` | `reference<T>`, struct-enum, `vector<T>`, collection — raw DbRef triple; valid at snapshot time only (see [SC-ST-6](#sc-st-6--refval-coordinates-silently-dangle)) |
| `FnVal` | `fn(T) -> R` — definition number |
| `OtherVal` | Iterator state or any type without a direct scalar representation; `description` holds the type name |

`RefVal` exposes the raw `(store, rec, pos)` triple from the DbRef. The coordinates are
valid only at the instant `stack_trace()` is called; they may point to a reallocated or
freed record if the trace is retained after the source frame has returned (see
[Known Limitations](#known-limitations)). The `description` field of `OtherVal` contains
the loft type name as a text string (e.g. `"iterator<Item, integer>"`).

### `ArgInfo` — one function argument

```loft
pub struct ArgInfo {
    pub name:      text,     // parameter name as declared in the function signature
    pub type_name: text,     // loft type as text: "integer", "text", "vector<Foo>", …
    pub value:     ArgValue, // inspectable value at the time of stack_trace()
}
```

### `StackFrame` — one call frame

```loft
pub struct StackFrame {
    pub function:  text,            // bare function name, e.g. "compute_score"
    pub file:      text,            // source file path
    pub line:      integer,         // 1-based source line of the call site
    pub arguments: vector<ArgInfo>, // one entry per declared parameter, in declaration order
}
```

`line` is the line of the **call site** (the instruction that invoked this function),
not the line of the function declaration. For the innermost frame (the frame that called
`stack_trace()`) it is the line of the `stack_trace()` call itself.

---

## API

```loft
pub fn stack_trace() -> vector<StackFrame>;
```

Returns the call stack as a vector of frames ordered **outermost first** (index 0 is
`main`; the last element is the frame that called `stack_trace()`). The vector is fully
materialised at the moment of the call; mutations to the stack after the call do not
affect it.

---

## Usage Examples

### Print a stack trace on error

```loft
fn assert_positive(n: integer) {
    if n <= 0 {
        for frame in stack_trace() {
            say("{frame.file}:{frame.line}  {frame.function}");
            for arg in frame.arguments {
                say("  {arg.name}: {arg.type_name} = {inspect_arg(arg.value)}");
            }
        }
        assert(false, "n must be positive");
    }
}

fn inspect_arg(v: ArgValue) -> text {
    match v {
        NullVal           => "null",
        BoolVal   { b }   => "{b}",
        IntVal    { n }   => "{n}",
        LongVal   { n }   => "{n}",
        FloatVal  { f }   => "{f}",
        SingleVal { f }   => "{f}",
        CharVal   { c }   => "'{c}'",
        TextVal   { t }   => "\"{t}\"",
        RefVal    { store, rec, pos } => "ref({store},{rec},{pos})",
        FnVal     { d_nr } => "fn#{d_nr}",
        OtherVal  { description } => "<{description}>",
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
        say("called from {caller.function} with {len(caller.arguments)} args");
        for arg in caller.arguments {
            say("  {arg.name}: {arg.type_name}");
        }
    }
}
```

---

## Runtime Design

### Why a dedicated `call_stack` vector is needed

The existing bytecode stack stores return addresses inline (a `u32` code position written
by `fn_call` at `stack_pos + args_size`). Walking backwards from the current frame
requires knowing each frame's `args_size`, which is only available from the bytecode
stream — not from the stack itself. Reconstructing it at trace time would require
parsing each function's entry sequence, which is expensive and fragile.

The clean solution is to maintain a shadow call-frame vector on `State` that `fn_call`
pushes to and `fn_return` pops from. The overhead is one `Vec::push` + one `Vec::pop`
per call, which is negligible compared with the function dispatch itself.

### `CallFrame` — internal shadow frame (Rust)

```rust
struct CallFrame {
    d_nr:      u32,   // definition number of the called function
    call_pos:  u32,   // bytecode position of the call instruction (line-number lookup)
    args_base: u32,   // stack_pos at the start of this frame's arguments
    args_size: u16,   // total byte size of all parameters (sum of size_of each param type)
}
```

`State` gains a field:

```rust
call_stack: Vec<CallFrame>,
```

`fn_call` is extended with two new explicit parameters (see [SC-ST-4](#sc-st-4--fn_calls-_size-is-not-args_size)):

```rust
pub fn fn_call(&mut self, d_nr: u32, args_size: u16, local_size: u16, to: i32) {
    // args have already been pushed; args_base is current stack top minus args
    let args_base = self.stack_pos - u32::from(args_size);
    self.call_stack.push(CallFrame { d_nr, call_pos: self.code_pos, args_base, args_size });
    self.put_stack(self.code_pos);
    self.code_pos = to as u32;
}
```

`fn_return` pops:

```rust
// At the end of fn_return, after restoring code_pos:
self.call_stack.pop();
```

`fn_call_ref` already delegates to `fn_call`; it supplies `d_nr` from the value it
reads and `args_size` from its existing `arg_size` parameter.
Static calls (via `library`) are not loft functions and are not pushed.

### `OpStackTrace` — dedicated opcode

The native function table (`library`) only gives native functions access to
`(&mut Stores, &mut DbRef)`. That is insufficient for stack trace materialisation,
which requires `&State`, `&Data` (definition table), and `&call_stack`.

`stack_trace()` is therefore implemented as a **dedicated opcode** `OpStackTrace`
(following the same pattern as `OpCallRef`, which also has direct `&mut self` access):

```rust
// In fill.rs or state/mod.rs:
OpStackTrace => {
    let result = self.materialise_stack_trace(&data);
    self.put_stack(result);
}
```

### Materialising argument values

For each `CallFrame`, the function's parameters are read from
`data.definitions[d_nr].attributes` (name, type, stack offset). Before any iteration,
`call_stack` is **cloned into a local snapshot** (see [SC-ST-3](#sc-st-3--re-entrant-stack_trace-corrupts-call_stack-iteration)).

For each parameter, the byte offset from `args_base` is computed and **bounds-checked**
against `args_size` before any read (see [SC-ST-5](#sc-st-5--no-bounds-check-on-argument-reads)).
If the offset overflows the argument region, `OtherVal { description: "read-out-of-bounds" }`
is produced for that parameter.

| `Type` | Stack read | Null sentinel | `ArgValue` variant |
|---|---|---|---|
| `Boolean` | `u8` at offset | byte `0` or `255` (false) | `BoolVal` or `NullVal` |
| `Integer(min, max, _)` | `i32`/`i16`/`i8` (width from range) | `i32::MIN` / `i16::MIN` / `i8::MIN` | `IntVal` or `NullVal` |
| `Long` | `i64` at offset | `i64::MIN` | `LongVal` or `NullVal` |
| `Float` | `f64` at offset | NaN | `FloatVal` or `NullVal` |
| `Single` | `f32` at offset | NaN | `SingleVal` or `NullVal` |
| `Character` | `u32` at offset | `0` (NUL) | `CharVal` or `NullVal` |
| `Text` | `Str { ptr, len }` at offset | `ptr == STRING_NULL.as_ptr()` | heap-copy into `TextVal`; null → `NullVal` |
| `Reference`, `Vector`, collection | `DbRef` (12 bytes) at offset | `rec == 0` | `RefVal{store,rec,pos}` or `NullVal` |
| `Function(_, _)` | `i32` d_nr at offset | — | `FnVal{d_nr}` |
| anything else | — | — | `OtherVal{description: type.to_string()}` |

**Text null detection** (see [SC-ST-1](#sc-st-1--text-null-sentinel-is-strnewstring_null-not-a-null-pointer)):
loft text on the stack is `Str { ptr: *const u8, len: u32 }` (12 bytes), not a Rust
`String`. The null sentinel is `STRING_NULL: &str = "\0"` — a static byte — so null
text is detected by `str.ptr == STRING_NULL.as_ptr()`, not by checking for a null
pointer or an empty string.

**Text deep-copy** (see [SC-ST-2](#sc-st-2--str-may-borrow-a-live-strings-buffer)):
a `Str` may point into the static `text_code` pool (safe to copy) or into a dynamic
`String`'s heap buffer (pointer becomes dangling after `OpFreeText`). To be safe in
all cases, every non-null text argument is materialised into a freshly heap-allocated
`String` via `str.str().to_owned()`. The owned `String` is then stored in the
`TextVal{t}` field of the heap record. This cost is acceptable since `stack_trace()` is
expected to be called only in diagnostic paths.

### Line number resolution

`State.line_numbers: HashMap<u32, u32>` maps bytecode positions to 1-based source line
numbers. `CallFrame.call_pos` is the code position **of the call instruction**, so
`line_numbers.get(&call_pos).copied().unwrap_or(0)` gives the call-site line. If no
entry exists (e.g. for synthetic or inlined code), `line` is reported as `0`.

### File name resolution

`data.definitions[d_nr].position.file` holds the source file path of the function
definition. This is used as-is for `StackFrame.file`.

---

## Safety Concerns and Mitigations

### SC-ST-1 — Text null sentinel is `Str::new(STRING_NULL)`, not a null pointer

**Problem:** The loft text type on the stack is `Str { ptr: *const u8, len: u32 }`
(12 bytes), not a Rust `String`. The null sentinel is the static string
`STRING_NULL = "\0"`, so null text is `Str { ptr: STRING_NULL.as_ptr(), len: 1 }`.
The pointer is never truly null; `len` is 1 (the NUL byte). Checking `is_empty()`
(len == 0) or a null-pointer guard misidentifies all null text arguments as valid
`TextVal` entries containing garbage content.

**Mitigation:** Null text detection in `materialise_stack_trace` must compare
`str.ptr == STRING_NULL.as_ptr()`. No other check is correct.

---

### SC-ST-2 — `Str` may borrow a live `String`'s buffer; shallow copy produces a dangling pointer

**Problem:** Static string literals are `Str` pointing into `text_code` (static
lifetime — safe). Dynamically built strings use a Rust `String` on the stack; a text
parameter receives a `Str` borrowing that `String`'s heap buffer. If materialisation
copies only `(ptr, len)`, the `TextVal` record's pointer becomes dangling the moment
`OpFreeText` frees the source `String`.

**Mitigation:** Every non-null text argument is materialised into an independently
heap-allocated `String` via `str.str().to_owned()`. The `Str`-vs-`String` distinction
is irrelevant at the point of copy: `str.str()` yields a `&str` slice regardless of the
backing source, and `.to_owned()` always allocates a fresh independent buffer.

---

### SC-ST-3 — Re-entrant `stack_trace()` corrupts `call_stack` iteration

**Problem:** `materialise_stack_trace` iterates over `self.call_stack`. Any loft
function call during materialisation (e.g. a format conversion, a store allocation
callback) would `push` to `call_stack` while the iteration is live, invalidating the
iterator and producing wrong frame counts or use-after-reallocate on the `Vec`.

**Mitigation:** The first statement of `materialise_stack_trace` must clone `call_stack`
into a local snapshot:

```rust
fn materialise_stack_trace(&mut self, data: &Data) -> DbRef {
    let frames = self.call_stack.clone();   // snapshot — mandatory first step
    for frame in &frames { ... }
}
```

The live `self.call_stack` may then be freely mutated during materialisation without
affecting the iteration.

---

### SC-ST-4 — `fn_call`'s `_size` parameter is not `args_size`; `args_base` would be wrong

**Problem:** The current signature is `fn_call(&mut self, _size: u16, to: i32)` where
`_size` is documented as *"the amount of stack space maximally needed for the new
function"* — the local-variable reservation size. Using it as `args_size` to compute
`args_base = stack_pos - _size` would yield the wrong base address, causing all
argument reads to land at incorrect positions.

**Mitigation:** `fn_call` is extended with two explicit, clearly named parameters:

```rust
pub fn fn_call(&mut self, d_nr: u32, args_size: u16, local_size: u16, to: i32)
//                                   ^^^^^^^^^^^      ^^^^^^^^^^
//                        sum of param sizes      max local var space (existing _size)
```

Every call site in `fill.rs` that currently passes `_size` must be audited:
- `args_size` = sum of `size_of(param.typedef)` for each parameter in
  `data.definitions[d_nr].attributes`; this is always known at the `OpCall` emission
  site in codegen.
- `local_size` = the existing `_size` value, passed through unchanged.

`fn_call_ref` already has `arg_size: u16`; it maps directly to `args_size`.

---

### SC-ST-5 — No bounds check on argument reads; metadata mismatch causes UB memory access

**Problem:** If `data.definitions[d_nr].attributes` is out of sync with the actual
bytecode (first-pass vs. second-pass type mismatch, or a `default`-parameter count
difference), the computed `offset + param_size` may exceed `args_size`, reading into the
return-address slot or local variables above it — undefined behaviour in Rust.

**Mitigation:** Before reading each parameter's bytes, assert:

```rust
if offset + param_size > usize::from(frame.args_size) {
    // produce a safe sentinel instead of an OOB read
    push ArgValue::OtherVal { description: "read-out-of-bounds".to_string() };
    continue;
}
```

This turns a potential UB memory access into a visible diagnostic value. A debug-build
`debug_assert!` may additionally panic to surface the metadata mismatch early.

---

### SC-ST-6 — `RefVal` coordinates silently dangle after the source frame's stores are freed

**Problem:** After a function returns and its store allocations are freed, a `RefVal` in
a retained trace may show coordinates that now belong to a freshly reallocated record of
an unrelated type. Since `RefVal` stores only integers (store, rec, pos), no memory
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
shared collection would not free the worker's stores (leak), or the worker's stores
would be freed on worker exit while the main thread still holds the DbRefs
(use-after-free in Rust store memory).

**Mitigation:** The compiler must reject any assignment of a `stack_trace()` result
to a cross-thread shared variable inside a `par(...)` body — the same rule that governs
other store-owned values that must not cross thread boundaries. Until that check is
implemented, document the restriction explicitly:

> Do not assign the result of `stack_trace()` to a variable that is shared between
> the parallel worker and the enclosing scope. Use it only within the same thread
> (log it, assert on it, or discard it before the worker body ends).

---

### Summary of safety concerns

| ID | Concern | Severity | Resolution in this design |
|---|---|---|---|
| SC-ST-1 | Text null is `ptr == STRING_NULL.as_ptr()`, not a null pointer or empty string | High | Null check corrected in materialisation table and SC-ST-1 section |
| SC-ST-2 | `Str` may borrow a `String` buffer; shallow copy produces dangling pointer | High | Always `to_owned()` into a fresh heap buffer |
| SC-ST-3 | Re-entrant loft calls during materialisation mutate `call_stack` under iteration | High | Mandatory `call_stack.clone()` snapshot as first step |
| SC-ST-4 | `_size` in `fn_call` is local-var space, not args size; `args_base` would be wrong | Medium | `fn_call` extended with explicit `d_nr`, `args_size`, `local_size` parameters |
| SC-ST-5 | No bounds guard on argument reads; metadata mismatch causes UB | Medium | Per-parameter `offset + size <= args_size` guard; OOB → `OtherVal` |
| SC-ST-6 | `RefVal` coordinates dangle silently after source frame returns | Low | Documented; warning in Known Limitations |
| SC-ST-7 | Trace result in worker stores cannot be shared with main thread | Low | Documented; compiler enforcement deferred |

---

## Implementation Phases

### Phase 1 — Shadow call-frame vector (`src/state/mod.rs`)

1. Define `CallFrame { d_nr, call_pos, args_base, args_size }`.
2. Add `call_stack: Vec<CallFrame>` to `State`.
3. Extend `fn_call` to `fn_call(d_nr: u32, args_size: u16, local_size: u16, to: i32)`:
   - compute `args_base = self.stack_pos - u32::from(args_size)`;
   - push `CallFrame`; then write return address and jump as before.
4. Extend `fn_return` to pop the top frame after restoring `code_pos`.
5. Update all `fn_call` call sites in `fill.rs`: supply `d_nr` (from the emitted
   `OpCall` operand) and `args_size` (computed from `data.definitions[d_nr].attributes`
   at codegen time and encoded as a second `OpCall` operand); rename existing `_size` to
   `local_size`.

#### Tests — Phase 1

| Test | What it verifies |
|---|---|
| `call_stack_depth` | `call_stack.len()` equals the expected nesting depth inside a known call chain |
| `call_stack_pop` | depth returns to 0 after all functions return |
| `call_stack_d_nr` | top frame's `d_nr` matches the currently executing function |
| `call_stack_args_base` | `args_base` for a two-parameter function points to the first parameter byte |
| `call_stack_args_size` | `args_size` matches the sum of `size_of` for all parameter types |

---

### Phase 2 — `ArgValue` enum and `StackFrame` struct (`default/04_stacktrace.loft`)

1. Declare `ArgValue`, `ArgInfo`, and `StackFrame` as `pub` in a new file
   `default/04_stacktrace.loft`, loaded after `03_text.loft`.
2. Add the file to the default load order in `src/main.rs`.
3. Declare `pub fn stack_trace() -> vector<StackFrame>` with a `#opcode "OpStackTrace"`
   annotation (or equivalent mechanism to bind a loft declaration to a dedicated opcode).

#### Tests — Phase 2

| Test | What it verifies |
|---|---|
| `types_declared` | `ArgValue`, `ArgInfo`, `StackFrame` are resolvable by name |
| `stack_trace_callable` | `stack_trace()` compiles without error; returns a `vector<StackFrame>` |

---

### Phase 3 — `OpStackTrace` implementation (`src/state/mod.rs` or `src/fill.rs`)

Implement `materialise_stack_trace(&data) -> DbRef`:

1. **Snapshot** `self.call_stack` into a local `Vec<CallFrame>` (SC-ST-3).
2. Allocate a `vector<StackFrame>` store.
3. For each `CallFrame` in the snapshot (outermost first):
   a. Look up `data.definitions[d_nr]` for function name and source position.
   b. Resolve line number: `line_numbers.get(&call_pos).copied().unwrap_or(0)`.
   c. Allocate a `vector<ArgInfo>` store.
   d. For each parameter in `def.attributes` (declaration order):
      - Compute `offset` = sum of `size_of(prev_param.typedef)` for all prior params.
      - **Bounds-check**: if `offset + size_of(param.typedef) > args_size`, push
        `OtherVal { description: "read-out-of-bounds" }` and continue (SC-ST-5).
      - Read raw bytes from `stack_cur.pos + frame.args_base + offset`.
      - **For `Text`**: check `str.ptr == STRING_NULL.as_ptr()` for null (SC-ST-1);
        otherwise `str.str().to_owned()` into a fresh heap `String` (SC-ST-2).
      - Classify into the appropriate `ArgValue` variant using the null sentinels in
        the materialisation table.
      - Allocate an `ArgInfo` record and append to the argument vector.
   e. Allocate a `StackFrame` record (function, file, line, arguments) and append.
4. Push the result `DbRef` onto the stack.

#### Tests — Phase 3

| Test | What it verifies |
|---|---|
| `trace_function_name` | innermost frame has the correct function name |
| `trace_file_line` | `file` and `line` match the source of the `stack_trace()` call |
| `trace_arg_int` | integer argument appears as `IntVal{n}` with the correct value |
| `trace_arg_null_int` | null integer (`i32::MIN`) appears as `NullVal` |
| `trace_arg_text` | text argument appears as `TextVal{t}`; modifying the copy does not affect the original |
| `trace_arg_null_text` | null text (`STRING_NULL` sentinel) appears as `NullVal` |
| `trace_arg_ref` | `reference<T>` argument appears as `RefVal{store, rec, pos}` |
| `trace_arg_null_ref` | null reference (`rec == 0`) appears as `NullVal` |
| `trace_depth` | `len(stack_trace())` equals the actual call depth |
| `trace_ordering` | frame 0 is `main`; last frame is the direct caller |
| `trace_no_args` | function with no parameters produces an empty `arguments` vector |
| `trace_multi_arg` | three-parameter function shows all three in declaration order |
| `trace_text_independent` | mutating the original text after `stack_trace()` does not change the captured `TextVal` |
| `trace_reentrant_safe` | calling any loft function during a format operation on the trace result does not corrupt the frame list |

---

### Phase 4 — Line number emission for call sites (`src/state/codegen.rs`)

Verify that `line_numbers` entries are emitted at **call instructions** (not only at
function entry points). If any call opcode is emitted without a corresponding
`line_numbers` entry, add the mapping at code-generation time using the parser's current
source position.

#### Tests — Phase 4

| Test | What it verifies |
|---|---|
| `line_number_at_call` | line reported by `stack_trace()` matches the source line of the call expression |
| `line_number_nested` | each frame in a three-deep call chain reports its own call-site line |

---

## Known Limitations

| ID | Limitation | Workaround |
|---|---|---|
| ST-1 | `RefVal` exposes raw DbRef coordinates; further dereferencing requires native code | Use `description` in `OtherVal` for display; format the variable as `{v:j}` before calling `stack_trace()` |
| ST-2 | `RefVal` coordinates are a point-in-time snapshot — they may describe a reallocated or freed record if the trace is retained after the source frame returns | Read `RefVal` entries immediately; do not cache a `vector<StackFrame>` across scope exits of the traced functions |
| ST-3 | Static native calls (via `library`) do not appear as frames | By design: native functions have no loft source location |
| ST-4 | `line` is `0` for compiler-synthesised call sites (default parameter evaluation, `#iterator` wrappers) | Unavoidable without synthetic source positions |
| ST-5 | `stack_trace()` inside a `par(...)` worker returns only the current worker's frames; the result must not be assigned to a variable shared with the enclosing scope — doing so causes a store-ownership violation (leak or use-after-free on worker exit) | Log or assert on the trace within the worker body; do not return it to the caller |

---

## Non-Goals

- **Modifying the call stack** — `stack_trace()` is a read-only snapshot.
- **Resumable exceptions** — this design does not introduce exception unwinding.
- **Full heap dump** — `RefVal` gives the DbRef coordinates; dereferencing into a full
  struct dump is out of scope.
- **Source text retrieval** — line numbers are provided; returning the source text of
  that line is not.
- **Performance tracing / profiling** — use the logger and `par(...)` timing instead.

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
