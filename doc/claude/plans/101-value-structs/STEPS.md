<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN101 — Detailed implementation steps

Reading order: [README.md](README.md) (design + status) → this file (verifiable steps) →
[INSPECTION.md](INSPECTION.md) (the code mechanism behind every step, confirmed 2026-07-08).
Each step states **what to change**, **where** (file/fn), and a **verification** that must
pass on **both backends** (`--interpret` + `--native`).

> **Headline from the inspection:** `Stores::finish_type` (`database/types.rs:317`) ALREADY
> computes every struct's inline byte layout (`types[t_nr].size`/`.align`/field `position`).
> A value struct is the SAME record bytes stored **inline** instead of behind a `DbRef` — an
> *inlining* change, not a layout redesign. Reference + value structs share the one layout
> home. Full mechanism map: INSPECTION.md.

> **Line-ref provenance:** refs marked **[confirmed 2026-07-08]** were read on today's tree;
> refs marked **[locate]** are the mechanism to find + confirm at implementation time. As in
> @PLN99, re-grep before editing — the tree moves.

> **The governing invariant (single home):** a value struct's *inline byte layout* has ONE
> home — the same `element_offsets` / `calculate_positions_with_groups` that lays out tuples
> and struct records today. Every path (size, construct, field r/w, copy, native codegen)
> reads that one layout; none re-derives it. This is the @PLN99/58 lesson applied to
> representation: single-home invariants make the off-diagonal matrix cells unable to disagree.

---

## Slice 0 — the composition matrix + the alloc-count harness (SPEC, before any code)

The matrix is the acceptance oracle; the alloc harness makes "zero-cost" a hard assertion.

- **0.1 — alloc-count harness.** The store tracker already counts allocations (the leak
  gate / "stores not freed" path — `LOFT_STORES` / the ownership-oracle harness **[locate:
  the store-allocation counter used by `tests/leak.rs` / ownership_oracle]**). Add a probe
  mode that reports **total store records allocated by a program run** (not just leaked), so
  a probe can assert `allocs == N`. Value-struct cells assert this count is **unchanged**
  vs. an all-scalar baseline (the struct added zero heap records).
  *Verify:* a reference-`struct` program reports `allocs > 0`; the same program with the
  struct flipped to `value struct` reports the baseline count.
- **0.2 — the matrix probes** (`/tmp`, `--interpret` first, then both). Vary ONE axis per
  probe; hand-compute value AND alloc-count AND leak:
  - **placement** × {local, temp, field-in-reference-struct, field-in-value-struct,
    `vector<V>` element, fn arg (by value), fn return}
  - **operation** × {construct, field read, field write, `<`/`==`/`-`, `{x}`/`{x:spec}`, `as`}
  - **field shape** × {all-scalar (pure-value), nested value struct, (Slice 4) text/vector}
  - **backend** × {interpret, native}
- **0.3 — freeze the spec.** Record the pass/fail + alloc-count table in this file; it is the
  Slice-1..5 acceptance oracle. Probes graduate to `tests/scripts/` as each slice lands.

### Slice 0 findings (2026-07-08, CORRECTED) — the real cost is the per-construction scratch store

Harness landed: two counters on `Stores` (`database/mod.rs`) — `records_created` (`record_new`
events, `structures.rs:60`) and **`stores_allocated`** (store-SLOT go-live events,
`allocation.rs`) — reported at exit via `LOFT_ALLOC_REPORT=1` (`state/mod.rs::check_store_leaks`)
→ `loft-alloc: stores=S records=R`.

**`record_new` MISLED (first write-up was wrong).** It counts logical records (0 for locals,
100 for vector appends), which hid the real cost. Report is now
`loft-alloc: peak=P allocs=A records=R` — **`peak`** = max LIVE stores (memory), **`allocs`** =
store alloc/free CYCLES (the per-construction abstraction cost). Measured (`--interpret`,
100× loops):

