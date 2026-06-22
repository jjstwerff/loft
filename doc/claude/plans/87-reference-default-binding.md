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
- **P0** — size the migration (sweep for non-`&` reassignment-propagation reliance; **gates P2**).
- **P1** — `&` on local bindings (additive, non-breaking).
- **P2** — reassignment-locality (**breaking**: non-`&` reassignment → local rebind; `&` writes back).
- **P3** — W4 redundant-`&` lint.

## Concerns (detail in the issue)
P2 breaking-change risk (P0 first) · the `&vector` realloc edge (LOFT.md:1529 may be stale) ·
both-backends parity · borrow-checker integration (read the carried `deps` fact, don't
re-derive) · scalar-vs-heap `&` distinction · store-lifetime safety (the @PLN85 fixes stay
green) · out of scope: partial-move / copy-on-write · dogfood against a real heavy-mutation
consumer.
