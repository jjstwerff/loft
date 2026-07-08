<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN101 — Code inspection (mechanisms behind the steps)

Detailed, code-grounded documentation of every mechanism [STEPS.md](STEPS.md) touches.
All refs read on the tree **2026-07-08**; re-grep before editing.

## Headline: value struct = a struct record stored INLINE, same layout

`Stores::finish_type` (`src/database/types.rs:317`) **already computes every struct's full
inline byte layout** — `self.types[t_nr].size` (total record bytes), `.align`, and each
field's `position` (byte offset) via `calc::calculate_positions` /
`calculate_positions_with_groups` (`types.rs:432-445`, the `fields[field_nr].position = *pos`
loop). Reference structs live in a heap `Store` record and are referenced by a 12-byte
`DbRef`; **a value struct is the SAME record bytes stored inline** (in the parent slot,
record, or vector element). The layout is byte-identical — this plan changes *where the bytes
live*, not *how they're laid out*. That is the single-home invariant: `finish_type` /
`calculate_positions_with_groups` is the ONE layout home; value + reference structs both read
it. This is why the effort is tractable, not a heap-model rewrite.

---

## §A — Declaration: parsing `value struct` + marking the def

- **`parse_struct`** — `src/parser/definitions.rs:2235`. Guards on `self.lexer.has_token("struct")`
  (`:2236`); creates the def via `self.data.add_def(&id, pos, DefType::Struct)` (`:2258`) and
  sets `returned = Type::Reference(d_nr, Deps::none())` (`:2259`).
- **`DefType`** — `src/data.rs:2332` (variants: `Struct`, `Enum`, `EnumValue`, `Generic`,
  `Interface`, …). No value kind today.
- **Change:** parse an optional `value` keyword *before* `struct` (a `has_token("value")` at
  the `parse_struct` call site or at its head), and record it. **Recommended:** a `bool
  is_value` on `Definition` (read only at the ~6 chokepoints below) rather than a new
  `DefType` variant (which forces a decision at every `DefType::Struct` match — e.g.
  `data.rs:3674`, plus each `def_type == Struct` site). Keep `DefType::Struct` so all field /
  method / @PLN99-dispatch machinery applies unchanged. `returned` stays `Type::Reference`
  OR becomes a new `Type::Value` (see §B).

## §B — Representation + inline size (the crux; STEPS 1.2)

Two type worlds, both real:
- **Parser `Type`** (`src/data.rs:1344`, `enum Type`) — the compile-time IR type.
  `Type::Reference(def_nr, deps)` is a struct value.
- **Database type** (`self.types[t_nr]` in `Stores`) — the runtime record layout, carrying
  `.size` / `.align` / per-field `.position`, filled by `finish_type`.

The size chokepoints take **`&Type` alone** and return `size_of::<DbRef>()` (12) for a
struct:
- `variables::size(tp, ctx)` — `src/variables/mod.rs:1895`; `Type::Reference | … => size_of::<DbRef>()` (`:1910`).
- `variables::align(tp)` — `src/variables/mod.rs:1936`; Reference → 4 (`:1951`).
- `data::element_size(t)` — `src/data.rs:1928`; Reference → `size_of::<DbRef>()` (`:1948`).
- `data::element_align(t)` — `src/data.rs:1887`.

**To report the packed inline size instead**, the layout code must recognise a value struct
from the type alone. Introduce **`Type::Value(def_nr, deps)`** (parallel to `Reference`).
Its fields live on the DEF (unlike `Type::Tuple(elems)` which carries element types inline),
so its size cannot be computed from the variant — but it is ALREADY computed and cached as
`self.types[t_nr].size` by `finish_type`. **Open Q1:** make that size readable from the
`&Type` layout fns by either (a) embedding it — `Type::Value(def, size)` — set when the type
is minted post-finish, or (b) caching `inline_size: u16` back onto the parser `Definition`
after `finish_type` and giving the layout fns a `Data` handle. Lean (a): keeps
`variables::size(&Type)` / `element_size(&Type)` signatures intact (no `Data`/`Stores`
threading through their many callers).

