<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN50 — Phase 0a probe (MVP)

End-to-end smoke test for the `@PLAN50` wire protocol — the
minimum-viable validation that the design's sight-range cutoff +
explicit `EXIT` signaling work as specified before phase 0
commits to a full phone-client implementation.

**Status:** working MVP — sight cutoff and EXIT signaling
verified correct.  Throughput per-peer at full rate hasn't been
investigated yet (observed ~12 Hz, design target 30 Hz); the
protocol shape is sound regardless.  Rate-LOD bands (15 Hz mid,
7.5 Hz outer) deferred to v2.

See [`../../doc/claude/plans/51-bumper-airplanes/00a-network-probe.md`](../../doc/claude/plans/51-bumper-airplanes/00a-network-probe.md)
for the full design of the production-grade probe.  This MVP
intentionally covers only the v1 must-pass behaviours; it is
the *first* validation step, not the only one.

## Files

| File | Purpose |
|---|---|
| `probe_server.loft` | Minimum echo server: receives `INPUT` frames carrying each client's position, per 30 Hz tick broadcasts `POSE` for in-sight peers + one-shot `EXIT` on outward sight-range crossings |
| `probe.loft` | Synthetic-client harness: connects N clients on a fixed grid (20 m spacing × 6 wide), sends one INPUT per client, drains incoming POSE + EXIT for a fixed duration, reports per-client counts; then moves client 0 far away and asserts other clients see exactly one EXIT for it |
| `interp_test.loft` | **Standalone interpolation correctness test.**  No network, no server.  Generates 3 ground-truth trajectories (constant-velocity line, constant-turn circle, sudden-swing bounce at t=1.5), subsamples at 30 / 15 / 7.5 Hz, reconstructs at 60 fps via both linear and Hermite cubic interpolation, measures max + mean position error vs ground truth.  PASS threshold: max position error < 10 cm.  Findings drove two design decisions: (1) Hermite is the default interp method (linear has 67 mm error on circular motion at 7.5 Hz, Hermite has 0); (2) bounces always emit a priority POSE keyframe — at low rates a missed bounce produces 0.3–1.0 m peak error that no interp method alone can hide |

## Wire protocol (smoke-test framing)

Text wire frames in the PLAN36 `<msg_id>:<payload>` convention.

| Direction | `msg_id` | Payload | Meaning |
|---|---|---|---|
| client → server | `1` | `cid,x,y,z` | INPUT — client cid is at world position (x, y, z).  Probe-only shape; production INPUT carries thumb positions and the server runs physics |
| server → client | `2` | `plane_id,seq,x,y,z` | POSE — peer `plane_id` is at the given position (sequence number for drop detection) |
| server → client | `3` | `plane_id` | EXIT — peer `plane_id` just crossed out of your sight range |

## How to run

In two terminals (from the repo root):

```bash
# Terminal 1 — server
./target/release/loft --no-warnings --lib lib/ \
    tools/audience-demo-50/probe_server.loft

# Terminal 2 — probe (run AFTER server prints "listening")
./target/release/loft --no-warnings --lib lib/ \
    tools/audience-demo-50/probe.loft
```

Env knobs for the probe:

| Variable | Default | Purpose |
|---|---|---|
| `LOFT_PROBE_CLIENTS` | 4 | Number of synthetic clients to spawn |
| `LOFT_PROBE_SECS`    | 3 | Duration of the stationary drain phase |

## Expected output (verified working)

Server stdout:
```
probe_server: listening on port 18084
  ws://127.0.0.1:18084/ws
  sight range: 80 m
  tick rate:   30 Hz (every 33333 us)
probe_server: ws client 0 connected (active: 1)
probe_server: ws client 1 connected (active: 2)
probe_server: ws client 2 connected (active: 3)
probe_server: ws client 3 connected (active: 4)
```

Probe stdout (4 clients at (0,0) (20,0) (40,0) (60,0) — all
within 80 m of each other):

```
=== probe: 4 clients, drain 2 sec ===
probe: 4 clients connected
probe: drain phase for 2 sec...
probe: stationary phase report:
  client 0 (at 0,0): poses=75 exits=0
  client 1 (at 20,0): poses=75 exits=0
  client 2 (at 40,0): poses=74 exits=0
  client 3 (at 60,0): poses=72 exits=0
  expected ~180 POSE/client at full sight, no LOD
  total POSE received: 296

probe: moving client 0 to (1000, 1000, 0) — expecting EXITs
probe: EXITs received per client after move:
  client 1: 1  (expected: 1)
  client 2: 1  (expected: 1)
  client 3: 1  (expected: 1)

probe: done
```

The two important behaviours:
1. **Stationary phase**: POSE counts are roughly uniform across
   all 4 clients (sight filter is symmetric); no EXITs (all
   peers stay in range).
2. **After-move phase**: clients 1–3 each receive **exactly 1
   EXIT** for plane_id=0 once it crossed `peer_sight_range`
   outward.  Client 0 doesn't EXIT itself.

## Known limitations of this MVP

- **No rate-LOD bands** — every in-sight peer gets every tick.
  The v2 probe adds the 30/15/7.5 Hz banding and asserts the
  ratios match the design.
- **No projector simulation** — only the per-phone sight filter
  is tested.  Production server-to-projector is unfiltered;
  v2 probe will spawn a synthetic projector client to verify.
- **No EVENT messages** — sound-trigger events (bounce, score,
  stall) aren't covered.  Wire shape is documented in the plan
  but the probe doesn't yet exercise it.
- **No latency histograms** — measurements are just frame
  counts.  v2 probe attaches send/receive timestamps and
  reports p50 / p95 / p99 latency tails.
- **Throughput under target** — observed ~12 Hz/peer instead of
  30 Hz/peer at this load.  Likely a server loop pacing issue
  (`web::sleep_ms(2)` after each empty pump pass, plus the
  per-tick O(n²) broadcast walk).  Worth investigating before
  phase 0 commits to the existing pump pattern; doesn't change
  the protocol design.

## What this proves

The protocol design from [README.md § Wire protocol](../../doc/claude/plans/51-bumper-airplanes/README.md#wire-protocol--message-kinds-v1)
maps cleanly onto `lib/server`'s existing event-pump pattern:
no new native functions, no new lib API surface — `srv.send_to(cid, msg)`
is exactly what per-recipient sight filtering needs.

Phase 0a v2 (full benchmark) and phase 0 (real phone client) can
proceed without re-litigating the wire format.
