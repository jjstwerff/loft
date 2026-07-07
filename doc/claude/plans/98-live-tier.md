<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 98 — Live/debug tier: one primitive (the interpreter over the shared live store)

**Status:** DESIGN (2026-07-07) · [`@PLN98`](https://github.com/loft-lang/plans/issues/98) ·
`subject:loft` · design-doc-first (Design Protocol 1), no code yet. Consumers: the `@PLN16` debugger,
the game / `engine_host` loop, and `routing`'s offline `--html` build (its `loft-feedback.md` 2026-07-07).

---

## The invariant (one sentence)

> **The interpreter evaluates a (possibly edited) function against the SAME live `Stores` the compiled
> program runs over — a swap of the world into the parked `State`, never a copy — so the compiled fast
> path and the interpreted live path observe ONE heap.**

Every live/debug capability anyone has asked for — a breakpoint's `eval`, `setValue`, hot-reload, a
game's on-the-fly script edit, a browser live-edit — is a *use of this one primitive*. This plan is not
"build a debugger" or "build hot-reload"; it is **make that one primitive complete and correct**, then
let the three consumers ride it.

## Why this is a design, not a feature list (the tell)

Three separate-looking requests — an agent debugger (`@PLN16`), the game changing scripts without a
restart, and `routing`'s offline browser build — all reduce to the *same sentence*: "run this function
interpreted, against the live heap, without stopping the program." If they were three mechanisms, that
triplication would be the brittleness. They are **one** mechanism, and loft already has it. The design's
whole job is to drive `N` (the number of places that re-assert the invariant) toward 1 and fix the one
place it is currently broken.

## The primitive that already exists (grounding)

`live_dispatch` (`@PLN18`/`@I78`, `src/live_dispatch.rs`) is the target architecture in embryo:

- A `--native` binary under `LOFT_LIVE_FLIP=1` **bootstraps a parked interpreter** `State` from the same
  sources the binary was generated from (`LOFT_LIVE_SRC`/`_STDLIB`/`_LIBS`).
- **The program's `Stores` IS the bootstrap's world** — shared by a swap-in/swap-out into `state.database`
  around each dispatched call (`State::reenter`/`reenter_ret`), *not* an alias or a copy.
- Every generated user fn opens with a one-atomic-load `live_flipped(idx)` entry check (cold cost when
  off: a single relaxed load).
- **Flipping** a fn routes its calls into the interpreter (`live_call_void`/`_i64`/`_ref`/…) over those
  same stores.
- **Tier-0 live reload** (`@PLN18` 08-S3): an edit repoints `state.fn_positions[d_nr]`; the flipped fn
  then runs the *new* body against the *live* state. The code position is deliberately resolved per
  dispatch (never cached) so an edit is picked up immediately.
- A **breakpoint** flips a fn and runs it under `State::reenter_dbg` (the debug re-entry; suspensions
  block in the pause loop).
- **Failure posture:** every bootstrap problem WARNS and falls back to fully compiled execution — live
  mode is an instrument, never a halt. Single-threaded by construction (the check fires on the
  bootstrap thread; worker threads fall through to the compiled body).

That is the invariant, already realised for `--native`. The three gaps below are where it is incomplete.

## The three consumers, one mechanism

| consumer | a "live action" is… | status today |
|---|---|---|
| **Debugger** (`@PLN16`) | breakpoint = flip + `reenter_dbg`; `eval` = interpret an expression against the paused frame; `setValue` = write a frame var | breakpoints/step/frame-read work; **`eval`/`setValue` broken on any frame with a `vector` local** |
| **Game on-the-fly scripts** | edit a `.loft` fn → `fn_position` repoints → next call runs the new body over the live game world (no restart, no state loss) | works via `live_reload` on `--native`; the `engine_host` wiring is the confirm |
| **Offline / browser** | the *same* parked interpreter compiled into the `--html` wasm module, fed sources through `host_input` instead of the filesystem | **not yet** — the wasm-portability gap |

## Failure paths (what breaks — the design's core)

Enumerating how it breaks is where the design lives (Design Protocol 1: write the failure paths first).

### F1 — the evaluator dies on store-backed locals (the load-bearing bug)

`debug_eval`/`debug_set` (in `src/state/debug.rs`, reached over `--rpc` via `src/rpc.rs`) return
`value:null` / `"edit rejected"` **whenever the paused frame holds a `vector` local** — on BOTH
`--interpret` and `--native`, for *any* expression (even a literal `2 + 2` that references no local),
and even when the vector is DEAD at the breakpoint. Scalars and **structs** in the frame are fine (both
are `DbRef`-backed, so the fault is vector-specific, not "any heap value"). This is the **shared
primitive's evaluator failing**: reflecting the paused frame's var-table into the interpreter's eval
scope aborts the *whole* eval on the vector slot rather than binding it. Fixing this one site fixes the
debugger *and* every future eval-based tool. **The fix is the proof** — this is the first thing built.

*Minimal repro (routing, verified):* delete the `v` line and `eval "2 + 2"` returns `4`; keep it and it
returns `null`.
```loft
fn main() {
  v: vector<integer> = [1, 2, 3];
  x = 2 + 2;
  println("{x} {len(v)}");
}
```

### F2 — no source delivery without a filesystem (wasm)

`live_dispatch` bootstraps from sources read off the **filesystem** (`LOFT_LIVE_SRC` + file reads). A
`--html`/wasm build has no filesystem and a print+GL-only host surface. The parked `State` is Rust and
*can* live in the wasm module (single-threaded, which wasm is — the swap model holds), but its **sources
must arrive over a channel**: the `host_input` primitive that already shipped (loft#476) is the delivery
path. So the source loader must abstract *filesystem-vs-channel* behind one seam.

### F3 — the entry check has no opt-out (packaging)

`live_flipped(idx)` is emitted at the top of **every** generated function (confirmed in `--native`
output). That is right for a live/debug build and wrong for a lean release. The tier must be **opt-in**:
a codegen flag gates (a) the per-fn entry-check emission, (b) the parked-interpreter bootstrap, and, for
wasm, (c) the interpreter's inclusion in the module — so the lean build is zero-cost-absent and the
live/debug build pays for what it uses.

## Re-assertion sites — the tell (`N` is small, on purpose)

The invariant must hold at exactly two kinds of site:

1. **World-swap sites** — every place the live world moves into/out of the parked `State`. These already
   funnel through the `live_call_*` helper family: **one chokepoint**, `N ≈ 1`. Good.
2. **Frame-reflection site** — the single place `debug_eval`/`debug_set` builds an eval scope from a
   paused frame. The F1 bug is that this ONE site does not handle every value kind uniformly.

So `N` is already near 1 — the primitive centralises the swap. The design must **keep it there**: the F1
fix belongs at the single reflection chokepoint, handling a vector slot the way it already handles a
struct slot — *not* a per-value-kind spray. If the fix wants to grow a vector-specific path, that is the
alarm (the invariant would be wider than "reflect the frame uniformly") and routes back to the search.

## The design, per axis

### A1 — fix the evaluator (F1): reflect every value kind uniformly

The frame→eval-scope reflection must bind a store-backed (`DbRef`) `vector` local the same way it binds
a `struct` local (which works) — both are `DbRef`s into the shared store. The abort is a missed branch
or an over-strict guard on the vector slot; pin the exact line (`debug_eval` scope construction in
`state/debug.rs`), then bind vectors uniformly. `setValue` (`debug_set`) is symmetric — a scalar edit
already works; a vector-slot frame must not reject the *whole* edit.

- **Acceptance:** in a vector-local frame, `eval "2 + 2"` → `4`, `eval "len(v)"` → the length,
  `eval "v[0]"` → the element; `setValue x = 42` edits; all on BOTH backends, matching `--interpret`
  ground truth. Guard: a `tests/` script over `{literal, scalar-ref, vector len/index, struct field}` ×
  `{live, dead vector}` × both backends.

### A2 — wasm source delivery + the parked interpreter in the module (F2)

Abstract the source load behind one loader with two backends: **filesystem** (native, today) and
**`host_input` channel** (wasm). A JS-side driver pushes the source bundle + `flip`/`edit` commands over
the channel; frames/eval-results return over `loft_host_print` (or a dedicated debug channel). The swap
model is unchanged because wasm is single-threaded. The parked interpreter is included only in the
opt-in build (A3).

- **Acceptance:** `routing`'s headless-Chromium parity harness (`tools/kernel_headless_test.sh`
  pattern) — in a `--html` live build, set a breakpoint, `eval` a vector expression and read the value;
  edit a fn and observe the live world pick up the new body. Values equal the `--interpret` run.

### A3 — opt-in packaging (F3)

A single codegen flag (working name `--live`) gates the whole tier: the `live_flipped` entry-check
emission, the bootstrap wiring, and the wasm interpreter inclusion. Lean release (flag off) emits **no**
entry checks and bundles **no** interpreter — smallest and fastest. Live/debug build (flag on) is the
tier. `LOFT_LIVE_FLIP` stays the *runtime* activation within a live-capable build.

- **Acceptance:** a lean `--native` build's emitted Rust contains **zero** `live_flipped` calls (grep
  the generated source); a `--live` build contains them and the tier works. Record the size/perf delta
  (esp. the wasm module size — the `--html` cdylib is ~1.1 MB today; the bundled interpreter is the
  opt-in cost to measure).

