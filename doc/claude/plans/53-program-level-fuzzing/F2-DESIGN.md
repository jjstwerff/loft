<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN53 F2 — keyed-container axis — design gate (F2.0)

**Step F2.0 of the F2 decomposition** ([`STEPS.md`](STEPS.md)). No production
code. This pins the invariant a generated `hash`/`sorted`/`index` program
self-checks, read off hand-written exemplars, via the design-protocol skill's
*constructive instrument* (the schema-coupled grammar is not formable from the
desk — plot a concrete valid program and read the invariant off it).

## What the F2 keyed-container axis is

`program_ownership` (the shipped F2 partial) generates valid-by-construction
programs over the **ownership** grammar. The open axis generates programs that
exercise loft's **keyed collections** — `hash`, `sorted`, `index` — which the
ownership grammar leaves untouched. These are schema-coupled (`Key{type_nr,
position}` indexes the type registry; RB-node layout interleaves user fields,
the comparison key, and links), so the right way to exercise them is through
real loft programs where the schema is correct by construction.

Reified like F1: a library `generate_keyed(spec) -> String` + an in-process
runner, so `cargo test` drives generation over the spec space on stable; the
`fuzz_target!` is a shim.

## The one invariant

> **A generated program builds a keyed collection from a set of DISTINCT keys
> via a known sequence of inserts and key-removes, and self-checks that the
> collection is exactly the abstract KEY→VALUE map that sequence defines:**
> - **population** (`.len()` for `hash`/`sorted`; iteration count for `index`)
>   equals the surviving-key count;
> - every **surviving key** looks up its last-inserted value; every removed or
>   absent key looks up **`null`**;
> - **`for`** visits exactly the surviving keys, in the collection's declared
>   key order.

The generator KNOWS the operation sequence at emit time, so it bakes the
concrete expected values into the assertions (population count, per-key values,
the ordered key string). A codegen or store-lifetime bug that corrupts the
collection breaks one of those assertions — that panic is the finding, exactly
as `program_ownership`'s length-K self-check works, but for a keyed map.

The untested-case property: any generated (key set, op sequence) never tried
before is still self-checking, because the expected map is *computed from the
same op list that is emitted* (one model — see re-assertion sites).

## The three keyed types — facts read off the exemplars (both backends)

Verified by running `f2-exemplars/ex_{hash,sorted,index}.loft` on `--interpret`
**and** `--native` (all pass):

| Type | Declaration | Key | Duplicate key | `.len()` | Remove | `for` order | In-loop |
|---|---|---|---|---|---|---|---|
| `hash<T[k]>` | struct field or local | one+ fields | **REPLACE** (set) | **yes** | `c[k]=null` | ascending (snapshot) | `#remove` **rejected** |
| `sorted<T[k]>` | " | one field, `k` asc / `-k` desc | **KEEP** (multiset) | **yes** | `c[k]=null` | declared order | `#first`/`#count`/`#remove` |
| `index<T[k1,k2]>` | " | multi-part | KEEP (multiset) | **NO** (use iter count) | `c[k…]=null` (F2.1 confirm) | multi-key order | `#first`/`#count`; range `c[a..b, k]` |

Shared syntax: `c[key]` → element or `null`; `c[key] = null` removes (no-op if
absent); `+=` inserts; `for e in c` iterates in key order.

## Failure paths (the generative enumeration)

Every way the generator gives a **wrong verdict**:

1. **Expected-map model drifts from emitted ops** — the generator holds the
   expected map separately from the op sequence it emits; they diverge → a
   valid program carries a wrong assertion → **false finding**. The central
   risk (the F2 analog of F1's swallowed-panic).
2. **Accidental duplicate keys** — a repeated key exposes the `hash`-REPLACE vs
   `sorted`/`index`-KEEP divergence, so a uniform population count is wrong →
   false finding. Cured by the distinct-key constraint.
3. **Real store/codegen corruption** — a wrong lookup value, wrong population,
   or wrong iteration order → an assertion fails → **true finding** (the goal;
   this is what F4 poison amplifies for the store-UAF sub-class).
4. **Type-specific op misuse** — e.g. emitting `#remove` inside a `hash`
   iteration (a compile error) → a **generator** bug, not a language finding.

## Re-assertion sites — the brittleness, counted now (Protocol step 2)

The invariant "the baked assertions match the collection the ops build" must
hold across every operation the generator emits. If the generator maintains the
expected map in a **separate** pass from emitting the ops, that is **N sites**
(one per op kind: insert, remove) where the two silently diverge — failure path
1.

**Collapse to N = 1:** the generator emits from a single ordered **op list**,
and folds the *same* list into the expected map — one model produces both the
program text and the expectation. There is no second place to keep in sync, so
there is no drift to forget. (F1's "one function, two drivers" applied to a
generator: one op list, two projections — source and expectation.)

## Load-bearing claims + probes (Protocol steps 3–4)

- **Claim — "distinct keys give one uniform population/lookup/iteration
  invariant across all three types."** Probe (run): the three exemplars, each
  distinct-key with inserts + (hash/sorted) a key-remove, asserting population +
  per-key lookup + ordered-key string. Result: **all pass on both backends.**
  The invariant holds on concrete instances.
- **Claim, attacked hardest (over-unification guard) — "one grammar, identical
  across `hash`/`sorted`/`index`."** **FALSE**, and the probes show exactly how:
  `hash` REPLACES a duplicate key (set) while `sorted`/`index` KEEP it
  (multiset); `index` has multi-part keys, range queries, and **no `.len()`**;
  `hash` **rejects** `#remove` inside iteration. So the grammar is
  **parameterized per type** — only the *distinct-key map* invariant is shared.
  Compressing the three under one identical grammar would break the moment a
  generated program asserted a type's real difference.

## The over-unification residual — what stays a per-type axis (not the floor)

The shared floor is the distinct-key map. These real differences are **deferred
axes with per-type expected outcomes**, added deliberately after the floor, not
folded into it:

- **Duplicate-key policy** — `hash` replace vs `sorted`/`index` keep. A future
  axis asserts the per-type outcome (a hash of N dup inserts has population 1; a
  sorted has N).
- **`index` multi-prefix + range** — `c[a..b, k]` visiting a key slice.
- **In-loop `#remove`** (sorted/index) and its rejection (hash).
- **Descending keys** (`-k`) and multi-field sort order.

## The generator model (what F2.1 builds against this contract)

An `arbitrary`-derived `KeyedSpec { kind: Hash|Sorted|Index, key_shape,
n_keys, ops: Vec<Insert|Remove>, value_kind }`. `generate_keyed(spec)`:
1. draws `n_keys` **distinct** keys from an indexed pool (distinctness by
   construction — failure path 2 impossible);
2. folds `ops` into an expected `Map<Key, Value>` (survivors + last value);
3. emits the program: the declaration for `kind`, the insert/remove statements,
   then assertions baked from the expected map (population, per-key lookups,
   the ordered-key string computed in the type's declared order).

`f2-exemplars/ex_{hash,sorted,index}.loft` are the three concrete shapes this
generator parameterizes — the grammar templates.

## Exit criterion (F2.0)

Met: the invariant, the three types' exact facts (read off exemplars that pass
on both backends), the failure paths, the single-model chokepoint, and the
parameterized-not-identical guard are all pinned. F2.1 (reify `generate_keyed`
behind `fuzzing`+`cfg(test)`) may proceed against this contract; F2.2 plants a
mis-generated program to prove the self-check can fail before F2.1's sweep is
trusted.