**Blast radius of a new `Type` variant:** every exhaustive `match` on `Type` gains a `Value`
arm — chiefly `Type::name` (`data.rs:1677`), `Type::is_equal` (`:1645`), `Type::depend`
(`:1560`), `Type::for_each_child` (`:1385`), `size`/`align`/`element_*`, and the parser's
type-dispatch. Most arms mirror `Reference`; the layout arms differ (inline size, no free).

## §C — Construction writes inline, no `new_record` (STEP 1.3)

- Reference construction allocates a heap record: **`new_record`** — `src/fill.rs:1907`
  (`s.new_record()`), backed by `Stores.allocations: Vec<Store>` (`src/database/mod.rs:213`);
  a new store bumps `allocations.len()` (`mod.rs:627`). Field writes then go through the
  DbRef.
- Tuples build **inline** — the tuple literal packs elements into its slot; element write is
  IR `Value::TuplePut(var_nr, idx, val)` (`src/data.rs:533`).
- **Change:** when `is_value`, lower `V { … }` to the inline-block construction (the tuple
  path) — write each field at its `position` into the destination slot; emit NO `new_record`
  and NO `DbRef`.

## §D — Field access: drop the inline DbRef (STEP 1.4)

- **`field_ref`** (`src/database/structures.rs:193`) is the key mechanism. For a struct field
  it returns a **fat pointer into the SAME store/rec at an offset**:
  `DbRef { store_nr: data.store_nr, rec: data.rec, pos: data.pos + fields[field].position }`.
  So a nested struct's bytes are ALREADY inline in the parent's store (this is why
  `vector<Outer>` with nested `Inner` fields allocates one record per element, not three —
  §Slice-0 findings). The 12-byte DbRef is a pointer INTO the same store — pure indirection
  overhead for a value field.
- Reference field access: **`get_record`** (`src/fill.rs:1942`) derefs the DbRef, then
  **`get_field`** (`src/fill.rs:1400`) reads at the field `position`; emitted as `OpGetRecord`
  (`src/parser/fields.rs:1250`). Tuple element access: IR `Value::TupleGet(var_nr, idx)`
  (`src/data.rs:531`) reads inline at the element offset.
- **Change (LOCKED, user 2026-07-08):** for a value-struct receiver, **get rid of the inline
  DbRef** — read/write **directly at `base + offset`** (the cached field `position`), no
  `field_ref` fat pointer, no `OpGetRecord` deref. `event.when.ms` becomes `parent_base +
  when_offset + ms_offset` — offsets only.

## Locked design decisions (user, 2026-07-08)

- **Part of ONE Store.** A value struct's fields are packed contiguously inside the parent's
  single Store allocation — never a separate record/store. The allocator already supports a
  sub-record living inside a parent allocation (`field_ref` proves the same-store offset
  addressing); value structs always take that path. → §D drops the DbRef; §B sizes it inline.
- **Non-null by initialisation.** No null representation. The compiler forbids an
  uninitialised value struct (require a value or a declared default). "Inside another record
  there is no null" — an inline value field is always present. Closes Q4.

## §E — Value semantics: copy on assign (STEP 1.5)

- Tuples are value types: assignment / by-value arg / return **copies the packed bytes**
  (memcpy of the block), not a shared DbRef. Value structs inherit this — the observable
  difference from reference structs (`b = a; b.x = 9; a.x` unchanged).
- `&`-ref params (in-place mutation) — **Open Q2**: confirm what tuples support today (a `&`
  tuple param path exists in `variables/`), and mirror it; otherwise value structs pass by
  copy only at first (state the limitation).

## §F — No lifetime / no free (STEP 1.6)

- **`has_lifetime_concern(t)`** — `src/data.rs:1864`. True for `Text | Reference | Vector |
  Enum-struct | Sorted/Hash/Index/Spacial | RefVar`, and for a `Tuple` iff any element is
  (`:1876`). Callers that route the free / ownership-transfer machinery:
  `parser/definitions.rs:1039-1043` (tuple-return re-route through `__tuple<…>`) and
  `parser/mod.rs:2986`.
