<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — the coherent immutability model

**Status: design (greenfield).**  A clean-slate spec for loft's immutability,
designed for coherence first — *ignoring* what is currently built (the shipped
@PLN40 field-const, and the ad-hoc const-param/local behaviour, all become one
quadrant of this model; the libs get re-annotated to match).  No new keyword.

## First principle — two orthogonal facts, and loft already has one of them

Immutability is really **two independent questions**:

1. **Can the slot be re-pointed?**  (`x = otherValue`) — a property of the
   *binding* (the variable / field slot).
2. **Can the value be mutated through this name?**  (`x.f = …`, `x[i] = …`,
   `x += …`) — a property of the *access* to the value.

These map cleanly onto machinery loft **already** has:

- Question 2 is exactly the **borrow mode**.  loft already has `&T` = a *mutable*
  borrow (write-through).  Its missing counterpart is the *immutable* borrow — a
  read-only view.  **That is what value-`const` is** (`&T` : `const T` :: Rust
  `&mut T` : `&T`).
- Question 1 is the **binding mode** — `let` vs `let mut` in Rust.  loft's default
  binding is mutable; binding-`const` is the immutable binding.

So the model is not a bolt-on: it completes the borrow system (adds the read-only
borrow) and adds an immutable binding.  One keyword, positioned to say *which fact*:

## The model

- **`const` before the name → binding-const.**  The slot never re-points.
  `const x = …` (local), `const v: T` (field), `const p: T` (param).
  Rejects `x = …`.  Says nothing about the value.
- **`const` before the type → value-const (a read-only borrow of the value).**
  `x: const T`, `v: const T`, `p: const T`.  The value cannot be mutated through
  this name: no `x.f = …`, no `x[i] = …`, no `x += …`.  **Transitive** — reading a
  field/element of a `const` value yields a `const` view, so immutability can't be
  laundered by extracting a part.
- **Both compose:** `const v: const T` — a slot that never re-points, holding a
  value that can't be mutated = fully immutable.

| declaration | `x = …` (rebind) | `x.f=` / `x[i]=` / `x += …` (mutate) |
|---|---|---|
| `x: T` (default) | ✓ | ✓ |
| `const x: T` (binding) | ✗ | ✓ |
| `x: const T` (value) | ✓ | ✗ |
| `const x: const T` (both) | ✗ | ✗ |

### Coherence rules (what makes it total, not ad-hoc)

1. **Scalars collapse.**  A scalar (`integer`, `float`, `boolean`, `character`,
   `text` when used by value) has no "contents" distinct from its binding, so
   binding-const and value-const coincide — `const x: integer` is the whole story;
   `x: const integer` is accepted as the same thing (no separate meaning).
2. **`const` is transitive on a value.**  `const S` on a struct freezes every field
   recursively; `const vector<T>` freezes the vector *and* makes each element
   `const T`.  A read (`c.inner.x`, `v[i]`) is always allowed and yields a `const`
   view; a write anywhere under a `const` is rejected.
3. **It composes into generics.**  `vector<const Point>` = a *mutable* vector whose
   *elements* are frozen (you can append/replace elements, but not mutate a Point in
   place); `const vector<Point>` = a frozen vector of (transitively frozen) Points.
   This nested control is the payoff of putting value-const in the type.
4. **`const T` is the immutable borrow.**  As a parameter it is the read-only
   counterpart to `&T`: `f(p: const T)` promises not to mutate the caller's value;
   `f(p: &T)` may.  A `const` value may be passed to a `const` param but not a `&`
   param (can't hand a read-only value to something that will mutate it).
5. **No laundering via return.**  A function returning a borrow of a `const`
   parameter/field returns a `const` borrow.

## Where the four quadrants are used (guidance)

- **`const x: const T` (fully immutable)** — value / record types: `Pixel`, `Vec3`,
  `Rect`, `GeoPoint`, `Event`, `HttpResponse`, DB rows.  "It is a value; freeze it."
- **`const v: T` (binding-const)** — builder/accumulator fields grown then read:
  `Mesh.verts`, `Scene.meshes`, `Args.options`, `World.chunks`, `CellSnap.*`.  The
  slot is fixed; the container still grows via `+=`/element writes.
- **`p: const T` (value-const)** — read-only parameters (the shared borrow): the
  `const Image` / `const Region` accessor style, made honest (truly no mutation).
- **`p: &T` (unchanged)** — the mutable borrow, for write-through params.

## Implementation — safe small steps

The two axes are enforced at two chokepoints loft already has.

| # | Step | Verify (both backends) |
|---|---|---|
| 0 | **Matrix probe** (throwaway): the 4×2 table above × {local, field, param} × {scalar, struct, vector, hash, text, nested} — hand-computed target verdicts.  This is the spec. | records the target |
| 1 | **Binding-const, unified.**  One flag on the slot (field `const_field`, var `const_binding`); reject only a *rebind* of that slot at the write chokepoints (`validate_write` for fields; the `is_const_param`/local guards for vars).  Allow all contents mutation (element, append, nested). | binding row identical for local/field/param |
| 2 | **Value-const carried by the type.**  A `Type::Const(Box<Type>)` wrapper (or a `const` bit on `Type`) set when `const` precedes a type; parse it in field/param/local/generic type positions. | `x: const T` parses; type round-trips |
| 3 | **Enforce value-const at read/write lowering.**  A write whose target has a `const` value-type is rejected (element, append, field, nested).  A field/element READ of a `const` value yields a `const` view (transitivity + no-laundering). | value row + transitivity, both backends |
| 4 | **Borrow interaction.**  `const T` accepted where `&T` is not for mutation; a `const` value can't bind to a `&` param.  Scalars collapse (rule 1). | param matrix; the `&`-vs-const cases |
| 5 | **Docs + graduate** the full matrix into `tests/scripts/` + `tests/issues.rs` negatives; LOFT.md/loft-write carry the 4-quadrant table + the borrow analogy. | `make ci` |

## The library-scoped const-suggest lint

Suggests the RIGHT quadrant, reusing the two write chokepoints as the oracle:
- **binding-const** (`const` prefix) on a field never *rebound*;
- **value-const** (`: const T`) on a field/param never *mutated at all* (only read /
  passed to `const` params) — the value/record types;
- library-scoped (fires on library packages, quiet on app code); advisory (a `pub`
  field's or an exported fn's writers may be unseen — author judges).

## Re-annotating the libs

"We update the libs ourselves" — the classification from the uptake survey drives it:
- upgrade the pure value/record types to `const x: const T` (Vec/Mat/Pixel/Rect,
  Cbor/Hpke/GameEnvelope/Event, MonsterDef/Way/GEdge/SubPath/TerrainSample, the
  `value struct` DateTime/Duration);
- keep the builders as `const v: T` (Mesh/Scene/Args/World/PTile/EdgeCosts/CellSnap…);
- turn the read-only accessor params (`const Image`/`Region`/`Hydro`) into honest
  value-const `p: const T`.

## See also

- [README.md](README.md) — @PLN40 field-const, now the `const v: T` quadrant
- [const-suggest-lint.md](const-suggest-lint.md) — the lint (subsumed above)
- `src/parser/expressions.rs:3453` (`validate_write`) + `parser/operators.rs:20` /
  `collections.rs:794` (`is_const_param`) — the binding chokepoints
- THREADING.md / the `&`-borrow model — value-const is its read-only sibling
