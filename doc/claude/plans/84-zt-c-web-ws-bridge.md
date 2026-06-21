<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN84 ZT-C — `web` WebSocket wasm bridge: detailed design

Status: **implemented + verified (2026-06-21)** — C2 + C3 GREEN on both backends
(`native == wasm`); see § 11 for the resolved R1–R5. The `web` lib changes are staged
in `loft/tools/zt-c-web-staging/` (the lib lives in a separate repo) and applied to the
live lib; the loft-side changes (`src/main.rs` asyncify `--pass-arg`, `tools/wasm_ws_repro.mjs`
driver, `tools/zt-c-web-staging/` probe + echo server) are in this worktree. ·
Parent: [`84-zero-trust-libs.md`](84-zero-trust-libs.md) § ZT-C · Home library:
`loft-libs-net/web` · Compiler touch-points: `loft/src/main.rs` (--html asyncify),
`loft/tools/` (the headless WS driver).

> One WS-client loft source must run unchanged as the **native sync-agent** and the
> **browser client** — `native == wasm`. Unlike ZT-B crypto (pure compute), the WS
> client is **stateful and event-driven**, so this bridge is the async↔sync case.
> This doc is the de-risked plan: the load-bearing decision (asyncify frame-yield),
> the host-import ABI, the host.js + Rust shapes, the CBOR-frame path, and the
> verify harness that must exist *before* the bridge is trustworthy.

---

## 1. Acceptance (the gates, from @PLN84)

- **C1 — keep the polling WS-client API** (connect, non-blocking `recv`, `send`/
  `send_binary`, `close`). *Already shipped in the lib; no API change.*
- **C2 — `[wasm.bridge]` → browser `WebSocket`.** *Check:* ONE client `.loft` source
  runs unchanged on both backends (native sync-agent + browser client) — a text
  message echoes back and is received.
- **C3 — CBOR binary frames (opcode 2) round-trip** both backends. *Check:* a CBOR
  frame sent by a native peer is received **byte-identical** by the wasm client and
  vice versa.

A phase is done only when its check passes on BOTH `--interpret`/native AND the wasm
backend (the master invariant). Same discipline as ZT-B: prove the runtime, both
backends, before claiming done.

---

## 2. Background — two bridge mechanisms; WS needs host imports

`[wasm.bridge]` offers two paths (both used by crypto):

1. **Routed `pub fn`** (`[wasm.bridge.routes]`): codegen replaces the `#native` body
   with a direct call into the bridge crate (`output_wasm_bridge_call`,
   `loft/src/generation/mod.rs:3616`). Pure compute only — runs to completion
   synchronously inside the wasm.
2. **Host import**: the bridge crate declares `#[link(wasm_import_module="…")]
   unsafe extern "C" { fn f(…); }` and `host.js` provides it (the crypto
   `random_fill` pattern).

**WebSocket I/O touches a live JS `WebSocket` object, so every `n_ws_*` symbol must
bridge via HOST IMPORTS (mechanism 2), under import module `loft_web`.** The
`[wasm.bridge.routes]` table stays empty for WS (no routed pure-compute fns); we keep
`[wasm.bridge] crate=… host_js=…` so the driver concatenates `host.js` and links the
bridge crate.

**Pre-existing stub to reconcile:** `web/native/src/ws_client.rs` already has a
`wasm_impl` module (~`:615-680`) with 4 imports defaulting to module `"env"`,
text-only send, and a hardcoded `last_opcode()==1`. It is incompatible with this
design (wrong module, no binary opcode, no length channel). **It is rewritten**, not
extended; `pub use wasm_impl::{…}` (`ws_client.rs:~685`) keeps `lib.rs` unchanged.

---

## 3. The native polling contract (what the bridge must replicate)

From `web/native/src/ws_client.rs` + `web/src/web.loft`:

