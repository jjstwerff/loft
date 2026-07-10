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

The three items, in the maker's priority order (2026-07-10):

## 1. HIGHER — packed multi-narrow-field corruption

In a struct with **several** narrow-nullable fields, writing `null` to one field
mis-round-trips or clobbers a *sibling*. Repro: `Multi { a: i8?, b: u8? }` constructed
with `b: 20`, then `m.a = null` → `m.b` reads back `0`. A struct with a **single**
narrow-nullable field round-trips `null` correctly, so it is layout/offset-specific
(the null write to one packed field disturbs an adjacent field's bytes).

**Why highest priority:** this is silent corruption of an *unrelated* field — the most
serious failure kind (it violates the soundness promise, not just a nullable edge). Fix
the packed-field offset/masking so a null write touches only its own field.

## 2. MEDIUM — `vector<narrow?>` cannot represent null

`vector<u8?>` / `vector<u16?>` collapse `null` (and an overflowing `x as u8?`) to `0`:
the element reads back `0` and `== null` is **false**. A `u8?` **field** uses a reserved
in-band sentinel and *can* hold null — a vector element cannot.

**Fix direction (maker's call): vectors should carry sentinels too** — give a
`vector<narrow?>` element the same reserved-sentinel encoding a field has, so an element
can represent null exactly like a field does. (`vector<boolean?>` and `vector<character?>`
already hold null — only the packed-narrow-int element kind is missing it.)

## 3. LOW — narrow field sentinel collision

A `u8?` field storing its **extreme** value reads back as `null`: `255 → null` (255 is
the in-band null sentinel); likewise `u16?`=65535, `i8?`=-128, `i16?`=-32768. The extreme
value doubles as the sentinel, so it is unrepresentable-as-a-value in the nullable form.

**Why low priority:** the extreme is rarely the intended payload, the behaviour is
consistent across backends, and the real fix (a wider packed representation that reserves
a sentinel *outside* the value range) is the same deep storage change as items 1–2 — so
this rides along with them rather than earning its own effort.

## Relationship

Items 1–2 want the same thing — a packed narrow-nullable representation that reserves a
null sentinel without stealing an in-range value and without disturbing neighbours. Item
3 dissolves once that representation exists. Pick this up as one focused storage effort
(the A-vs-B decision in RESUME.md's F2 Part-2 note), not piecemeal.
