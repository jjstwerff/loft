# Phase 2c migration — diff-driven execution

## Context

C54 Phase 2c widens unbounded `integer` from 4-byte i32 storage + i32
arithmetic to 8-byte i64 storage + i64 arithmetic.  Bounded narrow
variants (`u8` / `u16` / `i8` / `i16` / `integer limit(lo, hi)` inside
i32 range) keep their packed storage widths.

The companion `2c.diff` is the **starting patch**: it covers the
type-system widening (`data.rs`, `variables/mod.rs`), the int
arithmetic / conversion / bitwise functions in `src/ops.rs`, the
`get_int`/`set_int` accessors in `src/store.rs`, the stdlib type
alias update, and 7 × `OpConstInt` codegen emission sites.

Applying 2c.diff alone **breaks the build with ~130 errors** across
21 files.  A significant subset is **NOT mechanical sweep-work** — see
the "Architectural blocker" section below before starting.

## Artifacts

- `doc/claude/plans/01-integer-i64/2c_migration.md` — this file
- `doc/claude/plans/01-integer-i64/2c.diff` — unified diff (611
  lines) covering 6 files, 22 hunks
- `doc/claude/plans/01-integer-i64/probes/` — 3 probes to run after
  build succeeds (probe_08, probe_11, probe_12)

## Architectural blocker — dual use of `set_int` / `get_int`

Before the diff can safely apply, resolve this first.

`set_int` / `get_int` in `src/store.rs` currently serve **two
semantically different roles** in the code base:

1. **User-level integer field read/write** — e.g. `x: integer` in a
   loft struct.  The field's width must track `Type::Integer`.  After
   Phase 2c this is **8 bytes / i64**.
2. **Internal 4-byte word storage** in collection headers — vector
   header `[rec:u32][len:u32]`, hash buckets, tree nodes, sorted-index
   offsets.  These are raw u32/i32 words that are **NOT** loft
   `integer` fields and must stay 4 bytes to preserve collection
   layout.

