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
existing server holds its world as a `main` local. Consequence for the kernel's
inversion of control: **the world is STORE-ANCHORED** — the kernel keeps the
world's `DbRef` and hands it to handlers through a kernel-registered native
(`State::static_fn`, ABI `fn(&mut Stores, &mut DbRef)` with `stores.put(stack,
…)` returns — the same registration `native.rs` uses). loft side:
`fn world() -> World; #native "n_kernel_world"`. Probe 2 (with the skeleton)
exercises the DbRef round-trip through that native.

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
- **Dispatch v1:** parse+compile the program once (diagnostics-checked), one
  persistent `State`, `execute_argv` per handler call; world via the anchor
  native. Handlers: `init()` (build world, hand it to the kernel),
  `on_event(args: vector<text>)`, `on_tick()`.
- **CLI:** `loft host <program.loft> [--port N]`.
- **Acceptance:** the @PLN6 audience server's meaning ported onto handlers —
  identical observable behaviour under the existing probe/load tools; then the
  00(c) stamp chain re-run: the ~40 ms interp-harness term must collapse.
