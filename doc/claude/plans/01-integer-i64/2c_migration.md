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
alias update, and 8 × `OpConstInt` codegen emission sites.

Applying 2c.diff alone **breaks the build** — downstream callers
across ~10 files pass `as i32` arguments to `set_int` (now takes
`i64`) and cast `get_int` return values (now returns `i64`).  A
sed-style sweep closes that gap.

## Artifacts

- `doc/claude/plans/01-integer-i64/2c_migration.md` — this file
- `doc/claude/plans/01-integer-i64/2c.diff` — unified diff (547
  lines) covering 6 files

## Apply workflow

### Step 1 — apply 2c.diff (starting patch)

```bash
cd /home/ubuntu/loft
patch --dry-run -p1 < doc/claude/plans/01-integer-i64/2c.diff  # should "check"
patch -p1 < doc/claude/plans/01-integer-i64/2c.diff             # apply
```

After apply, `cargo build --release` reports ~52 mismatched-type
errors in callers of `set_int` / `get_int` that use the old i32
signatures.

### Step 2 — downstream sweep (sed-style)

Close the `set_int(… as i32)` and `get_int() as i32` calls by
widening to `as i64`.  Files affected (counts per initial grep):

| File | `set_int(… as i32)` calls |
|---|---|
| `src/database/allocation.rs` | 9 |
| `src/codegen_runtime.rs` | 12 |
| `src/native.rs` | 20 |
| `src/hash.rs` | 5 |
| `src/radix_tree.rs` | 2 |
| `src/compile.rs` | 1 |
| `src/extensions.rs` | 1 |
| `src/png_store.rs` | 1 |
| `src/store.rs` | 1 |

Sed sweep for each file:

```bash
# Replace `as i32)` in set_int arguments with `as i64)`.
# NB: narrow pattern so it only matches set_int contexts.
for f in src/database/allocation.rs src/codegen_runtime.rs \
         src/native.rs src/hash.rs src/radix_tree.rs \
         src/compile.rs src/extensions.rs src/png_store.rs \
         src/store.rs ; do
  sed -i 's/\(set_int([^)]*\) as i32)/\1 as i64)/g' "$f"
done
```

Also check for `get_int(...) as i32` sites that need adjustment (the
return type is now i64).  Grep:

```bash
grep -rn "get_int([^)]*)[[:space:]]*as i32" src/
```

Update each site to drop the `as i32` (i64 is the canonical type now)
or keep it as `as i32` if the surrounding code needs a narrow value
(rare — usually not).

### Step 3 — regen fill.rs

```bash
cargo test --test issues regen_fill_rs -- --ignored --nocapture
```

This refreshes `src/fill.rs` with handlers matching the new
`i64`-based `Type::Integer` rust_type.  Without regen, build continues
to fail on the stale `get_stack::<i32>()` calls for int handlers.

### Step 4 — build

```bash
cargo build --release
```

May surface additional `i32::MIN` / `i32::MAX` references that need
widening to `i64::MIN` / `i64::MAX`.  Fix iteratively.

### Step 5 — targeted probes

```bash
./target/release/loft --tests doc/claude/plans/01-integer-i64/probes/probe_08_nullcoalesce_on_arithmetic.loft
./target/release/loft --tests doc/claude/plans/01-integer-i64/probes/probe_11_g_hybrid_all_operators.loft
./target/release/loft --tests doc/claude/plans/01-integer-i64/probes/probe_12_u32_boundaries.loft
```

Probe 00 (baseline overflow at i32::MAX) may need an update — i64 has
room, so `(i32::MAX + 1)` no longer traps.  Adjust to `i64::MAX` for
the same invariant.

### Step 6 — full suite

```bash
cargo fmt -- --check
cargo clippy --release --all-targets -- -D warnings
./scripts/find_problems.sh --bg && ./scripts/find_problems.sh --wait
cat /tmp/loft_problems.txt  # expect "(none)"
```

## Rollback

```bash
patch -R -p1 < doc/claude/plans/01-integer-i64/2c.diff
# Undo the sed sweep:
git checkout HEAD -- src/database/allocation.rs src/codegen_runtime.rs \
  src/native.rs src/hash.rs src/radix_tree.rs \
  src/compile.rs src/extensions.rs src/png_store.rs src/store.rs
cargo test --test issues regen_fill_rs -- --ignored --nocapture
cargo build --release
```

## What 2c.diff covers

The diff includes these hunks (verified clean-applying via
`patch --dry-run -p1`):

| File | Hunk(s) | Purpose |
|---|---|---|
| `default/01_code.loft` | 1 | `pub type integer size(4)` → `size(8)` |
| `src/data.rs` | 2 | `element_size()` + `rust_type()` widen `Type::Integer` to 8-byte / `i64` |
| `src/variables/mod.rs` | 1 | `size()` widens `Type::Integer` |
| `src/ops.rs` | ~18 | All arithmetic / unary / bitwise / conversion `_int` functions widen to `i64`; bodies forward to `_long`.  In-file tests update `i32::MAX/MIN` → `i64::MAX/MIN`; panic expectations change to `"long overflow"`. |
| `src/state/codegen.rs` | 7 | `OpConstInt` emission sites widen payload from `i32` to `i64`; `i32::MIN` sentinels → `i64::MIN` |
| `src/store.rs` | 1 | `get_int` / `set_int` widen to `i64` |

## What 2c.diff does NOT cover

### Downstream call sites (~52 total)

`set_int(… as i32)` callers outside the 6 covered files — Step 2's
sed sweep closes these.

### Narrow storage accessors — optional

`get_short`, `set_short`, `get_byte`, `set_byte` in `src/store.rs`
keep their current `i32` signatures for now.  If needed for Phase 2c
completeness, widen them too with a supplementary diff.

### Native FFI entry points

`src/native.rs` functions expose integer-returning / integer-taking
entry points to host code.  Those signatures may need deliberate
widening or explicit narrowing at the boundary.

### Test-expectation updates

Tests that assert specific `i32::MAX`-level overflow behaviour
(`p180_int_widens_to_long_field`, `p54_as_long_*`, etc.) may need
review after the build passes.

### `.loftc` cache invalidation

Handled automatically by `CARGO_PKG_VERSION`.  Ensure the version
bumps on the same PR if shipped.

### `--migrate-i64` persisted-database tool

Out of scope for 2c.diff; separate Phase-follow-up.

## Commit shape post-apply

Recommended single commit covering diff + sed sweep + regen:

```
fix(integer-i64): Phase 2c — widen unbounded `integer` to i64 backing

Applies doc/claude/plans/01-integer-i64/2c.diff + downstream `as i32`
→ `as i64` sweep + fill.rs regen.

- Type::Integer default stack / field size: 4 → 8 bytes.
- ops.rs: ~18 op_*_int functions forward to op_*_long (i64 semantics).
- store.rs: get_int/set_int widen to i64.
- default/01_code.loft: `integer size(4)` → `size(8)`.
- codegen.rs: 7 × OpConstInt emission sites widen to 8-byte payload.
- Downstream callers across 9 files sed-updated `as i32` → `as i64`.
- src/fill.rs: regenerated.

User-visible: unbounded `integer` has i64 headroom.  Phase 1 G-hybrid
(overflow trap + `??`-discharge) still holds on the i64 level.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
```

## Non-goals

- Applying the diff automatically — kept as reviewable artifact.
- `Value::Int(i32)` → `Value::Int(i64)` enum change (287 call sites).
- Persisted-DB migration tool (`--migrate-i64`).
- Phase 2d / 2e / 2g / 2h — follow-up after 2c lands.
