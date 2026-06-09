<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 14 — Store-resident REPL session

> **Identity:** `@PLN14` — [loft-lang/plans#14](https://github.com/loft-lang/plans/issues/14)
> (`status:future` — the issue is the source of truth for lifecycle state). Slug
> `store-resident-repl-session`.

## Status

Open — design ready, no implementation. The model is established in
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

## Effort + design

- **Effort:** H — execution-core + store change, multi-phase.
- **Design:** ~ — [CONVERGENCE.md](CONVERGENCE.md) establishes the model and
  proves the store/mmap/cache infra exists; this plan sequences the build and
  names the open questions.
- **Last touched:** 2026-06-08

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

## Open design questions

1. **Prior-name resolution: seed-frame vs store-load codegen.** Seed-frame reads
   each referenced binding's value store→slot before the run, then reuses today's
   slot codegen untouched; store-load codegen compiles a name to a `DbRef`-deref
   opcode. **Lean seed-frame** — it touches no codegen and keeps one execution model.
2. **Materialization: store-copy vs own-format round-trip.** Direct store-to-store
   `DbRef` copy is **exact and same-version** (no float-decimal / enum-qualifier
   edge cases); the own-format `show_loft` round-trip is the **migration /
   cross-schema** tool (and display). They coexist — store-copy for the session
   environment, own-format for live schema migration. State the separation so the
   serializer isn't mistaken for the session-persistence path.
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
