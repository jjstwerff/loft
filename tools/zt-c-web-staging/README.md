<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# ZT-C web WebSocket bridge — staging dir (@PLN84)

This directory stages the files that belong in the **`loft-libs-net/web`**
library (a SEPARATE repo at `~/workspace/loft-libs-net/web`, outside this loft
worktree).  They are copied here so this worktree's git history captures the
exact verified versions; the live copies under `~/workspace/loft-libs-net/web`
are where they actually build + run.

The loft-side files (the asyncify Node driver + the asyncify probe) live in
`tools/` proper and are committed normally — they are not staged here.

## Where each staged file belongs

| Staged path | Destination in `loft-libs-net/web/` | Change |
|---|---|---|
| `web/wasm/Cargo.toml` | `wasm/Cargo.toml` | **NEW** — bridge crate manifest (rlib, loft path dep, no other deps) |
| `web/wasm/src/lib.rs` | `wasm/src/lib.rs` | **NEW** — the WS bridge: routed `pub fn`s + the low-level `loft_web` host imports (ws_connect/poll/msg_len/msg_copy/opcode/send/send_binary/close/ws_yield) + pure-compute pack/byte_at |
| `web/wasm/host.js` | `wasm/host.js` | **NEW** — the `loft_web` host namespace: a `Map<handle,{socket,inbound,current}>` over a live `WebSocket`, plus the `ws_yield` asyncify suspend shim |
| `web/loft.toml` | `loft.toml` | **EDIT** — add `[wasm.bridge]` (crate + host_js) + `[wasm.bridge.routes]` (every WS native → its bridge `pub fn`); add `frame_yield = "n_yield_frame"` to `[native.functions]` |
| `web/src/web.loft` | `src/web.loft` | **EDIT** — add `pub fn frame_yield(); #native "n_yield_frame"` (the async↔sync poll-loop yield) after `sleep_ms` |
| `web/native/src/lib.rs` | `native/src/lib.rs` | **EDIT** — add `n_yield_frame()` (a native no-op; the browser lowers it to the asyncify suspend) |
| `web/native/src/ws_client.rs` | `native/src/ws_client.rs` | **UNCHANGED** (staged for reference only — the `wasm_impl` rewrite the design contemplated is NOT needed: `--html` uses the bridge crate, and `wasm_impl` only covers the orthogonal wasm32-wasip2/`loftHost.ws_*` path) |
| `web/tests-network/ws_echo.loft` | `tests-network/ws_echo.loft` | **NEW** — C2 text-echo regression |
| `web/tests-network/ws_cbor.loft` | `tests-network/ws_cbor.loft` | **NEW** — C3 CBOR byte-identity regression |

> NOTE — the native `web/native/Cargo.toml` patches `loft-ffi` to a sibling
> loft checkout.  The native cdylib must be built against the SAME loft-ffi the
> running loft uses, or its `loft_register_v1` ABI fingerprint mismatches and
> the library "did not load".  For the verify run below the cdylib was rebuilt
> against this worktree's `loft-ffi`.

## Verification (all four gates GREEN — `native == wasm`)

Run the echo server (`tools/zt-c-web-staging/ws_echo_server.mjs`) on a port,
then:

```bash
# C2 / C3 NATIVE — run the .loft from OUTSIDE the lib tree (a copy under /tmp);
# running it from inside web/tests-network/ confuses --lib auto-discovery.
loft --interpret --lib ~/workspace/loft-libs-net/web /tmp/ws_echo.loft  ws://127.0.0.1:9099/
loft --interpret --lib ~/workspace/loft-libs-net/web /tmp/ws_cbor.loft  ws://127.0.0.1:9098/

# C2 / C3 WASM — build with --html (worktree loft, which adds loft_web.ws_yield
# to the asyncify --pass-arg), extract the wasm, drive it headless.
loft --html /tmp/ws_echo.html --lib ~/workspace/loft-libs-net/web /tmp/ws_echo.loft
LOFT_WASM_HOST_JS=~/workspace/loft-libs-net/web/wasm/host.js \
  node tools/wasm_ws_repro.mjs /tmp/ws_echo.wasm
```

Observed:
- C2 native: `C2 PASS: echoed hello-zt-c`
- C2 wasm:   `C2 PASS: echoed hello-zt-c`
- C3 native: `C3 PASS: 10 bytes byte-identical, opcode 2`
- C3 wasm:   `C3 PASS: 10 bytes byte-identical, opcode 2`
- harness-can-fail control (no echo server): `C2 FAIL: ... after 2000 iters`
  (the poll loop yields 2000× and exits cleanly — no @P337 deadlock).
