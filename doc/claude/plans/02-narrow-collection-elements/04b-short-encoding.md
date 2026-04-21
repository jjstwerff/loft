<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4b — Direct-encoded 2-byte storage for narrow vectors

**Status:** blocked — 2026-04-21 attempt reverted after regression
surfaced in `tests/native.rs::native_dir` (the `16-parser.loft`
native test ran into an infinite loop, >20 min CPU time).  Root
cause not yet identified; the plan's no-regression ground rule
forbids shipping this until the root cause is known and the
regression has a regression guard.

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

Hypotheses not yet verified:

1. **Opcode numbering collision** — the 7-step plan added
   `OpGetShortRaw` + `OpSetShortRaw` as new opcodes.  The parser
   test may have hit an opcode dispatch that the native codegen
   produces with a stale library index.  Check whether
   `tests/lib/native_pkg/native/target/release/libloft_native_test.so`
   or the parser's emitted Rust ended up embedding a now-shifted
   opcode number.
2. **Infinite loop in inference** — the parser test likely
   exercises `parse_type` for every alias it encounters; the new
   `vector_narrow_width` returning `Some(2)` may route some
   previously-unreachable branch of the resolver that loops.
3. **Hang in `OpAppendVector` for short-stride elements** — the
   raw-byte copy path in `vector_add` may misread `elem_size` for
   `Parts::ShortRaw` vs `Parts::Short` and produce a zero-length
   copy that the caller retries forever.

Next investigation step: revert Step 7 only (gate stays at
`{1, 4}`) and re-run `native_dir`.  If that passes, Steps 1-6
are safe and Step 7's opening of the gate is the direct trigger.
If native_dir still hangs, the problem is structural in the
Phase 2-5 pipeline interacting with the new variant — back out
further.

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

### Step 6 — Parser dispatch

`src/parser/mod.rs::get_val`:

```rust
if s == 1 {
    self.cl("OpGetByte", &[code, p, Value::Int(spec.min)])
} else if s == 2 {
    // P184 Phase 4b: narrow 2-byte vector element read.
    // NOT OpGetShort — that uses the legacy +1 encoding for
    // `Parts::Short`.  OpGetShortRaw matches the direct-encoded
    // `Parts::ShortRaw` that Phase 2 now registers for narrow
    // vector contents.
    self.cl("OpGetShortRaw", &[code, p, Value::Int(spec.min)])
} else if s == 4 {
    self.cl("OpGetInt4", &[code, p])
} else {
    self.cl("OpGetInt", &[code, p])
}
```

**Critical correctness constraint:** only vector-element reads
(`alias == u32::MAX` branch) should reach this `s == 2` case, and
only if `vector_narrow_width()` returns `Some(2)`.  Struct-field
reads with `alias != u32::MAX` still emit `OpGetShort` via the
captured-alias path — those continue using `Parts::Short` legacy
encoding on struct fields that opted into `size(2)`.

This is why the `get_val` three-way dispatch (kept from Phase 4a)
is load-bearing: it routes each call to the correct encoding.

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