- **Handle** = a plain `integer` (i32) slot index into a `thread_local CONNS:
  Vec<Option<Conn>>`; `connect(url)` returns the index, `-1` on malformed URL.
- **`recv(handle) -> bool` is poll-and-return-immediately** (native uses a 7 ms read
  timeout): `true` ⇒ a frame landed and its payload/opcode are latched into
  thread-local `LAST_MSG`/`LAST_OP`; `false` ⇒ nothing-yet OR connection down (the
  two are not distinguished — the caller just retries).
- **`message()` / `opcode()` take NO handle** — they read the single-slot
  `LAST_MSG`/`LAST_OP` register set by the most recent `recv`. Contract: call
  `recv(h)`, then read `message()`/`opcode()` *before any other recv*.
- **Opcodes:** `TEXT=1`, `BINARY=2`, `CLOSE=8`, `PING=9`, `PONG=10` (PING auto-PONGs
  and returns `false`; CLOSE marks disconnected).
- **The loft loop** (`web.loft` `pump`): `while ws_client_recv_native(id) {
  on_message(ws_client_message_native()) }`.
- **`ws_group_*`** multiplexes: round-robin `poll_group` over a `thread_local
  WS_GROUP: Vec<i32>`, returns the first ready handle or `-1`.

The browser bridge replicates exactly this: a synchronous `ws_poll(h) -> has_msg`
that drains a JS inbound queue, plus a latched current-frame register read.

---

## 4. THE load-bearing decision — async↔sync via the asyncify frame-yield

**The invariant this protects:** *a synchronous poll loop in `--html` must let the
JS event loop run between iterations, or no WebSocket event ever fires.*

### 4.1 Why a purely synchronous bridge DEADLOCKS

loft host imports are synchronous and loft cannot yield to the JS event loop
mid-execution (`WASM.md:1183`). The browser `WebSocket`'s `onopen`/`onmessage`/
`onclose` are queued macrotasks that run **only when wasm returns control to JS**. So:

```loft
h = web::ws_handler("ws://…")
while true { ev = h.try_recv(); … }    // never returns to JS
```

spins forever inside one synchronous `loft_start()`. `onmessage` never fires,
`inbound` stays empty, `ws_poll` always returns 0 — the page locks up (the `@P337`
render-loop failure). **A pure-synchronous WS bridge cannot work.**

### 4.2 The fix — reuse the existing asyncify frame-yield

The `--html` driver already ships an asyncify suspend/resume loop for GL:

- `wasm-opt --asyncify --pass-arg=asyncify-imports@loft_gl.loft_gl_swap_buffers`
  (`src/main.rs:~5461`) registers a suspend point.
- The suspend import calls `asyncify_start_unwind` → unwinds the wasm stack back to
  JS (`doc/loft-gl-wasm.js:~77`).
- The driver, on seeing the `asyncify_start_unwind` export, drives
  `requestAnimationFrame(frame)` calling `ac.resume('loft_start')` (`main.rs:~5635`).
  Between unwind and the next rAF, the JS event loop runs — that is when
  `WebSocket.onmessage` fires and fills the inbound queue.

So the WS client calls a **`yield_frame()` once per poll iteration**; under `--html`
it maps to an asyncified suspend point, returns to the event loop, lets WS events
deliver, then resumes. Native `yield_frame()` is a no-op (or `sleep_ms(0)`).

```loft
h = web::ws_handler("ws://…")
while running {
  while h.try_recv() { handle(web::ws_client_message_native()) }
  web::yield_frame()     // --html: asyncify suspend -> event loop -> resume; native: no-op
}
```

### 4.3 Decision — which suspend point