Widening `set_int` to i64 **corrupts role (2)**.  Example: in
`src/vector.rs:42`, `store.set_int(db.rec, db.pos, vec_rec as i32)`
writes 4 bytes at `db.pos+0` (the vector's "rec" slot).  If `set_int`
now writes 8 bytes, it **overwrites the length word at `db.pos+4`**,
destroying the vector.

### Resolution path

Add **new 4-byte raw accessors** to `src/store.rs` and migrate all
role-(2) callers to them, **before** widening `set_int`:

```rust
#[inline]
pub fn get_u32_raw(&self, rec: u32, fld: u32) -> u32 { ... }

#[inline]
pub fn set_u32_raw(&mut self, rec: u32, fld: u32, val: u32) -> bool { ... }

#[inline]
pub fn get_i32_raw(&self, rec: u32, fld: u32) -> i32 { ... }

#[inline]
pub fn set_i32_raw(&mut self, rec: u32, fld: u32, val: i32) -> bool { ... }
```

(Names TBD — `set_word` / `set_slot` / `set_raw32` are alternatives.)

### Files with role-(2) call sites (internal collection code)

| File | Sites | Nature |
|---|---|---|
| `src/store.rs` | **2** | `set_str` / `set_str_ptr` length writes at offset 4 — **inside the diff's own target file** |
| `src/vector.rs` | 23 | vector header `[rec:u32][len:u32]`, slice offsets, resize |
| `src/tree.rs` | 9 | B-tree node pointers, index counts |
| `src/hash.rs` | 5 (+more) | hash bucket refs, collision chains |
| `src/radix_tree.rs` | 8 | radix-tree node ptrs, branch indices |
| `src/database/allocation.rs` | 1+ | inter-store reference swap (role-1 mixed with role-2) |
| `src/database/structures.rs` | 7 | record-header metadata |
| `src/database/format.rs` | 5 | type-tag writes (`i32::from(tp)`) |
| `src/database/io.rs` | 9 | persisted-DB record walk |

#### Critical finding from inspection — `set_str` / `set_str_ptr`

During trial apply + build, **2 compile errors surface inside
`src/store.rs` itself** at lines 1230 and 1244:

```rust
// src/store.rs:1228
pub fn set_str(&mut self, val: &str) -> u32 {
    let res = self.claim(((val.len() + 15) / 8) as u32);
    self.set_int(res, 4, val.len() as i32);  // line 1230 — breaks
    // ...
}

pub fn set_str_ptr(&mut self, ptr: *const u8, len: usize) -> u32 {
    let res = self.claim(((len + 15) / 8) as u32);
    self.set_int(res, 4, len as i32);         // line 1244 — breaks
    // ...
}
```

String record layout: `[pad 0–3][len:u32 at 4–7][data from byte 8]`.
The mechanical fix (`as i32` → `as i64`) **compiles but silently
corrupts every string**: `set_int` now writes 8 bytes starting at
offset 4, which zero-fills offsets 8–11 (the first 4 bytes of the
string data).

The matching reads — `get_str:1215` (`let len = self.get_int(rec, 4)`)
and `append_str:1253` (`let prev = self.get_int(record, 4)`) — have
no compile errors but **read wrong lengths** post-2c: `get_int`
reads 8 bytes, pulling the first 4 bytes of string data into the
high 32 bits of the length.

This is **the single clearest reason** the role-1 sweep is not
mechanical.  The prerequisite raw accessors (Step 0) must ship
before 2c.diff can apply cleanly even to its own target file.

Fix shape (post-prerequisite):

```rust
self.set_u32_raw(res, 4, val.len() as u32);   // was set_int
let len = self.get_u32_raw(rec, 4);            // was get_int
```

Each site needs **manual review** to decide role-1 vs role-2 — a
mechanical sed will get it wrong.  Signals:

- `store.set_int(db.rec, db.pos, ...)` where `db` is a **vector /
  sorted / hash / index / tree header** → role-2 → `set_u32_raw`
- `store.set_int(db.rec, db.pos, ...)` where `db` points at a **user
  integer field** → role-1 → keep `set_int`, widen operand to i64
- `set_int(..., i32::from(tp))` where `tp` is a `Type` discriminant
  or format tag → role-2 (tag fits in i32) → `set_i32_raw` or
  `i64::from(tp)` (depending on where the tag is stored)
- `set_int(..., record_u32 as i32)` → role-2 → `set_u32_raw`
- `set_int(..., user_value)` where `user_value` originates from an
  `OpGetInt` or user expression → role-1

### Sequenced migration

1. **Prep diff (new)** — add `get_u32_raw` / `set_u32_raw` /
   `get_i32_raw` / `set_i32_raw` to `src/store.rs`.  No callers yet.
2. **Internal migration diff (new)** — each of the 8 files above
   replaces role-2 calls with the new accessors.  Build still green
   (old `set_int` is still 4-byte, new `set_u32_raw` is 4-byte, no
   width change yet).
3. **2c.diff (existing)** — now `set_int` safely widens to 8-byte,
   since no internal collection code calls it.
4. **Sweep diff (new, mechanical)** — the remaining role-1 callers
   in user-level code paths get `as i32` → `as i64`.
5. **Regen `fill.rs`** — runs on the now-consistent tree.
6. **Test tail** — expectations review.

Steps 1 + 2 are the **critical prerequisites**.  Without them, 2c.diff
leaves the build broken in ways that aren't sed-fixable and masks
layout corruption bugs if force-applied.

## Full list of what needs to change

This is the exhaustive list for a green build after Phase 2c lands.

### A. Store accessors (`src/store.rs`)

- [ ] **ADD** `get_u32_raw` / `set_u32_raw` (4-byte u32 raw).
- [ ] **ADD** `get_i32_raw` / `set_i32_raw` (4-byte i32 raw) — if any
      role-2 callers need signed.
- [ ] **WIDEN** `get_int` → `-> i64`, `set_int(val: i64)`.
- [ ] **CONSIDER** widening `get_short` / `set_short` / `get_byte` /
      `set_byte` to take/return `i64` at the boundary (packed storage
      unchanged).  Optional for minimal 2c; required if sweep hits
      them.

### B. Type system (`src/data.rs`, `src/variables/mod.rs`)

- [ ] `element_size(Type::Integer)` → 8.
- [ ] `rust_type(Type::Integer)` default → `"i64"`.
- [ ] `variables::size(Type::Integer)` → 8.
- [ ] Verify `Type::Integer(from, to, _)` narrow-variant dispatch in
      `rust_type` keeps i8/i16 for bounded ranges.

### C. Ops (`src/ops.rs`)

- [ ] Widen ~18 `op_*_int` functions to `(i64, i64) -> i64`; bodies
      forward to matching `op_*_long` (Phase 2b G-hybrid semantics
      already covers i64 overflow/null).
- [ ] Groups: arithmetic (5), nullable variants (5), unary (2),
      conversions to-int (4), conversions from-int (4), enum/char
      (3), bitwise (5), `format_int` (1).
- [ ] In-file tests: replace `i32::MAX/MIN` → `i64::MAX/MIN`; panic
      message `"integer overflow"` → `"long overflow"`.
- [ ] Remove `no_i64_sentinel_in_int_functions` test — invariant
      inverts.
- [ ] Macros `checked_int!`, `checked_int_nullable!`, `sentinel_int!`
      become unused — `#[allow(unused_macros)]` or delete.
- [ ] `src/ops/arith.rs` (2 sites) — check multiplication types (one
      error: `cannot multiply i64 by i32`).
- [ ] `src/internal_macros.rs` (3 sites) — verify macro expansion
      still typechecks with i64.

### D. Stdlib (`default/01_code.loft`)

- [ ] `pub type integer size(4)` → `pub type integer size(8)`.
- [ ] Verify no downstream `default/*.loft` relies on a 4-byte
      integer footprint (spot-check `default/02_images.loft`,
      `default/03_text.loft`).

### E. Codegen (`src/state/codegen.rs`)

- [ ] 7 × `OpConstInt` emission sites: `code_add(*value)` →
      `code_add(i64::from(*value))`.
- [ ] 3 × `i32::MIN` sentinel constants → `i64::MIN` at the affected
      sites.
- [ ] Check `OpGetInt` / `OpSetInt` opcode handler codegen — may
      need width-aware emit.

### F. Fill.rs (auto-generated)

- [ ] Run `cargo test --test issues regen_fill_rs -- --ignored
      --nocapture` **after** roles B + C + D + E all compile.
- [ ] Verify 26 stale `get_stack::<i32>()` / `push::<i32>()` for int
      handlers regenerate as `<i64>`.

### G. Execution-state callers

- [ ] `src/state/io.rs` (11 sites) — `file_ref != i32::MIN` → i64
      comparison; `set_int(r.rec, 4, i32::from(db_tp))` — decide
      role.
- [ ] `src/state/text.rs` (2 sites) — `self.set_string(len, ptr)`
      where `len` is now i64 but `set_string` takes i32 — add
      `len.try_into().unwrap()` **or** widen `set_string` to i64.
- [ ] `src/compile.rs` (2 sites) — `set_int(db.rec, 4,
      i32::from(vec_tp))` and `set_int(..., *v)` where `*v` is
      `Value::Int(i32)`; decide role and widen/narrow.
- [ ] `src/codegen_runtime.rs` (14 sites) — runtime `set_int` calls
      mixed role; review each.
- [ ] `src/extensions.rs` (1 site) — i32 cast in extension glue.
- [ ] `src/png_store.rs` (1 site) — PNG metadata i32 write.

### H. Vector / collection layer

- [ ] `src/vector.rs` (23 sites) — all role-2 header writes.
- [ ] `src/tree.rs` (9 sites) — B-tree node walks.
- [ ] `src/hash.rs` (5 sites, plus 1 error from initial build) —
      bucket chain walks.
- [ ] `src/radix_tree.rs` (8 sites) — radix node walks.
- [ ] `src/vec/mod.rs` (1 site) — dispatch glue.
- [ ] `src/iter/traits/iterator.rs` (1 site) — `Vec<i32>` cannot be
      built from i64 iterator: widen target type to `Vec<i64>` OR
      `.map(|v| v as i32).collect()`.

### I. Database layer

- [ ] `src/database/allocation.rs` (1+ site) — cross-store ref
      relocate `into.set_int(to.rec, to.pos, s_pos)` where `s_pos` is
      u32.
- [ ] `src/database/structures.rs` (7 sites) — record header writes.
- [ ] `src/database/format.rs` (5 sites) — type-tag writes (`Type`
      discriminants serialize as i32).
- [ ] `src/database/io.rs` (9 sites) — persisted-file record walk.
- [ ] `src/database/types.rs` (1 site).

### J. Native FFI (`src/native.rs`)

- [ ] 15 sites.  FFI boundary — each function exposes a Rust
      signature that either takes/returns loft `integer`.  Decide per
      call: widen to i64, or narrow at the boundary with
      `try_into().unwrap()` / explicit truncation.

### K. Test expectations

- [ ] `tests/parse_errors.rs` — line-number expectations for
      `01_code.loft:NNN:CC` error messages shift if the type-alias
      comment changes line count.  Not expected for size(4)→size(8)
      (same line count), but verify.
- [ ] `tests/issues.rs` — same scan.
- [ ] Overflow tests — any `#[should_panic(expected = "integer
      overflow")]` around user-level `integer` operations → `"long
      overflow"`.
- [ ] `p180_int_widens_to_long_field` — semantics change: `integer`
      is now i64, so widening to `long` is identity.  Test may be
      redundant or need a `long -> bigger-long` alternative.
- [ ] `p54_as_long_*` family — same.
- [ ] Probe 00 (baseline overflow at `i32::MAX`) — adjust to
      `i64::MAX` for the same invariant.
- [ ] Probe 11, 12 — should still pass (they already target
      bitwise / u32 boundaries / G-hybrid).

### L. `.loftc` cache

- [ ] Handled automatically by `CARGO_PKG_VERSION` key in cache
      header.  Ensure the version bumps on the release PR (likely
      0.8.3 → 0.9.0 given the breaking change).

### M. Persisted-database migration

- [ ] `--migrate-long` CLI (already shipped in Phase 2f) rewrites
      on-disk databases that stored `integer` as 4-byte u32 into
      8-byte i64.  **Users with persisted DBs MUST run this before
      loading under the new binary.**
- [ ] Document in `CHANGELOG.md` with explicit migration command.
- [ ] Verify migration covers vector / sorted / index / hash /
      radix header fields (role-2) are NOT mass-rewritten — only
      true user integer columns.
- [ ] Add a sanity test: create a small DB on the old binary, run
      `--migrate-long`, open on new binary, verify round-trip.

## Apply workflow (revised)

### Step 0 — prerequisite diffs (NOT included in 2c.diff)

```bash
# Diff 1: add get_u32_raw / set_u32_raw / get_i32_raw / set_i32_raw
#   to src/store.rs.  No callers.  Build stays green.

# Diff 2: migrate all role-2 callers in vector.rs / tree.rs /
#   hash.rs / radix_tree.rs / database/*.rs to the new raw
#   accessors.  Build stays green (set_int still 4-byte).
```

Commit these as separate PRs (or separate commits on the same
branch) **before** applying 2c.diff.

### Step 1 — apply 2c.diff (starting patch)

```bash
cd /home/ubuntu/loft
patch --dry-run -p1 < doc/claude/plans/01-integer-i64/2c.diff
patch -p1 < doc/claude/plans/01-integer-i64/2c.diff
```

After apply, `cargo build --release` reports ~30 mismatched-type
errors (down from 130 once Step 0 prerequisites are done).  These
are genuine **role-1 user-level** callers.

### Step 2 — downstream sweep (sed-style, role-1 only)

```bash
# Replace `as i32)` in set_int arguments with `as i64)`.
for f in src/codegen_runtime.rs src/native.rs src/compile.rs \
         src/extensions.rs src/png_store.rs src/state/io.rs \
         src/state/text.rs src/ops.rs ; do
  sed -i 's/\(set_int([^)]*\) as i32)/\1 as i64)/g' "$f"
done

# Drop `as i32` suffix on get_int() sites.
grep -rn "get_int([^)]*)[[:space:]]*as i32" src/
#   → none expected after the migration; if any, drop the cast.

# Sentinel updates (user-level role).
grep -rn "i32::MIN\|i32::MAX" src/ | grep -v _raw
#   → review each; widen to i64 where the semantic is "user integer
#     sentinel" and keep i32 for packed-storage / format-tag roles.
```

