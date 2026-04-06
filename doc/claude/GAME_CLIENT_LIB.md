
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Game Client Library

Design for `game_client` — a client-side library for multi-player loft programs
that provides WebSocket connectivity, typed game message protocol, client-side
prediction and server reconciliation, lobby management, and dynamic loading of
loft programs compiled to WASM as hot-swappable game scripts.

The companion server-side library is `server` ([WEB_SERVER_LIB.md](WEB_SERVER_LIB.md)).

---

## Contents
- [Goals](#goals)
- [What is in loft vs native](#what-is-in-loft-vs-native)
- [Package structure](#package-structure)
- [WebSocket client](#websocket-client)
- [Game message protocol](#game-message-protocol)
- [Lobby and matchmaking](#lobby-and-matchmaking)
- [Game loop](#game-loop)
- [Client-side prediction and reconciliation](#client-side-prediction-and-reconciliation)
- [State synchronization](#state-synchronization)
- [Latency and ping](#latency-and-ping)
- [WASM script loading](#wasm-script-loading)
- [Shared game logic pattern](#shared-game-logic-pattern)
- [Security model](#security-model)
- [Targets: browser and native](#targets-browser-and-native)
- [Complete example](#complete-example)
- [Native layer boundary](#native-layer-boundary)
- [Implementation phases](#implementation-phases)
- [Dependencies](#dependencies)

---

## Goals

1. A loft program compiled to WASM can connect to a `server`-based game
   backend via WebSocket and exchange typed game messages.
2. A native loft client (desktop, CLI) connects identically — the API is the
   same on both targets.
3. Client-side prediction is built in: the player sees immediate response to
   their own inputs while waiting for server confirmation.
4. Game behavior modules (physics, AI, rules) are compiled separately to WASM
   and loaded at runtime — enabling mods, hot-reload, and guaranteed
   client-server determinism when both sides run the same WASM module.
5. Lobby and matchmaking state management is handled in loft so game code
   never deals with raw WebSocket messages.

---

## What is in loft vs native

### Implemented in loft

- All message type definitions (`GameEnvelope`, `GameMessage` enum)
- Message serialization to JSON (using `":j"` format) and deserialization
  (`Type.parse()`)
- Lobby state management (player list, ready state, countdown)
- Client-side prediction buffer (input history, tick tracking)
- Server reconciliation logic (replay inputs after confirmed state)
- Delta compression diffing (compute and apply state deltas)
- Ping measurement (round-trip time tracking)
- Game loop fixed-timestep logic and interpolation factor computation
- WASM script interface contract (expected export names, calling convention)
- Authentication handshake sequence
- Chat message handling

### Native (unavoidable)

- WebSocket connection to a remote URL (`tokio-tungstenite` on native,
  `web_sys::WebSocket` on WASM target)
- Non-blocking message poll (integrates with the platform event loop)
- WASM module instantiation and host function binding (`wasmtime` on native,
  browser `WebAssembly` API on WASM target)
- High-resolution timer for game loop (`std::time::Instant` on native,
  `performance.now()` in browser)
- `requestAnimationFrame` integration for render loop (WASM target only)
- Binary message framing (raw bytes without base64 overhead, for hot paths)
- WASM module signature verification (Ed25519)

---

## Package structure

```
game_client/                    ← GitHub: jjstwerff/loft-game-client
  loft.toml
  src/
    client.loft                 ← WsClient type, connect/send/receive/poll
    protocol.loft               ← GameEnvelope, GameMessage, serialization
    lobby.loft                  ← Lobby, PlayerInfo, LobbyState
    game_loop.loft              ← GameLoop, fixed-timestep, interpolation
    prediction.loft             ← PredictionBuffer, InputRecord, reconcile
    sync.loft                   ← StateSync, delta apply, snapshot management
    ping.loft                   ← PingTracker, rtt_ms, jitter_ms
    wasm_loader.loft            ← WasmModule, wasm_load, wasm_call
    auth.loft                   ← connect_authenticated, token refresh
  native/
    libloft_game_client.so      ← Linux
    libloft_game_client.dylib   ← macOS
    loft_game_client.dll        ← Windows
    loft_game_client.wasm       ← browser target (web-sys bindings)
  tests/
    protocol.loft               ← serialization round-trips
    prediction.loft             ← prediction + reconciliation unit tests
    sync.loft                   ← delta apply tests
    ping.loft
    wasm_loader.loft            ← load a test wasm module, call exports
```

---

## WebSocket client

```loft
struct WsClient { /* opaque */ }

enum WsState {
    Connecting,
    Open,
    Closing,
    Closed { code: integer, reason: text },
    Failed { reason: text },
}

enum WsMessage {
    Text   { content: text },
    Binary { data: text },      // raw bytes as base64
    Ping,
    Pong,
    Close  { code: integer, reason: text },
}

// Connect to a WebSocket server.
// Blocks until the handshake completes or fails.
// url: "ws://host:port/path" or "wss://host:port/path"
pub fn ws_connect(url: text) -> WsClient

// Connect with a JWT Bearer token in the Upgrade request headers.
pub fn ws_connect_auth(url: text, token: text) -> WsClient

// True if the connection is open.
pub fn ws_is_open(client: WsClient) -> boolean

// Send a text message.  Blocks until the frame is written.
pub fn ws_send(client: &WsClient, msg: text)

// Send raw binary.  data is base64-encoded bytes.
pub fn ws_send_binary(client: &WsClient, data: text)

// Wait for the next message.  Blocks.
pub fn ws_receive(client: &WsClient) -> WsMessage

// Non-blocking poll.  Returns null if no message is available.
// Used in the game loop to drain the queue without blocking the render tick.
pub fn ws_poll(client: &WsClient) -> WsMessage

// Current connection state.
pub fn ws_state(client: &WsClient) -> WsState

// Close with an optional reason.
pub fn ws_close(client: &WsClient, code: integer, reason: text)
```

### Reconnection (in loft)

`ws_connect` does not retry automatically.  Reconnection with back-off is
written in loft so application code controls the strategy:

```loft
pub fn ws_connect_retry(url: text, max_attempts: integer) -> WsClient {
    attempt = 0;
    for _ in 0..max_attempts {
        client = ws_connect(url);
        if ws_is_open(client) { return client; }
        attempt += 1;
        delay = min(30, 1 << min(attempt, 5));   // exponential back-off, cap 30 s
        sleep_ms(delay * 1000);
    }
    null   // caller handles failure
}
```

---

## Game message protocol

All messages use a typed envelope so the dispatcher can switch on `type_id`
without parsing the payload first.

```loft
struct GameEnvelope {
    type_id: integer,       // discriminates which GameMessage variant
    seq:     integer,       // monotonically increasing per sender
    tick:    integer,       // server game tick this message relates to
    payload: text,          // JSON-encoded GameMessage variant fields
}

// Serialize an envelope for sending:
pub fn envelope_encode(env: GameEnvelope) -> text     // returns JSON
// Deserialize a received JSON text into an envelope:
pub fn envelope_decode(raw: text) -> GameEnvelope     // null on parse error

// Type ID constants (integer discriminators):
MSG_HELLO         = 1;
MSG_WELCOME       = 2;
MSG_JOIN_LOBBY    = 3;
MSG_LOBBY_STATE   = 4;
MSG_PLAYER_JOINED = 5;
MSG_PLAYER_LEFT   = 6;
MSG_READY         = 7;
MSG_GAME_START    = 8;
MSG_GAME_OVER     = 9;
MSG_INPUT         = 10;
MSG_STATE_FULL    = 11;
MSG_STATE_DELTA   = 12;
MSG_PING          = 13;
MSG_PONG          = 14;
MSG_CHAT          = 15;
MSG_LOAD_WASM     = 16;    // server pushes a WASM module URL to the client
MSG_ERROR         = 99;
```

### Typed message structs (serialized as payload)

```loft
// Connection handshake
struct MsgHello   { version: integer, player_id: text, auth_token: text }
struct MsgWelcome { player_id: text, tick_rate: integer, server_tick: integer }

// Lobby
struct MsgJoinLobby   { lobby_id: text, player_name: text }
struct MsgLobbyState  { lobby_id: text, players: vector<PlayerInfo>,
                        max_players: integer, host_id: text,
                        state: text }     // LobbyState serialized as text
struct MsgPlayerJoined { player: PlayerInfo }
struct MsgPlayerLeft   { player_id: text }
struct MsgReady        { ready: boolean }

// Game lifecycle
struct MsgGameStart { tick: integer, seed: long, initial_state: text }
struct MsgGameOver  { winner_id: text, tick: integer, scores: text }

// In-game
struct MsgInput      { tick: integer, action: text }   // action is game-defined JSON
struct MsgStateFull  { tick: integer, state: text }
struct MsgStateDelta { from_tick: integer, to_tick: integer, delta: text }

// Utility
struct MsgPing     { client_time: long }               // ticks() microseconds
struct MsgPong     { client_time: long, server_time: long }
struct MsgChat     { player_id: text, message: text }

// WASM loading
struct MsgLoadWasm { module_id: text, url: text, signature: text }

struct MsgError    { code: integer, message: text }
```

### Dispatcher (in loft)

```loft
// Process one envelope received from the server.
// Calls the appropriate handler from the Dispatcher struct.
pub fn dispatch(env: GameEnvelope, d: &Dispatcher)

struct Dispatcher {
    on_welcome:      fn(MsgWelcome),
    on_lobby_state:  fn(MsgLobbyState),
    on_player_joined: fn(MsgPlayerJoined),
    on_player_left:  fn(MsgPlayerLeft),
    on_game_start:   fn(MsgGameStart),
    on_game_over:    fn(MsgGameOver),
    on_state_full:   fn(MsgStateFull),
    on_state_delta:  fn(MsgStateDelta),
    on_pong:         fn(MsgPong),
    on_chat:         fn(MsgChat),
    on_load_wasm:    fn(MsgLoadWasm),
    on_error:        fn(MsgError),
}
```

---

## Lobby and matchmaking

```loft
struct PlayerInfo {
    id:    text,
    name:  text,
    ready: boolean,
    ping:  integer,     // current RTT in milliseconds
}

struct Lobby {
    id:          text,
    players:     vector<PlayerInfo>,
    max_players: integer,
    host_id:     text,
    state:       LobbyState,
}

enum LobbyState {
    Waiting,
    Countdown { seconds_remaining: integer },
    InGame    { current_tick: integer },
    Finished  { winner_id: text },
}

// High-level lobby actions (serialize to GameEnvelope and send):
pub fn lobby_join(client: &WsClient, lobby_id: text, name: text)
pub fn lobby_set_ready(client: &WsClient, ready: boolean)
pub fn lobby_send_chat(client: &WsClient, message: text)

// Apply a received MsgLobbyState to a local Lobby struct:
pub fn lobby_apply_state(lobby: &Lobby, msg: MsgLobbyState)
pub fn lobby_apply_joined(lobby: &Lobby, msg: MsgPlayerJoined)
pub fn lobby_apply_left(lobby: &Lobby, msg: MsgPlayerLeft)

// Computed lobby helpers (implemented in loft):
pub fn lobby_ready_count(lobby: Lobby) -> integer
pub fn lobby_all_ready(lobby: Lobby) -> boolean
pub fn lobby_is_host(lobby: Lobby, player_id: text) -> boolean
pub fn lobby_find_player(lobby: Lobby, player_id: text) -> PlayerInfo
```

---

## Game loop

The game loop runs at a fixed tick rate (for simulation) and as fast as
possible (for rendering), using interpolation to smooth visual output between
simulation steps.  This prevents spiral-of-death when frames take too long.

```loft
struct GameLoop {
    tick_rate:      integer,    // simulation ticks per second (e.g. 20 or 60)
    max_frame_time: float,      // cap accumulated time (seconds) to prevent spiral
}

// Handler function types:
// update: advance simulation by exactly one tick.
//         dt = fixed_dt = 1.0 / tick_rate (constant).
// render: draw current frame.
//         alpha = interpolation factor 0.0..1.0 between last two ticks.
//
// The loop runs until the process exits or stop_loop() is called.
pub fn run_loop(
    loop:    GameLoop,
    update:  fn(tick: integer),
    render:  fn(alpha: float),
)

// Signal the loop to exit after the current frame.
pub fn stop_loop()

// Current monotonic time in microseconds (wraps native high-res clock).
pub fn now_us() -> long
```

### Loop internals (implemented in loft)

```loft
// Pseudocode of the fixed-timestep loop (what run_loop does internally):

fn run_loop(loop: GameLoop, update: fn(integer), render: fn(float)) {
    fixed_dt = 1.0 / loop.tick_rate as float;
    accumulator = 0.0;
    tick = 0;
    last_time = now_us();
    running = true;

    for _ in 0..MAX_INT if running {
        current_time = now_us();
        frame_time = (current_time - last_time) as float / 1000000.0;
        last_time = current_time;

        // Cap to prevent spiral of death after long pauses
        if frame_time > loop.max_frame_time {
            frame_time = loop.max_frame_time;
        }
        accumulator += frame_time;

        // Drain the network queue without blocking
        // (game code calls ws_poll inside update)

        // Fixed-rate simulation steps
        for _ in 0..loop.tick_rate if accumulator >= fixed_dt {
            update(tick);
            tick += 1;
            accumulator -= fixed_dt;
        }

        // Render with interpolation
        render(accumulator / fixed_dt);

        // Platform yield (requestAnimationFrame on WASM, sleep(0) on native)
        yield_frame();
    }
}
```

---

## Client-side prediction and reconciliation

Client-side prediction lets the player see immediate response to their own
inputs without waiting for a server round-trip.  When the server sends a
confirmed authoritative state, the client replays any unconfirmed inputs on
top of it.

```loft
struct InputRecord {
    tick:  integer,
    action: text,           // JSON action blob (game-defined)
}

struct PredictionBuffer {
    inputs:                vector<InputRecord>,
    last_confirmed_tick:   integer,
    last_confirmed_state:  text,    // last authoritative state from server
}

// Record a player action and apply it locally.
// apply: fn(state: text, action: text) -> text
//   Takes the current game state JSON and an action JSON, returns new state JSON.
//   This is the game's own update function — or the shared WASM module.
pub fn predict(
    buf:    &PredictionBuffer,
    tick:   integer,
    action: text,
    state:  &text,         // current predicted state (mutated in place)
    apply:  fn(text, text) -> text,
)

// Server has confirmed state at a given tick.
// Discard older inputs, reapply remaining inputs on top of confirmed state.
// Returns the corrected current state.
pub fn reconcile(
    buf:             &PredictionBuffer,
    confirmed_tick:  integer,
    confirmed_state: text,
    apply:           fn(text, text) -> text,
) -> text

// Discard all inputs older than or equal to confirmed_tick.
pub fn prediction_prune(buf: &PredictionBuffer, confirmed_tick: integer)
```

### Reconciliation algorithm (in loft)

```loft
fn reconcile(buf: &PredictionBuffer, confirmed_tick: integer,
             confirmed_state: text, apply: fn(text, text) -> text) -> text {
    buf.last_confirmed_tick = confirmed_tick;
    buf.last_confirmed_state = confirmed_state;

    // Remove inputs that the server has already accounted for
    prediction_prune(buf, confirmed_tick);

    // Replay remaining unconfirmed inputs on top of confirmed state
    state = confirmed_state;
    for rec in buf.inputs {
        state = apply(state, rec.action);
    }
    state
}
```

### Divergence detection

```loft
// Compare predicted state with confirmed state.
// Returns a divergence score: 0.0 = identical, > 0 = diverged.
// Implemented in loft by comparing JSON field by field.
pub fn divergence(predicted: text, confirmed: text) -> float

// Log a warning if divergence exceeds threshold.
pub fn check_divergence(predicted: text, confirmed: text, threshold: float)
```

---

## State synchronization

The server can send either full state snapshots or incremental deltas.
Delta compression reduces bandwidth significantly for large game states.

```loft
struct StateSync {
    current_tick:  integer,
    current_state: text,    // JSON game state
    snapshots:     vector<Snapshot>,   // recent states for delta reference
}

struct Snapshot {
    tick:  integer,
    state: text,
}

// Apply a full state update (replaces current_state).
pub fn sync_apply_full(sync: &StateSync, msg: MsgStateFull)

// Apply a delta to produce a new current state.
// Delta format is game-defined JSON describing field changes.
pub fn sync_apply_delta(sync: &StateSync, msg: MsgStateDelta,
                        apply_delta: fn(text, text) -> text)

// Keep a rolling window of snapshots for delta reference.
// Old snapshots (older than window_ticks) are pruned automatically.
pub fn sync_keep_snapshot(sync: &StateSync, tick: integer, window_ticks: integer)

// Compute a delta between two states (for the server-side mirror of this API).
pub fn compute_delta(from_state: text, to_state: text) -> text
```

---

## Latency and ping

```loft
struct PingTracker {
    pending:    vector<PingRecord>,
    rtt_ms:     float,          // exponential moving average
    jitter_ms:  float,          // standard deviation of RTT
    lost:       integer,        // pings sent with no pong received
}

struct PingRecord {
    seq:      integer,
    sent_at:  long,    // now_us() when ping was sent
}

// Send a ping and record it.
pub fn ping_send(client: &WsClient, tracker: &PingTracker)

// Process a received pong.  Updates rtt_ms and jitter_ms.
pub fn ping_receive(tracker: &PingTracker, msg: MsgPong)

// Prune ping records older than timeout_us microseconds (mark as lost).
pub fn ping_prune(tracker: &PingTracker, timeout_us: long)

// Returns a human-readable latency summary.
pub fn ping_summary(tracker: PingTracker) -> text
// e.g. "RTT: 42 ms  jitter: 3 ms  lost: 0"
```

---

## WASM script loading

Game scripts are loft programs compiled to `.wasm`.  Loading them at
runtime enables:

- **Mods** — users supply custom game logic without recompiling the client.
- **Hot reload** — update game behavior in development without restarting.
- **Determinism** — client and server run the same WASM module for physics
  and game rules, guaranteeing identical results and minimising corrections.

```loft
struct WasmModule { /* opaque — backed by wasmtime or browser WebAssembly */ }

// Load from a base64-encoded byte string (for embedded modules or
// modules received over the network).
pub fn wasm_load_bytes(bytes: text) -> WasmModule

// Download and load from a URL.
// Blocks until the download and instantiation complete.
pub fn wasm_load_url(url: text) -> WasmModule

// Call an exported function.
// args and return value are JSON strings for type safety across the boundary.
pub fn wasm_call(module: &WasmModule, func: text, args: text) -> text

// True if the module exports a function with this name.
pub fn wasm_has_export(module: &WasmModule, name: text) -> boolean

// Release the module instance.
pub fn wasm_unload(module: &WasmModule)

// Verify a module's Ed25519 signature before loading.
// public_key: base64url-encoded public key.
// signature: base64url-encoded signature from MsgLoadWasm.
// Returns true if the module bytes match the signature.
pub fn wasm_verify(bytes: text, public_key: text, signature: text) -> boolean
```

### Standard game script interface

A loft program compiled to WASM and used as a game script is expected to
export these functions.  The host (client or server) calls them through the
`wasm_call` API.

```loft
// The game script module exports these names (compiled loft function names):
//   n_script_init(config: text) -> text
//     Called once after loading.  config is game-defined JSON.
//     Returns the initial script-managed state as JSON.
//
//   n_script_update(state: text, input: text) -> text
//     Called each simulation tick.  input is the player action JSON.
//     Returns the new state JSON.
//
//   n_script_render_hint(state: text) -> text
//     Optional.  Returns a JSON object with rendering hints
//     (position, sprite, animation frame, etc.) for the client renderer.
//
//   n_script_is_terminal(state: text) -> text
//     Returns "true" or "false" — whether the game is over.
//
//   n_script_score(state: text, player_id: text) -> text
//     Returns the score for a given player as a JSON number.

// Wrapper that calls n_script_update through wasm_call:
pub fn script_update(module: &WasmModule, state: text, input: text) -> text {
    wasm_call(module, "n_script_update",
              "{\"state\":{state},\"input\":{input}}")
}
```

### Receiving a WASM module from the server

When the server sends `MsgLoadWasm`, the client downloads and verifies the
module before loading it:

```loft
fn handle_load_wasm(msg: MsgLoadWasm, trusted_key: text) {
    bytes = http_get(msg.url).body;     // from the web package
    if !wasm_verify(bytes, trusted_key, msg.signature) {
        log_warn("WASM module {msg.module_id} failed signature check — not loaded");
        return;
    }
    module = wasm_load_bytes(bytes);
    register_module(msg.module_id, module);
}
```

---

## Shared game logic pattern

The most powerful use of WASM script loading is when the client and server
both run the same compiled module for game physics and rules.

```
            ┌─────────────────────────────┐
            │  rules.loft                 │  shared source
            └─────────────┬───────────────┘
                          │ compile
              ┌───────────┴───────────┐
              │                       │
    ┌─────────▼──────────┐   ┌────────▼──────────┐
    │  rules.wasm        │   │  rules.wasm        │
    │  (server side)     │   │  (client side)     │
    │  authoritative     │   │  prediction         │
    └────────────────────┘   └────────────────────┘
```

**How it works:**

1. `rules.loft` is the game's canonical simulation: physics step, collision,
   scoring.  It exports `n_script_update(state, input) -> state`.
2. The server loads `rules.wasm` via `wasm_load_url` at startup.
3. On `GameStart`, the server includes the module URL and signature in
   `MsgLoadWasm`.
4. The client verifies and loads the same module.
5. On each tick, the client calls `script_update(rules_module, state, input)`
   for prediction.  The server calls the same function authoritatively.
6. Server corrections (`MsgStateDelta`) become rare because both sides are
   running identical logic.

**Result:** the client predicts perfectly unless randomness, network reorder,
or anti-cheat corrections cause divergence.  Corrections are small and infrequent.

---

## Security model

### Module signature verification

The server signs each WASM module with an Ed25519 private key held
server-side.  The client has the corresponding public key baked in (or
received during the authenticated handshake).

The native layer performs the Ed25519 verification.  The loft API exposes
it as `wasm_verify(bytes, public_key, signature) -> boolean`.

**The client must not load a module whose signature does not verify.**

### Sandboxing

WASM modules are inherently sandboxed: they cannot access the network,
filesystem, or native memory outside what the host exposes.  The host
(native loft layer) provides only these imports to game script modules:

| Import | Purpose |
|--------|---------|
| `loft::rand_u32() -> i32` | Random number source (seeded by server) |
| `loft::log(level: i32, msg: ptr, len: i32)` | Debug logging only |

No file I/O, no network access, no system calls.  Game scripts are
pure-computation sandboxes.

### Connection authentication

`ws_connect_auth` passes a JWT Bearer token in the WebSocket upgrade request
headers (the `Authorization` header).  The server validates it using the JWT
middleware from `server`.  The client receives its token from the login flow
(an ordinary HTTP POST handled by the HTTP client `web` package).

---

## Targets: browser and native

The library compiles to two targets with the same loft API.

### Browser (WASM target)

- WebSocket: uses `web_sys::WebSocket` via `wasm-bindgen`.
  `ws_poll` is non-blocking (checks the JS message queue).
- Game loop: `run_loop` calls `requestAnimationFrame` for the render step;
  the fixed-timestep accumulator runs as a JS callback.
- WASM loading: uses `WebAssembly.instantiateStreaming` (async, bridged to
  blocking in the loft API).
- Timer: `performance.now()` via `web_sys`.

### Native (desktop / server-side tooling)

- WebSocket: `tokio-tungstenite` (same crate as `server`).
  `ws_poll` uses a non-blocking channel read.
- Game loop: `std::time::Instant` for timing; `std::thread::sleep` for the
  render yield.
- WASM loading: `wasmtime` runtime — can load modules from disk or bytes.
- Timer: `std::time::Instant` with microsecond resolution.

The same loft source and the same compiled `.loft` files work on both targets.
Only the native layer changes.

---

## Complete example

A minimal two-player turn-based game (Tic-Tac-Toe) over WebSocket.

### Game script: `rules.loft` (compiled to `rules.wasm`, loaded by both sides)

```loft
// Rules compiled to WASM.  Exported by the loft native-codegen toolchain.

struct Board { cells: vector<text>, turn: text, winner: text }

pub fn n_script_init(config: text) -> text {
    board = Board {
        cells: ["", "", "", "", "", "", "", "", ""],
        turn: "X",
        winner: "",
    };
    "{board:j}"
}

pub fn n_script_update(state: text, input: text) -> text {
    board = Board.parse(state);
    action = Action.parse(input);     // Action { cell: integer }
    if board.winner != "" { return state; }
    if board.cells[action.cell] != "" { return state; }
    board.cells[action.cell] = board.turn;
    board.winner = check_winner(board.cells);
    board.turn = if board.turn == "X" { "Y" } else { "X" };
    "{board:j}"
}

pub fn n_script_is_terminal(state: text) -> text {
    board = Board.parse(state);
    if board.winner != "" { "true" } else { "false" }
}

pub fn n_script_score(state: text, player_id: text) -> text {
    board = Board.parse(state);
    if board.winner == player_id { "1" } else { "0" }
}
```

### Client: `game.loft` (compiled to WASM, runs in browser)

```loft
use game_client;
use graphics;   // for rendering

TRUSTED_KEY = "MCowBQYDK2VdAyEA...";   // server's Ed25519 public key (base64url)

board_state = "";
rules_module = null;
my_id = "";
client = null;
game_running = false;

fn on_welcome(msg: MsgWelcome) {
    my_id = msg.player_id;
}

fn on_game_start(msg: MsgGameStart) {
    board_state = msg.initial_state;
    game_running = true;
}

fn on_state_full(msg: MsgStateFull) {
    board_state = msg.state;
}

fn on_load_wasm(msg: MsgLoadWasm) {
    bytes = http_get(msg.url).body;
    if !wasm_verify(bytes, TRUSTED_KEY, msg.signature) {
        log_error("WASM verification failed");
        return;
    }
    rules_module = wasm_load_bytes(bytes);
}

fn on_game_over(msg: MsgGameOver) {
    game_running = false;
    if msg.winner_id == my_id {
        println("You win!");
    } else {
        println("You lose.");
    }
}

fn update(tick: integer) {
    // Drain network messages without blocking
    running = true;
    for _ in 0..100 if running {
        msg = ws_poll(client);
        if msg == null { running = false; return; }
        match msg {
            Text { content } => dispatch(envelope_decode(content), Dispatcher {
                on_welcome: fn on_welcome,
                on_game_start: fn on_game_start,
                on_state_full: fn on_state_full,
                on_load_wasm: fn on_load_wasm,
                on_game_over: fn on_game_over,
                // ... others: fn on_pong, fn on_chat, etc.
            }),
            _ => {},
        }
    }
}

fn render(alpha: float) {
    draw_board(board_state);
}

fn send_move(cell: integer) {
    if !game_running { return; }
    action = "{\"cell\":{cell}}";
    // Predict locally if rules module is loaded
    if rules_module != null {
        board_state = script_update(rules_module, board_state, action);
    }
    ws_send(client, envelope_encode(GameEnvelope {
        type_id: MSG_INPUT,
        seq: next_seq(),
        tick: 0,
        payload: "{MsgInput { tick: 0, action: action }:j}",
    }));
}

fn main() {
    client = ws_connect_auth("wss://game.example.com/ws/play", get_token());
    lobby_join(client, "room1", "Alice");

    run_loop(GameLoop { tick_rate: 20, max_frame_time: 0.25 },
             fn update, fn render)
}
```

---

## Native layer boundary

| Symbol | Purpose |
|--------|---------|
| `n_ws_connect` | Open WebSocket connection (blocking handshake) |
| `n_ws_send_text` | Write text frame |
| `n_ws_send_binary` | Write binary frame |
| `n_ws_receive` | Blocking receive |
| `n_ws_poll` | Non-blocking receive |
| `n_ws_close` | Send close frame |
| `n_ws_state` | Query connection state |
| `n_now_us` | High-resolution monotonic clock (microseconds) |
| `n_yield_frame` | Platform frame yield (`requestAnimationFrame` or sleep) |
| `n_wasm_load` | Instantiate a WASM module from bytes |
| `n_wasm_call` | Call an export function |
| `n_wasm_has_export` | Check for an export |
| `n_wasm_unload` | Release a module instance |
| `n_wasm_verify_ed25519` | Verify Ed25519 signature |
| `n_sleep_ms` | Blocking sleep (reconnect back-off) |

Everything above these 15 symbols is implemented in loft.

---

## Implementation phases

### Phase 1 — WebSocket client + protocol

- `n_ws_connect/send/receive/poll/close/state` in native
- `WsClient`, `WsMessage`, `WsState` in loft
- `GameEnvelope`, all `Msg*` structs, `envelope_encode/decode` in loft
- `Dispatcher` struct and `dispatch()` in loft
- `ws_connect_auth` in loft
- Tests: protocol serialization round-trips, dispatcher routing

### Phase 2 — Lobby + game loop

- `Lobby`, `PlayerInfo`, `LobbyState` in loft; all `lobby_*` functions
- `n_now_us`, `n_yield_frame` in native
- `GameLoop`, `run_loop` in loft (fixed-timestep accumulator)
- Tests: lobby state transitions, loop tick counting

### Phase 3 — Prediction + state sync

- `PredictionBuffer`, `InputRecord`, `predict`, `reconcile` in loft
- `StateSync`, `Snapshot`, `sync_apply_full/delta` in loft
- `compute_delta`, `divergence` in loft
- `PingTracker`, `ping_send/receive/prune` in loft
- Tests: reconcile replays inputs correctly; delta compression

### Phase 4 — WASM script loading

- `n_wasm_load/call/has_export/unload/verify` in native
  - Browser: `WebAssembly.instantiateStreaming` via `wasm-bindgen`
  - Native: `wasmtime::Engine` + `wasmtime::Module`
- `wasm_load_bytes/url/call/has_export/unload/verify` in loft
- `script_update/score/is_terminal` wrapper functions in loft
- `MsgLoadWasm` dispatch + `handle_load_wasm` pattern in loft
- Tests: load test WASM, call exports, verify rejects bad signature

### Phase 5 — Shared game logic

- `--native-wasm` integration in interpreter to compile a loft script
  exposing the `n_script_*` exports
- Documentation and example game (`rules.loft` → `rules.wasm`)
- End-to-end test: server and client load the same module; verify no
  divergence over 1000 ticks of deterministic input

---

## Dependencies

### Rust crates (native layer)

| Crate | Version | Purpose | Target |
|-------|---------|---------|--------|
| `tokio` | 1 | Async runtime | native |
| `tokio-tungstenite` | 0.24 | WebSocket client | native |
| `web-sys` | 0.3 | Browser WebSocket + WebAssembly API | wasm |
| `wasmtime` | 23 | WASM runtime | native |
| `ed25519-dalek` | 2 | Ed25519 signature verification | both |
| `base64` | 0.22 | Signature and module bytes encoding | both |

### Loft dependencies (`loft.toml`)

```toml
[dependencies]
web           = ">=0.1"     # HTTP client — used by wasm_load_url and wasm_verify flow
game_protocol = ">=0.1"    # shared GameEnvelope, Msg* structs, WsMessage enum
```

### Relationship to other libraries

| Library | Relationship |
|---------|-------------|
| `server` | Provides the WebSocket server side; see WEB_SERVER_LIB.md § Game server additions for required server extensions |
| `game_protocol` | Shared package — canonical `GameEnvelope`, `Msg*`, `WsMessage`; both `server` and `game_client` depend on it |
| `web` | HTTP client used by `wasm_load_url` to download module bytes |
| `graphics` | Used by the game rendering layer (not a direct dependency of `game_client` — the game imports it separately) |
