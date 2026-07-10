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

## 2. MEDIUM — `vector<narrow?>` cannot represent null — ✅ FIXED (2026-07-10)

**Fix landed.** A `vector<narrow?>` element now carries the SAME reserved sentinel a
nullable field does, so it holds null across every narrow width (`u8?`/`i8?`/`u16?`/`i16?`/
`i32?`), on both backends — value, length, OOB-read-null, overflowing `x as u8?`→null, and
no-regression for non-null `vector<u8>` (255 stays a value). Regression:
`tests/scripts/25-narrow-nullable-vector-null.loft`; hand-computed boundary matrix +
prediction in `bytecode-comparisons/vector-narrow-null-prediction.md`.

**Root cause — one missing type-fact, not a storage redesign.** The second probe's
"read-side-only" hypothesis was wrong: the append stored the *integer* sentinel (low byte 0),
so a read-only fix read 0≠255. The first probe's 3-piece analysis was right, and it reduced
to a single fact never threaded to the op selector — the element is DECLARED nullable
(`Optional`), so its slot reserves a sentinel. The `narrow_vec` flag had been a proxy for
"raw, no sentinel," neutralised everywhere by `&& !narrow_vec`. The fix threads the real fact:
1. `data.rs narrow_vector_content` — peel `Optional`, register the NULLABLE narrow Parts
   (`byte(min,true)` / `short(min,true)` / `int(min,true)`) so the element stores narrow + can
   hold null (width-2 nullable uses `Parts::Short`'s `+1` sentinel, not `ShortRaw`).
2. `data.rs NarrowIntKind::of` — a nullable narrow byte/short is `ByteNullable`/`Short`
   (sentinel) whether field or vector; added `reserves_sentinel()` so the read/write `min`
   derives from the kind, not a re-computed `nullable && !narrow_vec`.
3. `fields.rs` index read — pass the element's declared nullability (`Optional`?) to
   `get_val`, not the hardcoded OOB `true`.
4. `mod.rs get_val` + `set_field_check` — `min` from `kind.reserves_sentinel()`.
5. `vectors.rs new_record` — route the narrow element WRITE through `NarrowIntKind` (peel
   `Optional`) so the append op is the exact twin of the index-read op for every width.
6. `OpGetByteNullable` (loft-source `#rust` body + `fill.rs` twin) — add the `rec == 0` OOB
   guard mirroring `OpGetByte`'s `#403` (byte 255 is value-ambiguous, so OOB null keys on the
   null DbRef; `Short`/`Int4` OOB already worked because record-0's zeros ARE their sentinel).

### Original diagnosis (kept for the record)

`vector<u8?>` / `vector<u16?>` collapsed `null` (and an overflowing `x as u8?`) to `0`:
the element read back `0` and `== null` was **false**. A `u8?` **field** used a reserved
in-band sentinel and *could* hold null — a vector element could not.

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

**Sharper hypothesis (2026-07-10, second probe) — likely a READ-SIDE-ONLY fix.** The append
ALREADY stores the full sentinel: `OpSetInt(_elm, 0, OpConvIntFromNull())` writes `i32::MIN`
into the stride-8 element slot, so the null sentinel IS in the store. The bug is the READ:
`OpGetByte(OpGetVectorNullable(v, 8, i), 0, 0)` reads only the LOW byte (`i32::MIN`'s low byte
is `0`) → reads `0`, never null. So the smallest coherent fix is **read-side only**: for a
nullable narrow element emit a full-width read + `i32::MIN`→null decode instead of `OpGetByte`,
keying on `Optional`, leaving the wide storage + `OpSetInt` write untouched. RISK: that read
path (`get_val` / `elm_size` dispatch, `fields.rs:858-905`) is SHARED with FIELD reads (which
already work) and the `collections.rs`/`expressions.rs` shapes — so the decode must key on
(vector-element ∧ `Optional`) without disturbing field reads. Verify against the no-regression
matrix (non-nullable `vector<u8>`, `vector<boolean?>`/`character?`, field reads) + full suite.
Two forks reverted here rather than half-build; this wants a FRESH focused session starting
from the read-side hypothesis, not another one-shot on saturated context.

## 3. LOW — narrow field sentinel collision

A `u8?` field storing its **extreme** value reads back as `null`: `255 → null` (255 is
the in-band null sentinel); likewise `u16?`=65535, `i8?`=-128, `i16?`=-32768. The extreme
value doubles as the sentinel, so it is unrepresentable-as-a-value in the nullable form.

**Why low priority:** the extreme is rarely the intended payload, the behaviour is
consistent across backends, and the real fix (a wider packed representation that reserves
a sentinel *outside* the value range) is the same deep storage change as items 1–2 — so
this rides along with them rather than earning its own effort.

## Relationship

Items 1 and 2 are fixed. Both turned out to be bounded, separable fixes — NOT the deep
storage redesign first feared: item 1 a store-write-width bug, item 2 a single missing
type-fact threaded to the op selector (the reserved-sentinel encoding a field already had,
extended to the vector element). **Item 3 is the only residual** — a narrow field storing
its EXTREME value reads back as null (the extreme doubles as the in-band sentinel). Its real
fix is a wider packed representation that reserves a sentinel *outside* the value range; it is
low priority (the extreme is rarely the intended payload, behaviour is consistent across
backends) and stands alone now that item 2 no longer needs it.