- **Change:** add a `Type::Value` arm: `false` iff every field is non-lifetime-bearing
  (recurse over the def's fields) → no `OpFreeRef`, no store, no deps edges. A value struct
  with a `text`/`vector`/reference field is lifetime-bearing (Slice 4).
- **Open Q3 (ownership):** audit the free-emission + deps sites keyed on `Type::Reference`
  (`src/scopes.rs`, the @PLN85/@PLN90 deps analysis, `tests/ownership_oracle.rs`) for a
  needed `Type::Value` = non-heap case.

## §G — @PLN99 dispatch is representation-independent (STEP 1.7)

- Operators/format/conversions resolve via **`Data::find_op_method`** (`src/data.rs`, @PLN99)
  → `t_<len><Type>_Op…` by def name. Value structs keep `DefType::Struct`, so dispatch is
  unchanged; `to_text` / `OpConv…` likewise. Expected to work with zero changes — the
  verification is the flipped `515`.

## §H — Native backend (STEP 1.8)

- `--native` uses `LoftStore`/`LoftRef` for `Reference` / `Vector` / keyed-collection values
  (`src/generation/mod.rs:1336`, `:1476`). **Pure-value tuples already emit no `LoftStore`**
  (the Rust tuple ABI path — `data.rs:1859-1865`).
- **Change:** route `Type::Value` codegen through the inline (no-`LoftStore`) path — a Rust
  struct / tuple laid out to match the interp inline bytes. Emission sites:
  `src/generation/mod.rs` (arg/field/return marshalling) and `src/state/codegen.rs` (struct
  build/read). Verify byte-identical results to `--interpret` on `515`.

## §I — Alloc-count harness (STEP 0.1)

- Allocations = growth of `Stores.allocations: Vec<Store>` (`src/database/mod.rs:213`,
  `:627`). Leaks are read by `check_store_leaks` (`tests/leak.rs`, ~`:800`); the
  ownership_oracle harness (`tests/ownership_oracle.rs`) walks `tests/scripts`.
- **Add:** a probe/env mode reporting **total records allocated** for a run (peak/aggregate
  `allocations` growth), so a probe asserts `allocs == baseline`. Value-struct cells assert
  the count is unchanged vs the all-scalar baseline (zero heap records added).

## §Slice 2/3 — inline inside records + vectors

- **Records (Slice 2):** because a value-struct field's inline size is `types[t_nr].size`,
  `calculate_positions_with_groups` already lays a nested aggregate inline — the parent
  record's field `position`s account for the value struct's full bytes (no DbRef). Confirm
  `finish_type` recurses value-struct fields as inline (their `size`), not as a 12-byte
  DbRef; that is the one edit for "records too".
- **Vectors (Slice 3):** vector element stride = element `size`. A `vector<V>` uses the
  value struct's inline `size` as stride (like `vector<tuple>` / `vector<scalar>` inline
  elements), so N elements add zero records. Element storage: `src/store.rs` /
  `src/database` vector paths.

## Confirmed-vs-open summary

| Step | Confirmed mechanism | Open decision |
|---|---|---|
| 1.1 declare | `parse_struct` 2235; `DefType` 2332; `add_def` 2258 | Q5 keyword; flag vs variant |
| 1.2 layout | `finish_type` 317 computes `size`/`align`/`position`; size fns 1895/1928 | Q1 size access (embed vs cache) |
| 1.3 construct | `new_record` 1907; `TuplePut` 533; `allocations` 213/627 | — |
| 1.4 access | `get_record` 1942, `get_field` 1400; `TupleGet` 531 | — |
| 1.5 copy | tuple value-copy semantics | Q2 `&`-ref |
| 1.6 lifetime | `has_lifetime_concern` 1864 + callers 1039/2986 | Q3 ownership sites |
| 1.7 dispatch | `find_op_method` (@PLN99) — no change expected | — |
| 1.8 native | `LoftStore` sites `generation/mod.rs`; pure-value tuple path 1859 | — |
| null | — | Q4 inline `value struct?` sentinel (@PLN25) |
