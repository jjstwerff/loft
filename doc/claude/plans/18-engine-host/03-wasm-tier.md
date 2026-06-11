# Phase 03 — tier 1: the background WASM swap

**Goal.** The function that dropped to the interpreter (tier 0) quietly
returns to ~compiled speed when a background build lands: loft → Rust →
`wasm32` for that ONE fn, hot-swapped at a frame boundary through the same
dispatch table phase 02 built.  A trap falls back to tier 0 — the live loop
degrades, never dies.  The same artifact shipped over a wire live-patches a
REMOTE kernel (dev-to-ops, one mechanism).

**The invariant.** *Tier 1 changes a dispatch target, nothing else* — same
table, same identity, same frame-boundary atomicity, same parity guarantee.
If tier 1 ever needs the consumer to know it exists, the design is wrong
(GOALS § F).

## Entry gate — already passed, with one open number

The phase-00 bridge-tax probe measured **wasm exec ≈ 1.0× native** (the
tier-1 speed claim holds for execution) and **interp ≈ 6×** (the gap tier 1
closes).  The OPEN number is the **per-call bridge tax** in the engine
context: a swapped fn is *call-bridged* (wasm linear memory cannot alias the
host store), so every store access from inside the fn crosses a host bridge.
The wasip2 runtime (WASM.md) already runs whole programs this way; what the
gate must measure is a FRAME-PATH fn with realistic store traffic:

> **Gate probe (slice 1):** one pose-update-shaped fn (read 30 records,
> write 30) as native / interp / wasm-bridged inside a kernel tick.  Tier 1
> earns its place iff wasm-bridged lands meaningfully closer to native than
> to interp — otherwise tier 1 narrows to store-light logic fns and the doc
> says so.

## What already exists

| Piece | State | Where |
|---|---|---|
| loft → Rust → wasm32 per-program | **shipped** (`--html` cdylib pipeline + wasip2 rlib) | [WASM.md](../../WASM.md), [HTML_EXPORT.md](../../HTML_EXPORT.md) |
| Store access from wasm | **shipped** (host bridges; the store is the ABI) | WASM.md host bridges |
| The dispatch table + frame-boundary swap | **phase 02** | [02-dispatch.md](02-dispatch.md) |
| Background builds that never block a run | **shipped per-library** (N3; first-run-after-settle is foreground — its polish item moves it back) | NATIVE.md § N9 |
| wasmtime on the dev box | available (verified) | env |

## Design sketch

- **The artifact.** One fn compiled into a tiny wasm module exporting that
  fn over the existing bridge ABI.  Per-fn modules keep swap = drop-old-
  instance (unload safety is THE reason wasm beat a native cdylib here —
  ENGINE_HOST Part 1).
- **The runtime.** The kernel process embeds a wasm runtime for tier 1.
  This is a REAL dependency decision: wasmtime is heavy (compile time,
  binary size) — gate it behind a cargo feature (`--features wasm-tier`),
  default-off, so the base loft binary stays lean; the lavition engine build
  turns it on.  (Falsify first: measure the build/binary cost before
  committing — if prohibitive, the alternative is the browser-side pattern
  in reverse: the IDE hosts the runtime and the kernel only swaps natives,
  which weakens the remote-patch story — record the trade if it comes to
  that.)
- **The build flow.** Tier-0 flip happens instantly (02); the kernel (or the
  `--serve` host) enqueues a background single-fn build: reuse the N9 cdylib
  codegen with a `wasm32-wasip2` target + the bridge wrapper instead of
  `LibArg`.  On success: `dispatch[fn] = Wasm(instance, export)` queued to
  the next frame boundary.  On compile failure: stay tier 0, report to the
  IDE (a warning, never a halt).
- **The trap contract.** A runtime trap in the swapped fn (OOB, unreachable,
  bridge misuse) is caught at the dispatch point → `dispatch[fn] =
  Interp(d_nr)` (back to tier 0) + one warning with the trap reason.  The
  sandboxing IS the feature: a bad edit cannot segfault the cabinet
  mid-evening.
- **Remote patching (the ops story).** The artifact + its fn identity +
  the source hash it was built from travel over the wire (WS bulk class —
  or 05c when it lands); the receiving kernel verifies the hash matches its
  own source state before swapping.  Same table, same boundary, same trap
  fallback.  v1 scope: localhost loopback proof; the trust/signing story
  joins the registry's existing Ed25519 machinery when this goes cross-
  machine for real.

## Slices

1. **The gate probe** (above) — falsify the bridge-tax claim in the frame
   path before any building.
2. **Feature-gated wasmtime embed** + one hand-built module swapped through
   the 02 table on a loopback kernel; trap → fallback proven (a module that
   traps on its Nth call).
3. **The single-fn build pipeline** (N9 codegen reuse, wasip2 target,
   background).  Acceptance: the 02 slice-3 edit scenario, extended — edit →
   instant (tier 0) → within seconds the stamp chain shows the fn's cost
   drop to ~native with zero frame hitch at the swap boundary.
4. **Remote swap over loopback**: ship the same artifact to a second kernel
   process via the bulk path; verify hash-gated apply + trap fallback.

## Risks / honest residuals

- **The bridge tax may split the fn population** — store-heavy fns may stay
  better at tier 0-then-native-rebuild; the table doesn't care (targets are
  per-fn data), but the doc must state which fns tier 1 serves.
- **wasmtime dependency weight** — measured before committed (slice 2 gate).
- **Parity across THREE targets** is the standing Goal-D obligation; the
  differential sweep gains a wasm-swapped column for dispatched fns.
- **Browser kernels don't get tier 1 v1** (no wasmtime in wasm) — the
  browser already IS wasm; its tier model is the wasm interpreter + future
  module-relink, explicitly out of scope here (ENGINE_HOST's "honest
  residual").
