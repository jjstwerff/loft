<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 98 — Live/debug tier: one primitive (the interpreter over the shared live store)

**Status:** DESIGN, refined against a full code map (2026-07-07) · [`@PLN98`](https://github.com/loft-lang/plans/issues/98) ·
`subject:loft` · design-doc-first (Design Protocol 1). Consumers: the `@PLN16` debugger, the game /
`engine_host` loop, and `routing`'s offline `--html` build (its `loft-feedback.md` 2026-07-07).

Prior art this COMPLETES (not greenfield): `@PLN16` (debugger, `status:finished`) and `@PLN18`/`@I78`
(engine-host live tier — phase 08 `08-live-build-swap.md` shipped for native). The primitive is proven;
this plan closes the last eval increment, adds the browser tier, and makes the whole thing opt-in.

---

## The invariant (one sentence)

> **The live/interpreted path — whether RUNNING an edited function or EVALUATING a debug expression —
> operates on the ONE live `Stores` the compiled program uses, by direct reference: never a copy, never
> a re-serialisation of its state.**

Two facets of one principle (one live heap, no copies):
- **Execution** — a flipped/edited function re-enters the interpreter over the program's own `Stores`
  (a swap into the parked `State`, not an alias/copy). **Shipped** (`live_dispatch`).
- **Eval** — reading a value out of a paused frame reads the **live store record**, not a reconstructed
  text literal of it. **This is where the design is violated today** — see F1.

Every capability anyone asked for — breakpoint `eval`, `setValue`, hot-reload, a game's on-the-fly
script edit, a browser live-edit — is a use of that one principle. The plan is **make the principle hold
everywhere it currently doesn't**, not build three features.

## Why this is a design, not a feature list (the tell)

Three separate-looking asks — an agent debugger (`@PLN16`), the game changing scripts without a restart,
`routing`'s offline browser build — all reduce to the same sentence: "run/evaluate against the live heap
without stopping the program." One mechanism (`live_dispatch`) already realises it for native. The
design's job is to keep `N` (re-assertion sites) at ~1 and repair the one place the invariant leaks.

## The primitive that exists (grounding, with refs)

`live_dispatch` (`src/live_dispatch.rs`, `@PLN18`/`@I78`):

- A `--native` binary under `LOFT_LIVE_FLIP=1` (`live_dispatch.rs:101`) bootstraps a **parked
  interpreter** `State` from the same sources (`bootstrap`, `:112-205`; reads `LOFT_LIVE_SRC/_STDLIB/
  _LIBS` off disk, `:113-126`).
- **The program's `Stores` IS the bootstrap's world** — swapped into `state.database` around each
  dispatched call (`State::reenter`/`reenter_ret`), not aliased or copied.
- Every generated user fn opens with a one-atomic-load `live_flipped(idx)` check
  (`Emitter::live_entry_check`, `src/generation/mod.rs:1201-1241`, emitted at `:3252`); cold cost off is
  a single relaxed load (`live_dispatch.rs:73-85`).
- **Flip** routes a fn's calls into the interpreter (`live_call_void/_i64/_ref/…`) over those stores.
- **Tier-0 live reload** (`src/live_reload.rs`): `poll` runs inside the execute loop every ~32k ops
  (`state/mod.rs:3632-3651`, `RELOAD_POLL_OPS`), diffs `fn`-blocks, and `reload_fn` repoints
  `state.fn_positions[d_nr]` + the recorded call sites (`live_reload.rs:386-398`) — the flipped fn then
  runs the **new** body over the **live** state. Also an explicit `reload` control command.
- A **breakpoint** auto-flips the target fn (a compiled body cannot pause — `debug_apply_bp` →
  `set_flip_in`, `live_dispatch.rs:399-413`) and runs it under the pause loop (`debug_pause_loop`,
  `:444-508`).
- **Failure posture:** every bootstrap problem WARNS and falls back to compiled. Single-threaded by
  construction.

**Status honesty:** native execution/flip/reload is **shipped** (`@PLN18` 08, `tests/engine_host_reload.rs`);
the native debugger is **shipped** (`@PLN16`, incl. D2 "live-frame eval, first cut"). So the game's
"change scripts on-the-fly" **already works on native today** (flip + `LOFT_LIVE_RELOAD=1` or the
`reload` control command). The remaining work is three specific gaps.

## The three consumers, one mechanism

| consumer | a "live action" is… | status |
|---|---|---|
| **Debugger** (`@PLN16`, `--rpc`) | breakpoint = flip + pause; `eval` = evaluate against the paused frame; `setValue` = write a frame var | breakpoints/step/frame-read **shipped**; **`eval`/`setValue` broken on any frame with a heap (vector/keyed) local** — F1 |
| **Game on-the-fly scripts** | edit a `.loft` fn → `fn_position` repoints → next call runs the new body over the live world | **shipped (native)**; note the game control-channel `eval` is a live `hit.locals` lookup (`live_dispatch.rs:464-471`), NOT the reconstruct path, so it does NOT hit F1 |
| **Offline / browser** | the same parked interpreter compiled into the `--html` module, sources embedded, cooperative pause | **not wired** — `loft_start` (`generation/mod.rs:1710`) bypasses the tier entirely; hard blockers in F2 |

## Failure paths (the design's core — write how it breaks first)

### F1 — `eval` re-serialises the frame through a lossy text round-trip (the invariant violation)

`debug_eval` has two paths (`src/repl.rs:2277-2314`):
- **(A) live read** — a bare identifier that names a heap local is read straight from the paused store
  via `State::eval_frame_heap` (`state/mod.rs:2294-2327`). **Honours the invariant; works.**
- **(B) reconstruct** — ANY non-bare expression (`2 + 2`, `len(v)`, `v[0]`, `s.f`) falls through
  (`repl.rs:2298-2313`): it binds **every** captured local into a text `prefix` and compiles/runs
  `fn replmain() -> τ { <prefix> <rhs> }` on a throwaway `State` (`capture_typed`, `:2697-2730`).

The captured literals come from `render_frame_local` (`state/mod.rs:3187-3296`), whose heap arm
(`:3263-3282`) renders with **`show_loft_bounded(…, depth 2, 8 elems)`** — a **bounded** render that
emits non-reparseable truncation tokens: `,...` past 8 elems (`database/format.rs:1258`), `[...]` past
depth (`:1231`), `{...}` for a struct past depth (`:1080`); keyed collections hit the catch-all
`other => "<…>"` (`mod.rs:3294`). Any realistic heap local → a literal like `[1,…,8,...]` that **fails
to parse** → `Capture::Skip` → `value_of_fmt` returns `None` → `eval` replies `value:null`. Because the
poison sits in the **shared prefix**, it kills eval of **every** `rhs`, including `2 + 2`. `debug_set`
calls `debug_eval` first (`repl.rs:1923-1925`), so `setValue` fails the same way — even for a scalar.

**This is not a random bug: it is the one place the design's invariant is violated.** Path B copies the
frame OUT to text and back IN through a lossy channel, instead of reading the live store. The true
trigger is "any frame local whose captured literal doesn't round-trip through the parser" — not vectors
specifically. `@PLN16` README:280 already names it: *"a heap field path still routes through the
reconstruct path → null for a vector field… Extending the live read down a field/index path is the next
increment."* So F1 = **finish extending path A to seed heap locals for path B.**

*Minimal repro (routing, verified):* delete `v` and `eval "2 + 2"` → `4`; keep it → `null`.
```loft
fn main() { v: vector<integer> = [1, 2, 3]; x = 2 + 2; println("{x} {len(v)}"); }
```

### F2 — the browser bypasses the tier + four hard blockers

`--html` runs **fully compiled**: `loft_start` (`generation/mod.rs:1710`) does `Stores::new(); init; n_main`
— it never calls `boot_stores`, so there is no parked interpreter, no flip, no breakpoints. The
interpreter `State` itself *computes* on wasm (cfg stubs exist, `live_dispatch.rs:437-438`), but four
things are `--native`-only and must be replaced for the browser:
1. **Source delivery** — bootstrap reads `LOFT_LIVE_SRC` + `parse_dir` off disk; the browser has no fs.
   Must bootstrap from **bytes embedded in the wasm bundle**, delivered via the shipped `host_input`
   channel (loft#476). (`--html` host imports are print + GL + `host_input` only — `HTML_EXPORT.md:59-92`.)
2. **The pause loop blocks a thread** — `thread::sleep(2ms)` (`live_dispatch.rs:506`); a browser is a
   single-threaded event loop. Must become a **cooperative yield**.
3. **The control transport is loopback TCP** (`engine_host.rs:1908-1961`, `#[cfg(not(wasm32))]`). Browser
   needs a **`postMessage`/WebSocket** control channel.
4. **`std::env`/`std::fs`/child-process/rustc** (native rebuild+swap S4/S5) are unavailable — out of
   scope for the browser by design (`03-wasm-tier.md`).

None require wasm to *compute* something it can't; they require a different bootstrap + transport + a
non-blocking pause. The interpreter is the reusable piece.

### F3 — the entry check has no opt-out (packaging)

`live_entry_check` is emitted for **every** user fn unconditionally (`generation/mod.rs:3252`, gated only
by fn *shape*, never a build flag), and the `LOFT_LIVE_FNS` table + `boot_stores` call are always emitted
(`mod.rs:1807-1818`). `LOFT_LIVE_FLIP` is a **runtime** env, not compile-time. There is **no** cfg to
strip the guards/table for a lean release. Opt-in means adding one codegen gate around those emission
sites (and, for wasm, the interpreter's inclusion).

## Re-assertion sites — `N` is small (keep it there)

- **World-swap** funnels through `live_call_*` — one chokepoint, `N ≈ 1`. Good; unchanged.
- **The eval seam** is the single reconstruct path (`repl.rs:2298-2313`). F1 is that this one seam copies
  through text. The fix belongs at that one seam — route heap locals through the live read — **not** a
  per-value-kind literal-widening spray. If the fix wants to teach `render_frame_local` to emit
  unbounded round-trippable literals for *every* kind, that is the alarm: it re-serialises (still a copy)
  instead of reading the live store, and it will re-break on the next non-round-trippable kind. Prefer
  the live read; the unbounded-literal seed is the fallback only where a live read is genuinely
  unavailable.

## The design, per axis

### A1 — fix `eval`/`setValue` (F1): seed only referenced locals, heap ones live+unbounded ✅ LANDED (first cut)

**Done (2026-07-07)** in `debug_eval_fmt` (`src/repl.rs`). Two changes, one seam:
1. **Seed only the locals the expression NAMES** (`expr_idents`) — not every frame local. This was the
   real root: seeding *all* locals let one whose literal isn't loft source poison the whole parse. The
   actual poison was the vector's compiler backing `__vdb_N` (renders `main_vector<…>{…}`), so even
   `2 + 2` (no local) returned null. Now an unrelated heap/keyed local can never break an expression.
2. **Render a referenced heap local live + UNBOUNDED** via `eval_frame_heap` (path A's render) instead
   of the bounded display literal, so a >8-element user vector round-trips too. Scalars fall back to
   the captured literal.

- **Verified:** in a vector-local frame `eval "2+2"`→4, `len(v)`→3, `v[0]`→1, `v[1]+x`→25; a 10-element
  vector `v[9]`→100; struct `s.a+s.b`→16; `setValue x=42` accepted → `continue` prints the edited run.
  Regression guard `tests/rpc.rs::rpc_eval_and_set_in_a_vector_local_frame`; 38+10 debugger tests green;
  backend-independent (the REPL debugger always interprets).
- **Residual → P1b:** a *referenced* keyed collection (`hash`/`sorted`/`index`) still renders
  non-reparseable (the `<…>` catch-all) → its eval is a graceful `null` (no crash). Closing it needs
  the true live-frame eval (compile the expression in the paused function's scope + `reenter_dbg` over
  the frame), not a text seed — the invariant-honoring form.

### A2 — the browser live/debug tier (F2)

Under the opt-in flag (A3), `--html` bootstraps the parked interpreter from **embedded source bytes**
(a build-time blob, not `LOFT_LIVE_SRC`); a **cooperative-yield** pause replaces `thread::sleep`; a
**`postMessage`/WS** control channel replaces loopback TCP; the JS driver pushes `flip`/`edit`/`eval`
frames and reads results over the debug channel. `loft_start` gains a "live" variant that opts into the
tier. Native rebuild/swap stays out. `03-wasm-tier` (per-fn wasm module promotion) is **parked/superseded**
by 08 as the promotion tier — this is the *interpreter* tier, not module relink.

- **Acceptance:** `routing`'s headless-Chromium parity harness — in a `--html` live build, breakpoint +
  `eval` a vector expr → the value; edit a fn → the live world picks up the new body; a compiled write
  then an interpreted read of the same var agree (proof of one heap, not two — probe 2).

### A3 — opt-in packaging (F3)

One codegen flag (working name `--live`) gates: the `live_entry_check` emission (`mod.rs:3252`), the
`LOFT_LIVE_FNS`/`boot_stores` emission (`mod.rs:1807-1818`), and (wasm) the interpreter's inclusion.
Lean release (off): zero entry checks, no bundled interpreter. `LOFT_LIVE_FLIP` stays the runtime
activation within a live-capable build.

- **Acceptance:** a lean `--native`/`--html` build's emitted Rust has **zero** `live_flipped` calls
  (grep proves it — probe 3); a `--live` build has them and the tier works. Record the size/perf delta
  (esp. the wasm module — the `--html` cdylib is ~1.1 MB today; the bundled interpreter is the opt-in
  cost to measure).

## Probes (falsify before building)

1. **"F1 is the eval seam copying through text, fixable at one place."** ✅ largely confirmed by the map
   (path B vs path A; `@PLN16` README:280). Remaining probe: read `eval_frame_heap` and confirm a
   field/index live-read composes without a text round-trip.
2. **"Sharing is one heap on wasm too."** After A2: compiled write → interpreted read of the same var →
   agree. If they diverge, the swap didn't carry the world.
3. **"Opt-in is zero-cost-absent."** Grep the lean build's emitted Rust for `live_flipped` → must be 0.

## Out of scope (declared)

- A language-level debug API/syntax (host/runtime capability only).
- Changing interpreter eval *semantics* (it evaluates as `--interpret` does).
- Multi-threaded live flip; native rebuild/swap in the browser; per-fn wasmtime tier (`03`, parked).

## Phase order

**P1 (A1) — fix the evaluator.** ✅ LANDED (first cut) — `eval`/`setValue` work in heap-local frames;
the invariant violation for round-trippable values is closed; residual P1b (referenced keyed
collections → the true live-frame eval). **P2 (A3) — the opt-in flag.** Cheap; the boundary every other
piece plugs into. **P3 (A2) — the browser tier.** The largest piece (four blockers); lands the
offline/browser consumer. Each phase has its own acceptance gate above.