| shape | peak (memory) | allocs (cycles) | reading |
|---|---|---|---|
| scalar loop | 2 | 2 | baseline |
| **local `struct`**, 100× | **3** | **102** | **memory is FINE — the struct's store is REUSED each iteration (peak flat).** The cost is 100 per-construction **alloc/free cycles + DbRef indirection**, not memory (user: "an individual variable of T can be its own store… the same store will be reused") |
| `vector<P>` (flat), 100 elems | 4 | 4 | elements **inlined into the backing** (store #1); no per-element cycles |
| record field (`struct` in `struct`) | — | — | **already inline** (`finish_type` sizes a field by the nested record's full `size`, not a DbRef) |

**Goal = "zero abstraction structures" (user):** a `value struct` must have **NO** per-construction
alloc/free cycle **and NO** DbRef indirection — inline bytes wherever it lives. The per-slice
metric is **`allocs` delta → 0** vs the reference-struct baseline (peak is already reuse-bounded,
so it is NOT the target — cycles + indirection are), plus a structural "no scratch store, no DbRef".

**Corrected consequences (this REVISES the earlier note):**
1. The big win is **transient locals / temporaries / args / returns** — every reference-struct
   value you construct allocates a scratch store (100 loops → 100 stores). A `value struct`
   built inline allocates **none**. So **Slice 1 (value struct as a local) is the highest-value
   slice, not moot** — the earlier "locals are free" was a `record_new` artifact.
2. **Record fields are already inline** (storage-wise) — "zero cost inside records" is largely
   done for STORAGE; the residual value-struct win there is dropping the `field_ref` access
   DbRef + value/copy semantics.
3. **`vector<P>` already inlines P** into a single backing store (#1); it is NOT a single total
   allocation (4 slots: stack + backing + per-element construction scratch). The residual win
   is eliminating the per-element construction scratch (build P directly in the element slot).
4. **Metric for every slice:** `stores_allocated` DELTA vs the scalar baseline on a fixed
   program must collapse to ~0 for the value-struct version (plus a structural "no scratch
   store, no DbRef" check).

---

## Slice 1 — APPROACH: an ISOLATED copy pass (user, 2026-07-08) — NOT wired into the type system

**Decision (supersedes the `Type::Value` approach below).** Threading a distinct `Type::Value`
through the type system caused a broad blast radius (an `if let Type::Reference` sweep across
many runtime sites + an unbounded embedded-size refinement). Reverted (commit 1240f459). Value
structs **stay `Type::Reference`**, marked only by `Data.value_structs` (Step 1.1). Value (copy)
semantics is a **single self-contained IR pass** — no type-system wiring, no assignment-path
surgery, no ownership-oracle change.

**The pass — `value_struct_copy(data)` in `src/scopes.rs`, called from `check()` AFTER
`move_elide` and BEFORE the ownership scan (`scan_set`).** Mirrors `move_elide`'s shape (clone
each fn's `code`, rewrite, write back). For each `Set(v, rhs)` where `v`'s type is
`Type::Reference(P)` with `is_value_struct(P)` AND `rhs` is a VIEW
(`use_analysis::ownership_of(data, d_nr, rhs) == Own::Borrowed` — a field/element read), rewrite:

```
Set(v, rhs)  →  Insert([
    Set(v, Null),
    Call(OpDatabase,   [Var(v), Int(P.known_type)]),   // v gets its own fresh store
    Call(OpCopyRecord, [rhs,    Var(v), Int(P.known_type)]),  // deep-copy the view into v
])
```

Then **clear `v`'s view dep** (`variables.tp(v)` → `Type::Reference(P, Deps::none())`) so `v` is
Owned and freed at scope exit (no leak, no double-free). Because `v = OpDatabase(…)`, the
ownership scan that runs next classifies `v` `Owned` **correctly** (via `db_vars`) — sound, no
oracle change. Copy-elision is free: an already-`Owned` rhs (a fresh `P{…}`) isn't a view →
skipped. Method params (`self`/`both`) are untouched — a call arg is not a `Set(local, …)`, so it
stays a DbRef into the store (zero-copy hot path).

**Why this is less complex:** ~1 self-contained function + a recursive `Value` walk (mirror
`build_scope_parents`); reuses `OpDatabase`/`OpCopyRecord`/`ownership_of` (all exist); touches no
`Type` match, no size function, no `Type::Reference` site. **Verify (both backends + leak + UAF):**
`value struct P { x: integer }`, `b.items=[P{x:1}]`, `e = b.items[0]; e.x = 99` ⇒
`b.items[0].x == 1`; ownership_oracle clean; full suite green. Then flip `515`.

---

## Slice 1 — REFOCUSED (user, 2026-07-08): value structs as FIELDS + VECTOR ELEMENTS

**Scope cut:** do NOT eliminate the DbRef for a standalone local `value struct` variable — a
reused store + a pointer is negligible overhead; keep it simple. **The target is the bigger
stores:** these types are normally used **as parts of other records** and **directly inside
vectors**, and *those* cases must be **zero-overhead** (no per-element/per-field construction
scratch store, no DbRef indirection, no separate record, no free).

**Approach (user):** everything needed is already written — **link the existing inline
machinery** (tuples / inline record fields / vector inline elements), don't add a parallel
representation. Minimal new Rust; mostly routing value structs to the paths that already
store scalars/tuples inline. Avoid a new `Type::Value` variant if a flag + existing-path
routing does the job.

Driver: `tests/scripts/515` DateTime/Duration as `value struct`, PLUS `vector<DateTime>` and a
`struct` with a `DateTime` field — assert the `allocs`/scratch-store DELTA is 0 vs the scalar
baseline for the field/element cases, every assertion green, both backends.

### Step 1.1 — DONE (2026-07-08): parse + mark the kind
`value struct T { … }` parses (a `value` soft-keyword prefix, PEEKED as an identifier —
`has_token` only matches keyword lexemes) and marks T in `Data.value_structs` (a **set**, NOT a
`Definition` field — those serialize through `ir_schema`/`ir_read`; `Data::is_value_struct(d_nr)`
reads it). Verified: parses + a value struct as a vector element stores inline out-of-the-box
(`peak=4 records=5` for 5 elems, same as a normal struct), both backends; no regression
(issues 748, wrap 158, parse_errors 51). Commit `5f43c13b`.

### The plan: a distinct `Type::Value(def)` value type (copy semantics FALLS OUT)

**Why (proven).** A value struct currently ALIASES (`e = b.items[0]; e.x = 99` ⇒
`b.items[0].x == 99`) ONLY because it is `Type::Reference` — the borrow/dep/DbRef/free machinery.
TUPLES (loft's value type) already copy-on-bind on both backends (`e = b.t; e.0 = 99` ⇒
`b.t.0 == 1`). So making value structs a distinct **value type** makes copy-on-bind, deps = none,
and no-free FALL OUT of the same machinery — no surgery on the heap-#1 assignment path. **A value
struct = a named tuple**: `Type::Value(def)` reuses the tuple value-type machinery while keeping
the struct def for @PLN99 dispatch. (The parse-level `OpDatabase + OpCopyRecord` recipe is retired
to the appendix below as a fallback only.)

### Step 1.2 — add `Type::Value(u32, Deps)` + walk the compiler through the arms — DONE (commit 19e6ef33)
Added the variant to `enum Type` (`src/data.rs:1319`). Per the loft `Optional` idiom, every
exhaustive `match Type` became a compile error — **7 sites**, all mirroring `Reference` (no
semantic change yet, since `Value` is not minted until Step 1.4): `data.rs` `for_each_child`
(leaf), `name` + `type_name_str` (→ def name); `ir_node` `native_type_kind` (`K::Reference`);
`variables/validate` `short_type` (`val(t)`); `ir_schema` serialize+deserialize (distinct
`"Value"` tag, round-trips); `ir_store` (mirrors `Reference` — TODO Slice 1.8: distinct
`TY_VALUE` discriminant). Build clean; no regression (issues 748, wrap, IR round-trip 8/8 green);
the `value struct` probe still parses + works (as `Reference`). Fewer sites than feared — the
`Type` variant is well-contained.
- **Note on `is_equal`:** the leaf/name arms mirror `Reference`; the `is_equal` `(Reference,
  Reference) => r == o` arm does NOT yet cover `Value` (it falls to `self == other`, which
  distinguishes `Value` from `Reference` correctly). Revisit in 1.4 if a `Value`↔`Reference`
  cross-compare is needed at a coercion boundary.

### Step 1.3 — the arms that DIFFER — DONE (commit ba029ef7)
Q1 resolved: the slot allocator (`slots_v2`) has no `Data`, so the inline size is **embedded in
the variant** — `Type::Value(u32 def, u16 inline_size, Deps)`. `variables::size`/`element_size`
now return that size (inline slot, not a 12-byte DbRef — the inline slot is what makes copy fall
out); `variables::align`/`element_align` → 8. The other two "differing" arms needed NO change:
`has_lifetime_concern(Value)` = false (not in the true-list) and `depend(Value)` = empty
(`_ => {}`) are already correct. `ir_schema` carries the embedded size (round-trips). Dead code
until minting; build + suite green.

#### Step 1.3 (original text — for reference)
- **Size/align/inline layout:** `variables::size` (`variables/mod.rs:1895`), `variables::align`,
  `data::element_size` (`data.rs:1928`), `element_align` — a `Type::Value` returns the **packed
  inline record size** (`Stores::finish_type`'s `types[t_nr].size`, already computed), NOT
  `size_of::<DbRef>()`. **Open Q1** (embed the cached size in the variant vs. `Data` lookup).
- **Deps = none:** `Type::depend` (`data.rs:1560`) → empty for `Value` (a value type does not
  borrow), so no `["b"]` view dep is ever attached.
- **No free:** `has_lifetime_concern` (`data.rs:1864`) → `false` for a pure-value `Value` (recurse
  fields), so no `OpFreeRef`; the free-emission sites keyed on `Reference` skip `Value`.
- **Copy-on-bind:** inherited — the assignment machinery copies value types (proven on tuples),
  so `e = value_struct_view` copies with NO change to `parse_assign_op`/`generate_set`/the oracle.

### Step 1.4 — mint `Type::Value` at parse for a marked struct
- A `value struct`'s `returned` type becomes `Type::Value(d_nr, none)` instead of
  `Type::Reference` (in `parse_struct`, using the `Data.value_structs` marker from Step 1.1).
- Value-struct-typed FIELDS, `vector<V>` ELEMENTS, and value-struct LOCALS resolve to
  `Type::Value` wherever the type is looked up (the type resolver / `field_type` / element type).
- *Verify:* an introspect / dump shows a value-struct local + field typed `Value(P)`, sized inline.

### Step 1.5 — copy semantics: VERIFY, don't implement
- The behaviour change is now emergent. *Verify (both backends):* `value struct P { x: integer }`,
  `b.items = [P { x: 1 }]`, `e = b.items[0]; e.x = 99` ⇒ **`b.items[0].x == 1`** (copy). Method
  params still zero-copy: `dt.to_text()` / `a < b` pass `self`/`both` by DbRef into the store
  (no copy) — confirm the value type does not force a copy at the CALL boundary (per the user's
  "copy only on local bind"). `allocs` delta ~0 (inline copy, no scratch store).

### Step 1.6 — @PLN99 dispatch keeps working
- `find_op_method` (`data.rs`, @PLN99) resolves by `type_def_nr(tp)` — add a `Type::Value` arm so
  operators/format/conversions resolve `t_<len><Type>_Op…` on the def. `to_text` / `OpConv…`
  likewise. *Verify:* flip `515` DateTime/Duration to `value struct` — every assertion green.

### Step 1.7 — non-null init (Q4, locked)
- Reject an UNINITIALISED value struct at compile time (require a value or a declared default) — a
  value struct has no null. Enforce where a value-struct local/field is declared without an init.

### Step 1.8 — native ABI + the mandatory validation matrix
- Native rides the pure-value tuple ABI (`data.rs:1859-1865`, no `LoftStore`) — route `Type::Value`
  codegen there (`generation/mod.rs`, `state/codegen.rs`). **Validation bar (heap invariant #1):**
  full suite + leak (`ownership_oracle`) + UAF (ASan), BOTH backends, green before landing; flip
  `515` + add `vector<DateTime>` / a `DateTime`-field struct with `allocs`-delta-0 assertions.

**Open sub-questions.** (a) Q1 size access (embed vs `Data` lookup). (b) `self`/`both` params of
`Type::Value` must stay DbRef-into-store at the call boundary (no copy) — verify the value type
does not over-copy method receivers. (c) A `Type::Value` field of a `Type::Reference` struct
(Slice 2) — inline embedding via `finish_type` (already inline).

### Appendix — retired approaches (MECHANISM REFERENCE only; SUPERSEDED by Steps 1.2–1.8 above)
Two earlier framings, kept for the code refs they pin (not the plan):
- the **parse-level `OpDatabase + OpCopyRecord` surgery** for copy semantics — retired in favour
  of the `Type::Value` value type (copy falls out; no heap-#1 surgery);
- the **`is_value`-flag-on-`Type::Reference`** representation with per-site inline routing —
  superseded by the distinct `Type::Value` variant. The `Data.value_structs` marker survives, but
  only to decide `Type::Value` vs `Type::Reference` at parse.
The construct/field/access mechanism notes below remain accurate as REFERENCE; a value-struct
LOCAL now becomes inline `Type::Value` (not a kept DbRef) as a natural consequence of the value
type — which is fine (a bonus, not extra work).

### Step 1.1 — declaration: the `value struct` kind
- **Parse** the `value` modifier before `struct` in the definition parser
  **[locate: `src/parser/definitions.rs` struct-definition entry — near the `add_fn`/context
  block at ~741, and the struct/enum/typedef dispatch in `parser/definitions.rs`]**.
- **Mark the def.** `DefType` **[confirmed 2026-07-08: `src/data.rs:2332`]** has `Struct`
  but no value kind. Add either `DefType::ValueStruct` **or** a `bool is_value` on
  `Definition`. **Recommendation:** a `bool is_value` flag on `Definition` — a new `DefType`
  variant forces a decision at every `DefType::Struct` match site (there are many:
  `data.rs:3674`, `def_type` dispatch), whereas a flag is read only at the ~6 chokepoints
  below. Keep `DefType::Struct` so all existing struct machinery (fields, methods, @PLN99
  dispatch) applies unchanged.
- *Verify:* `value struct V { a: integer }` parses; `data.def(v).is_value == true`; a plain
  `struct` is `false`; introspect shows the field layout.

### Step 1.2 — representation + inline layout (THE crux)
- **Type variant.** A layout function takes `&Type` alone (`variables::size(tp)`,
  `data::element_size(t)` — **[confirmed 2026-07-08]** both return `size_of::<DbRef>()` for
  `Type::Reference`). To report the **packed inline size** instead, the layout code must
  distinguish a value struct **from the type alone**. Introduce `Type::Value(def, deps)`
  (parallel to `Type::Reference`). *A value struct's fields live on the DEF, not in the
  Type* (unlike `Type::Tuple(elems)`), so its size cannot be computed from the variant
  alone — see next bullet.
- **Cache the inline size + field offsets on the def at finish.** Structs already assign
  field `position` via `calculate_positions_with_groups` / `finish_type`
  **[locate: `Stores::finish_type`, `calculate_positions_with_groups` in `src/database/`]**.
  Compute + cache the value struct's **total packed size** (and per-field offsets) there,
  reusing `element_offsets` **[confirmed 2026-07-08: `src/data.rs:1983`]** /
  `group_size` **[confirmed: `data.rs:2435`]**. Then `size(Type::Value(def))` /
  `element_size(Type::Value(def))` read the cached size (they gain read access to it — via a
  size embedded in the variant `Type::Value(def, size)`, or via a small `Data` lookup; decide
  in Slice 1, **Open Q1**).
- **Wire the size/align chokepoints:** `variables::size` **[confirmed: `variables/mod.rs:1895`]**,
  `variables::align` **[confirmed: `variables/mod.rs:1936`]**, `data::element_size`
  **[confirmed: `data.rs:1928`]**, `data::element_align` **[confirmed: `data.rs:1887`]** —
  add a `Type::Value` arm returning the cached packed size/align (NOT `size_of::<DbRef>()`).
- *Verify:* `sizeof`-style probe (or the alloc harness + a record with a value-struct field)
  shows the value struct occupies its packed field bytes inline, not 12.

### Step 1.3 — construction writes inline (no `new_record`)
- Reference-struct construction allocates a store record (`new_record`
  **[confirmed 2026-07-08: `src/fill.rs:1907`]**) and returns a `DbRef`. For a value struct,
  construction must write the fields **into the destination slot inline** and produce no
  `DbRef` — the tuple-literal lowering already does this (a tuple literal writes packed
  elements into its slot). **Reuse the tuple construction path** for `V { … }` when
  `is_value` **[locate: tuple-literal lowering in `parser/` + its fill op]**.
- *Verify:* constructing a value struct in a loop N times keeps `allocs` at the baseline
  (harness 0.1); the interpreter dump shows an inline block, not an `OpNewRecord` + DbRef.

### Step 1.4 — field access at inline offset
- `get_field` / `get_record` **[confirmed 2026-07-08: `src/fill.rs:1400`, `:1942`]** deref a
  `DbRef` then read at the field position. For a value struct the value IS the inline block —
  read/write at `base + offset` with the cached offset from 1.2. Route field access to the
  **inline-offset read/write** (the tuple-element access op) when the receiver is
  `Type::Value` **[locate: `OpGetField`/`OpSetField` emission in `parser/fields.rs:1250`
  area, and the tuple `OpGetTuple`/element-access op]**.
- *Verify:* `v.a`, `v.a = x`, `v.a += 1` read/write the correct bytes; `event.when.ms`
  (value-struct field of a reference struct — Slice 2 preview) reads with ONE indirection.

### Step 1.5 — value semantics (copy on assign / arg / return)
- Assigning, passing by value, or returning a value struct **copies the inline bytes**
  (memcpy of the packed block), NOT a shared `DbRef` — this is the observable difference from
  reference structs. The tuple path already copies on assignment (tuples are value types);
  inherit it. `&`-ref params for in-place mutation are **Open Q2** (do tuples support `&`
  today?).
- *Verify:* `b = a; b.x = 9; assert(a.x != 9)` — mutating a copy does not touch the original
  (the reference-struct contract is the opposite; this proves value semantics).

### Step 1.6 — no lifetime / no free
- `has_lifetime_concern` **[confirmed 2026-07-08: `src/data.rs:1863`]** gates heap-vs-inline
  and store-side ownership tracking. A **pure-value** value struct (all fields scalar,
  recursively) must return `false` → no `OpFreeRef`, no store, no deps edges. Add the
  `Type::Value` case: `false` iff every field is itself non-lifetime-bearing (recurse); a
  value struct with a `text`/`vector`/reference field is lifetime-bearing (Slice 4).
- Ensure the @PLN85/@PLN90 ownership analysis emits **no free + no deps** for value structs
  **[locate: the free-emission / deps sites keyed on `Type::Reference` — `scopes.rs`, the
  ownership oracle]** (**Open Q3**).
- *Verify:* ownership_oracle reports zero leaks AND zero frees for a value-struct-only
  program; `LOFT_STORES` shows no allocation.

### Step 1.7 — @PLN99 dispatch still works (should be free)
- Operators/format/conversions dispatch by type-def + `t_<len><Type>_Op…` via
  `Data::find_op_method` **[confirmed 2026-07-08: `src/data.rs` @PLN99 fix]** — keyed on the
  def name, representation-independent. Value structs keep `DefType::Struct`, so this
  resolves unchanged. `to_text` / `OpConv…` likewise.
- *Verify:* the flipped `515` — `<`, `==`, `-` (→ value-struct `Duration`), `{d:date}`,
  `"…" as DateTime`, `dt as integer` — all pass with `DateTime`/`Duration` as `value struct`.

### Step 1.8 — native backend parity
- `--native` must emit an inline Rust struct/tuple for a value struct (the pure-value tuple
  path already uses the Rust tuple ABI — **[confirmed 2026-07-08: `data.rs:1859-1865`
  "pure-value tuples continue to use Rust's tuple ABI under --native"]**). Route value-struct
  codegen through that inline path **[locate: `src/generation/mod.rs`, `src/state/codegen.rs`
  struct emission]**.
- *Verify:* `515` passes on `--native` with byte-identical results to `--interpret`; the
  generated Rust has no `LoftStore`/DbRef for the value struct.

### Slice 1 acceptance
`515` flipped to `value struct DateTime`/`Duration`: every assertion green on both backends
**AND** `allocs == 0` (harness 0.1). Graduate the flipped `515` + the Slice-0 local probes to
`tests/scripts/`.

---

## Slice 2 — zero-cost INSIDE records ("records too")

- **2.1** A value-struct FIELD of a reference struct embeds its packed bytes inline in the
  parent record (no DbRef). Reuse the cached offsets (1.2) inside
  `calculate_positions_with_groups` so the parent record layout accounts for the value
  struct's full inline size, and `event.when.ms` reads at `parent_base + when_offset +
  ms_offset` — **one** indirection (the parent DbRef), not two.
- **2.2** A value-struct field of ANOTHER value struct nests inline recursively.
- *Verify:* `struct Event { when: DateTime, … }` with `value struct DateTime`: constructing
  Events adds **no** per-Event DateTime record (alloc harness); `event.when.ms` correct both
  backends; the @PLN97 layout-contract conformance test covers the inline embedding.

## Slice 3 — collections: `vector<V>` inline (the DB-column win)

- Vector elements of a value struct store packed inline (element stride = cached packed
  size), so a million-row timestamp column is a million i64s, not a million records +
  DbRefs. Reuse the existing `vector<tuple>` / `vector<scalar>` inline-element path
  **[locate: vector element storage in `src/store.rs` / `database`]**.
- *Verify:* `vector<DateTime>` of N elements adds zero records; element read/write correct;
  bulk scan stays inline both backends.

## Slice 4 — lifetime-bearing value structs (text/vector fields) — OR defer

- A value struct with a `text`/`vector`/reference field carries a lifetime concern: its
  inline block holds an owned handle that must be freed/copied on move. Either (a) support it
  (the value struct's move copies/frees the inner handle — the tuple-with-Text path already
  faces this, `has_lifetime_concern` = true) or (b) **defer with a stated trigger** and make
  the compiler reject such a `value struct` with a clear message until then.
- *Verify:* either the mixed value struct round-trips leak-free both backends, or the
  rejection diagnostic fires with the fix hint.

## Slice 5 — native ABI parity + perf proof

- Confirm the native representation matches the inline layout byte-for-byte (the
  ir_schema_roundtrip / layout-hash gates), and land the **alloc benchmark**: `515` (and a
  `vector<DateTime>` bulk case) assert `allocs == 0`, committed as a regression.
- *Verify:* full suite green both backends; alloc-count regressions in `tests/scripts/`.

---

## Key representation decision + open questions

- **Open Q1 — size access.** `Type::Value(def, size)` (size embedded, layout fns read it
  directly) vs. `Type::Value(def)` + a `Data` lookup for the cached size. Embedded size keeps
  `variables::size(&Type)`/`element_size(&Type)` signature-compatible (no Data threading) —
  **lean embedded**, decide in Slice 1.2 by which touches fewer call sites.
- **Open Q2 — `&`-ref value structs.** In-place mutation via `&V` param: what do tuples do
  today? If unsupported, value structs pass by copy only initially (state the limitation).
- **Open Q3 — ownership.** Make @PLN85/@PLN90 deps analysis treat value structs as non-heap
  (no deps, no free) — the cleanest hook is `has_lifetime_concern` returning false (1.6);
  audit every free/deps site keyed on `Type::Reference` for a needed `Type::Value` case.
- **Open Q4 — nullable `value struct?`.** No DbRef `store_nr` sentinel; needs an inline null
  representation (an `Optional(Value)` with a reserved sentinel field, or a discriminant
  byte) — ties into @PLN25. Deferable past Slice 1 (start with non-null value structs).
- **Open Q5 — keyword.** `value struct` (recommended — reads as "a struct that is a value")
  vs `inline struct` vs a `#value` modifier. Decide at Slice 1.1.
- **Open Q6 — @PLN97 layout contract.** Value structs change record layout; the conformance
  tests + layout hash must gain value-struct cases (Slice 2).
