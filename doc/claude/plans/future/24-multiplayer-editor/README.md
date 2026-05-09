<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# MULTIPLAYER_EDITOR — minimal multi-client hex editor (real-game milestone)

**Status:** plan, not yet built.  This is the first **real,
playable** loft program with multiplayer — the moros editor
talking to a real loft server, with two clients editing one
shared world.

This **consumes** the protocol ground layers laid down in
[TIC_TAC_TOE.md](../32-tic-tac-toe/README.md), it does not replace them:

- TIC_TAC_TOE v1 (shipped) → server-arbited handshake +
  namespace registry + `<id>:<payload>` text frames.  This
  editor reuses the same protocol shape.
- TIC_TAC_TOE v2 (next ground layer) → multi-client server
  primitives (non-blocking accept, per-client poll, broadcast).
  This editor's Step 1 IS the v2 ground layer; the editor and
  the v2 text-mode tic-tac-toe verifier share the same
  `lib/server` extension.
- TIC_TAC_TOE v3 (later ground layer) → asset serving + browser
  WS bridge.  When this lands, the editor gains a browser
  variant (the desktop variant runs unchanged).
- TIC_TAC_TOE v4 (later ground layer) → server-side compile +
  hot-swap.  When this lands, the editor's logic can be live-
  edited from inside its own browser session.

Tic-tac-toe verifies each ground layer's protocol works
text-mode; this editor is the first consumer that uses the same
infrastructure for actual gameplay UX (mouse, render, sound).

## Goal

A minimum-viable multi-client hex editor:

1. Empty plane on startup (existing `map_empty()` from `lib/moros_map`).
2. Mouse click on a hex paints it **bright red** (a new "painted"
   ground material that's distinct from the default gray empty
   terrain).
3. The local client applies the change *immediately* (optimistic).
4. The change is sent to the server in the background.
5. The server holds the master copy and broadcasts the change to all
   other connected clients.
6. A second client connecting later receives the current world state
   on connect (snapshot replay) and renders the same map; its camera
   moves independently; clicks made on either client appear on both.

The editor is otherwise unchanged: same camera, same render, same
input handling, same hex math.  The integration adds *one* new code
path (background WS sync) and *one* new visible behaviour (painted
hexes turn red and propagate).

---

## Existing pieces (reuse as-is)

| Piece | Where | Used for |
|---|---|---|
| `lib/web` ws_handler / try_recv / send | `lib/web/src/web.loft` | Client side of the WS connection |
| `lib/server` listen / next / ws_upgrade | `lib/server/src/server.loft` | Server side — needs **multi-client extension** (see § New pieces) |
| MAP handshake + namespace registry | TIC_TAC_TOE.md v1 pattern | Same shape: server arbitrates handler ids; client conforms |
| `lib/moros_map` Hex / HexAddress / Chunk | `lib/moros_map/src/types.loft` | Map data structure (server's master copy + each client's copy) |
| `map_paint_material(m, q, r, cy, mat)` | `lib/moros_map` | The mutation primitive both clients call locally |
| `lib/moros_render` build_hex_meshes / camera | `lib/moros_render/src/moros_render.loft` | Per-client render loop |
| `lib/graphics` mouse polling + render frame | `lib/graphics/src/graphics.loft` | Per-client input + draw |
| `lib/moros_editor` mouse-to-hex picking | `lib/moros_editor/src/moros_editor.loft` | The "which hex did I click" logic |

---

## New pieces

| Piece | Why | Where |
|---|---|---|
| **Multi-client WS server primitives** | Today's `lib/server` accepts one client at a time, sequentially.  The editor demo needs ≥2 simultaneous clients. | Extend `lib/server/native/src/lib.rs` and `lib/server/src/server.loft` |
| **Server program** | Holds master world + accepts/broadcasts edits | `lib/moros_editor/examples/edit_server.loft` (new) |
| **Client program** | Editor wired with WS sync | `lib/moros_editor/examples/edit_client.loft` (new) |
| **"Painted" material** | The bright-red ground material the click produces | Added to the demo program's palette setup |

No new design docs.  No new docstrings.  No new compiler work.  All
on-shipped pieces.

---

## Wire protocol

Same shape as tic-tac-toe v1 — text frames, integer-id prefix,
server-arbited MAP handshake.  Three handlers:

| Name | Direction | Payload |
|---|---|---|
| `Snapshot` | server → client (once on connect, after MAP) | `q1,r1,cy1,mat1;q2,r2,cy2,mat2;…` (semicolon-separated cell list; empty payload = empty world) |
| `HexEdit` | bidirectional | `q,r,cy,mat` (single cell change) |
| `Connect` | client → server (once after handshake) | empty (just signals "send me the snapshot") |

Frames after MAP are `<id>:<payload>`.  The client's flow:

1. Connect, parse MAP.
2. Send `Connect` to request the snapshot.
3. Receive `Snapshot`, apply all cells to local map.
4. Enter main loop.

