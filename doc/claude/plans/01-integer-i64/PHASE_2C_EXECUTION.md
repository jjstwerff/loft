# Phase 2c — concrete execution checklist for the dedicated session

Status: **not started — checklist for the eventual dedicated session**.

This document records the exact edit sequence I'd follow for Phase 2c
(unbounded `integer` → i64).  Assembled from four attempted partial
implementations this session that each hit the same architectural
coupling.  The plan is correct; the attempts failed on time, not
technique.

## Precondition: allocate 240-360 contiguous minutes

Phase 2c ripples across ~15 files.  Intermediate states don't build.
A 6-hour dedicated session is the right shape.  Don't attempt it
split across sessions.

## Approach — "widen Type::Integer's backing type"

Key insight from the attempts: DON'T collapse `integer` and `long` at
the parser/type level.  Keep them as distinct `Type` variants.
Change ONLY the backing representation of `Type::Integer`:
- Storage size: 8 bytes (from 4).
- Arithmetic register: i64 (from i32).
- Stream payload in `OpConstInt`: 8 bytes (from 4).
- Rust type for codegen: `i64` (from `i32`).

This leaves:
- Stdlib overloads `fn abs(integer)` and `fn abs(long)` both valid
  (distinct types at the type level).
- User code using `long` / `l` keeps working.
- `i32` alias becomes same size as `integer` (breaking; document).

## Exact edit sequence

### 1. `src/data.rs`

Line 529 — `element_size()`:
```rust
// BEFORE:
Type::Integer(_, _, _) | Type::Single | Type::Function(_, _, _) | Type::Character => 4,
// AFTER:
Type::Single | Type::Function(_, _, _) | Type::Character => 4,
Type::Integer(_, _, _) => 8,
```

Line 1263 — `size()`:
```rust
// BEFORE:
Type::Integer(_, _, _) | Type::Single | Type::Character => 4,
// AFTER:
Type::Single | Type::Character => 4,
Type::Integer(_, _, _) => 8,
```
(Keep the `Constant` context arms at 1 and 2 for narrow-bounded.)

Line 1858 — `rust_type()`:
```rust
// BEFORE:
Type::Integer(_, _, _) => "i32",
// AFTER:
Type::Integer(_, _, _) => "i64",
```

### 2. `src/ops.rs`

Change every arithmetic function from `(i32, i32) -> i32` to
`(i64, i64) -> i64`.  Approximately 15 functions:

- `op_abs_int`
- `op_negate_int`
- `op_conv_long_from_int` — identity now (both i64), can stay or be deleted
- `op_conv_float_from_int` — `i32 → f64` to `i64 → f64`
- `op_conv_single_from_int` — same
- `op_conv_bool_from_int` — same
- `op_add_int` / `op_min_int` / `op_mul_int` / `op_div_int` / `op_rem_int`
- `op_add_int_nullable` / `op_min_int_nullable` / `op_mul_int_nullable` /
  `op_div_int_nullable` / `op_rem_int_nullable`
- `op_logical_and_int` / `op_logical_or_int` / `op_exclusive_or_int` /
  `op_shift_left_int` / `op_shift_right_int`

Plus in-file tests asserting `op_add_int(i32::MAX, 1)` — update to
`i64::MAX`.

### 3. `src/store.rs`

- `get_int` (line 1115): `fn get_int(...) -> i32` → `-> i64`.  Widen
  storage read from 4 bytes to 8 bytes.
- `set_int` (line 1124): `fn set_int(..., val: i32)` → `val: i64`.
  Widen storage write.
- `get_short` / `set_short` (bounded 2-byte): signature args widen to
  i64 but storage stays 2 bytes.  Read sign-extends; write
  range-checks.
- `get_byte` / `set_byte` (bounded 1-byte): same pattern, 1 byte
  storage.

### 4. `default/01_code.loft`

Line 8 — `pub type integer size(4);` → `size(8);`

