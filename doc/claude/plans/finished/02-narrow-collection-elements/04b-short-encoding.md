<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4b — Direct-encoded 2-byte storage for narrow vectors

**Status:** ✅ landed 2026-04-22 via Option L-minimal (Parts::ShortRaw
+ iter-next guard + emit_field/emit_type_creation narrow lookup +
build_vector_code narrow-write hook + iter collection narrow-stride
+ get_val/set_field_check s==2 split).  All `p184_vector_*` tests
green; `tests/native.rs::native_dir` green; 28 of 29 test binaries
green (the 1 failing binary is pre-existing `50-tuples.loft` native
tests hitting a linker disk-space issue, unrelated to P184).

No-regression constraint: every existing `Parts::Short` consumer
must continue working unchanged.

## 2026-04-21 attempt — what went wrong

The full 7-step implementation landed locally (new variant, Store
`get_u16_raw` / `set_u16_raw`, database io arms, `narrow_vector_content`
arm for size 2, `OpGetShortRaw` / `OpSetShortRaw` opcodes,
`get_val` three-way dispatch with vector-narrow gate, codegen_runtime.rs
ShortRaw byte-conversion arms).  All `p184_*` unit tests passed —
including a new `p184_vector_u16_round_trip` guard for the u16
narrow path.

The full-suite run surfaced one hang: `tests/native.rs::native_dir`
(which runs every `tests/docs/*.loft` via `loft --native`) got
stuck compiling or executing `tests/docs/16-parser.loft`.  The
native binary burned 20+ minutes of CPU without terminating.
Killing it produced no diagnostic — exit code was signal 9 from
the kill, not a panic or assert.

**Root cause (identified 2026-04-22)**: `get_val` (`src/parser/mod.rs`)
routes ALL `s == 2` reads to a single opcode, but three distinct
paths can land on `s == 2`:

1. **Struct field with `u16` / `i16` alias** (`alias != u32::MAX`,
   `forced_size(alias) == Some(2)`) — storage is `Parts::Short` via
   the bounds-heuristic `byte_width(nullable)` path in
   `fill_database` for struct fields.  **Uses `+1` encoding.**
2. **Struct field with `integer limit(...)` bounds that fit in 2
   bytes** (no alias forced_size, `byte_width(nullable) == 2`) —
   storage is `Parts::Short` via bounds heuristic.  **Uses `+1`
   encoding.**
3. **Vector element read when `vector_narrow_width` opens to 2**
   (`alias == u32::MAX`, `spec.forced_size == Some(2)`) — storage
   would be `Parts::ShortRaw` (direct, no `+1`).

The original Step 6 (below) replaced the `s == 2` arm with
`OpGetShortRaw` unconditionally.  That correctly handled path 3
but broke paths 1 and 2.  `lib/parser.loft::Parser::prio: u16` and
`lib/code.loft::Block { length: u16 }` (and many others) are path
1 — every u16 struct field read returned `stored - 1`.  The parser
library uses those indices as pointers into its own AST
structures; off-by-one indices corrupted traversal and produced
infinite loops at runtime.  All unit tests passed because the
p184 tests exercised fresh programs without u16 struct fields in
their hot paths; `16-parser.loft` invokes the parser library
itself at runtime, which IS the hot path for u16 struct fields.

**Fix** (folded into the Step 6 revision below): split the
`s == 2` dispatch into vector-narrow vs. legacy-struct based on
which branch of `get_val` chose `s`.  Only vector-narrow goes to
`OpGetShortRaw`; struct fields keep `OpGetShort`.

**Bisect option retained**: if a future regression has no obvious
dispatch-side explanation, apply Steps 1-6 with the gate closed
(`vector_narrow_width` returning `None` for 2), run `native_dir`,
then open Step 7.  That confirms whether the issue is structural
in Steps 1-6 or only triggered by Step 7.

## 2026-04-22 Option D attempt — also failed, deeper invariant broken

A second re-implementation attempt tried a simpler design:

**Option D** — reuse the existing `Parts::Short` for narrow 2-byte
vector content (not a new variant).  Three changes:
1. `src/data.rs::vector_narrow_width` opens `2 => Some(2)`.
2. `src/data.rs::narrow_vector_content` adds
   `2 => Some(database.short(spec.min, false))`.
3. `src/parser/vectors.rs::get_type` adds
   `2 => self.database.short(spec.min, false)`.

Hypothesis: source literals and destination fields both register
`Parts::Short`, their `+1` encoding matches, `vector_add`'s
raw-byte copy stays valid, `OpGetShort` / `OpSetShort` already
dispatch correctly in `get_val` / `set_field_check` at `s == 2`.
No new opcodes, no bootstrap.

**What actually happened:**

1. `p184_vector_u16_round_trip` failed at `v[0] == 1` (got 0).
   The `build_vector_code` write path in `src/parser/vectors.rs`
   calls `set_field(ed_nr = type_def_nr(in_t), f_nr = usize::MAX, …)`.
   `type_def_nr(Type::Integer(u16_spec))` returns the PLAIN
   `integer` def_nr (see `src/data.rs:2047`), so `attr_type`
   gives back the wide `integer`'s returned type (no
   `forced_size`), and `set_field_check` picks `s = 8` →
   emits `OpSetInt` (8-byte write).

2. **Why this accidentally worked for `u8`/`i32` and breaks for
   `u16`**: `Parts::Byte` and `Parts::Int` use DIRECT encoding
   (no `+1` shift).  An 8-byte little-endian write's low 1 or 4
   bytes coincide with the correct raw content for the narrow
   slot.  `Parts::Short`'s `+1` encoding expects `raw = val + 1`;
   the low 2 bytes of a wide write carry just `val`, so
   `get_short` decodes `val - 1` (off by one).  The pre-P184
   `u8`/`i32` narrow storage has been relying on this
   accidental encoding symmetry the whole time.

3. Adding a narrow-write hook in `build_vector_code` (emit
   `OpSetByte` / `OpSetShort` / `OpSetInt4` directly when
   `in_t` has `vector_narrow_width == Some(n)`) fixed the
   `p184_vector_u16_round_trip` test — writes encode correctly.

4. **But** `native_dir::16-parser` STILL hung.  Deeper
   investigation: `lib/parser.loft::type_def` returns `u16` and
   the parser library has a local `parameters = []; parameters
   += [p]` where `p: u16`.  The native codegen registered this
   vector wrapper as `main_vector<integer(0, 65535)>` with
   content type `t0` (the WIDE `integer`, not `Parts::Short`).
   So storage was 8-byte-stride, but my narrow-write hook keyed
   off `in_t`'s `forced_size == Some(2)` and emitted `OpSetShort`
   into an 8-byte slot — writing `+1`-encoded 2-byte values into
   wide slots with 6 trailing zero bytes.  Reads via `OpGetVector`
   with `elm_size = 2` got the `+1`-encoded u16 correctly, decoded
   via `get_short` to `val - 1 + 1 = val` — no wait, both would be
   off if the stride was 8-byte.  Actual effect: parser AST u16
   indices came back off by one, corrupting the parser's internal
   record pointers and looping infinitely.

## Root architectural finding

There are **at least two independent registration paths** for
vector content types in the compiler:

- **Path 1 — `typedef.rs::fill_database` Vector arm + `Parser::vector_of`**.
  Consults `Data::narrow_vector_content` (Phase 5 canonical).
  Registers `Parts::Short` / `Parts::Byte` / `Parts::Int` when
  narrow content applies.
- **Path 2 — `main_vector<T>` wrapper-struct field registration**.
  Used for local vector variables.  Registers the wrapper's
  `vector` field with content type `t0` (wide integer) for
  `vector<u16>` locals — it does NOT consult
  `narrow_vector_content`.  The name "main_vector<integer(0, 65535)>"
  (bounds-based, not forced_size-based) is the visible tell.

When Path 1 and Path 2 disagree about the content encoding,
`vector_add`'s raw-byte copy and every write/read opcode see a
mix of `Parts::Short` sources and wide destinations (or vice
versa).  The `u8`/`i32` cases happen to work because direct
encoding tolerates the mismatch on the low bytes.  `Parts::Short`
exposes it because `+1` encoding does not.

