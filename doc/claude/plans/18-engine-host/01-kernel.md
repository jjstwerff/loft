<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN18 phase 01 — the kernel: design notes + entry probes

> Status: **◐ in progress** (entered 2026-06-10 after phase 00 closed at v1).
> The loop/queue/wire design is [ENGINE_HOST.md](ENGINE_HOST.md) Part 2 + the
> host-boundary principle; this file records the **dispatch mechanics** that the
> kernel build rests on, probed before building.

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
**Probe 2 becomes:** Rust invoking a registered loft closure re-entrantly —
precedent: `parallel.rs` dispatches loft fns from Rust worker threads.

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
  flush output → idle backoff — the @PLAN50 `probe_server` loop, in Rust.
- **Pump v1:** the WS listener (reusing `src/serve.rs`'s frame/handshake code)
  on a pump thread, **non-blocking reads with the peek pattern** (the phase-00
  fix, native from day one), feeding an mpsc the loop drains.
- **Classes v1:** events only — the audience server is pure events; conflation
  slots + bulk accumulation land when the consumer (@PLAN50 / assets) arrives,
  per wire-schema-as-data registration (table present from day one, one row).
- **Dispatch v1:** parse+compile the program once (diagnostics-checked), run
  `main` once; `kernel(port)` / `k.on_event(fn…)` / `k.on_tick(fn…)` natives
  register closure values; `k.run()` enters the Rust loop, which invokes the
  registered closures per event/tick (probe 2's mechanism; `parallel.rs` is the
  precedent for Rust-side loft-fn invocation).
- **CLI:** `loft host <program.loft> [--port N]`.
- **Acceptance:** the @PLN6 audience server's meaning ported onto handlers —
  identical observable behaviour under the existing probe/load tools; then the
  00(c) stamp chain re-run: the ~40 ms interp-harness term must collapse.
