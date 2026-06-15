<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Embedded-record null — a container-held validity bit

> **Part of** [@PLN25](README.md) (nullable sequences). **Status:** design (not yet
> built). **Resolves:** finding 12's open `reference`/struct element case, finding 9
> (nullable struct field defaults), and the `field = nullable_source()` crash
> (`allocation.rs:560` OOB on `store_nr==u16::MAX`; native `OpCopyRecord(cell, ())`).
> **Method:** written as a hypothesis under `design-protocol`; the load-bearing
> claims and their falsification probes are listed in § Probes — run them before code.

## The problem (grounded)

A struct stored **inline** — a `vector<Row>` element (`8 + i*size`) or an embedded
field (`Box { item: Row }`, `OpGetField(bx, 0, …)`) — has no encoding for "absent".
Reference null is `rec==0` and integer null is `i64::MIN`; an inline struct slot is
just the struct's bytes, and **every** byte pattern is a valid value (`Row{id:0,
tag:null}` ≠ "no Row"). So a null `Row` value (which P2 lets a *variable* hold via the
`u16::MAX` DbRef sentinel) cannot be *stored* into an inline slot: `OpCopyRecord`
derefs the null source's store and OOB-crashes.

Storing the struct **by-reference** (a `rec==0`-nullable handle + a separate record per
element) would add an indirection per embedded record — a pointer-chase and a separate
allocation on loft's hottest data path. **That breaks the flat/inline memory model that
makes loft efficient, so it is rejected.**

## The invariant (the one rule)

> An embedded-record slot is null **iff** its **validity bit — held by the container,
> not the record — is clear**. The inline bytes are never a null sentinel; when the bit
> is clear they are simply ignored.

The bit is *out-of-band*, which is the whole point: it never collides with any inline
value, so null becomes representable without reserving a byte pattern the record could
legitimately take. No indirection — the bit lives in memory already in hand (the vector
header's neighbour, or the parent struct), read with a bit-test, no extra load to a
separate object and no per-record allocation.

This is the **same bargain loft already strikes for narrow scalars**: a nullable narrow
integer reserves one code as its null sentinel and `not null` reclaims the full range
(`IntegerSpec::byte_width(nullable)`). The validity bit is that bargain one structural
level up — nullability costs a little encoding space; `not null` buys it back.

## Re-assertion sites — the count that dictates the layout

`src/vector.rs` hardcodes the element base offset (`8 + i*size`) at **32 sites**. If the
bitmap shifted that base (e.g. `[claim, length, bitmap, data]`), the invariant would
silently re-assert at all 32 — a wrong-result spray, not a compile error. **N=32 ×
silent ⇒ reject any layout that moves the element base.** Two consequences fall out:

1. **Bitmap as a side record, element base unchanged.** A nullable-element vector field
   slot holds **two** handles — `[data_rec, validity_rec]` (8 bytes, vs 4 today) — and
   the validity bits live in their own record sized to the data's capacity. Element
   data stays at `8 + i*size`: the 32 sites are untouched.
2. **One validity chokepoint.** The bit is read/written only through a single
   helper pair — `vec_validity_get(db, i) -> bool` / `vec_validity_set(db, i, present)`
   (and the struct-field analog) — so the *new* invariant ("consult the bit") has **one
   home**, not a spray across the element ops. The element ops (`get_vector`,
   `vector_append`, `remove_vector`, copy, default) call the chokepoint; they do not
   open-code the bit math.

## Concrete layout (read the invariant off the instance)

`v: vector<Row> = [Row{1,"a"}, Row{2,"b"}, Row{3,"c"}]; v[1] = null;`

```
field slot (nullable-element):  [ data_rec : u32 | validity_rec : u32 ]   // 8 bytes
  data_rec   →  [ claim | length=3 | Row0(12B) | Row1(stale 12B) | Row2(12B) ]
  validity_rec → [ claim | bits = 0b101 ]            // bit i = element i present
