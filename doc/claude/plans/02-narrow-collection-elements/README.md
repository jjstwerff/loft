<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# P184 — Narrow integer elements in collection types

**Status:** open — Phase 0 ready to start.  First attempt 2026-04-21
reverted (see postmortem below).

**Goal:** make `vector<i32>` / `hash<i32>` / `sorted<i32>` /
`index<i32>` (and their `u8` / `u16` / `u32` / `i8` / `i16`
siblings) honour the `size(N)` annotation on the integer alias.
Today they silently store and access elements as 8-byte i64.

**Blast radius:** any binary-format user code — glTF writers, PNG,
custom network protocols — that trusts `vector<i32>` to mean 4 bytes
per element.  Surfaced by
`lib/moros_render/tests/geometry.loft::test_map_export_glb_header`
which currently passes only because `lib/graphics/src/glb.loft`
carries an inline-cast workaround (`glb_write_indices`).

## Phases

| # | Phase | File | Status | Blocks |
|---|---|---|---|---|
| 0 | Representation — `Type::Integer(IntegerSpec)` named-struct carrier with bounds + `forced_size` | [00-representation.md](00-representation.md) | ✅ done — commit `d05c8b0` | 1 |
| 1 | Parser populates `IntegerSpec.forced_size` from the user-typed alias | [01-parser-populate.md](01-parser-populate.md) | ✅ done — commit `bf4db07` | 2 |
| 2 | Resolver (`fill_database`) emits narrow vector database types | [02-resolver-narrow.md](02-resolver-narrow.md) | ✅ done — commit `3b6fd43` (struct fields only; sizes 1 + 4) | 3 |
| 3 | Read path (`parse_vector_index` + iterator) uses narrow stride | [03-read-path.md](03-read-path.md) | ✅ done — commit `3b6fd43` | 4a |
| 4a | Short-encoding mismatch: `vector<u16>` / `vector<i16>` stay wide with consistent round-trip | [04-append-set.md](04-append-set.md) | open — uncommitted work in-tree | 5 |
| 4b | Introduce `Parts::ShortRaw` direct-encoded variant so 2-byte narrow storage lands without touching the legacy `Parts::Short` | [04b-short-encoding.md](04b-short-encoding.md) | **blocked** — 2026-04-21 attempt hung in `native_dir::16-parser`; reverted. Bisect required before re-attempt. | 6 |
| 5 | Apply narrow-vector registration at local-var, parameter, and return-type sites | [05-locals-returns.md](05-locals-returns.md) | open — **larger than planned** (needs code, not just tests) | 6 |
| 6 | Extend to Hash / Sorted / Index | [06-hash-sorted-index.md](06-hash-sorted-index.md) | open — audit | — |

## Scope surprises found during implementation

Documented here so the plan stays honest and future sessions have the
right expectations.

### Phase 4 split into 4a (shipped) and 4b (deferred)

`Parts::Short` uses legacy `raw = val - min + 1` encoding (stored
`u16` where raw 0 is the null sentinel).  This diverges from the
raw-byte vector-copy path in `vector_add` (`src/database/structures.rs`)
— bytes move from source to dest without applying the +1 shift, so
reads decode garbage.  `Parts::Byte` and `Parts::Int` use direct
`raw = val - min` / `raw = val` encoding that agrees with raw-byte
copies, which is why 1-byte and 4-byte narrowing landed cleanly in
Phase 2+3.

**Phase 4a** (landing now) sidesteps the mismatch by gating
`IntegerSpec::vector_narrow_width()` to `Some(1) | Some(4)` only;
`u16` / `i16` vector fields stay at 8-byte wide storage and their
reads use an 8-byte stride, keeping write + read in agreement.  The
regression guard `p184_vector_u16_round_trip` confirms values
round-trip cleanly through the wide fallback.

**Phase 4b** (planned, no regressions allowed) introduces
`Parts::ShortRaw` — a direct-encoded 2-byte variant parallel to
`Parts::Int`.  It is strictly additive: existing `Parts::Short`
consumers (struct fields with `u16` / `i16` / `integer limit(...)`)
keep the legacy `raw = val - min + 1` encoding unchanged, while
narrow vector elements route through the new raw variant that
agrees with raw-byte copies.  Seven-step plan with acceptance
criteria and a NO-CHANGE regression checklist.  Tracking:
[04b-short-encoding.md](04b-short-encoding.md).

### Phase 5 is larger than planned — real code, not just tests

The original plan stated "Phase 5 is mostly a test phase — verify
Phases 1-4 already covered these cases."  That was wrong.

`typedef.rs::fill_database`, where Phase 2's narrow-vector-type
registration lives, runs **only on struct definitions** (the first
loop gate at the top of `fill_all`).  Local variables, function
parameters, and return types that carry a `vector<i32>` type never
reach that code path — they get the default wide (8-byte)
`vector<integer>` registration at `parser/*.rs::database.vector(c_tp)`
call sites instead.

Evidence: attempting to revert `lib/graphics/src/glb.loft`'s
`glb_write_indices` workaround to the natural form

