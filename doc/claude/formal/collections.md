<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/collections.md — collection kinds, indexing & slicing (strict) · **SCOPE**

> **STATUS: SCOPE (2026-07-10).** This is the *scoping* pass, not the finished rules. It
> inventories the shipped behavior of loft's collection kinds — `vector`, `hash`, `sorted`,
> `index`, `spatial` (`Radix`), `trie` — and the shared index/slice surface, names each rule with its
> intent + anchor, and lists what must be **both-backends-verified** before the rules graduate to
> the normal `## Rules` / `## Deviations` form at **0 deviations**. This is **describe-SHIPPED-
> behavior** mode (the usual `formal/` discipline — the opposite of the spec-first @PLN35 work).
>
> **Why this doc exists.** `spatial` shipped (PR #550) into an area the formal spec never covered:
> the collection **type formers** aren't in [types.md](types.md), and **indexing/slicing** is
> nowhere. Today the only keyed-collection rule is [concurrency.md](concurrency.md)'s `C-Order`
> (hash bucket-walk under `par`) plus passing mentions in [calls.md](calls.md) / [capabilities.md](capabilities.md).
> This doc closes that gap. It also firms up the ground [matching.md § PEG patterns](matching.md)
> (@PLN35) stands on — its pattern cursor reuses the same slice/`⟨i, src⟩` surface.

---

## 0. The shared slice mechanism (the load-bearing structural fact)

Slicing is **one type-directed mechanism**, not per-structure code. The index/slice parser
(`src/parser/fields.rs`, the `[...]` dispatch at ~`:660-717`) branches on the receiver type into
**two families** with different result kinds:

| family | receiver | form | result | anchor |
|---|---|---|---|---|
| **Value slice** | `vector<T>`, `text` | `v[a..b]` `[a..=b]` `[a..]` `[..b]` | a **fresh sub-collection VALUE** (`vector<T>` / `text`); bounds clamp | LOFT.md:1203-1206, :790-813; fields.rs:670-679 |
| **Keyed range-slice iterator** | `sorted`, `index`, `spatial`(`Radix`), `trie` | `c[lo..hi]`; spatial `c[(x,y)..(x,y)]` / `[(x,y)..]` / `[(x,y)..:n]`; trie `c[pre..]` | a **`for`-only iterator** (`Value::Iter`) over the raw KEY interval — **not a value** | fields.rs:688-717, :1228 (`D-key-1`), :779 (trie); STDLIB.md:277-281 |

`hash` has **point lookup only** (`h[key]`) — no order ⟹ no range slice. `spatial` is the
**multi-axis (tuple-key, Morton) instance** of the keyed-range-slice family; nothing about slicing
is spatial-specific except the tuple key and the Morton interval. `trie` is the **single-`text`-key
instance**, whose range is a PREFIX rather than a scalar interval (`parse_trie_slice`; a trie never
reaches the generic scalar-range branch). This split is the doc's spine.

---

## 1. Rule inventory (labels + intent; each grounded in shipped behavior)

### 1.1 Collection kinds & type formers — `Col-Type` (some land in [types.md](types.md))

```
  (Col-Vec)     vector<τ>                     ordered, 0-based integer index; the only value-sliceable collection.
  (Col-Hash)    hash<T[k…]>                   O(1) point lookup by key; UNSORTED (bucket) order.
  (Col-Sorted)  sorted<T[k…]>                 red-black tree; KEY-ORDERED iteration + range slices.
  (Col-Index)   index<T[k…]>                  BOTH — a red-black tree AND a hash table over the same records
                                              (O(1) lookup AND ordered/range).  (DATABASE.md:704)
  (Col-Spatial) spatial<T[a]> / [a,b] / [a,b,c]   1–3 coordinate axes (MAX_AXES=3), Morton/Z-order radix tree;
                                              the runtime Parts variant is `Radix`.  integer-not-null coord keys;
                                              negative coords via offset-binary (signed axes order like sorted).
  (Col-Trie)    trie<T[k]>                    a radix tree over ONE `text` key field: exact lookup, KEY-ORDERED
                                              iteration, and a PREFIX slice — the operation the kind exists for.
                                              Shares `radix_tree` with `Radix` and nothing above it: `Radix` is
                                              GEOMETRIC (Morton interleave, boxes, nearest) and none of that
                                              means anything for a word, which is why `spatial` is not spelled
                                              `radix` at the surface.
```
*Anchors:* `Type::{Vector,Hash,Sorted,Index,Radix,Trie}` (src/data.rs); DATABASE.md:693,:704; spatial
surface tests/scripts/48-spatial-construct-free.loft; trie `Parts::Trie` (database/mod.rs:194) + @PLN134. **To decide when writing:** which formers
live here vs in types.md's former list (recommend: types.md gains the one-line formers; collections.md
owns their *operations + order*).

