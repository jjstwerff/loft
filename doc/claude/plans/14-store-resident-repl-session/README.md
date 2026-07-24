<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 14 — Store-resident REPL session

> **Identity:** `@PLN14` — [loft-lang/plans#14](https://github.com/loft-lang/plans/issues/14)
> (`status:future` — the issue is the source of truth for lifecycle state). Slug
> `store-resident-repl-session`.

## Status

Open — design ready, no implementation. **Detailed build design added 2026-07-24**
(see *Implementation design — small safe steps* below): the *in-memory* store
round-trip is de-risked by the debugger's proven `snapshot_heap` sibling (reused for
arc B), the residual is isolated to arc F (persist/resume + a schema-version gate),
and the build is sliced into flag-gated, differential-checked steps that cannot
regress the corpus. The model is established in
[CONVERGENCE.md](CONVERGENCE.md) (moved here from @PLN12 on its close); this plan
is the build. It is the successor to **@PLN12** (REPL + introspection, now
finished): @PLN12's heavy residuals — the re-run *cost* and exact value-for-value
resume — land here, and @PLN12's small open tail (an in-process
result-as-`String` eval API for embedding) is absorbed as sub-arc **H** below
(the store-resident value makes it nearly free).

The @PLN12 **REPL.X value-snapshot interim** has **shipped** as the *mitigation*: a
binding's RHS runs once and is rewritten to an own-format literal, so side-effect
repetition and error-poison are gone **for renderable types**. It does **not**
remove the re-run *cost* (a long session still replays a body of literals each
observe) and it **recomputes** non-deterministic values (`random()`, `now()`) on
resume. This plan is the bottom-up successor that removes replay entirely.

## Goal

Replace the REPL's replayed-source session with a **store-resident binding
environment** (`name → DbRef` / boxed-scalar in one persistent session store) so each
binding's RHS runs **exactly once**, observing reads the stored value, and resume
restores values **verbatim** — dissolving side-effect repetition, error-poison, and
re-run cost in one model.

**Design with the debugger in view ([@PLN16](../16-debugger/README.md)).** The same
environment is the *breakpoint frame*: the debugger (browser-natural, but also
terminal/embedded) drops into a REPL whose variables are a paused frame's locals. So
the env must be **seedable from an arbitrary live frame** (slot table + values), not
only from typed `name = …` bindings — "REPL session" and "breakpoint frame" are one
env model. Keep this in scope for sub-arcs A/D so the env isn't re-shaped later.

## Effort + design

- **Effort:** H — execution-core + store change, multi-phase.
- **Design:** ~ — [CONVERGENCE.md](CONVERGENCE.md) establishes the model and
  proves the store/mmap/cache infra exists; this plan sequences the build and
  names the open questions.
- **Last touched:** 2026-07-24 (detailed step design + `snapshot_heap` sibling reuse)

## The root the interim only masks

Replay-source is the *shared* root: all three warts are the same mechanism
(reconstruct-by-re-execution) seen from three angles. The interim masks two of them
for renderable types by making the replayed source *pure* — but a pure replay is
still a replay, so the cost and the non-determinism survive. Removing replay (store
the value, read the value) removes the whole class, not three cases of it. That is
the Goal-E move: one owned home for each bound value, every path consults it.

## Composition matrix — Stage A

The load-bearing claim is *"every value type survives `bind → session-store →
observe` and `bind → session-store → save → mmap-restore` exactly equal."* Falsify
it across the axes this change actually touches **before** wiring the environment in
(throwaway `/tmp` probes on `--interpret`, then graduate to `tests/scripts/`):

- **type-kind** — `integer` (incl. a **>32-bit** value), `float`, `single`,
  `boolean`, `character`, `text` (empty · embedded `"`/`\n`/`\` · unicode · >256 B),
  simple `enum`, struct-`enum`, `struct`, `vector`, **nested** struct,
  vector-of-struct, `null`.
- **persistence path** — `bind` · `observe` · **re-bind same name** (`n = n + 1` —
  the old record orphans) · **cross-binding ref** (`b = a` — value-copy, must not
  alias) · **save → restore** (resume).
- **backend** — `--interpret` and `--native` (cross-mode divergence is real).
- **negative / aliasing controls** — `b = a` then mutate `a` → confirm `b`
  unchanged (loft value semantics, store-copy not alias); a **faulting** binding
  records *nothing* (no environment entry, no poison) — the structural form of the
  interim's `Capture::Failed`.

Round-trip per cell: value → session store → observe → **equal**; and → save →
mmap-restore → observe → **equal**. The feature is done when every cell is green on
both backends, not when the demo runs.

## Sub-arcs

