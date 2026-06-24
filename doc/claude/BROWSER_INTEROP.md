<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Browser interaction model — the engine as an agnostic service provider

How a loft `--html` program and its libraries interact with the browser and
with JavaScript. This is a **design doc**, not yet a shipped reference: the
asyncify yield and the host-import bridge it builds on are shipped (see
[HTML_EXPORT.md](HTML_EXPORT.md)); the agnostic byte-channel service and the
input-service surface below are the proposed additions. Where a claim is
already proven, this doc says so and points at the running code.

**Motivating consumer.** The zero-trust browser client (a `--html` page that
runs the same platform-blind loft core as the native binary, behind a clickable
file list + editor) is the dogfood that drives this. But nothing here is
document- or game-specific by design — that is the whole point.

---

## The one invariant

> **The browser host — the "engine" — moves only opaque bytes (and scalars)
> between a loft routine and JavaScript. It never knows the shape or meaning of
> what crosses. All knowledge of structure — loft types on one side, JavaScript
> structures on the other — lives in loft *libraries* that consume the engine,
> never in the engine itself. JavaScript-specific knowledge is confined further
> still, to libraries that declare themselves browser-only.**

Everything else in this doc is a consequence of that one rule. The engine is a
service provider with no opinion about its consumer; a game and a document
editor are both just routines that compose its services.

### Why this rule, and how it stays true (the re-assertion sites)

The brittleness of any "let loft talk to the browser" design lives in **how
many places encode knowledge of the other side**. The cure is to push that
count to near zero at the layer that must stay generic:

- The engine's host-import surface is **small and closed** — render, input
  (incl. text/IME/clipboard), the frame loop/scheduler, and one generic byte
  channel. Each entry takes/returns only bytes or scalars.
- A loft *type* (a struct, a vector) or a JavaScript *shape* (a JSON key, a
  typed-array layout) appearing in an engine host-import signature **is the
  leak** — the single smell to watch for in review or a lint. Because the
  engine surface is closed and tiny, this is a visible, catchable violation, not
  a silent one.

So `N` (sites that must restate the invariant) equals the number of engine
services — small, fixed, additive-only — and omission is **loud** (a non-byte
type in that surface is reviewable). That is the prospective tell that this
design is not a spray.

---

## The four tiers

| Tier | Knows about | Runs where | Example |
|---|---|---|---|
| **Engine** (host bridge) | nothing domain- or language-specific; moves bytes/scalars | the browser host (JS in the generated page; native libs natively) | render, input, loop, the byte channel |
| **Platform-blind loft libs** | loft types + a domain; **not** the platform | every target identically | the doc model, merge, crypto, CBOR |
| **Browser-target loft libs** | loft types **and** JavaScript shapes; declared browser-only | wasm/browser only | a JS-facing SDK; a DOM/typed-array adapter |
| **Consumer routine** | composes libraries into an application | wherever it is built | the zero-trust client; a game |

The document-editor behaviour I earlier mistook for "the engine lacks it"
(text layout, selection, a caret, undo) is **not** an engine concern and not a
gap in an agnostic surface — it is a **widget library** in the platform-blind
tier, a client of the engine's input + render services, reusable by a game's
in-world text field and a settings dialog alike. The engine gains general
*services* (text/IME/clipboard input); the *behaviour* stays in a library.

---

## What is shipped vs what is new

**Shipped** (today, on the installed loft):

- The `--html` AOT path: loft → generated Rust → `wasm32-unknown-unknown`
  cdylib, embedded base64 in a self-contained `.html`. One export today:
  `loft_start` (= `n_main`), plus `memory` and the asyncify control exports.
- The **host-import bridge**: codegen emits one `env`-namespace import per
  native a reachable program uses — this is how `loft_println`, the `gl_*`
  functions, and `web`'s `ws_*` become JS-implemented bridge functions. **A
  loft library already declares its own host imports this way.**
- The **asyncify yield**: a suspending import (`loft_gl.loft_gl_swap_buffers`,
  `loft_web.ws_yield`) returns control to the JS event loop and resumes intact.
  Issue #450 fixed the resume pump for hidden/headless pages (an unthrottled
  `MessageChannel` while hidden, `requestAnimationFrame` while visible).