**This is the same root-cause class as the 2026-04-21 attempt**,
just manifesting one layer up from `get_val`'s dispatch.  Fixing
Step 6 is necessary but not sufficient.

## Path forward — what 4b actually needs

Phase 4b cannot safely land without ONE of:

**Option E — Unify the registration paths.**  Route every vector
creation site (including the `main_vector<T>` wrapper's field
registration) through `Data::narrow_vector_content`.  Audit every
`database.vector(...)` call plus every
`database.structure("main_vector<T>", 0)` + `db.field(..., vec_vector)`
emission in `src/generation/mod.rs`.  Estimated scope: 6 direct
call sites in `src/parser/` + a larger audit of
`src/generation/mod.rs::init()` emission for `main_vector<T>`
structs.  Preserves Option D's simplicity (no new variant)
but widens the refactor.

**Option B-revisited — `Parts::ShortRaw` with direct encoding.**
The original 7-step plan.  Direct encoding tolerates the Path 1 vs
Path 2 mismatch the same way `Parts::Byte` / `Parts::Int` do
today.  Requires the 10-step opcode bootstrap, but is more
robust to the "multiple registration paths" architectural
weakness.  Must still use the corrected Step 6 dispatch
(split `s==2` into vector-narrow vs struct-legacy) per the
2026-04-22 root cause analysis above.

**Option F — narrow element writes by consulting storage, not
type annotation.**  Change `build_vector_code`'s narrow-write
hook to check the ACTUAL registered vector content
(`database.types[vec_db_tp].parts`), not `in_t`'s `forced_size`.
If content is `Parts::Short`, emit `OpSetShort`; if `Parts::Int`,
`OpSetInt4`; etc.  This treats storage as the source of truth
and gracefully handles Path 1 vs Path 2 divergence.  Less
invasive than Option E, but harder to reason about: future
storage-path changes could silently re-break the read/write
consistency.

Recommendation: **Option E** — it removes the architectural
weakness rather than tolerating it.  Option B-revisited is the
fallback if Option E turns out to be larger than expected.

## 2026-04-22 Option D revert

Source changes reverted to Phase 4a baseline:
- `src/data.rs::vector_narrow_width` → `{1, 4}` only
- `src/data.rs::narrow_vector_content` → `1 and 4` arms only
- `src/parser/vectors.rs::get_type` → `1 and 4` arms only
- `src/parser/vectors.rs::build_vector_code` → no narrow-write hook

All `p184_vector_*` tests green.  `tests/docs/16-parser.loft`
under `loft --native` completes successfully.  Plan's
no-regression rule preserved.

Files reverted to the Phase 5 baseline state:
- `default/01_code.loft`
- `src/codegen_runtime.rs`
- `src/data.rs`
- `src/database/{format.rs, io.rs, mod.rs, search.rs, structures.rs, types.rs}`
- `src/fill.rs`
- `src/parser/{mod.rs, vectors.rs}`
- `src/store.rs`
- `tests/issues.rs` (the `p184_vector_i16_narrow_read` guard was
  part of the revert; re-add when Phase 4b lands properly)

**Goal:** land 2-byte narrow storage for `vector<u16>` / `vector<i16>`
(and any `integer limit(...) size(2)` alias) without touching the
legacy `Parts::Short` encoding that struct fields depend on.

**Approach:** introduce a second 2-byte `Parts` variant
(`Parts::ShortRaw`) that uses direct `val = raw + min` encoding
parallel to `Parts::Int`'s approach.  Use it ONLY for vector
elements; struct fields keep `Parts::Short` and its `raw = val - min + 1`
null-sentinel encoding.  No existing site is touched; all changes
are additive.

---

## Why a new variant, not an encoding change

Three options considered:

| Option | Change | Risk |
|---|---|---|
| A — Change `Parts::Short` encoding to direct | Strip the `+1` shift from `set_short` / `get_short` | Breaks every struct field with `u16` / `i16` / `integer limit(-32768, 32767)` that uses the null sentinel.  Tests + user programs would silently corrupt data. |
| B — Add `Parts::ShortRaw` (direct-encoded) | New variant with its own Store accessors and runtime opcode family | Additive.  No existing site changes behaviour.  Rollback is a single revert. |
| C — Make `vector_add`'s raw-byte copy encoding-aware | Loop through each element and re-encode via `set_short` | Loses the one-memcpy path.  Adds branching in a hot path.  Also needs `OpAppendVector` and every other raw-byte-copy site audited. |

**Option B is chosen.** Symmetric with how `Parts::Int` and
`Parts::Byte` coexist today (they differ only in element width, both
use direct encoding).  The only code paths that need to learn about
`ShortRaw` are the ones that ALREADY dispatch on Parts variants.

---

## Work breakdown

Seven steps; each is independently reviewable and small.  Land them
as a single atomic commit (either all pass or none) — the cross-step
dependencies are tight and a partial state would fail the full
suite.

### Step 1 — `Parts::ShortRaw` variant in the type table

`src/database/types.rs`:

- Add variant: `Parts::ShortRaw(i32, bool)` — same shape as
  `Parts::Int` with min bound + nullable flag.
- Extend `database.size(tp)` so `ShortRaw(_, _) => 2`.
- Extend `database.is_base(tp)` / `is_linked(tp)` as needed so
  callers treating "any primitive that fits in a stack value"
  correctly include `ShortRaw`.
- Add a registration helper:

```rust
pub fn short_raw(&mut self, min: i32, nullable: bool) -> u16 {
    let name = format!("short_raw<{min},{nullable}>");
    if let Some(nr) = self.names.get(&name) { return *nr; }
    let num = self.types.len() as u16;
    self.types.push(Type::new(&name, Parts::ShortRaw(min, nullable), 2));
    self.names.insert(name, num);
    num
}
```

Every `match` on `Parts` currently enumerates every variant — add
the `ShortRaw` arm with the appropriate behaviour.  Run
`cargo check --release` and fix each `non_exhaustive_patterns`
warning until silent.

### Step 2 — `Store::get_i16_raw` / `set_i16_raw`

`src/store.rs`:

```rust
#[inline]
pub fn get_i16_raw(&self, rec: u32, fld: u32) -> i16 {
    if rec != 0 && self.valid(rec, fld) {
        *self.addr(rec, fld)
    } else {
        i16::MIN
    }
}

#[inline]
pub fn set_i16_raw(&mut self, rec: u32, fld: u32, val: i16) -> bool {
    if rec != 0 && self.valid(rec, fld) {
        *self.addr_mut(rec, fld) = val;
        true
    } else {
        false
    }
}
```

Direct raw 2-byte read/write.  `i16::MIN` as null sentinel mirrors
`i32::MIN` in `get_i32_raw` / `set_i32_raw`.  No `+1` shift.

### Step 3 — `database/io.rs` read_data / write_data arms

`src/database/io.rs`:

- `read_data`: add `Parts::ShortRaw(_, _) => store.get_i16_raw(...).to_le_bytes()`.
- `write_data`: add `Parts::ShortRaw(_, _) => set_i16_raw(...)`.
- Size lookup (`self.size(elem_tp)`): already covered by Step 1's
  `database.size()` change.

This is what makes `f += vector<u16>` emit 2 bytes per element in a
binary file.

### Step 4 — Vector-resolver arm extension

`src/typedef.rs::fill_database`, Vector arm:

```rust
match forced.get() {
    1 => Some(database.byte(spec.min, false)),
    2 => Some(database.short_raw(spec.min, false)),  // ← NEW
    4 => Some(database.int(spec.min, false)),
    _ => None,
}
```

One line added.  Now `vector<u16>` / `vector<i16>` fields get
2-byte-stride storage via the new direct-encoded variant.

### Step 5 — `OpGetShortRaw` / `OpSetShortRaw` opcodes

These emit the narrow-stride read/write when used at vector
indexing sites.