### Step 3 — regen fill.rs

```bash
cargo test --test issues regen_fill_rs -- --ignored --nocapture
```

### Step 4 — build

```bash
cargo build --release
cargo build --release --no-default-features   # wasm feature gate
```

May surface a handful of additional `i32::MIN/MAX` + `cannot multiply
i64 by i32` + `Vec<i32>` sites.  Fix iteratively.

### Step 5 — targeted probes

```bash
./target/release/loft --tests doc/claude/plans/01-integer-i64/probes/probe_08_nullcoalesce_on_arithmetic.loft
./target/release/loft --tests doc/claude/plans/01-integer-i64/probes/probe_11_g_hybrid_all_operators.loft
./target/release/loft --tests doc/claude/plans/01-integer-i64/probes/probe_12_u32_boundaries.loft
```

Probe 00 (baseline overflow at `i32::MAX`) needs an update — i64 has
room, so `(i32::MAX + 1)` no longer traps.  Adjust to `i64::MAX` for
the same invariant.

### Step 6 — full suite

```bash
cargo fmt -- --check
cargo clippy --release --all-targets -- -D warnings
cargo clippy --no-default-features --all-targets -- -D warnings
./scripts/find_problems.sh --bg && ./scripts/find_problems.sh --wait
cat /tmp/loft_problems.txt  # expect "(none)"
```

