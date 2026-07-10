<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Narrow-nullable representation — the deferred family (post-@PLN25-core)

The null/dense value model landed (see [RESUME.md](RESUME.md); `formal/types.md` at
0 open deviations). This file records the one **residual family** it left behind: how a
nullable **narrow** scalar (`u8?` / `u16?` / `i8?` / `i16?`) is *stored* across value,
field, and vector positions. Surfaced by a both-backend probe sweep (2026-07-10).

**Key framing — these are representation gaps, not parity bugs.** Every item below is
**identical on `--interpret` and `--native`** (silent-wrong, never a backend divergence).
So they are correctness/storage work, not the cross-backend class. They are the "`u8?`
narrow-packed A-vs-B decision" the plan flagged as its own focused effort — a
storage/layout/packing change in loft's #1-weakness area, **not low-hanging**. The
`boolean?` native-codegen gap found in the same sweep is a *different* thing (a codegen
wrap gap, not representation) and is fixed separately.

The items, in the maker's priority order (2026-07-10):

## 1. HIGHER — packed multi-narrow-field corruption — ✅ FIXED (2026-07-10)

In a struct with **several** narrow-nullable fields, writing `null` to one clobbered a
*sibling* (`Multi { a: i8?, b: u8? }`, `b: 20`, then `m.a = null` → `m.b` read `0`).

**Root cause (turned out NOT to be the deep representation — a bounded store-width bug):**
`Store::set_byte` / `set_short` wrote the null sentinel with a bare literal
(`*addr_mut(rec, fld) = 255;` / `= 0;`). The untyped literal inferred `i32`, so
`addr_mut::<i32>` wrote **4 bytes** and zeroed the packed fields after this one. The value
path was safe because it wrote `(val - min) as u8` / `as u16` (correct width). Fixed with
the `255u8` / `0u16` suffix. Regression: `tests/scripts/25-narrow-nullable-packed-field-clobber.loft`
(byte + short + mixed, both backends). This was separable from items 2–3 and did not need
the sentinel-encoding redesign.

## 2. MEDIUM — `vector<narrow?>` cannot represent null

`vector<u8?>` / `vector<u16?>` collapse `null` (and an overflowing `x as u8?`) to `0`:
the element reads back `0` and `== null` is **false**. A `u8?` **field** uses a reserved
in-band sentinel and *can* hold null — a vector element cannot.

**Fix direction (maker's call): vectors should carry sentinels too** — give a
`vector<narrow?>` element the same reserved-sentinel encoding a field has, so an element
can represent null exactly like a field does. (`vector<boolean?>` and `vector<character?>`
already hold null — only the packed-narrow-int element kind is missing it.)

**Scope (probed 2026-07-10) — type-propagation + op-dispatch, NOT storage.** The byte
stays 1 byte; the sentinels (`Byte` 255, `ShortRaw` `u16::MAX`, `Int` `i32::MIN`) are all
in-band, no stride growth. But it is **not a one-chokepoint flip** — flipping
`narrow_vector_content`'s `nullable` flag alone is *inert* (verified). Both the write and
read sides are non-nullable by construction:
1. `narrow_vector_content` (`data.rs`): peel `Optional`, pass `nullable`, pick the nullable
   `Parts` — necessary but not sufficient.
2. **Append element-build**: the append emits a generic `OpSetInt(elm, 0, null)` whose low
   byte is `0`, so null stores as `0` not the width-correct sentinel. Must width-match the
   element set op (`OpSetByte` / `OpSetShortRaw` / `OpSetInt4`) so null encodes the sentinel.
   (This generic-`OpSetInt`-for-a-byte-element quirk is load-bearing — confirm nothing else
   relies on it before changing.)
3. **Index read**: `v[i]` emits a plain `get_byte` with no sentinel decode (H6 removed it so
   `vector<u16>` can hold `65535`); the nullable element needs the sentinel-decoding read.
All three, ×3 widths, ×2 backends. A deliberate append-lowering + index-read-dispatch slice,
not squeezed in. Loci (`data.rs` / `vectors.rs` / `fields.rs` / `fill.rs` / `generation`) do
not collide with mac-work.

## 3. LOW — narrow field sentinel collision

A `u8?` field storing its **extreme** value reads back as `null`: `255 → null` (255 is
the in-band null sentinel); likewise `u16?`=65535, `i8?`=-128, `i16?`=-32768. The extreme
value doubles as the sentinel, so it is unrepresentable-as-a-value in the nullable form.

**Why low priority:** the extreme is rarely the intended payload, the behaviour is
consistent across backends, and the real fix (a wider packed representation that reserves
a sentinel *outside* the value range) is the same deep storage change as items 1–2 — so
this rides along with them rather than earning its own effort.

## Relationship

Item 1 was a bounded store-write-width bug, separable and now fixed. **Items 2–3 are the
remaining deep family** — both want the same thing: a packed narrow-nullable representation
that reserves a null sentinel *without* stealing an in-range value (item 3) and lets a
vector element carry it like a field does (item 2). Pick those up as one focused storage
effort (the A-vs-B decision in RESUME.md's F2 Part-2 note), not piecemeal.