`default/01_code.loft`: add declarations mirroring `OpGetInt4` /
`OpSetInt4`:

```loft
fn OpGetShortRaw(v1: reference, fld: const u16, min: const i16) -> integer;
#rust"{{let db = @v1; let raw = stores.store(&db).get_u16_raw(db.rec, db.pos + u32::from(@fld)); if @min < 0 {{ i64::from(raw as i16) }} else {{ i64::from(raw) }}}}"
fn OpSetShortRaw(v1: reference, fld: const u16, min: const i16, val: integer);
#rust"{{let db = @v1; let raw: u16 = if @min < 0 {{ (@val as i16) as u16 }} else {{ @val as u16 }}; stores.store_mut(&db).set_u16_raw(db.rec, db.pos + u32::from(@fld), raw);}}"
```

`src/fill.rs` and native codegen (`src/codegen_runtime.rs`) are
regenerated from the `#rust"…"` bodies.  See **Opcode-addition
procedure** below for the exact bootstrap sequence — skipping any
step produces the `Too many defined operators` panic or the
`no method named 'get_u16_raw'` compile error the first 4b
attempt hit.

---

### Opcode-addition procedure (verified 2026-04-22)

New opcodes require a bootstrap because `regen_fill_rs` compiles
`loft` in order to discover the declared ops, and `loft` cannot
compile without the generated dispatch entries the regeneration
produces.  The procedure is therefore:

1. **Add any new Store/stores methods first.**  The `#rust"…"`
   bodies you'll declare in the next step reference them.  E.g.
   `Store::get_u16_raw` / `set_u16_raw` must exist in `src/store.rs`
   before the regen can compile their callers.
2. **Declare the opcodes in `default/01_code.loft`** with
   `fn OpName(...) -> ret;` plus the `#rust"…"` body.  Keep the
   declaration adjacent to the existing `Op*` family it extends
   (e.g. new `OpGetShortRaw` next to `OpGetInt4`) so regen
   output is readable.
3. **Grow the `OPERATORS` array size in `src/fill.rs`** — change
   `&[fn(&mut State); N]` to `&[fn(&mut State); N+k]` where `k`
   is the number of new ops.  Without this, regen panics with
   `Too many defined operators (N of N used)` before it writes
   a single line.
4. **Append placeholder identifiers at the bottom of the
   `OPERATORS` array**, matching the snake_case form of the new
   op names (e.g. `OpGetShortRaw` → `get_short_raw`).  Append in
   the order declared in `default/01_code.loft` — the array
   index becomes the opcode number and must match what the
   parser emits via `data.def_nr("OpGetShortRaw")`.
5. **Add placeholder function definitions with matching
   signatures** at the end of `src/fill.rs`.  Empty bodies are
   fine — regen overwrites them.  Required so the array
   references resolve and the crate compiles.
6. **Build**: `cargo build --release`.  Must succeed before
   regen runs.
7. **Regenerate**: `cargo test --release --test issues
   regen_fill_rs -- --ignored --nocapture`.  This overwrites
   `src/fill.rs` with canonical content derived from every
   `#rust"…"` body in `default/*.loft`.  The OPERATORS array
   shrinks to match what was declared — if you grew too much,
   the regen reports the correct size.
8. **Rebuild dependents**:
   - `cargo build --release --lib` — refreshes the interpreter.
   - `cargo build --release --target wasm32-unknown-unknown --lib
     --no-default-features --features random` — refreshes the
     WASM rlib.  The freshness check in `tests/html_wasm.rs`
     catches this if you skip it.
   - `(cd tests/lib/native_pkg/native && cargo build --release)`
     — refreshes the fixture cdylib.  Same freshness check in
     `tests/native_loader.rs`.
9. **Audit native codegen** (`src/codegen_runtime.rs`):
   regen_fill_rs does NOT touch this file.  Any `match parts`
   that enumerates every `Parts::*` variant gets a non-exhaustive
   warning when the new variant is added; add the new arm
   manually.  For opcodes that add new `stores.method()` calls,
   add equivalent calls in codegen_runtime.rs if the native
   codegen path uses them (look for parallel `OpGetInt4` /
   `OpSetInt4` handling and mirror).