```loft
fn glb_idx_buf(tris: vector<mesh::Triangle>) -> vector<i32> {
  result: vector<i32> = [];
  for t in tris { result += [t.a as i32, t.b as i32, t.c as i32]; }
  result
}
```

fails `test_map_export_glb_header` with the same BIN-chunk double-
counting as pre-P184: the **local** `result: vector<i32>` uses wide
storage, so `f += result` writes 8 bytes per element while the
header's `idx_bytes = nt * 3 * 4` computation assumes 4.

Phase 5's real scope: extract Phase 2's narrow-detection logic into
a helper (candidate name: `Data::narrow_vector_content`) and invoke
it at every `database.vector(c_tp)` call site in `src/parser/` that
currently uses `data.def(c_nr).known_type` as the content.  Roughly
6 sites (see `grep 'database.vector' src/parser/`).  See
[05-locals-returns.md](05-locals-returns.md).

## Ground rules

**Top-level rule (inherited from `doc/claude/plans/README.md`):
plans never introduce regressions.**  Every phase preserves every
currently-green test and user-facing behaviour.  If a step surfaces
a scope surprise, the plan is re-scoped BEFORE the next commit —
no degrade-now-fix-later.  Phase 4 was split into 4a (shipped) and
4b (planned, additive `Parts::ShortRaw`) for exactly this reason:
4a's read-side gate on its own would have regressed u16 fields
without 4b's matching write-side support, so 4a was scoped down to
"correctness for 1-byte and 4-byte narrowing; 2-byte stays wide".

Per-initiative rules:

1. **All or nothing per collection kind.**  Either every code path
   (storage, read, append, iterate, native codegen) honours the
   narrow content, or none do.  The 2026-04-21 half-fix narrowed
   storage without touching reads and produced worse behaviour than
   the current bug (silent garbage values).  **Never tag a release
   with Phase 2 landed but Phase 3 not yet landed.**
2. **Test narrow AND wide side-by-side.**  Every regression test
   adds a `vector<integer>` control next to the `vector<i32>` case
   to prove we didn't accidentally narrow the default.
3. **Pick the representation first.**  Phase 0's Type::Integer
   extension is the foundation; deviation means replaying the 2026-04-21
   failure.

## Postmortem — why the 2026-04-21 attempt failed

**What was tried**: plumbed `content_alias_d_nr` through
`Attribute`, captured it in `parse_field` via a sticky
`Parser.last_collection_content_alias` signal set by `sub_type`,
and made the `fill_database` Vector arm use
`database.byte/short/int()` as the content type when forced_size
was set.

**What happened**: storage narrowed correctly (a `vector<i32>` in a
struct field got a narrow 4-byte-stride DB type).  But
`src/parser/fields.rs::parse_vector_index` computes the element
size from `Data::type_elm(&Type::Integer(...))` which always
returns the base `integer` def-nr, ignoring the alias.  So
`OpGetVector` carried an 8-byte stride even though storage was
4-byte.  Indexing returned `(v[i+1] << 32) | v[i]`.

**Why the shortcut didn't work**: `Attribute.content_alias_d_nr`
only exists on struct fields; local variables don't have
Attributes.  Threading alias info to the indexer from the
Attribute means the indexer needs a reverse lookup from
`Value::Call(OpGetField, ...)` → Attribute → content_alias.  That's
not available at parse-index time.

**Lesson**: put the size info on `Type::Integer` itself (Phase 0).
That's what the phased plan does.

## Representation choice comparison

- **Option A — extend `Attribute.content_alias_d_nr`.**
  The failed attempt's approach.  Works for struct-field
  collections but not for local variables or return types.
  Rejected.
- **Option B — wrap the Integer payload in a named struct
  `IntegerSpec { min, max, not_null, forced_size }` and change
  `Type::Integer(i32, u32, bool)` → `Type::Integer(IntegerSpec)`.**
  The alias signal flows naturally through `Box<Type>` in
  `Type::Vector` and every other container.  ~130 call sites
  migrate, but most collapse to `Type::Integer(s)` + `s.field`
  access — shorter than the 4-tuple `(_, _, _, _)` form, and
  future-proof when more fields are added.  Constructor helpers
  (`IntegerSpec::u8()` / `signed32()` / `wide()`) consolidate ~10
  sites that duplicate magic bound constants today.  **Chosen —
  see Phase 0.**

  (Earlier revision: Phase 0 was scoped as "add a fourth tuple
  field" — `Type::Integer(i32, u32, bool, Option<NonZeroU8>)`.
  The mechanical refactor compiled but degraded readability at
  every pattern site.  Scoped up to a named struct on 2026-04-21
  after the in-progress bulk edit surfaced the debt.)
- **Option C — remap `Data::type_elm` to return alias def-nrs.**
  Requires a bounds-to-alias lookup.  Breaks when multiple
  aliases share the same bounds (`i32` and plain `integer` do
  post-C54).  Rejected.

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

## Related work

- `doc/claude/CAVEATS.md` § C54 post-migration caveats — binary
  writers, cdylib FFI layout, memory footprint.
- `doc/claude/PROBLEMS.md` § P184 — user-facing workaround and
  symptom log.
- `lib/graphics/src/glb.loft::glb_write_indices` — current
  workaround pattern; reverts in Phase 4.
