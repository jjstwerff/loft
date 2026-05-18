<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Audience demo — dev workspace

This directory holds the **server** for the @PLAN36 audience demo +
this README documenting the development workflow.  The **client**
lives at [`doc/audience-demo/`](../../doc/audience-demo/) because
that's the GitHub Pages publish root (see `.github/workflows/release.yml`
`publish_dir: ./doc`).

## Files

- [`server.loft`](server.loft) — the multi-client world server.
  Built on `lib/server`'s WebSocket primitives.  Listens on
  `ws://localhost:18083/`, broadcasts every change to all connected
  clients, replays current state to new connections.  See
  [the deployable demo's README](../../doc/audience-demo/README.md)
  for the wire format and run commands.
- `.loft/` — per-source native binary cache (gitignored).

## Run end-to-end locally

```
# terminal A — server
./target/release/loft --no-warnings --lib lib/ tools/audience-demo/server.loft

# terminal B — static client (the doc/audience-demo/index.html is
# what gets shipped to GitHub Pages; the local server serves the
# whole doc/ tree so brick-buster + the gallery are also reachable)
python3 -m http.server 8766 -d doc

# open http://localhost:8766/audience-demo/ in two browser tabs
```

## Phone testing (same WiFi)

```
hostname -I                                    # e.g. 192.168.1.50
# phone opens http://192.168.1.50:8766/audience-demo/
# the client uses location.hostname for the WS URL, so it auto-
# resolves to the same host that served the page
```

## Headless integration test (used to validate this branch)

Two scripts in `/tmp/` from the development session:

- `/tmp/wsclient.mjs` — single raw Node WebSocket client; sends
  three frames.  Used to prove the wire format against `lib/server`.
- `/tmp/phase1_test3.mjs` — two parallel headless-Chrome tabs, both
  attach via CDP, A sends a paint, both A + B read back the cell
  state via `window.__phase0.cells`.

These aren't checked in as Rust tests yet because the headless harness
needs polishing (it caught @P286 — the Sec-WebSocket-Accept GUID
transposition — but is brittle around tab attach timing).  A cleaner
end-to-end test is filed under @PLAN36 phase 1.8 (multi-client load
test).

## Related

- [`doc/audience-demo/`](../../doc/audience-demo/) — the deployable
  client (this is what GitHub Pages serves)
- [`doc/claude/plans/36-audience-generative-art/`](../../doc/claude/plans/36-audience-generative-art/)
  — full development plan
- [`lib/server/`](../../lib/server/) — the WebSocket library
