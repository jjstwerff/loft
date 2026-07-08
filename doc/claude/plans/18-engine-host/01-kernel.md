<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN18 phase 01 — the kernel: design notes + entry probes

> Status: **✅ complete** (2026-06-12): `src/engine_host.rs`
> (pump with the peek pattern, event queue, drift-free ticks, send/broadcast) +
> `lib/engine_host` (the budgeted-drain `run()` skeleton) pass the end-to-end test
> (`tests/engine_host_kernel.rs`: WS client → connect event → handler closure →
> broadcast round trips → tick counting via a struct world). Both acceptance items
> landed: the audience-server port (differential-green, `tests/engine_host_audience.rs`)
> and the stamp-chain re-run on the kernel (server hold p50 18.6 ms @12 ≈ the half-tick
> floor) — see the README phase-01 row.
> The loop/queue/wire design is [ENGINE_HOST.md](ENGINE_HOST.md) Part 2 + the
> host-boundary principle; this file records the **dispatch mechanics** that the
> kernel build rests on, probed before building.
> **2026-06-12 — the third role landed:** `run_local` (the standalone windowed
> host — the connector loop with no transport; ENGINE_HOST.md § Update
> 2026-06-12), driven by the crawler consumer's gap report
> [#343](https://github.com/loft-lang/loft/issues/343).

## Entry probes (2026-06-10)

**Probe 1 — persistent-State repeated dispatch: HOLDS.**
`tests/engine_host_probe.rs::persistent_state_repeated_dispatch` — one
long-lived `State` dispatches a loft fn 50× via `execute_argv` with intact
arguments throughout. This is the kernel's per-event call pattern
(interpreter-only N9 dispatch — every fn's dispatch target is the interpreter
until phase 02 adds the table).

**Finding — loft has NO mutable globals.** Top-level bindings are constants
(the parser enforces UPPER_CASE; mutation is rejected) — which is *why* every
existing server holds its world as a `main` local.

**Design pivot (2026-06-10, via the @PLN13 script-mode evaluation):** the
kernel's state model is **closure capture, not an anchor native**. @PLN13's
scripts-without-main design (plan-as-issue, future/freeze-parked) makes loose
top-level statements a *synthesized-main's locals* — confirming main-locals +
closure capture as loft's canonical mutable-state idiom. The kernel program is
therefore the **already-proven server shape**:

```loft
fn main() {
  world = World { cells: [], ticks: 0 };
  k = kernel(PORT);
  k.on_event(fn(args) { …world… });   // closures capture the world
  k.on_tick(fn() { world.ticks = world.ticks + 1; });
  k.run();                            // ← the Rust loop; never returns
}
```

`k.run()` IS the Rust kernel: main's frame lives for the program's lifetime, so
the captured world persists in the store under the existing closure-record
machinery — no language change, no anchor, and the audience-server migration
becomes nearly mechanical (`srv.run(fn(ev){…})` is already this shape).
Forward-compatible with @PLN13 (script mode just removes the `main` wrapper).
**Probe 2 — RESOLVED (2026-06-10), with a crash found on the way.** The loop
skeleton lives in the kernel's **loft library** (`run()` calling `kernel_wait`/
`kernel_next_event`/`kernel_tick_due` natives), so closures are invoked by the
interpreter's **own fn-ref call** — no Rust→closure machinery at all; every
loop decision stays behind the natives (host-boundary intact). The probed
matrix then found the registration-shape boundary:

| Closure shape | Verdict |
|---|---|
| passed as a **fn argument**, invoked in callee (the `srv.run` shape) | ✅ both backends |
| in a **local**, same-fn invoke | ✅ |
| plain fn (no captures) in a struct field, cross-fn | ✅ |
| **capturing closure in a struct field, invoked cross-fn** | 💥 **both backends** — filed [#313](https://github.com/loft-lang/loft/issues/313) (interp SIGSEGV / native OOB `u16::MAX`), `wa:clean` |

So registration is **by arguments**: `kernel_run(on_event, on_tick)` rather than
a `Kernel { on_event: fn… }` struct (until #313 lands). Final probe: a library
`run` loop invoking arg-closures 100× with captures mutating throughout —
**identical results on interp and native** (`events=100 ticks=34`). The
dispatch mechanic is fully proven in pure loft.

**Harness lesson (not a compiler bug):** two codegen panics during probing
(`Incorrect var _elm_3…`, `Incorrect stack…`) were both *my harness compiling a
program that already had parse errors* (the lowercase-global constant rule) —
diagnostics must be checked before `byte_code`, exactly as `load_program` does.
The kernel's program loader inherits that rule. (Also re-confirmed the
bind-indexed-lookup-to-a-local-first pattern for hash mutation.)

## The v1 kernel shape (what gets built next)

- `src/engine_host/` in the loft crate (phase 02 needs deep `State` access;
  extraction into the lavition repo is a later, mechanical move).
- **Loop:** drain (budgeted) → tick when due (drift-free `last += INTERVAL`) →
  flush output → idle backoff — the @PLN51 `probe_server` loop, in Rust.
- **Pump v1:** the WS listener (reusing `src/serve.rs`'s frame/handshake code)
  on a pump thread, **non-blocking reads with the peek pattern** (the phase-00
  fix, native from day one), feeding an mpsc the loop drains.
- **Classes v1:** events only — the audience server is pure events; conflation
  slots + bulk accumulation land when the consumer (@PLN51 / assets) arrives,
  per wire-schema-as-data registration (table present from day one, one row).
- **Dispatch v1:** parse+compile the program once (diagnostics-checked), run
  `main` once; main calls the kernel library's `run(on_event, on_tick, port)`
  (closures **as arguments**, per #313), whose loft-side loop body invokes them
  via ordinary fn-ref calls while `kernel_wait`/`kernel_next_event`/
  `kernel_tick_due`/`kernel_send` natives carry all mechanics (pumps, queues,
  budgets, drift-free ticks) in Rust.
- **CLI:** `loft host <program.loft> [--port N]`.
- **Acceptance:** the @PLN6 audience server's meaning ported onto handlers —
  identical observable behaviour under the existing probe/load tools; then the
  00(c) stamp chain re-run: the ~40 ms interp-harness term must collapse.


## Build findings (the v1 core, 2026-06-10)

- **The text-return ABI bit hard:** a native returning `text` must be
  **destination-passing** (@PLN10's convention — caller allocates, the native
  `push_str`s into the passed `DbRef`, the base name routed by
  `is_text_dest_native`). My first `n_kernel_event_payload` pushed a `String`
  directly and corrupted the stack — every program died on FIRST EVENT
  (SIGSEGV/SIGABRT/panic by shape) while startup smokes stayed green. Ladder
  lesson: **smoke the event path, not just the listen line.**
- **Two pre-existing closure-capture crashes filed en route**, both with probed
  matrices + clean workarounds: [#313](https://github.com/loft-lang/loft/issues/313)
  (capturing closure in a struct *field*, cross-fn invoke) and
  [#314](https://github.com/loft-lang/loft/issues/314) (bare scalar captured by a
  reader closure + a writer closure). **Struct-held world state avoids both** —
  and is the natural kernel idiom; `lib/engine_host`'s docs say so.
- The kernel lib trips the auto-native cdylib build (the natives are in-binary;
  the cdylib can't link them) and falls back to interpret with a warning —
  cosmetic for now; the right fix is a manifest mark for in-binary-native
  libraries (or routing through codegen_runtime for `--native` programs), noted
  for phase 02/03.

## The audience-server port + differential acceptance (2026-06-10)

`tools/audience-demo/server_kernel.loft` is the @PLN6 audience server's meaning
on the kernel: identical structs/helpers (Cell/Player/World, seed/erase/ignore/
clear/select/snapshot/join-replay), `main` holds the world in a struct and
passes `on_event`/`on_tick` closures to `engine_host::run`.
`tests/engine_host_audience.rs` drives ONE protocol scenario against BOTH
servers and asserts the per-client transcripts are equal — green (kernel ≡
original, ~1.5 s).

**The acceptance test surfaced a pre-existing parser bug** (all binaries, not
this branch): in package mode the package root sits in `lib_dirs` (that is what
makes intra-package `use otherfile;` work), so a package file named exactly
like a declared dependency SHADOWED it — `use server;` inside
`tools/audience-demo/server.loft` resolved to the file itself, the `server`
library never loaded, and `server::Server` came back "Undefined type". Every
program in the package broke the same way (`single_port_server.loft` included);
nothing CI-ran them, so the demo rotted silently. Diagnosis ran the full
matrix: the "kernel never listened" symptom was actually the ORIGINAL server's
parse failure in the test's second spawn — the kernel block had been passing.
Fixed at the chokepoint (`Parser::lib_path`): a name declared under
`[dependencies]` never resolves to a file inside the declaring package
(`package_declared_deps` + the `blocked()` guard, canonical-path compare).
Regression: `tests/fixtures/dep_shadow/` +
`package_layout::declared_dep_beats_same_named_package_file`.
`single_port_server.loft` additionally needed `as i32`/`as u8` casts (implicit-
narrowing strictness landed after it was written) — fixed.

Also landed: `bind_reuseaddr` (SO_REUSEADDR on unix) in the kernel listener —
a restarted cabinet must rebind through TIME_WAIT.

Remaining for phase 01: the 00(c) stamp-chain re-run on the kernel (the ~40 ms
interp-harness term must collapse) and optionally `load_test.loft` at the
kernel port.

## The 00(c) stamp chain on the kernel — phase acceptance (2026-06-10)

`tools/audience-demo-50/probe_server_kernel.loft` is `probe_server.loft`'s
meaning on the kernel (same wire protocol + port 18084); the unchanged
`probe.loft` client drives both.  12 clients, 12 s, 30 Hz, echo every 200 ms:

| metric (µs) | lib/server baseline | kernel | floor |
|---|---|---|---|
| server hold p50 (t5−t1, one clock, exact) | 253 561 | **18 600** | 16 667 (half-tick — the by-design next-tick wait) |
| server hold max | 1 138 862 | 32 178 | 33 333 (one tick) |
| total turn-around p50 | 368 494 | 56 477 | |
| wire+client legs p50 | 114 933 | 37 877 | (the probe client's own interp drain — the measuring instrument, not the kernel) |

- **The acceptance metric held:** the interp-harness pump term collapsed —
  hold residual above the half-tick floor is **~1.9 ms** (was ~237 ms over
  floor on the unfixed lib/server pump; the loft-libs-net peek fix brings
  that baseline down too, but the kernel needs no library fix at all — the
  pump IS the Rust peek loop).
- **N-stable:** 30 clients → hold p50 22 756, max 39 537 (tick-bounded; the
  old pathology scales N×21 ms ≈ 630 ms).  Full pose fan-out sustained:
  center clients receive ~29.8 poses/tick = **30 clients × 30 Hz through the
  interpreted kernel loop** (~21 k sends/s) — the phase-04 checkpoint hit at
  probe level.
- The audience `load_test.loft` run is superseded by this richer 30-client
  probe (same class, more load, plus the stamp chain).

**Phase 01 verdict: complete.**  Built: the kernel natives + `lib/engine_host`
loop skeleton, both acceptance gates green (audience differential + stamp
chain).  Deliberately NOT built (no consumer yet — they land with the phase
that needs them, not as scaffolding):

- **Conflation slots + bulk class** → 05a (UDP state-sync datagrams) is their
  first real consumer; the event queue is the only class with traffic today.
- **Wire-schema-as-data table** → lands with the second traffic class (a
  one-row table dispatching nothing is machinery without meaning).
- **Connector role** (client-side kernel, window/GL feature-gate) → first
  native client (phase 04 / bumper-airplanes).
- **Stamp-at-queue-boundary debug primitive** → with the IDE pipeline panel
  (@PLN16 follow-up); the loft-side chain (t1 at drain via `ticks()`) already
  measures what phases 01–05a need.