### 1.2 Construction, insert, length — `Col-Cons` / `Col-Insert` / `Col-Len`

```
  (Col-Cons)    c: <kind><…> = []             empty-literal construction (all kinds).
  (Col-Insert)  c += [ rec, … ]               append/insert a record; keyed kinds place it by key.
  (Col-Len)     c.len()                        element count; O(1) (verified O(1) for spatial).
```
*Anchor:* tests/scripts/48-spatial-construct-free.loft (construct/append/len).

### 1.2b Removal — `Col-Remove` (a vector RENUMBERS; a keyed kind does not)

```
  (Col-Remove)      v.remove(i)  ·  v#remove  ·  c[key] = null  ·  e#remove
                    delete one element.  The two kinds differ in what happens to the OTHERS:
  (Col-RemoveDense) a VECTOR stays DENSE.  Removing index i shifts every later element down
                    one, so len decreases by 1 and every position after i is RENUMBERED.  There
                    are no holes and no tombstones: index j > i now names what was at j+1.
  (Col-RemoveKeyed) a KEYED kind (hash / index / sorted / spatial / trie) removes BY KEY, and every
                    other key stays reachable and unchanged — keys are not positions, so nothing
                    is renumbered.
```
*Anchor:* tests/scripts/200-vector-stays-dense.loft (density); measured for the keyed kinds —
`s[30] = null` on a `sorted<Elm[key]>` leaves `s[10]` and `s[50]` intact.

**Why this rule is load-bearing beyond collections.** `Col-RemoveDense` is exactly what makes a
held element reference go stale: a reference names a POSITION, and a removal renumbers positions,
so a reference taken before the removal names a different element after it. That is the fact
[binding.md](binding.md)'s `B-Disturb` / `B-Ref-Reshape` and [heap.md](heap.md)'s `H-Materialise`
rest on, and it is why density is a *contract* rather than an implementation detail: a
hole-punching vector would keep references valid and was decided against (@PLN130 F3) because
every read would then pay for the check.

### 1.3 Point lookup is nullable — `Col-Lookup` (reuses [types.md](types.md) `τ?`)

```
  (Col-Lookup)  Γ ⊢ c[key] ⇒ τ?              a keyed point lookup is NULLABLE — an absent key yields the
                                              null record (P285), discharged by `?? d` / `match` like any τ?.
```
*Anchor:* fields.rs:700-706 (P285, the `expr_not_null` clear); mirrors types.md `(N-Index)` for `v[i]`.

### 1.4 Iteration order per kind — `Col-Order` (EXTENDS [concurrency.md](concurrency.md) `C-Order`)

```
  (Col-Order)   for x in c { … } visits in a per-kind ORDER, identical on both backends:
                  vector  → index order 0,1,2,… (iteration.md I-For)
                  hash    → UNSORTED bucket walk (no key order) — the C-Order decided edge
                  sorted  → key order
                  index   → key order (its tree side)
                  spatial → Morton / Z-order
                  trie    → key order (lexicographic over the text key)
```
*Anchor:* concurrency.md `C-Order` (hash); STDLIB.md/DATABASE.md (spatial Morton). **This is the
divergence-prone rule** (interp store-walk vs native emitted loop) — the whole reason the area needs
pinning. `C-Order` already states the hash edge; `Col-Order` generalises it to every kind.

