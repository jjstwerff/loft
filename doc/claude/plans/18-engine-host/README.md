<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN18 — Engine host: the C71/N9 execution-model build (one kernel, tiers, main-loop IO contract)

> **Subject:** loft   ·   **Type:** plan   ·   **Area:** native · runtime · server
> **Effort:** H   ·   **Value:** Enabling
> **Driven-by:** the @PLN6 audience demo + @PLAN50 bumper-airplanes (the dogfood
> consumers), and @PLN16's 6b/6c (hot-swap `reload` + breakpoint-in-game) which are
> gated on this plan
> **Depends-on:** —  (consumes existing substrate: `lib/server`'s pump, the wasm
> pipeline, the @PLN16 debug engine)
> **Live status · milestone · order:** [loft-lang/plans ▸ @PLN18](https://github.com/loft-lang/plans/issues/18)  ← single source of truth for lifecycle

## Thesis

Build the **lavition engine host**: a semantics-free Rust kernel (frame cycle, socket
pumps, queue machinery for three traffic classes, wire-schema-as-data, store-resident
bulk accumulation) plus the **N9 per-function dispatch table** over the shared store —
[C71](../../DESIGN_DECISIONS.md#c71--native-libraries-compile-scripts-interpret--the-steady-state-execution-model)
made buildable, with the tier model
([LAVITION § Execution granularity](../../LAVITION.md#execution-granularity--per-function-interpret-over-a-compiled-baseline)
is the canonical statement: edit → interpret instantly → background wasm swap).
Until this ships, no loft game can be **live-edited while it runs**: the IDE
(@PLN16) can launch a game only as an opaque child process, the audience demo cannot
be patched mid-show, and @PLAN50's 30 Hz pose sync has no kernel to run on. The
governing property is the **host-boundary principle**: the kernel owns *mechanics*,
loft owns *meaning* — new features land as loft functions (tier 0 instantly), never
as host recompiles.

## Phases

| Phase | Goal | Status | Outcome / ref |
|---|---|---|---|
| 00-probes | The entry-gate measurements: (a) the wasm **bridge-tax** probe (one frame-path fn: interpreted vs wasm-bridged vs native — tier 1 only earns its place if meaningfully closer to native); (b) adopt @PLAN50's [`00a` network probe](../future/50-bumper-airplanes/00a-network-probe.md) as the pump harness and settle the **~12 Hz vs 30 Hz** finding (the mechanics fix lands once in `lib/server`'s native half) | ☐ todo | |
| 01-kernel | The one-kernel loop (drain → tick → output; drift-free timing; idle backoff) + queue machinery (events queue-to-empty · state-sync conflates per sender · bulk budget-ingests into claimed store regions, publishes as a completion event) + **wire-schema-as-data** registration. Listener/connector pump roles over one shared core; window/GL feature-gated. **Accepted by migrating the @PLN6 audience server onto it** — identical observable behaviour (the existing `load_test.loft` + probe suites are the harness) | ☐ todo | |
| 02-dispatch | The N9 per-function dispatch table, **interpreter target first** (= C71 minimal): swap one fn of the *running* demo to interpret on edit, over the shared store. This phase lights up @PLN16 **6b** (hot-swap `reload`) and **6c** (breakpoint-in-game → the IDE's variables panel against a live frame) | ☐ todo | |
| 03-wasm-tier | The background promotion tier: server compiles the edited fn (loft → Rust → `wasm32`) and the kernel swaps it in at a frame boundary; a trap falls back to tier 0. Gated on the phase-00 bridge-tax probe; unlocks remote-server patching (the same swap artifact over the wire) | ☐ todo | |
| 04-state-sync-at-rate | @PLAN50's probe at **30 clients × 30 Hz** on the kernel: the fixed-rate traffic class proven at target (sight-range interest management, edge-triggered EXIT, priority keyframes), with the probe's published targets + failure thresholds. Unblocks bumper-airplanes phase 0 | ☐ todo | |
| 05-udp-frontend | The custom UDP pump frontend ([design](ENGINE_HOST.md#the-udp-pump-frontend--a-custom-layer-the-class-table-keeps-small-evaluated-2026-06-10) + [LAN notes](ENGINE_HOST.md#udp-on-a-normal-lan--whats-actually-needed-evaluated-2026-06-10)): one Gaffer-style reliable event channel, naked `seq` datagrams for state sync, bulk stays on TCP/WS; stateless-cookie handshake, keepalive-as-radio-wake, ≤1200 B datagrams, LAN discovery beacon. Phones stay `wss`; native peers ride UDP in the same world | ⏸ parked — **trigger:** an arcade consumer whose feel outgrows `wss` on the venue LAN (input→display latency budget missed with conflation + interpolation in place), measured by the phase-00 harness + a loss% axis, not assumed | |
| 06-snapshot-ring | The store **snapshot / restore / diff** primitive with a short tick-history ring — the ONE mechanics piece behind prediction, lag-compensation rewind, rollback, and delta-compressed snapshots (all loft *meaning* over it; embryos exist: the @PLN16 M2 undo journal, the store journal, record-refs #15) | ⏸ parked — **trigger:** an arcade consumer needs client prediction / hit-rewind (the bumper-planes forecast tests are the likely first caller), or replay/delta encoding gets demanded by a shipped game | |

## The goal this plan serves — arcade-style multiplayer (one machine, distributed)

This goal is an instance of the project's purpose
([GOALS § Purpose](../../GOALS.md) — **bringing fun to game developers and
players**): for *players*, the arcade machine is the most fun-dense multiplayer form
ever built — walk up, grab the stick, compete with the people in the room; for
*developers*, the same-machine model is the most fun to BUILD on — one world, one
tick, no netcode illusions to maintain, and the whole thing live-editable while
friends are playing it (the @PLN16 IDE + this kernel). Every technical choice below
serves that, not a latency spec.

**Arcade-style multiplayer is a named goal of the engine** (the user's call,
2026-06-10), and the model is precise: **a traditional LAN setup where people compete
as if they were on the same arcade machine.** The server *is* the cabinet; the LAN
only spreads the joysticks and screens around the room:

- **One authoritative world, one tick** — the server simulates THE game; every player
  acts in the *same frame of the same world*. No client-side divergence: clients send
  inputs and render authoritative state — extra joysticks + extra screens on one
  machine (linked-cabinet style).
- **Same-frame fairness for free at LAN latency.** Sub-millisecond RTT means
  input → server tick → broadcast → render fits the arcade responsiveness budget
  *without* prediction, rollback, reconciliation, or lag compensation — the
  single-machine illusion holds because the network is effectively a long controller
  cable. The thin-client model is not a limitation here; it **is** the goal.
- **Walk-up join** — grab the stick: QR/instant join (the audience demo's proven
  flow), drop-in/drop-out, no lobby ceremony.
- **The shared spectacle** — the projector/big screen is the cabinet's screen
  (exactly the audience-demo + bumper-planes shape); personal screens are controls
  and auxiliary views.

This is also why the parked phases stay parked: prediction/rollback machinery
(06-snapshot-ring) exists to *fake* the single-machine illusion **over the
internet** — the LAN/same-machine model deliberately avoids needing it. The dogfood
consumers (@PLN6, @PLAN50) are already this model; phases 00–04 give it its kernel.
The coverage evaluation ([ENGINE_HOST § Coverage](ENGINE_HOST.md#coverage--is-this-rich-enough-for-most-multiplayer-games-evaluated-2026-06-10))
guarantees nothing must be *undone* if a consumer ever outgrows the LAN; the parked
rows activate on a **consumer's measured need** (the dogfood loop), never
speculatively.

## Design

The full design is [ENGINE_HOST.md](ENGINE_HOST.md) (grown from the @PLN16
exploration; graduated to this plan). Its load-bearing sections:

- **The host-boundary principle** (the keystone): kernel = mechanics, loft = meaning;
  wire-schema-as-data; the residual that still recompiles routes to C71's library tier.
  Existence proof: @PLAN50 built all its richness in loft over `poll_event()` with
  zero Rust changes.
- **Part 1 — tiered execution** (canonical model in LAVITION): tier 0 interpret /
  tier 1 wasm swap / native baseline; why wasm is the swap artifact (unload safety,
  sandbox-not-segfault, wire-shippable, the store is already the shared ABI); the
  risk register (bridge tax, rustc latency only acceptable because tier 0 exists,
  bulk loops stay native, **the dispatch table is the real build**).
- **Part 2 — the main-loop IO contract**: three traffic classes with three drain
  rules; head-of-line blocking as the failure mode; store-resident accumulation
  (claim on OPEN, write at offset, publish a `DbRef`); backpressure via the budget;
  three wire shapes (two-channel / chunked / **decompose-into-idempotent-events** —
  the @PLN6-proven preference); the dependent-event ordering decision to settle
  before the protocol freezes.
- **One kernel, two roles**: server and client are configurations of the same crate;
  loopback testing + single-player for free; the IDE's `--serve` host converges on
  the listener role; the browser stays a separate thinner host (frame-yield contract,
  not the kernel binary).
- **Prior art**: @PLAN50 (`probe_server.loft` runs the whole loop; interpolation +
  forecast findings; the probe discipline) and @PLN6 (the event-world shape;
  snapshot-as-replayed-deltas).

## See also

- [@PLN16 IDE.md slice 6](../16-debugger/IDE.md) — the IDE surface that consumes
  phases 02/03 (6b/6c); slice 6a's `launchGame` child process is the interim
- [@PLAN50 bumper-airplanes](../future/50-bumper-airplanes/README.md) — phase-04
  consumer; its `00a` probe is phase 00's harness
- [@PLN6 audience demo](../6-audience-generative-art/README.md) +
  `tools/audience-demo/` — phase-01 acceptance consumer
- [`lib_plans/08-server`](../../lib_plans/future/08-server/README.md) — receives the
  pump-mechanics work (phase 00b) as cross-linked commits
- [LAVITION.md](../../LAVITION.md) § Engine runtime architecture — the canonical
  architecture this builds
