# Design — a Rust → loft host-call API (`loft::host`)

> Status: **P1 + P2 SHIPPED** (`src/host.rs`, `State::execute_host`, `tests/host_call.rs`,
> `loft fmt`). P3 (struct/vector/enum returns) deferred until a consumer needs it — the
> language server is the expected driver. Driver: `loft fmt` needs to call the
> loft-written formatter's `format(text) -> text` from Rust. But the gap is general —
> embedding loft, tool integration, and testing all want a first-class "call a loft
> function by name with typed args, get a typed return, errors as `Result`" surface.

## The gap

Rust can run a whole program (`State::execute_argv("main", …)`, argv as strings, void
return) or invoke the **par-worker** primitives (`execute_at_text` / `execute_at_raw_*` /
`execute_at_ref` / `execute_at_void`). The latter ARE the stack ABI, but they are:

- **specialised per return type** (text vs primitive vs ref vs void), and
- **par-worker-shaped**: they assume `parallel_ctx` / `fn_positions` are already primed,
  take the first argument as a `WorkerArg` and the rest as `&[u64]` extras, and compute
  `n_hidden_text` / `n_hidden_dests` at the call site (see `native.rs:1705`).

There is no ergonomic, public, typed entry a host can call without knowing any of that.
`loft fmt` exposed the gap: I was about to hand-roll a fragile one-off (temp files or a
raw `execute_at_text` call) instead of the real thing.

## The invariant (one hypothesis)

> **A host call routes through the SAME stack ABI the interpreter uses internally — it
> adds a typed façade, argument/return marshalling *by declared type*, and entry priming,
> but it does NOT re-implement how a value is pushed or read.** A host `call(f, args)` is
> therefore byte-for-byte equivalent to an in-language call of `f` with the same values.

The one marshalling implementation stays the `execute_at_*` family (+ `put_stack` /
`get_stack::<T>()`). The host API is a *dispatcher* over them, keyed by the function's
signature. If that holds, correctness is inherited, not re-derived.

## Re-assertion count (the brittleness, named now)

The failure this design must avoid is a **second marshaller**: a host-side copy of "how a
text is pushed / how an int return is read" that drifts from the interpreter's. Count the
sites that must know the ABI:

- **Today:** `execute_argv` (argv push, hidden-dest sentinels, entry frame) + the four
  `execute_at_*` (per-type in/out). N≈5, but they already share `put_stack`/`get_stack`.
- **After this design:** **N unchanged.** The host API calls the existing four; the only
  new ABI-touching code is *entry priming* (extracted verbatim from `execute_argv`, not a
  new copy) and a *type→`Value`* switch that delegates to the existing primitives.

Cure applied: **collapse to the chokepoint** — the host path selects an existing
`execute_at_*` by return type; it never opens the stack itself. A regression test
(`host call == in-language call`) makes any future divergence a *loud* failure, not a
silent one.

## API shape

```rust
// src/host.rs  →  pub mod host;
pub struct Program { data: Data, database: Stores }   // parsed IR + store schema; load once

impl Program {
    pub fn from_source(src: &str) -> Result<Program, LoftError>;   // stdlib + src
    pub fn from_file(path: &Path)  -> Result<Program, LoftError>;
    pub fn instance(&self)         -> Instance<'_>;                // fresh exec context
    pub fn call(&self, func: &str, args: &[Value]) -> Result<Value, LoftError>; // one-shot
}

pub struct Instance<'p> { state: State, data: &'p Data }          // primed; call many times
impl<'p> Instance<'p> {
    pub fn call(&mut self, func: &str, args: &[Value]) -> Result<Value, LoftError>;
}

pub enum Value {                     // host-side value
    Void,
    Bool(bool),
    Int(i64),                        // integer + u8/u16/i32/… — width taken from the param type
    Float(f64),                      // single
    Text(String),
    // DEFERRED: Struct, Vector, Enum, Null (see below)
}
impl Value { fn into_text(self)->Result<String,LoftError>; fn as_int(&self)->Option<i64>; … }

pub enum LoftError {
    Parse(String),
    UnknownFn(String),
    ArgCount  { func: String, expected: usize, got: usize },
    ArgType   { func: String, index: usize, expected: String, got: String },
    Unsupported { func: String, what: String },   // a struct/vector/enum in the signature (Phase 2)
    Runtime(String),                              // database.runtime_error, rendered
}
```

### `call` algorithm

1. `d_nr = data.def_nr(&format!("n_{func}"))` → `UnknownFn` if `u32::MAX`.
2. `def = data.def(d_nr)`; `params` = non-hidden attributes, `ret = def.returned`.
3. **Validate** `args.len() == params.len()`, and each `Value` variant is assignable to its
   param `Type` → `ArgCount` / `ArgType` with a readable message.
4. **Guard the supported surface**: every param + the return must be text / integer-family
   / single / boolean / void → else `Unsupported` (Phase 2 lifts this).