### 1.5 Value slices (vector / text) — `Slice-Value`

```
  (Slice-Value)  v[a..b] / v[a..=b] / v[a..] / v[..b]  yields a FRESH sub-collection value:
                   vector<τ> → a fresh vector<τ> (H-Alloc); text → a text substring.
                 Bounds CLAMP: a partial-OOB slice returns the in-range part; a fully-OOB slice ⟹ [].
                 `..` is end-EXCLUSIVE, `..=` end-INCLUSIVE.  (Index vs slice asymmetry for text:
                 v[i] ⇒ character, v[i..j] ⇒ text.)
```
*Anchors:* LOFT.md:1203-1206, :790-813; clamp behavior plans/25-nullable-sequences/README.md:234.
**To verify when writing:** the exact clamp values on both backends; freshness (a value slice is
independent of the source — cross-link heap.md H-Alloc / iteration.md I-Comp).

### 1.6 Keyed range-slice iterators — `Slice-KeyedIter` (`D-key-1`, the shipped decided edge)

```
  (Slice-KeyedIter)  a keyed range slice c[lo..hi] is a `for`-ONLY ITERATOR (Value::Iter) over the raw
                     KEY interval, in the collection's key order.  It is NOT a value: `x = idx[lo..hi]`
                     in value position is a STATIC ERROR ("a keyed range slice is a for-loop iterator,
                     not a value — iterate it").  (Applies to sorted / index / spatial / trie.)
```
*Anchors:* fields.rs:1228,:1237 (`D-key-1`); RELEASE.md (the D-key-1 crash-fix, value-position reject);
STDLIB.md:281. sorted-slice design: [../plans/38-sorted-slice/](../plans/38-sorted-slice/).

### 1.7 Spatial slices — `Slice-Spatial` (the Morton specialization of `Slice-KeyedIter`)

```
  (Slice-Box)    xs[(x1,y1)..(x2,y2)]   iterate records whose MORTON code is in [code(x1,y1), code(x2,y2)],
                                        in Morton order.  This is a SUPERSET of the geometric box — Z-order
                                        threads codes outside the box IN, so the caller filters/`break`s for
                                        an exact shape.  (INV-Superset — a deliberate contract, not a bug.)
  (Slice-Open)   xs[(x,y)..]            open outward walk from a point; the caller `break`s to stop.
  (Slice-Cap)    xs[(x,y)..:n]          same, capped at n records (k nearest-in-Morton).  EXACTLY
                                        n when the collection holds n — the cap does not vary with
                                        where the query sits (answers open question 4 below).
                 1–3 axes; lowers to n_spatial_range(...); the same scratch path as iteration.
```

