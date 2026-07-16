<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — two-level const: binding-const (`final`) vs value-const (deep)

**Status: design, not started.**  Spun off from @PLN40 + the const uptake.  Should
become its own plan / `loft-lang/features` issue.  Goal: let a programmer say either
"this slot can't be rebound, but its contents are mutable" (Java `final`) OR "this
value is fully frozen" (C++/Rust `const`) — **without a new keyword**, and unify the
three const forms that currently disagree.

## Verified current state (2026-07-16)

`const` today is inconsistent across the three positions, and none of them is deep:

| form | `x = …` rebind | `x += …` append | `x[i] = …` element | `x.f = …` field/nested |
|---|---|---|---|---|
| **field** `const v: T` (prefix) | reject | **allow** | allow | allow |
| **param** `p: const T` (type-qual) | reject | reject | allow | allow |
| **local** `const x` (prefix) | reject | reject | allow | allow |

Three problems: (1) field vs param/local **disagree on `+=`** (a regression the
append change introduced); (2) **no deep level** — every form allows `x.f=`/`x[i]=`;
(3) fields put `const` **before the name**, params put it **on the type** — an
accidental split.

## The model — two orthogonal axes, one keyword, two positions

Make position #3 meaningful instead of accidental:

- **`const` PREFIX on the slot → binding-const ("final").**  The slot is write-once;
  the *value it holds* is mutable in place.  Rejects only a rebind (`x = …`).  Allows
  element write, **append (`+=`)**, and nested/field mutation.
  Uniform: `const v: T` (field), `const p: T` (param), `const x` (local).
- **`const` on the TYPE (`: const T`) → value-const (deep).**  The *value* is frozen:
  no rebind, no append, no element write, no `x.f=`.  Reads only.  Transitive (a
  `const struct` freezes its nested fields).
  Uniform: `v: const T` (field), `p: const T` (param), `x: const T` (local).
- **They compose:** `const v: const T` = write-once slot holding a frozen value.

| | `x = …` | `x += …` | `x[i] = …` | `x.f = …` |
|---|---|---|---|---|
| **binding-const** (`const x: T`) | reject | allow | allow | allow |
| **value-const** (`x: const T`) | *allow* | reject | reject | reject |
| **both** (`const x: const T`) | reject | reject | reject | reject |

This is the C/C++ pointer-const model (`T* const` vs `const T*` vs `const T* const`),
which loft's reference-like fields map onto directly — principled, no new keyword.

## What has to change (and the migration)

1. **Unify binding-const to allow `+=` everywhere.**  const-param and const-local
   currently reject `+=`; make them allow it (contents mutation), matching const-field.
   Chokepoints: the `is_const_param` / const-local guards (`parser/operators.rs:20`,
   `collections.rs:794/865`, `expressions.rs`) — they must let a compound op on a
   collection/text slot through, exactly like `validate_write` now does for fields.
2. **Redefine `p: const T` (type-qual) from today's shallow-mixed to value-const (deep).**
   Today it rejects rebind+append but ALLOWS `p.x=`/`p[i]=` — an incoherent middle.
   Deep = reject all mutation.  **Migration:** a const-param that only READS is already
   deep-compatible (the survey found `const Image`/`const Region`/`const Hydro` are
   read-only accessors — low blast radius); a const-param that mutates its contents must
   switch to the new `const p: T` (binding-const prefix) or drop const.  The lint (below)
   finds them; a one-time scan (`grep '\bconst [A-Z]'` param sites, then check for
   `.field=`/`[i]=` on that param) sizes it before step 1.
3. **Make `const p: T` (prefix) parse for params** — today only `p: const T` parses;
   the prefix form is the binding-const param.  (`parser/definitions.rs` param loop, the
   same shape as the struct-field prefix already added for @PLN40.)

## Safe small steps

| # | Step | Verify |
|---|---|---|
| 0 | **Matrix probe** (throwaway): the 3×4 table above × field/param/local × both backends, hand-computed.  Snapshot the CURRENT verdicts (the table in "current state"). | records what moves |
| 1 | **Unify binding-const append.**  Let a compound `+=` on a collection/text const-param/const-local through (mirror the field fix). | binding-const row identical for field/param/local, both backends |
| 2 | **Scan + size the const-param migration** (step-2 above): list every `p: const T` that mutates the param's contents. | a concrete migration list (expected: small) |
| 3 | **Parse `const p: T`** (binding-const param prefix). | `const p: T` parses; is binding-const |
| 4 | **Implement value-const (deep) for `: const T`** — reject ALL mutation (rebind/append/element/field), transitively.  Applies to field/param/local type positions. | deep row of the matrix, both backends; migrate the list from step 2 |
| 5 | **Docs + graduate** the matrix into `tests/scripts/` + `tests/issues.rs` negatives; update LOFT.md/loft-write with the two-axis table. | `make ci` |

Each step is contained (binding-const append in step 1 can only turn errors into
successes; value-const in step 4 is opt-in per field/param).  No env-gate needed.

## The library-scoped const-suggest lint (folds in [const-suggest-lint.md](const-suggest-lint.md))

With two levels, the lint suggests the RIGHT one:
- **Suggest binding-const** (`const` prefix) on a field never *rebound* (`x = …`) — it
  may still be appended/element-written.
- **Suggest value-const** (`: const T`) on a field never mutated AT ALL (only read /
  passed to readers) — the pure value/record types.
- Library-scoped (fires on library packages), advisory (a `pub` field's writers may be
  unseen — author judges), reuses `validate_write` as the write oracle.

## Places in the libs to change

From the uptake survey, classify the already-const'd fields into the two levels:

**value-const (deep — pure value/record types, only ever read):**
`Vec2/3/4`, `Mat4`, `Vertex`, `Triangle`, `Rect`, `Circle`, `Pixel`, `GeoPoint`,
`Coord`, `BBox` (graphics/routing); `CborEntry`, `Decoded`, `HpkeSealed`,
`GameEnvelope`, `WsMessage`, `HttpResponse`, `Request`, `Event` (core/net);
`MonsterDef`, `ItemDef`, `Way`, `GEdge`, `SubPath`, `TerrainType`, `TerrainSample`
(crawler/routing/world); `DateTime`/`Duration` (already `value struct` — deep is exact).
These want the whole struct frozen — the strongest guarantee and the least surprising
for these "it's a value" types.

**binding-const (final — builders/accumulators that need contents mutation):**
`Mesh.verts`, `Scene.meshes`/`materials`/`nodes`, `Renderer.vaos`, `ChunkField.xs…`,
`GroupAcc.chunk_cks` (graphics); `Args.options`/`results`/`positionals` (core);
`World.chunks` (world); `PTile.*`, `EdgeCosts.*`, `TTile.roads/steps` (routing);
`CellSnap.*`, `CrystalMesh.*`, `CrystalState.*` (audience_crystal).  These grow via
`+=`/element writes, so they MUST stay binding-const (deep would reject the builder).

The uptake so far made everything binding-const (that is all that existed).  Step 5
of this design revisits the value-type list and upgrades them to `: const T`.

## See also

- [README.md](README.md) — @PLN40 const fields (binding-const, shipped)
- [const-suggest-lint.md](const-suggest-lint.md) — the lint (subsumed here)
- `src/parser/expressions.rs:3453` (`validate_write`, field const), `parser/operators.rs:20`
  + `collections.rs:794` (`is_const_param`, param/local const) — the chokepoints
- the `libs-maximize-const-fields` owner-policy memory
