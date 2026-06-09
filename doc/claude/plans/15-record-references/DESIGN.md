<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN15 — DESIGN — the two-edge model, copy/cross-tree fixup, serialise/deserialise

Mechanism detail for [README.md](README.md). The invariant and the Stage-A matrix live in
the README; this file is the *how*.

## The model — two kinds of edge

| | Owner edge (exists today) | Reference edge (this plan) |
|---|---|---|
| Cardinality | exactly one per record (the tree) | many per record; optional |
| Stored as | offset-4 back-pointer (`get_u32_raw(rec, 4)`) | 4-byte same-store rec-id (`ChildRec`-like, non-owning) |
| Identity | **position-derived** — read on demand by `path()` | **position-bound** — a stored pointer |
| On relocate/copy | follows for free (re-read offset-4) | **breaks unless fixed up** |
| Serialised as | nesting / `[index]` | a canonical **path key** |

## What already exists (do not rebuild)

- **Key generator** — `Stores::path(db, tp)` (`src/database/format.rs:114`): walks up the
  offset-4 owner pointer to root (`rec==1` → `"/"`), then at each level finds the parent
  field whose content type `contains(tp)` and appends `field[index]` (positional, by
  scanning the vector) or `field[key]` (keyed collection). Shape: `/users[3].posts[id]`.
- **Schema knowledge** — `Type.parents: BTreeSet<u16>` (`src/database/types.rs:1779`,
  populated at `types.rs:116/126`): which struct types embed each type.
- **Owner back-pointer** — offset 4, maintained for collection backbones (`structures.rs:49`)
  and tree/hash element records; already **rewritten on copy** for tree nodes
  (`allocation.rs:972/1032`).
- **Emission placeholder** — the TODO at `format.rs:887` in `write_struct`. `path()` is
  wired into no serialiser today.

Missing halves: (a) a reference field *type*; (b) *emit* the key on write; (c) *resolve*
the key on read; (d) *remap* references on copy; (e) lifetime.

## Phase 01 — schema + surface

A reference field stores a **4-byte same-store rec-id** — the target record's word offset in
the *same* store, exactly like `Parts::ChildRec` but **non-owning** (`ChildRec` deep-copies
its child; a reference owns nothing and is never followed for ownership). **Cross-store
references are deliberately unsupported**: a pointer into another store would hamper that
store — it could no longer be freed, serialised, or `clone_for_worker`-cloned independently —
breaking the store-isolation invariant the threading model rests on. That isolation, not the
4-vs-12-byte cost, is the deciding factor. Reuse `ChildRec`'s 4-byte storage + copy machinery
as the starting point.

**Referenceability is a type-system property — the single source of truth the whole scheme
hinges on.** It converts the riskiest constraint ("a reference target must be a stable
standalone record") from a runtime hazard into a **compile-time error**: the compiler
rejects `&T` whenever `T` is not a referenceable type. This is the design-protocol
*make-omission-loud* cure (Goal E applied to the representation) and the chokepoint every
later phase consults.

`referenceable(T)` **cascades into layout** — and this is the "only the records that need it"
cost concentration:

- `referenceable(T)` ⇒ **T is always a boxed standalone record**, so it automatically has a
  stable address, an offset-4 owner (for `path()` keys), and room for the gen/tombstone word
  (Phase 06). A `vector<T>` of a referenceable `T` becomes **by-reference** storage, not
  inline — so "cannot reference a by-value vector element / inline sub-struct" is not a guard
  to enforce but a state that **cannot arise**: referenceable types are never stored by value.
- Every **non-referenceable** type is untouched — no owner maintenance, no gen word, still
  inlined. Cost lands only where referenceability was declared.

Legal targets are therefore exactly the boxed records: top-level records and by-reference
collection elements (`sorted` / `hash` / `index`). Field syntax proposal: `&Type`.

Open question 1 (representation) — **resolved 2026-06-09**. Open question 5 (declared vs
inferred referenceability) drives whether the flag is authored on the type or computed at
`finish()` from `&T` occurrences (like the existing `Type.parents`).

## Phase 02 — owner-pointer guarantee (drive `N × silence → 0`)

"offset-4 always names the owner" must hold at every record **create / move (collection
resize, tree rebalance) / deep-copy** site — `grep "set_u32_raw(_, 4, _)"` shows ~20 sites
(many are vector-length writes; the owner-writes are the live ones). Omitting it anywhere is
*silent* future-key corruption. Cure: route referenceable-record claims through **one "claim
record under owner P" chokepoint** that always sets offset-4, **and** add a debug
`validate_parents()` (sibling of `tree::validate` / `hash::validate`) that walks every
referenceable record up to root and panics on a zero / non-terminating chain — a missed
parent becomes a loud test failure, not a corrupt key months later.

## Phase 03 — serialise (the TODO at `format.rs:887`)

Add a reference-field arm to `ShowDb::write` that calls `self.stores.path(target, target_tp)`
and emits the key. Own-format (`loft:true`, `show_loft`) and JSON (`$ref`-shaped) emit the
key; debug may keep the raw `DbRef(...)` shape. The generator exists — mostly dispatch +
choosing the on-the-wire key syntax.

## Phase 04 — deserialise (inverse of `path()`)

Add `resolve(path_key, root) -> DbRef`, navigating field names + `[index]` / `[key]` from the
root. Forward references resolve in a **deferred fixup pass**: during `walk_parsed_into`
(`format.rs:281`) collect `(ref_field_location, path_key)`; after the whole tree is
materialised, resolve all and write the `DbRef`s. Errors flow through the existing
`"line N:M path:X"` formatter.

## Phase 05 — copy / cross-tree robustness (load-bearing)

`copy_claims` (`src/database/allocation.rs`) is the deep-copy walk: recurses
Struct / Vector / Array / Hash / Index / `ChildRec`, handles cross-store via
`copy_block_cross_store` (`allocation.rs:1155`), and already rewrites tree-node owner
pointers. References add **one rule and one pass**:

1. **Owner edges** — followed; the copies' offset-4 rewritten to the new owner (generalise
   the tree-node behaviour to every referenceable record, via the Phase-02 chokepoint).