> **`Slice-Open`/`Slice-Cap` HELD only from 2026-08-19** (loft#1002). Until then both lowered to
> `radix_db::range` — the one-directional walk — so they answered the Z-order **tail**: records
> at or after the query only. A record one code behind was unreachable however close it was
> (from `(12,11)`, `C` at distance 2.2 was never returned while `E` at ~12 was), and the cap
> under-delivered by however close the query sat to the end of the curve — measured 3, 3, 3, 2,
> 1, 0 over five records as the query moved along, with a query past every record answering
> nothing at all. The rule above is what settled it: the issue proposed *"keep the tail and
> rename it"* as an equal option, and it was not one — the code changes to match the rules.
> Now lowers to `radix_db::near_range`, the n-axis form of `spatial::near` (two cursors seeded
> either side of the query, each step yielding whichever is closer), which existed, was correct,
> was unit-tested, and no loft program could reach.
>
> **The walk is APPROXIMATE and the rule says so** — `k nearest-in-Morton`, not nearest-in-space.
> Morton distance tracks euclidean distance closely but jumps at quadrant boundaries, so a
> truly-near point can arrive a place late; `Slice-Box` is the exact form. Every record is
> yielded eventually, each once, which is what makes `break` the intended way to stop.
*Anchors:* fields.rs:688-696,:1558 (parse_spatial_slice); default/01_code.loft:1176
(`spatial_range`); STDLIB.md:272-281; DATABASE.md:668-674; radix_db.rs:238 (superset comment);
tests/scripts/48b-spatial-slice.loft (the asserted box/open/cap slices). CAVEATS.md:593 (spatial op set).

### 1.8 Storage & whole-value copy — `Col-Store` / `Col-Copy` (cross-link [heap.md](heap.md))

```
  (Col-Store)   a collection is store-backed (Parts::{Vector,Hash,Sorted,Radix,Trie}); index = tree+hash over
                one record set; addressed by DbRef.  (Layout/format ⟶ layout.md; steps ⟶ heap.md.)
  (Col-Copy)    a keyed whole-value bind COPIES (g = h; g += … leaves len(h)) — heap.md H-Copy for keyed.
```
*Anchors:* DATABASE.md:693,:704; VERIFICATION.md heap.md "H-Copy (keyed)" (oracle `16`).

### 1.9 The linked GROUP — `Col-Group` (cross-link [DATABASE.md § Clearing one member](../DATABASE.md))

```
  (Col-Group)   two or more collections over ONE element type in ONE struct are several ROUTES
                to a single record set, provided at least one of them is keyed.  A record
                entering through any member is in every member, by any write route.  Membership
                is a fact about the PAIR — not about declaration order, not about which member
                is written first, and not about whether the element is dense (vector<E>) or
                nullable (vector<E?>).
                Two members neither of which is keyed (two plain vectors) are INDEPENDENT.
```

Five fixes are all instances of this one rule, which is why it is written here rather than left
to the issues: `trie`/`spatial` were absent from the pairing test (loft#927); the `others` link
ran one way, so which member maintained the rest depended on declaration order (loft#843); the
test asked only whether the field being ADDED was keyed, so a plain `vector<E>` declared second
formed no group (loft#1158); only `hash` had its element rewritten to a nullable sibling's
`__nullable<E>`, so the other four kinds no longer matched by content; and a whole vector VALUE
(`data = rows()`) reached only the member it was assigned to, because the bulk write never
passed the per-record chokepoint that maintains the group (loft#1152, and loft#1159 for the
same route into a KEYED member).

Every one of them **failed silently** — the pairing was never refused, a second independent
collection was built instead, and `len` of the empty view is a legal value.  That is the shape
to test for: a group's failure mode is not an error, it is a zero.

⚠ **Not settled by this rule: which member HOLDS the records.**  The first-declared member is
the holder and the rest are views.  loft#1158 predicted that a keyed-first group would need the
vector made holder regardless of order; measured, it does not — all four write routes
(element-wise `+=`, whole vector value, keyed literal, keyed `+=`) read back complete through
both members in both orders, on both backends, under `LOFT_STRICT_STORES=1`, with no holder
machinery touched.  The holder choice is not observable through the rule, so the rule does not
name one.

⚠ **OPEN (loft#1160):** *"by any write route"* does not yet hold for a write spelled through an
enum variant's `match` / `is` field BINDING.  The binding is a view of the field, so the write
lands in the member it names and reaches no sibling — measured in both declaration orders, on
both backends.  `record_finish` maintains a group only when it is given the FIELD the write is
spelled through, and the binding does not carry it.

*Anchors:* `Stores::field` (`src/database/types.rs`, the pairing test + `other_indexes`);
`Parser::link_shared_nullable_views` (`src/parser/definitions.rs`, the nullable-element
rewrite); `Stores::record_finish` (`src/database/structures.rs`, the per-record sibling
insert); `Stores::insert_keyed_copy` (`src/database/search.rs`, the one keyed insert both the
point write and the bulk fill take); DATABASE.md § Clearing one member of a linked group;
tests/scripts/a-keyed-view-joins-a-nullable-element-vector.loft;
tests/scripts/a-collection-group-does-not-depend-on-declaration-order.loft;
tests/scripts/1158-a-group-forms-whichever-member-is-declared-first.loft;
tests/scripts/1152-a-vector-value-into-a-group-reaches-every-member.loft;
tests/scripts/1159-a-keyed-collection-filled-from-a-vector-value.loft;
tests/scripts/927-trie-spatial-linked-group.loft;
tests/scripts/901-linked-group-fill.loft.

---

## 2. Invariants (the both-backends contracts this doc pins)

- **INV-Order** — per-kind iteration order (`Col-Order`) is IDENTICAL on `--interpret` and `--native`.
  The load-bearing one: interp walks a store index, native emits a Rust loop, so a reordering in
  either is a definitional error (the `C-Order` precedent, generalised to every kind incl. spatial Morton).
- **INV-KeyedSlice** — a keyed range slice is a `for`-only iterator, never a value (`D-key-1`); a
  value-position use is rejected identically across `--dump`/`--interpret`/`--native` (driver-agreement).
- **INV-Superset** — a spatial box slice yields a SUPERSET of the geometric box (caller filters). The
  honest contract; both backends return the same superset (same Morton interval), so a divergence in
  membership or order is the error.
- **INV-LookupNull** — a keyed point lookup is `τ?` (absent ⟹ null); enforced by `(N-Store)` like any
  other nullable, both backends.
- **INV-SliceFresh** — a `vector`/`text` value slice is a FRESH, independent value (H-Alloc); mutating
  it never touches the source.

## 3. Deviations / decided edges to record (expected: mostly decided edges, 0 or few OPEN)

- **`C-Order`** (hash bucket-walk) — already a decided edge in concurrency.md; `Col-Order` references it.
- **`D-key-1`** (keyed slice = iterator) — a shipped decided edge (the value-position crash was fixed to a
  clean diagnostic, RELEASE.md 2026-07-04); formalized as `INV-KeyedSlice`, not an open deviation.
- **INV-Superset** — a deliberate design decision (raw Morton interval), not a deviation; record as an edge
  with a DESIGN_DECISIONS cross-link.
- **Candidate OPEN (verify):** the per-query scratch-vector allocation for spatial slices (CAVEATS.md notes
  it as the next efficiency lever) — a performance note, likely NOT a formal deviation.

OPEN: **0** — `D-col-null` was opened and CLOSED the same day (2026-08-28, below).

### `D-col-null` — OPENED AND CLOSED (2026-08-28, loft#1120): two answers to *"is this collection null?"*

`(Col-Lookup)` and `(N-Index)` make an absent element that type's null, and `(E-Coalesce)` makes
`e ?? d` yield `d` for exactly that null.  One value, one null, one answer — and the tree carried
two, each right about the half the other got wrong.

`??` asked `OpConvBoolFromRef` (`rec != 0`).  That reads the encoding a MISSED LOOKUP uses and
nothing else, so a nullable collection FIELD — whose read is a sub-reference carrying the HOLDER's
record — was "present" whatever the slot contained: the default was unreachable, and a `hash` /
`index` field then dereferenced the record the absent slot names and stopped the run.  `==  null`
asked `OpVectorIsNull`, which reads the handle sentinel and the slot word but called a record-less
DbRef present, so `vv[9] == null` answered `false` for an index plainly out of range.  `spatial`
and `trie` were in neither list: the coalesce's hand-written variants named `Vector`/`Sorted`/
`Hash`/`Index` only, so they fell to the generic convert, which hands back the bare handle —
`--interpret` read twelve pointer bytes as a boolean and `--native` would not compile the `if`.

Closed by giving the question ONE implementation: `vector::is_absent_collection` answers ABSENT for
a DbRef that reaches no slot (the missed-lookup encoding it used to call present), and the coalesce
asks `Parser::collection_is_null` — the lowering `== null` already used — through
`is_collection_type`, which names every kind including `Radix` and `Trie`.  The condition position
(`if c`) shares that lowering and was wrong in the same three ways.

⚠ **The oracle under the neighbouring `OPEN: 0`s could not see this.**  Five guards already covered
nullable collection fields (`909`, `917`, `920`, `922`, `936`) and every one of them writes `?? []`
— and empty is what the wrong answer looks like, so each cell agreed with itself.  A default whose
length differs from both the empty and the present arm is what separates them; that is what
`tests/scripts/1120-one-null-question-for-a-collection.loft` writes, over six collection kinds ×
{null, empty, filled} × {field, element field, parameter, handle, lookup} × {`??`, `== null`, `if`}.

## 4. Conformance / oracle plan (how each rule gets pinned — [VERIFICATION.md](VERIFICATION.md))

Existing coverage: oracle `16` (keyed copy / hash behaviour). To add, as a `collections.md` block in
VERIFICATION.md (one ☐ row per rule, both-backends + leak + driver-agreement):
- `Col-Order` per kind (esp. spatial Morton order + hash unsorted vs sorted key-order).
- `Slice-Value` clamp + freshness (vector + text).
- `Slice-KeyedIter` value-position REJECT (driver-agreement) + iterate-in-key-order.
- `Slice-Box/Open/Cap` — the superset membership + `:n` cap + open-walk `break` (extend
  tests/scripts/48b-spatial-slice.loft → an oracle program).
- `Col-Lookup` nullable (absent key ⟹ null, discharge required) — pinned by
  `tests/scripts/1120-one-null-question-for-a-collection.loft`, which scores `??`, `== null` and the
  condition position against each other so no two of them can drift apart again.  Its defaults are
  never `[]`: see `D-col-null` for why that is the whole difficulty.

## 5. Open questions / to-verify when writing the rules

1. **Former placement** — collection type formers in types.md (one-line each) vs here (recommend split:
   types.md = the former, collections.md = operations + order).
2. **Exact vector-slice clamp** — hand-verify the partial-OOB and fully-OOB values on both backends
   (plans/25 says partial → in-range part, fully-OOB → `[]`); pin the boundary.
3. **sorted vs index slice** — do they differ observably (index has both tree+hash)? Confirm both expose
   the same `Slice-KeyedIter` iterator; is a `hash` range slice a clean reject (no order)?
4. ~~**`:n` cap semantics** — exact count guarantee for `[(x,y)..:n]` (≤ n? exactly n if available?)~~
   **ANSWERED 2026-08-19 (loft#1002): exactly n when the collection holds n, from any origin.**
   The cap bounds the WALK, and the walk is outward from the query rather than onward from it, so
   the count no longer depends on where the query lands. Pinned in
   `tests/scripts/48b-spatial-slice.loft`, which sweeps the origin along the curve (the axis the
   count-only cells above cannot see) and asserts WHICH records each origin answers, with the
   euclidean distances each expectation follows from. The superset interaction is `Slice-Box`'s
   only — the open forms do not filter, they order.
5. **Both-backends spatial order** — is Morton order proven identical interp-vs-native? (48b runs both +
   leak; confirm it also pins ORDER, not just set membership.)
6. **Scope boundary** — does this doc also state `for x in c` (iteration.md already owns `I-For`; here just
   the per-kind ORDER as `Col-Order`, cross-linking rather than restating)?

## 6. See also

- [types.md](types.md) — the type formers + `τ?` (lookup nullability, value-slice element type).
- [iteration.md](iteration.md) — `I-For` cursor; `Col-Order` fixes the per-kind order it iterates.
- [concurrency.md](concurrency.md) — `C-Order`, the hash edge this generalises.
- [heap.md](heap.md) — store steps (`H-Alloc` for fresh slices, `H-Copy` keyed), [layout.md](layout.md) — byte layout.
- [matching.md § PEG patterns](matching.md) — @PLN35 reuses the slice / `⟨i, src⟩` cursor surface.
- Code: `src/parser/fields.rs` (the shared index/slice dispatch, `parse_spatial_slice`, `D-key-1`);
  DATABASE.md / STDLIB.md (the user-facing surface); [../plans/38-sorted-slice/](../plans/38-sorted-slice/).