| Option | Cost | Verdict |
|---|---|---|
| **(1) Reuse `loft_gl_swap_buffers` as the yield point** | `web::yield_frame()` (native no-op) maps under `--html` to the already-asyncified `loft_gl_swap_buffers` import; **no change to `src/main.rs`** | **Recommended first cut** — proves the model with zero compiler change. Caveat: a headless WS client has no GL window, so validate that `loft_gl_swap_buffers`'s JS shim is safe to call with no window (it may need a no-op-when-headless guard in `loft-gl-wasm.js`). |
| **(2) Dedicated `loft_web.ws_yield` suspend import** | one line at `main.rs:~5461` (add `,loft_web.ws_yield` to `--pass-arg`) + host.js suspend wiring | Cleaner semantically; do it once (1) proves the model, if the GL-coupling is awkward. |

**Open risk R1:** does asyncify resume work in `--html` with NO GL window (headless
WS)? The rAF loop + `asyncify_start_unwind` export exist regardless of GL, but the
GL swap shim assumes a context. Probe this FIRST (§9 P1). If GL-coupling blocks
headless, go straight to option (2).

---

## 5. The host-import ABI (module `loft_web`)

Host imports CAN return values (loft_gl handles do: `loft-gl-wasm.js:206` etc.), and
bytes cross via the ptr/len ABI over `getMem().buffer` (always re-fetch — memory can
grow/detach). Proposed set (poll → len → copy → opcode, so opcode + arbitrary length
both cross cleanly — the single-call 64 KiB-scratch shape the stub uses cannot carry
opcode and caps size):

```
ws_connect(url_ptr, url_len) -> i32     // handle (slot id) or -1
ws_send(h, ptr, len) -> i32             // 1 ok / 0 not-open; emits a TEXT frame
ws_send_binary(h, ptr, len) -> i32      // 1 ok / 0 not-open; emits a BINARY frame
ws_poll(h) -> i32                        // 1 = a frame dequeued+latched as "current"; 0 = nothing/down
ws_msg_len(h) -> i32                     // byte length of the latched current frame
ws_msg_copy(h, ptr) -> ()                // copy latched frame bytes into wasm memory
ws_opcode(h) -> i32                      // 1 text / 2 binary of the latched frame
ws_close(h) -> ()
```

`ws_group_*` in-browser: a thin JS loop over the same handle map, OR stubbed (a
browser client rarely multiplexes; the native sync-agent is the multiplexer).
**Open question R2:** does C2/C3 need the group API on wasm, or only the native peer?
Default: stub `ws_group_poll` on wasm to scan handles client-side; revisit if a real
browser multiplexer use case appears.

**Module name `loft_web`** is accepted by the `--html` import-module guard via the
`loft_`-prefix allowance landed for crypto (`src/native_utils.rs`).

---

## 6. host.js design

Mirror `crypto/wasm/host.js`: push a `LOFT_WASM_EXTENSIONS` callback `(imports,
_ctrl, getMem)` adding an `imports.loft_web` namespace holding a `Map<handle,{socket,
inbound[], current}>`. Key points (full sketch in the reconnaissance):

- `ws_connect`: `new WebSocket(url)`, `sock.binaryType = "arraybuffer"`; `onmessage`
  pushes `{op:1, bytes:enc.encode(str)}` for strings, `{op:2, bytes:new
  Uint8Array(buf)}` for ArrayBuffers; return a monotonic handle.
