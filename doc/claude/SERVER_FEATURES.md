// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Language Features for Server and Game Client Libraries

Design for five language and API improvements identified by evaluating the
`server` and `game_client` library designs.  Each item is motivated by a
concrete pain point in the library code and removes either a workaround, a
source of bugs, or unnecessary boilerplate.

---

## Contents
- [Overview — pain points and fixes](#overview)
- [C55 — Type aliases](#c55--type-aliases)
- [C56 — Null-coalesce with early return (`?? return`)](#c56--null-coalesce-with-early-return)
- [A15 — `parallel { }` structured concurrency](#a15--parallel---structured-concurrency)
- [I13 — Iterator protocol (`for x in custom`)](#i13--iterator-protocol)
- [C57 — Route decorator syntax (`@get`, `@post`, `@ws`)](#c57--route-decorator-syntax)
- [Implementation order and milestones](#implementation-order-and-milestones)
- [See also](#see-also)

---

## Overview

Evaluating the `server` and `game_client` source patterns surfaces five
recurring pain points:

| Pain point | Current workaround | Feature |
|------------|-------------------|---------|
| Function type spelled out in every API signature | Repeated long literals | C55 — type aliases |
| Null-result early exit is 2-line check + return | if/return pattern | C56 — `?? return` |
| Run server + game loop concurrently | native `n_spawn_thread` hack | A15 — `parallel {}` |
| WebSocket loop needs explicit MAX_INT sentinel | ugly for-with-flag pattern | I13 — iterator protocol |
| Route registration separated from handler | explicit registration block | C57 — `@route` decorator |

None of these are new concepts in language design — they are well-understood
features with clear implementation paths.  They are ordered here from smallest
to largest effort so the highest-return items can land first.

---

## C55 — Type aliases

**Motivation:** every route registration function repeats the same function
type, making the server API hard to read and the type hard to change.

```loft
// Today — same complex type written in every signature:
pub fn route(app: &App, method: text, pattern: text, handler: fn(Request) -> Response)
pub fn get(app: &App, pattern: text, handler: fn(Request) -> Response)
pub fn post(app: &App, pattern: text, handler: fn(Request) -> Response)
pub fn put(app: &App, pattern: text, handler: fn(Request) -> Response)
pub fn delete(app: &App, pattern: text, handler: fn(Request) -> Response)
pub fn route_ws(app: &App, pattern: text, handler: fn(Request, &WebSocket))
pub fn use_middleware(app: &App, mw: Middleware)

// And on the call side, the user must also write it out each time they
// store a handler in a variable:
handler: fn(Request) -> Response = fn handle_health;
```

**Feature:** a `type` declaration at file scope creates a named alias.

```loft
// In server.loft — defined once:
pub type Handler   = fn(Request) -> Response
pub type WsHandler = fn(Request, &WebSocket)

// Signatures become readable:
pub fn route(app: &App, method: text, pattern: text, handler: Handler)
pub fn get(app: &App, pattern: text, handler: Handler)
pub fn route_ws(app: &App, pattern: text, handler: WsHandler)

// User code:
h: Handler = fn handle_health;
```

### Semantics

- A type alias is purely a compile-time substitution.  `Handler` and
  `fn(Request) -> Response` are the same type; no implicit conversion needed.
- Aliases may be exported with `pub type`.  Importing code can use `Handler`
  as if it were defined locally.
- Recursive or mutually recursive aliases are a compile error (not needed and
  hard to implement soundly).
- Generic aliases are not included in this item:
  `type Result<T> = T?` is deferred — generics complicate monomorphisation.

### Implementation

1. **Lexer:** `type` is already a keyword candidate (it is reserved in most
   contexts).  Ensure it tokenises as `Token::Type`.
2. **Parser (definitions.rs):** add `parse_type_alias()` — consume `type`,
   name (CamelCase), `=`, type expression.  Store in `Data` as
   `Def::TypeAlias { name, target: Type }`.
3. **Type resolution (typedef.rs):** when resolving a named type, check
   `Def::TypeAlias` and expand to its target recursively (cycle detection via
   visit set).
4. **Codegen:** no changes required — aliases expand before bytecode emission.
5. **Native codegen (generation/mod.rs):** aliases expand at the same point as
   the interpreter.

**Tests:** `type Handler = fn(Request) -> Response`; use in parameter; verify
expanded type is stored; verify cycle detection errors cleanly.

**Effort:** XS — ~50 lines in parser + typedef.rs.

---

## C56 — Null-coalesce with early return

**Motivation:** every handler that calls a nullable function must test the
result and return an error response, producing a two-line check for every
optional lookup.

```loft
// Today — two lines per nullable result:
fn handle_get_post(req: Request) -> Response {
    id_text = param(req, "id");
    if id_text == null { return response_bad_request("missing id"); }
    id = id_text as integer;
    if id == null { return response_bad_request("id must be integer"); }
    post = find_post(id);
    if post == null { return response_not_found(); }
    response_json("{post:j}")
}
```

**Feature:** allow a `return` expression on the right-hand side of `??`.  If
the left-hand side is null, the function returns immediately with the
right-hand expression's value.

```loft
// With ?? return — one line per nullable result:
fn handle_get_post(req: Request) -> Response {
    id_text = param(req, "id")      ?? return response_bad_request("missing id");
    id      = id_text as integer    ?? return response_bad_request("id must be integer");
    post    = find_post(id)         ?? return response_not_found();
    response_json("{post:j}")
}
```

The same pattern works in `game_client` handlers:

```loft
fn on_state_full(env: GameEnvelope) {
    state = env.payload as GameState ?? return;   // ignore malformed packets
    sync_apply_full(sync, state);
    request_render();
}
```

### Semantics

`expr ?? return val` desugars to:

```
tmp = expr
if tmp == null { return val }
tmp
```

The `return` keyword on the right side of `??` is a **control-flow expression**
that produces the function's return type.  `??` with a `return` right-hand side
has the same type as the left-hand side's inner type (non-null).

`?? return` (with no value) is valid when the enclosing function returns void.

### Implementation

1. **Parser (expressions.rs):** in `parse_null_coalesce()`, after consuming
   `??`, if the next token is `Token::Return`, consume it, parse the optional
   return expression, and construct `Value::NullCoalesceReturn { left, ret }`.
2. **Codegen (codegen.rs):** `NullCoalesceReturn` emits:
   - evaluate left into a temp slot
   - `OpJumpNotNull` → skip the return path
   - evaluate the return expression (if any)
   - `OpReturn`
   - land target: the temp slot value is now the result
3. **Type checker:** the result type of `NullCoalesceReturn` is the non-null
   inner type of the left expression; the right-hand return expression must
   match the function's declared return type.

**Tests:** `x ?? return default_val`; `x ?? return` (void); chained `?? return`
expressions; nesting inside other expressions.

**Effort:** XS — ~30 lines in expressions.rs + codegen.rs.

---

## A15 — `parallel { }` structured concurrency

**Motivation:** a game server needs to run a fixed-rate tick loop and an HTTP +
WebSocket server simultaneously.  Today this requires calling the native
`n_spawn_thread` workaround, which bypasses loft's store safety model.

```loft
// Today — native spawn required; store safety not guaranteed:
spawn_thread(fn serve_tasks);
run_game_loop(GameLoop { tick_rate: 20 }, fn game_tick);
```

**Feature:** `parallel { }` — a statement that runs each top-level expression
in the block as an independent concurrent task and waits for all of them to
finish.

```loft
fn main() {
    // ... set up Apps, registry, game state ...
    parallel {
        serve_all([public, ws_app]);
        run_game_loop(GameLoop { tick_rate: 20 }, fn game_tick);
    }
    // Execution continues only when both have returned.
}
```

### Semantics

- Each expression in the `parallel {}` body is a **task arm**.
- Task arms run on separate OS threads (or tokio tasks in the native layer).
- The `parallel {}` block **blocks the calling thread** until all arms complete.
- Early return / panic in one arm: all other arms receive a cancellation signal
  and are given `read_timeout` seconds to terminate; after the timeout the
  process exits.
- **Store isolation** — same model as `par(...)` workers:
  - Each arm receives a read-only snapshot of the outer stores at the moment
    `parallel {}` is entered.
  - Writes made by one arm are **not** visible to other arms during execution.
  - On completion, writes from all arms are merged using the same last-write
    policy as parallel loops.
  - For game servers, the arms communicate through native-backed shared state
    (`ConnectionRegistry`, rate-limit counters) rather than through loft stores.
    This is the same pattern as parallel workers sharing a database store via
    `par()`.

### Why not just `par()`?

`par(...)` maps over a collection — it is designed for homogeneous data
parallelism over many items.  `parallel {}` runs a **fixed set of distinct
tasks** with different types and lifetimes; it is structured task concurrency.

### Difference from spawning a background thread

`n_spawn_thread` (the workaround) fires and forgets.  `parallel {}` is
**scoped**: the block can only exit after all arms return, preventing use-after-
free of stack variables and ensuring deterministic merge on completion.  This is
the same safety property that Rust's `std::thread::scope` provides.

### Implementation

1. **Lexer/parser (expressions.rs):** parse `parallel { arm1; arm2; ... }` as
   a new `Value::Parallel { arms: Vec<Value> }`.  Each arm is an expression
   statement.
2. **Codegen (codegen.rs):** emit `OpParallelBegin(n_arms)`, one `OpParallelArm`
   per arm (contains the arm's bytecode offset), `OpParallelJoin`.
3. **State executor (fill.rs / state/mod.rs):** `OpParallelBegin` spawns N
   threads (from the existing thread pool) each jumping to their arm's bytecode.
   `OpParallelJoin` blocks until all have signalled completion.  Worker threads
   use the same store-isolation mechanism as `par()` workers.
4. **Scope analysis (scopes.rs):** variables declared inside a `parallel {}` arm
   are not visible in other arms or outside the block.  Variables captured from
   the outer scope are read-only inside the arm (same restriction as `par()` body
   parameters).

**Tests:** `parallel { slow_task(); fast_task(); }` — verify both run, result
is deterministic.  Test store-merge policy.  Test cancellation on panic.

**Effort:** M — new parser construct + 3 new opcodes + store-merge policy for
long-running tasks (not just one-iteration workers).  Builds on the existing
`par()` infrastructure.

---

## I13 — Iterator protocol

**Motivation:** a WebSocket receive loop must be written as an explicit
infinite `for` loop with a mutable flag because loft's `for` only supports
built-in collections.

```loft
// Today — awkward infinite loop with mutable exit flag:
running = true;
for _ in 0..1000000 if running {
    msg = ws_receive(ws);
    match msg {
        Close { _, _ } => running = false,
        Text { content } => process(content),
        _ => {},
    }
}
```

The same pattern appears in the game client's event loop, state sync loop, and
ping tracker.

**Feature:** any type that defines `fn next(self: &Self) -> T?` can be used
directly in a `for` loop.  Returning `null` from `next` terminates the loop.

```loft
// WebSocket implements the iterator protocol (in websocket.loft):
pub fn next(self: &WebSocket) -> WsMessage? {
    msg = ws_receive(self);
    match msg {
        Close { _, _ } => null,
        other          => other,
    }
}

// Handler becomes clean and idiomatic:
for msg in ws {
    match msg {
        Text { content } => process(content),
        _ => {},
    }
}
```

### Formal definition

```loft
// The iterator protocol — not a formal interface yet (deferred to I5+),
// but a structural convention recognised by the for-loop desugaring:
//   if T has fn next(self: &T) -> Item?
//   then  for x in val : T  is valid

// Desugaring:
for x in val {
    body
}
// becomes:
for _ in 0..2147483647 {
    x = val.next();
    if x == null { break; }
    body
}
```

When the full interface system (I5+) lands, `Iterator<T>` becomes a formal
interface and the protocol is enforced at compile time.  Before that, the
for-loop desugaring applies to any type that structurally has a matching `next`
method — duck-typing at the parser level.

### Types that benefit from the iterator protocol

| Type | Item | Loop pattern |
|------|------|-------------|
| `WebSocket` | `WsMessage` | Receive loop until Close |
| `WsClient` (game_client) | `WsMessage` | Client receive loop |
| `ConnectionRegistry` | `(text, WebSocket)` | Iterate all connections |
| User-defined producers | any | Custom iterators |

### Loop attributes with custom iterators

`#count` works as usual (counts completed iterations).
`#first` works as usual (true on the first call to `next()`).
`#index` and `#remove` are not available for custom iterators.

### Implementation

1. **Parser (collections.rs):** in `parse_for()`, after resolving the iterable
   type, check if it has a method `next` returning `T?`.  If so, construct the
   desugared `Value::For` block rather than emitting a collection iterator.
2. **Codegen:** desugaring is entirely at the parser/IR level — no new opcodes
   needed.  The generated IR is a standard `for _ in 0..MAX` with a
   `ws.next()` call and a null-break.
3. **Type checking:** the inferred type of the loop variable is `T` (the inner
   type of `T?` returned by `next`).

**Tests:** `for msg in ws` on a mock WebSocket that returns three messages then
null; verify 3 iterations; verify the `break` fires on null.

**Effort:** MH — parser change in `parse_for()` + type resolution for method
lookup; no new opcodes; requires method dispatch to work on reference types.
Formally depends on I5+ for enforcement; structurally can land without it.

---

## C57 — Route decorator syntax

**Motivation:** in a real application, route registrations appear far from the
handler functions they register.  With 20–40 routes, finding the URL for a
handler requires scrolling to the `main()` registration block.

```loft
// Today — handler and its URL are in different places:
fn handle_health(req: Request) -> Response { response_ok("ok") }
fn handle_login(req: Request) -> Response { ... }
fn handle_user(req: Request) -> Response { ... }
// ... 300 lines ...
fn main() {
    app = new_app(srv);
    get(app,  "/health",  fn handle_health);
    post(app, "/login",   fn handle_login);
    get(app,  "/users",   fn handle_user);
    // easy to get out of sync
}
```

**Feature:** `@annotation` syntax before function definitions.  An annotation
is a compile-time registration call synthesised by the library that defines the
annotation.

```loft
@get("/health")
fn handle_health(req: Request) -> Response { response_ok("ok") }

@post("/login")
fn handle_login(req: Request) -> Response { ... }

@ws("/ws/chat")
fn handle_chat(req: Request, ws: &WebSocket) { ... }

fn main() {
    app = new_app(Server { port: 8080 });
    register_routes(app);   // generated: calls get/post/ws for each annotation
    serve(app);
}
```

### Annotation expansion model

The `server` library declares annotations:

```loft
// In server.loft:
annotation get(pattern: text)       // expands to: get(app, pattern, fn decorated)
annotation post(pattern: text)      // expands to: post(app, pattern, fn decorated)
annotation put(pattern: text)
annotation delete(pattern: text)
annotation ws(pattern: text)        // expands to: route_ws(app, pattern, fn decorated)
```

At compile time:
1. The parser collects all `@get(...)`, `@post(...)`, etc. annotations.
2. A synthetic `register_routes(app: &App)` function is generated containing
   one registration call per annotation, in declaration order.
3. The user calls `register_routes(app)` in `main()`.

The `app` variable is **not** captured at annotation time — it is passed to
`register_routes` when the user calls it.  This avoids introducing
implicit global state.

### Annotation definition syntax

```loft
// Define an annotation named 'get' with one parameter 'pattern':
annotation get(pattern: text)
    expands fn(handler: fn(Request) -> Response) {
        get(app, pattern, fn handler)
    }
```

The `app` name in the expansion is a well-known identifier resolved from the
`register_routes` call site — similar to how Rust procedural macros emit code
that resolves names at the call site.

### What annotations are NOT

- Not runtime metadata — annotations have no runtime representation.
- Not general macros — they apply only to function definitions, not expressions.
- Not Turing-complete — the expansion body is a fixed template; no conditional
  logic or loops are allowed in annotation expansion.

### Why this cannot be done with closures alone (A5)

Even when A5 (closure capture) lands, the co-location problem remains: the
route URL and handler would still be registered in `main()`.  Decorators co-
locate the URL directly with the handler declaration, which is a different
ergonomic property than closures provide.

### Implementation

1. **Lexer:** `@` as a new token `Token::At`.
2. **Parser (definitions.rs):** before parsing a `fn`, check for one or more
   `@name(args)` annotations.  Each is stored as `Annotation { name, args }`
   on the `FnDef`.
3. **Annotation registry:** the compiler maintains a table of declared
   annotations (from `annotation name(params) expands ...` declarations).
   These are collected in the first pass.
4. **Synthesis pass:** after parsing all definitions, generate the
   `register_routes` function body by iterating annotated functions and
   expanding each annotation's template.
5. **Error reporting:** unknown annotation name → compile error.  Mismatched
   parameter count → compile error.  Annotation on a non-function → compile
   error.

**Tests:** two annotated handlers; `register_routes(app)` calls both; verify
routing; test error cases (unknown annotation, wrong parameter type).

**Effort:** H — new token, new definition form, annotation registry, synthesis
pass; requires the two-pass parser infrastructure already present.

---

## Implementation order and milestones

| ID | Feature | Effort | Milestone | Key benefit |
|----|---------|--------|-----------|-------------|
| C55 | Type aliases (`type Handler = fn(...)`) | XS | 0.8.4 Sprint 10 | Readable API signatures |
| C56 | `?? return expr` null early-exit | XS | 0.8.4 Sprint 10 | Concise null-safe handlers |
| A15 | `parallel { }` structured concurrency | M | 0.8.4 Sprint 10 | Game loop + server concurrently |
| I13 | Iterator protocol (`for msg in ws`) | MH | 0.8.4 Sprint 10 | Natural WebSocket loops |
| C57 | `@get` / `@post` route decorator | H | 1.1+ | Co-locate URL and handler |

**C55 and C56 are highest priority** — they are tiny changes with immediate
payoff in every handler function in every loft web application.  They should
land together in a single PR.

**A15 is the enabler for the game-server pattern** — without it, the server
and game loop must be wired together with a native workaround.

**I13 depends on method dispatch working cleanly on reference types** and
benefits from the I5+ interface system for type-safety.  It can land
structurally before I5+ but is more valuable with formal interface enforcement.

**C57 is the most complex** and the most easily replaced by clear documentation
(put registration near the handler in comments).  It should be deferred until
the other four are stable.

---

## See also

- [WEB_SERVER_LIB.md](WEB_SERVER_LIB.md) — server library design + game server additions
- [GAME_CLIENT_LIB.md](GAME_CLIENT_LIB.md) — game client library design
- [PLANNING.md](PLANNING.md) — items C55, C56, A15, I13, C57
- [INTERFACES.md](INTERFACES.md) — I5+ interface system (prerequisite for formal I13)
- [PLANNING.md § A5](PLANNING.md) — closure capture (prerequisite for factory-style middleware)
