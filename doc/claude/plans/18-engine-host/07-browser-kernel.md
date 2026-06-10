# Phase 07 — the browser kernel: the same loft script on a phone

**Goal.** A phone (or any browser) runs the SAME loft client script a native
seat runs — `run_client`, `sync_class`, `client_sync_next`, unchanged — with
the kernel underneath swapped for the browser's own machinery.  The phone is
the third host of one program, never a second implementation (the walk-up
tier of the arcade model: QR → playing; native seats stay the full seats).

**The invariant (user-stated, 2026-06-10): the script is the contract; the
kernel is swappable.**  If a client `.loft` file needs ANY edit to move
between a native seat and a phone browser, the phase has failed.  The
acceptance test pins it (§ Acceptance).

**Not a server.**  The browser kernel is the CONNECTOR role only — no
listener, no UDP socket, no cookie issuance.  It uses everything the browser
already provides instead of porting socket machinery.

## The split that makes it small

The native connector divides cleanly into two halves:

| Half | Contents | Browser strategy |
|---|---|---|
| **Pure machinery** | `conflate_slot`, the `SYNC_IDS` wire-schema table, `msg_id_of`/keys, keyframe (`S:`) parsing, event queue, budgeted drain, drift-free tick arithmetic | **compile unchanged** — no sockets in it; one implementation, two targets (the must-never-fork rule, enforced by construction) |
| **Socket I/O** | TCP/WS pump, UDP path, hello/keepalive, sleeps | **replaced by the browser**: `WebSocket` (callback-driven; `onmessage` feeds the same queue), the event loop as the idle, `performance.now()` for the tick grid |

Transport compatibility costs NOTHING: a browser cannot UDP, so it never
hellos, `udp_bound()` is false, and the server's `sync_send`/`keyframe`
fall back per client — the 05a contract already treats the phone as a
first-class slow-lane peer.  Keyframes arrive as plain reliable messages
(the unbound-peer carrier); sync arrives as ordinary WS frames.  The class
table still matters on the phone: inbound conflation (drain newest at the
frame) is the SAME latest-value semantics, applied to a render loop.

## The five seams (the slice list)

Closing these — kernel-side, never script-side — is the whole build:

1. **Same symbols, second registration.**  Every `n_kernel_client_*` /
   `n_kernel_connect` / `n_kernel_sync_class*` symbol registers a wasm-side
   body (browser WebSocket via the W1c host-bridge pattern), plus honest
   stubs for the surface that cannot exist (`n_kernel_client_udp_bound` →
   false).  `lib/engine_host`'s `.loft` source is shared verbatim —
   `cfg`-split `src/engine_host.rs` into the pure half (all targets) and
   the socket half (native only).
2. **The yield hides in the shared lib.**  A browser tab cannot host
   `run_client`'s `while` loop; the loop lives in `lib/engine_host`, so a
   per-turn `kernel_client_frame()` native (no-op natively; frame-yield in
   wasm — the shipped `--html` GL contract: set `frame_yield`, return from
   execute, JS resumes per `requestAnimationFrame`) keeps the lib's loft
   source identical and scripts untouched.  NOTE: yield must be
   unconditional per turn — the native `idle(2000)` path only runs when a
   turn was idle, and a busy tab must still yield.
3. **Time.**  `ticks()` must mean the same thing on both kernels.  VERIFIED
   2026-06-10: the browser runtime bridges it (`src/wasm.rs
   host_time_ticks` → JS `time_ticks`); the zero-stub trap is wasip2/
   wasmtime headless only.  Remaining work: confirm the doc/pkg bundle's
   host JS supplies `time_ticks` from `performance.now()`, and cover time-
   paced behavior in the differential (a cadence assert, not an eyeball).
4. **One input surface.**  Touch and mouse must reach scripts through one
   API (pointer events at the bridge) or input-reading scripts fork per
   host.  This seam belongs to the graphics/input bridge, not engine_host —
   coordinate with the `--html` input bridge work; the walk-up consumer
   (taps) needs only pointer-down with coordinates.
5. **The connect target.**  A phone connects to the cabinet that served it
   the page; a native seat takes a host argument.  `engine_host::
   default_host()` (browser = the page origin; native = `LOFT_HOST` env /
   args fallback) lets one script say `run_client(engine_host::
   default_host(), …)` and be literally identical — no "same except the
   IP".

## Acceptance — the one-script differential

The audience-differential pattern applied to HOSTS instead of servers:

> ONE client `.loft` file, run twice against the same kernel server —
> (a) native `run_client`, (b) the browser kernel under headless chromium
> (the html-bundle harness from `check_html_bundle.mjs` lineage) — and the
> per-client transcripts must be equal modulo transport (the native seat's
> sync rides datagrams; the phone's rides WS — same payloads, same order
> per kind, same world).

Plus the phone-reality probe (manual, recorded in the probe report): QR →
join latency on a real phone on LAN, the radio-wake effect of the WS
heartbeat cadence, and bundle size over the cabinet's LAN serve.

## Risks / honest residuals

- **Per-message JS→wasm copy** — irrelevant at walk-up rates (taps up,
  ≤30 Hz state down); measure once in the differential run.
- **Secure-context rules** — plain `http`+`ws` on LAN is the audience-demo
  reality and stays the default; `wss`/TLS is an off-LAN concern (parked
  with 05b's DTLS note).
- **Background-tab throttling** — the classes absorb it by design (poses
  conflate, events queue); the differential should include a deliberate
  pause/resume.
- **No tier 1 on phones** — the browser client is already wasm; its
  live-edit story is tier 0 (the in-browser interpreter) or page reload.
  Explicitly out of scope here (03's honest residual).
- **The unreliable browser fast path** (WebRTC datachannels / WebTransport)
  stays parked: heavyweight machinery, iOS dependability unclear — same
  trigger discipline as 05b.

## Sequencing

After the first phone-seat consumer exists (bumper-planes walk-up tier is
the natural driver); slices 1–3 are self-contained kernel work and can land
ahead of 4–5 if the consumer arrives sooner.  Slice order: 3 (verify time —
cheapest falsification), 1 (the split + registration), 2 (the yield), then
the differential; 4–5 ride the consumer.
