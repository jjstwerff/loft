<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# @PLN87 — Reference-default `&`-binding ownership semantics

**Tracking issue: [loft-lang/plans#87](https://github.com/loft-lang/plans/issues/87)** — the
detailed plan (phases, gates, the matrix) and the eight load-bearing concerns live there.
Implemented by the loft2 agent.

Implement loft's binding-ownership model: **heap is reference-by-default** (a binding or
parameter aliases the source; field/element mutation writes through), and **`&` makes a
whole-binding reassignment write back** to the source. One uniform meaning for `&`; fixes the
recurring "`&Object` needed to mutate" confusion; realizes the OWNERSHIP_MODEL beacon for
binding semantics.

Design + rationale: [OWNERSHIP_MODEL.md § The law](../OWNERSHIP_MODEL.md) +
[DESIGN_DECISIONS.md C77](../DESIGN_DECISIONS.md). Builds on @PLN85 (the store-lifetime
cluster); the W4 lint joins the @PLN46 warning family.

## Phases
- **P0 — ✅ DONE (2026-06-22): the migration is EMPTY → P2 is SAFE.** Swept the whole
  ecosystem for heap-typed PARAMETERS reassigned wholesale (an IR `Set` on an argument
  slot, which propagates to the caller today) via a gated `scopes::check` instrument
  (`LOFT_SWEEP_P0=1 loft … / --tests …`). Result: **0** in the stdlib (`default/`),
  **0** across all 10 registry libs (cbor, crypto, web, server, time, random, regex,
  arguments, game_protocol, input), and **0** across the entire `zero-trust-shared-files`
  application (20+ packages incl. `server/core`). The only raw hits anywhere are
  crawler's `cube`/`plane`/`sphere` (3) — NRVO return-buffers (`fn cube() -> Mesh`
  builds + RETURNS a Mesh), safe by construction: the caller receives the value via the
  RETURN, not param propagation, so P2's local-rebind change cannot break them. **No code
  relies on non-`&` reassignment propagating — the load-bearing P2 risk is retired.**
  (The sweep instrument is throwaway; remove before the PR.)
- **P1** — `&` on local bindings (additive, non-breaking).
- **P2** — reassignment-locality (**breaking**: non-`&` reassignment → local rebind; `&` writes back).
- **P3** — W4 redundant-`&` lint.

## Concerns (detail in the issue)
P2 breaking-change risk (P0 first) · the `&vector` realloc edge (LOFT.md:1529 may be stale) ·
both-backends parity · borrow-checker integration (read the carried `deps` fact, don't
re-derive) · scalar-vs-heap `&` distinction · store-lifetime safety (the @PLN85 fixes stay
green) · out of scope: partial-move / copy-on-write · dogfood against a real heavy-mutation
consumer.
