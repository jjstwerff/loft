<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — Implementation steps (full designs)

Small, independently verifiable steps for [README.md](README.md).  Each step is
one focused change with a concrete verify command; nothing in a later step is
needed to land an earlier one.  All code lives in **`lib/web`** (the shipped
HTTP library) — **no change to the loft core** is required, because every native
signature below already has an arm in the `src/extensions.rs` auto-marshal
dispatcher (verified 2026-05-26):

| Native signature | Dispatcher arm | Used by |
|---|---|---|
| `() -> text` | `(&[], Some(Text))` | `http_headers_raw` (exists for `http_body`) |
| `(text) -> text` | `(&[Text], Some(Text))` | `ascii_lower`, `base64_*` |
| `(integer) -> integer` | `(&[I32], Some(I32))` | `http_session_new` |
| `(integer)` | `(&[I32], None)` | `http_session_use` |
| `(text,text,text,text) -> integer` | `(&[Text×4], Some(I32))` | `http_do` (exists, reused) |

**Ground facts (ureq 2.12.1, verified):** `Response::header(name)->Option<&str>`
(first, case-insensitive), `Response::all(name)->Vec<&str>` (every value — needed
for repeated `Set-Cookie`), `Response::headers_names()->Vec<String>`;
`AgentBuilder::redirects(u32)` (0 = don't follow); with the `cookies` feature an
`AgentBuilder::new().build()` carries a default cookie jar (agent.rs:299);
`Agent::request(method,path)` + `ureq::request(method,path)` both exist; `Agent`
is `Clone` and Arc-backed (clones share the jar + pool).  `text.find(v)` returns
`i64::MIN` on miss (so `>= 0` is the found-guard); `split(sep: character)`,
`vector += [x]`, and `s[a..b]` slicing are the loft idioms (no `lower()` /
`substring()` exists — hence the `ascii_lower` native).

Build/run after each step:

```bash
# rebuild the web cdylib (loft loads it next to the package)
(cd lib/web/native && cargo build --release)
# run a loft test program against the freshly built library
cargo run --release --bin loft -- --lib lib lib/web/tests/http.loft
```

`loft-ffi-build` source-scans `web.loft` for `#native` declarations to generate
the register list, so a new `#native` fn + its exported symbol + the cdylib
rebuild is the entire wiring (no hand-edited manifest).

---

## P1 — response headers

### Step P1.1 — capture response headers natively (Rust only, no loft surface yet)

**Goal:** every HTTP response stores its headers in a thread-local, name-lowercased
and newline-joined, duplicates preserved.  Pure addition; nothing reads it yet.

**File:** `lib/web/native/src/lib.rs`

```rust
// add beside LAST_BODY
thread_local! {
    static LAST_BODY: RefCell<String> = const { RefCell::new(String::new()) };
    static LAST_HEADERS: RefCell<String> = const { RefCell::new(String::new()) };
}

// Format a response's headers as newline-joined "name: value" lines.
// Names are lowercased (HTTP header names are case-insensitive) so the loft
// side can match with `==` and no `lower()`.  Duplicates (e.g. multiple
// Set-Cookie) appear as separate lines, in header order.  Call BEFORE
// into_string() — it borrows the response.
fn capture_headers(resp: &ureq::Response) -> String {
    let mut out = String::new();
    for name in resp.headers_names() {
        let lname = name.to_ascii_lowercase();
        for val in resp.all(&name) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&lname);
            out.push_str(": ");
            out.push_str(val);
        }
    }
    out
}
```

Change `do_request` to return the captured headers as a third tuple element:

```rust
fn do_request(
    agent: Option<&ureq::Agent>,   // None today; the session step (P2.1) passes Some
    method: &str,
    url: &str,
    body: Option<&str>,
    headers: &[(&str, &str)],
) -> (i32, String, String) {       // (status, body, headers)
    let mut req = match agent {
        Some(a) => a.request(method, url),
        None => ureq::request(method, url),
    };
    for (k, v) in headers {
        req = req.set(k, v);
    }
    let response = if let Some(b) = body { req.send_string(b) } else { req.call() };
    match response {
        Ok(resp) => {
            let status = resp.status() as i32;
            let hdrs = capture_headers(&resp);
            (status, resp.into_string().unwrap_or_default(), hdrs)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let hdrs = capture_headers(&resp);   // 4xx/5xx still have headers
            (code as i32, resp.into_string().unwrap_or_default(), hdrs)
        }
        Err(_) => (0, String::new(), String::new()),
    }
}
```

Update the existing `n_http_do` body to thread the new arg + store headers
(stateless path passes `None`):

```rust
let (status, response_body, response_headers) = do_request(None, method, url, body, &headers);
LAST_BODY.with(|b| *b.borrow_mut() = response_body);
LAST_HEADERS.with(|h| *h.borrow_mut() = response_headers);
status
```

**Verify:** `(cd lib/web/native && cargo build --release)` compiles clean.
**Done when:** the crate builds; no behaviour change yet (`http.loft` still passes).

---

### Step P1.2 — expose headers to loft (`headers` field + `header` / `headers_for`)

**Goal:** `HttpResponse` carries a `headers: vector<text>` snapshot; two accessors
read it.  Snapshot-safe — the accessors read the struct field, never re-query the
thread-local (so a stored response keeps its own headers after later requests).

**File A:** `lib/web/native/src/lib.rs` — two tiny natives:

```rust
/// Return the last response's headers, newline-joined "name: value".
#[unsafe(no_mangle)]
pub extern "C" fn n_http_headers_raw() -> LoftStr {
    LAST_HEADERS.with(|h| loft_ffi::ret_ref(&h.borrow()))
}

/// ASCII-lowercase a string.  Generic helper — loft has no `lower()`; lets the
/// header accessors normalise a caller-supplied name.  (Cleaner long-term home
/// is a core `text.lower()`; kept local to avoid core scope-creep — see README
/// open question.)
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_ascii_lower(ptr: *const u8, len: usize) -> LoftStr {
    let s = unsafe { loft_ffi::text(ptr, len) };
    loft_ffi::ret(s.to_ascii_lowercase())
}
```

**File B:** `lib/web/src/web.loft`

```loft
pub struct HttpResponse {
  status: integer,
  body: text,
  headers: vector<text>      // NEW: each entry "name: value" (name lowercased);
                             //      repeated headers (multiple Set-Cookie) are
                             //      separate entries
}

fn http_headers_raw() -> text;
#native

fn ascii_lower(s: text) -> text;
#native     // bare → symbol defaults to n_ascii_lower

// Split the newline-joined header blob into one "name: value" entry per line.
fn split_headers(raw: text) -> vector<text> {
  if raw.len() == 0 { return []; }
  raw.split('\n')
}

// First value of a header (case-insensitive name); "" when absent.
pub fn header(self: HttpResponse, name: text) -> text {
  q = ascii_lower(name);
  for h in self.headers {
    i = h.find(": ");
    if i >= 0 && h[0..i] == q { return h[i + 2..h.len()]; }
  }
  ""
}

// All values for a header (e.g. every Set-Cookie); empty vector when absent.
pub fn headers_for(self: HttpResponse, name: text) -> vector<text> {
  q = ascii_lower(name);
  out: vector<text> = [];
  for h in self.headers {
    i = h.find(": ");
    if i >= 0 && h[0..i] == q { out += [h[i + 2..h.len()]]; }
  }
  out
}
```

Then add `headers: split_headers(http_headers_raw())` to **all eight**
constructors (`http_get`/`post`/`put`/`delete` + the four `_h` variants), e.g.:

```loft
pub fn http_get(url: text) -> HttpResponse {
  status = http_do("GET", url, "", "");
  HttpResponse { status: status, body: http_body(), headers: split_headers(http_headers_raw()) }
}
```

**Note — call order:** `http_body()` then `http_headers_raw()` both read
thread-locals set by the immediately preceding `http_do`; struct fields evaluate
left-to-right, so `body` then `headers` is safe.  (If a future change makes
field-eval order uncertain, hoist into locals first.)

**Verify (network):** add to `lib/web/tests/http.loft`:

```loft
fn test_response_headers() {
  r = web::http_get("https://httpbin.org/response-headers?X-Test=hi");
  assert(r.status == 200, "status 200, got {r.status}");
  assert(len(r.headers) > 0, "headers populated");
  assert(r.header("x-test") == "hi", "x-test header = hi, got '{r.header("x-test")}'");
  assert(r.header("X-Test") == "hi", "case-insensitive name lookup");
  assert(r.header("absent") == "", "absent header is empty");
}
```

**Done when:** `test_response_headers` passes; the existing `.status`/`.body`
tests still pass (field is additive).  **This step alone unblocks Garmin A2** —
the caller reads `Set-Cookie` and re-sends it via the existing `headers` vector.

---

## P2 — cookie-jar `HttpSession`

### Step P2.1 — agent registry + session natives (Rust only)

**Goal:** a thread-local registry of cookie-aware `ureq::Agent`s, selected one-shot
before a request, with `n_http_do` routing through the selected agent.

**File A:** `lib/web/native/Cargo.toml` — enable the cookie jar:

```toml
ureq = { version = "2", features = ["cookies"] }
```

**File B:** `lib/web/native/src/lib.rs`

```rust
use std::cell::Cell;

thread_local! {
    // session handle -> Agent; index is the handle. Agents are Arc-backed, so a
    // clone shares the jar + connection pool.
    static AGENTS: RefCell<Vec<Option<ureq::Agent>>> = const { RefCell::new(Vec::new()) };
    // agent selected for the NEXT http_do; -1 = stateless. One-shot: http_do
    // resets it to -1 after reading, so a stray stateless call never inherits
    // a session.
    static ACTIVE_AGENT: Cell<i32> = const { Cell::new(-1) };
}

/// Create a cookie-jar session. `redirects` = max redirects to follow
/// (0 = don't follow, so the caller reads Location). Returns the handle.
#[unsafe(no_mangle)]
pub extern "C" fn n_http_session_new(redirects: i32) -> i32 {
    let agent = ureq::AgentBuilder::new()
        .redirects(redirects.max(0) as u32)
        .build();
    AGENTS.with(|a| {
        let mut v = a.borrow_mut();
        v.push(Some(agent));
        (v.len() - 1) as i32
    })
}

/// Select the session to use for the next http_do call.
#[unsafe(no_mangle)]
pub extern "C" fn n_http_session_use(handle: i32) {
    ACTIVE_AGENT.with(|c| c.set(handle));
}
```

Route `n_http_do` through the selected agent (replaces the P1.1 body):

```rust
// take-and-reset the one-shot selection, then clone the agent out so we don't
// hold the registry borrow across the blocking request.
let active = ACTIVE_AGENT.with(|c| c.replace(-1));
let agent = if active >= 0 {
    AGENTS.with(|a| a.borrow().get(active as usize).and_then(|o| o.clone()))
} else {
    None
};
let (status, response_body, response_headers) =
    do_request(agent.as_ref(), method, url, body, &headers);
LAST_BODY.with(|b| *b.borrow_mut() = response_body);
LAST_HEADERS.with(|h| *h.borrow_mut() = response_headers);
status
```

**Verify:** `(cd lib/web/native && cargo build --release)` builds clean with the
`cookies` feature; stateless `http.loft` tests still pass (active == -1 path).
**Done when:** crate builds; no behaviour change for stateless calls.

---

### Step P2.2 — `HttpSession` loft API (struct + verbs)

**Goal:** the clean consumer surface — a session whose `get`/`post`/`put`/`delete`
carry cookies automatically.

**File:** `lib/web/src/web.loft`

```loft
// An HTTP client that persists cookies across calls on this handle.
pub struct HttpSession {
  id: integer not null
}

fn http_session_new_native(redirects: integer) -> integer;
#native "n_http_session_new"

fn http_session_use_native(handle: integer);
#native "n_http_session_use"

// Open a cookie-jar session. follow_redirects=false lets the caller read
// Location on a 3xx and drive the redirect itself (the jar is still carried).
pub fn http_session(follow_redirects: boolean) -> HttpSession {
  r = if follow_redirects { 10 } else { 0 };
  HttpSession { id: http_session_new_native(r) }
}

pub fn get(self: HttpSession, url: text, headers: vector<text>) -> HttpResponse {
  http_session_use_native(self.id);
  status = http_do("GET", url, "", join_headers(headers));
  HttpResponse { status: status, body: http_body(), headers: split_headers(http_headers_raw()) }
}

pub fn post(self: HttpSession, url: text, body: text, headers: vector<text>) -> HttpResponse {
  http_session_use_native(self.id);
  status = http_do("POST", url, body, join_headers(headers));
  HttpResponse { status: status, body: http_body(), headers: split_headers(http_headers_raw()) }
}

pub fn put(self: HttpSession, url: text, body: text, headers: vector<text>) -> HttpResponse {
  http_session_use_native(self.id);
  status = http_do("PUT", url, body, join_headers(headers));
  HttpResponse { status: status, body: http_body(), headers: split_headers(http_headers_raw()) }
}

pub fn delete(self: HttpSession, url: text, headers: vector<text>) -> HttpResponse {
  http_session_use_native(self.id);
  status = http_do("DELETE", url, "", join_headers(headers));
  HttpResponse { status: status, body: http_body(), headers: split_headers(http_headers_raw()) }
}
```

**Why "select then call" is race-free:** `http_session_use_native(self.id)` and
`http_do` are consecutive synchronous native calls on one thread; nothing runs
between them, and `http_do` consumes-and-resets `ACTIVE_AGENT`, so each verb
re-selects.  (`lib/web` HTTP is blocking/single-threaded — same model as the
existing `LAST_BODY` thread-local.)

**Verify (network):** add to `lib/web/tests/http.loft`:

```loft
fn test_session_cookie_roundtrip() {
  s = web::http_session(true);                                  // follow redirects
  s.get("https://httpbin.org/cookies/set?foo=bar", []);         // sets cookie, 302 -> /cookies
  r = s.get("https://httpbin.org/cookies", []);                 // jar carried
  assert(r.status == 200, "cookies status 200, got {r.status}");
  assert(r.body.contains("foo"), "cookie 'foo' echoed back: {r.body}");
}

fn test_session_redirect_location() {
  s = web::http_session(false);                                 // do NOT follow
  r = s.get("https://httpbin.org/redirect-to?url=/get", []);
  assert(r.status >= 300 && r.status < 400, "3xx, got {r.status}");
  assert(r.header("location") != "", "Location header readable");
}
```

**Done when:** both tests pass — the cookie set by the first `get` is sent on the
second (round-trip), and a non-following session exposes `Location`.  This is the
full native Garmin login path (credential POST → MFA POST on the same jar →
ticket 302).

---

## P3 — base64 helpers (optional)

### Step P3.1 — `base64_encode` / `base64_decode` in `lib/web`

**Goal:** the small encoding helper A2's OAuth/SSO header construction wants.
Optional — the training port has a pure-loft base64 today; land only if convenient.

**File A:** `lib/web/native/Cargo.toml`

```toml
base64 = "0.22"
```

**File B:** `lib/web/native/src/lib.rs`

```rust
use base64::Engine;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_base64_encode(ptr: *const u8, len: usize) -> LoftStr {
    let s = unsafe { loft_ffi::text(ptr, len) };
    loft_ffi::ret(base64::engine::general_purpose::STANDARD.encode(s.as_bytes()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_base64_decode(ptr: *const u8, len: usize) -> LoftStr {
    let s = unsafe { loft_ffi::text(ptr, len) };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .unwrap_or_default();
    loft_ffi::ret(String::from_utf8_lossy(&bytes).into_owned())
}
```

**File C:** `lib/web/src/web.loft`

```loft
pub fn base64_encode(s: text) -> text;
#native
pub fn base64_decode(s: text) -> text;
#native
```

**Verify (no network):**

```loft
fn test_base64() {
  assert(web::base64_encode("hello") == "aGVsbG8=", "rfc vector encode");
  assert(web::base64_decode("aGVsbG8=") == "hello", "rfc vector decode");
}
```

**Done when:** the RFC test vector round-trips.  `sha256` (via `sha2`) is a
follow-on if a digest is needed; same `(text) -> text` shape (hex out).

---

## Landing order + commits

1. **P1.1 + P1.2** in one commit ("lib/web: expose response headers") — XS–S,
   additive, unblocks A2 minimally.  Training verifies native login with manual
   cookie handling.
2. **P2.1 + P2.2** in one commit ("lib/web: cookie-jar HttpSession") — promotes
   manual cookies to an automatic jar.  Depends on P1 (reuses `http_headers_raw`).
3. **P3.1** opportunistically, separate commit.

Each commit: rebuild the cdylib, run `lib/web/tests/http.loft`, and (per the
testing skill) add the new `test_*` functions to that file as the regression
guard.  The training port then runs its end-to-end Garmin login and reports back.