| Item | Status |
|---|---|
| **A** — session store + binding-environment record (`name → (Type, handle)`) | Open |
| **B** — value materialization: store-to-store copy of a run's result into the session store, returning a stable `DbRef` | Open |
| **C** — scalars at rest (`x = 5`): boxed into the store, or a tiny inline tagged env value | Open |
| **D** — frame-seed: prior names load from the session store into their slots before a new statement runs | Open |
| **E** — observe / `:vars` read from the environment — **no body replay** | Open |
| **F** — resume: mmap the session store + schema-version gating (stale image → fresh fallback) | Open |
| **G** — lifetime: orphaned records on re-bind (`:reset` wipe first; GC only if it bites) | Open |
| **H** — in-process result-as-`String` eval API (`eval(line) → rendered value`) for embedding/GUI — absorbed from @PLN12's REPL.T tail; nearly free once values are store-resident (the renderer already exists in `render_capture` / `show_loft`) | Open |

## Phase ordering

1. **A + B together** — the env record and the store-copy primitive are the spine;
   nothing can observe-from-store until a value can *live* in the session store.
2. **C (scalars)** — makes the model uniform: every binding's value lives in the
   store, so seeding (D) is one path, not two.
3. **D (frame-seed)** — a prior-name reference reads store → slot, then the existing
   slot-based expression codegen runs unchanged (less invasive than new store-load
   opcodes; see Q1).
4. **E (observe reads the env)** — removes the replay; this is where side-effect
   repetition **and** re-run cost die.
5. **F (resume)** — reuses the startup-cache mmap mechanism
   (`src/data_store.rs`/`src/cache.rs`); add the schema-version gate so a loft
   upgrade rejects a stale image and falls back to fresh (or to the portable
   text-replay resume already shipped).
6. **G (lifetime)** — accept growth + `:reset` first; GC deferred unless a real
   session hits it.

## Implementation design — small safe steps (2026-07-24)

Written after reading the debugger's store-revert (`@PLN63 RX`) as the **proven
sibling**. This is an exact-invariant domain (a store round-trip is a construction to
*recover*, not a space to explore), so the method is: name the one invariant, count
where it is re-asserted, use the sibling to say what is already proven and where the
residual is, then sequence the build so each step lands green and the one risky flip is
preceded by a differential check.

### The invariant (north star — constitutive, not confirmatory)

> **Every bound value has exactly one owned home in the session store, and every path
> that observes it consults that home — so `bind → store → observe` and
> `bind → save → mmap-restore → observe` both return it byte-for-byte equal.**

The Stage-A matrix cells are not evidence *that* this holds; they are the *only reason*
it holds — each value-type × persistence-path × backend cell is a case the one rule must
cover for the same reason. Build the matrix (Step 0) before wiring anything in.

### Re-assertion sites — the brittleness, counted now (N = 2)

The invariant is asserted in exactly two places; the design's whole job is to keep it
two and make each safe (drive `N × silence → 0`):

1. **Materialize (arc B) — the in-memory copy chokepoint.** Every bind routes through
   ONE primitive that copies a run's result into the session store; no bind may write
   the env by any other path. **Already proven:** `Stores::snapshot_heap` /
   `Store::snapshot_copy` (`src/database/mod.rs`) is a writable deep byte-copy of every
   store allocation, exercised on every value type each reverse step. Arc B reuses
   `Store::snapshot_copy` — the in-memory round-trip is not a research risk. One
   chokepoint → this site is safe by collapse.
2. **Persist / resume (arc F) — the on-disk round-trip.** The residual risk lives here,
   and the sibling *proves* it: `snapshot_heap` **refuses a file-backed store** (its doc:
   *"its on-disk state cannot be reversed"*) and **does not capture the compile-time
   schema** (`types`, `names`) — both are exactly what resume must cross (a durable
   store, possibly a different loft build). Make omission **loud, not silent**: a
   **schema-version gate** on the image so a stale/incompatible store *falls back to
   fresh (or to the shipped text-replay resume)* — never miscomputes. One loud guard →
   this site is safe by noise.

### The cleanest claim, attacked (over-unification guard)

*"Reuse the debugger's snapshot and fidelity is solved"* is **true for site 1, false for
site 2.** `snapshot_heap` is same-process, in-memory, whole-heap, schema-*invariant*;
resume is cross-process, on-disk, per-binding, schema-*variant*. Do not let the elegant
reuse absorb arc F — the persist round-trip is a genuinely different family, and it gets
its own matrix cells (save→restore) and its own guard (the schema gate). This is the one
place the design could read clean and break at the first case.

### What already exists — reuse, do not rebuild

