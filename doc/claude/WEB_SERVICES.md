// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Web Services Syntax Evaluation

Design evaluation for HTTP client access in loft, built on top of lambda expressions (P1).

**Date:** 2026-03-18
**Prerequisite:** P1 (lambdas), P3 (aggregates), existing callable fn-refs (0.8.0)

---

## What loft already provides

- **JSON output** — `{value:j}` format flag serialises any struct/enum to JSON text.
- **Callable fn-refs** — named functions can be passed as values; type is `fn(A) -> B`.
- **Lambdas (P1, 0.8.3)** — `fn(x: T) -> U { body }` inline at call site.
- **`#rust "..."` annotation** — maps a loft `pub fn` declaration to a Rust implementation.
- **`not null` / `??`** — standard null-safety mechanism.
- **String interpolation** — `"{expr}"` is already loft's format string syntax.

What is **not** present yet:
- JSON deserialization (only serialization via `:j`).
- HTTP client functions.
- Type-parameterised generic calls (`fetch<T>(url)` style).

---

## Design constraints

| Constraint | Implication |
|---|---|
| Synchronous interpreter | HTTP calls are blocking; no `async/await` |
| No generic functions | Cannot write `fetch<User>(url)` — type must be resolved at compile time |
| No `Result<T,E>` | Error handling must use null, a status struct, or an error callback |
| Callable fn-refs exist | `User.from_json` can be passed to a parse helper without generics |
| `:j` already serialises | `to_json` is nearly free; `from_json` needs a new Rust implementation |

---

## Approach A — Service blocks (declarative)

New `service` keyword. Each endpoint declared as a typed function with an HTTP verb.
The compiler generates the HTTP call, URL construction, and JSON round-trip.

```loft
#json
struct Repo { name: text; description: text; stars: integer; language: text }

#json
struct Issue { title: text; state: text; number: integer }

#json
struct NewIssue { title: text; body: text }

service GitHub = "https://api.github.com" {
    header "Accept" = "application/vnd.github.v3+json"

    fn repo(owner: text, name: text) -> Repo = get "/repos/{owner}/{name}"
    fn issues(owner: text, repo: text) -> vector<Issue> = get "/repos/{owner}/{repo}/issues"
    fn create_issue(owner: text, repo: text, body: NewIssue) -> Issue
        = post "/repos/{owner}/{repo}/issues"
}

// Usage
let api = GitHub(header "Authorization" = "Bearer " + token)
let open = api.issues("rust-lang", "rust")
    .filter(fn(i: Issue) -> boolean { i.state == "open" })
    .map(fn(i: Issue) -> text { i.title })
```

**New syntax required:**
- `service Name = "base-url" { ... }` block with `header` and endpoint declarations
- `= get/post/put/delete "path"` endpoint bodies
- `{param}` in path strings as URL template substitution
- `#json` struct annotation (shared with all approaches)

**Pros:** Minimum call-site ceremony; URL templates are type-safe; headers centralised.
**Cons:** Large compiler surface; `header` in service body and constructor creates two contexts;
path templates and struct fields must be validated at compile time.

---

## Approach B — Annotated structs + fn-ref deserialization

`#json` annotation generates `from_json` and `to_json` for each tagged struct.
HTTP is a plain stdlib: `http_get(url) -> HttpResponse`.
Lambdas and fn-refs handle the deserialization step.

```loft
#json
struct User { id: integer; name: text; email: text }

#json
struct NewUser { name: text; email: text }

struct HttpResponse {
    status: integer
    body:   text
    ok:     boolean
}

// Stdlib (implemented in Rust via #rust):
pub fn http_get(url: text) -> HttpResponse;
pub fn http_get_with(url: text, headers: vector<text>) -> HttpResponse;
pub fn http_post(url: text, body: text) -> HttpResponse;
pub fn http_post_with(url: text, body: text, headers: vector<text>) -> HttpResponse;
pub fn json_array(body: text, parse: fn(text) -> $T) -> vector<$T>;

// Usage — simple GET
let resp = http_get("https://api.example.com/users")
let users = if resp.ok { json_array(resp.body, User.from_json) } else { [] }

// Usage — POST with body
let payload = NewUser { name: "Alice", email: "a@example.com" }
let resp2 = http_post("https://api.example.com/users", "{payload:j}")
let created = if resp2.ok { User.from_json(resp2.body) } else { null }

// Usage — with headers via vector<text>
let auth = ["Authorization: Bearer " + token, "Accept: application/json"]
let resp3 = http_get_with("https://api.github.com/user", auth)

// Usage — filter + map with lambdas
let names = users
    .filter(fn(u: User) -> boolean { u.name.starts_with("A") })
    .map(fn(u: User) -> text { u.email })
```

