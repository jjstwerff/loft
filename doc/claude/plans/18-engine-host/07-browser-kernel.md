# Phase 07 — the browser kernel: the same loft script on a phone

> **Status (2026-06-10): slices 1+2+5 SHIPPED** (+ the time seam re-opened
> and closed for real).  `src/engine_host.rs` now compiles on every target:
> the pure machinery is shared, the socket half is `cfg`-native, and the
> browser kernel lives as the `browser` submodule — every `n_kernel_*`
> client symbol registered under the SAME name in the wasm build
> (`native.rs::KERNEL_FUNCTIONS_WASM`), so `lib/engine_host` is shared
> verbatim.  `run_client` gained the per-turn `kernel_client_frame()` yield
> (no-op native, frame-yield in browser); `default_host()` landed (browser =
> page origin via `origin_host`; native = `LOFT_HOST`/loopback); the async
> WS open is bridged by `ws_ready` + an outbox (sends queue until open —
> the ordered carrier keeps semantics).  **The build also completed the
> routing symmetry**: inbound WS frames now class-route on BOTH receive
> paths (server pump + connector), so a phone's sync sends land in the
> server's slots exactly like a native seat's datagrams — `conflate_ws`
> (ordered carrier: arrival order is the seq, one out-seq counter across
> carriers so the bind transition never reads stale).  TWO pre-existing
> time bugs found + fixed during the audit-then-build: `doc/loft-rt.js`
> defaulted `time_ticks` to 0 (now real µs; `fakeTicks` still overrides)
> and `doc/loft-gl.js` returned MILLISECONDS where loft expects µs (a
> 1000× clock bug in browser GL games).  Bundle rebuilt (`make wasm`).
> SHIPPED LATER THE SAME DAY: touch input (touches map onto the mouse state
> in loft-gl.js — one input surface), `engine_host.loft` bundled into the
> browser build (BUNDLED_LIB_FILES), and the FULL one-script differential
> harness (`doc/kernel-differential.html` + `engine_host_connector::
> browser_kernel_one_script_differential`): **both legs now pass.**  The
> browser leg was briefly `#[ignore]`d on a REAL finding the harness caught —
> an `Instant::now` "time not implemented" panic on the `compile_and_run` +
> frame-yield + `run_client` path (short non-yielding playground programs
> never hit it) — plus a second finding it unmasked once the panic fell: a
> yielding program run through the plain `compile_and_run` entry silently
> DROPPED its parked frame state.  BOTH are resolved: the differential now
> drives `compile_and_start` + `resume_frame` (the correct frame-yield
> pairing) and asserts the full transcript, and it is a **live gate** — a
> plain `#[test]` that self-skips only when chromium/node/the bundle is
> unavailable (verified green here 2026-07-07 against a freshly-built
> `doc/pkg/loft.js`).  REMAINING: only the real-phone probe (hardware day).

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

## What already exists (audited 2026-06-10 — most of this phase is shipped)

The user's instinct was right: the browser runtime already carries far more
of this kernel than the first draft assumed.

| Piece | State | Where |
|---|---|---|
| **WebSocket bridge** | **shipped end-to-end** — `ws_connect`/`ws_send`/`ws_recv`/`ws_last_message`/`ws_close` in the interpreter's own wasm build, JS side provided by `doc/loft-rt.js`; built so `use web` WS programs run in the browser TODAY | `src/wasm.rs` § WebSocket bridge; `doc/loft-rt.js` |
| **Time** | **shipped end-to-end** — `ticks()` bridges via `host_time_ticks` → JS `time_ticks` in BOTH host JS files (the zero-stub trap is wasip2-headless only) | `src/wasm.rs:350`; `doc/loft-rt.js`, `doc/loft-gl.js` |
| **Frame yield + resume** | **shipped** — session machinery (`frame_yield` → execute returns → `resume_frame`), RAF loop in the GL host | `src/wasm.rs` sessions; `doc/loft-gl.js` |
| **Input** | **mouse + keys shipped** (`gl_key_pressed`/`gl_mouse_*` via `wasm_gl.rs`); **touch missing** in `loft-gl.js` | `src/wasm_gl.rs:64-67` |
| **GL render** | **shipped** — the `--html` WebGL2 pipeline | `HTML_EXPORT.md` |
| **Lib loading in browser** | **shipped** — VirtFS + `DEFAULT_FILES` gating | `src/wasm.rs` |

So the seams below are mostly ASSEMBLY, not construction: the engine_host
wasm natives wrap the EXISTING `host_ws_*` functions; the yield native sets
the EXISTING `frame_yield`; only touch input and `default_host()` are new
code at all.  Revised effort: **S/M** (was M).

## The five seams (the slice list — revised against the inventory)

Closing these — kernel-side, never script-side — is the whole build:

1. **Same symbols, second registration.**  Every `n_kernel_client_*` /
   `n_kernel_connect` / `n_kernel_sync_class*` symbol registers a wasm-side
   body **wrapping the existing `host_ws_*` bridge** (no new transport
   code), plus honest stubs for the surface that cannot exist
   (`n_kernel_client_udp_bound` → false).  `lib/engine_host`'s `.loft`
   source is shared verbatim — `cfg`-split `src/engine_host.rs` into the
   pure half (all targets) and the socket half (native only).
2. **The yield hides in the shared lib.**  A browser tab cannot host
   `run_client`'s `while` loop; the loop lives in `lib/engine_host`, so a
   per-turn `kernel_client_frame()` native (no-op natively; frame-yield in
   wasm — the shipped `--html` GL contract: set `frame_yield`, return from
   execute, JS resumes per `requestAnimationFrame`) keeps the lib's loft
   source identical and scripts untouched.  NOTE: yield must be
   unconditional per turn — the native `idle(2000)` path only runs when a
   turn was idle, and a busy tab must still yield.
3. **Time.**  CLOSED — verified end-to-end 2026-06-10: `src/wasm.rs
   host_time_ticks` → JS `time_ticks`, provided by BOTH `doc/loft-rt.js`
   and `doc/loft-gl.js`.  Residual: a cadence assert in the differential
   (cover it, don't rebuild it).
4. **One input surface.**  Mouse + keys are shipped (`wasm_gl.rs` →
   `gl_key_pressed`/`gl_mouse_*`); the ONLY gap is touch — ~10 lines in
   `doc/loft-gl.js` mapping touch/pointer events onto the existing mouse
   state (the walk-up consumer needs only pointer-down with coordinates).
   Scripts keep reading the one existing input API.
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
the natural driver); slices 1–2 are self-contained kernel work and can land
ahead if the consumer arrives sooner.  Revised order (post-inventory):
1 (the split + same-symbol registration over the existing `host_ws_*`
bridge), 2 (the yield native over the existing session machinery), the
differential, then 4–5 (touch + `default_host`) ride the consumer.  Seam 3
is closed.  An interim proof exists even before slice 1: `use web`
WS-client programs already run in the browser — what's missing is only the
`engine_host` surface over the same bridge (the same-script invariant).
