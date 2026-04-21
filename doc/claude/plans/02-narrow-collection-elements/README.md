<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# P184 — Narrow integer elements in collection types

**Status:** open — failed attempt 2026-04-21, reverted.

**Goal:** make `vector<i32>` / `hash<i32>` / `sorted<i32>` / `index<i32>`
actually honour the `size(4)` annotation on the `i32` alias.  Today
they silently store and access elements as 8-byte i64, diverging from
the struct-field path that already honours `size(N)` via
`Attribute.alias_d_nr` → `Data::forced_size()`.

**Blast radius:** any binary-format user code — glTF writers, PNG,
custom network protocols — that trusts `vector<i32>` to mean 4 bytes
per element.  Currently surfaced by `lib/graphics/src/glb.loft`
(inline-cast workaround), `lib/graphics/tests/glb.loft`,
`lib/moros_render/tests/geometry.loft::test_map_export_glb_header`.

## Background

The C54 Phase 2c migration (commit `864dafe`, 2026-04-21) made plain
`integer` an 8-byte i64 end-to-end while keeping `type i32 = integer
size(4);` as a narrow alias that forces 4-byte storage **at struct
field level only**.  `src/typedef.rs::fill_database`'s `Type::Integer`
arm consults `forced_size(alias)` for fields; the Vector arm (and
Hash / Sorted / Index arms) do not, because the alias name collapses
into `Type::Integer(min, max, not_null)` with no alias information
preserved in `Type::Vector(Box<Type>, ...)`.

See `doc/claude/PROBLEMS.md` § P184 for the minimal reproducer and
symptom log.

## Why the first attempt failed

On 2026-04-21 I plumbed a `content_alias_d_nr` field through
`Attribute`, captured it in `parse_field` via a sticky
`Parser.last_collection_content_alias` signal set by `sub_type`, and
made the `fill_database` Vector arm use `database.byte/short/int()`
as the content type when `forced_size` was set.

That worked for **storage**: a `vector<i32>` in a struct field got a
narrow (4-byte-stride) database type registered and `vector_append`
used the correct stride.

It did **not** work for **reads**: `src/parser/fields.rs::parse_vector_index`
computes `elm_size = database.size(def(elm_td).known_type)` where
`elm_td` resolves to the base `integer` def-nr via
`Data::type_elm(&Type::Integer(...))` — always the same, regardless
of which alias the user typed.  The emitted `OpGetVector` carried an
8-byte stride even though storage was 4-byte, so `b.v[0]` returned
`(v[1] << 32) | v[0]` instead of `v[0]`.

The half-fix produced **worse** behaviour than the documented
workaround (storage looked right, reads looked wrong — silent data
corruption indistinguishable from code bugs in user programs).
Reverted entirely; the bug stands with the inline-cast workaround.

## Ground rules for the next attempt

1. **All or nothing.** Either every path (storage, read, append,
   insert, set, iterate, native codegen) honours the narrow
   content, or none do.  Partial landing is worse than no change.
2. **Pick the representation first.**  Before touching any
   emission path, decide where the narrow-content signal lives:
   on `Attribute`, on `Type::Vector`, in a side-table, or by
   changing `Data::type_elm` to return alias def-nrs.  The choice
   ripples through every consumer.
3. **One collection kind at a time is OK.**  Vector-only is a
   valid phase boundary — extend to Hash / Sorted / Index / Spacial
   in a follow-up.  But vector-only must STILL honour every
   vector code path (read, append, iterate, …).  You can't split
   "read vs write" within a single collection kind.
4. **Test narrow AND wide side-by-side.**  Every regression test
   adds a `vector<integer>` control next to the `vector<i32>` case
   to prove we didn't narrow the default.

## Representation choice — preferred

