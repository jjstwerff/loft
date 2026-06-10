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

**Current focus (the user's call, 2026-06-10): this is the main goal.** Get the
**Rust version of the main loop running** (phase 01, entered through phase 00's
measurements) and make a **quick UDP channel possible beside the websockets**
(phase 05a — the minimal cut: datagrams for the state-sync class only, everything
reliable stays on WS). The arcade-machine model (§ below) is what these two serve.

## Phases

| Phase | Goal | Status | Outcome / ref |
|---|---|---|---|
| 00-probes | The entry-gate measurements: (a) the wasm **bridge-tax** probe (one frame-path fn: interpreted vs wasm-bridged vs native — tier 1 only earns its place if meaningfully closer to native); (b) adopt @PLAN50's [`00a` network probe](../future/50-bumper-airplanes/00a-network-probe.md) as the pump harness and settle the **~12 Hz vs 30 Hz** finding (the mechanics fix lands once in `lib/server`'s native half); (c) **the turn-around stamp chain** — extend the probe's `t_us` seed into a per-phase breakdown of the full round trip (`t0` input → `t1` pump recv → `t2` drained → `t3` tick consumed → `t4` broadcast enqueued → `t5` sent → `t6` client recv → `t7` rendered): server-local deltas need no clock sync (one clock — exposes drain lag, **tick wait** the dominant hidden phase, send-queue time exactly), the wire legs come from the echo (RTT = (t6−t0) − (t5−t1), split /2 on a symmetric LAN). Baseline the LAN first (`ping` / `iperf3 -u` / `ss -ti`, check `TCP_NODELAY`) so the loop is never debugged against a wifi problem. The stamp chain graduates into a **debug-mode kernel primitive** in phase 01 (stamp at each queue boundary when enabled) — which later feeds the IDE a live pipeline-breakdown panel | ✅ v1 complete (2026-06-10) | (a) exec ratios: interp 6×, **wasm ≈ 1.0× native** (bridge cost → 03); (b) root-caused (20 ms blocking poll × in-order scan; sweep 252 ms→17 µs @12, 6 µs @30; fix on `loft-libs-net:pump-fast-idle-poll`, lands with the arc) + 30 Hz held at 30 clients; (c) stamp chain live — hold p50 ≈ half-tick (the tick-wait, measured), ~40 ms residual = interp harness (the kernel's acceptance metric). [probes/00b-baseline](probes/00b-baseline-2026-06-10.md). Baseline reproduced 2026-06-10 — [probes/00b-baseline](probes/00b-baseline-2026-06-10.md): 12 clients × 10 s → ~27–45% of target pose rate (the ~12 Hz band) + **a new finding: EXIT events drop under pose load** (4/11 clients missed a must-deliver event on reliable transport → the pump starves the event class; the class-separation argument, measured) |
| 01-kernel | The one-kernel loop (drain → tick → output; drift-free timing; idle backoff) + queue machinery (events queue-to-empty · state-sync conflates per sender · bulk budget-ingests into claimed store regions, publishes as a completion event) + **wire-schema-as-data** registration. Listener/connector pump roles over one shared core; window/GL feature-gated. **Accepted by migrating the @PLN6 audience server onto it** — identical observable behaviour (the existing `load_test.loft` + probe suites are the harness) | ✅ complete (2026-06-10) | [01-kernel.md](01-kernel.md) — natives (src/engine_host.rs) + `lib/engine_host` loop skeleton; closures-as-arguments (#313), struct-held world (#314); **audience server ported + differential acceptance green** (`tests/engine_host_audience.rs`: kernel ≡ original transcripts); **00(c) stamp chain re-run on the kernel**: server hold p50 18.6 ms @12 ≈ the half-tick floor (16.7 ms by-design tick wait + ~1.9 ms drain; baseline 253.6 ms — the interp-harness pump term collapsed), N-stable @30 (22.8 ms, tick-bounded) with full 30×30 Hz pose fan-out (~29.8 poses/tick at center, ~21 k sends/s) — the phase-04 checkpoint at probe level. En-route fix: package-file-shadows-declared-dep parser bug (`Parser::lib_path` guard + `tests/fixtures/dep_shadow/`). Deferred: bulk class (05c). Conflation slots + the wire-schema table (minimal form) + the **connector role** (`run_client`) all landed in/after 05a; the in-binary-native cdylib warning fixed by `[native] in_binary` (05a) |
| 02-dispatch | The N9 per-function dispatch table, **interpreter target first** (= C71 minimal): swap one fn of the *running* demo to interpret on edit, over the shared store. This phase lights up @PLN16 **6b** (hot-swap `reload`) and **6c** (breakpoint-in-game → the IDE's variables panel against a live frame) | ☐ todo | |
| 03-wasm-tier | The background promotion tier: server compiles the edited fn (loft → Rust → `wasm32`) and the kernel swaps it in at a frame boundary; a trap falls back to tier 0. Gated on the phase-00 bridge-tax probe; unlocks remote-server patching (the same swap artifact over the wire) | ☐ todo | |
| 04-state-sync-at-rate | @PLAN50's probe at **30 clients × 30 Hz** on the kernel: the fixed-rate traffic class proven at target (sight-range interest management, edge-triggered EXIT, priority keyframes), with the probe's published targets + failure thresholds. Unblocks bumper-airplanes phase 0 | ☐ todo | |
| 05a-udp-quick-channel | **The quick UDP channel beside the websockets** (in scope per the user's call, 2026-06-10): a datagram channel for the **state-sync class only** — events + bulk **stay on WS**, so no reliable-UDP layer, no fragmentation (poses are small), no congestion control is needed; just the stateless-cookie handshake (identity = source 4-tuple + cookie tied to the WS session), `seq`-stamped ≤1200 B datagrams into the conflation slots, keepalive-as-radio-wake, host-firewall note in the runbook. The minimal cut the class table makes possible — native peers gain the UDP fast path while phones stay `wss`, same world | ✅ v1 shipped (2026-06-10) | [05a-udp.md](05a-udp.md) — same-port UDP socket, `H:`/`A:` cookie handshake, negotiated fully kernel-internally (`X-Loft-UDP` header on the WS 101 — no loft code touches it; GOALS § Goal F engine-surface instance), conflation slots live (first consumer), `sync_send` transport-transparent (UDP when bound, WS fallback), 3 s keepalive timeout, ≤1200 B cap. E2E green (`tests/engine_host_udp.rs`: conflate-to-newest, stale-discard, timeout-revert; + the real-consumer proof — probe_server_kernel declares `sync_class(2/5)` and uses ordinary `send` everywhere; web client gets WS / native client gets datagrams from one call site). **Classes are declarative** (wire-schema table, minimal form): `sync_class(msg_id)` once at startup; no per-call transport API (`sync_send` absorbed); inbound conflation per (sender, msg_id). **Transport selection is automatic per client** (user-confirmed contract): a native client hellos (it CAN speak UDP) and `sync_send` rides datagrams; a web page cannot send UDP, never binds, and the same call stays WS — meaning-code never branches on transport. The connector kernel (`run_client`, landed) auto-performs the hello — automation is end-to-end for native seats with zero client transport code (`tests/engine_host_connector.rs`) |
| 05b-udp-full-frontend | The full custom UDP frontend ([design](ENGINE_HOST.md#the-udp-pump-frontend--a-custom-layer-the-class-table-keeps-small-evaluated-2026-06-10) + [LAN notes](ENGINE_HOST.md#udp-on-a-normal-lan--whats-actually-needed-evaluated-2026-06-10)): the Gaffer-style reliable event channel, MTU fragmentation, pacing, DTLS, LAN discovery beacon | ⏸ parked — **trigger:** a consumer needs reliable traffic off TCP (measured by the phase-00 harness + a loss% axis), or the discovery beacon gets demanded by a LAN consumer | |
| 05c-udp-bulk | **Bulk over UDP — the one-to-many push** (evaluated 2026-06-10, user-directed): the arcade-cabinet shape is N seats receiving the SAME level/world/asset pack — TCP must send N copies (30 seats × 100 MB = 3 GB through the cabinet NIC); UDP **broadcast/multicast sends it once** to every wired seat, NACK rounds mop up each seat's losses individually. The custom piece is a NACK-based chunk protocol (~200–300 lines kernel mechanics): ≤1200 B chunks, receiver bitmap, NACK rounds at tick cadence, sender pacing (token bucket — unpaced floods silently drop 30–50% in the receiver's socket buffer), integrity check; simpler than a general reliable channel (no ordering — complete-or-nothing). Slots in as a third frame type (`B:`) over 05a's socket + cookie handshake, budget-ingests into claimed store regions, publishes a completion event (the bulk-class design in ENGINE_HOST.md). Wifi/phone seats fall back to unicast chunks or plain WS (broadcast is base-rate + AP-filtered on wifi — wired-seats-only win). For a SINGLE receiver TCP is already ≈line-rate on a LAN — that case stays on WS by design. **Broadcast is measured, never assumed** ([design](ENGINE_HOST.md#broadcast-bulk-measured-never-assumed-user-directed-2026-06-10)): the probe burst + per-seat bitmap acks decide who rides broadcast; seats demote to unicast/WS mid-transfer when their measured loss flips the math — chunks are transport-fungible, so fallback is not a mode switch | ⏸ parked — **trigger:** a consumer that pushes big payloads to many seats (level/world push; phase-04 / bumper-planes era or the asset pipeline) | |
| 05d-services | **The service surface** (user-designed 2026-06-10, [design](ENGINE_HOST.md#services--register-meaning-assign-speed-later-design-verified-2026-06-10)): register services (listener & writer combined — the ONE home for a message kind), then assign lanes LATE (fast = state-sync / normal = events / slow = bulk) — one reversible line per service, after meaning works; the lane never leaks into service code. Dissolves the hand-rolled `handle_message` if-chains; `sync_class` (landed) is this table's lane column; `on_event` stays as the default service so consumers migrate one service at a time | ◐ design verified — **gate:** the #313 closure fix (`bugs321`, fixed-pending-merge) must land, then probe the handler-storage matrix (field ✓ expected, vector = open cell) before building the registry | |
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
- **Minimal latency is a goal in its own right** (the user's call, 2026-06-10) —
  short-term accuracy is appreciated, **latency wins when they conflict**. Concretely:
  - The **latency budget is dominated by self-inflicted waits, not the wire**: tick
    wait (an input arriving just after a tick waits a full interval — up to 33 ms at
    30 Hz, dwarfing ~1 ms LAN RTT), broadcast batching, client frame pacing. The
    phase-00 stamp chain is this goal's instrument — every wait phase measured, then
    minimized (send immediately after the tick; apply inputs at drain time; late-latch
    the freshest state into the render; `TCP_NODELAY` on the WS path).
  - **The interpolation trade flips**: smoothing renders the *past* (@PLAN50's Hermite
    work buys accuracy at 1–2 ticks of display lag). Under this goal the client
    renders the **newest state** (or forward-extrapolates) by default; interpolation
    delay becomes a tunable that defaults toward zero, not toward smooth.
  - The quick UDP channel (05a) serves this goal directly: no retransmit stalls, no
    in-order head-of-line — a stale-but-late pose is *discarded*, never waited for.
  - **The keystone: the advance-collision-forecast protocol** (already prototyped —
    @PLAN50's `forecast_test.loft`: the server detects imminent bounces *ahead of
    time*; the measured open question is how often an input change inside the
    lookahead window invalidates the forecast). Forecasting converts latency into
    **lookahead**: a forecast event (`bounce at t+Δ at X`) is broadcast *before its
    effect time*, so clients schedule it and render it **at the right instant** —
    effectively zero perceived latency exactly where latency is *felt* most in an
    arcade game (collisions, hits). Continuous motion is covered by
    newest-state/extrapolation (above); **discontinuities are covered by
    forecast-ahead events**, with a correction sent on invalidation (the Q2
    invalidation rate is what bounds how often a correction flickers). This is
    loft-side *meaning* over the kernel — it needs the event class + timestamps, not
    the parked snapshot ring — so it rides phases 01/05a, not 06.
  - **Cheating for feel is sanctioned** (the user's call, 2026-06-10): the game may
    deliberately bend truth where it makes play *feel* better — and the same-machine
    model makes this **fair by construction** (one authority → everyone sees the same
    bend; no per-client divergence, unlike internet-netcode trickery). The deepest
    form rides the forecast keystone: **promise-keeping physics** — once a forecast
    is broadcast, the server *prefers making it true* (nudging the sim within a
    tolerance) over issuing a correction; truth follows presentation. Corrections are
    reserved for genuine invalidation (a real input change), so correction flicker —
    the worst feel-killer — approaches zero. The same license covers the classic
    arcade feel-cheats at meaning level: input backdating (eat latency in the sim),
    favor-the-player collision tolerances, input buffering / coyote time. All loft
    *meaning*; the kernel never knows the game is lying.
- **Two participation tiers — and the phone is the on-ramp, not the end goal.** The
  **full arcade seat** is a *native client*: a real screen, real input (gamepad /
  keyboard / cabinet stick), full rendering — that is what the experience is designed
  toward, and what the native(/eventually UDP) path serves. The **phone-browser
  client** is the *walk-up tier*: QR-scan, zero install, in the game in seconds (the
  audience demo's proven flow), drop-in/drop-out, no lobby ceremony — how a bystander
  joins the fun instantly, within what a phone's input/display affords. Easy
  participation by design; never the bar the game is built against.
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