### Step 7 — persisted-DB smoke

```bash
# Create a small DB with the old binary (or a pre-2c checkout), then:
./target/release/loft --migrate-long path/to/db
./target/release/loft path/to/db          # verify opens cleanly
```

## Rollback

```bash
patch -R -p1 < doc/claude/plans/01-integer-i64/2c.diff
# Undo downstream sweep + prerequisite diffs:
git checkout HEAD -- src/
cargo test --test issues regen_fill_rs -- --ignored --nocapture
cargo build --release
```

## What 2c.diff covers

The diff includes these hunks (verified clean-applying via `patch
--dry-run -p1`):

| File | Hunk(s) | Purpose |
|---|---|---|
| `default/01_code.loft` | 1 | `pub type integer size(4)` → `size(8)` |
| `src/data.rs` | 2 | `element_size()` + `rust_type()` widen `Type::Integer` to 8-byte / `i64` |
| `src/variables/mod.rs` | 1 | `size()` widens `Type::Integer` |
| `src/ops.rs` | ~11 | All arithmetic / unary / bitwise / conversion `_int` functions widen to `i64`; bodies forward to `_long`.  In-file tests update `i32::MAX/MIN` → `i64::MAX/MIN`; panic expectations change to `"long overflow"`.  **Dead macros removed:** `checked_int!`, `checked_int_nullable!`, `sentinel_int!` — now unreferenced after the forward-to-`_long` migration. |
| `src/state/codegen.rs` | 6 | `OpConstInt` emission sites widen payload from `i32` to `i64`; `i32::MIN` sentinels → `i64::MIN` |
| `src/store.rs` | 1 | `get_int` / `set_int` widen to `i64` |

