<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0a — Network throughput probe

**Goal:** Validate that the existing `lib/server` Tier-A′ pump
(shipped at @PLN6 phase 1.9, commit `a72ad22` in PR #214)
sustains the **30 Hz × N** input + broadcast load that PLAN50's
per-frame pose sync demands, at the player counts the design
targets (12 / 20 / 30 simultaneous players).

This is the cheapest pre-phase-0 work item: a half-day of
synthetic-load code that converts the dominant unknown into a
known.  If the pump holds, phase 0 (phone client) starts with
confidence.  If it doesn't, the design needs to add
server-authoritative dead-reckoning + delta updates before phase
0 — a non-trivial scope bump that is *much* cheaper to discover
at probe-stage than after a full phone client is in flight.

## What's already proven from @PLN6

PLAN36's `load_test.loft` (in
[`tools/audience-demo/`](../../../../tools/audience-demo))
already validated:

- **Connection establishment** at 30 clients: ~10 sec total
  after the Tier-A′ pump fix (was 220 sec before).
- **Broadcast correctness** at 30 clients: 100% fan-out, zero
  send failures, zero drops at every tier (3 / 12 / 30).
- **Crash-resistance** to malformed/hostile frames.

But the **paint** protocol PLAN36 measured is *sparse* — clients
emit one frame per user tap (typically < 1 Hz per client).  PLAN50
needs each client to emit 30 frames/sec.  That's a **~30 × higher
inbound rate** the pump has never been measured under.

**Outbound broadcast is substantially lighter than worst-case**
because of `net.peer_sight_range` filtering (see [NUMBERS.md](NUMBERS.md)
§ Networking).  Each phone only receives pose-frames for OTHER
planes within visual range of its own plane — typical visible-peer
count is much smaller than N − 1.  Concretely: with `peer_sight_range
= 80 m` and a ~100 × 100 m map carrying 30 players spread out, a
typical phone sees ~5–10 peers (not 29), so its outbound stream is
~5–10 × 30 Hz × 32 B ≈ 5–10 KB/s, not 27 × 30 Hz × 32 B ≈ 26 KB/s.
Inbound (input frames) is fixed at 1 × 30 Hz × ~16 B ≈ 0.5 KB/s
per client.

The **projector** is never sight-filtered — it consumes all N
planes' poses at 30 Hz, so its receive rate is `N × 30 Hz × 32 B`
(roughly 30 KB/s at N=30).  The projector matters separately from
phones because it's one client with high inbound, not N clients
with moderate inbound.

## What the probe measures

| Metric | Target | Failure threshold |
|---|---|---|
| **Per-client inbound rate sustained** | 30 Hz | Server-side enqueue rate consistently < 28 Hz per client at any tier |
| **Per-client outbound broadcast rate sustained** | 30 Hz | Per-client receive rate consistently < 28 Hz at any tier |
| **End-to-end latency (input → broadcast back)** | < 100 ms p99 | p99 > 150 ms at the target tier |
| **Drop rate (inbound or outbound)** | 0 | Any drop in a 30-sec run at the target tier |
| **CPU (server process) at peak tier** | < 75 % of one core | > 90 % sustained — pump is bottlenecking |
| **Per-pose-frame wire size** | ≤ 100 bytes (text protocol) or ≤ 40 bytes (binary) | Larger than design budget; revisit framing |

Target tier for "ship": 30 clients @ 30 Hz both directions.

Acceptance fallback: 20 clients @ 30 Hz → ship phase 0 with a cap
at 20, design accommodates it.  12 clients or fewer at 30 Hz
without throwing → triggers a server architecture rethink before
phase 0.

## Probe shape

A small standalone tool (likely `tools/audience-demo-50/probe.loft`
when phase 0a lands) that adapts `load_test.loft`'s harness:

1. **Connect N synthetic clients** to a running PLAN36-pattern
   single-port server (real WebSocket, `lib/web`'s `ws_handler`).
   Plus one synthetic *projector* client that doesn't get
   sight-filtered.
2. **Each client emits** a synthetic `(L, R, L_on, R_on, t)` pose
   frame at 30 Hz.  Wire format draft: 16-byte fixed-point frame
   (`INPUT:cid,lx,rx,lo,ro,t_us` text is also acceptable for the
   probe; tighter framing is for the real client).
3. **Server's job** during the probe: receive each input
   frame, integrate a *minimal* synthetic pose (just a moving
   point — physics is out of scope), and broadcast to recipients
   per the sight-range filter (`net.peer_sight_range`).  Each
   "phone" client receives pose-frames for peers within range +
   its own pose-echo; the projector client receives all poses.
4. **Each client drains** its incoming snapshots, counts them,
   records per-frame arrival timestamps for latency histograms.
5. **Per-tier summary** at the end: connect time, per-phone
   inbound rate (out-of-range filter validation), per-phone
   outbound rate (input echo), projector receive rate, drop
   counts, latency histograms (p50 / p95 / p99).

**Synthetic position distribution.**  Clients emit pose-frames
positioning themselves spread across a virtual 100 × 100 m world
(uniform grid: `client_i` at position `((i%6) × 20, (i/6) × 20)`).
At `peer_sight_range = 80 m`, each client typically has ~9 peers
in range; of those, ~1–2 are within `peer_rate_full_radius = 25
m` (full rate), ~3–4 in the half-rate ring, and ~3–4 in the
quarter-rate outer ring.  This is the **representative case**;
the probe should also test:

- **Worst case** (all clients clustered within `peer_rate_full_radius`
  of each other → every peer gets every tick at full rate);
- **Best case** (clients spread beyond `peer_sight_range` → no
  peer pose-frames broadcast, only own-pose echoes);
- **LOD-stress case** (clients at controlled distance bands to
  verify the server is correctly applying the three-tier rate
  filter, not just sending everything to everyone).

The LOD-stress case is the one the probe specifically validates
that wasn't measured under PLAN36 — make sure to assert the
**fraction of frames each band receives** matches the design
(full ≈ 100 %, half ≈ 50 %, outer ≈ 25 %) within tolerance.

Tiers ramp `3 → 12 → 20 → 30` (PLAN36's load_test pattern with
the extra 20-client tier added) so the failure-mode degradation
curve is visible, not just pass/fail at one tier.

## Implementation notes

- **Use `lib/web::WsGroup`** for the drain side (same as PLAN36
  load_test phase 1.9) — single round-robin scan across all
  client sockets, not per-socket polling.
- **Run probe + server in separate processes** (same `loft`
  binary, different scripts), not in-process — measures the
  socket layer, not Rust function-call overhead.
- **Wall-clock timing via `ticks()`** (microsecond precision);
  use percentile aggregation in a `vector<integer>` accumulator
  flushed at end of run.
- **No JSON for input frames** — text or binary fixed-format
  framing only; JSON parsing per-frame would dominate the
  measurement on the input path.
- **Sustained run of ≥ 30 sec per tier** so any per-second
  outlier (Tier-A′ pump's read timeout, GC, etc.) shows up in
  the percentile tails.

## Where it goes when shipped

`tools/audience-demo-50/probe.loft` plus a small README in the
same directory describing:

- How to start the server (probably reuse PLAN36's
  `single_port_server.loft` with a `--echo-mode` flag, or a
  new minimal echo server).
- How to run the probe.
- How to interpret the output (per-tier summary lines + a
  final pass/fail verdict).

If pass/fail: report inline.  If degradation curve interesting:
copy the per-tier table into `doc/claude/plans/51-bumper-airplanes/NUMBERS.md` § Networking
as the empirical evidence for the `net.player_cap` parameter
default (currently 30).

## Decision point — what to do with the verdict

| Verdict at 30 clients | Action |
|---|---|
| All metrics pass | Lock `net.player_cap = 30`, proceed to phase 0 |
| Inbound rate degrades but broadcast holds | Cap input rate to whatever sustains; degraded control feel is acceptable.  Lock `net.input_rate` lower in NUMBERS.md |
| Broadcast rate degrades | Server architecture rethink: dead-reckoning + delta updates instead of full per-frame snapshots.  This is a substantial scope bump — block phase 0 |
| Drops or crashes | Pump bug to investigate; file P-issue against `lib/server`; block phase 0 |

The probe is **disposable** once phase 0 ships its real WS
loop — at that point real client traffic replaces synthetic.
Worth keeping the script archived in `tools/` for future
regression checks (e.g. after `lib/server` refactors).

## Effort

XS — half a day, mostly cribbed from `load_test.loft` with the
paint-protocol replaced by pose-protocol and the metric panel
adjusted.  No new library work, no new language features.

## Cross-references

- [`tools/audience-demo/load_test.loft`](../../../../tools/audience-demo/load_test.loft)
  — the existing harness this adapts.
- [@PLN6 phase 1.9](../6-audience-generative-art/01-server-state.md)
  — what's already validated about the underlying pump.
- [README.md § Risks](README.md) — the risk this probe closes.
- [NUMBERS.md § Networking](NUMBERS.md) — the parameters this
  probe will lock or revise.
