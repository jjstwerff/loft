<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Ownership analysis — validation + the exact gaps (what the compiler must do)

The Stage-1 `Owned|Borrowed|Join` classifier (`src/use_analysis.rs`) is the INPUT the
Stage-3 compiler fix reads. Before wiring any free site we must know its gaps — *"we
can only detect what we need to do in the compiler once we know the exact gaps in the
analysis"* (user). This is that validation.

**Instrument:** [`fuzz/classify_vs_runtime.py`](fuzz/classify_vs_runtime.py) correlates,
per probe cell, the **classification** (the `OWN fn=…` dump) against the **runtime
over-free outcome on BOTH backends** (CRASH/LEAK, with compile-errors + asserts
separated out — they are not over-free signals). Run over the generated 54-cell matrix
(`grammar_gen.py`) + the 28 `probes/over-free-sweep/` probes + the 5 `462-*` repros.

## The headline: the analysis is SOUND (zero misses)

> **Across all 87 probe cells, every value that actually over-frees (CRASH or LEAK) is
> classified `Join` or `Borrowed` — NEVER pure `Owned`.** The analysis never tells the
> compiler "free this" about a live borrow. `SOUND MISSES: NONE`.

This is the load-bearing property: the classifier is a SOUND foundation. The gaps below
are about **completeness** (surfacing the free SITES) and **precision** (carrying the
base, scoping), not soundness.

## The gap map (none-churn matrix; classification × runtime)

| shape (struct) | class at escape | interp | native | reading |
|---|---|---|---|---|
| **elem_accumulate** | `pick:ret=Join` | **CRASH** (UAF) | clean | flag-OK; **interp-only** over-free |
| **match_return** | `deliver:ret=Join` | **CRASH** (UAF) | clean | flag-OK; **interp-only** over-free |
| **local_source** | `one:reassign(chosen) prior=Owned rhs=Join` | **LEAK** | **LEAK** | flag-OK; **both backends** |
| field_return / field_local / field_reassign / nested_field | `…:ret=Borrowed` | clean | clean | safe borrow — "don't free" is correct |
| index_read | `deliver:ret=Join` | clean | COMPILE-ERR | Join accurate; native bug (below) |
| if_return | — | COMPILE-ERR | COMPILE-ERR | doesn't type-check (below) |
| *all scalar* | `ret=Owned` / `ret=Join` | clean | clean | scalar never over-frees |

Key refinement: **`elem_accumulate` and `match_return` over-free on INTERP only — native
is already correct** (right values, no leak). Only `local_source` leaks on both. So native
is a correctness REFERENCE for two of the three shapes — mirror what it does, don't invent.

## The exact gaps — and the compiler action each unblocks

### Gap A — the analysis classifies VALUES; two of three fixes act at a FREE SITE it doesn't surface
The over-free decision lives at a free site, and the analysis currently exposes only two of
the three site kinds:

| shape | the over-free SITE | analysis surfaces it? | compiler action |
|---|---|---|---|
| `local_source` | the owned-slot **reassign** | ✅ `reassign_sites` gives `prior=Owned rhs=Join` | **READY** — wire `scopes.rs` free-placement (both backends leak) |
| `elem_accumulate` | the **append element source-free** (`OpCopyRecord` `0x8000` on `pick`'s Join return, in `collect`) | ❌ only `pick:ret=Join`, not "collect's append source-frees a Join" | needs **append-source-free site classification** → then fix interp's source-free |
| `match_return` | the **arm-return delivery** (`materialize_vector_arms_into` reassigns the buffer to a borrowed enum field) | ❌ only `deliver:ret=Join`, not the delivery site | needs **arm-return delivery site classification** → then fix interp's delivery |

So: **`local_source` can be wired NOW; `elem_accumulate` + `match_return` need the analysis
extended to surface their free sites first** (a Stage-1.5 increment — still inert, still
testable, before any emit change).

### Gap B — `Own::Borrowed`/`Join` carry no BASE
The Join fix is "materialise the borrow arm to owned at the escape" — which needs to know
WHICH store to copy. `Own::Borrowed` is currently opaque (Stage 1 dropped base tracking).
**The fact must carry the borrow base** (the projection's arg-0 / source var) before the
materialise can be emitted.

### Gap C — `Join` is over-flagged on clean cells (precision) — but closed by acting at the SITE
`match_return scalar` and `index_read` classify `Join` yet run clean. A value-level
"materialise every Join" would do needless work and risk regressing them. **Resolved by
Gap A's framing:** act at the FREE SITE, which only fires for record-element stores — so
scalar joins are never touched, no narrowing predicate needed. (Confirms the fix is
site-driven, not value-driven.)

## Adjacent bugs the sweep surfaced (NOT the over-free class; branch-internal)

Both around an empty `[]` literal used as a `vector<T>` default in a branch/coalesce tail
— a single likely root (empty-vector tail not typed/coerced to `vector<T>`):

1. **`if cond { v.rows } else { [] }`** → `error: expected vector<E>, got void on else`
   (the whole `if_return` shape fails to type-check). POISON-independent.
2. **`vv[i] ?? []`** → interp clean, **native `error[E0308]: mismatched types`** (the
   `index_read`/#426B shape; an interp↔native codegen divergence).

These block two probe shapes from even reaching the over-free analysis. Documented here
(branch-internal, stacked on @PLN25); not the join-ownership work — fold or file separately.

## Next step (revised by this validation)

1. **`local_source` first** — the analysis is COMPLETE for it (`reassign_sites prior=Owned`)
   and it leaks deterministically on BOTH backends → cleanest both-backend gate. Wire
   `scopes.rs` free-placement behind `LOFT_JOIN_OWN`.
2. **Extend the analysis (Stage 1.5, inert)** to surface the append-source-free site +
   the arm-return delivery site (Gap A) and carry the borrow base (Gap B); test via the
   dump as in Stage 2.
3. **Then** wire `elem_accumulate` (interp source-free) and `match_return` (interp
   delivery), each against its `462-*` repro + the matrix + POISON, mirroring the
   already-correct native behaviour.