- The **target-conditional library + JS bridge** mechanism: a `[wasm.bridge]`
  in `loft.toml` plus a `wasm/host.js`. The `web` library already ships exactly
  this (its `host.js` is where the browser WebSocket lives).

**New** (the proposed additions this design names):

1. A generic **byte channel** as an engine service — push bytes out to JS, poll
   bytes in from JS — declared as host imports that carry only `(ptr, len)`.
2. The **input-service surface** the engine is missing for any text UI: typed
   text + IME compose state + clipboard read/write, backed JS-side by a hidden
   editable element. General I/O services, not document features.
3. Convention (not a language feature) for **browser-target libs to emit
   JavaScript-shaped data** — see the cost gradient below.

No new core-language feature is required for any of this — no wasm-export ABI,
no wasm-bindgen. Everything reduces to the shipped yield + host-import
primitives, composed in libraries. (The wasm-export / JS-calls-loft ABI was
considered and set aside: it solves synchronous typed JS→loft calls, which this
model does not need — see *The data boundary* and *Rejected alternatives*.)

---

## The data boundary — push, poll, gather-until-enough

loft owns the loop (the engine's scheduler service); JavaScript is event-driven.
They never call each other's functions. They meet at the engine's byte channel:

- **Push (loft → JS).** A library calls a host import (`fn js_emit(channel:
  text, payload: text);`) with serialized bytes; the bridge routes it to a JS
  callback. Identical in shape to how `println` pushes a string the page
  appends. Available today.
- **Poll (JS → loft).** JavaScript enqueues a request on the host side; the loft
  routine polls it each frame and pushes a response back — the same channel run
  the other way. Polling each frame is already how loft reads input
  (`gl_poll_events`).

Because the **library serializes to bytes before it pushes**, the boundary never
carries a loft heap object. JavaScript reads `(ptr, len)` out of `memory.buffer`
exactly as it does for `println`; it never walks loft's store. The "a struct is
a store-addressed heap object JS can't read" problem simply does not arise.

### "Pretend to be synchronous" — the gather-until-enough contract

The loft side does **not** pay for the event-driven boundary in its own code.
The asyncify yield lets a library present a **synchronous-looking** API over the
frame-yielding engine: a function gathers inbound bytes across however many
frames it takes, decides it has *enough* (a complete request, a finished
computation), and returns the finished unit. The yielding is invisible to the
loft caller.

This is **already proven** — the zero-trust `ztclient` transport does exactly
it. `poll_for` loops `try_recv()` and calls `frame_yield()` each pass,
correlating frames by request-id until the match arrives, then returns it. The
loft consumer writes `let reply = poll_for(h, mid);` — straight-line — while
asyncify suspends to the event loop and resumes between polls.

**The single load-bearing rule:** *"blocking" must mean yield-and-accumulate,
never a hard spin.* The gather loop has to give the frame back each pass
(`frame_yield()`), so the engine keeps rendering and taking input while the
library waits. A wait that does **not** yield starves the event loop — the page
never receives, the UI freezes. (That is the exact failure issue #450's repro
showed: a synchronous loop that never returns to the event loop.) The yield
contract enforces this; `poll_for` obeys it.

### The cost gradient — how a browser-target lib shapes data for JS

A browser-only library is free to emit JavaScript-native structures. Pick the
cheapest rung that fits the payload; default to the top:

1. **JSON text, one push → `JSON.parse`.** JS gets a native object/array.
   Cheapest, the default; right for structured metadata (a file list, a record).
2. **A typed-array view over wasm memory (zero-copy).** The real reason to go
   past JSON: bulk binary (pixels, audio, file bytes) handed to JS as a
   `Uint8Array`/`Float32Array` with no copy.
3. **A host-import object-builder API** (`js_obj_new` / `set` / `push`).
   Richest, but **chatty** — many bridge calls per object, and the rung where
   you have hand-rolled a slice of wasm-bindgen. Reserve it for a genuinely
   needed live JS object graph; never the default.

---

## Failure paths (what breaks, and what holds the line)

Enumerated because this is where the invariant earns its keep — each row is a
way the design degrades and the thing that prevents it.

- **Event-loop starvation.** A gather/wait loop that does not yield freezes the
  UI and never receives (the #450 class). *Held by:* the yield-and-accumulate
  rule; every blocking-looking library primitive must `frame_yield()` on its
  wait path. Worth a lint/test that a suspend-using `--html` program reaches its
  final line both visible and hidden (`tests/html_asyncify.rs` already does this
  for the engine).
- **Agnosticism leak into the engine.** A convenience host import grows a loft
  struct arg or a JSON-shaped parameter, and domain knowledge seeps into the
  generic layer. *Held by:* the engine surface stays closed and bytes/scalars
  only; a non-byte type in an engine host-import signature is the reviewable
  smell.
- **JS-coupling leak into a platform-blind lib.** A lib that is supposed to run
  on every target starts speaking JSON keys or DOM shapes, and silently stops
  being platform-blind. *Held by:* JS-fluency is allowed **only** in libraries
  that declare themselves browser-only; the platform-blind tier carries byte
  payloads (e.g. CBOR) and nothing JS-specific.
- **Bridge chattiness.** Reaching for the object-builder rung by default turns
  one logical result into dozens of boundary crossings. *Held by:* default to
  JSON-text; escalate only for zero-copy binary or a genuinely required live
  object.
- **Partial / reordered inbound data.** A request arrives split or batched
  across frames. *Held by:* the library owns reassembly/framing — exactly as
  `ztclient` correlates frames by id rather than taking the first off the wire.
- **Reentrancy.** Asyncify is one suspendable stack: the gather is cooperative
  and single-threaded, so a library cannot be mid-wait on request A and process
  request B on the same stack. *Held by:* this is the correct shape for a UI
  (one event at a time); the per-frame poll-and-accumulate loop interleaves
  naturally with render. Name it so no one expects threads.

---

## First concrete work, and the probe before it

The smallest step that proves the model is the **input-service surface**, since
text input is the one capability a text UI needs and the engine lacks today:

- Engine: host imports for typed text + IME compose state + clipboard
  read/write, backed JS-side by a hidden editable element; declared in the
  graphics / game-infra library.
- Library tier: a text-editor **widget** that consumes those services.

**The cheap probe to run first** (an afternoon `--html` spike, before building
the real shell): a page showing a **clickable file list + one editable text
box**, exercising the input service end-to-end. It answers the one question the
clean architecture cannot answer from the desk — *does the engine's input
surface actually cover text/IME/clipboard for an arbitrary consumer* — and it
does so agnostically of any document model. If text editing on the input service
turns out to be a deeper hole than IME+clipboard, that is the lesson to harvest
**before** the UI is built on top of it, not after.

---

## Rejected alternatives (and when they would flip)

- **Export loft `pub fn`s as wasm exports + a JS→loft call ABI** (the
  "JS renders, loft computes" compute-core model). Solves *synchronous typed*
  JS→loft calls. Set aside because this model does not need them: loft owns the
  loop and the synchronicity lives inside the library via the yield. *Flips* if
  a future consumer genuinely needs synchronous typed JS→loft calls with loft
  *not* driving the loop.
- **wasm-bindgen / web-sys for the JS glue.** It automates the easy half (the JS
  marshalling helper, alloc/free, ptr/len packing) while leaving the actual work
  (loft-heap ↔ boundary bytes, which it cannot see), and it costs the
  self-contained `.html`, a pinned `wasm-bindgen-cli` toolchain, and a separate
  ES-module output. *Flips* only under the loft-drives-a-rich-DOM model (model B
  below), where wide bidirectional typed interop is the whole job.
- **loft drives the DOM directly** (a large DOM host-import surface). A much
  wider, typed, bidirectional bridge to maintain. Out of scope; the canvas +
  byte-channel model covers the consumer without it.

---

## See also

- [HTML_EXPORT.md](HTML_EXPORT.md) — the shipped `--html` pipeline: cdylib
  codegen, the WebGL2 bridge, the asyncify resume loop, the frame-yield contract
  this design builds on.
- [WASM.md](WASM.md) — the broader WASM runtime (VFS, host bridges, threading).
- [LAVITION.md](LAVITION.md) — why an agnostic engine-as-service-provider is the
  product thesis, not just a convenience.