Main-loop frame shape:
- On mouse click → compute hex coord → `map_paint_material(local, q, r, cy, painted_mat_idx)` → send `HexEdit:q,r,cy,mat`.
- Polling: drain inbound; for each `HexEdit:...` apply to local map.

Server main loop:
- Accept new connections (non-blocking).  On accept, send MAP, then send Snapshot.
- For each connected client, drain inbound.  On `HexEdit:...`, apply to master + broadcast to all *other* clients.

---

## Implementation steps

### Step 1 — Multi-client server primitives in `lib/server` (= TIC_TAC_TOE v2 ground layer)

**Goal**: `lib/server` can hold many simultaneous clients and poll
each non-blockingly.  This is shared work with the **TIC_TAC_TOE
v2 protocol-only ground layer** — the same `lib/server` extension
serves both the text-mode v2 tic-tac-toe verifier and this editor.
Land it once; both consumers benefit.

**Native (lib/server/native/src/lib.rs):**

- Replace single `CURRENT_CONN: Option<TcpStream>` with `WS_CONNS:
  Vec<Option<TcpStream>>` (already exists for the `n_ws_*` family
  but only after upgrade — extend to keep the listener non-blocking
  and per-client streams non-blocking with short timeouts).
- New C-ABI exports:
  - `n_tcp_accept_nonblocking(handle: i32) -> i32` — returns
    new client_id (≥0), or -1 if no pending connection.
  - `n_ws_recv_from(client_id: i32) -> bool` — try to read one
    frame from a specific client; non-blocking-ish (short read
    timeout).
  - `n_ws_send_to(client_id: i32, msg) -> bool` — send to specific
    client (already analogous to `n_ws_send`, but keyed on the
    new client_id namespace).
  - `n_ws_clients() -> length+pointer` — return list of active
    client ids; needed for the broadcast loop.
  - `n_ws_disconnect(client_id: i32)` — close one client.
- Set listener and per-client streams to `set_nonblocking(true)` /
  `set_read_timeout(Some(Duration::from_millis(20)))` so the loft
  program can poll without blocking.

**Loft (lib/server/src/server.loft):**

- New `WsServer { handle: integer }` struct.
- `pub fn ws_listen(port) -> WsServer`.
- `pub fn ws_accept(srv) -> integer` (returns client_id ≥ 0 or -1).
- `pub fn ws_poll(client_id) -> text` (returns next frame or null).
- `pub fn ws_send(client_id, msg) -> boolean`.
- `pub fn ws_broadcast(srv, msg, except: integer)` (loft-side helper
  iterating `ws_clients`).
- `pub fn ws_disconnect(client_id)`.

**Validation:** unit test in `lib/server/tests/`: spin up two
clients, exchange messages, verify each sees the other's traffic
without head-of-line blocking.

**Time:** 1-2 sessions of Rust + loft.

---

### Step 2 — Server program

**File:** `lib/moros_editor/examples/edit_server.loft` (new).

Holds the master world (a single `Map` from `lib/moros_map`).

```
fn main() {
    reg = Registry { next_id: 0 };
    h_snapshot = register(reg, "Snapshot");
    h_hex_edit = register(reg, "HexEdit");
    h_connect  = register(reg, "Connect");

    world = map_empty();
    add_painted_material(world);   // adds "painted" red material at index P

    srv = ws_listen(7878);
    while true {
        // Accept new clients
        new_id = ws_accept(srv);
        if new_id >= 0 {
            ws_send(new_id, "MAP:Snapshot={h_snapshot},HexEdit={h_hex_edit},Connect={h_connect}");
            // Wait for Connect, then send Snapshot
        }

        // Poll all clients
        for cid in ws_clients(srv) {
            raw = ws_poll(cid);
            if raw == null { continue; }
            id = parse_id(raw);
            body = parse_payload(raw);
            if id == h_connect {
                ws_send(cid, "{h_snapshot}:{encode_world(world)}");
            } else if id == h_hex_edit {
                apply_edit(world, body);
                ws_broadcast(srv, "{h_hex_edit}:{body}", cid);
            }
        }
    }
}
```

`encode_world(m)` walks the map's chunks and emits the
semicolon-separated cell list.  `apply_edit(world, "q,r,cy,mat")`
parses the four integers and calls `map_paint_material`.

**Validation:** run server, connect with `wscat`, manually send
`MAP:`, `Connect:`, observe `Snapshot:` reply.

**Time:** 1 session.

---

### Step 3 — Client integration

**File:** `lib/moros_editor/examples/edit_client.loft` (new), or extend
an existing example.

The CLIENT does:

1. Open WS connection.
2. Wait for MAP, populate registry.
3. Send `Connect`.
4. Wait for `Snapshot`, apply all cells to local map.
5. Enter main loop:
   - poll mouse → on click compute hex → paint local + send `HexEdit`.
   - poll WS → drain `HexEdit` from server-broadcast → apply to local map.
   - render frame.
