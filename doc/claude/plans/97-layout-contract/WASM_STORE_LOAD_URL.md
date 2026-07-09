<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `store_load_url_trusted` in the browser (`--html`) — design + verifiable steps

> **Part of @PLN97 arc G** (remote store loader). **Requirement (user):** the
> **same internal API** on every target — `store_load_url_trusted(r, url) ->
> boolean` is one synchronous call; only the fetch underneath differs. **Status:
> Rust core DONE + compile-verified on native and `wasm32-unknown-unknown`; JS
> shell + browser validation remain (this doc).**

## The invariant

> A `--html` program's `store_load_url_trusted(r, url)` fetches the image over the
> browser's async `fetch()` and returns the SAME boolean, with the SAME structural
> validation and heap-load, as native — WITHOUT blocking the event loop. It stays
> a synchronous loft call; the async→sync bridge is **asyncify** (the stack
> unwinds to the event loop during the fetch, then rewinds with the bytes).

## Done (Rust core — the same-API layer)

- `lib.rs`: `loft_io.loft_host_http_get(url_ptr,url_len)->usize` + `_copy(ptr)`
  imports (browser cfg).
- `net::fetch_bytes(url)`: browser → the host import; native → `ureq`; gated to
  exist exactly where `load_url` does.
- `Stores::load_url` un-gated (`registry` OR browser) + routed through
  `net::fetch_bytes`; `n_store_load_url_trusted` un-gated + registered for browser.
- `main.rs`: `loft_io.loft_host_http_get` added to the `--asyncify` allowlist.
- **Verify (already passing):** `cargo check --lib` **and**
  `cargo check --lib --target wasm32-unknown-unknown --no-default-features
  --features random` both exit 0.

## The load-bearing JS mechanism (why the steps are shaped this way)

`AsyncifyCtrl` (`doc/loft-gl-wasm.js`) exposes `start`/`resume`/`sleeping` and a
`suspend()` an import calls. A value-returning suspend import is invoked **TWICE**:

1. **unwind call** (`asyncify_get_state() !== REWINDING`): kick off the async op
   (`fetch(url).then(… ac.resume('loft_start'))`), then `ac.suspend()` →
   `asyncify_start_unwind` unwinds the whole stack back to JS. The return value is
   ignored (the module is unwinding).
2. **rewind call** (state `REWINDING`, after `resume`): the fetch has completed and
   its result is stashed in JS; `ac.suspend()` here does `asyncify_stop_rewind` and
   the import **returns the real value** (the byte length, or `usize::MAX` on
   error). Execution continues past the call.

`_copy` is a plain (non-suspending) import that memcpys the stashed bytes. This is
exactly the `loft_web.ws_yield` pattern, extended to carry a return value +
payload.

## Steps (each with its verification)

### Step 1 — the fetch imports in the JS shell

Add to the `loft_io` imports object (BOTH templates — see Step 2 for the headless
one): `loft_host_http_get` (the double-call suspend import above) and
`loft_host_http_get_copy` (memcpy `ctrl.httpBytes` into `mem` at `ptr`). Stash
`ctrl.httpBytes` (a `Uint8Array` or `null` on error) between the two calls.

- **Verify V1a (generated source):** `LOFT_KEEP_NATIVE_RS=1 loft --html
  tests/fixtures/<store-load>.loft` (or read the emitted page) contains
  `loft_host_http_get` in the imports and a `fetch(` call. `grep` the generated
  html.
- **Verify V1b (unit-level, headless JS):** a tiny node harness that instantiates
  the wasm with a mock `loft_host_http_get` returning fixed bytes and asserts
  `store_load_url_trusted` returns `true` and the record reads back — proves the
  double-call contract independent of a real network.

### Step 2 — make the headless `--html` template asyncify-capable

`AsyncifyCtrl` currently lives in `{gl_js}` (GL template only); the headless
template runs `loft_start()` synchronously. Extract `AsyncifyCtrl` (and
`decodeLoftAssets` if needed) into a **shared** `const asyncify_js` string
(`include_str!` a new small `doc/loft-asyncify.js`), include it in BOTH templates,
and give the headless template the same drive loop the GL one has
(`if(exports.asyncify_start_unwind){ac=new AsyncifyCtrl(...); ac.start('loft_start');
if(ac.sleeping) pump via MessageChannel}`).

