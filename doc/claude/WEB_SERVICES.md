
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Web Services Design

HTTP client and JSON support for loft, built on existing parsing infrastructure.

**Updated:** 2026-03-24
**Status:** JSON serialization and deserialization work.  HTTP client is deferred to 1.1+ (H4 in PLANNING.md).

---

## What loft already provides

| Capability | Syntax | Status |
|---|---|---|
| JSON output | `"{value:j}"` format flag | Working |
| JSON input (struct) | `MyStruct.parse(json_text)` | Working |
| Parse error tracking | `record#errors` accessor | Working |
| Quoted field names | Both `name: value` and `"name": value` | Working |
| Field constraints | `assert(expr)` in struct definitions | Working |
| Callable fn-refs | `fn worker` passed as value | Working |
| Lambdas | `fn(x: T) -> U { body }` | Working |
| String interpolation | `"{expr}"` | Working |

**No `#json` annotation needed.** The existing `Type.parse()` mechanism handles JSON
deserialization for any struct, and `":j"` handles serialization.  This eliminates the
need for compiler-synthesized `from_json` / `to_json` methods entirely.

**What is still needed:**
- HTTP client functions (`http_get`, `http_post`, etc.)
- `HttpResponse` struct in the standard library
- User documentation page

---

## JSON — already working

### Serialization (struct → JSON text)

```loft
struct User { id: integer; name: text; email: text }

u = User { id: 42, name: "Alice", email: "a@example.com" };
json = "{u:j}";
// → {"id":42,"name":"Alice","email":"a@example.com"}
```

The `:j` format flag works on any struct, enum, or vector.  No annotation required.

### Deserialization (JSON text → struct)

```loft
input = `{"id":42,"name":"Alice","email":"a@example.com"}`;
u = User.parse(input);
assert(u.name == "Alice");
```

`Type.parse()` accepts both JSON-style (`"name": value`) and loft-style (`name: value`)
field names.  Missing fields get null sentinels.  Extra fields are skipped.

### Error handling

```loft
bad = User.parse(`{"id":"not_a_number"}`);
for e in bad#errors {
    log_info("parse error: {e}");
}
```

The `#errors` accessor returns an iterable of error messages from the most recent parse.

### Vectors

```loft
struct Score { value: integer }
items = `[{"value":10},{"value":20},{"value":30}]`;
scores = vector<Score>.parse(items);
assert(len(scores) == 3);
```

### Field constraints (validation)

```loft
struct User {
    id: integer
        assert(id > 0)
    name: text
        assert(len(name) > 0, "name must not be empty")
    email: text
}

u = User.parse(`{"id":-1,"name":"","email":"x"}`);
// u#errors contains "id > 0", "name must not be empty"
```

---

## HTTP client — planned

### Design: `HttpResponse` struct + plain functions

No builder chains, no service blocks, no new syntax.  HTTP calls are regular stdlib
functions that return a struct.  Implemented via `ureq` (small blocking HTTP library,
feature-gated).

### Stdlib additions

```loft
// In default/06_web.loft (feature = "http")

pub struct HttpResponse {
    status: integer
    body:   text
}

pub fn ok(self: const HttpResponse) -> boolean {
    self.status >= 200 and self.status < 300
}

// Simple HTTP verbs
pub fn http_get(url: text) -> HttpResponse
pub fn http_post(url: text, body: text) -> HttpResponse
pub fn http_put(url: text, body: text) -> HttpResponse
pub fn http_delete(url: text) -> HttpResponse

// With headers (vector of "Name: Value" strings)
pub fn http_get_h(url: text, headers: vector<text>) -> HttpResponse
pub fn http_post_h(url: text, body: text, headers: vector<text>) -> HttpResponse
pub fn http_put_h(url: text, body: text, headers: vector<text>) -> HttpResponse
```

### Usage patterns

```loft
struct User { id: integer; name: text; email: text }

// GET and parse a single JSON object
resp = http_get("https://api.example.com/users/42");
if resp.ok() {
    user = User.parse(resp.body);
    log_info("got user: {user.name}");
}

// GET and parse a JSON array
resp = http_get("https://api.example.com/users");
if resp.ok() {
    users = vector<User>.parse(resp.body);
    for u in users {
        log_info("{u.name}: {u.email}");
    }
}

// POST with JSON body
new_user = User { name: "Alice", email: "a@example.com" };
resp = http_post("https://api.example.com/users", "{new_user:j}");
if resp.ok() {
    created = User.parse(resp.body);
    log_info("created user #{created.id}");
}

// With authorization header
auth = ["Authorization: Bearer " + token];
resp = http_get_h("https://api.github.com/user", auth);
if resp.ok() {
    me = User.parse(resp.body);
}

// Parse error handling
resp = http_get("https://api.example.com/data");
if resp.ok() {
    data = MyStruct.parse(resp.body);
    if len(data#errors) > 0 {
        for e in data#errors {
            log_warn("parse issue: {e}");
        }
    }
}
```

### Parallel-safe

`HttpResponse` is a plain struct — no global state.  `http_status()` is NOT provided
because it would be thread-unsafe with parallel workers.

### Implementation plan

| Step | Description | Effort | Dependencies |
|------|-------------|--------|-------------|
| 1 | Add `ureq` dependency (feature-gated `http`) | Small | Cargo.toml |
| 2 | `HttpResponse` struct + `ok()` method in `default/06_web.loft` | Small | — |
| 3 | Native functions in `src/native_http.rs` via `#rust` | Medium | ureq |
| 4 | User documentation page `tests/docs/NN-web-services.loft` | Small | Steps 2–3 |
| 5 | Integration tests in `tests/scripts/` | Small | Steps 2–3 |

### Error handling conventions

- Network error → `HttpResponse { status: 0, body: "" }`
- DNS failure → `HttpResponse { status: 0, body: "" }`
- Timeout → `HttpResponse { status: 0, body: "" }`
- Non-2xx response → status code set, body contains server response
- Never panics

### Cargo feature

```toml
[features]
http = ["dep:ureq"]

[dependencies]
ureq = { version = "2", optional = true }
```

When `http` is not enabled, the `http_*` functions are not registered and produce
a compile error if called.

---

## Comparison with original approaches

The original design (2026-03-18) evaluated four approaches.  With `Type.parse()` now
implemented, the comparison simplifies:

| | Original Approach B | Current design |
|---|---|---|
| JSON deserialization | `#json` + synthesized `from_json` | `Type.parse()` — already works |
| JSON serialization | `#json` + synthesized `to_json` | `"{value:j}"` — already works |
| Error handling | `from_json` returns null on failure | `Type.parse()` + `#errors` |
| Nested structs | Phase 3 of `#json` | `Type.parse()` handles nesting |
| Arrays | `json_items()` + `map(T.from_json)` | `vector<T>.parse()` |
| Annotation needed | `#json` on every struct | None |
| Implementation effort | Medium (H1–H5) | Small (HTTP only) |

The `#json` annotation, `json_text()`, `json_int()`, and `json_items()` functions
from the original design are **no longer needed**.  `Type.parse()` replaces all of them.

---

## See also

- [STDLIB.md](STDLIB.md) — Standard library reference
- [LOFT.md](LOFT.md) — Language reference (Type.parse, format strings)
- [PLANNING.md](PLANNING.md) — H-tier items in the backlog
- [ROADMAP.md](ROADMAP.md) — 0.8.4 milestone