## What 2c.diff does NOT cover

### Prerequisite — internal raw accessors (Step 0 above)

The `get_u32_raw` / `set_u32_raw` split + role-2 migration across
8 files (vector, tree, hash, radix_tree, database/*).  Must land
**before** 2c.diff to avoid layout corruption.

### Why the role-1 sweep cannot be folded in mechanically

A sed sweep of `set_int(… as i32)` → `set_int(… as i64)` **appears**
to be safe mechanical work (~30–50 sites across `codegen_runtime.rs`,
`native.rs`, `compile.rs`, `extensions.rs`, `png_store.rs`,
`state/io.rs`, `state/text.rs`).  It is NOT.

Trial apply surfaced the full picture: of those ~50 sites, the
**vast majority** are role-2 in disguise:

| File | Sites | Role signal |
|---|---|---|
| `src/codegen_runtime.rs` | ~12 | All are vector-header / record-header writes (`set_int(vec_rec, 4, count)`, `set_int(header_rec, 4, vec_rec as i32)`) |
| `src/compile.rs` | 1 | `set_int(rec.rec, rec.pos, s_pos as i32)` writes a string-table offset (u32) into a struct field |
| `src/extensions.rs` | 1 | `set_int(header.rec, 4, r.rec as i32)` — vector header rec pointer |
| `src/native.rs` | ~10 | Stack-frame introspection, parallel-result packing — packed 4-byte element storage |
| `src/png_store.rs` | 1 | PNG record metadata — 4-byte u32 write |
| `src/state/io.rs` | ~8 | `file_ref == i32::MIN` comparisons — `File.ref` is explicitly `i32` in `default/02_images.loft:52`, so stays 4 bytes |
| `src/state/text.rs` | 1 | `get_int(rec, 4)` reads a 4-byte string length from the const store |

If a sed sweep widens these, the build compiles but **corrupts
memory at runtime** — e.g. `set_int(vec_rec, 4, count as i64)` writes
8 bytes at offset 4 of a vector header, overwriting element 0's
low bytes.

The role-1 sites that ARE mechanical are already in 2c.diff's
`ops.rs` hunks (the test updates and macro removals).  No other
purely-mechanical fixes exist.

### Downstream role-1 call sites after prerequisite

Once the internal raw-accessor migration (Step 0) moves the role-2
sites to `set_u32_raw` / `set_i32_raw`, the remaining genuine role-1
callers become visible — likely < 10 sites.  Close with sed at that
point:

```bash
for f in src/codegen_runtime.rs src/native.rs src/state/io.rs ; do
  sed -i 's/\(set_int([^)]*\) as i32)/\1 as i64)/g' "$f"
done
```

(Same sed as before, but safe only because role-2 sites are gone.)

### `cannot multiply i64 by i32` sites

`src/ops/arith.rs:2 sites` — int * i32-literal patterns need a
`.into()` or cast added.

### `Vec<i32>` collect target

`src/iter/traits/iterator.rs:1 site` — likely needs `Vec<i64>` or
`.map(|v| v as i32)`.

### Narrow storage accessors — semi-optional

`get_short` / `set_short` / `get_byte` / `set_byte` boundary types.
If the sweep hits call sites that pass i64 values into these, widen
the function signatures to take/return i64 (packed storage still
2-byte / 1-byte internally).

### Native FFI entry points

`src/native.rs` — 15 sites.  Each is a deliberate decision:
- Pure internal scaffolding → widen to i64 silently.
- Public FFI exposed to host code → document the new width.

### Test-expectation updates

- `tests/parse_errors.rs` / `tests/issues.rs` — line-number
  expectations re-sync if `01_code.loft` line count shifts (not
  expected but check).
- `p180_int_widens_to_long_field` — redundant post-2c (int == long).
- `p54_as_long_*` — same.
- Any `#[should_panic(expected = "integer overflow")]` in the user
  code path → change to `"long overflow"`.

### `.loftc` cache invalidation

Handled automatically by `CARGO_PKG_VERSION`.  Bump the version on
the release PR (0.8.3 → 0.9.0 likely, breaking change).

### `--migrate-long` persisted-database tool

Already exists (shipped in Phase 2f).  `CHANGELOG.md` must reference
the migration command explicitly when 2c ships.

## Commit shape post-apply

If the full stack lands as one PR, group commits as:

```
1. feat(store): add get_u32_raw / set_u32_raw (no callers)
2. refactor(collections): migrate vector/tree/hash/radix/database to
   u32_raw for header writes
3. feat(integer-i64): Phase 2c — widen unbounded `integer` to i64
   backing (applies 2c.diff + role-1 sweep + fill.rs regen)
4. test(integer-i64): update p180 / p54_as_long / probe_00 expectations
5. chore(release): bump CARGO_PKG_VERSION for the i64 integer break
```

Alternatively, each step as its own PR for reviewability.

## Non-goals

- Applying the diff automatically — kept as reviewable artifact.
- `Value::Int(i32)` → `Value::Int(i64)` enum change (287 call sites) —
  Phase 2c works around via `i64::from(*value)` at codegen emit sites.
- Full `--migrate-long` re-validation — covered by Phase 2f ship.
- Phase 2d / 2e / 2g / 2h — follow-up after 2c lands.

## Quick reference — error patterns → fix

Seen during trial apply (working tree reverted).  Use this as a
cheat-sheet during the sweep:

| Error | Example | Fix |
|---|---|---|
| `set_int(…, rec as i32)` role-2 | `store.set_int(db.rec, db.pos, vec_rec as i32)` | `store.set_u32_raw(db.rec, db.pos, vec_rec)` |
| `set_int(…, (len) as i32)` role-2 | `store.set_int(vec_rec, 4, new_length as i32)` | `store.set_u32_raw(vec_rec, 4, new_length)` |
| `set_int(…, i32::from(tp))` role-2 | `.set_int(db.rec, 4, i32::from(vec_tp))` | `.set_i32_raw(db.rec, 4, i32::from(vec_tp))` or convert tag to u32 |
| `set_int(…, user_expr as i32)` role-1 | last i32 sweep — mechanical | `… as i64` via sed |
| `file_ref != i32::MIN` | `src/state/io.rs:476` | `file_ref != i64::MIN` |
| `set_string(len, ptr)` | `src/state/text.rs:53` where `len: i64` | `set_string(len.try_into().unwrap(), ptr)` or widen `set_string` |
| `cannot multiply i64 by i32` | `src/ops/arith.rs` | `a * i64::from(b)` or `a * b.into()` |
| `Vec<i32>` from i64 iter | `src/iter/traits/iterator.rs` | widen target to `Vec<i64>` or `.map(\|v\| v as i32)` |
| `let _: i32 = get_int(...)` | any | drop explicit annotation; i64 infers |
| `i32::MIN` sentinel role-1 | any user-value sentinel | `i64::MIN` |
| `i32::MIN` / `i32::MAX` role-2 | packed-storage bounds | keep as i32 |
| `*pos < 8 + (length - 1) * i32::from(size)` | `src/vector.rs:457` | internal slice walk uses u32 arithmetic — confirm `*pos` type |
| Narrow storage `set_short(…, val: i32)` | any | decide: widen signature, or narrow at caller |