- `ws_send`: send a **string** (`dec.decode(...)`) → TEXT frame.
  `ws_send_binary`: send a **Uint8Array** (`.slice()` to detach) → BINARY frame.
  *(Getting this text-vs-binary split right is load-bearing for C3 — a binary CBOR
  payload sent as a text frame mangles CBOR's zero bytes.)*
- `ws_poll`: `current = inbound.shift(); return current ? 1 : 0`.
- `ws_msg_len`/`ws_msg_copy`/`ws_opcode`: read `current`. **Re-fetch
  `getMem().buffer` in every import** (memory may have grown); `.slice()` before
  `WebSocket.send`.

---

## 7. The Rust bridge — `web/wasm/` crate + `ws_client.rs` rewrite

- **New `web/wasm/` crate** mirroring `crypto/wasm/`: `Cargo.toml`
  (`crate-type=["rlib"]`, `loft` path dep; **no** dalek/RustCrypto — WS needs no
  Cargo deps, so the build-extension's cargo-build step is skipped by its
  `has-nonloft-deps` gate). `src/lib.rs` may be a thin shim or empty — the WS natives
  are `#native` symbols whose wasm bodies come from `ws_client.rs::wasm_impl`, not
  from routed pub-fns. **Confirm R3:** does a host-import-only bridge still need a
  `[wasm.bridge.routes]` entry, or does declaring the extern in the native crate's
  `wasm_impl` (compiled into the main wasm) suffice? (crypto's `random_fill` lives in
  the native lib's own wasm path, not the bridge crate — mirror that.)
- **Rewrite `ws_client.rs::wasm_impl`** under `#[cfg(target_arch="wasm32")]`:
  ```rust
  #[link(wasm_import_module = "loft_web")]
  unsafe extern "C" {
      fn ws_connect(ptr: *const u8, len: usize) -> i32;
      fn ws_send(h: i32, ptr: *const u8, len: usize) -> i32;
      fn ws_send_binary(h: i32, ptr: *const u8, len: usize) -> i32;
      fn ws_poll(h: i32) -> i32;
      fn ws_msg_len(h: i32) -> i32;
      fn ws_msg_copy(h: i32, ptr: *mut u8);
      fn ws_opcode(h: i32) -> i32;
      fn ws_close(h: i32);
  }
  ```
  `recv(h)` = `if ws_poll(h)!=1 {false} else { n=ws_msg_len(h); v=vec![0;n];
  ws_msg_copy(h,v.as_mut_ptr()); LAST_MSG=v; LAST_OP=ws_opcode(h); true }`. Add a
  `LAST_OP` thread-local (the stub hardcodes 1 — the C3 bug). Map the extern set onto
  the public `connect/send/send_binary/recv/last_message/last_opcode/close` names so
  `pub use` keeps `lib.rs` unchanged.

---

## 8. C3 — CBOR binary frames

The loft side is already NUL-safe via the `pack_*` builder (`pack_reset` →
`pack_u8/u16_le/u32_le` → `pack_take()->text` whose bytes are verbatim) and `byte_at`
for decode (loft `text` is a byte buffer; `from_utf8_unchecked` preserves zeros). The
native binary path takes `&[u8]` directly (not via `loft_ffi::text`), so zeros
survive. To keep C3 byte-identical the bridge must:

1. `ws_send_binary` emits a **binary** WS frame (`send(Uint8Array)`), opcode 2.
2. `onmessage` sets `binaryType="arraybuffer"`, tags inbound binary `op:2`;
   `ws_opcode` reports 2 so `last_opcode()==2` works on wasm.
3. `ws_msg_copy` moves raw bytes (no UTF-8 transcoding on the binary branch).

C3 producer/consumer: build a CBOR frame with the `cbor` lib (ZT-A), `send_binary`,
receive on the peer, compare every byte via `byte_at`. Native peer = the `server`
lib's WebSocket (the symmetric counterpart of `ws_client.rs`).

---

## 9. The verify harness (must exist BEFORE the bridge is trustworthy)

`tools/wasm_repro.mjs` calls `loft_start()` **synchronously, then `process.exit(0)`**
— no asyncify resume loop — so it CANNOT drive an interactive WS client (the event
loop never runs; §4.1). It is a trap-harness only.

**Build a new asyncify-aware Node driver** (`tools/wasm_ws_repro.mjs` or extend
`wasm_repro` behind a flag): detect the `asyncify_start_unwind` export, build an
`AsyncifyCtrl` (reuse the one in `loft-gl-wasm.js:44-84`), `ac.start('loft_start')`,
then drive `ac.resume('loft_start')` on a `setImmediate` loop (Node's rAF
equivalent) so WS events interleave between resumes. Node 22 has a global
`WebSocket`, so `host.js` runs unchanged; the peer is a tiny Node WS echo server.

This driver IS the C2/C3 verification loop. Per the loft-codegen discipline, build it
FIRST (step 1 below) — without it the bridge is unverifiable here.

---

## 10. Implementation sequence (each step gated)

1. **Verify-loop first.** New asyncify-aware Node driver + a Node WS echo server.
   Prove the resume loop works on a *trivial* asyncified loft program (a counter that
   `yield_frame`s) — confirm it returns to the event loop and resumes (the matrix
   "prove the harness can fail/pass" rule). **Resolve R1 here** (does headless
   asyncify resume work without GL?).
2. **Bridge plumbing.** `web/wasm/` crate + `host.js` (socket Map) + rewrite
   `ws_client.rs::wasm_impl` + `[wasm.bridge]` in `web/loft.toml`. Confirm R3 (routed
   vs native-wasm-path).
3. **`yield_frame` wiring.** Add `web::yield_frame()` (native no-op; `--html` →
   asyncify suspend, option (1) or (2) per R1). Build via `--html`; confirm the wasm
   imports only `loft_*` modules and instantiates.
4. **C2 — text echo.** One `.loft` client: connect → send text → `yield_frame` loop
   → receive the echo. Pass on native (real TCP, `--interpret`/native) AND wasm (the
   new driver + echo server). *Gate: same source, both backends.*
5. **C3 — CBOR byte-identity.** Native peer (`server` lib) ↔ wasm client exchange a
   fixed CBOR frame (built with the `cbor` lib); assert byte-identical both
   directions via `byte_at`. *Gate: opcode 2, zeros preserved, both backends.*

Graduate the C2/C3 programs to `web/tests-network/ws_*.loft` (+ the Node driver
invocation) as the regression record.

---

## 11. Risks / open questions — RESOLVED (implemented + verified 2026-06-21)

All four gates GREEN on both backends (`native == wasm`): C2 echo + C3 CBOR
byte-identity each pass on `--interpret` (real TCP) AND `--html` (the new
asyncify Node driver + Node echo server).

- **R1 (load-bearing) — RESOLVED, option (2).** Headless `--html` asyncify resume
  works with NO GL window. Proven by a hand-written minimal wasm
  (`tools/zt-c-web-staging/asyncify_probe.rs`): one `loft_web.ws_yield` suspend
  import, asyncified by the same `wasm-opt --pass-arg` pattern, resumes headless
  under the `setImmediate` loop. We took option (2) — a dedicated
  `loft_web.ws_yield` import added to the asyncify `--pass-arg` in `src/main.rs`
  — because `yield_frame()` does NOT lower to `loft_gl_swap_buffers` (option 1
  was a non-starter: a non-GL program never calls swap_buffers).
  **Two ABI corrections** to the `doc/loft-gl-wasm.js` AsyncifyCtrl were needed
  for the headless resume loop (the GL/browser path tolerates the originals; a
  Node poll loop does not): (a) rewind reads the save buffer DOWNWARD from the
  saved top — do NOT reset `current` to base before rewind; (b) the suspend shim
  must be STATE-AWARE — `stop_rewind`+return while rewinding, else `start_unwind`
  — or it spins forever on one loop iteration. Both folded into
  `tools/wasm_ws_repro.mjs`.
- **R2 — RESOLVED, client-side stub.** `ws_group_*` is stubbed on wasm
  (`ws_group_poll` reports nothing-ready). Revisit on a real browser-multiplexer
  need; the native sync-agent is the multiplexer.
- **R3 — RESOLVED: ROUTE, do not bare-host-import.** The non-routed host-import
  path the design floated CANNOT carry a `text`-returning native: the compiler
  declares e.g. `safe fn n_ws_client_message() -> i32` under `loft_gl`, then the
  wrapper reads `.ptr`/`.len` off that `i32` → a generated-code compile error
  (observed on this exact lib). So EVERY WS native is routed through
  `web/wasm/src/lib.rs` `pub fn`s (the crypto `text -> text` shape), which own
  wasm memory and copy frame bytes in via the ptr/len ABI. The bridge crate
  declares its OWN low-level `loft_web` host imports (`ws_connect`/`ws_poll`/
  `ws_msg_len`/`ws_msg_copy`/`ws_opcode`/`ws_send`/`ws_send_binary`/`ws_close`/
  `ws_yield`); host.js provides them. The native `ws_client.rs::wasm_impl` is
  therefore UNTOUCHED — it only covers the orthogonal wasm32-wasip2/`loftHost`
  path, not `--html`.
- **R4 — confirmed.** `wasm-opt` v108 present locally; the bundle asyncifies.
  CI must have binaryen.
- **R5 — confirmed safe.** The latched single-slot `LAST_MSG`/`LAST_OP` register
  survives the yield because the yield is at the loop TOP, after the read.

### Gotchas surfaced during implementation
- `yield` is a reserved loft keyword, so the loft API is **`frame_yield()`**, not
  `yield_frame()` (the native symbol stays `n_yield_frame`).
- The lib's `build.rs` generates the `loft_register_*` list from the `#native`
  annotations in `src/**/*.loft`; cargo caches it, so adding a `#native` needs a
  clean `target/release/build/<crate>-*` to re-emit (else the new symbol is
  unregistered → "library did not load").
- Run the C2/C3 `.loft` from OUTSIDE the lib tree: running a program located
  inside `web/tests-network/` makes `--lib` auto-discovery double-resolve the
  library and the native cdylib fails to register.

---

## 12. Probes (design-protocol falsification — run before/with implementation)

- **P1 — the deadlock probe (confirms §4.1):** a `--html` loft program that polls a
  WS without `yield_frame` — assert it deadlocks (page/driver hangs); the same with
  `yield_frame` makes progress. Proves the asyncify need is real, not assumed.
- **P2 — the headless-asyncify probe (R1):** a trivial asyncified counter loop under
  the new Node driver — assert resume interleaves with `setImmediate`.
- **P3 — C2 echo:** text round-trip, both backends.
- **P4 — C3 byte-identity:** CBOR frame with embedded zero bytes, both directions,
  both backends — assert every `byte_at` matches (the zero-byte preservation is the
  thing most likely to silently break).

---

## 13. File manifest

| Change | File | Repo |
|---|---|---|
| New bridge crate | `web/wasm/{Cargo.toml,src/lib.rs}` | loft-libs-net |
| New host imports | `web/wasm/host.js` | loft-libs-net |
| Rewrite `wasm_impl` | `web/native/src/ws_client.rs` | loft-libs-net |
| `[wasm.bridge]` + (maybe) routes | `web/loft.toml` | loft-libs-net |
| `yield_frame` API | `web/src/web.loft` (+ native no-op) | loft-libs-net |
| Asyncify Node driver | `tools/wasm_ws_repro.mjs` | loft |
| (option 2) dedicated suspend import | `src/main.rs` `--pass-arg` | loft |
| (R1 fallback) headless GL-swap guard | `doc/loft-gl-wasm.js` | loft |
| C2/C3 regressions | `web/tests-network/ws_{echo,cbor}.loft` | loft-libs-net |

---

## See also
- [`84-zero-trust-libs.md`](84-zero-trust-libs.md) — the ZT tracker; ZT-A (cbor) +
  ZT-B (crypto, incl. the build-extension + host-import patterns this reuses) shipped.
- The crypto bridge (`loft-libs-core/crypto/wasm/`) — the working host-import example.
- [WASM.md](../WASM.md) — the synchronous-host-ABI constraint + the asyncify frame yield.