| Capability | Where | Reuse |
|---|---|---|
| Writable deep byte-copy of every store allocation | `Stores::snapshot_heap` / `Store::snapshot_copy` (`src/database/mod.rs`) | **arc B** — the materialize primitive |
| Restore a heap snapshot in place | `Stores::restore_*` (`@PLN63 RX1`, `src/state/mod.rs`) | arc D/E — in-memory seed/restore |
| mmap + content-hashed startup-cache | `src/data_store.rs` / `src/cache.rs` | **arc F** — persist/resume image |
| Own-format value renderer | `show_loft` / `render_capture` (`src/repl.rs:915`,`:954`) | arc H (eval display); Q2's migration tool |
| REPL.X value-snapshot interim (RHS → own-format literal) | `capture_binding` (`src/repl.rs:902`) | the model @PLN14 supersedes — **the differential oracle for Steps 4–5** |
| Slot-based expression codegen | existing | arc D seed-frame (Q1 — no new store-load opcodes) |

### Small, safe, independently-landable steps

Each step fires only behind a flag or writes a shadow structure nothing reads yet, so
none can regress the ~corpus; the one risky flip (Step 5) is gated behind a differential
proof against the still-running replay. Ordered so each lands green on **both backends**.

- **Step 0 — the composition matrix (probe, no product code). Effort XS–S.**
  The Stage-A matrix as throwaway `/tmp` probes on `--interpret`, then graduated to
  `tests/scripts/`: for each value-type × persistence-path × backend cell, assert
  `bind → Store::snapshot_copy → read` equal **and** `save → restore` equal, with the
  aliasing/negative controls (`b = a` then mutate `a` → `b` unchanged; a faulting bind
  records nothing). *This is the generative act* — capture the sibling's in-memory
  round-trip as the proven half and let the matrix isolate where the **on-disk** cells
  diverge (schema, >32-bit int, embedded-quote / >256 B text, nested struct,
  enum-qualifier). **Risk: none** (no product code). Pins the invariant.

- **Step 1 — the materialize primitive (arc B), standalone. Effort S–M.**
  `materialize(result: DbRef, session: &mut Stores) -> DbRef` built on
  `Store::snapshot_copy`, returning a stable session `DbRef`. Rust unit + loft-script
  tests hit it *directly*, across the Step-0 matrix. **Safety:** dead code until wired —
  cannot regress anything. Enforces re-assertion site 1 at one chokepoint.

- **Step 2 — session store + env record (arc A), write-only shadow. Effort M.**
  Add a persistent session `Stores` + `env: name → (Type, DbRef)`. Each REPL bind ALSO
  materializes into the session store and records the env entry — but the **replay model
  stays the source of truth** (the env is written, never read yet). **Safety:** additive
  shadow, behaviour unchanged. **Verify:** every env entry equals the replayed value
  across the matrix (the shadow is the differential oracle warming up).

- **Step 3 — scalars at rest (arc C). Effort S.**
  Make scalar binds (`x = 5`) materialize uniformly — boxed 1-field record vs inline
  tagged env value (Q5) — so seeding (D) is one path, not two. **Safety:** still
  write-only env; behaviour unchanged.