6. On exit, close WS.

The mouse-click path replaces the editor's existing
"cycle-through-materials" or whatever is shipped with
"set to painted-red", limited to the painted-material index.

The render loop is unchanged; the renderer reads the local map.
Local mutation + applied broadcasts both flow through the same
`map_paint_material` primitive, so the renderer sees them
identically.

**Validation:** run server + one client.  Click hexes, verify they
turn red.  Server log shows `HexEdit` arriving.

**Time:** 1-2 sessions, mostly figuring out where to hook into the
existing editor's main loop.

---

### Step 4 — Two-client validation

Run server + two clients (separate processes / separate windows).

Validation checklist:

- [ ] Client A starts, sees empty plane.  Camera moves; nothing
      painted.
- [ ] Client A clicks a hex.  Hex turns red on A.  Server log
      shows `HexEdit` received.
- [ ] Client B starts (later).  Receives Snapshot containing
      A's edit.  Sees the red hex.
- [ ] Client B's camera position is independent of A's.
- [ ] Client B clicks a different hex.  Hex turns red on B.
      A's window shows the same hex turn red shortly after.
- [ ] Both clients see all edits made by both.
- [ ] Disconnect A → B keeps running, can still edit.  When A
      reconnects, the snapshot includes B's edits made while A
      was away.

**Time:** 1 session of running + debugging sync.

---

## Total time

5-6 sessions for the minimum-viable end-to-end demo, distributed:

| Step | Sessions |
|---|---|
| 1 — Multi-client server primitives (= TIC_TAC_TOE v2 ground layer) | 1-2 |
| 2 — Server program | 1 |
| 3 — Client integration | 1-2 |
| 4 — Two-client validation + debugging | 1 |

**Bonus deliverable from Step 1**: a text-mode tic-tac-toe v2
verifier — two text clients connecting to the same server, each
playing their own game while observing the other as a spectator.
Same `lib/server` primitives; small extra demo program.  Ship
both verifier and editor on top of the same Step 1.

If anything substantial is added beyond click → red → sync
(camera sync between clients, walls, items, height, persistence,
authentication, …), it moves to a follow-up plan; this plan
stays focused.

---

## Out of scope (named explicitly so they don't creep in)

- **Persistence to disk.**  Server forgets the world on restart.
  v2 adds `world.save("world.dat")` on shutdown.
- **Authentication / sessions / identity.**  Any client can edit
  anything.  v2 adds basic per-client identity if the demo grows.
- **Concurrent-edit conflict handling.**  Last-write-wins; if two
  clients click the same hex within 100 ms, the server applies
  both in arrival order.  Acceptable for v1.
- **Camera sync between clients.**  Each client's camera is
  local; players don't see where other players are looking.
- **More than two clients.**  Not tested.  The architecture
  supports N but the smoke test stops at two.
- **Walls, items, item rotations, height changes.**  Only the
  ground-material change is wired.  Painting `painted` →
  `painted` is idempotent; un-painting (back to gray) is a v2
  feature.
- **Mouse-hover preview / selection cursor.**  Click-only.
- **Serialisation efficiency.**  Snapshot is plain text;
  binary-frame payloads come with the EventLoop full implementation.
- **Hot-swap, V4 script-upload, any of TIC_TAC_TOE.md v3/v4.**
  This plan is the post-v1-tictactoe milestone, not the
  post-everything-else milestone.

---

## What this validates that tic-tac-toe didn't

- **Multi-client server in production shape**, not a toy.  Two
  simultaneous connections, broadcast traffic, snapshot replay.
- **Real game state synchronised**, not just request-response.
  The world is shared, persistent (in-memory), and observed by
  multiple clients.
- **Real loft rendering** (lib/moros_render) wired to a network
  source.  First time the editor's render loop has a remote
  authority it pulls state from.
- **Snapshot delivery** — the smallest non-trivial bulk-data
  exchange across the wire.  Stresses the text-mode encoder for
  payloads larger than a single line.
- **The smallest real loft multiplayer game**, in the sense of
  "this is something a player would actually open and use" once
  the painted-material set grows beyond one colour.

---

## Cross-references

- [TIC_TAC_TOE.md](../32-tic-tac-toe/README.md) — protocol validator (v1
  shipped); v2/v3 are deferred to focus this milestone instead.
- [EVENT_PROTOCOL.md](../23-event-loop/PROTOCOL.md) — wire format spec.
- [EVENT_LOOP.md](../23-event-loop/README.md) — full event-loop spec; this
  demo lives below the EventLoop's implementation.
- `lib/server/src/server.loft` — extension target for Step 1.
- `lib/moros_editor/`, `lib/moros_map/`, `lib/moros_render/` —
  the existing editor stack the demo plugs into.
- `lib/game_protocol/examples/tictactoe_*.loft` — the patterns
  this demo's protocol code mirrors.
