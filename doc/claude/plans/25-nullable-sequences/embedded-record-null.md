<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Embedded-record null — the nullable-enum representation

> **Part of** [@PLN25](README.md) (nullable sequences). **Status:** design (not yet
> built). **Resolves:** finding 12's open `reference`/struct element case, finding 9
> (nullable struct field defaults), the `field = nullable_source()` crash, **and the
> same crash for nullable enums** (a pre-existing bug — see § The unification). **Method:**
> written as a hypothesis under `design-protocol`; § Probes lists the falsification tests
> to run before code. An earlier draft proposed an out-of-band validity bitmap; § Rejected
> alternative records why the in-band discriminant wins.

## The problem (grounded)

A struct stored **inline** — a `vector<Row>` element (`8 + i*size`) or an embedded field
(`Box { item: Row }`) — has no encoding for "absent". Every byte pattern is a valid value
(`Row{id:0, tag:null}` ≠ "no Row"), so a null `Row` value (which a *variable* can hold via
the `u16::MAX` DbRef sentinel) cannot be *stored* into an inline slot: `OpCopyRecord` derefs
the null source's store and OOB-crashes (`allocation.rs:560`, index `65535`; native
`OpCopyRecord(cell, ())`).

Storing the struct **by-reference** (a `rec==0`-nullable handle + a separate record per
element) adds an indirection per embedded record — a pointer-chase and an allocation on
loft's hottest data path. **That breaks the flat/inline memory model, so it is rejected.**

## The unification (what probing revealed)

The crash is **not** struct-specific. Probed both scalar cases:

| value | storage | `x = mb(false)` (null source) |
|---|---|---|
| struct **variable** `m: Row` | a `DbRef` slot | ✅ holds null (`m == null` true) |
| **enum variable** `x: Shape` | **inline value record** | 💥 `allocation.rs:560` OOB (index 65535) |
| struct **field / vector element** | **inline value record** | 💥 same crash |

So the real family is **inline value records** — enums *today*, plus embedded structs and
vector elements. A struct *variable* escapes only because it is a reference (`DbRef`) slot.
Nullable enums (`Type::Enum(_, /*nullable*/ true, _)`) are an *intended* feature that is
**currently broken by this exact crash.** One representation + one fix covers all three.

And the encoding already exists: an enum's discriminant at offset 0 numbers variants from
**1** (`v = f_nr + 1` in `set_default_value`), leaving **`0` = no variant = null**. A
nullable inline record is null iff its discriminant is 0 — the layout loft already ships.

## The invariant (the one rule)

> A nullable inline value record is null **iff** its discriminant (offset 0) is `0`. A
> nullable struct field/element carries that discriminant and is therefore **byte-identical
> to a nullable enum** (`Null | Some(Row)`); `not null` carries no discriminant and is
> pure inline (today's bytes).

This is not a new encoding — it is loft's existing enum-value representation (discriminant
at offset 0, fields after it, `match` dispatch, `0`-default). A nullable `Row` ≈
`Option<Row>` ≈ a 2-variant enum, and reuses that machinery rather than inventing a bitmap.

## Why in-band discriminant beats an out-of-band bitmap (the slice/copy axis)

The discriminant lives **in** each element's bytes, so every data-movement op carries it for
free; a bitmap is out-of-band and must be moved/aligned separately:

| op | out-of-band bitmap | in-band discriminant |
|---|---|---|
| **slice `v[a..b]`** (loft materializes element-by-element) | extract bits `a..b`, **re-pack** to a new bit-aligned buffer | per-element copy carries the discriminant — nothing extra |
| **copy `v2 = v1`** | two memcpys + two allocations, kept in sync | one memcpy, discriminants included |
| **append / remove-shift** | grow/shift **two** regions in lockstep (silent-desync risk) | grow/shift one region |
| **space** | 1 bit / elem | +1–2 bytes / elem (the discriminant) |
| **machinery** | all-new (bitmap, bit-slice, two-buffer free) | **reuses enum** (layout, match, default, field offsets) |

The bitmap's only win is space (a bit vs a byte). Weighted by loft's element-by-element
slices and whole-vector copies — and by the machinery it would NOT have to build — the
in-band discriminant wins decisively. The space cost is opted out by `not null`.

## The two orthogonal `not null` axes

