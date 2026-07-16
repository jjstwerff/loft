<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — const model Phase 2: value-const on fields (deep-frozen records)

**Status: DONE (Phase 2 CORE, steps 0–5).**  Landed on `tuxedo-work`: value-const
field flag + parse (INERT) → LHS chain-walk (INERT) → `validate_write` enforcement →
builders proven untouched (audience_crystal / gridmesh / routing_kernel) → graduated
tests (`tests/scripts/40-const-fields.loft` positives + `tests/issues.rs pln40_vc_*`
negatives, both backends) + docs (LOFT.md § Fields four-quadrant table + loft-write).
Store serialization deferred to Phase 2b (INERT, documented in `ir_read.rs`);
type-carried transitivity is Phase 3 (post-1.0).  Continues
[const-model.md](const-model.md) (Phase 1 shipped: value-const params/locals + scalar
collapse + binding-const fields).  Phase 2 adds the **value-const FIELD** — the missing
half of the `const v: const T` fully-frozen record — enforced for direct writes by one
LHS-chain walk.  The heavier **type-carried** transitivity (survives store round-trips,
function-return laundering, `vector<const T>` generics) is split out as **Phase 3**
(post-1.0; no library needs it yet — see the analysis).

## Library analysis (the dogfood grounding — what actually needs what)

Full sweep of every installable lib's `const` fields, by field type:

| field kind | count | Phase-1 status | needs Phase 2? |
|---|---|---|---|
| **scalar** `const c: integer/float/…` | **251** | **FROZEN** — `validate_write`'s contents-append exemption is text/vector/keyed only, so a scalar const field rejects `s.c = x` AND `s.c += x` | no — done |
| **compound COLLECTION** `const v: vector/hash<T>` | **91** | binding-const: slot write-once, **contents mutable** (`s.v[i]=`, `s.v+=`) | **NO — these are BUILDERS** |
| **compound STRUCT/ENUM** `const v: SomeType` | **~16** | binding-const: slot write-once, `s.v.x=` allowed | some (records) |

**The decisive finding: the 91 compound-collection const fields are BUILDERS, not
records.**  The mutation sweep proves it — every package with them is heavily
contents-mutating them: audience_crystal 17 fields / 26 field-writes, gridmesh 8 / 18,
routing_kernel 12 / 21, arguments 3 / 7 (the zero-write ones — hex_terrain 12 / 0 — write
through `&`-borrows instead, same thing).  These fields WANT `s.v += …` / `s.v[i]=…`;
binding-const is exactly right for them.  **Deep-freezing them would break every builder.**

So Phase 2 is **strictly additive and opt-in**: it introduces value-const on a field
(`v: const T`) as a *new* thing a lib author reaches for on a genuine value/record type,
and **must not change the meaning of the 91 binding-const builders**.  No current lib is
blocked on it — the value/key records that matter for safety today (`ChunkKey`,
`GroupKey`, `Coord`, `CellRef`) are all-scalar and already frozen.  It ships before 1.0
for **completeness + future safety** (an opt-in truly-immutable record: shared config, a
compound hash key, a value handed to `par()` threads), not to fix a current break.

## The invariant (one sentence)

> A write is rejected **iff its LHS access chain passes THROUGH a value-const step** — a
> value-const base *variable* (Phase 1) **or** a value-const *field* dereferenced along
> the way — determined by ONE downward type-tracking walk of the LHS.  A value-const
> value is therefore read-only at every depth, while a *rebind* of the value-const
> binding/field itself (`s.v = other`) re-points the slot and is allowed.

This is the exact Phase-1 base-resolution generalized: Phase 1's `lhs_base_var` checked
only the leaf variable; Phase 2 walks the SAME chain but also inspects each **field
dereference** for `value_const`, tracking the type top-down so it knows each node's type.

## Re-assertion sites — count N (the brittleness tell)

The rule is enforced at exactly the write chokepoints Phase 1 already funnels through:

- **`validate_write`** (`expressions.rs`, component writes `s.f=`, `s.f[i]=`, `s.a.b=`) —
  the ONE site the new chain-walk plugs into.
- `const_write_blocked` (`operators.rs`/`collections.rs`, direct `x=`/`x+=`) — unchanged;
  a direct write to a value-const *field* is a rebind (`s.v = x`), which is allowed, so
  no new logic here.

**N = 1** load-bearing site (the chain-walk fn, consulted once in `validate_write`).
Omission is not silent — a missing check means a deep-frozen field is mutable, caught by
the Step-0 matrix on both backends.  No `N > 1` spray.

## Representation — NO `Type::Const` wrapper (the ripple is measured)

A `Type::Const(Box<Type>)` wrapper would ripple through **1576** `Reference/Vector/Enum/
Text` construct+match sites in `src/` (measured) — the exact codegen-regressing change
Phase 1 avoided.  Phase 2 CORE needs no `Type` change at all: value-const on a field is a
**`bool` on `Attribute`** (`value_const`), and the chain-walk reads it by resolving each
LHS field node's `(def, position)` — the machinery `validate_write` already uses for
`const_field`/`mutable`.  The type-carried bit (Phase 3) is where `Type`/`Deps` threading
comes in; deliberately deferred.

