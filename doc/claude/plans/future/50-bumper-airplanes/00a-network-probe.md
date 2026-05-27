<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 0a — Network throughput probe

**Goal:** Validate that the existing `lib/server` Tier-A′ pump
(shipped at @PLAN36 phase 1.9, commit `a72ad22` in PR #214)
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

## What's already proven from @PLAN36

PLAN36's `load_test.loft` (in
[`tools/audience-demo/`](../../../../tools/audience-demo/))
already validated:

- **Connection establishment** at 30 clients: ~10 sec total
  after the Tier-A′ pump fix (was 220 sec before).
- **Broadcast correctness** at 30 clients: 100% fan-out, zero
  send failures, zero drops at every tier (3 / 12 / 30).
- **Crash-resistance** to malformed/hostile frames.

But the **paint** protocol PLAN36 measured is *sparse* — clients
emit one frame per user tap (typically < 1 Hz per client).  PLAN50
needs each client to emit 30 frames/sec.  That's a **~30 × higher
inbound rate** the pump has never been measured under.  Outbound
broadcast rate similarly jumps from sparse to 30 Hz × pose-snapshot.

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
2. **Each client emits** a synthetic `(L, R, L_on, R_on, t)` pose
   frame at 30 Hz.  Wire format draft: `INPUT:cid,lx,rx,lo,ro,t_us`
   (text, < 50 bytes per frame — same `lib/server`-Tier-A′ pattern
   as PLAN36).
3. **Server's only job** during the probe: receive each input
   frame and immediately broadcast a synthetic per-client pose to
   all clients (the pose can be a no-op echo with sequence
   numbers for measurement purposes — physics is not in scope).
4. **Each client drains** its incoming snapshots at 30 Hz, counts
   them, records per-frame arrival timestamps.
5. **Per-tier summary** at the end: connect time, inbound rate,
   outbound rate, drop count, latency histogram (p50 / p95 / p99).

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
copy the per-tier table into `doc/claude/plans/future/50-bumper-airplanes/NUMBERS.md` § Networking
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
- [@PLAN36 phase 1.9](../../36-audience-generative-art/01-server-state.md)
  — what's already validated about the underlying pump.
- [README.md § Risks](README.md) — the risk this probe closes.
- [NUMBERS.md § Networking](NUMBERS.md) — the parameters this
  probe will lock or revise.
