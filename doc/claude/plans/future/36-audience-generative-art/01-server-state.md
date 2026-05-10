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

A first cut of the world structure:

```loft
struct world {
    cells: vec<cell>,         // sparse — keyed by (q, r) hash
    seeds: vec<seed>,         // active growth sources
    players: vec<player>,     // who is connected, what colour, when last active
    tick: i64,                // monotonic
}

struct cell {
    q: i64,
    r: i64,
    color: i64,               // palette index
    age: i64,                 // ticks since planted
    planted_by: i64,          // player_id that originally seeded this cluster
}

struct seed {
    q: i64,
    r: i64,
    color: i64,
    planted_at: i64,
}

struct player {
    id: i64,                  // assigned on connect
    color: i64,               // most recently selected color (-1 = cleared)
    last_active_q: i64,       // most recent (q, r) the player painted at
    last_active_r: i64,
    last_active_tick: i64,
}
```

(Final shape pinned at CI-2 once multi-client behaviour is
observed.)

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

1. Run the generation step (phase 2 script): for each empty hex
   adjacent to a live cell, optionally fill it with a color
   biased by neighbor majority + per-direction color votes.
2. Compute `world_delta`: list of cells whose color or age
   changed since last tick.
3. Broadcast delta to all subscribers.
4. Garbage-collect: cells older than N ticks may "freeze" or
   "die" depending on the generation variant.

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
