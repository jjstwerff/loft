<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# P184 — Narrow integer elements in collection types

## Status — DONE 2026-04-22

`vector<i32>` / `vector<u8>` / `vector<u16>` (and i8 / i16 / u32
siblings) honour the `size(N)` annotation on the integer alias.
Hash / sorted / index struct-key narrowing also covered.

Reference for the post-@PLAN02 storage rules lives in:

- [`../../../INTERMEDIATE.md`](../../../INTERMEDIATE.md) §
  "Integer Storage Size" — the struct-field-vs-vector-element
  variant tables, `IntegerSpec::vector_narrow_width()` API,
  and the `fill_database` runs-only-on-struct-defs gotcha.
- [`../../../DATABASE.md`](../../../DATABASE.md) § "Narrow vector
  elements" — the `Parts::Short` vs `Parts::ShortRaw` divergence
  rationale (raw-byte-copy mismatch) and the
  `Data::narrow_vector_content()` registration helper.

This file is the closure record; phase files in this directory
remain as historical archaeology.

### Phase outcome

| # | Phase | File | Outcome |
|---|---|---|---|
| 0 | Representation — `Type::Integer(IntegerSpec)` named-struct carrier with bounds + `forced_size` | [00-representation.md](00-representation.md) | DONE — commit `d05c8b0` |
| 1 | Parser populates `IntegerSpec.forced_size` from the user-typed alias | [01-parser-populate.md](01-parser-populate.md) | DONE — commit `bf4db07` |
| 2 | Resolver (`fill_database`) emits narrow vector database types | [02-resolver-narrow.md](02-resolver-narrow.md) | DONE — commit `3b6fd43` (struct fields only; sizes 1 + 4) |
| 3 | Read path (`parse_vector_index` + iterator) uses narrow stride | [03-read-path.md](03-read-path.md) | DONE — commit `3b6fd43` |
| 4a | Short-encoding mismatch sidestep: `vector<u16>` / `vector<i16>` stay wide with consistent round-trip | [04-append-set.md](04-append-set.md) | DONE — commit `e61176f` |
| 4b | Introduce `Parts::ShortRaw` direct-encoded variant (Option L-minimal) | [04b-short-encoding.md](04b-short-encoding.md) | DONE — commit `be39b01`.  Bug α (iter-next `narrow_int_cast` destroys `i64::MIN` sentinel) was the root cause of the 2026-04-21 hang; fix landed alongside the ShortRaw variant |
| 5 | Apply narrow-vector registration at local-var, parameter, and return-type sites | [05-locals-returns.md](05-locals-returns.md) | DONE — commit `e78f65c` |
| 6 | Extend to Hash / Sorted / Index | [06-hash-sorted-index.md](06-hash-sorted-index.md) | DONE — primitive-content forms are parse errors; struct-key narrowing works via the existing Phase 2 struct-field path; regression guard landed |

## Scope-surprise narratives (the closure-relevant ones)

### Phase 4 split into 4a (shipped) and 4b (deferred-then-shipped)

`Parts::Short` uses legacy `raw = val - min + 1` encoding where
raw 0 is the null sentinel.  This diverges from the raw-byte
vector-copy path in `vector_add` (`src/database/structures.rs`):
bytes move source → dest without applying the +1 shift, so reads
decode garbage.  `Parts::Byte` and `Parts::Int` use direct
`raw = val - min` / `raw = val` encoding that agrees with
raw-byte copies, which is why 1-byte and 4-byte narrowing landed
cleanly in Phase 2+3.

**Phase 4a** (shipped first) sidestepped the mismatch by gating
`vector_narrow_width()` to `Some(1) | Some(4)` only; `u16` /
`i16` vector fields stayed at 8-byte wide storage with consistent
round-trip.

**Phase 4b** (shipped 2026-04-22, Option L-minimal) introduced
`Parts::ShortRaw` — a direct-encoded 2-byte variant parallel to
`Parts::Int`.  Strictly additive: existing `Parts::Short`
consumers (struct fields with `u16` / `i16` / `integer limit(...)`)
keep the legacy `raw = val - min + 1` encoding unchanged; narrow
vector elements route through the new raw variant.

### Phase 5 was larger than planned — real code, not just tests

Originally scoped as "verify Phases 1-4 already covered these
cases."  That was wrong: `typedef.rs::fill_database` (where
Phase 2's narrow-vector-type registration lived) runs ONLY on
struct definitions.  Local variables, function parameters, and
return types that carry a `vector<i32>` type never reach that
code path — they get the default wide (8-byte)
`vector<integer>` registration at every `database.vector(c_tp)`
parser call site.