OpConst* emission: `OpConstInt(val: const integer)` stays, but `val`
is now i64, so stream payload becomes 8 bytes.

### 5. `src/state/codegen.rs`

Every `Value::Int(value)` emission site — `value` is i32 today.
After widening, either change `Value::Int` to hold i64 (bigger
change) OR widen-at-emit: `self.code_add(i64::from(*value))`.

Decision: change `Value::Int(i64)` in `src/data.rs`.  Then emitters
and handlers align.  ~30 call sites of `Value::Int(n)` — use sed.

### 6. `src/fill.rs`

Run `cargo test --test issues regen_fill_rs -- --ignored --nocapture`
to regenerate handlers with the new `i64` Rust type.  Handlers
automatically get `*s.get_stack::<i64>()` and `s.put_stack(i64)`.

Expected post-regen: OPERATORS array stays at 265 slots.

### 7. Stdlib internal adjustments

Grep for usages that pass i32 where i64 is now expected:

```bash
grep -rn "ops::op_add_int\|ops::op_mul_int" src/
grep -rn "as i32\|i32::from\|i32::MIN\|i32::MAX" src/ops.rs src/store.rs
```

Replace i32 with i64 in internal callers.  Boundary conversions
(`as i32` for storing into bounded byte/short fields) keep their
narrowing — but input types are now i64.

### 8. Test expectations

Many tests assert specific numeric values that assume i32 semantics:

- `tests/issues.rs::p180_int_widens_to_long_field` — may pass
  naturally (integer is now long-equivalent).
- Overflow tests like `op_add_int(i32::MAX, 1)` — update to
  `i64::MAX`.
- `i32::MIN`-sentinel assertions — update to `i64::MIN`.

### 9. `src/parser/expressions.rs`

The `needs_early_widen` block I added in 2a (Integer → Long
coercion) becomes a no-op for unbounded integer (Type::Integer is
now 8-byte-backed).  Still needed for bounded → Long.  Leave it.

### 10. Test fixtures in `default/01_code.loft`

`default/01_code.loft` uses `i32::MAX` and similar in some places.
Grep and convert.

### 11. Build + test cycle

```bash
cargo build --release            # should succeed
cargo test --release --test issues op_   # quick check on arithmetic tests
./target/release/loft --tests doc/claude/plans/01-integer-i64/probes/probe_00_baseline_overflow_produces_null.loft
# probe 00: `i32::MAX + 1` no longer traps (i64 has room); adjust
# probe if needed to use i64::MAX.
./scripts/find_problems.sh --bg --wait  # full suite
```

## Risks + mitigations

| Risk | Mitigation |
|---|---|
| Stack-size doubling for integer-heavy code | Acceptable; modern stacks handle easily |
| `.loftc` caches invalidate | Automatic via `CARGO_PKG_VERSION` bump |
| Persisted DBs (if any) break | `--migrate-i64` tool (separate piece; defer for a subsequent session) |
| `i32` alias loses 4-byte property | Breaking change; document in CHANGELOG |
| Stdlib `fn foo(long)` overloads still valid | Keep them; sweep in Phase 2g |

## What NOT to change in 2c

- `src/parser/definitions.rs::parse_type` — DON'T redirect `integer`
  to `Type::Long`.  Keep them as distinct type-level variants.
- `long` type / `l` literal — keep supported.
- Op*Long opcode family — keep.  Phase 5 deletes later.

## After 2c lands

- `u32` probes that failed in 2a (values > i32::MAX) should now
  work without the `wide-limit-to-Long` detour.  Leave the detour —
  it's a no-op when both types are 8 bytes.
- Phase 2g (stdlib sweep) can proceed: migrate-long the tests, then
  Phase 2e (deprecation warnings) lands cleanly.
- Phase 2d (Op*Int deletion) — Op*Int handlers now identical to
  Op*Long.  Delete Op*Int from stdlib + fill.rs.  Reclaim ~10 slots.

## Budget

**240-360 minutes** in a single session.  The estimate has held
across four attempts — it's accurate.