`not null` is the efficiency control on both axes; only the **element** axis is new.

| spelling | non-null thing | encoding |
|---|---|---|
| `v: vector<Row> not null` | the **container** (no absent-vector) | P1/P2 `u16::MAX` store sentinel |
| `vector<Row not null>` | the **elements** | **no discriminant** — pure inline |
| `Box { item: Row not null }` | the embedded **field** | no discriminant on `item` |
| `vector<Row not null> not null` | both | fully dense — **byte-identical to today** |

`vector<T not null>` is established: `vector<integer not null>` parses and reclaims the
scalar sentinel code today; the struct case is the structural analog (drop the
discriminant). The parser currently rejects `not null` after a *named* element type in a
generic (`Expect token >`) — wiring that is part of the work.

**Default = nullable** (matches loft's "nullable unless `not null`" model): a plain
`vector<Row>` / `item: Row` carries the discriminant (and so becomes enum-shaped). **`not
null` = zero overhead:** no discriminant, inline bytes only, `[i] = null` / `= null` is a
*compile error*. Dense, perf-critical data declares `not null` and is byte-identical to
today; only declared-nullable data pays the discriminant.

## The fix surface (small, because the layout is reused)

- **Represent** a nullable struct field/element with the discriminant-at-0 (enum) layout;
  fields sit after the discriminant, exactly as enum-value fields already do.
- **Assign null** (`slot = null`, or a runtime-null source `slot = mb(false)`): set the
  discriminant to `0` and free the slot's heap deps — **do not** call `OpCopyRecord` on the
  null source. This is what retires the `allocation.rs:560` / native crash, for enums and
  structs alike.
- **Detect null** (`== null`): read the discriminant.
- **Default value**: discriminant `0` (null) for a nullable slot; a zero-initialized inline
  record for `not null` (finding 9's resolution).
- **Slice / copy / append / remove**: unchanged — the discriminant is inline, so the
  existing byte-movement carries it.

## Probes — falsify before building (design-protocol step 3)

Each is the cheapest test that could prove a load-bearing claim FALSE. Expect to falsify.

1. **Byte-identity with a hand-written enum (the core claim).** *Claim:* a nullable
   embedded `Row` is byte-identical to an explicit `enum { Null, Some(Row) }`. *Probe:*
   `introspect` both — record layout, discriminant size/offset, field offsets, and the
   generated field-read code — and diff. A mismatch means "reuse the enum representation"
   is aspirational, not real (the protocol's over-unification guard, step 4).
2. **`not null` reproduces today's bytes.** *Claim:* `vector<Row not null>` /
   `item: Row not null` carry no discriminant and are byte-identical to today. *Probe:*
   dump the record bytes before/after; any diff means the discriminant leaked into the fast
   path.
3. **Null-assign no longer derefs the sentinel.** *Claim:* `slot = mb(false)` writes
   discriminant `0` instead of copying from store 65535. *Probe:* the exact crashing
   programs (`vr[i] = mb(false)`, `bx.item = mb(false)`, **and `enum_var = mb(false)`**) run
   clean on both backends; `== null` is true after.
4. **Slice/copy carry the discriminant.** *Claim:* `v[a..b]` and `v2 = v1` preserve which
   elements are null with no bitmap logic. *Probe:* a vector with a null in the middle,
   sliced and copied; assert each surviving element's null-ness rode along.
5. **Discriminant size / field-offset shift is handled on both backends.** *Claim:* fields
   after the discriminant read at the right offset in interp AND native. *Probe:*
   `vr[i].id` on a present element + `--native-emit` the field read; confirm the offset
   includes the discriminant.

## Implementation plan — verifiable steps

Each step states **Do** (the concrete change + site) and **Verify** (the gate that passes
only if the step is correct). A step does not start until the previous one's gate is green;
each phase ends with a full-suite gate on **both backends**. Every step begins by reading
its named site — the sites below are the entry points, not a claim that nothing else moves.

### Phase E1 — nullable enum VARIABLE [DONE — both backends green]
*Probing corrected the plan: an enum **variable** is a `DbRef` slot (like a struct
reference), so its null is the `store_nr==u16::MAX` SENTINEL — NOT a discriminant-0 record.
The discriminant-0 encoding is for INLINE enums (fields/elements), which is E2. E1 turned
out to be two distinct bugs, neither the assign:*

- **E1.0 Characterize.** The assign (`x = mb(false)`) does NOT crash — `x` holds the
  sentinel fine. The crash is in **`x == null`**: enum `==null` lowered to
  `OpEqInt(OpConvIntFromEnum(OpGetEnum(x,0)), …)`, and `OpGetEnum` derefs the absent
  record (`store_nr=65535`) → `allocation.rs:560` OOB.
- **E1.1 `== null` via the sentinel.** Added `OpRefIsNull(r) = store_nr==u16::MAX`
  (`default/01_code.loft`, regenerated into both backends) and an `enum_null` branch in the
  `==`/`!=` dispatch (`operators.rs`) that lowers `enum ⊗ null` to it. NOT `OpEqRef`: its
  `rec==0` test misreads a *present* enum on native (enums are inline-represented there, so
  `rec==0`), the same finding-1/4 distinction.
- **E1.2 Native return-ABI bug (deeper, pre-existing — surfaced by E1's native gate).** A
  nullable-returning fn's *present* enum value came back as the sentinel on native, so even
  `match` crashed. Root: the ref-retbuf **tail-capture** (`pre_eval.rs heap_shape_matches`)
  only matched `(Reference, Reference)` / `(Enum,true × Enum,true)`. The tail
  `if c { Circle{} } else { null }` infers to the **variant** type `Reference(Circle)`,
  while the target is `Enum(Shape,true)` — no match → capture missed → the present value was
  dropped and a sentinel returned. Fix: match a variant-ref/enum tail against its enum target
  via `def(variant).parent() == enum`. Nullable structs were unaffected (inline retbuf).
- **E1 gate met:** `enum == null` (null→true, present→false), present payload survives the
  nullable return, on BOTH backends; regression `enum_null()` in
  `tests/scripts/25-nullable-sequences.loft`; full suite green (2381).

(Original plan text, for E2/E3, kept below.)

- **E1 gate:** enum-var null round-trips on both backends; `find_problems` full suite green;
  the repro graduates to `tests/scripts/`.

### Phase E2 — nullable struct fields + vector elements

**E2.1 gate result (probed 2026-06-15): byte-identity is NOT free — chose approach (a).**
A plain struct and the hand-written enum differ: `Row{id,tag}` lays out `id@0, tag@8`
(size 12, no discriminant); the enum `RSome{id,tag}` reorders for packing — `disc@0,
tag@4, id@8`. So the only way to byte-match is to **represent the nullable struct
field/element AS a synthesized nullable enum** (approach (a)) and run it through the
existing enum layout/construct/access/copy machinery — not to bolt a discriminant onto the
struct layout. Staged below; each stage has its own gate and the entry point found in the
machinery survey.

- **E2a.1 — Synthesis helper [foundation].** *Do:* add a memoized
  `nullable_enum_for(struct_d) -> enum_d` that builds, once per struct, a synthetic enum
  `{ Null, Some<fields-of struct_d> }` — mirror `parse_enum` (`definitions.rs:367`,
  `add_def(DefType::Enum)`) + `parse_enum_values` (`:199`, `add_def(DefType::EnumValue)` ×2,
  `parent = enum_d`, the `Some` variant carrying `struct_d`'s attributes), then
  `mark_synthetic`. *Verify (Probe 1):* a test calls it and asserts the synthesized type's
  layout (discriminant@0 + packed fields) equals a hand-written `enum { Null, Some{…} }`.
- **E2a.2 — Wire nullable struct FIELDS to it.** *Do:* at the field-type/offset hook
  (`typedef.rs`, where `attr_nullable && !not_null` is known), when a field's content is a
  struct and the field is nullable, set the field's content type to `nullable_enum_for(struct_d)`.
  *Verify:* `introspect Box { item: Row }` shows `item`'s type is the synthetic enum, fields
  offset past the discriminant.
- **E2a.3 — Construction coercion.** *Do:* `Box { item: Row{…} }` builds the `Some` variant
  (struct-literal → `Some(...)`; discriminant = present). *Verify:* construct + read
  `bx.item.id` round-trips on both backends.
- **E2a.4 — Access / null / default.** *Do:* `bx.item.id` unwraps `Some` (fields at the
  post-discriminant offsets); `bx.item == null` → inline discriminant test (distinct from
  E1's sentinel — this value is inline); `bx.item = null` → `Null` variant; default → `Null`
  (finding 9). *Verify:* null/present round-trip, neighbour fields intact, both backends.
- **E2a.5 — Vector elements.** *Do:* `vector<Row>` elements take the synthetic enum; wire
  `vr[i]=null` / read / `== null` / iterate. *Verify:* Probe 3 (`vr[i]=null` clean both
  backends), Probe 4 (slice/copy preserve null-ness), Probe 5 (`vr[i].id` at the
  post-discriminant offset on native).
- **E2a.6 — `not null` opt-out.** *Do:* `Row not null` field/element skips synthesis (plain
  inline). *Verify:* Probe 2 — byte-identical to today.
- **E2a.7 — native parity + full suite** on both backends; regression cells.

*Risk note:* E2a.3/E2a.4 (struct-literal↔`Some` coercion, `.field` unwrap) are the load-
bearing unknowns — the syntax says `Row` but the type is the enum, so construct/access need
glue. If the existing enum machinery does NOT make these fall out cheaply, that is the alarm
to re-scope (the "reuse, don't build" premise would be weaker than the design assumed).

### Phase E3 — `not null` opt-out + dense fast path
- **E3.1 Parser.** *Do:* accept `not null` after a named element type in a generic
  (`vector<Row not null>`) and on embedded fields (`item: Row not null`). *Verify:*
  `vector<Row not null>` parses (today it errors `Expect token >`); `vr[i] = null` on it is a
  *compile error*.
- **E3.2 Skip the discriminant.** *Do:* a `not null` field/element carries no discriminant.
  *Verify:* **Probe 2** — byte-dump a `not null` vector/field before vs after the whole change:
  **byte-identical to today** (no discriminant leaked into the fast path).
- **E3 gate:** dense `not null` path is zero-overhead and byte-identical to today; full suite
  green.

**Interim crash-safety (until E1/E2 land):** a `not null`-only stopgap — reject `= null` on
inline struct fields/elements at compile time and raise a clean recoverable error on a
runtime-null source — keeps the `allocation.rs:560` OOB / native codegen crash from shipping
while the representation work proceeds.

## Open questions

- **Discriminant size** — enums use a byte (small) or short at offset 0; confirm via Probe
  1 which a nullable struct should take, and whether a 1-byte discriminant suffices for the
  2-state (null / present) case while staying compatible with multi-variant enums.
- **Default cost vs. consistency** — default-nullable adds a discriminant to existing
  `vector<Struct>` / embedded fields tree-wide. Consistent with the model and `not null`
  recovers the fast path, but it IS a layout change; confirm via Probe 2 + the corpus that
  cost is paid only where nullability is declared, and decide on a migration note.
- **`u16::MAX` (container null) vs. discriminant-0 (element/inline null)** stay distinct —
  both are this plan's H6 surface; keep them as the two agreed axes, not a third sentinel.

## Rejected alternative — out-of-band validity bitmap

A prior draft put a validity bit per slot in the container (a side record beside the vector
data, or a hidden bitmap word in the parent struct). Rejected because (a) loft's slices
materialize element-by-element and its vectors copy wholesale, so an out-of-band bitmap pays
bit-slice + two-buffer-sync cost on every slice/copy/append/remove — the in-band
discriminant pays none; and (b) it would be all-new machinery, where the discriminant
**reuses the enum representation loft already ships** and fixes nullable enums in the same
stroke. The bitmap's lone advantage (1 bit vs ~1 byte) does not pay for that.

## See also
- [README.md](README.md) findings 8–12 (the matrix that surfaced this), § The invariant.
- `src/database/structures.rs` `set_default_value` (enum discriminant at offset 0, `v =
  f_nr + 1` ⇒ `0` = null; the Vector + Struct arms gain the discriminant).
- `src/state/io.rs` `do_copy_record` / `copy_ref_or_null` (the null-source path E1 retires).
- `src/vector.rs` (the 32 `8 + i*size` element-base sites — the discriminant rides *inside*
  the element, so these stay untouched).
- `src/data.rs` `Type::Enum(_, true, _)` (the nullable-enum type this representation unifies
  with); `IntegerSpec::byte_width(nullable)` (the scalar-sentinel precedent).
