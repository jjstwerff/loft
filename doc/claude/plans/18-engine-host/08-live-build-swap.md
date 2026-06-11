# Phase 08 — the live build swap: one loop for every host (design + test catalog)

**The vision (user-stated, 2026-06-10).** A totally compiled loft project
where, due to the debugger, parts can be pushed to the interpreter (in the
Rust layer), seamlessly debugged and even CHANGED (body-only edits — no
struct/enum layout changes for now), after which everything still works
with some functions interpreted — and then the server builds a new client
in the background and **swaps to the new build without stopping anything**.
The same loop holds for wasm: **the browser loads the new binary via
websockets and swaps to it.**

**This is the heart of the engine** (user, 2026-06-10: *"if we can do this
on both the native and the wasm build we are golden"*).  Every other piece
of the stack — the store, the dispatch table, the wire classes, the
debugger, the host layers — is judged by whether it serves this loop.

**The invariant.** *The build is replaceable; the state (the store) and the
connections (the host layer) persist.*  Everything below is that one
sentence, applied per host:

| Host | The persistent host layer | The replaceable build | State carrier |
|---|---|---|---|
| native | the OS + the next process (sockets via handover or seat reconnect) | the whole compiled binary | store snapshot at a frame boundary (the durable format) |
| browser | **the page** — `loft-rt`/`loft-gl` keep the WebSocket + canvas open across instances | the whole wasm module, fetched over the WS bulk channel | store export → import across instances (same format) |

The browser is the EASY case: its host layer already exists and owns the
sockets, so "swap" is fetch–snapshot–instantiate–resume with zero
connection churn.  Native must build the equivalent (cutover choreography).

This phase **supersedes 03's per-fn wasm modules as the promotion tier**:
for native hosts the background artifact is a plain full build (no wasmtime
dependency, no bridge tax, no per-fn bookkeeping), and for browsers the
whole-module swap inherits wasm's drop-an-instance unload safety anyway.
03's measurements stand (wasm exec ≈ 1.0× native); its per-fn residual is
retired.  Tier 0 (02's dispatch) is unchanged and remains the substrate.

**v1 bounds (deliberate):** body-only edits — the signature guard already
enforces this for reload; the build swap adds the layout guard (identical
type schemas between builds).  Lenient serialization later lifts the layout
bound into live data migration — that is its named purpose.

**The "small" after-part (user, 2026-06-10, ironically):** converting one
store format to another on class changes.  Schema-as-data (`Stores.types`)
is what makes it tractable: both builds' schemas are values, so the diff is
mechanical — added fields take defaults, dropped fields drop, widenings are
free; only RENAMES are ambiguous and need a hint, which is exactly the
frozen rename-migration-setter language feature (its thaw trigger is this
phase's v2).  Until then: layout-identical swaps, and a swap-time schema
hash comparison that refuses (gracefully — keep serving the old build)
rather than corrupts.

---

## The pipeline, as scenarios with tests

Each scenario names its observable and its test.  The differential rule
(Goal D, extended): at EVERY stage the meaning output must be identical
across interp / mixed / compiled — only timing may differ.

### S1 — the compiled baseline serves the game  ✅ (2026-06-11)

A kernel program (`run`/`run_client`) built `--native`, driven by the
existing e2e clients.  **SHIPPED**: the kernel natives gained TYPED TWINS
(`engine_host::typed`, re-exported through `codegen_runtime`, registered in
`CODEGEN_RUNTIME_FNS`) sharing the kernel internals with the bytecode-stack
natives — one implementation, two calling conventions; the bodies that were
inline in the stack natives (listen/pump×2/tick×2/sync-next×2/client-send)
were factored into shared `_impl`/`_kernel`/`_client` helpers so the queue
machinery cannot fork.  `engine_host_kernel::s1_native_baseline_matches_
interpreted` runs the SAME fixture both ways — transcripts byte-equal.
Process-model finding: a `--native` run spawns the compiled binary as a
GRANDCHILD of the loft driver — test guards must kill the process group
(probe-caught: a rerun connected to the previous run's orphan).

> **Test (extend `engine_host_kernel`/`_audience`):** the same scenario
> transcripts as the interpreted run — the audience-differential pattern
> with the SERVER swapped for its native build.  Pass = byte-equal
> transcripts.

### S2 — the debugger pushes one fn to the interpreter, live

Compiled callers call through the per-fn dispatch point; the target flips
native → interp at a frame boundary; nothing else changes.

> **Gate probes first (02 slices 1–2):** compiled→interp re-entry over the
> shared store — correctness matrix (scalar/struct/vector args, text
> returns, null) + crossing cost; the inlining check (a flip must not be
> defeated by an inlined call site).
> **Test:** a native kernel server with a control input that flips
> `broadcast_tick`; clients observe identical behavior before/after (the
> flip visible ONLY via `LOFT_DISPATCH_DEBUG` and the stamp chain's
> bounded interp cost, ≤ the measured 6×).

### S3 — edit the interpreted fn; the mixed build keeps running

Tier-0 reload against the compiled baseline: the shadow session parses the
edit, the dispatch target moves to the new bytecode, world state persists.

> **Test (extend `engine_host_reload` to the native baseline):** edit →
> new behavior next frame; the world counter continues (no restart);
> broken edit → old body keeps serving; signature change → rejected.

### S4 — the background rebuild

The serve host compiles the full project (rustc, minutes) keyed to the
source hash, while the old build keeps serving.

> **Test:** request a rebuild mid-run; assert (a) the artifact lands and
> its embedded source-hash matches, (b) the stamp chain shows NO tick
> degradation while rustc runs (build isolation), (c) an edit during the
> build invalidates the artifact (hash mismatch → rebuild requeued).

### S5 — the native swap: a new process under a running world

Cutover at a frame boundary: freeze meaning at tick N, snapshot the store,
boot the new build, hand over (or let seats reconnect), resume at N+1.
Rollback if the new build fails to boot.

> **Tests:**
> 1. **State continuity** — a monotonically counting world crosses the
>    swap with no lost/duplicated tick (the counter and the tick stamp
>    are exactly N+1 after the cutover).
> 2. **Event integrity** — every event-class message sent before the swap
>    is delivered exactly once (the queue drains or migrates; the class
>    contract holds across builds).  Sync class may conflate across the
>    gap — by design, assert only freshness afterwards.
> 3. **Seat continuity** — a native seat (`run_client`) and a WS client
>    ride through with a bounded gap; the connector gains reconnect
>    semantics (re-hello, new cookie) and the test asserts the gap <
>    the keepalive timeout (no path expiry cascade).
> 4. **Rollback** — a deliberately-broken new build (panics on boot):
>    the old process keeps serving; the stamp chain never stops; the
>    failure is reported, not fatal.
> 5. **Dispatch reset** — the interpreted fns from S3 run compiled again
>    after the swap (the sentinel shows zero interp dispatches).

### S6 — the browser swap: a new module under a living page

The server pushes the new wasm bundle over the WS bulk channel (05c's
first real consumer); the page snapshots the store out of instance A,
instantiates B, imports the store, resumes at the next animation frame —
the SAME WebSocket object, no reconnect.

> **Test (headless-chromium, the kernel-differential harness lineage):**
> client page runs build A with a visible behavior marker + a counting
> world; the server pushes build B (marker changed); assert (a) the
> page's WebSocket object identity is unchanged (no reconnect event),
> (b) the counter continues exactly, (c) B's marker appears within a
> bounded number of frames, (d) a corrupt push (bad hash) is rejected
> and A keeps running.
> **Gate first:** the known `Instant::now` panic in the compile_and_run +
> frame-yield combo must be fixed (it blocks ANY long-running browser
> client test — already `#[ignore]`-documented in the 07 differential).

### S7 — the debugger loop end-to-end (@PLN16 6b/6c convergence)

Breakpoint in a compiled fn → flip + pause (the debug suspension loop
keeps mechanics alive: a mechanics-only mini-pump answers keepalives) →
frame bindings into the REPL over the control channel → edit → resume
mixed → background rebuild → swap → still debuggable.

> **Test:** a scripted control-channel session against a child kernel
> process asserting each stage's observable in order: hit reported with
> bindings, REPL evaluates a frame variable, edit acknowledged
> (structured reload feedback), resume, rebuild notice, swap notice,
> a post-swap breakpoint hits in the NEW build.
> **Sub-tests:** breakpoint re-resolution by (fn, line) across both a
> reload (S3) and a swap (S5) — offsets move, identity must not.

### S8 — the standing differential (cross-cutting)

> **Test:** one meaning scenario executed in four states — interpreted,
> compiled, mixed (post-S3), post-swap — transcripts byte-equal.  This is
> Goal D's sweep extended to the mixed states; it pins "a target change
> is observable only as speed" permanently.

---

## Load-bearing claims to falsify before building (the probe gates)

1. **Re-entry is a dispatch, not a rewrite** (S2) — the 02 slice-1 matrix.
2. **Generated call sites can route through the table** without measurable
   cost when un-flipped (S2) — emit-level probe + the inlining check.
3. **The durable store format round-trips a LIVE world** (S5/S6) — churn a
   world while snapshotting at a boundary; byte-compare after restore.
   (store_durable tests are the seed; the new axis is "mid-run".)
4. **Build-while-serving does not disturb the tick** (S4) — rustc at full
   load vs the stamp chain.
5. **Socket handover choice** (S5) — probe BOTH: fd-passing (zero-gap,
   unix-only, complex) vs seat-reconnect (bounded-gap, portable, needs
   connector reconnect semantics).  Measure the gap; pick per evidence.
6. **Store export/import across wasm instances** (S6) — size + time at a
   realistic world; the page freeze budget is one frame at 30 Hz.

## Sequencing

02's re-entry probe remains the first build step — every scenario stands
on the dispatch table.  Then: S1 (cheap, mostly existing tests) → S2/S3
(the debugger substrate, converges with @PLN16 6b/6c and the control
channel) → S4 → S5 (native swap) → S6 (browser swap; gated on the wasm
time fix) → S7 wired through → S8 locked into CI.  05c's bulk channel can
land just-in-time before S6 — the build artifact is its first consumer.
