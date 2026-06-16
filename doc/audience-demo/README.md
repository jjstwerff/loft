<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Audience demo — phone client

Single static HTML/JS page each audience member loads in their phone
browser.  Renders a 9×7 hex world, lets the user tap to paint, sends
each paint event over WebSocket to a shared loft server.  Every other
connected client sees the paint appear in their hex grid in real time.

Part of the @PLN6 audience-driven generative-art demo
(see [`doc/claude/plans/6-audience-generative-art/`](../claude/plans/6-audience-generative-art)
for the development plan).

## Try it (static-only — no server)

Open this page in any modern browser:

  - On the deployed site: <https://loft-lang.org/loft/audience-demo/>
  - Locally: `python3 -m http.server 8766 -d doc` then
    <http://localhost:8766/audience-demo/>

You'll see the 9×7 hex grid, the 9-color palette, and a status line
at the bottom saying *"disconnected — retrying in Ns"* (no server
running).  The palette has a colour pre-selected at random; tap any
hex in the centre area to paint locally.  Refresh to clear.

## Full integration test — multi-client paint (needs the loft server)

1. **Build loft**: `cargo build --release` from the repo root.

2. **Start the server** (terminal A):
   ```
   ./target/release/loft --no-warnings --lib lib/ tools/audience-demo/server.loft
   ```
   Server listens on `ws://localhost:18083/` and logs every connect +
   incoming message.

3. **Serve the page** (terminal B):
   ```
   python3 -m http.server 8766 -d doc
   ```

4. **Find your host's LAN IP** (so other devices on the same WiFi can
   reach you):
   ```
   hostname -I       # e.g. 192.168.1.50
   ```

5. **Open from your phone** — same WiFi as the host:
   ```
   http://192.168.1.50:8766/audience-demo/
   ```
   The status line should turn green: *"connected to ws://192.168.1.50:18083/"*.

6. **Open another tab on your laptop** (or a second phone): same URL.

7. **Tap hexes on one device** — they appear on every other connected
   device within ~100ms.  The server logs each event:
   ```
   audience server: [connect cid=0]
   audience server: [connect cid=1]
   ```
   (Per-event logs are emitted by the broadcast path — uncomment
   `println` lines in `tools/audience-demo/server.loft` to see them.)

## Wire format (current MVP)

Single WebSocket carries `<msg_id>:<payload>` text frames in both
directions:

| msg_id | Direction | Payload | Meaning |
|---|---|---|---|
| 1 | client → server | `q,r,color` | Seed a hex (color 1-9 = palette) |
| 2 | client → server | `q,r` | Clear a hex |
| 3 | client → server | `color` | Player picked a colour (0 = none) |
| 4 | server → client | `q,r,color` | Cell changed; color 0 = now empty |
| 5 | server → client | `q,r` | Jump-to-active hint (another player painted here) |

The full demo plan ([§ Wire format](../claude/plans/6-audience-generative-art/01-server-state.md#wire-format--json-events--binary-world-blobs))
calls for JSON-in / binary-out — the MVP uses
`lib/server`'s native msg_id framing for both directions to keep the
prototype small.  Migration to JSON + binary blobs is filed under
phase 1 step 2.

## Known limits in this MVP

- No tick loop yet — cells never decay (the full demo decays cells
  over ~5 minutes; here they stay forever until explicitly cleared).
- No swipe gesture — taps only.
- No outer-ring pan zones — the visible window is fixed on world (0, 0).
- No view-centring on activity at load — view always starts at (0, 0).
- `Jump to active` button placeholder — flashes on incoming
  active-player signals but doesn't recenter the view.
- Wire is `<msg_id>:<payload>` text, not the spec'd JSON + binary.

These ship in subsequent phase 1 + phase 0 commits.

## Code

- [`index.html`](index.html) — the deployable static client
  (single file, inline CSS + JS, no build pipeline).  This is the
  source of truth; GitHub Pages serves this verbatim.
- [`tools/audience-demo/server.loft`](../../tools/audience-demo/server.loft)
  — the loft server (dev-only, not deployed).
