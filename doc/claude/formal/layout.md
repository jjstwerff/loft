<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/layout.md — the store byte layout (strict)

**Catalogue:** @F3 (heap/store), @PLN97 (layout contract), #477 (the nested-vector stride that
motivated it), #399 (narrow-int storage).

> **Rules then deviations** (see [README](README.md)). This is the **layout relation** for loft's
> store — the function `layout(τ)` that turns a type into bytes: record size, field/element byte
> offsets, scalar widths, the reference encoding. It is the FORMAT counterpart to
> [heap.md](heap.md): heap.md gives the STEPS (`H-Read` reads at `pos + field_offset`), this doc
> DEFINES that `field_offset`. The two meet at every access — heap.md's semantics operate on the
> bytes this doc lays out.
>
> **One format — no in-memory / on-disk split.** loft's store is ONE thing: the durable file is
> bit-for-bit identical to the in-memory store (`src/store.rs`: *"durability is a metadata layer,
> not a payload-layout change"*). So `layout(τ)` is a single relation, and its rules hold equally
> for the bytes in RAM and the bytes on disk. `layout` is computed once, in `src/data.rs`
> (`element_size` / `element_align` / `element_offsets`) and `Stores::finish` (struct positions);
> **both backends read the same offsets from the one type table** — a divergence is a bug, not a
> second layout.

## The store byte model (the ground the rules stand on)

- **`Store`** — a word-addressed byte region (`src/store.rs`): a fixed header
  `[0 = SIGNATURE (4 B), 4 = free-space index (4 B), 8 = record size (4 B), 12 = content …]`, then
  records. `layout(τ)` governs the CONTENT bytes of a record; the header is a fixed store frame.
- **arena frame** — records live in one arena. **Record 0** is the header (above); **record 1 =
  PRIMARY** is the root container (the program's top `hash` / `sorted` / struct value), its type-id
  at `(rec = 1, pos = 8)`. Every record begins with an `i32` **size word** (in 8-byte words; a
  NEGATIVE size marks a free block), so the chain is walkable by `rec += |size|`. Addressing is
  **word-based**: `rec` is a word index, byte offset `= rec · 8`; a sub-reference is a `u32` at
  `(rec, pos)`. This frame is what makes a store file **self-describing and native-endian** (native ↔
  wasm, RAM ↔ disk) and lets a reader walk it by byte offset — the basis of the #522 remote
  working-set range-reader. The multi-byte words (the `i32` size, the `u32` sub-references, the
signature) are written **native-endian** (`store.rs` `to_ne_bytes`/`from_ne_bytes`), so a store
is portable among **like-endian** hosts — every current target (native and wasm) is
little-endian; a big-endian reader needs a byte-swap (only the `.dcache` sidecar is fixed
little-endian). Endianness is thus a **physical-encoding** detail, outside the frozen logical
layout contract (which pins field/enum identity + order, not byte encoding).
- **record** — a value of type τ occupies `size(τ)` contiguous bytes; a field/element sits at a
  fixed byte offset within it. `H[r ⊕ n]` (heap.md) reads at `r.pos + n`.
- **`DbRef`** = `(store_nr: u16, rec: u32, pos: u32)` (`src/keys.rs`) — the universal pointer. A
  STORED reference to another record is a **4-byte record pointer** (the `rec` into the target
  store); the full 12-byte `DbRef` is stored only where a value must round-trip its whole pointer
  (the closure half of a fn-ref field, `Parts::DbRef`).
- **null is in-band** — a nullable field uses a SENTINEL inside its own bytes (`i64::MIN` for
  integer, `nullref` for a reference), not an extra byte. Nullability is therefore NOT a layout
  fact (rule `L-Null`).
- **the identity** — `layout_algo_hash` + the per-type `layout_dump` (`src/database/types.rs`) is
  the store's **layout identity**: a stable fingerprint of every rule below applied to the
  program's types. It is the version the `.dschema` sidecar records (`src/schema_sidecar.rs`).

## Notation

- `layout(τ) = (size(τ), align(τ), offs(τ))` — the total byte layout of τ: its record size,
  natural alignment, and the offset of each field/element.
- `off(τ, f)` — the byte offset of field/element `f` within a τ record (`offs(τ)` indexed).
- `width(τ)` — the stored width of τ as a FIELD of another record (a scalar's size, or 4 for a
  record pointer). `id(P)` — the layout identity of a program's type table `P`.
- `τ?` is the nullable form of τ; `τ ≈ σ` means "same byte layout" (`layout(τ) = layout(σ)`).

---

## Rules

### The layout function is total and shared

```
  (L-Total)   layout(τ) is a TOTAL function of the finished type τ: every registered type has
              exactly one (size, align, offset-vector), computed in src/data.rs + Stores::finish.
              BOTH backends (interpreter store, native DbRef ABI) read these SAME offsets — a
              backend that computes a different offset is a bug (D-op-1), not a second layout.
```

**In words.** There is one layout per type, and it is decided by the type alone (plus the fixed
algorithm) — never by which backend, which run, or which call site reads it. This is what makes a
store written by one build readable by another *of the same layout*.

### Scalar and narrow-int widths

```
  (L-Scalar)  width(boolean)=1  width(character)=4  width(single)=4
              width(float)=8    width(integer)=8    width(text)=4 (a Str handle)
              align(τ) = its natural alignment (1,4,4,4,8,8,4 respectively).
  (L-Narrow)  a range-annotated integer stores in the SMALLEST width that holds its range (#399):
              u8 → 1 B, u16/i16 → 2 B, i32 → 4 B, else 8 B.  The narrowing is a WIDTH change,
              so it moves offsets and record size — a layout fact the golden pins.
```

**In words.** Each base type has a fixed stored width. A narrow integer (`i32`, `u8`, …) stores in
fewer bytes — which is exactly why a narrowing is a layout change: it shifts every following
field. (#399 is the change class; the golden test pins each width.)

### References, collections, and child records

```
  (L-Ref)     a stored reference / collection field (Reference, Vector, Hash, Sorted, Ordered,
              Index) is a 4-byte RECORD POINTER into the target store.  A ChildRec is a 4-byte
              co-located rec id.  The full 12-byte DbRef is stored only for Parts::DbRef (a
              fn-ref field's closure half).  A collection's ELEMENT stride is width(element).
```

**In words.** A field that points at other records holds a small (4-byte) pointer, not the data
inline. The data lives in the target store; the collection's per-element stride is the element's
own width — so a change to an element's width (e.g. the nested-vector stride, #477) is a layout
change even though the field pointer is unchanged.

### Structs, enums, tuples

```
  (L-Struct)  a struct record packs its fields by DESCENDING alignment; off(τ, fᵢ) is the packed
              position; size(τ) is the packed total.  A field access is H[r ⊕ off(τ, f)].
  (L-Enum)    an enum is a 1-byte discriminant; a data-carrying variant (EnumValue) is
              [tag byte] followed by the variant's fields (L-Struct packing).
  (L-Tuple)   a tuple (τ₀,…,τₙ) is a synthetic __tuple<…> struct.  Element offsets are
              natural-alignment packing — off = the next position ≥ the element's alignment —
              and a tuple has TWO layout views that must compute the SAME offsets: the STACK
              view (data::element_stack_offsets / element_stack_size) and the STORAGE view
              (the synthetic struct, calc::calculate_positions_with_groups, read back by
              data::stored_tuple_offsets).  Their agreement is part of the rule, not an
              implementation detail.
```

**In words.** Fields are packed largest-alignment first, so the record has no wasted padding and
every field lands on its natural boundary. Enums carry a 1-byte tag; a variant with data is that
tag plus the variant's own fields. A tuple is stored as a hidden struct, packed the same way.

⚠ A tuple lives in two places — on the stack and in a record — and the two are computed by
different code. That is why `L-Tuple` names both and requires them to agree: @PLN114 split the
one ambiguous `element_offsets` into the two named views precisely so a site has to declare which
it means, and a site that picks the wrong one reads a plausible offset from the wrong model.

### Nullability is a sentinel, not a layout

```
  (L-Null)    τ ≈ τ?  —  layout(τ) = layout(τ?).  A nullable field has the SAME bytes as its
              not-null form; absence is a sentinel IN those bytes (i64::MIN / nullref), never an
              extra byte or a moved offset.  Nullability is a SCHEMA fact (semantics), not a
              LAYOUT fact.
```

**In words.** Making a field nullable does not change the bytes — the "missing" value is a
reserved bit pattern inside the field. So a nullable and a not-null integer have identical layout;
the difference is meaning, carried by the schema (`data_to_json`), not the layout hash. (Verified:
the golden renders both identically.)

⚠ **The rule as written is true for a SENTINEL type and false for an INLINE STRUCT, measured
2026-08-28 (loft#1134).** `LOFT_DUMP_TYPES=1` on two pairs of structs that differ only in a `?`:

```
IntD[16/8]   k:integer[0]  n:integer[8]        IntN[16/8]   k:integer[0]  n:integer[8]
Dense[24/8]  k:integer[0]  item:S[8]           Nble[32/8]   k:integer[0]  item:__nullable<S>[8]
```

The scalar pair is identical, exactly as the rule says. The struct pair is not: the record grows
by eight bytes and `S`'s own fields move from offset 8 to offset 16, behind a discriminant. That
is not a sentinel — a struct stored inline has no reserved bit pattern to spend, which is why
[types.md](types.md) gives it the tagged `__nullable<S>` (discriminant + payload) and calls the
property bought *"no collision"*. So `(L-Null)` holds for every type that HAS a sentinel and does
not hold for the one representation that needs a tag, and the two documents did not agree.

**The incomplete half is what licensed a defect.** A reader of this rule alone concludes that
`S?` and `S` share their offsets, which is precisely the belief that wrote a dense `S` into a
tagged slot: field `a` landed on the discriminant, presence became a data byte, and a present
`S { a: 0, … }` read back absent (loft#1134, and its layout-side account in
[tuples.md](tuples.md) § D-tup-6). A rule stated for four of its five cases is more expensive
than no rule, because it is CITED.

The rule wants splitting rather than weakening, and the split is decidable from the type alone:

```
  (L-Null)     τ ≈ τ?  for every τ that reserves a null VALUE  —  layout(τ) = layout(τ?);
               absence is a sentinel in those bytes (i64::MIN / NaN / 255 / nullref / codepoint
               0), never an extra byte or a moved offset.
  (L-Null-Tag) a struct stored INLINE — a `vector`/keyed element, an embedded field, a tuple
               member — has no sentinel to spend, so `S?` is the tagged `__nullable<S>`:
               layout(S?) = discriminant ++ layout(S) at the payload base, and
               size(S?) > size(S).  Absence is discriminant 0.  Every writer and reader of such
               a slot goes through the tag; the pair that holds this is
               `Parser::emit_nullable_slot_write` / `emit_nullable_slot_read`.
```

Neither half is a new decision — both are what the code has shipped since @PLN25 — so this
records the boundary rather than moving it. The falsifier below covers `(L-Null)`; `(L-Null-Tag)`
is covered behaviourally by
`tests/scripts/1134-a-nullable-tuple-element-is-stored-behind-its-tag.loft`, whose zero-valued
first field is the cell that a size-and-offset golden cannot see — the same blind spot the ⚠
under the falsifier list already records for the sentinel half.

### The soundness invariant — no silent cross-version misread

```
  (L-Sound)   a reader whose layout identity id(P) differs from a store's RECORDED identity MUST
              reject-or-migrate, NEVER read the bytes raw.  A raw handoff is admitted ONLY when the
              two identities are equal.  Discharged by the .dschema sidecar (schema_sidecar): the
              stored id is compared on load; a mismatch is CorruptReason::SchemaMismatch → rebuild.
```

**In words.** The layout is allowed to change between builds — but a store written under an old
layout must never be *read* under a new one as if nothing moved. The sidecar records the layout
identity the store was written with; on load it is compared, and a mismatch routes to
migrate-or-rebuild. This is the layout analogue of heap.md's `H-Sound`: this doc defines the
cliff (what the bytes mean under a given layout), and the sidecar keeps the program from walking
off it (reading bytes under the wrong layout). Its absence is exactly the #477 failure — a layout
change noticed only by broken data. The gate matters most across a boundary the reader cannot see
behind: the **#522** working-set loader range-reads a REMOTE store over HTTP — it MUST verify the
remote store's layout identity (its `.dschema` / `layout_algo_hash`) before walking the bytes, or
it silently misreads data fetched over the network. `schema_sidecar::check_beside` / `classify` is
that gate, now applied across a network boundary.

---

## Deviations

- **D-layout-1 — no version guard on persisted bytes** (the motivating gap). Before @PLN97 the
  layout was **nowhere written** and **nothing recorded which layout a store was written under**,
  so a layout-changing fix (#477: nested-vector stride 4→8) **silently misread** existing data —
  caught only by breakage, never by a check. `L-Sound` is the rule it violated.

  **Status — mechanism shipped, auto-enforcement pending a consumer.** The rule is now
  *enforceable*: the golden test (`tests/layout_golden.rs`) catches a layout change at commit time,
  and the `.dschema` sidecar (`src/schema_sidecar.rs`, `CorruptReason::SchemaMismatch`) detects a
  stale store at load and routes it through the durable store's `on_corruption` rebuild. **Residual:**
  the durable store (`plans/43`) is not yet driven by a loft builtin, so nothing *calls* the
  load-time gate automatically yet — the deviation closes fully when a persistence consumer wires
  `check_beside` into its open path. Until then the guard exists but is opt-in.

- **D-layout-2 — the `?` changed the layout** (2026-08-28, loft#1125). `L-Null` says
  `layout(τ) = layout(τ?)`, and three sites decided layout by naming `Type` variants BARE, so a
  wrapped shape reached none of them.

  The visible one: the walk that gives an `index` its bookkeeping triple a position runs at the
  end of `fill_all` precisely so `#left_N / #right_N / #color_N` are appended to the ELEMENT
  struct before it is sized. `Optional(Index(…))` matched nothing there, the triple was appended
  afterwards, and `finish_type` returns early for a type that already has a size — so all three
  kept `position: 0`, on top of each other and of the first real field. The nullable form then
  refused to lay out at all while its dense twin was fine.

  Its two siblings, same rule: the `null` → sentinel conversion asked for `Type::Vector` alone
  and was short by the five KEYED kinds and by the wrapper, so a `spatial<P[x,y]>? = null` local
  kept a bare `Value::Null` — which writes nothing — and the scope-exit `OpFreeRef` read the
  untouched bytes as store #0 (BUG #306); and an OMITTED nullable collection FIELD took the zero
  its type gives, where zero is the EMPTY collection and absence has its own reserved id
  (`DbRef::ABSENT_REC`, loft#917), so `c: vector<τ>? = null` read back present-and-empty.

  **Status — CLOSED.** All three read through `base()`. Guard:
  `tests/scripts/a-nullable-collection-lays-out-like-its-dense-twin.loft`, which gives every
  keyed kind its OWN element struct: the layout half is invisible whenever the same index type
  also has a dense local somewhere in the program, because that one registers the bookkeeping in
  time and the nullable form inherits a correct layout.

- **D-layout-3 — three writers did not go through the tag** (2026-08-30, loft#1198). `L-Null-Tag`
  ends *"every writer and reader of such a slot goes through the tag; the pair that holds this is
  `emit_nullable_slot_write` / `emit_nullable_slot_read`"*, and the sentence was a description of
  one writer out of four. Deciding to tag needs the SOURCE's type, and a nullable struct has two
  spellings that mean one thing — the dense `S` and the `S?` a function returns or a local
  declares. The tuple's writer asked `needs_nullable_wrap`, which reads both. The struct field
  (`objects.rs::handle_field`), the element store (`collections.rs`) and the append
  (`vectors.rs`) each spelled `let Type::Reference(src_d, _) = src_tp` instead and so could see
  only the dense one.

  For every `S?`-spelled source the dense record therefore went in untagged, which is `L-Null`'s
  layout applied where `L-Null-Tag` governs — the same confusion of the two halves that D-tup-6
  and D-layout-2 are, arriving this time from the WRITE side. Two faces: a present value landed
  one field low so every read came back one field high, and a value the callee withheld at
  runtime wrote nothing at all, leaving the slot reading PRESENT with its previous value. With
  the discriminant aliased onto the payload's first field, `S { a: 0, … }` read back ABSENT.

  **Status — CLOSED.** All three route through `emit_nullable_slot_write`, which now also
  releases the payload the slot held on its PRESENT arm — one of the three carried that free and
  the shared home did not, so absorbing them without it would have traded a wrong answer for a
  leak. Guard: `tests/scripts/1198-a-nullable-source-is-tagged-into-its-slot.loft`, whose
  controls are the dense source (the half a corpus of literals can see) and the tuple member
  (the writer that already obeyed the rule).

---

## Conformance

Every rule is a program-independent fact both backends must agree on, and each has a standing
falsifier ([@PLN97](../plans/97-layout-contract/README.md)):

- **`L-Total` / `L-Scalar` / `L-Narrow` / `L-Ref` / `L-Struct` / `L-Enum` / `L-Tuple`** — pinned
  byte-exact by the **golden layout test** (`tests/layout_golden.rs` + `tests/golden/layout/`):
  record size + every field position + narrow encoding + collection element stride, over a corpus
  spanning every storage kind. Any change is a red diff; proven to fail on a #477-class
  perturbation. The **coverage audit** (exhaustive over `Parts`) keeps a new storage kind from
  slipping in unpinned.
- **Both backends** — `tests/scripts/509-layout-parity.loft` constructs the corpus and asserts the
  read-path (narrow widths, the nested-vector stride at runtime, tuple packing, enum tag+fields) on
  interpreter, `--native`, and wasm: a value-corrupting ABI divergence fails on the divergent
  backend. (D-op-1's differential falsifier applies here as elsewhere.)
- **`L-Null`** — the golden renders a nullable and a not-null field identically (same size, same
  offsets); nullability lives in the schema, not the hash.

  ⚠ **That falsifier covers the first half of the rule only, and the second half broke under
  it (2026-08-22).** `L-Null` says two things: a nullable field has the same BYTES as its
  not-null form, and absence is a SENTINEL in those bytes. The golden tests the first — sizes
  and offsets — and cannot see the second, because a sentinel is a value and the golden
  compares layout. So a writer may spell absence any way it likes and the golden stays green:
  loft's `JsonValue` JSON walker wrote the zero code into one-byte widths, so an absent `u8?`
  read back as the VALUE `0` and an absent `boolean?` as `false`, while every layout gate
  passed. The sentinel half is now enforced structurally instead — the encodings live at one
  address (`Stores::write_absent_value` / `Stores::write_narrow_value`, the write-side twins of
  `Stores::is_null`) and both JSON walkers call them, so a second spelling has nowhere to live.
  Behavioural guard: `tests/scripts/json-walker-absent-field.loft`, both backends.
- **`L-Sound`** — `src/schema_sidecar.rs` tests: an unchanged store → `Identical` (raw handoff); a
  changed layout → detected (`SchemaVerdict::Changed`), a garbage sidecar → `Unreadable`, each
  mapping to `CorruptReason::SchemaMismatch`. The layout identity a store records is
  `Stores::layout_algo_hash`, itself pinned by the golden.