```

- `v[1]` → `vec_validity_get(v,1)` is 0 ⇒ return the `u16::MAX` null sentinel DbRef
  (then P1's null-safe field access covers `v[1].id` on a null element).
- `v[1] = null` → clear bit 1 **and free element 1's heap deps** (its `tag` text
  handle) so the discarded record does not leak; leave the inline bytes.
- `v[1] = row` → set bit 1, copy `row` inline (today's path).
- `v == null`-style element test → the bit; `len(v)` and iteration are unchanged
  (length-based termination from finding 12 already yields all slots, null included).

**Embedded field** (`Box { item: Row }`): the *parent* struct carries a hidden bitmap
word — one bit per nullable embedded-struct field (a `u64` covers 64). `bx.item` reads
the bit; `bx.item = null` clears it. Same invariant, container = the parent struct.

## The two orthogonal `not null` axes

`not null` is the efficiency control on **both** axes; only the **element** axis is new.

| spelling | non-null thing | encoding |
|---|---|---|
| `v: vector<Row> not null` | the **container** (no absent-vector) | P1/P2 `u16::MAX` store sentinel |
| `vector<Row not null>` | the **elements** | **no validity bit** — pure inline |
| `Box { item: Row not null }` | the embedded **field** | no parent bit for `item` |
| `vector<Row not null> not null` | both | fully dense — **byte-identical to today** |

`vector<T not null>` is the established spelling: `vector<integer not null>` parses and
works today (it reclaims the scalar sentinel code); the struct case is the structural
analog (skip the bit). The parser currently rejects `not null` after a *named* element
type inside a generic (`Expect token >`) — wiring that is part of the work.

**Default = nullable** (matches loft's whole-language "nullable unless `not null`"
model): a plain `vector<Row>` / `item: Row` gets the bit. **`not null` = zero overhead**:
no bit, inline bytes only, `[i] = null` / `= null` is a *compile error*. So dense,
perf-critical data (moros/dryopea hot paths) declares `not null` and pays **nothing —
identical layout and speed to today**; only code that asked for nullability pays a bit.

## Probes — falsify before building (design-protocol step 3)

Each is the cheapest test that could prove a load-bearing claim FALSE. Expect to falsify.

1. **Element base is untouched.** *Claim:* the side-record layout leaves all 32
   `8 + i*size` sites correct. *Probe:* grep-confirm the 32 sites read only `data_rec`;
   build the side-record + chokepoint and run the full vector suite — any `get_vector`
   regression falsifies "untouched".
2. **`not null` reproduces today's bytes (the over-unification guard, step 4).** *Claim:*
   `vector<Row not null>` is byte-identical to today's `vector<Row>`. *Probe:* dump the
   record bytes of a `not null` vector before/after the change; any diff means the bit
   leaked into the fast path — the design would then be taxing the case it promised to
   leave alone.
3. **Append/grow keeps bits aligned.** *Claim:* `vector_append`'s ~2× growth and
   `remove_vector`'s shift keep bit *i* paired with element *i*. *Probe:* a matrix —
   append past a capacity boundary, remove-at-0 (shift), with a null in the middle;
   assert each surviving element's presence bit tracks its value. (This is where a
   resize that forgets to grow/copy the bitmap corrupts silently — the highest-risk
   cell.)
4. **Native bit-test, no indirection.** *Claim:* the chokepoint codegens to an inline
   bit-test, not a load through a separate object. *Probe:* `--native-emit` the element
   read; confirm no extra allocation/deref appears versus today's `get_vector`.
5. **Heap-dep free on `= null` (no leak, no double-free).** *Claim:* clearing a bit
   frees exactly the element's nested handles once. *Probe:* a churn loop
   (`v[i] = row; v[i] = null` ×N) under the leak test; a growing store falsifies "freed",
   a use-after-free SIGSEGV falsifies "exactly once".

## Scope / phasing (when built)

1. **E1 — vector elements.** Side-record bitmap + chokepoint; wire `get_vector` /
   `append` / `remove` / copy / default / `== null`. `vector<Row not null>` parser +
   skip-bit. Probes 1–5.
2. **E2 — embedded fields.** Parent-struct bitmap word; `OpGetField` / copy / default;
   `Row not null` field parser. This is finding 9's real fix.
3. **E3 — the crash, retired by construction.** With the bit present, `slot = null`
   clears the bit instead of `OpCopyRecord(null, …)`; the OOB/native crash paths are
   removed. Until E1/E2 land, a `not null`-only stopgap (reject `= null`, raise on a
   runtime-null source) keeps the crash from shipping.

## Open questions

- **Default cost vs. consistency.** Making the *default* nullable adds a bit (and, for
  vectors, a second handle) to existing `vector<Struct>` / embedded fields tree-wide.
  Consistent with the language model and `not null` recovers the fast path — but it IS a
  layout change for existing code. Confirm via Probe 2 + the full corpus that the cost is
  only paid where nullability is declared, and decide whether a migration note is needed.
- **Bitmap word in the struct header vs. trailer**, and whether enum-value records
  (which already carry a `type` discriminant at offset 0) can fold the field-presence
  bits into spare discriminant space instead of a new word.
- **`u16::MAX` (container) vs. validity bit (element)** stay distinct encodings — both
  are this plan's H6 surface; keep them as the two agreed axes, not a third sentinel.

## See also
- [README.md](README.md) findings 8–12 (the matrix that surfaced this), § The invariant.
- `src/vector.rs` (the 32 `8 + i*size` sites; `get_vector`/`append`/`remove`).
- `src/database/structures.rs` `set_default_value` (the Vector + Struct arms gain the bit).
- `src/state/io.rs` `do_copy_record` / `copy_ref_or_null` (the null-source path E3 retires).
- `src/data.rs` `IntegerSpec::byte_width(nullable)` — the scalar precedent this mirrors.
