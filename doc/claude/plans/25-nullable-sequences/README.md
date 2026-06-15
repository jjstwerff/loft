<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 25 — Nullable sequences (`vector<T>` participates in the null model)

> **Identity pending.** The `loft-lang/plans` issue (`@PLN25`) is NOT yet filed —
> creating it needs explicit authorization (external-repo write). `25` is provisional
> (tracker latest = 24); rename the dir if the issue lands on a different number.

## Status

Open — design validated, no implementation yet. Promoted from the design discussion
on loft#384 (negative-slice silent-empty / wrap-around garbage) and loft#387 (text
fn-ref buffer family). The invariant was validated against a boundary matrix and three
subsystem maps (struct-or-null blueprint, vector-consumer inventory, H6 sentinel
model); the representation decision is pinned. **This plan intersects the H6 stability
hotspot** — it is the vector-axis reconciliation of the scattered null-sentinel matrix,
not adjacent to it.

## Goal

Make `vector<T>` nullable like every other loft value: a vector can hold **null**
(absent), distinct from an **empty** `[]`, using the canonical reference sentinel
`store_nr = u16::MAX` — so a slice or lookup that has no answer returns a real `null`
instead of a silent-empty or corrupted vector.

## Effort + design

- **Effort:** H (multi-backend: interpreter + native + wasm; ~15 re-assertion sites).
- **Design:** ~ (invariant validated + phases defined; per-phase cell expectations TBD).
- **Last touched:** 2026-06-15

## The invariant (the one rule)

A vector is null **iff** its backing reference is the canonical null sentinel
`store_nr = u16::MAX` — the SAME sentinel struct references already use. Empty `[]`
stays a VALID store with length 0 (`rec==0` / a real record). Null ≠ empty is
guaranteed because `u16::MAX` is distinct from every real `store_nr`. Every site that
reads a vector's backing store routes through **one** chokepoint that reports "null"
only for `u16::MAX`, so the guard is asserted once, not sprayed.

**Why not the existing `rec==0`:** vector code today treats `rec==0` as *both*
unallocated and empty — so null and empty are currently the same state and cannot be
distinguished. Reusing it would make a null sub-sequence indistinguishable from an
empty one (the half-baked outcome). Reusing the reference sentinel instead nets H6
toward a single heap-null encoding.

## Composition matrix — Stage A (write as `/tmp` probes on `--interpret` FIRST)

The feature is done when **every cell is green on both backends**, not when a demo
runs. Axes: {null vector, empty `[]`, populated} × {operation} × {backend}.

| operation | null vector (`u16::MAX`) | empty `[]` | populated | notes |
|---|---|---|---|---|
| `v == null` / `!= null` | true / false | false / true | false / true | new operator overload |
| `len(v)` | `0` (decision Q2) | `0` | n | must not deref |
| `for x in v` | 0 iterations | 0 iterations | n | must not deref/OOB |
| `v[i]` (raising) | `null(oob)` raise | `null(oob)` raise | elem/raise | loud, recoverable |
| `v[a..b]` slice | null vector | clamp/empty (Q3) | sub-seq | OOB → null (the #384 payoff) |
| `v += x` | error or auto-init (Q4) | append | append | decide |
| `sort`/`reverse`/`remove`/`clear` | no-op (null-safe) | no-op | mutate | chokepoint guard |
| return `null` for `vector<T>` | compiles | — | — | unify like Reference |
| nullable field unset | null | — | — | `default_native_value` Vector arm |

Probes graduate to `tests/scripts/25-nullable-sequences.loft` as the regression suite.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **P1** — Foundation | ✅ Chokepoint `DbRef::is_null()` + `DbRef::NULL` (`store_nr==u16::MAX`) in `src/keys.rs`; vector store-accessors (`length`/`get`/`append`/`remove`/`insert`/`clear`; `sort`+`reverse` transitively via `length`) guarded through it. A null vector flows through `len`/`for`/`index`/`append`/`remove` with no `stores[u16::MAX]` OOB; null stays distinct from empty `[]`. Verified by `plan25_null_vector_tests` (4 tests) + full suite (2381 pass, 0 fail). | Shipped |
| **P2** — Surface | mirror the 5 struct-or-null mechanisms for `Vector`: `convert(Null⇄Vector)`, `==`/`!=` overload, return-unification (nullable unless `not null`), codegen ref_ops Vector arm. `v == null` and `fn f() -> vector<T> { null }` compile + behave. | Open |
| **P3** — Producers | `default_native_value` Vector arm (nullable field → `u16::MAX`; `not null` → empty); slice out-of-range → emit null vector. **Wires loft#384.** | Open |
| **P4** — Hardening | consumer audit (every `t_*vector*` native fn), both backends + wasm, full suite, regression tests, docs (LOFT.md null model + STABILITY_HOTSPOTS H6). | Open |

## Phase ordering

1. **P1 first** — the chokepoint is the load-bearing risk-reducer: it collapses ~9
   silent OOB-risk guards to one. Until a null vector survives every read op, the
   surface work has nothing safe to point at.
2. **P2** — once the runtime is null-safe, expose null at the type/operator layer.
3. **P3** — only after `null` is a first-class vector value do producers emit it.
4. **P4** — audit + multi-backend + docs last; each prior phase already ran the matrix.

## Open design questions

1. **Null encoding** — RESOLVED: `store_nr = u16::MAX` (reuse reference sentinel; nets
   H6 toward one encoding). Alternative (new vector-only code) rejected: adds a 6th
   sentinel, worsens H6.
2. **`len(null)`** — `0` (treat absent as zero-length, matches existing `rec==0`→0) or
   itself `null`? Leaning `0` for ergonomics; revisit against the `!v` truthiness rule.
3. **Slice out-of-range** — return a null vector (consistent with this plan) vs
   clamp-and-warn vs empty-and-warn. Leaning **null vector** now that null is a real
   value — that is the whole point of the feature.
4. **`v += x` on a null vector** — error (loud), or auto-initialize to `[x]`? Leaning
   error: appending to an absent vector is a logic bug, make it loud.
5. **`not null vector<T>`** — does the field/param modifier suppress nullability and
   unlock the empty-only fast path (skip the `u16::MAX` guard)? Mirror `Reference`.

## Cross-arc dependencies

- **H6 — null-sentinel matrix** (`STABILITY_HOTSPOTS.md`): this plan IS the vector-axis
  of H6. Coordinate so the `u16::MAX` unification here is the same one H6 adopts
  tree-wide; do not introduce a parallel encoding.
- **loft#384** (negative-slice) — closed by P3 (slice OOB → null + from-end bounds).
- **loft#387** (text fn-ref ABI) — sibling buffer/null family; not blocking, but the
  `convert`/return-unification work in P2 is adjacent.

## See also

- `doc/claude/LOFT.md` § Null representation (the "nullable unless `not null`" model).
- `doc/claude/STABILITY_HOTSPOTS.md` § H6 (null-sentinel matrix) — the shared hotspot.
- `doc/claude/DATABASE.md` (Store / DbRef / vector record layout).
- `src/vector.rs` (store-accessors), `src/parser/operators.rs` (`==` dispatch),
  `src/parser/control.rs` (`block_result`/`convert`), `src/generation/ops/ref_ops.rs`
  (null-aware codegen), `src/database/structures.rs` (`set_default_value`).
- loft#384, loft#387 (source design discussion); `@PLN25` (tracker — pending).