**Option A: extend `Attribute.content_alias_d_nr` (the failed
attempt's approach).**  Works for struct-field collections but not
for local-variable collections (e.g. `x: vector<i32> = []`) because
locals don't have an Attribute.  Would need a parallel side-table
keyed by variable.

**Option B: extend `Type::Integer` to `Type::Integer(i32, u32, bool,
Option<NonZeroU8>)` or similar, carrying the forced size.**  The
alias info flows naturally through `Box<Type>` in `Type::Vector`
and every other container.  Breaking ABI change — every `Type::Integer`
constructor and pattern-match has to update.  ~150 sites per `grep -c`.

**Option C: extend the side-channel on the `i32`/`u32`/… alias
defs so `Data::type_elm(&Type::Integer(...))` returns the *alias*
def-nr when the bounds match a registered alias, not always
`"integer"`'s def-nr.**  Requires a bounds-to-alias lookup table
and breaks the invariant that `type_elm` is deterministic for
numerically-equal Integer bounds.  Rejected — `i32` and `integer`
have identical bounds post-2c.

**Preferred: Option B.**  The ABI churn is large but mechanical,
and it eliminates the "which code paths know about alias?" worry
at every site.  A once-and-done.

## Phased plan

### Phase 0 — Representation

**Status:** open.

Change `Type::Integer` from `(i32, u32, bool)` to `(i32, u32, bool,
Option<NonZeroU8>)` (or a dedicated `IntegerSpec` struct).  The
fourth field holds the `size(N)` value for the alias the user
typed; `None` means "use the bounds heuristic" (default).

Call sites to audit:
- `src/data.rs` — `pub static I32`, `I64` constants; the `I32.clone()`
  used by `typedef.rs::complete_definition("integer", ...)`; all
  pattern matches on `Type::Integer(...)`.
- `src/parser/*.rs` — wherever `Type::Integer` is constructed
  (grep `Type::Integer(`).
- `src/typedef.rs::fill_database` — read the new field in both
  the Integer arm (already does via `forced_size(alias)`) and the
  new Vector arm.
- `src/generation/*.rs` — pattern matches on `Type::Integer`.
- `src/state/io.rs`, `src/state/codegen.rs`, `src/scopes.rs`,
  `src/variables/*.rs` — same.

Backwards-compat: the new field defaults to `None` so no existing
behaviour changes.  The fix only activates when a parser site
populates it from the alias's `forced_size`.

**Acceptance:** full `cargo test --release --no-fail-fast` is green
with the field present but populated only by the existing field
path (Phase 0 is a no-op refactor).

### Phase 1 — Parser populates `Type::Integer`'s forced-size

**Status:** blocked by Phase 0.

Wherever the parser resolves an alias like `i32`, `u16`, etc. and
produces `Type::Integer`, set the fourth field from
`data.forced_size(alias_d_nr)`.  Sites:
- `src/parser/definitions.rs::parse_type` and `sub_type` — where
  alias names like `i32`, `u8`, `u16` resolve to `Type::Integer(...)`.
- Any helper that clones `I32` or `I64` into a field's type — the
  clones MUST carry the forced-size of the alias that spawned
  them, not the bare primitive.

Regression: existing `forced_size(alias)` path in the Integer arm
of `fill_database` can now read from `Type::Integer`'s fourth
field instead of `Attribute.alias_d_nr`.  Keep both paths live
until Phase 2 proves equivalence.

### Phase 2 — Resolver emits narrow collection types

**Status:** blocked by Phase 1.

In `src/typedef.rs::fill_database`, update the Vector arm (line
325) to read the forced size from the `Type::Integer` content and,
when present, call `database.byte/short/int()` to produce a
narrow-element content type before `database.vector(c_tp)`.

Extend the same pattern to:
- `Type::Hash(c_nr, _, _)` — hash<i32, key...> content; entries
  stored as structs rather than scalars, so the narrow path only
  applies to hashes of `integer`-alias keys.  Audit carefully.
- `Type::Sorted`, `Type::Index` — same story as Hash.
- `Type::Spacial` — currently a diagnostic-only stub (C7/P22),
  skip until Spacial is actually implemented.

**Acceptance (Phase 2):** a struct field `v: vector<i32>` produces
a database type named `vector<int<min,null>>` (not `vector<integer>`)
and `database.size(content_tp_nr)` returns 4 for it.  Verified via
a dedicated Rust unit test in `tests/data_structures.rs`.

### Phase 3 — Read path

**Status:** blocked by Phase 2.  **This is the hard phase.**

Every site that reads an element from a vector needs to use the
real stride from the database type, not a cached 8-byte constant.

**Parser emission sites** (generate `OpGetVector` / `OpVectorRef`):
- `src/parser/fields.rs::parse_vector_index` line 412 — `elm_size`
  computation.  Today: `database.size(def(elm_td).known_type)`.
  Fix: look up the vector's db_tp via the expression's context
  (field resolution carries the field's db_tp; a local-var
  expression carries the variable's db_tp).  Then read
  `Parts::Vector(content_tp)` → `database.size(content_tp)`.
- `src/parser/control.rs:1623` — same shape in for-loop iteration
  setup.
- `src/parser/fields.rs:458` — field-access vector index.

**Runtime sites** (consume `OpGetVector`):
- `src/fill.rs::get_vector` line 1498 — reads `v_size` from the
  code stream (baked at emission time).  Stays correct iff the
  parser sites above emit the right `v_size`.

**Native codegen sites** (`src/generation/*.rs`):
- Same story — any `vec[i]` expression emits a load whose width
  must match the narrow stride.  Audit
  `src/generation/expressions.rs` and `src/generation/emit.rs`.

**Threading the vector's db_tp to the indexer** is the subtle
part.  Options:
- **Option (i):** record `db_tp` on `Value::Call(OpGetField, ...)`
  as a fourth argument.  Requires parser and codegen updates.
- **Option (ii):** look up the field's db_tp synchronously from
  the expression's Type by running a `Type → db_tp` resolver.
  Feasible because fill_database has already assigned db_tp at
  this point.
- **Option (iii):** precompute a map from Type to db_tp in Data
  after fill_database, query it at parse-index time.

Option (ii) or (iii) is preferred — avoids changing IR shape.

**Acceptance (Phase 3):** the PROBLEMS.md § P184 reproducer runs
green:

```loft
struct Box { v: vector<i32> }
fn test() {
  b = Box { v: [] };
  b.v += [1 as i32, 2 as i32, 3 as i32];
  assert(b.v[0] == 1);        // passes
  assert(b.v[1] == 2);
  assert(b.v[2] == 3);
  f = file("/tmp/out.bin");
  f#format = LittleEndian;
  f += b.v;
  assert(f.size == 12);       // 3 × 4 bytes
}
```

Plus a `vector<integer>` control asserting 8-byte elements still
work.

### Phase 4 — Append / Insert / Set paths

**Status:** blocked by Phase 3.

Stride-using runtime opcodes that currently look up `size(elem_tp)`
via the database type should keep working as-is (the narrow
content_tp has size 4, so `database.size(content_tp) = 4`).  Audit:
- `src/fill.rs::clear_vector`, `remove_vector` — use `v_size` from
  code stream; parser emission must match the content type.
- `src/vector.rs::vector_append`, `vector_finish` — take `size`
  parameter from caller.
- `src/database/structures.rs::vector_add`, `vector_set_size` —
  also size-parametrised.

**Runtime write path** for binary files
(`src/state/io.rs::assemble_write_data` line 121-143) already uses
`database.size(elem_tp)` and will auto-honour narrow content.  No
change.

**Native codegen**: `src/codegen_runtime.rs` and `src/generation/`
— audit `vector_append`, `OpSet*`, `OpInsert*` sites.

**Acceptance (Phase 4):** `lib/graphics/src/glb.loft` can revert to
the natural `vector<i32>`-returning helper:

```loft
fn glb_idx_buf(tris: vector<mesh::Triangle>) -> vector<i32> {
  result: vector<i32> = [];
  for t in tris {
    result += [t.a as i32, t.b as i32, t.c as i32];
  }
  result
}
// caller:
f += glb_idx_buf(m.triangles);  // 4 bytes per index
```

`lib/moros_render/tests/geometry.loft::test_map_export_glb_header`
stays green; file size matches header `total_len`.

### Phase 5 — Local variables + return types

**Status:** blocked by Phase 4.

`x: vector<i32> = []` as a local variable is NOT a struct field, so
`Attribute`-based plumbing doesn't help.  Phase 0's `Type::Integer`
fourth field handles this automatically since local types also
flow through the same Type tree.

**Acceptance (Phase 5):** locals, function parameters, return types
all honour narrow content.  Test matrix:

```loft
fn take(v: vector<i32>) -> integer { ... }
fn make() -> vector<i32> { [1 as i32, 2 as i32] }
fn local() { x: vector<i32> = []; ... }
```

### Phase 6 — Hash / Sorted / Index extension

**Status:** blocked by Phase 5.

Repeat Phases 1-4 for `hash<AliasedScalar>` and equivalents.  Most
of the infrastructure is already in place from the Vector work;
this phase is mostly pattern replication + regression tests.

**Acceptance (Phase 6):** PROBLEMS.md § P184 quick-reference table
row can be struck out.  Detail entry moves to the ~~Fixed~~ section.

## Test strategy

Per-phase regression guards in `tests/issues.rs::p184_*`:

- `p184_vector_i32_binary_write_size` — `f += b.v` produces
  exactly `len × 4` bytes.
- `p184_vector_i32_index_reads_correct_value` — `b.v[0] == 1`
  (not `(v[1] << 32) | v[0]`).
- `p184_vector_integer_still_8_bytes` — control: plain
  `vector<integer>` still produces 8-byte elements.
- `p184_vector_i32_local_var` — `x: vector<i32> = []` honours
  narrow content.
- `p184_hash_i32_key_narrow` — (Phase 6) hash<Struct[i32_key]>.

Integration regressions:
- `lib/graphics/tests/glb.loft` — revert `glb_idx_buf` workaround
  when Phase 4 lands; confirm the natural form produces correct
  byte counts.
- `lib/moros_render/tests/geometry.loft::test_map_export_glb_header`
  — already passes with the workaround; re-verify post-fix.

## Non-goals

- **Changing the default `integer` size.**  Plain `integer` stays
  8 bytes post-2c.  Only *aliased* integers with explicit `size(N)`
  annotations narrow.
- **Adding new narrow aliases.**  `i8`, `u8`, `i16`, `u16`, `i32`,
  `u32` are the existing surface; this plan doesn't introduce
  new ones.
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
  workaround pattern.