## Probes (falsify the load-bearing claims before building)

1. **"The F1 fix is one chokepoint, not per-value-kind."** Read where struct-locals succeed and
   vector-locals fail in `debug_eval`; if they diverge at ONE branch, the claim holds and the fix is
   local; if a vector needs a whole separate reflection path, the invariant is wider than assumed —
   record the real axis.
2. **"Sharing is a swap, not a copy, and survives wasm."** After A2, a compiled write to a var then an
   interpreted read of the *same* var (same store record) must agree — prove one heap, not two.
3. **"Opt-in is zero-cost-absent."** The lean build's emitted code has NO entry checks — grep proves it,
   don't assume it.

## Out of scope (declared)

- A new *language-level* debug API or syntax — the tier is a host/runtime capability.
- Changing the interpreter's evaluation *semantics* — it evaluates exactly as `--interpret` does.
- Multi-threaded live flip — single-threaded by construction; worker threads run compiled.

## Phase order

**P1 (A1) — fix the evaluator.** The load-bearing bug; unblocks the debugger on real code with no new
surface. **P2 (A3) — the opt-in flag.** Cheap, and it is the boundary every other piece plugs into.
**P3 (A2) — wasm delivery.** The largest piece; lands the offline/browser consumer. Each phase has its
own acceptance gate above; P1 ships value alone.
