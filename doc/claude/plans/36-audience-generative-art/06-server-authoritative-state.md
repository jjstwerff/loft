<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN36 sub-arc 6 — Server-authoritative crystal state + deterministic multi-client rendering

## Status

**Open (design) — 2026-05-22.**  Prompted by live-demo feedback: the projector
derives each hex's crystal role *itself*, every frame, so roles flip and clients
can disagree; and decay removals don't reach phones until they reconnect.

## Why — what the per-client model gets wrong

The crystal is "grown" client-side: the projector's `crystal_segments_aged`
(chunk path) re-derives, per cell every frame, whether a hex is a **full
crystal** (a CENTRE that grows + sprouts branches) or a **line** (a thin
connection).  Three problems surfaced on the live demo (2026-05-22):

1. **Roles flip frame-to-frame.**  The decision is recomputed from the current
   snapshot, and (after the overlap fix that made connection depend on the
   *nearest filled* cell rather than the *nearest older*) a newer hex filling a
   gap can flip an existing hex between line ↔ bloom.
2. **Two adjacent hexes can both become separate crystals.**  A blocked
   connection promotes a hex to a spurious new centre.
3. **No persistence / no agreement.**  Because the role isn't stored, it isn't
   stable across rebuilds and **two projectors can render different crystals**
   from the same world.

The lib already has the *stable* decision — `assign_cells` (placement-ordered:
the first hex of a chain is the CENTRE, later reachable hexes are SUPPORTERS) —
but the chunk path doesn't consult it.

## Model — share inputs, not pixels

Move the role decision to the **server** (single source of truth) and broadcast
the minimal authoritative **seed** — `cells + role + birth` — to every client.
Each OpenGL/web client then runs its *own* deterministic growth + render model
*from that seed* and arrives at the **same crystal**.  The server never renders;
clients share inputs, not output.  This is the natural fit for the user's model
("only full crystals grow; the existing crystal reaches toward each new point;
the role should persist") and cleanly subsumes problems 1–3.

Two layers:

### Layer 1 — server-authoritative centre/line role (fixes 1–3)

- **World state:** add a per-cell `role` (CENTRE vs LINE; LINE also records its
  centre) to the server's `world.cells`.
- **Decision at paint time:** when a paint arrives, the server runs the
  placement-ordered assignment (the `assign_cells` rule — `use audience_crystal`;
  the hex helpers are already reusable) to set the new cell's role and join it to
  the nearest reachable crystal within `MAX_AXIS_GAP`, else start a new centre.
  - **Promotion:** *only full crystals grow*, so a click beside a hex that is
    only a LINE promotes that line to a CENTRE so growth can originate there
    (decision recorded once; role only ever LINE→CENTRE, never back).  [Confirm
    promotion target with the presenter: existing line vs the new click.]
- **Persistence:** bump `world.bin` to **v3** (role per cell); v2 files load with
  roles recomputed once on first tick.
- **Wire:** broadcast the role — extend the paint delta `4:x,y,color` →
  `4:x,y,color,role`, plus a small **`7:x,y,role`** message for *promotions*
  (an already-painted cell whose role changes).  Phones ignore the new field;
  the projector consumes it.
- **Projector:** drop its local `assign_cells`/per-cell role guessing entirely
  and render from the server-supplied role — CENTRE → full crystal that grows,
  LINE → thin connection.  **Revert the 2026-05-22 overlap fix** in
  `crystal_cell_segments` (it caused the flip); the server's centre-chain
  assignment + one-main-per-axis replaces it.

This makes geometry **identical and stable** across all clients (deterministic
given shared cells-in-order + roles), with no flip and no spurious crystals.

### Layer 2 — shared clock for identical multi-client *animation* (optional)

Geometry agrees for free once roles are shared.  The growth *animation* only
matches across clients if the bloom uses a **shared clock**: today the projector
blooms from its *local* frame counter for both the cell birth and `uNow`, so two
projectors that connected at different times animate the same crystal on
different schedules (they converge to the same final image, but mid-growth
differs).  To make them frame-identical, drive birth + `uNow` from the
**server's** tick (already on the wire as `birth_tick`) instead of local frames.
Only needed if more than one live projector runs simultaneously.

## Reliable removal sync (the phone bug — 2026-05-22)

Decay removals (`4:x,y,0`) are broadcast, but **phones keep showing removed
hexes until the phone disconnects and re-requests the world** (msg_id 6).  So the
removal delta is either not delivered to, or not applied by, the browser client.
Fix as part of this sub-arc (server-authoritative state is only useful if state
*changes*, including removals, reach every client reliably):

- Confirm the server `broadcast`s the `4:x,y,0` removal to **all** clients (not
  just the painter / not dropped under the WS-pump fan-out).
- Confirm `doc/audience-demo/index.html` **applies** a `4:x,y,0` delta as a
  cell *removal* (clear the hex), not just non-zero colours as paints.  The
  native projector already fades removed cells out; the browser path is the gap.
- Add a removal to the multi-client load/smoke test so this can't regress.

## Implementation order

1. Layer 1 server side: role in `world.cells` + `assign_cells` at paint +
   promotion + v3 persistence + `4:…,role` / `7:` wire.  (`single_port_server.loft`)
2. Layer 1 projector side: consume role; remove local role derivation; revert
   the overlap fix.  (`projector.loft`, `lib/audience_crystal/src/crystal.loft`)
3. Reliable removal fix (server broadcast audit + browser apply).
4. Layer 2 (only if multi-projector): server-clock-driven bloom.

## See also

- [01-server-state.md](01-server-state.md) — world state, decay, broadcast, wire.
- [03-projector-view.md](03-projector-view.md) — crystal mesh + GPU bloom (the
  two-phase old→new growth + per-segment bloom anchor landed 2026-05-22).
- [00-audience-browser-page.md](00-audience-browser-page.md) — the phone client
  that must apply removal deltas.