## Small safe steps (each contained; introspect + matrix on BOTH backends)

| # | Step | Verify |
|---|---|---|
| 0 | **Boundary matrix (throwaway).**  A `value struct Rec { v: const vector<int>, r: const Inner }` × {rebind `s.v=`, append `s.v+=`, element `s.v[i]=`, nested `s.r.x=`, read} plus a **binding-const builder twin** (`const b: vector<int>` — `s.b+=` MUST stay legal).  Hand-compute: value-const field ⇒ rebind ✓, all mutation ✗; builder ⇒ rebind ✗, mutation ✓.  Snapshot CURRENT (Phase-1 gaps: value-const field mutation wrongly allowed). | spec recorded |
| 1 | **`Attribute.value_const` + parse + serialize (INERT).**  Add the bool; parse `const` before a field TYPE (`v: const T`, `const v: const T`) → set it (was a parse error — purely additive).  Thread through IR: `ds::ATTR_VALUE_CONST` in `ir_store.rs`/`ir_read.rs` + the JSON in `ir_schema.rs` + the snapshot test.  Nothing READS the flag yet. | parses; flag set (white-box); `ir_schema_roundtrip` + suite green; loft-codegen byte-identical for non-value-const structs |
| 2 | **The chain-walk `lhs_frozen_through` (INERT).**  Generalize Phase-1 `lhs_base_var`: walk the LHS `args[0]` chain top-down from the base var, tracking each node's type (`var_type` → field-`position` lookup → `Type::content()` for elements); return `Some(name)` if the base var is value-const OR any dereferenced FIELD is value_const.  New fn, not yet called. | unit-probed on hand-built IR; suite green (inert) |
| 3 | **Wire into `validate_write` (load-bearing).**  Reject a component write when `lhs_frozen_through(to)` is `Some` — deep-freeze for `s.v.x=`, `s.v[i]=`, `s.v+=`, nested.  A rebind `s.v=` is the OUTERMOST node (not an inner deref) → not flagged → allowed. | Step-0 matrix green both backends; gate on the written matrix |
| 4 | **Prove the 91 builders UNTOUCHED.**  Binding-const compound fields (`const v: T`, NOT `: const T`) must keep `s.v+=`/`s.v[i]=` legal.  Re-run the lib corpus. | audience_crystal (10) / gridmesh / graphics / hex_world / routing_kernel green — zero new errors |
| 5 | **Docs + graduate.**  Full matrix into `tests/scripts/40-const-fields.loft` + `tests/issues.rs` negatives; LOFT.md § Fields + loft-write carry the 4-quadrant table with `const v: const T` = fully frozen. | `make ci` |

**Over-unification guard (design-protocol §4).**  The cleanest claim — *"one chain-walk
handles variable AND field value-const uniformly"* — is probed at Step 3 by the exact
rebind-vs-mutation cell: `s.v = other` (rebind, must stay ✓) vs `s.v.x = y` (mutation,
must ✗).  Both share the base `s`; the walk distinguishes them because rebind targets the
OUTERMOST node while mutation dereferences THROUGH the value-const field (an inner step).
If Step 0 shows the walk flags the rebind, the invariant is wrong — falsified before Step
3 lands.

## Phase 3 (deferred, post-1.0, own plan) — type-carried const

The remaining transitivity Phase 2 CORE does **not** give (because the const bit lives on
`Attribute`, not in the value's `Type`):

- **Field-read laundering:** `local = s.v` (v value-const) — the local isn't const, so
  `local[i]=x` slips.  Needs the const bit ON the read's result type.
- **Return no-laundering:** `fn f(p: const T) -> …borrow…` returns a non-const view.
- **Generics:** `vector<const T>` — frozen elements of a mutable vector.
- **Store round-trip:** a const value persisted + re-read.

These all need `const` threaded through `Type`.  Representation candidate (to probe in
Phase 3, NOT now): a `const` bit riding the **`Deps`** that heap `Type` variants already
carry (`Text/Enum/Reference/Vector/keyed/Function` all hold `Deps`; scalars collapse and
need none) — one field on one struct with ~5 constructor updates, vs 1576 for a wrapper.
Whether `Deps` is the right home (it tracks ownership, an orthogonal axis) is the
Phase-3 design's first probe.  **Not weekend-critical:** the analysis shows no library
launders a value-const field today (the compound const fields are builders, not frozen
records that get read-then-mutated).

## Sequencing for the 1.0 deadline

Steps 0–5 (Phase 2 CORE) are the 1.0 deliverable — opt-in deep-frozen records, enforced
for direct writes, additive, builders untouched.  Phase 3 (type-carried) is post-1.0.
The const model is then COMPLETE for direct access on all four quadrants; only
cross-boundary laundering (a value-const value that ESCAPES via a local/return/generic)
waits for Phase 3 — and nothing ships needing it.
