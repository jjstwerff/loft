<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 1 — Server state

## Status

Open.  Owns the shared world state and the protocol that connects
audience clients ↔ projector view.  Effort: **M**.

## Goal

A loft program that:

1. Holds the authoritative shared world state (a sparse hex grid
   + per-cell color + per-cell age + active-player registry).
2. Accepts WebSocket connections from audience clients (phase 0)
   and the projector (phase 3).
3. On each event from a client, updates state.
4. On each tick, runs the generation step from phase 2 and
   broadcasts a `world_delta` to all subscribers.
5. Detects "active player" signals (color changes, recent input
   bursts) and broadcasts them so the audience clients can
   surface a **Jump to active** indicator.

Builds on the shipped `lib/server` multi-client WebSocket API
(`srv.run(on_event)`, `ws_clients_*`, `ws_event_*`).

## State model

The world reuses the **`lib/moros_map` chunk pattern**: a sparse
collection of 32×32 chunks at integer chunk coordinates, with
the same `chunk_idx_32` / `hex_idx_32` addressing helpers (which
handle negative-coordinate floor-division correctly).  The
per-hex payload is **slimmer** than the moros editor's `Hex`
struct — the demo only needs colour, height, and age — but
addressing, chunk lifecycle, and the sparse-storage discipline
are reused as-is.

```loft
struct World {
    chunks: vector<Chunk>,    // sparse — only chunks that have at least one
                              // non-empty cell exist; emptied chunks are removed
    players: vector<Player>,  // who is connected, what colour, when last active
    tick: integer not null,   // monotonic
}

// 32x32 grid at chunk coords (cx, cz).  Index into ck_cells: cx * 32 + cz.
// Same shape as lib/moros_map's Chunk minus cy (this demo is single-layer).
struct Chunk {
    ck_cx:    integer not null,
    ck_cz:    integer not null,
    ck_cells: vector<Cell>,
}

// 4-byte cell — colour and height use loft's u8 typedef (integer
// limit(0, 255) size(1)), age uses u16 (size(2)).  Storage packs
// to exactly 4 bytes per cell.
struct Cell {
    c_color:  u8 not null,       // 0 = empty (no crystal), 1-9 = palette
    c_height: u8 not null,       // 0..255, derived from filled-neighbour count
    c_age:    u16 not null,      // 0..65535 ticks since the cell became non-empty;
                                 // projector keys the 5-second growth animation off
                                 // this (small age = still growing, ~50 ticks at
                                 // 10 Hz tick rate = grown)
}

struct Player {
    p_id:               integer not null,  // assigned on connect
    p_color:            integer not null,  // 0 = cleared (no active colour),
                                           // 1-9 = palette index
    p_last_active_q:    integer not null,  // most recent (q, r) painted at
    p_last_active_r:    integer not null,
    p_last_active_tick: integer not null,
}
```

The `Cell` carries no growth-direction or planting-author
metadata.  Both are derived where they are needed:

- **Growth direction** for the projector's 5-second tilt
  animation is computed at render time from the **age
  comparison with neighbours** — older neighbour cluster on one
  side, this cell newer = the new cell extrudes from the older
  side.
- **Plant ownership** (who seeded a cluster) is not tracked
  per-cell; the active-player signal lives on `Player` and is
  enough to drive the audience-client jump-to-active flash.

(Final shape pinned at CI-2 once multi-client behaviour is
observed.)

### Chunk lifecycle

A chunk is **created on first non-empty write**: when a `seed`
event lands at world hex (q, r), the server computes
`(cx, cz) = (chunk_idx_32(q), chunk_idx_32(r))`, ensures a
chunk exists at those coords (build a 32×32 of empty cells if
not), and writes the cell.

A chunk is **deleted** when all 1024 of its cells become empty
(colour = 0).  This matches the user-visible rule: a chunk that
is entirely black does not exist.  Garbage-collection runs at
the end of each tick after generation + age updates.

The world coordinates of any cell are recoverable from
`(ck_cx * 32 + hx, ck_cz * 32 + hz)` where `hx, hz ∈ [0, 31]`
are the in-chunk indices.  No per-cell `(q, r)` storage needed.

## Event handling (client → server)

| Event type | Server action |
|---|---|
| `seed` | Append to `cells` if empty; if hex was own-color, treat as removal; if hex was other-color, ignore.  Update `player.color` + `last_active_*`.  Add active-player signal entry |
| `clear` | Clear active color on the player (no world change) |
| `color_select` | Update `player.color` and broadcast active-player signal |
| `connect` (implicit on WS open) | Assign player_id, broadcast initial world snapshot |
| `disconnect` (implicit on WS close) | Mark player inactive but keep their cells |

