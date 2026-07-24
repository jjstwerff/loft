<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Audience demo — dev workspace

Two loft servers + a README documenting the local development
workflow.  The **client** lives at [`doc/audience-demo/`](../../doc/audience-demo)
because that's the GitHub Pages publish root (see
`.github/workflows/release.yml` → `publish_dir: ./doc`).

## Files

- [`dev_server.loft`](dev_server.loft) — **single-port HTTP + WebSocket**
  dev server.  Listens on `:18083` and serves both
  `doc/audience-demo/index.html` (HTTP `/`) and the WebSocket
  endpoint (`/ws`) from one process.  Re-reads the HTML from disk
  on every request — edit the file, refresh the browser, see the
  change.  Single-client WS (one tab at a time) — echoes seed /
  clear events back as world deltas so painting roundtrips even
  with no peers.  This is the fastest iteration loop for client-
  side edits.
- [`server.loft`](server.loft) — the **multi-client world server**.
  WebSocket-only (no HTTP serving).  Listens on `ws://localhost:18083/`,
  holds the shared world, broadcasts every change to all connected
  clients, replays current state to new connections.  Use this
  whenever you want to test more than one tab / phone at once.
  Wire format and full sub-arc status documented at the top of the
  file.
- [`projector.loft`](projector.loft) — the **GL projector view**, since
  2026-06-11 a full **engine-host connector client** (@PLN18): it dials the
  kernel server through `engine_host::run_client` (auto-hello, keepalives,
  UDP auto-path — zero transport code), the render loop IS the connector's
  tick (one `on_tick` = one frame), deltas arrive through `on_event`, and
  closing the window calls the new `client_stop()`.  It reconnects through
  a server restart mid-show (snapshot re-request heals the world).  Pair it
  with `server_kernel.loft`:

  ```
  ./target/release/loft --no-warnings --lib lib tools/audience-demo/server_kernel.loft
  ./target/release/loft --no-warnings --lib lib tools/audience-demo/projector.loft
  ```

  `LOFT_PROJ_SERVER` still accepts the legacy `ws://host:port/ws` form (or
  `host:port`); the default host honours `LOFT_HOST`.  Cross-closure
  mutable state lives in the `Proj` record (the #314 struct-world idiom);
  captured vars feeding `&` params use the copy-out/copy-back locals
  pattern (see the comments at the call sites).
- `.loft/` — per-source native binary cache (gitignored).

## Run end-to-end locally — fastest loop

```
./target/release/loft --no-warnings --lib lib/ tools/audience-demo/dev_server.loft
```

On startup the dev server prints every URL it's reachable at:

```
audience dev-server: listening on port 18083
  reachable URLs (any of these should work in a browser):
    http://localhost:18083/
    http://<hostname>:18083/
    http://<lan-ip>:18083/
  the page derives its WebSocket URL from `location.host`,
  so whichever URL above you opened, /ws on the SAME host:port
  is used automatically — no need to edit the client.
```

Hostname comes from `/etc/hostname`; LAN IPs are harvested from
`/proc/net/fib_trie` (non-loopback `/32 host LOCAL` entries).  Pick
any of them on any device on the same LAN — the phone, a tablet,
the dev box — and the page connects WebSocket back to the *same*
host:port it loaded from.

The default port is 18083; change it by editing `dev_server.loft`
(one constant in `main()`).  The client follows whatever port is
in `location.host`, so no client edit is needed.

## Run end-to-end locally — multi-client

For more than one tab at once (real audience flow):

```
# terminal A — multi-client WebSocket server
./target/release/loft --no-warnings --lib lib/ tools/audience-demo/server.loft

# terminal B — serve the static client (the whole doc/ tree so
# brick-buster + the gallery are also reachable on the same port)
python3 -m http.server 8766 -d doc

# open http://localhost:8766/audience-demo/ in two tabs
# (or http://<lan-ip>:8766/audience-demo/ from a phone)
```

`server.loft` listens on `:18083` (WS only); the page derives the WS
URL from `location.host` — *which is `localhost:8766` when served
this way*.  So this combo needs the page's `?ws=ws://localhost:18083/ws`
override (or matching ports if you change `server.loft`).  The
`dev_server.loft` loop avoids this entirely by serving both on one
port.

## Phone testing (same WiFi)

With `dev_server.loft`: just pick the LAN-IP URL out of the startup
banner and type it into the phone's browser.  Same instance,
WebSocket auto-routed.

With `server.loft` + `python3 -m http.server`: the phone reaches the
page at the static-server's IP+port, but needs `?ws=ws://<lan-ip>:18083/ws`
appended so the WebSocket talks to the loft server instead of the
static server.

## Known dev-loop pitfall

If you've ever opened the page from `file://` (no host), `location.host`
is empty and the fallback `localhost:18083` kicks in — fine on the dev
box, broken on the phone.  Always load via `http://` so the auto-derive
works.

## Headless integration test (used to validate this branch)

Two scratch scripts from the development session:

- `/tmp/wsclient.mjs` — single raw Node WebSocket client; sends
  three frames.  Used to prove the wire format against the `server` library.
- `/tmp/phase1_test3.mjs` — two parallel headless-Chrome tabs, both
  attach via CDP, A sends a paint, both A + B read back the cell
  state via `window.__phase0.cells`.

These aren't checked in as Rust tests yet because the headless harness
needs polishing (it caught @P286 — the Sec-WebSocket-Accept GUID
transposition — but is brittle around tab attach timing).  A cleaner
end-to-end test is filed under @PLN6 phase 1.8 (multi-client load
test).

## Related

- [`doc/audience-demo/`](../../doc/audience-demo) — the deployable
  client (this is what GitHub Pages serves)
- [`doc/claude/plans/6-audience-generative-art/`](../../doc/claude/plans/6-audience-generative-art)
  — full development plan
- [`server`](https://github.com/loft-lang/loft-libs-net/tree/main/server) — the WebSocket library