10. **Run `native_dir`** (`cargo test --release --test native
    native_dir`) **before committing**.  The full-suite run is
    ~5 min of pure native-mode test compilation; catches the
    class of regression that bit the 2026-04-21 4b attempt where
    every unit test passed but one native-compiled script hung.
    Do NOT commit based on unit-test success alone.

**Ordering constraint**: the opcode number is determined by the
order entries appear in `OPERATORS`, which `regen_fill_rs`
derives from the declaration order in `default/*.loft`.  Any
reordering of existing opcodes in `default/*.loft` invalidates
every `.loftc` cache and every pre-compiled native package that
embeds the old numbers — NEVER reorder existing op declarations
while adding new ones.  Append at the end of the relevant family.

**Failure mode audit (2026-04-21)**: the first 4b attempt added
opcodes to the BOTTOM of `default/01_code.loft` (after
`const_store_text`).  Regen ran cleanly.  All `p184_*` unit
tests passed.  `native_dir` hung on `tests/docs/16-parser.loft`
for 20+ minutes of CPU.  The regeneration bootstrap itself is
not the root cause — every step completed correctly — but
step 10 (native_dir before commit) would have caught the
regression before it shipped.  Adding it to the procedure above
is the 2026-04-22 takeaway.

### Step 6 — Parser dispatch (corrected 2026-04-22)

Three paths can land on `s == 2`:

- **Path A — struct field with `u16` / `i16` alias** (`alias != u32::MAX`,
  `forced_size(alias) == Some(2)`).  Storage is `Parts::Short`.  Must
  dispatch to **`OpGetShort` / `OpSetShort`** (legacy `+1` encoding).
- **Path B — struct field bounds-heuristic landing at 2 bytes**
  (`alias == u32::MAX`, `spec.forced_size.is_none()`, `byte_width == 2`).
  Storage is `Parts::Short`.  Must dispatch to **`OpGetShort` /
  `OpSetShort`** (legacy encoding).
- **Path C — vector element with narrow forced_size**
  (`alias == u32::MAX`, `spec.forced_size == Some(2)`,
  `vector_narrow_width == Some(2)`).  Storage is `Parts::ShortRaw`.
  Must dispatch to **`OpGetShortRaw` / `OpSetShortRaw`** (direct
  encoding).

The first 4b attempt dispatched ALL `s == 2` to `OpGetShortRaw`,
regressing paths A and B.  `lib/parser.loft::Parser::prio: u16` is
path A; `lib/code.loft::Block { length: u16 }` is path A.  Every
u16 struct-field read returned `stored - 1`, producing off-by-one
indices that caused infinite loops in parser AST traversal — the
`16-parser.loft` native hang.

**`src/parser/mod.rs::get_val`** — track which branch picked `s`
so the `s == 2` case can split:

```rust
let (s, narrow_vec) = if alias != u32::MAX {
    (self.data.forced_size(alias)
         .unwrap_or_else(|| spec.byte_width(nullable)), false)
} else if let Some(n) = spec.forced_size.and_then(|_| spec.vector_narrow_width()) {
    (n, true)  // Path C — vector-narrow
} else {
    (spec.byte_width(nullable), false)  // Path B
};
// …
if s == 1 {
    self.cl("OpGetByte", &[code, p, Value::Int(spec.min)])
} else if s == 2 && narrow_vec {
    self.cl("OpGetShortRaw", &[code, p, Value::Int(spec.min)])
} else if s == 2 {
    self.cl("OpGetShort", &[code, p, Value::Int(spec.min)])
} else if s == 4 {
    self.cl("OpGetInt4", &[code, p])
} else {
    self.cl("OpGetInt", &[code, p])
}
```

**`src/parser/mod.rs::set_field_check`** — symmetric fix for the
write side.  `insert(vector<u16>, idx, x)` routes through
`set_field(ed_nr=<u16 elem>, f_nr=usize::MAX, …)` which currently
takes `alias_nr == u32::MAX` and lands at `tp.size(nullable)` via
bounds heuristic — path B.  Add a path-C branch:

```rust
let (s, narrow_vec) = if alias_nr != u32::MAX {
    (self.data.forced_size(alias_nr).unwrap_or_else(|| tp.size(nullable)), false)
} else if let Type::Integer(spec) = tp
    && let Some(n) = spec.forced_size.and_then(|_| spec.vector_narrow_width())
{
    (n, true)
} else {
    (tp.size(nullable), false)
};
// …
if s == 2 && narrow_vec {
    self.cl("OpSetShortRaw", &[ref_code, pos_val, m, val_code])
} else if s == 2 {
    self.cl("OpSetShort", &[ref_code, pos_val, m, val_code])
} else …
```

**Invariant:** `narrow_vec == true` iff the underlying storage is
`Parts::ShortRaw` iff the read/write opcode MUST use direct
encoding.  Every opcode choice is now driven by the same predicate
`narrow_vec` that the resolver uses to pick `Parts::ShortRaw` over
`Parts::Short` — single source of truth.

### Step 7 — `vector_narrow_width` gate opens to 2

`src/data.rs`:

```rust
pub fn vector_narrow_width(&self) -> Option<u8> {
    match self.forced_size?.get() {
        1 => Some(1),
        2 => Some(2),  // ← added
        4 => Some(4),
        _ => None,
    }
}
```

---

## Integration: vector writes through `vector_add`

The append path uses raw-byte copy (`vector_add` at
`src/database/structures.rs:149`).  With `Parts::ShortRaw`, source
and dest agree on encoding:

- Dest registered via `database.short_raw(0, false)` → size 2, direct encoding.
- Source (a vector literal `[1 as u16, 2 as u16]`) is also typed as
  `vector<u16>` — Phase 5's narrow-registration helper (see
  `05-locals-returns.md`) ensures the literal's db_tp uses the
  same `Parts::ShortRaw`.
- Raw byte copy moves bytes `[01 00][02 00]` directly.  Read at
  `get_i16_raw` yields `1` and `2`.  ✓

For element-assign `b.v[i] = 42`, codegen emits `OpSetShortRaw(pos, val)`
— no re-encoding surprise.

---

## Test matrix for Phase 4b

Unignore + extend `p184_vector_*` in `tests/issues.rs`:

| Test                                    | Assertion                                         |
|-----------------------------------------|---------------------------------------------------|
| `p184_vector_u16_narrow_read` *(new)*   | u16 field reads correct values after `+=` append. |
| `p184_vector_i16_narrow_read` *(new)*   | i16 field with negative values in bounds.         |
| `p184_vector_u16_element_assign` *(new)*| `b.v[i] = x` lands at the right 2-byte slot.      |
| `p184_vector_u16_binary_write_size` *(new)* | `f += b.v` produces `len × 2` bytes.          |
| `p184_vector_u16_round_trip`            | Existing 4a guard — still passes (semantic unchanged). |

Plus control guards:

- Struct-field with `r: u16 not null`: writes + reads through
  `Parts::Short` legacy encoding unchanged.  Run `06-structs.loft`
  equivalent with `u16`-typed fields.
- `integer limit(-32768, 32767)` struct field: bounds heuristic
  path, `alias = u32::MAX`, `spec.forced_size = None` →
  `get_val` uses `byte_width(nullable) == 2` → `OpGetShort`
  (legacy).  UNCHANGED from pre-P184.

---

## Regression checklist — what must NOT change

Every one of these behaviours is verified by existing tests.  Add
NEW tests that lock them in BEFORE touching the Parts code so the
guards catch any regression during implementation:

- [ ] Struct field `u16 not null` stores correctly via `OpSetShort`
      (legacy encoding, null sentinel at raw=0).
- [ ] Struct field `integer limit(0, 100)` uses `Parts::Short`,
      bounds heuristic, reads via `OpGetShort`.
- [ ] Full `tests/scripts/06-structs.loft` green.
- [ ] `tests/scripts/20-binary.loft` green — binary writers that
      emit `u16` values as struct fields.