- **Step 4 — frame-seed (arc D), flag-gated + differential. Effort M (the risk phase).**
  Before a new statement runs, load prior names from the session store into their slots
  (seed-frame, Q1 — reuses today's slot codegen, no new opcodes), so a prior-name
  reference *can* read from store. **Keep replay running in parallel** and assert
  `seed-frame value == replay value`. **Safety:** differential — the new path is
  validated against the still-correct replay; any divergence is a **loud** test failure,
  never a silent wrong value.

- **Step 5 — observe reads the env; replay OFF behind the flag (arc E). Effort M.**
  The flip: observe / `:vars` reads the env; body replay is removed (behind the flag).
  This is where side-effect repetition **and** re-run cost die, and where `random()` /
  `now()` stop recomputing on observe. **Safety:** behind the flag, and Step 4's
  differential already proved the seeded value equals the replayed one. **Verify:** a
  binding whose RHS prints runs the print **once**; a non-deterministic value is stable
  across repeated observes.

- **Step 6 — resume via mmap + schema gate (arc F). Effort M–MH.**
  Save the session store and mmap-restore on resume, reusing the startup-cache infra
  (`src/data_store.rs` / `src/cache.rs`); add the **schema-version gate** so a stale or
  cross-build image *falls back to fresh (or to the shipped text-replay resume)*. This
  is re-assertion site 2 and the residual-risk arc (the sibling explicitly does **not**
  cover it); Step 0's `save → restore` cells are its guard. **Safety:** the gate makes a
  bad image fall back, never miscompute.

- **Step 7 — lifetime (arc G). Effort S.**
  Orphaned records on re-bind (`n = n + 1`): `:reset` wipes; accept growth; GC only if a
  real session bites. **Safety:** growth is memory, not correctness.

- **Step 8 — flip default + absorb H. Effort S.**
  Once the flag-gated model is green across the matrix on both backends, make it the
  default and expose `eval(line) -> rendered String` (arc H, @PLN12's absorbed tail) —
  near-free because values are already store-resident and the renderer (`show_loft` /
  `render_capture`) exists.

### The build is the last probe

**Prediction:** with the copy primitive *reused* (not rebuilt) and the flip
*differential-gated*, the code lands **short** — arcs A/B/C/D are a data-model addition
(Q1 seed-frame adds no codegen) and arc E is a *removal* (delete replay). If a step
**stutters** — the scope grows mid-build, seed-frame wants a new store-load opcode after
all, or the matrix won't go green on `--native` — that friction routes to the search
**before** pushing harder; the likeliest cause is that the store-copy is not as
schema-independent as the sibling made it look (site 2 leaking into site 1), which is a
signal to re-root at arc F, not to patch arc B.

## Open design questions

1. **Prior-name resolution: seed-frame vs store-load codegen.** Seed-frame reads
   each referenced binding's value store→slot before the run, then reuses today's
   slot codegen untouched; store-load codegen compiles a name to a `DbRef`-deref
   opcode. **Lean seed-frame** — it touches no codegen and keeps one execution model.
   **Resolved (2026-07-24):** the sibling settles it — `@PLN63 RX` restores the heap
   **and** the slot registers in place with **no** codegen change (`restore_checkpoint`),
   so seed-frame (store→slot, then existing slot codegen) is a proven shape; Step 4 is
   built on it. Store-load codegen is off the table unless Step 4 stutters.
2. **Materialization: store-copy vs own-format round-trip.** Direct store-to-store
   `DbRef` copy is **exact and same-version** (no float-decimal / enum-qualifier
   edge cases); the own-format `show_loft` round-trip is the **migration /
   cross-schema** tool (and display). They coexist — store-copy for the session
   environment, own-format for live schema migration. State the separation so the
   serializer isn't mistaken for the session-persistence path.
   **Resolved (2026-07-24):** store-copy is `Store::snapshot_copy` (arc B, the sibling's
   proven primitive); own-format stays arc H / cross-schema only. The separation is now
   load-bearing: `snapshot_copy` is schema-*invariant* (same-process) so it cannot be the
   **resume** path across a loft build — that is arc F's mmap image + schema gate, a
   distinct mechanism (see the re-assertion-site count above).
3. **@P381 CONST_STORE re-lock under a persistent session store.** The value lives
   in the *session* store, not the const-store; confirm a seed-from-store run
   sidesteps the re-lock the way today's fresh-State model does.
4. **Cross-binding value semantics.** `b = a` must copy (loft value semantics) —
   probe that mutating `a` leaves `b` unchanged; the store-copy primitive (B) is
   where this is enforced.
5. **Scalar representation (C).** Boxed 1-field store record (uniform with D) vs an
   inline tagged value in the env record (cheaper, but a second path through D).

## Cross-arc dependencies

- **@PLN12** (REPL + introspection, **finished**) — this is its REPL.X
  *store-resident endpoint*. The value-snapshot interim shipped under @PLN12;
  @PLN14 supersedes it and absorbs @PLN12's open tail (sub-arc H).
  [CONVERGENCE.md](CONVERGENCE.md) (moved here from @PLN12) is the design source.
- **@PLN11** (`Data` as a store) — same direction (store-resident records over
  in-memory structures); the session store reuses the store / mmap / startup-cache
  infrastructure @PLN11 also builds on.
- **[@PLN16](../16-debugger/README.md)** (debugger) — the prime consumer: the
  breakpoint frame *is* this store-resident env (seeded from a paused frame).
  @PLN14's env shape is load-bearing for it; the frame-seedable requirement above
  comes from there.
- **loft2 store-resident IR** — the eventual home (bindings as store records is a
  facet of the representation rewrite); @PLN14 is the REPL-scoped down payment that
  exercises the model against a real consumer first.

## See also

- [CONVERGENCE.md](CONVERGENCE.md) — the model, *why not mmap the stack*, *why the
  stores DO mmap*, the four remaining needs (this plan builds items 1–3), and the
  RNG decision (moved here from @PLN12/03 on its close).
- [DESIGN_DECISIONS.md § C72](../../DESIGN_DECISIONS.md) — resume restores stored
  values but deliberately **not** RNG generator state.
- [GOALS.md § Why a language, not a store bolted onto an existing one](../../GOALS.md)
  — the own-format migration north star (Q2's cross-schema tool).
- `src/store.rs` · `src/data_store.rs` · `src/cache.rs` — the mmap + content-hashed
  startup-cache infra phase F reuses.
- **Tracker:** [loft-lang/plans#14](https://github.com/loft-lang/plans/issues/14)
  (`plan` · `subject:loft` · `status:future`).