2. **Reference edges** — **not** followed as ownership (else the copy drags in the whole
   reachable graph). The copy builds a **`rec → rec′` remap table** for every record it
   copies, then runs a **reference fixup pass** over the copy:
   - target **inside** the copied region (internal edge) → remap to `rec′`;
   - target **outside** the region (external edge): in-store relocation preserves it
     unchanged (same store, rec still valid). **Cross-store** copy of a subtree with an
     external edge is a clean **error** (or null-the-edge) — references are same-store, so a
     subtree must be *reference-closed* to migrate cross-tree (Open question 3, resolved by
     Q1).

This is the **same fixup abstraction as Phase 04**: deserialise resolves *path-keys → DbRef*;
copy resolves *src-rec → dst-rec* via the remap table. One "reference fixup" concept, two key
spaces (path-keys at the textual boundary, the remap table at the in-memory boundary).
Keeping it **one concept** is the chokepoint that holds the re-assertion count down.

Copy edge-cases the probes (P6) must cover: internal ref (forward + backward within the
region), external ref (in-store), self-reference, cycle (A↔B inside the region), an external
ref under cross-store copy (must produce a clean error, never corruption), and the headline
case — a vector whose elements reference each other, relocated within a store **and** copied
cross-store.

## Phase 06 — lifetime (generation-tagged weak references)

A reference edge is non-owning, which breaks loft's `dep.is_empty() == owner` rule
([LIFETIME.md](../../LIFETIME.md)). Two consequences:

1. **`get_free_vars` treats a stored reference as borrowed** — never emits `OpFreeRef` through
   it. The target's lifetime is owned solely by its tree owner; the reference only observes.
2. **Staleness is detected, not prevented (Open question 4 — resolved: generation tag).** A
   referenceable record carries a **generation counter**; a reference stores the generation it
   captured when it was set, alongside the rec-id (`(rec, gen)`). The counter is **bumped on
   free** (one site) and **checked on resolve** (one site); a reference whose captured gen no
   longer matches the record reads **null** — safe even after the slot is reused (the gen
   mismatch catches the ABA the user flagged, so the allocator may reclaim the slot
   immediately; no tombstone hold, no sweep needed for correctness).

This makes references **weak pointers** (`Weak<T>`), consistent with the non-owning decision —
unlike a free-blocking refcount, which would make the reference a co-owner, cost maintenance
at every set / copy / free, and leak reference cycles. Generation touches exactly two sites,
so the re-assertion brittleness is near zero; it costs a gen word per referenceable record
(Phase 01 boxes the type, so the slot exists) + ~4B in the reference. Matrix P5 (free owner
while referencer lives) and the cycle cases pass by construction.

## Re-assertion sites — the brittleness, known now

Re-stated at: record **create** (structures.rs, allocation.rs, vector.rs, hash.rs, tree.rs),
record **move** (collection resize, tree rebalance), **deep-copy** (`copy_claims` +
`copy_claims_seq_vector` / `_array_body` / `_hash_body` / `_index_body`), **serialise** (one
arm), **deserialise** (one resolver), **free** (`get_free_vars`). The two cures above — the
single claim chokepoint (02) and the single fixup concept shared by copy + deserialise
(04/05) — exist specifically to keep this count near 1 with loud failure on omission.