- [ ] `lib/graphics/src/glb.loft::glb_json_u16_view` (if any) —
      audit for u16 usage.

If any of these are unguarded today, add a `tests/issues.rs`
regression test for each BEFORE starting Step 1.

---

## Performance implications

`Parts::ShortRaw`'s `get_i16_raw` / `set_i16_raw` avoid the `+1` /
`-1` arithmetic that `Parts::Short`'s accessors do.  Slightly
faster per-element access for narrow vectors.  No impact on struct
fields (still using `Parts::Short`).

`vector_add`'s raw byte copy becomes legal for 2-byte elements.
Today it's already used for 1-byte and 4-byte narrow vectors — no
code path change, just an additional element-size value.

---

## Acceptance

- [ ] `Parts::ShortRaw` variant exists with `database.short_raw()`
      registration.
- [ ] `Store::get_i16_raw` / `set_i16_raw` exist.
- [ ] `database/io.rs::read_data` and `write_data` handle
      `Parts::ShortRaw`.
- [ ] `fill_database` Vector arm registers `ShortRaw` for 2-byte
      narrow content.
- [ ] `OpGetShortRaw` / `OpSetShortRaw` opcodes work in interpreter
      AND `--native` mode.
- [ ] `get_val` dispatches to `OpGetShortRaw` only when the alias is
      absent AND `vector_narrow_width() == Some(2)`.
- [ ] `vector_narrow_width()` opens `Some(2)`.
- [ ] All new `p184_*` tests for 2-byte narrow vectors green.
- [ ] Zero regressions in `06-structs.loft` / `20-binary.loft` /
      other legacy `Parts::Short` consumers.
- [ ] `lib/graphics/src/glb.loft` revert succeeds post-Phase-5+4b;
      `test_map_export_glb_header` green.

---

## Rollback

Each of the seven steps is additive.  Revert order (if needed
mid-land):

1. Revert Step 7 (`vector_narrow_width` gate closes again) — u16
   vectors immediately fall back to wide behaviour.  Everything
   else stays correct.
2. Revert Steps 4-6 (fill_database + parser dispatch) — narrow 2-byte
   vectors no longer register; opcodes unused.
3. Revert Steps 1-3 (Parts::ShortRaw + Store methods + io arms) —
   new variant stops existing.

A full revert is mechanical because no Phase 2/3/4a code needs to
change in response.  Zero existing consumers depend on `ShortRaw`.

---

## Sequencing vs. Phase 5

Phase 5 (narrow vector registration at local-var / param / return
sites) uses the SAME registration helper that `fill_database`
invokes.  Phase 5 implementation should route through a
`Data::narrow_vector_content(&Type) -> Option<u16>` helper that is
CALLED by `fill_database` (existing site) AND by Phase 5's new
sites.  The helper's match on `forced.get()` is the single point of
truth that Phase 4b updates to include `2 => Some(database.short_raw(...))`.

**Phase 5 before 4b** is the recommended order:

1. Phase 5 extracts the helper + migrates the ~6 call sites.  u8 /
   i32 narrow storage works for locals / params / returns.
2. Phase 4b adds `Parts::ShortRaw` and flips the `2` arm in the
   helper.  u16 / i16 narrow storage works everywhere in one shot.
3. No intermediate state where u16 vectors are half-narrowed.

If Phase 4b lands BEFORE Phase 5, u16 fields narrow but u16 locals
don't — the same split-brain Phase 5 is designed to close.  That's
working but inconsistent; locals' wide storage would still surprise
users trying to restore the `glb_idx_buf` natural form with a
`vector<u16>` return type (not a realistic use case, but
principle-of-least-surprise).

---

## What Phase 4b explicitly does NOT do

- Replace or remove `Parts::Short`.  It stays for struct fields
  that rely on the null-sentinel encoding.
- Change any existing `OpGetShort` / `OpSetShort` opcode behaviour.
- Touch `vector_add`'s raw-byte-copy path.
- Migrate existing `vector<u16>`-using programs to the narrow form
  automatically.  The form IS narrow post-4b — user code doesn't
  change, just storage density improves.