Phase 5's real scope: extract the narrow-detection logic into
`Data::narrow_vector_content()` and invoke it at every
`database.vector(c_tp)` call site in `src/parser/`.  Roughly
6 sites.

This gotcha is now documented in INTERMEDIATE.md + DATABASE.md
so future contributors don't replay it.

## Postmortem — why the 2026-04-21 attempt failed

**What was tried**: plumbed `content_alias_d_nr` through
`Attribute`, captured it in `parse_field` via a sticky
`Parser.last_collection_content_alias` signal, and made
`fill_database`'s Vector arm use `database.byte/short/int()` as
the content type when forced_size was set.

**What happened**: storage narrowed correctly (a `vector<i32>` in
a struct field got a narrow 4-byte-stride DB type).  But
`src/parser/fields.rs::parse_vector_index` computes element size
from `Data::type_elm(&Type::Integer(...))` which always returns
the base `integer` def-nr, ignoring the alias.  So `OpGetVector`
carried an 8-byte stride even though storage was 4-byte; indexing
returned `(v[i+1] << 32) | v[i]`.

**Why the shortcut didn't work**: `Attribute.content_alias_d_nr`
exists only on struct fields; local variables don't have
Attributes.  Threading alias info to the indexer from the
Attribute means the indexer needs a reverse lookup from
`Value::Call(OpGetField, ...)` → Attribute → content_alias.
That's not available at parse-index time.

**Lesson**: put the size info on `Type::Integer` itself (Phase 0).
That's what the phased plan did — `Type::Integer(IntegerSpec)`
with `forced_size` lets the alias signal flow naturally through
`Box<Type>` in `Type::Vector` and every other container.

## Representation choice (Option B chosen)

| Option | Outcome |
|---|---|
| A — extend `Attribute.content_alias_d_nr` | The failed approach.  Works for struct-field collections but not for local variables / return types.  Rejected. |
| **B — wrap Integer payload in named struct `IntegerSpec`** | **Chosen.**  ~130 call sites migrated, but most collapse to `Type::Integer(s)` + `s.field` access; constructor helpers (`IntegerSpec::u8()` / `signed32()` / `wide()`) consolidate ~10 sites that duplicated magic bound constants. |
| C — remap `Data::type_elm` to return alias def-nrs | Requires bounds-to-alias lookup; breaks when multiple aliases share the same bounds (`i32` and plain `integer` do post-C54).  Rejected. |

Earlier revision: Phase 0 was first scoped as "add a fourth tuple
field" — `Type::Integer(i32, u32, bool, Option<NonZeroU8>)`.  The
mechanical refactor compiled but degraded readability at every
pattern site.  Scoped up to a named struct on 2026-04-21.

## Non-goals

- **Changing the default `integer` size.**  Plain `integer` stays
  8 bytes.  Only *aliased* integers with explicit `size(N)` narrow.
- **Adding new narrow aliases.**  Surface stays: `i8`, `u8`,
  `i16`, `u16`, `i32`, `u32`.
- **Fixing cdylib FFI asymmetry.**  Real production cdylibs
  (`lib/graphics/native`, `lib/moros_render`) still declare
  `*const i32` across the FFI boundary.  Whether that's
  consistent with in-process `vector<integer>` (8-byte) vs.
  `vector<i32>` (4-byte post-fix) is a separate audit —
  CAVEATS.md § C54 tracks it.

## See also

- [`../../../INTERMEDIATE.md`](../../../INTERMEDIATE.md) §
  "Integer Storage Size" — variant tables + `vector_narrow_width()`
  API + fill_database gotcha
- [`../../../DATABASE.md`](../../../DATABASE.md) §
  "Narrow vector elements" — `Parts::Short` vs `Parts::ShortRaw`
  divergence + `narrow_vector_content()` registration helper
- [`../../CAVEATS.md`](../../CAVEATS.md) § C54 — post-migration
  caveats: binary writers, cdylib FFI layout, memory footprint
- [`../../PROBLEMS.md`](../../PROBLEMS.md) § P184 — original
  bug entry
- `lib/graphics/src/glb.loft::glb_write_indices` — pre-fix
  workaround pattern (the `as i32` casts can now be removed)
- `src/database/types.rs` — `Parts::ShortRaw` definition + the
  `vector_add` raw-byte-copy site
- `src/data.rs::IntegerSpec` / `Data::narrow_vector_content` —
  the public API