**`#json` generates (compiler-synthesised for each annotated struct):**
```loft
// to_json: free — already works via ":j" format
pub fn to_json(self: User) -> text { "{self:j}" }

// from_json: compiler emits one json_field_* call per struct field
pub fn from_json(body: text) -> User {
    User {
        id:    json_int(body, "id"),
        name:  json_text(body, "name"),
        email: json_text(body, "email"),
    }
}
```

**New syntax required:**
- `#json` struct annotation only.
- `json_array(body, parse_fn)` is a stdlib function, not new syntax.
- Headers as `vector<text>` — no new syntax.

**Pros:** Minimal new syntax; `User.from_json` is a fn-ref and composes cleanly with `map`;
`to_json` reuses existing `:j` format; no new keywords.
**Cons:** Headers as `vector<text>` is slightly clunky; `json_array` needs a macro-like
`$T` type variable for the element type (or must be replaced by lambdas for the array case).

---

## Approach C — Builder chain with explicit lambda parsing

No struct annotation. HTTP returns a builder; lambdas or fn-refs transform the response.
The parsing lambda is always explicit — no implicit JSON mapping.

```loft
fn parse_user(body: text) -> User {
    User {
        id:    json_int(body, "id"),
        name:  json_text(body, "name"),
        email: json_text(body, "email"),
    }
}

// Usage
let user = http("https://api.example.com/users/42")
    .header("Authorization", "Bearer " + token)
    .get()
    .on_ok(parse_user)
    .on_error(fn(status: integer) -> User { User { id: 0, name: "", email: "" } })

// Inline lambda parsing
let names = http("https://api.example.com/users")
    .get()
    .on_ok(fn(body: text) -> vector<text> {
        json_array(body, fn(item: text) -> text { json_text(item, "name") })
    })
    .on_error(fn(status: integer) -> vector<text> { [] })
```

**New syntax required:**
- `http(url)` builder type with `.header()`, `.get()`, `.on_ok()`, `.on_error()` methods.
- Builder must carry the response type through the chain — needs careful type design.

**Pros:** Fully explicit; no hidden codegen; lambdas are front and centre.
**Cons:** Verbose; error callbacks required on every call even for throw-away scripts;
builder-chain typing is hard to implement without generics;
`on_ok(fn(text)->T) -> T` depends on the lambda's return type, which loft cannot infer
without tracking type parameters through the builder.

---

## Approach D — Direct stdlib only, manual JSON extraction

No new syntax at all. Plain functions for HTTP and for JSON field access.
All parsing is done by the caller.

```loft
// Stdlib:
pub fn http_get(url: text) -> text;            // returns body; empty string on error
pub fn http_status() -> integer;               // status of most recent call (thread-local)
pub fn json_text(body: text, key: text) -> text;
pub fn json_int(body: text, key: text) -> integer;
pub fn json_bool(body: text, key: text) -> boolean;
pub fn json_items(body: text) -> vector<text>; // top-level array → item bodies

// Usage
let body = http_get("https://api.example.com/users/" + id)
if http_status() == 200 {
    User {
        id:    json_int(body, "id"),
        name:  json_text(body, "name"),
        email: json_text(body, "email"),
    }
}

// Nested: array of users
let raw_users = json_items(http_get("https://api.example.com/users"))
let users = raw_users.map(fn(item: text) -> User {
    User { id: json_int(item, "id"), name: json_text(item, "name"), email: json_text(item, "email") }
})
```

**New syntax required:** None.

**Pros:** Simplest to implement; no new language features beyond P1; thread-local
`http_status()` pattern is familiar from C stdlib.
**Cons:** `http_status()` is implicit global state — breaks with parallel workers;
repetitive struct construction; no compile-time guarantee that field names match.

---

## Comparison

