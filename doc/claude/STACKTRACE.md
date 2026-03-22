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
| `TextVal` | `text` — value deep-copied from the stack |
| `RefVal` | `reference<T>`, struct-enum, `vector<T>`, collection — raw DbRef triple |
| `FnVal` | `fn(T) -> R` — definition number |
| `OtherVal` | Iterator state or any type without a direct scalar representation; `description` holds the type name |

`RefVal` exposes the raw `(store, rec, pos)` triple from the DbRef. The caller can use
this to identify which store record is being referenced, but further dereferencing
requires native code. The `description` field of `OtherVal` contains the loft type name
as a text string (e.g. `"iterator<Item, integer>"`).

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
    args_base: u32,   // stack_pos value at the start of this frame's arguments
    args_size: u16,   // total byte size of all parameters
}
```

`State` gains a field:

```rust
call_stack: Vec<CallFrame>,
```

`fn_call` is extended:

```rust
pub fn fn_call(&mut self, d_nr: u32, args_size: u16, to: i32) {
    let args_base = self.stack_pos - u32::from(args_size);
    self.call_stack.push(CallFrame { d_nr, call_pos: self.code_pos, args_base, args_size });
    self.put_stack(self.code_pos);
    self.code_pos = to as u32;
}
```

`fn_return` pops:

```rust
// At the end of fn_return:
self.call_stack.pop();
```

`fn_call_ref` already delegates to `fn_call`; it gains the `d_nr` it already reads.
Static calls (via `library`) are not loft functions and are not pushed.

### `OpStackTrace` — dedicated opcode

The native function table (`library`) only gives native functions access to
`(&mut Stores, &mut DbRef)`. That is insufficient for stack trace materialisation,
which requires `&State`, `&Data` (definition table), and `&call_stack`.

`stack_trace()` is therefore implemented as a **dedicated opcode** `OpStackTrace`
(similar to `OpCallRef`, which also has direct `&mut self` access inside
`fill.rs`/`state/mod.rs`):

```rust
// In fill.rs or state/mod.rs:
OpStackTrace => {
    let frames = self.materialise_stack_trace(&data);
    // push the vector<StackFrame> DbRef onto the stack
}
```

`materialise_stack_trace` walks `self.call_stack` outermost-first, reads argument
values from the raw stack bytes, and constructs the `vector<StackFrame>` in a store.

### Materialising argument values

For each `CallFrame`, the function's parameters are read from `data.definitions[d_nr].attributes`
(which holds name, type, and offset). The value at `stack_cur.pos + args_base + offset`
is interpreted according to the parameter's `Type`:

| `Type` | Stack read | `ArgValue` variant |
|---|---|---|
| `Boolean` | `u8` at offset | `BoolVal` or `NullVal` |
| `Integer(min, max, _)` | `i32` / `i16` / `i8` at offset (width from range) | `IntVal` or `NullVal` |
| `Long` | `i64` at offset | `LongVal` or `NullVal` |
| `Float` | `f64` at offset | `FloatVal` or `NullVal` (NaN) |
| `Single` | `f32` at offset | `SingleVal` or `NullVal` (NaN) |
| `Character` | `u32` at offset | `CharVal` or `NullVal` ('\0') |
| `Text` | `String` at offset | deep-copy into `TextVal`; null pointer → `NullVal` |
| `Reference`, `Vector`, collection | `DbRef` (12 bytes) at offset | `RefVal{store,rec,pos}` or `NullVal` |
| `Function(_, _)` | `i32` d_nr at offset | `FnVal{d_nr}` |
| anything else | — | `OtherVal{description: type.to_string()}` |

Null detection uses the same sentinels as the rest of the runtime:
- `integer`: `i32::MIN`
- `float`/`single`: NaN
- `character`: `'\0'` (0)
- `reference`/collection: `rec == 0`
- `text`: null pointer (checked via `String::is_empty` after a null-pointer guard)

### Line number resolution

`State.line_numbers: HashMap<u32, u32>` maps bytecode positions to 1-based source line
numbers. `CallFrame.call_pos` is the code position **of the call instruction**, so
`line_numbers[call_pos]` gives the call-site line. If no entry exists (e.g. for
synthetic or inlined code), `line` is reported as `0`.

### File name resolution

`Definition.position.file` (or the equivalent field on the source position) holds the
file path for each definition. `data.definitions[d_nr].position.file` gives the source
file of the function, which is the correct file for the call-site line since both the
call and the function live in the same compilation unit.

---

## Implementation Phases

### Phase 1 — Shadow call-frame vector (`src/state/mod.rs`)

1. Add `call_stack: Vec<CallFrame>` to `State`; define the `CallFrame` struct.
2. Extend `fn_call` to accept `d_nr` and `args_size`; push a `CallFrame`.
3. Extend `fn_return` to pop the top frame.
4. Update all call sites in `fill.rs` that invoke `fn_call` to supply `d_nr` and
   `args_size` (both are already known at those sites).

#### Tests — Phase 1

| Test | What it verifies |
|---|---|
| `call_stack_depth` | `call_stack.len()` equals the expected nesting depth inside a known call chain |
| `call_stack_pop` | depth returns to 0 after all functions return |
| `call_stack_d_nr` | top frame's `d_nr` matches the currently executing function |

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

1. Allocate a `vector<StackFrame>` store.
2. For each `CallFrame` in `self.call_stack` (outermost first):
   a. Look up `data.definitions[d_nr]` for the function name and source position.
   b. Resolve the line number from `self.line_numbers[call_pos]`.
   c. Allocate a `vector<ArgInfo>` store.
   d. For each parameter (from `def.attributes` in declaration order):
      - Compute its offset from `args_base`.
      - Read and classify the raw bytes into an `ArgValue` variant.
      - Deep-copy `text` arguments (the stack owns the buffer; the trace must not alias it).
      - Allocate an `ArgInfo` record and append to the argument vector.
   e. Allocate a `StackFrame` record and append to the result vector.
3. Push the result `DbRef` onto the stack.

#### Tests — Phase 3

| Test | What it verifies |
|---|---|
| `trace_function_name` | innermost frame has the correct function name |
| `trace_file_line` | `file` and `line` match the source of the `stack_trace()` call |
| `trace_arg_int` | integer argument appears as `IntVal{n}` with the correct value |
| `trace_arg_text` | text argument appears as `TextVal{t}`; modifying the trace copy does not affect the original |
| `trace_arg_null` | null integer appears as `NullVal` |
| `trace_arg_ref` | `reference<T>` argument appears as `RefVal{store, rec, pos}` |
| `trace_depth` | `len(stack_trace())` equals the actual call depth |
| `trace_ordering` | frame 0 is `main`; last frame is the direct caller |
| `trace_no_args` | function with no parameters produces an empty `arguments` vector |
| `trace_multi_arg` | three-parameter function shows all three in declaration order |

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
| ST-1 | `RefVal` exposes raw DbRef coordinates; further dereferencing requires native code | Use `description` in `OtherVal` for display; use `{:j}` JSON format on the variable before calling `stack_trace()` |
| ST-2 | `text` arguments are deep-copied; large text values incur allocation cost at trace time | Call `stack_trace()` only in error paths, not on the hot path |
| ST-3 | Static native calls (via `library`) do not appear as frames | By design: native functions have no loft source location |
| ST-4 | `line` is `0` for compiler-synthesised call sites (default parameter evaluation, `#iterator` wrappers) | Unavoidable without synthetic source positions |
| ST-5 | Parallel worker threads each have their own `call_stack`; `stack_trace()` returns only the current thread's frames | Accepted: cross-thread trace would require synchronisation and is rarely needed |

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
  frame layout, `line_numbers`, `fn_positions`, `text_positions` invariant
- [INTERNALS.md](INTERNALS.md) — native function registry, `library` call convention,
  `OpCallRef` as a precedent for opcodes with direct `&State` access
- [LOGGER.md](LOGGER.md) — runtime logging (complement to stack traces for diagnostics)
- [LOFT.md](LOFT.md) — enum and struct syntax used by `ArgValue` / `StackFrame`
- [THREADING.md](THREADING.md) — parallel execution model (ST-5 limitation context)
