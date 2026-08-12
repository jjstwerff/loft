<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN39 — TIC_TAC_TOE — protocol-validation vehicle (parked — blocked on infra)

**Status (2026-05-11):** **parked back to `plans/future/`.**
v1 / v2 / v3 (server + JS client) / v5 are shipped; the
remaining milestones (v3.5, v4, v6) all gate on infrastructure
that lives in other plans.  No un-gated work remains here.

Resume triggers:
- **v3.5 loft-WASM client** → unblocked by [@PLN32 YIELD.2](../32-event-loop/README.md#yield--four-target-async-portability-parallel-sub-arc)
  (the `compile_and_start` / `resume_frame` integration that lets
  a synchronous-ish loft program drive an async WebSocket).
- **v4 hot WASM swap** → gated on v3.5.
- **v6 closure retrofit** → unblocked by [@PLAN22 phase 2](../finished/22-mutable-closures/README.md#sequencing)
  (the implicit-by-body classifier).
- **General lib/server polish that helps every TTT vN** →
  [@PLN41](../41-server-hardening/README.md) items (a) + (b);
  XS each, can land independently.

Was promoted to active 2026-05-11 with intent to finish soon;
shipped v3 server-side + v3 JS-client browser game in that
session, then re-parked when the v3.5 spike confirmed the
remaining work all sits behind YIELD.2 / @PLAN22 phase 2.  The
architectural ceiling here is reached for now; pick this back
up after the gating plans land.

| Milestone | Status | What it adds |
|---|---|---|
| v1 | ✓ shipped | Single-client text protocol verifier (`tictactoe_*.loft`) |
| v2 | ✓ shipped | Multi-client + spectator routing (3/3 `multiplayer_v2` tests green) |
| v3 server-side | ✓ shipped 2026-05-11 | HTTP routing + Content-Type response on the same loft program that hosts the WebSocket — `tictactoe_server_v3.loft` serves `/`, `/client.loft`, `/favicon.ico` (404), unrouted (404), and upgrades `/ws` to the WebSocket game protocol.  2/2 `multiplayer_v3` tests green |
| v3 browser game (JS client) | ✓ shipped 2026-05-11 | Server's WS handler plays the v1-style single-game protocol; INDEX_HTML embeds an interactive vanilla-JS board.  End-to-end browser game without a loft client. 2/2 `multiplayer_v3` tests green |
| **v3.5** loft-WASM client | designed, not started | Replace the JS client with a real loft client running in the browser via the loft interpreter WASM (already shipped on the GitHub Pages site).  Reuses `doc/pkg/loft.js` + `doc/pkg/loft_bg.wasm` + `doc/loft-rt.js` 100%; adds WS host imports + a small bootstrap HTML + a `client.loft`.  Design + sequencing in [`v3.5-loft-wasm-browser-client.md`](v3.5-loft-wasm-browser-client.md).  **Also the mechanism for distributing the laptop client** for any loft program |
| v4 | after v3.5 | Client-uploaded scripts → server-side compile → hot WASM swap (the in-browser game-dev workflow) |
| v5 | ✓ shipped | Binary world stream + N clients + catch-up + sluggish tempo (5/5 `multiplayer_v5` tests green) |
| v6 | gated on @PLAN22 | Drop `Reference<T>` ceremony in the v5 server using writable closures.  Pure ergonomic cleanup; not on @PLN6's critical path |

The protocol mechanics for v3 / v4 are **planned and will be
built**, but **the visual side of tic-tac-toe is deferred
indefinitely — a graphical / playable tic-tac-toe game is never
going to ship**.

The purpose of tic-tac-toe in this codebase is **protocol
validation only.**  Each vN milestone adds one new protocol /
infrastructure capability and is verified end-to-end by the
smallest possible text-mode programs (no graphics, no real
gameplay UX).  The actual playable game targets that consume
these ground layers live in
[MULTIPLAYER_EDITOR.md](../33-multiplayer-editor/README.md) and beyond.

So when later sections describe "mouse handler", "drawing
handler", "render the board", etc. for any vN — read those as
**aspirational spec for what a graphical tic-tac-toe would look
like**, not as work that ships.  The actual deliverable for vN
is whatever text-mode test program proves the new ground-layer
capability works on the wire.

This document exists so the design survives across sessions.
The purpose is to record what the game validates and what
shape its code will take, **not** to design every detail in
advance.

---

## The architectural floor — interpreter as baseline

The loft interpreter is **not** a "fallback for when WASM isn't
ready" or a "development convenience."  It is the **always-running
baseline** that the rest of loft's runtime architecture rests on.
Native WASM is the optimisation; the interpreter is the floor.

Originally motivated by exactly this: a loft client always has
running code.  No build step, no compile wait, no "loading native
module…" moment, no required server for the client to function.
When the server ships compiled WASM, the client runs faster.  When
the server doesn't — cold start, network blip, dev environment
without a toolchain, hot-edit mid-frame, or the player simply not
having an internet connection — the client keeps running on the
interpreter.

This single property reframes design choices that look independent
in isolation:

- **Auto-reconnect with backoff** in `lib/web` isn't merely
  "graceful failure handling."  It's the client staying alive on
  the interpreter while the server comes back, with no
  user-visible interruption.  The reconnect loop pumps no events
  while disconnected; the game keeps running its current state.
- **The shared memory model between interpreter and
  `--native-wasm` output** isn't a coincidence.  It's the
  load-bearing invariant that makes the hot hand-off (V4) work:
  same `Stores`, same `DbRef`s, byte-identical struct / vector /
  hash layouts.  An object allocated by the interpreter at store
  7 slot 42 is at the same address with the same bytes when WASM
  takes over.  The swap rebinds which code reads the heap; it does
  not migrate state.  **Any future codegen optimisation that
  diverges interpreter and WASM layouts breaks this invariant.**
- **V4's hot-swap from server-compiled WASM** is one instance of a
  general pattern: the interpreter ALWAYS runs; native code layers
  in when and where it's worth the build cost.  Same shape applies
  to cold startup (interpret while WASM compiles in the
  background), to reconnect (interpret while server is down), to
  fallback on WASM verification failure, to dev iteration with no
  build server present.
- **V3's static-asset serving** doesn't strictly need to ship a
  pre-compiled `.wasm` at all in the first cut.  The server can
  ship `.loft` source; the browser's pre-loaded loft-interpreter
  WASM (the runtime itself) parses and runs it; the optimised
  per-program WASM-compile path lights up later as an
  optimisation, not a precondition.

The hello-world client running on `--interpret` without a separate
build step is the **first concrete demonstration of this
principle.**  Tic-tac-toe v1 needs no `--native-wasm` step
either — the interpreter handles it.  When v3 adds the browser
path, the same code runs through the already-shipped loft
interpreter WASM, executing the `.loft` source the server
delivers.

### Invariant to protect

The shared layout between interpreter and `--native-wasm` output
deserves CI protection: a fixed test program, allocated under
both code paths, with a byte-by-byte heap-image comparison.
Without that check, a refactor in either codegen path can
silently break the hand-off and v4's hot-swap.  This belongs in
the loft test suite once v4 implementation begins; recording it
here so the dependency is visible.

---

## Why this game

The hello-world round-trip proves a single text payload can travel
from a loft client (browser-or-native, via `lib/web`) to a loft server
(via `lib/server`) and back.  Tic-tac-toe is the smallest *real* game
that exercises the next layer up:

- **Three independent handlers on one connection.**  The client has
  more than one thing to listen for and more than one thing to send.
  This is the first concrete consumer of the handler-id-on-the-wire
  idea (see [EVENT_PROTOCOL.md](../32-event-loop/PROTOCOL.md) for the
  binary-frame wire spec).
- **State on both ends.**  The server owns the authoritative game
  state; the client renders a copy.  This is the shape every
  multiplayer loft game will have — first concrete instance.
- **Input + render + sound** in a single loft program.  Combines the
  graphics library with the WebSocket client; first time those two
  meet under one main loop.
- **Server-decided placements.**  The server doesn't echo; it
  chooses where to play and tells the client.  This validates the
  asymmetric request-response pattern (client sends "I clicked",
  server responds with "you're at X, I'm at Y").
- **Client autonomy under server outage.**  Per the floor
  principle above: if the server is unreachable mid-game, the
  client doesn't crash or freeze.  The auto-reconnect loop spins
  in `lib/web` while the interpreter keeps the UI responsive.
  When the server returns, the next click resumes the protocol.
  Tic-tac-toe v1's simplicity makes this the cleanest test of the
  invariant — no complex state to lose, just a few cells.

What this is **not**: a fun game, a polished game, a game with AI.
The server's "AI" is the simplest legal move (e.g., first empty
cell).  Visuals are a 3×3 grid with X / O glyphs.  This is a
protocol + architecture demonstrator.

---

## Wire protocol — `<id>:payload`

The full wire-format spec lives in
[EVENT_PROTOCOL.md](../32-event-loop/PROTOCOL.md).  Tic-tac-toe is the v1
text-mode consumer: every WebSocket text frame after the MAP
handshake is `<id>:<payload>` where `<id>` is an integer handed
out by the server's name→id registry and exchanged with the
client through the MAP frame.

The shape is deliberately close to the binary v2 frame
([EVENT_PROTOCOL.md § Binary-mode wire format](../32-event-loop/PROTOCOL.md#binary-mode-wire-format-v2))
which carries `handler_id: u32` plus priority / seq / flags /
length in a 12-byte header.  Going through a name→id registry
from day one means the upgrade from v1 (text) to v2 (binary)
changes only the encoder/decoder; the program keeps registering
names and receiving integer ids the same way.

### v1 handler registration — server-arbited via a handshake

The **server is the arbiter** of the name→id map.  The client
cannot decide ids unilaterally; even if both sides agree on a
default registration order, that's a fragile coupling that
breaks the moment one side adds, removes, or reorders a handler.
Instead, the wire protocol begins with a small handshake:

1. Client connects; WebSocket upgrade completes.
2. Server sends a **MAP frame** listing its name→id assignments:
   `"MAP:Click=0,Placement=1,GameOver=2"`.
3. Client parses the MAP frame and populates its local registry
   with the same assignments.
4. Both sides now exchange `<id>:<payload>` frames; the MAP
   prefix is reserved and disambiguated by the literal text.

The MAP frame is the single piece of out-of-band protocol on
the wire.  Every other frame is `<id>:<payload>` with `<id>` an
integer.  In v1 the names are fixed (`Click`, `Placement`,
`GameOver`) so the MAP is constant per server build, but the
mechanism scales: new server-side handlers appear in the MAP at
new ids, and clients learn them at handshake time without code
changes to the wire format.

When the EventLoop's full handshake lands (including encoding
negotiation, capability flags, version), the MAP becomes one
field in a richer handshake frame.  The shape — server is
arbiter, client conforms — does not change.

| Name | Direction | Payload format |
|---|---|---|
| `"Click"` | client → server | `row,col` |
| `"Placement"` | server → client | `mark,row,col` |
| `"GameOver"` | server → client | `winner` (`X` / `O` / `draw`) |

The id column is absent from this table because **the client
must not assume specific ids** — it learns them from the MAP
frame.  Server-side, ids are whatever the server's local
`register("Click")` etc. happen to assign in registration order
(0, 1, 2 in this case, but that's an implementation detail).

### The registry abstraction

Each program holds a `Registry` struct in its main loop with a
`hash<text, integer>` (and a parallel `vector<text>` for
reverse-lookup so dispatch can map id → name where needed):

```loft
struct Registry {
    name_to_id: hash<text, integer>,
    id_to_name: vector<text>,
}

fn register(reg: Reference<Registry>, name: text) -> integer {
    let id = reg.id_to_name.len();
    reg.name_to_id[name] = id;
    reg.id_to_name.append(name);
    id
}

fn lookup_id(reg: Reference<Registry>, name: text) -> integer {
    if reg.name_to_id.contains(name) { reg.name_to_id[name] } else { -1 }
}
```

Three guarantees this gives v1:

1. **Names appear at registration only.**  After startup, no
   string comparison runs in the hot path; `pack` and `dispatch`
   work on integers.
2. **The wire format already speaks integers.**  Moving from
   v1's `"<id>:<payload>"` text frames to the EventLoop's binary
   header means swapping the encoder/decoder; the registry, the
   handler logic, and the dispatch all stay unchanged.
3. **Cross-side coordination is just "register in the same
   order."**  No protocol-level negotiation in v1.  V4 evolves
   this into a handshake that exchanges the map dynamically.

Three IDs, three handlers on the client (one outbound, two
inbound).  The server has the inverse: one inbound (player
clicks) and two outbound (placements, game-over).

### Why text frames and not the full EventLoop binary header?

[EVENT_LOOP.md](../32-event-loop/README.md) specifies a 12-byte binary header
with handler id, priority, sequence, flags, length.  This game
deliberately does **not** ship that yet — the directive was
*"no priority or digital layer yet."*  The `<id>:<payload>`
text form is the smallest thing that can distinguish handlers
on a single connection, fits in WebSocket text frames, and
round-trips through the existing text-only `ws_send` / `ws_recv`
pair without binary-frame work.

When the EventLoop binary wire format lands, this game is the
natural first port: replace the text encoder/decoder with the
binary header; the handlers, registry, and MAP handshake stay
unchanged.

### Programmer-level dispatch (today)

Library-level multiplexing across handlers requires storing one
closure per handler in a struct field — currently blocked by
[@P213](../../PROBLEMS.md#213-typefunction-storage-layout-limit--full-design-for-the-proper-fix).
For this milestone, the program's loop drains messages with
`web::try_recv()` and matches the parsed integer id against the
handler ids learned from the MAP frame:

```loft
raw = ws.try_recv();
if raw == null { /* nothing yet */ }
else {
    id = parse_id(raw);
    body = parse_payload(raw);
    if id == h_placement { handle_placement(body); }
    else if id == h_gameover { handle_game_over(body); }
}
```

`handle_placement` and `handle_game_over` are top-level fns
(or closures capturing `Reference<Board>`) that mutate a
`Board` held in a Store.  Same workaround pattern as
`hello_client.loft`.

---

## Client architecture

> **Implementation status.**  The shipped first cut
> (`lib/game_protocol/examples/tictactoe_client.loft`) is
> **text-only**: it sends a hardcoded sequence of clicks and
> prints the placements / winner it receives, validating the
> wire protocol and registry mechanics end-to-end.  The
> graphical client described below — three handlers with mouse
> + sound + render via `lib/graphics` — is the next step;
> it's the v1 spec's full shape, layered on top of the same
> protocol the text-only client already drives.

Three loft-level handlers, sharing a single WebSocket connection:

### 1. Mouse handler (outbound, with sound)

Polls `lib/graphics`'s mouse state once per frame.  On a click
inside a board cell:
1. Compute (row, col) from mouse coords.
2. Play a click sound (lib/graphics has audio support in moros_*
   examples; choose the simplest `play_sound` API).
3. Send `<click_id>:row,col` over the WebSocket, where
   `click_id` is the integer assigned to the `Click` handler by
   the server's MAP frame.

The mouse handler does *not* update the local board — it just
sends the click.  The server's reply (Placement frame) is what
causes the visible mark.  This keeps the server authoritative.

### 2. Drawing handler (inbound)

Triggered when a `<placement_id>:mark,row,col` message arrives.
Updates the `Board` struct's cell, which the per-frame render
reads to draw X / O glyphs.  Does not draw directly — sets
state; the render loop draws.

### 3. Winner handler (inbound)

Triggered when a `<gameover_id>:winner` message arrives.  Sets
a `game_over` flag in the board struct with the winner's mark
(or "draw").  The render loop displays a banner.  Optionally:
stop sending mouse clicks once `game_over` is set.

### Main loop shape (graphical v1, future)

```
fn main() {
    state = init_board();
    ws = ws_handler("ws://localhost:7878/");

    // Phase 1: handshake — populate registry from MAP frame.
    // (See tictactoe_client.loft for the actual pattern.)
    let h_click     = ...;
    let h_placement = ...;
    let h_gameover  = ...;

    // Phase 2: play.
    while !state.game_over {
        // outbound
        if mouse_just_clicked() {
            let (r, c) = mouse_to_cell(...);
            play_click_sound();
            ws.send("{h_click}:{r},{c}");
        }

        // inbound — programmer-level dispatch
        raw = ws.try_recv();
        if raw != null {
            id = parse_id(raw);
            body = parse_payload(raw);
            if id == h_placement       { state.apply_placement(body); }
            else if id == h_gameover    { state.set_winner(body); }
        }

        render_board(state);
        gl_swap_buffers();
    }
}
```

`state` is allocated in a Store; `Reference<Board>` is captured
by handler functions (today's workaround per
[plans/finished/22-mutable-closures/README.md](../finished/22-mutable-closures/README.md)).  The shipped
text-only first cut omits `mouse_just_clicked` / `render_board`
/ `gl_swap_buffers` and walks through a hardcoded click sequence
instead.

---

## Server architecture

The server uses a handler-style API symmetric with the client:
Rust owns the listening socket, the accept loop, and the
per-client read pumps; loft passes a single handler closure that
fires for connection / message / disconnect events.  This is the
right architecture for the game and is documented as future work
in [§ Server-side handler API (future)](#server-side-handler-api-future)
below.

For the **shipped first build** (`tictactoe_server.loft`), the
existing lifecycle-explicit `lib/server` API is acceptable: one
client at a time, sequential accept / upgrade / recv loop in
loft.  Rough shape:

1. Accept WebSocket upgrade.
2. Send the MAP frame: `MAP:Click=<id>,Placement=<id>,GameOver=<id>`.
3. Receive a Click frame `<click_id>:row,col`.
4. Validate: is the cell empty?
5. Place the player's mark (X).  Send Placement
   `<placement_id>:X,row,col` to the client.
6. Check for win / draw.  If terminal, send GameOver
   `<gameover_id>:winner_or_draw` and disconnect.
7. Else: pick the simplest legal move (first empty cell,
   left-to-right top-to-bottom).  Place server's mark (O).
   Send Placement `<placement_id>:O,row,col`.
8. Check for win / draw again.  If terminal, send GameOver.
9. Loop back to step 3.

No turn timer, no rematch, no error frames.  Invalid moves are
silently dropped.

Server-side native code already supports text WebSocket frames
via `lib/server`; this milestone needs no new native work on the
server side for v1.

### Server-side handler API (future)

The proper end-state — symmetric with `lib/web`'s
`ws_handler / pump / send / close` — is for `lib/server` to grow
a `ws_serve / pump / send_to / broadcast / disconnect / close`
shape.  The Rust layer owns the listening socket, the accept
loop, and per-client read pumps; loft just registers an event
handler.

```loft
let srv = ws_serve(7878);

while !done {
    srv.pump(fn(event) {
        match event.kind {
            Connected(client_id)         => log("welcome " + client_id),
            Message(client_id, payload)  => dispatch(client_id, payload),
            Disconnected(client_id)      => cleanup(client_id),
        }
    });
}

srv.close();
```

Outbound:

```loft
srv.send_to(client_id, "{h_placement}:X,1,2");
srv.broadcast("{h_gameover}:X");
srv.disconnect(client_id);
```

`client_id` is an opaque integer the loft programmer carries
from the `Connected` event into later `send_to` calls; the Rust
side allocates and owns the per-client connection table.

#### Rust-side options

Two valid shapes for the Rust implementation, same loft API
either way:

- **Polled-only.**  The listening socket is non-blocking; each
  `pump()` call drives one round of `accept()` + per-client
  `read()` with short timeouts, builds an event list, returns
  events one by one.  Simpler; latency bounded by pump frequency.
  Matches the client's `ws_recv` polling shape.
- **Background thread + mpsc channel.**  A Rust thread blocks on
  `accept()` and per-client `read()`, pushes events into a
  channel; `pump()` drains the channel.  Lower latency, more
  concurrent, still single-threaded from loft's view.  Needs
  `Send` discipline on the connection table.

Recommended sequencing: ship polled-only first.  Move to
background-thread later if latency becomes a measurable
complaint.

#### What the symmetric API replaces

Today's `lib/server` API surface for WebSocket:

```loft
let srv = listen(port);
loop {
    let req = srv.next();        // blocks
    let ws = ws_upgrade();       // blocks
    loop {
        let msg = ws.next();     // blocks
        // ... handle one connection sequentially ...
    }
}
```

This is fundamentally **single-client** — the outer loop blocks
on `next()` and only handles one connection through disconnect
before accepting the next.  Tic-tac-toe v1 fits inside that, but
multiplayer features (lobby, multiple games, etc.) require the
many-client capability of the handler API.

Side benefit: the new C-ABI surface is a clean redesign with
consistent return types from day one — the existing
`Str` / `LoftStr` mismatch in `n_ws_message` (which currently
blocks native compile of the lifecycle-explicit API) becomes
irrelevant rather than something to patch.

This work is **not on the critical path for tic-tac-toe v1**;
it's recorded here so the design is ready to pick up immediately
after v1 ships.

---

## Components needed

| Component | Status | Notes |
|---|---|---|
| `lib/web` WebSocket client | **Shipped & validated** | Handler-style API (`ws_handler` / `send` / `pump` / `try_recv` / `close`); auto-reconnect with backoff; multi-address resolution.  Validated end-to-end by hello-world and the text-only tic-tac-toe v1. |
| `lib/server` WebSocket server | **Shipped** (interpret mode); native-compile mode has a separate pre-existing `Str`/`LoftStr` mismatch that doesn't affect this game | The WS handshake header bug (reading from `LAST_BODY` instead of `LAST_HEADERS`) was fixed in this branch.  Native-compile path of programs using lib/server still needs the C-ABI return-type fix; interpret-mode runs cleanly. |
| `lib/graphics` mouse + render + sound | Shipped (used by `moros_*` and `lib/graphics/examples/`) | Reuse existing primitives.  **Not yet wired into tic-tac-toe** — the shipped first cut is text-only. |
| Wire-frame parser | **Shipped (inline)** | `parse_id(raw)` and `parse_payload(raw)` in `tictactoe_server.loft` and `tictactoe_client.loft`; ~6 lines each.  Same shape on both sides. |
| Board state struct | **Shipped (server side)** | 9-cell vector with null = empty, plus a `winner: text` field, in `tictactoe_server.loft`.  Render-side board does not exist yet (text-only client). |

No new native code required for this game; everything builds on
already-shipped pieces.

---

## What this validates

- **Handler-id-on-the-wire as a real concept.**  Three named
  channels (`Click`, `Placement`, `GameOver`) coexisting on one
  connection, each with distinct semantics, distinguished by an
  integer id the receiver reads.
- **Server-arbited handshake.**  The server's MAP frame
  authoritatively assigns ids; the client conforms.  Validated
  in `tictactoe_server.loft` (sends MAP) and
  `tictactoe_client.loft` (parses MAP into a local registry).
- **Asymmetric channels.**  `Click` is client-out / server-in;
  `Placement` and `GameOver` are server-out / client-in.  No
  handler echoes — each is genuinely one-way.  Mirrors the
  EventLoop spec's per-direction model.
- **Programmer-level dispatch works.**  Even without library
  closure-in-struct routing, parsing the integer id in a
  poll-driven loop (via `web::try_recv`) is ergonomic enough
  for a real game.
- **Server-authoritative state.**  The client never assumes a
  click landed; it waits for the Placement frame to confirm.
  This is the pattern every multiplayer loft game will follow.
- **Auto-reconnect doesn't disturb the game logic.**  If the
  connection blips, the game pauses; once reconnected, the next
  user click resumes flow.  No special game-side handling.

---

## What this deliberately does NOT validate

- Priority lanes (no priority byte; everything is one lane).
- Binary-frame protocol (text only).
- JSON or struct serialisation (raw text payloads).
- Library-level multi-handler routing (programmer parses the
  prefix; library does not dispatch on it).
- Closure-in-struct-field captures (workaround via `Reference<T>`
  capture and per-call `pump(callback)`).
- Reconnect *recovery* of game state (server doesn't persist
  per-client; on disconnect the game resets).

These are all named milestones in their own right and are
explicitly out of scope here so this game stays minimal.

---

## Sequencing

The graphical tic-tac-toe is **not** on the sequence.  Each vN
ground layer adds protocol/infrastructure capability and is
verified by the smallest text-mode program that can exercise
it.  The actual playable game work (mouse, render, sound) goes
into [MULTIPLAYER_EDITOR.md](../33-multiplayer-editor/README.md) and beyond.

1. ~~**Land hello-world end-to-end**~~ — **DONE.**  Cleared two
   pre-existing blockers: transitive native dlopen for
   non-native parent packages (parser fix), and the lib/server
   WS-handshake header bug (`LAST_HEADERS` thread-local).
   Validated by `lib/game_protocol/examples/hello_*.loft`.
2. **Tic-tac-toe v1** — **DONE.**  Server-arbited MAP handshake;
   namespace handler registry; `<id>:<payload>` text frames;
   server-authoritative state model.  Verified by
   `tictactoe_server.loft` + `tictactoe_client.loft`
   (text-mode).  No graphics — confirms the protocol works.
3. **Tic-tac-toe v2 (protocol-only)** — multi-client server +
   per-client state + cross-client routing.  Validated by two
   text-mode clients connecting to the same server, each
   playing their own game while receiving spectator updates of
   the other.  This ground layer's primitives also unblock
   [MULTIPLAYER_EDITOR.md](../33-multiplayer-editor/README.md).
4. **Tic-tac-toe v3 (protocol-only)** — server delivers the
   client over HTTP; browser-side WS bridge in `loft-rt.js`;
   text-mode tic-tac-toe client compiled to WASM, loaded in a
   browser, connecting back to the same loft server.  No
   graphical board — `console.log` is the output.  This ground
   layer enables real browser-hosted loft programs.
5. **Tic-tac-toe v4 (protocol-only)** — client uploads loft
   source over WS; server compiles via `loft --native-wasm`;
   server returns WASM bytes via binary frame; client hot-swaps
   the running module.  Verified text-mode (the swapped-in
   module's behaviour is observed via console).  This ground
   layer enables the live-edit dev workflow.

Visual / playable tic-tac-toe is **deferred indefinitely** —
the protocol patterns are validated text-mode; visual UX work
happens in real games (the editor, eventually shipped game
prototypes), not in tic-tac-toe.
3. **Tic-tac-toe v2 — two simultaneous clients with spectator
   view.**  Server accepts two concurrent clients; each plays
   their own game vs the server; each client *also* sees the
   other client's board updating in real time alongside their
   own playable board.  See § Tic-tac-toe v2 below.
4. **Multi-client / lobby / matchmaking** — later milestone.

## Tic-tac-toe v2 — two clients, mutual spectatorship

Once v1 runs, this is the next concrete validation step.

### What it adds

- **Two concurrent clients on one server.**  Forces the
  symmetric `ws_serve / pump` architecture documented above to
  ship — the lifecycle-explicit lib/server API can't handle two
  clients at once.
- **Per-client state on the server.**  Each connected client has
  its own board; the server holds a small table of
  `client_id → Board`.
- **Cross-client routing (broadcast-shaped, but selective).**
  When client A makes a move, the server updates A's board AND
  forwards the move to client B as a *spectator* update.  Client
  B's display has two boards: their own (playable) and A's
  (read-only).

### New handler names (extension to v1)

In addition to v1's `Click` / `Placement` / `GameOver`, two more
handler names distinguish own-game from spectator updates.  The
server's MAP frame assigns ids to all five at handshake; both
sides learn the assignments from the same map.

| Name | Direction | Meaning | Payload |
|---|---|---|---|
| `Click` | client → server | player click | `row,col` |
| `Placement` | server → client | placement update — *own* game | `mark,row,col` |
| `GameOver` | server → client | game over — *own* game | `winner` or `draw` |
| `SpectatorPlacement` | server → client | placement update — other client's game | `mark,row,col` |
| `SpectatorGameOver` | server → client | game over — other client's game | `winner` or `draw` |

The wire format stays `<id>:<payload>` text frames.  No peer-id
is needed in the payload because each client only ever
spectates exactly one other client (the partner the server
paired them with at connect time).

### Client architecture (v2)

The client gains two more inbound handlers
(`SpectatorPlacement`, `SpectatorGameOver`) and renders a second
board next to the first.  The mouse handler is unchanged — it
still only sends `<click_id>:row,col` for the player's own
game.  The own-board is interactive; the spectator-board is
purely display.

```
┌──────────────────┬──────────────────┐
│   Your game      │   Other player   │
│                  │                  │
│   [3×3 grid,     │   [3×3 grid,     │
│    interactive]  │    read-only]    │
│                  │                  │
│   Status: ...    │   Status: ...    │
└──────────────────┴──────────────────┘
```

Five handlers total: one outbound (mouse-with-sound), four
inbound (own-placement, own-winner, spectator-placement,
spectator-winner).

### Server architecture (v2)

Per-client state: `Board` keyed on `client_id`.  Client pairing
strategy for v2: simple — pair the first two connected clients;
reject the third.  When client A clicks:
1. Process the move on A's board (player mark + server's mark).
2. Send `Placement` / `GameOver` back to A as before.
3. ALSO send `SpectatorPlacement` / `SpectatorGameOver` to B
   with the same placements, tagged as spectator events.

If a client disconnects, the partner's spectator view freezes
on the last update; the partner's own game continues.  No
rematch handling.

### What v2 validates

- **The handler-style server API under real load** — two clients
  active simultaneously, with both inbound and outbound traffic
  on each connection.
- **Per-client state on the server side** — first concrete
  consumer of the multi-connection capability.
- **Asymmetric handler-name usage between clients** — both
  clients use the same set of inbound names but the *meaning*
  of `Spectator*` (the other client's game) implies the server
  is doing real routing, not echo.  This is the simplest
  validation of "the server actively uses handler names to mean
  more than just message-type labels."
- **Five-handler clients** — pushes the programmer-level
  dispatch in `pump()` callback hard enough to demonstrate the
  pattern at meaningful scale.  If five handlers fit in the
  workaround pattern cleanly, the case for library-level
  dispatch (post-@P213) becomes purely about ergonomics, not
  feasibility.

### Out of scope for v2

- More than two simultaneous clients (rejected at connect).
- Reconnection / state recovery on disconnect.
- Lobby / matchmaking UI.
- Player names, chat, anything that isn't placement + winner.
- Sound for the spectator board (the click sound only fires for
  the player's own moves).
- Static asset serving (the v2 client is run via `loft <file>`;
  v3 wraps it in an actual browser).

These all become later milestones; v2 stays minimal-but-real.

## Tic-tac-toe v3 — server delivers the client

After v2 lands, this is the next concrete step.  The same loft
server now also acts as the **web server** that delivers the
client to the browser: the HTML page, the loft-compiled WASM
binary, any CSS / JS assets.  A real player opens
`http://localhost:7878/` in a browser, the server hands back the
page + WASM, the WASM connects back to the same server's
WebSocket endpoint, and the rest is v2's protocol.

### What v3 adds

- **HTTP asset serving** alongside WebSocket on the same loft
  server.  GET `/` returns `index.html`; GET `/client.wasm`
  returns the compiled client; GET `/client.css` returns
  styling; GET (anything else) returns 404.
- **The client's loft-compiled-to-WASM build** actually runs in
  a real browser, connecting via the browser-WebSocket bridge
  (the wasm32 path of `lib/web` that imports `loftHost.ws_*`).
- **One-process deployment.**  `loft hello_server.loft` (or its
  v3 equivalent) starts a single program that handles both
  asset serving and the game's WebSocket protocol.  No separate
  static-file server, no Express / nginx, no second process.

### Architecture changes

- **Server** routes by HTTP method + path:
  - `GET /` → respond with `index.html` (text content).
  - `GET /client.loft` → respond with loft source — the floor
    case (see § Architectural floor).  The client interpreter
    parses and runs it; no per-program WASM compile required.
  - `GET /client.wasm` → respond with optimised loft-compiled
    WASM bytes (binary content; lib/server gains a
    binary-respond primitive).  Optional optimisation layered
    on top once latency justifies the build step.
  - `GET /client.css` → respond with CSS text.
  - `GET /favicon.ico` → 404 (or a tiny default).
  - Any path with `Upgrade: websocket` header → WebSocket
    upgrade and game protocol from v2.
- **Client** loads the **loft interpreter WASM (already shipped
  as the loft runtime)** at page load.  The interpreter then
  parses and runs `client.loft` directly.  This is the floor
  configuration; minimal v3 ships only this.  An optimised
  configuration loads `client.wasm` once available and hands off
  state from the interpreter (V4-style mechanics).
- **`loft-rt.js`** host bridge gains the `ws_*` methods that
  wrap the browser's `WebSocket` (deferred from the hello-world
  milestone — this is where it actually has to land).  The
  WS-bridge work lives below the choice between source/WASM
  client.
- **Build/deploy step (only for the optimised path)**: a small
  script (or `loft --native-wasm` invoked at server startup)
  writes the WASM file to a known path the server reads on
  startup.  The HTML page is a fixed file the server can serve
  from disk or embed.  The minimal v3 skips this entirely and
  serves source.

### Wire IDs (unchanged from v2)

`#1` / `#2` (server → client own-game), `#3` (client → server
click), `#4` / `#5` (server → client spectator) — all carry
over.

### What v3 validates

- **The end-to-end browser flow** the architecture has been
  pointing toward: a player opens a URL, gets a game, plays it.
  No separate "install loft" step on the player's side.
- **Browser-side WebSocket bridge in `loft-rt.js`** — the
  wasm32 path of `lib/web`'s `ws_*` functions (currently stubs
  awaiting host imports) becomes load-bearing.
- **Binary HTTP responses** — the server gains the ability to
  send a `.wasm` payload, validating that the existing text-only
  `n_tcp_respond` is the right shape to extend.
- **One-process loft as a proper game/web server** — proves
  loft can be a complete deployment unit without external
  dependencies.

### Open questions / not-yet-decided for v3

- **Where the client WASM lives at server-start.**  Embedded in
  the server binary?  Read from disk relative to cwd?  The
  build/deploy story is the main scaffolding work for v3.
- **Asset hot-reload** — should asset edits be picked up
  without server restart?  Defer; minimal v3 is "read at
  startup."
- **TLS / HTTPS** — out of scope.  HTTP only; localhost only.
  Real deployment with TLS is a far-later milestone.
- **Client-side caching headers** — the simplest correct thing
  is `Cache-Control: no-cache` on everything.  Defer optimal
  caching.

### Out of scope for v3

- Multiple servers / horizontal scaling.
- Separate CDN for static assets.
- Authentication, sessions, cookies.
- Server-rendered HTML (the page is static; everything dynamic
  happens client-side via WASM + WebSocket).

## Tic-tac-toe v4 — client-uploaded scripts, server-side compile, hot WASM swap

The longer-term target.  A real game-development workflow where
the **client uploads loft source to the server**, the server
**compiles + optimises it** with the loft toolchain, the server
**returns the new WASM** to the client, and the client **swaps in
the new code without losing connection state**.

This is the workflow that makes loft useful as a *game
development environment*, not just a runtime: edit code in the
browser, see it running in the same browser within seconds,
without rebuilding+redeploying through a separate CI loop.

### What v4 adds (over v3)

- **Client-side editor** — a text area or richer editor in the
  page that lets the player (in dev mode) edit a loft snippet
  (e.g., the AI strategy for the server's tic-tac-toe player,
  or a custom client behaviour).
- **Upload protocol** — a new handler name, e.g. `ScriptUpload`
  → the client posts source text to the server.
- **Server-side compile** — the server invokes the loft
  compiler (`loft --native-wasm` or equivalent in-process API)
  on the uploaded source.  Diagnostics are returned via another
  handler, e.g. `Diagnostics`.
- **WASM hot-swap** — on success, the server sends the compiled
  WASM bytes back via a binary frame on the WebSocket
  (introduces the binary-payload path the EventLoop spec
  describes); the client loads the new WASM module without
  reloading the page or losing the connection.
- **Connection state preserved across hot-swap** — the client
  reconnects logically (same `client_id`) so the in-progress
  game continues on the new code.  Game state lives on the
  server; the WASM swap is on the client only.

### What v4 validates

- **Server-as-toolchain** — loft can host its own compiler as a
  service.  The server invokes `loft` programmatically and
  serves the result.  This is the fundamental capability that
  makes loft self-hosting in a deployment sense.
- **Binary frame on the WebSocket** — first concrete consumer
  of the binary-payload path the EventLoop spec describes.
  WASM bytes are the canonical use case for "the payload IS a
  bytes blob, no JSON wrapping."
- **Client-side WASM hot-swap mechanics** — `WebAssembly.compile`
  + `WebAssembly.instantiate` from a `Uint8Array` on the JS
  side, replacing the active module.  Standard browser API but
  validates the runtime can rebind.
- **State separation** — game state on the server, code on the
  client; the swap touches only the latter.  Architectural
  invariant for any later "live edit while playing" feature.
- **Round-trip development experience** — the player edits
  code, sees it running in seconds, in the same browser, via
  the same TCP connection.  No loft install on the dev's
  machine, no deploy step.

### Compile-latency strategy — interpreter floor + WASM hand-off

The naive concern with v4 is that `loft --native-wasm` takes
seconds; the dev experience demands < 2s round-trip on small
edits.  This is **already solved** by the architectural-floor
principle (see § The architectural floor — interpreter as
baseline): the interpreter always runs, so there's no "wait for
compile" gap to begin with.  The new code starts running
immediately on the interpreter; the optimised WASM swaps in
when ready.

The flow:

1. The dev edits a script in the browser editor.
2. The client **immediately** runs the new script through its
   on-board interpreter (slower, but starts in milliseconds —
   no compile wait).  The game keeps playing on interpreted
   code.
3. In parallel, the client posts the script to the server.
4. The server compiles to optimised WASM (seconds).
5. The server returns the compiled WASM bytes.
6. At the next frame boundary, the client's game-loop event
   handler **swaps the active execution from interpreter to
   WASM**.  The next frame runs natively-compiled.

The dev never waits for the compile.  The transition is
invisible at the gameplay layer: the only observable effect is
that frame-time drops once the swap completes.

The hand-off relies on the load-bearing invariant documented in
the floor section: shared `Stores` heap, byte-identical struct /
vector / hash layouts between interpreter and `--native-wasm`
output.  The swap rebinds which code reads the heap; it does not
migrate state.

#### Hand-off mechanics

The game-loop event handler is the orchestrator.  Loft programs
ALREADY have a frame-boundary structure (the `pump → render →
yield` sequence in the client's main loop).  At a frame
boundary, no opcode is mid-execution; no Rust-side I/O is
in-flight; the heap is in a consistent state.  This is the
correct point to switch.

When a `WasmReady` event arrives on the WebSocket:

1. Game loop finishes the current frame normally.
2. Before starting the next frame, the runtime invokes
   `WebAssembly.compile(bytes)` + `WebAssembly.instantiate(...)`
   to load the new module.
3. The new module is bound to the same `Stores` instance the
   interpreter was operating on (memory passed in as the WASM
   linear memory, or via shared host imports).
4. Subsequent frames call the WASM module's exported `step`
   function instead of the interpreter's.
5. The interpreter remains loaded in case a later script edit
   requires falling back again.

The event-handler shape doesn't change.  The CALLEE behind the
handler changes; the HANDLER is the same loft fn (registered at
program start), still receiving the same `Reference<GameState>`
captures, still mutating the same heap records.

#### Other open questions for v4

- **Sandboxing of uploaded code.**  In multiplayer, a malicious
  client must not be able to upload code that runs with full
  server privileges or affects other players' games.  The
  uploaded WASM compiles to client-side code only; the server
  is just a build service.  But the *compile step* runs on the
  server with whatever permissions the server has — this is a
  CPU/memory DoS vector that needs throttling and resource
  limits.
- **Server holds the compiler binary** — needs `loft` itself
  installed on the server, OR an embedded compile API exposed
  by the loft Rust crate.  The latter is cleaner for one-process
  deployment.
- **Diagnostics formatting on the client** — error messages
  with source positions need to highlight the right lines in
  the client's editor.  Reuse loft's existing diagnostic
  infrastructure but render in the editor.
- **Bundle size.**  Shipping the interpreter to the client
  doubles the WASM payload.  Acceptable in dev mode; in
  release builds, the interpreter ships only when hot-edit is
  enabled.
- **Interpreter / WASM divergence risk.**  Anything that
  changes the heap layout (new vector representation, new hash
  layout, alignment changes) must be applied to both code paths
  in lock-step.  Worth a CI check that asserts byte-identical
  layouts for a fixed test program.

### Out of scope for v4

- Real-time collaborative editing (one editor, one client at a
  time).
- Persistent project storage (the script is in-memory only;
  refresh = lose work).
- Version history / undo.
- Code review / peer approval flows.
- Production-grade sandbox / authentication.

### Sequencing relative to broader loft work

V4 depends on:
- Server-side ability to invoke the loft compiler in-process or
  as a subprocess.  Probably already feasible via `std::process`
  or by exposing a compile API in `libloft.rlib`.
- Binary frames in `lib/web` / `lib/server`'s WebSocket layer
  (currently text-only).  This is the EventLoop spec's
  `Encoding::Raw` path landing concretely.
- Client-side `loft-rt.js` extension to receive a binary payload
  from `loftHost.ws_recv` and pass it to `WebAssembly.compile`.

V4 is the milestone where **loft becomes a development
platform**, not just a language.  Keeping it documented here
means the architecture decisions in v1-v3 stay coherent with the
eventual target.

## Tic-tac-toe v5 — binary world stream + many clients + reconnect catch-up + sluggish tempo

**Status:** scoped 2026-05-10 to validate the wire-protocol
primitives [`plans/6-audience-generative-art/`](../6-audience-generative-art)
needs.  Builds **before v4** in the practical sequence — v4
also depends on binary WS frames; v5 lands them and v4
inherits.

The actual deliverable for v5 is **the smallest possible
text-mode test programs** that exercise each primitive.  Visual
tic-tac-toe boards remain off-limits (per the v1 protocol-
validation-only framing); the test program for v5 is a
multi-client world-bytes streamer that proves the primitives
work on the wire.

### Shared world data model — TTT board mirrors @PLN6

The v5 test programs use the **same `World` / `Chunk` / `Cell`
structures as @PLN6's audience-generative-art demo**.  TTT's
3×3 board sits at **origin (0, 0) of chunk (0, 0)** — cells
(0, 0) through (2, 2) of the world; resolved 2026-05-10 for
test-program simplicity.  Per-cell payload is the same 4 bytes
(1 byte colour + 1 byte height + 2 bytes age).  Colour values
are reinterpreted for TTT semantics:

| Colour byte | TTT meaning | @PLN6 meaning |
|---|---|---|
| 0 | empty cell | empty hex |
| 1 | X | red |
| 2 | O | green |
| 3-9 | unused | other palette colours |

`c_height` and `c_age` are present in the TTT cell but TTT can
leave them at default (0) — the binary serialiser still writes
them, the binary deserialiser still reads them, the wire format
is byte-identical.  This means primitives developed against the
TTT test programs **translate to @PLN6 with zero protocol
glue**: same struct definitions, same chunk addressing
(`chunk_idx_32` / `hex_idx_32`), same blob-pack/unpack code,
same session-tag header, same catch-up event shape.

The TTT board reuses the demo's data model; the demo reuses
TTT's proven wire infrastructure.

### What v5 adds (over v3 / v4)

Five new wire-protocol capabilities, each validated by its own
text-mode test:

1. **Binary WebSocket frames** in `lib/server` + `lib/web`.
   Currently both libraries route text frames only.  v5 extends
   `ws_send` / `ws_recv` (and the server-side equivalents) to
   send and receive raw byte arrays alongside the existing text
   path.
2. **Session-tagged binary blobs.**  Server can emit multiple
   binary frames sharing a session id.  Client buffers blobs
   by session id and applies them as a unit, so a single
   logical update that spans multiple chunks renders coherently
   regardless of packet ordering.

   **Blob header (resolved 2026-05-10): 5 bytes** —
   `[type:u8] [session:u32] [...payload...]`.  The type byte
   distinguishes snapshot / delta / control (256 frame types
   available); the u32 session id covers 4 billion values
   (plenty for one server run — at 10 Hz that is ~13 years);
   payload length is the WebSocket frame length minus 5.
3. **N-client routing with active-player signalling.**  Server
   tracks 30+ concurrent connections, holds per-client state,
   and broadcasts world deltas + a periodic active-player
   signal to all subscribers.  v2's two-client cap is lifted
   and the routing pattern generalised.

   **Active-player signal cadence (resolved 2026-05-10): once
   per second**, steady heartbeat regardless of activity level.
   Client always has a fresh "where is someone painting"
   answer; cheap (1 small JSON event per second per client);
   matches the sluggish-by-design tempo (no rapid flash bursts
   on the audience client even during high-activity moments).
4. **Catch-up recovery on reconnect.**  Client tracks its
   last-applied session id; on reconnect (or detected gap in
   incoming session ids) it sends a JSON `catch_up` request
   carrying that id.  Server replies with either replayed
   missed deltas (if cached) or a fresh full-state snapshot,
   under a single new session id the client renders as one
   coherent update.

   **Server-side replay cache (resolved 2026-05-10): last 60
   seconds of sessions** — at the 10 Hz tick rate, ~600
   sessions in cache.  Covers brief network blips; longer
   disconnects fall back to a fresh snapshot.  Memory cost
   small; tunable constant at first prototype if rehearsal
   shows different need.
5. **Sluggish-by-design world timings.**  Server runs a 10 Hz
   tick that ages cells (state grows on placement events,
   decays after a 5-minute base lifetime extended by per-
   neighbour leases, removes after a 30-second decay window).
   Validates that the server's tick loop is stable for hours
   and that the timing constants produce the desired
   inverse-growth aesthetic — even though the v5 test program
   does not render anything 3D.

### Test programs

Each primitive ships with one text-mode test program small
enough to fit in a single `.loft` file.  All v5 test programs
live in **`tests/scripts/`** and run as part of the wrap suite
(`cargo test`), reusing the existing 80+ loft-regression test
infrastructure.  Status lines the programs print become the
assertion surface (compared against expected output via the
wrap framework).  CI catches regressions automatically.

| Test | What it proves |
|---|---|
| `t1_binary_ws.loft` | A 4-byte payload sent as a binary frame in one direction round-trips back unchanged through a binary echo handler.  Server side + client side both hit the new primitive |
| `t2_session_blobs.loft` | Server emits 5 binary blobs sharing session id 7 then a single blob with session id 8.  Client logs "applied session 7 (5 blobs)" then "applied session 8 (1 blob)" — proves grouping works and the new-session-id flush triggers |
| `t3_n_clients.loft` | Spawn 30 simulated clients all sending periodic input; server keeps all 30 alive for 5 minutes without dropping any; broadcasts the 1 Hz active-player signal that all 30 receive.  No hard server-side cap (resolved 2026-05-10); connection-rejection-at-cap is out of scope for v5 — handled in a later hardening pass if a real deployment ever overflows |
| `t4_catch_up.loft` | Client connects, applies sessions 1-5, then simulates a 30-second disconnect; on reconnect sends `{"type":"catch_up","last_session":5}`; server replies with replayed sessions 6-N or a fresh snapshot under a new session id; client logs that it caught up cleanly.  Reconnect identity is **stateless** (resolved 2026-05-10) — server treats catch_up as 'give me deltas since N' from whoever holds the WS now; reconnecting client gets a new player_id but the world is consistent |
| `t5_world_timings.loft` | Server runs a small synthetic world for 10 minutes at 10 Hz; logs cell counts at 1 / 5 / 6 / 11 / 12 minute marks; verifies isolated cells decay around the 5-minute mark + 30 s window, fully-surrounded cells survive past the 11-minute mark |

The text-mode programs print short status lines that a wrap-
suite test can grep to verify behaviour.  No visual output, no
3D mesh, no JSON-events-vs-binary-blobs UI overlay — pure wire-
protocol verification.

### What v5 validates

- **The complete wire-protocol surface @PLN6 needs.**  Plan-36
  (audience-generative-art demo) becomes a straightforward
  consumer of proven primitives.  No protocol research happens
  on the demo's critical path.
- **`lib/server` + `lib/web` binary-frame extension** —
  required by v4 also; v5 lands it first.
- **Multi-client server stability** — pre-v5, the largest
  validated configuration is v2's two-client cap.  v5 forces
  the routing pattern to handle 30+ and reveals any per-
  connection state leaks the smaller test missed.
- **Tick-loop time-keeping under load** — a 10 Hz tick running
  for 10+ minutes with 30 active clients exercises drift /
  catch-up / pause handling that no shorter test surfaces.

### Out of scope for v5

- Any 3D rendering.  v5 ships text-mode tests; the demo's
  renderer is @PLN6's phase 3 work, layered on top of v5's
  wire primitives.
- Edge / line classification, frost mesh, ridge-and-crevice
  tops — all renderer-side, all in @PLN6's scope.
- Visual UX (palette, movement zones, jump-to-active) —
  @PLN6's phase 0.
- HTTP asset serving — that stays v3's concern.
- Hot WASM swap — that stays v4's concern.

### Sequencing relative to broader loft work

v5 depends on:
- v1's `<id>:payload` text protocol (still used for `catch_up`
  and other JSON events alongside the new binary path).
- The lifecycle-explicit `lib/server` API v2 documented (v5
  forces v2's `ws_serve / pump` pattern to ship if it hasn't
  yet).
- No language features beyond what v1 already needed.

v5 unblocks:
- v4 (hot WASM swap) — inherits the binary-frame primitive
  rather than rebuilding it.
- Plan-36 phase 1 (server) and phase 0 (phone client binary
  decoder) — both consume v5's primitives directly.

v5 surfaced (and deferred to a sibling plan):
- A handful of `lib/server` polish gaps that t1-t5 hit but
  worked around (most visibly t4's inline `n_ws_send_binary`
  declaration because `srv.send_to` is text-only).  Captured in
  [@PLN41](../41-server-hardening/README.md) — the natural
  prereq layer for @PLN6 phase 1.  Independent of v6's
  closure-retrofit work; either plan can land first.

### Build order recommendation

Build v5 before v3 / v4 if the audience-demo deadline drives
priority.  The v3/v4 progression (asset serving → hot WASM
swap) is its own arc and can land in parallel or after.

## Tic-tac-toe v6 — ergonomic retrofit using writable closures

**Status:** scoped 2026-05-10.  Pure cleanup pass; no new
protocol or runtime capability.  Depends on
[@PLAN22 (mutable closures)](../finished/22-mutable-closures/README.md)
landing in the language.  **Explicitly NOT on the audience-
demo's critical path** — if @PLAN22 has not shipped by the
meetup talk, @PLN6 server uses `Reference<T>` exactly like
v5 does today, and the demo functions identically.

### What v6 adds

Nothing new on the wire, in the runtime, or in the test
coverage.  v6 is a **diff** against v5's server code:

- Drop the `Reference<T>` wrapping around the server's mutable
  captured state (`world`, `next_session_id`, `replay_cache`,
  `last_active_player`, `tick_counter`).
- Replace every `state.inner.X` access with `state.X` (or just
  `X` if the binding shape is split per field).
- The pump callback (`srv.run(fn(ev: WsEvent) { … })`) reads
  10-20% shorter and reads as the binding pattern any reader
  would expect from the loft snippet without knowing the
  C38 history.

### Why v6 (and not just "we'll clean up later")

The audience-generative-art demo (@PLN6) projects loft code
on screen during the "loft snippet highlights" beats.  Visible
code structure is part of the talk's value proposition (art
show with loft footnotes) — `state.inner.X` clutter on the
projector reads as "see this language has rough edges" rather
than "see how compact this is."  v6 retrofits before the talk
specifically so the projected snippets are clean.

If v6 doesn't land before the talk, the snippets show
`Reference<T>.inner` and the talk works around it ("here's a
small ceremony loft uses today; here's the spec for the
ergonomic version landing soon").  Demo function unaffected;
talk loses some sales-pitch sharpness.

### Test coverage

v5's t1-t5 stays the assertion surface.  v6 is a pure
refactor; if any of t1-t5 changes behaviour, the retrofit
introduced a bug and gets reverted.

### Sequencing relative to broader loft work

v6 depends on:
- @PLAN22 (mutable closures) — SHIPPED 2026-05-13.
  See `plans/finished/22-mutable-closures/`.

v6 unblocks:
- @PLN6 server gets the same retrofit applied automatically
  (the two servers share the captured-state pattern, so the v6
  diff translates 1:1 to @PLN6 phase 1).

### Out of scope for v6

- Any new protocol capability (those land in v5 or future vN).
- Mutable-closure work in @PLN6's *renderer* (the projector
  + desktop client renderers are stateless per frame; the
  capture pattern doesn't apply there).
- Mutable-closure work in any earlier vN's reference code (v2's
  pump callback stays as-is in the design; only v5's server
  pattern gets retrofitted, and only if @PLAN22 lands in time).

---

## Cross-references

- [EVENT_LOOP.md](../32-event-loop/README.md) — eventual wire format and
  handler-id model.  Tic-tac-toe is its smallest validating
  game.
- [EVENT_LOOP_DISCUSSION.md](../32-event-loop/DISCUSSION.md) — open
  questions on the wider design.
- [plans/finished/22-mutable-closures/README.md](../finished/22-mutable-closures/README.md) — closure-capture
  spec (shipped 2026-05-13); the dispatch workaround in this
  game's pump callback rests on the documented `Reference<T>`
  capture pattern.  TTT v6 is the in-game consumer.
- [PROBLEMS.md § 213](../../PROBLEMS.md#213-typefunction-storage-layout-limit--full-design-for-the-proper-fix)
  — closure-in-struct-field layout limit; lifts the workaround
  once landed.
- `lib/web/src/web.loft` — handler-style WebSocket client API.
- `lib/server/src/server.loft` — WebSocket server API.
- `lib/game_protocol/examples/hello_*.loft` — the hello-world
  predecessor; same patterns at smaller scale.