## Tick loop

Server runs a fixed-rate tick (suggested 4-10 Hz; CI-2 to tune).
Each tick:

1. Run the generation step (phase 2 script): for each empty cell
   adjacent to a live cell, optionally fill it with a colour
   biased by neighbour majority + per-direction colour votes.
2. Update `c_height` on each cell whose filled-neighbour count
   changed (cell newly filled, or a neighbour newly filled /
   emptied).  Height is derived from the local neighbour count.
3. Increment `c_age` on every non-empty cell; clamp to 65535
   (the 2-byte ceiling).
4. Compute `world_delta`: per-chunk lists of `(hx, hz, color,
   height, age)` for cells whose payload changed since last
   tick.
5. Garbage-collect chunks: any chunk whose 1024 cells are all
   empty (colour = 0) is removed from `chunks`.
6. Broadcast delta to all subscribers (including chunk
   creations + chunk deletions).

Note: the **3D height field** the projector renders is computed
**from the per-cell `c_height` byte the server ships** (so all
clients agree on the surface shape).  The growth-direction tilt
on cells whose age < the 5-second window is derived
**client-side** from comparing this cell's age with its
neighbours' ages — older neighbours contribute the "from"
direction.  Server state stays minimal: 4 bytes per cell, no
per-cell direction or origin metadata.

## Active-player signal

Each player change is timestamped.  Once per second, the server
picks the most-recently-active player (excluding the recipient)
and broadcasts an `active_player_signal { x, y }` to each
audience client.  This drives the **Jump to active** box's flash.

(Avoid sending one signal per change — overwhelms the UI with
flashes during busy periods.  One every second feels like a
heartbeat.)

## Sub-tasks

| # | Task | Effort |
|---|---|---|
| 1.1 | World struct + sparse-cell access (`get_cell(q, r)` / `set_cell(q, r, color)`) | S |
| 1.2 | WebSocket event dispatch (route by `type` field) | XS |
| 1.3 | Per-player state + connect/disconnect lifecycle | S |
| 1.4 | Tick loop (timer-driven, fixed rate) | S |
| 1.5 | World-delta computation (diff against previous tick) | S |
| 1.6 | Broadcast helpers (single-client send + all-subscribers send) | XS |
| 1.7 | Active-player signal aggregation + emit | XS |
| 1.8 | Multi-client load test (2-3 → 10-30 → upper bound) | S |
| 1.9 | Crash-resistance — does a misbehaving client (malformed JSON, dropped mid-handshake) bring the server down? | S |

## Open design questions

- **Tick rate** — 4 Hz (cheap, choppy) or 10 Hz (smoother,
  bandwidth heavier).  CI-2 picks after watching multi-client.
- **Delta vs full-snapshot** — every tick send a delta and
  occasionally a full snapshot?  Or always delta + new clients
  receive a snapshot on connect?  Recommend delta-always + snapshot-on-connect.
- **Persistence** — does the world survive a server restart?
  Probably no for the talk; restart wipes the canvas, which is a
  feature ("watch round 2 grow from blank").  Add a `--persist`
  flag if useful for unattended installations later.
- **Bandwidth ceiling** — at 10 Hz × N clients × M cells/tick,
  does this saturate venue WiFi?  Phase 4 hosting deals with the
  network plane; phase 1 measures the volume.
- **Authority** — server is authoritative; clients render what
  the server sends (no client-side prediction needed for this
  talk's scale).

## Deliverable

A loft program (`lib_examples/audience_demo/server.loft` or
similar) that:

- Listens on a configurable port.
- Accepts WebSocket connections from any number of clients.
- Drives the world forward at the configured tick rate.
- Survives `--load-test 30` (30 simulated clients each tapping
  randomly for 5 minutes) without leaking memory or losing
  events.

## See also

- [`README.md`](README.md) — parent plan
- [`00-audience-browser-page.md`](00-audience-browser-page.md) —
  what sends events into this server
- [`02-generation-script.md`](02-generation-script.md) — the
  growth algorithm this server runs each tick
- [`03-projector-view.md`](03-projector-view.md) — the second
  subscriber type
- `lib/server/src/server.loft` — shipped multi-client WS API
- [`../../../lib_plans/future/08-server/`](../../../lib_plans/future/08-server/) —
  upstream library work this phase sharpens
