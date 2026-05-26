<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# HTTP client — planned

HTTP client design for loft.  Implemented via `ureq` (small
blocking HTTP library, feature-gated).  No builder chains, no
service blocks, no new syntax — HTTP calls are regular stdlib
functions that return a struct.

**Status:** planned.  Deferred to 1.1+ per ROADMAP H4.  This
document is the locked-in design once the H tier opens.

## Design — `HttpResponse` struct + plain functions

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

## Usage patterns

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

## Parallel-safe

`HttpResponse` is a plain struct — no global state.
`http_status()` is NOT provided because it would be thread-
unsafe with parallel workers.

## Implementation plan

| Step | Description | Effort | Dependencies |
|------|-------------|--------|-------------|
| 1 | Add `ureq` dependency (feature-gated `http`) | Small | Cargo.toml |
| 2 | `HttpResponse` struct + `ok()` method in `default/06_web.loft` | Small | — |
| 3 | Native functions in `src/native_http.rs` via `#rust` | Medium | ureq |
| 4 | User documentation page `tests/docs/NN-web-services.loft` | Small | Steps 2–3 |
| 5 | Integration tests in `tests/scripts/` | Small | Steps 2–3 |

## Error handling conventions

- Network error → `HttpResponse { status: 0, body: "" }`
- DNS failure → `HttpResponse { status: 0, body: "" }`
- Timeout → `HttpResponse { status: 0, body: "" }`
- Non-2xx response → status code set, body contains server response
- Never panics

## Cargo feature

```toml
[features]
http = ["dep:ureq"]

[dependencies]
ureq = { version = "2", optional = true }
```

When `http` is not enabled, the `http_*` functions are not
registered and produce a compile error if called.

---

## Comparison with original approaches

The original design (2026-03-18) evaluated four approaches.
With `Type.parse()` now implemented (see [JSON.md](JSON.md)),
the comparison simplifies:

| | Original Approach B | Current design |
|---|---|---|
| JSON deserialization | `#json` + synthesized `from_json` | `Type.parse()` — already works |
| JSON serialization | `#json` + synthesized `to_json` | `"{value:j}"` — already works |
| Error handling | `from_json` returns null on failure | `Type.parse()` + `#errors` |
| Nested structs | Phase 3 of `#json` | `Type.parse()` handles nesting |
| Arrays | `json_items()` + `map(T.from_json)` | `vector<T>.parse()` |
| Annotation needed | `#json` on every struct | None |
| Implementation effort | Medium (H1–H5) | Small (HTTP only) |

The `#json` annotation, `json_text()`, `json_int()`, and
`json_items()` functions from the original design are **no
longer needed**.  `Type.parse()` replaces all of them.

---

## Open work

> **Note:** the design body above predates the shipped library.  HTTP client
> shipped as **`lib/web`** (cdylib + `ureq 2.12`, body via a `LAST_BODY`
> thread-local; see `lib/web/src/web.loft` + `lib/web/native/src/lib.rs`), not
> `default/06_web.loft` / `src/native_http.rs`.

The next consumer-driven enhancement — **response headers + a cookie-jar
session** (to unblock native Garmin login A2 in the training port) — has its own
phased plan: **[`../23-http-session-auth/`](../23-http-session-auth/)**.  Related
items (base64 stdlib helper, the moot `exec()` gap, the deferred TLS/JA3
impersonation E1) are tracked there too.

## See also

- [README.md](README.md) — overview of the web-services library plan
- [JSON.md](JSON.md) — currently-shipped JSON capabilities (the
  serialization layer this client builds on)
- [../../../PLANNING.md](../../../PLANNING.md) — H-tier items in the backlog
- [../../../ROADMAP.md](../../../ROADMAP.md) — milestone placement (H4 → 1.1+)