5. **Prime** the state (once per `Instance`): `data_ptr`, `parallel_ctx`, `fn_positions`,
   `source_spans`, `stack_trace_lib_nr` — the exact block from `execute_argv:3824-3842`,
   extracted into `State::prime_for_host(data)`.
6. **Marshal args**: first arg → `WorkerArg` (`Text(Str::new)` / `Primitive{value,size}` /
   `Ref`), remaining scalar/ref args → `extras: Vec<u64>`.
7. **Dispatch by return type** to the existing primitive:
   - `Type::Text`    → `execute_at_text(pos, arg, &extras, n_hidden_text)` → `String`
   - `Type::Integer/…/Boolean/single` → `execute_at_raw_*` → read primitive of the right width
   - `Type::Void`    → `execute_at_void`-family
   - `Type::Reference/Vector/Enum(_,true,_)` → `execute_at_ref` (Phase 2)
   `n_hidden_text` / `n_hidden_dests` computed from `def.attributes` exactly as
   `native.rs:1705-1722`.
8. **Marshal the return** → `Value`.
9. If `state.database.runtime_error` is `Some` → `Err(Runtime(rendered))` (drain it first,
   mirroring `main.rs:7648`), skipping the leak check on the error path.

## Failure paths (enumerated — this is where the invariant earns its keep)

| Path | Handling |
|---|---|
| unknown function name | `UnknownFn` (never a `def(u32::MAX)` panic) |
| wrong arg count / type | `ArgCount` / `ArgType` before any execution |
| a signature with a struct/vector/enum param or return | `Unsupported` (Phase 1) — **explicitly not silently mis-marshalled** |
| the loft fn `raise`s / `assert` fails / faults | `Runtime` (drained typed error, rendered) |
| text return | `execute_at_text` owns hidden-buffer alloc/push/**drop** — reused, not re-done |
| heap-returning fn (vector/ref) | Phase 2: hidden-dest sentinels + `execute_at_ref` + store adoption (`adopt_worker_excess`) — the store-lifetime path is the hard part, deferred deliberately |
| second call on same `Instance` | each `execute_at_*` re-seats its own frame (`stack_step(4)` + fresh sentinel); priming is idempotent |

## Supported vs deferred (the over-reach guard)

The cleanest-sounding claim — *"marshal any loft type uniformly"* — is the one to distrust.
Structs / vectors / enums return through **hidden destination params** and live in **stores**
whose lifetime must be adopted into the caller (`execute_at_ref` + `rebase`/`adopt`). That is
genuinely a different family, not a wider scalar. So:

- **Phase 1 (ship):** text · integer-family · single · boolean · void. Covers the formatter
  (`text → text`) and the large majority of host calls. A non-scalar in the signature is a
  clean `Unsupported`, never a wrong marshal.
- **Phase 2 (later, on demand):** struct/vector/enum via `execute_at_ref`, with a
  `Value::Struct`/`Vector` that carries a store handle + typed field accessors, and the
  adoption/lifetime handling proven on a boundary matrix. Not before a real consumer needs it.

## Validation (probe the invariant before trusting it)

Cheapest falsifier first — **prove `host call == in-language call`** for each supported
shape, because the prose "it routes through the same ABI" is exactly where an error hides:

1. `fn add(a: integer, b: integer) -> integer { a + b }` — `call("add",[Int(2),Int(3)])==Int(5)`.
2. `fn greet(name: text) -> text { "hi {name}" }` — text in *and* out (the hidden-buffer path).
3. `format(text) -> text` on a corpus file == the current CLI formatter output (dogfood).
4. Error paths: unknown fn, arg-count, arg-type, a `raise`ing fn → `Runtime`.
5. Second `call` on the same `Instance` returns the same as the first (frame re-seat).

If (1)/(2) diverge from running the same fn in a tiny program, the routing claim is false —
fix the priming/marshal, do not paper over it in the host layer.

## Then: `loft fmt`

`run_fmt_command(args)`:
1. `Program::from_source(include_str!("../tools/fmt/whole.loft"))` — once.
2. Parse `loft fmt [--check|--write] <file…>`.
3. Per file: read → `prog.call("format",[Value::Text(src)])?.into_text()?` → compare/emit.
   - default: print formatted to stdout
   - `--write`: write back iff changed; report changed files
   - `--check`: exit 1 if any file differs (clean message, no stacktrace) — CI gate.

No temp files, no stdout capture, no raw `execute_at_*` at the call site. The formatter
stays 100 % loft (dogfood); the CLI owns files / flags / exit codes.

## Phasing

- **P1** `State::prime_for_host` (extract from `execute_argv`) + `host::{Program,Instance,Value,LoftError}`
  + scalar/text/void marshalling routed through `execute_at_*` + the 5 validation tests.
- **P2** `loft fmt` on top of P1.
- **P3** (deferred) struct/vector/enum returns via `execute_at_ref` + `Value::Struct/Vector`,
  when a consumer needs them.