- **Verify V2a (compile/gen):** a headless `--html` build of a program that does
  NOT fetch still runs `loft_start` and prints (no regression) — the existing
  `html_wasm` / `exit_codes` `--html` tests stay green.
- **Verify V2b (structural):** the emitted headless page contains
  `new AsyncifyCtrl` and the `resume('loft_start')` pump loop (grep the html).

### Step 3 — the JS-side `fetch` completion + error mapping

On the unwind call, `fetch(url)`: on `resp.ok` stash
`new Uint8Array(await resp.arrayBuffer())`; on `!ok` or a thrown error stash
`null`; either way call `ac.resume('loft_start')`. The rewind call returns
`httpBytes ? httpBytes.length : 0xFFFFFFFF` — so `net::fetch_bytes` maps the
sentinel to `Err(...)` → `load_url` returns `false` (the same failure contract as
native).

- **Verify V3 (error parity):** in the node harness (V1b), a mock that rejects →
  `store_load_url_trusted` returns `false`; a mock returning a corrupt image →
  `false` (structural validation rejects it, same as native). No panic, no hang.

### Step 4 — end-to-end browser validation (the real green)

A headless-Chrome test in the `tests/html_wasm.rs` harness: serve a real `.store`
image over `http://127.0.0.1:<port>/world.store` (the existing test HTTP server),
run a `--html` program that calls `store_load_url_trusted(w, url)` then reads a
known field, and assert the page prints the expected value. This exercises the
actual asyncify unwind/rewind + `fetch()` round-trip.

- **Verify V4 (browser):** the new `html_wasm` test passes headless
  (Chrome via the existing driver); the page neither freezes (asyncify works) nor
  errors. This is the definitive "functions in wasm" proof.

### Step 5 — cross-target parity (the same-API guarantee, tested)

Extend an `n3_parity`-style check: the SAME loft program calling
`store_load_url_trusted` yields byte-identical observable output under **interpret,
`--native`, and `--html`** (the `--html` leg via the Step-4 harness). This is the
executable form of the "same internal API" requirement.

- **Verify V5:** all three modes print the same result; a divergence fails the
  test. (Native/interpret already work; this pins them to the new `--html` leg.)

## Acceptance

Steps 1–3 verified locally (grep + node harness), Step 4 green in headless Chrome,
Step 5 three-mode parity green. Then `store_load_url_trusted` "functions in wasm"
with the identical synchronous API — no page freeze, same validation, same failure
contract.

## Verified so far (2026-07-09) — `harness/run.sh`

Steps 1–3 + Step 5 are GREEN via the repeatable `harness/` (a `.store` image, the
`--html` wasm of a URL-loading program, and a node driver that mocks
`loft_host_http_get`):

- **V1a/V2b** — the emitted page carries the imports + the asyncify driver (7×
  `loft_host_http_get`, the `AsyncifyCtrl` def, the minimal-page driver, `fetch(url)`).
- **V1b** — driving the REAL 355 KB wasm through `AsyncifyCtrl` with a mock fetch
  returning the image → `url keys=7,13,42` (the double-call unwind/rewind contract
  works; bytes flow through `net::fetch_bytes` → `load_url` → validate → adopt).
- **V3** — mock fetch error (`null` → `0xFFFFFFFF`) → `false`; corrupt image →
  `false` (fail-closed, same as native).
- **V5 (parity)** — the SAME program run natively via `file://` → `url keys=7,13,42`,
  byte-identical to the wasm output.

**Remaining: Step 4** — the real `fetch()` round-trip in headless Chrome (the
harness mocks the network; only a browser exercises the actual `fetch()`).

## Residual / notes

- **WASI (`--native-wasm`, `wasm32-wasip2`) is separate:** it has no `fetch()`; a
  follow-on could bridge `wasi:http` (or leave it as the documented
  "unavailable" error). The browser (`--html`) is what this doc covers.
- **`store_load_url` (SHA-verified) stays native-only** until `verify_sha256`
  moves out of the `registry`-gated module — a small follow-on; the *trusted*
  variant (the ask) needs no SHA.
- **Binary over the host bridge:** the fetch payload is raw bytes via `_copy`
  (an `arrayBuffer`), NOT the text `host_input` queue — so no UTF-8 round-trip
  corruption (the same reason the loaders fetch in Rust, not through `web`'s text
  body).