| | A (service blocks) | B (#json + HttpResponse) | C (builder chain) | D (direct stdlib) |
|---|---|---|---|---|
| New syntax | `service`, `header`, endpoint decl | `#json` annotation only | builder type + chain | none |
| JSON safety | compile-time | annotation-driven codegen | manual | manual |
| Error handling | implicit (throws/null) | explicit `resp.ok` | `.on_error()` lambda | `http_status()` global |
| Headers | centralised in block | `vector<text>` arg | `.header()` builder | none (no headers) |
| Parallel-safe | yes | yes | yes | no (`http_status()` global) |
| Compose with lambdas | yes (filter/map after call) | yes | yes | yes |
| Implementation effort | Very High | Medium | High | Low |
| Prototype friction | Very Low | Low | Medium | Medium |

---

## Recommendation: Approach B (`#json` + `HttpResponse` stdlib)

**Rationale:**

1. **`#json` is the key enabler.** `User.from_json` becomes a callable fn-ref, and fn-refs
   already work in loft. This means `json_array(body, User.from_json)` and
   `users.map(fn(u: User) -> text { u.name })` compose naturally — no generics needed.

2. **`HttpResponse` is a plain struct.** No builder magic, no thread-local state, no chain
   typing problem. `if resp.ok { ... }` is idiomatic loft.

3. **`to_json` is nearly free.** The compiler can synthesise `"{self:j}"` for any `#json`
   struct, reusing the existing format infrastructure.

4. **Minimal new syntax.** Only `#json` is new. HTTP functions are `#rust` stdlib additions.
   Service blocks (Approach A) can be added later as ergonomic sugar once B proves itself.

5. **Headers as `vector<text>`** is admittedly clunky.  A thin `Headers` struct
   (`struct Headers { items: vector<text> }` with an `add(key, value)` method) can be added
   to the stdlib without new syntax.

### Recommended stdlib additions

```loft
// HTTP response
pub struct HttpResponse {
    status: integer
    body:   text
}

pub fn ok(self: HttpResponse) -> boolean { self.status >= 200 and self.status < 300 }

// Blocking HTTP — implemented via ureq (small, no async)
pub fn http_get(url: text) -> HttpResponse;
pub fn http_post(url: text, body: text) -> HttpResponse;
pub fn http_put(url: text, body: text) -> HttpResponse;
pub fn http_delete(url: text) -> HttpResponse;

// With headers
pub fn http_get_h(url: text, headers: vector<text>) -> HttpResponse;
pub fn http_post_h(url: text, body: text, headers: vector<text>) -> HttpResponse;

// JSON primitives — operate on a JSON object body
pub fn json_text(body: text, key: text) -> text;
pub fn json_int(body: text, key: text) -> integer;
pub fn json_long(body: text, key: text) -> long;
pub fn json_float(body: text, key: text) -> float;
pub fn json_bool(body: text, key: text) -> boolean;
pub fn json_items(body: text) -> vector<text>;      // JSON array → item bodies
pub fn json_nested(body: text, key: text) -> text;  // nested object → body text

// #json annotation generates for each tagged struct:
// pub fn to_json(self: T) -> text      (synthesised: "{self:j}")
// pub fn from_json(body: text) -> T    (synthesised: field-by-field extraction)
```

### Usage patterns

```loft
#json
struct User { id: integer; name: text; email: text }

#json
struct NewUser { name: text; email: text }

// GET a single resource
let resp = http_get("https://api.example.com/users/42")
let user = if resp.ok() { User.from_json(resp.body) } else { null }

// GET an array, transform with lambdas
let resp2 = http_get("https://api.example.com/users")
let names = if resp2.ok() {
    json_items(resp2.body)
        .map(User.from_json)
        .filter(fn(u: User) -> boolean { u.name != "" })
        .map(fn(u: User) -> text { u.name })
} else { [] }

// POST with JSON body
let new_user = NewUser { name: "Alice", email: "a@example.com" }
let resp3 = http_post("https://api.example.com/users", new_user.to_json())
let created = if resp3.ok() { User.from_json(resp3.body) } else { null }

// With auth header
let auth = ["Authorization: Bearer " + token]
let resp4 = http_get_h("https://api.github.com/user", auth)

// Nested JSON (no #json struct needed for simple cases)
let resp5 = http_get("https://api.example.com/stats")
let count = json_int(resp5.body, "user_count")
let label = json_text(json_nested(resp5.body, "meta"), "version")
```

---

## `#json` implementation plan

**Phase 1 — to_json (trivial):**
The compiler recognises `#json` on a struct; for each struct `T` it synthesises
`pub fn to_json(self: T) -> text { "{self:j}" }`.  This reuses the existing JSON format
flag.  `from_json` is not yet generated.

**Phase 2 — from_json for scalar fields:**
For each `#json` struct the compiler synthesises `from_json(body: text) -> T` by emitting
one `json_text/json_int/json_bool/json_float/json_long` call per primitive field.  Enum and
nested struct fields are not yet supported.

**Phase 3 — nested structs and enums:**
Fields whose type is itself `#json`-annotated use `json_nested(body, key)` + the nested
type's `from_json`.  Enum fields are handled via a `json_text` key matched against variant
names.

**Phase 4 — arrays and optional fields:**
`vector<T>` fields map to `json_items(json_nested(body, key)).map(T.from_json)`.
Fields declared without `not null` default to the zero value when the JSON key is absent.

---

## Dependency on Rust crates

HTTP requires a blocking HTTP client.  Recommended: **`ureq`** (no async, pure Rust,
~100 KB compiled, no OpenSSL dependency).  Gate it behind an `http` Cargo feature.

JSON field extraction does **not** require `serde_json`.  The existing parsing
primitives in `src/database/structures.rs` (`match_text`, `match_integer`, `match_float`,
`match_boolean`, `skip_float`) already handle every JSON value type.  A new
`src/database/json.rs` module (~80 lines, no new dependency) adds three `pub(crate)`
functions on top of those primitives:

- `json_get_raw<'a>(text: &'a str, key: &str) -> Option<&'a str>` — find a key in
  a JSON object and return the raw value slice.
- `json_array_items(text: &str) -> Vec<String>` — return each element of a JSON array
  as a raw JSON string.
- `as_text / as_int / as_long / as_float / as_bool` — parse a raw JSON value slice into
  a Rust primitive.

The H2 native functions in `src/native_http.rs` call these helpers directly.  No
`serde_json` dependency is added.  See H2 in PLANNING.md for the full implementation
plan.

### `src/database/json.rs` — design sketch

```rust
// Skip whitespace.
fn skip_ws(text: &str, pos: &mut usize) { ... }

// Skip a complete JSON value (object, array, string, number, literal).
// Returns false if text is malformed.
fn skip_value(text: &str, pos: &mut usize) -> bool { ... }

// Extract and unescape a quoted JSON string.
fn extract_string(text: &str, pos: &mut usize) -> Option<String> { ... }

// Find `key` in a top-level JSON object; return the raw value slice.
pub(crate) fn json_get_raw<'a>(text: &'a str, key: &str) -> Option<&'a str> { ... }

// Return raw JSON text for each element of a top-level JSON array.
pub(crate) fn json_array_items(text: &str) -> Vec<String> { ... }

// Conversion helpers (return loft null sentinels on failure):
pub(crate) fn as_text(raw: &str) -> String { ... }  // strips quotes + unescapes
pub(crate) fn as_int(raw: &str) -> i32 { ... }      // i32::MIN on failure
pub(crate) fn as_long(raw: &str) -> i64 { ... }     // i64::MIN on failure
pub(crate) fn as_float(raw: &str) -> f64 { ... }    // f64::NAN on failure
pub(crate) fn as_bool(raw: &str) -> bool { ... }    // false on failure
```

`skip_value` handles nesting depth through recursion on `{`/`[` tokens, and handles
`\"` inside strings.  It mirrors the existing `parsing()` flow in `structures.rs` but
operates schema-free: no `Stores`, no `DbRef`, no type lookup needed.

The `ureq` dependency is still required for the HTTP client and remains gated behind
the `http` Cargo feature.

---

## Issues identified

- **#54** — `json_items` returns `vector<text>` but type of each item is opaque at compile
  time; no way to validate that `User.from_json` receives a valid JSON object body vs an
  arbitrary string.  Accepted limitation for now; a future `JsonValue` enum could address it.
- **#55** — `http_status()` global (Approach D) is not parallel-safe; `HttpResponse` struct
  (Approach B) avoids this entirely.  Do not add `http_status()` even as a convenience.

See [PROBLEMS.md](PROBLEMS.md) for the issue entries.

---

## See also
- [PLANNING.md](PLANNING.md) — H1–H5 items: full fix paths, effort estimates, and target milestone (0.8.4)
- [STDLIB.md](STDLIB.md) — `json_items` and existing text functions usable with JSON
- [LOFT.md](LOFT.md) — Struct annotation syntax (`#json`) and callable fn-ref conventions
- [THREADING.md](THREADING.md) — Why `http_status()` cannot be parallel-safe (issue #55)
