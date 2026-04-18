# Phase 2 — C54.A: widen `integer` to i64 + range-packed storage

Status: **not started** — blocked by Phase 1.

## Context

After Phase 1, overflow / div-zero no longer silently wrong-result.
Phase 2's remaining value is **headroom** — microsecond timestamps,
accumulating checksums, bitmasks, file offsets — not safety.  This is
the largest-blast-radius phase in the initiative: opcode replumb,
`.loftc` bump, persisted-DB migration.

## Scope

### Runtime

- `Op*Int` family in `src/fill.rs` — registers become i64 throughout.
- `get_int` / `set_int` in `src/store.rs` — widen on load, narrow on
  store with existing `limit` range check.
- `Type::size()` default arm `4 → 8` for unbounded `integer`.
- Bounded integer fields (`limit(...)`, `i8`, `u8`, `i16`, `u16`)
  retain current storage width.

### Bytecode constants

Per QUALITY.md § 421-432, opcodes stay width-graded by magnitude:

| Opcode | Stream bytes | Range |
|---|---|---|
| `OpConstTiny` | 1 | −128 ..= 127 |
| `OpConstShort` | 2 | −32 768 ..= 32 767 |
| `OpConstInt` | 4 | −2³¹ ..= 2³¹ − 1 |
| `OpConstLong` | 8 | full i64 |

All sign-extend into i64 registers.  Common-case (`x = 0`, loop
bounds, indices) store 1 byte after opcode — ~50 % bytecode-size saving
on integer-heavy programs.

### Cache format

- Bump `.loftc` version.  Old caches fail-closed with a clear diagnostic
  pointing at the migrator.
- Add schema version field if not already present.

### Migration tool

`loft --migrate-i64 <dbfile>`:
- Reads old-format persisted DB.
- Rewrites `integer` columns to 8-byte storage.
- Preserves `i8` / `u8` / `i16` / `u16` / `limit(...)` fields verbatim.
- Fails loudly on format mismatch; no silent data loss.

### Backends

- Interpreter: direct — swap registers to i64, update `get_int` /
  `set_int`.
- Native (`src/generation/`): emit i64 arithmetic; revalidate overflow
  trap (Phase 1) still fires correctly on wider registers.
- WASM: i64 is a first-class type in wasm32-wasip2 — should be a
  near-trivial port.  Verify.

## Test plan (from QUALITY.md § 434-440)

Un-ignore:
- `c54_i32_min_round_trip` — the canary test; explicit `-2_147_483_648
  * 1` now returns the same value (no more sentinel).
- `c54_arithmetic_at_boundary`
- `c54_bounded_storage_preserved`
- `c54_unbounded_storage_widens`
- `c54_u8_times_u8_no_overflow`
- `c54_loftc_cache_invalidated`
- `c54_migration_tool_roundtrip`
- `c54a_const_tiny_used_for_small_literals`

Plus cross-backend parity tests (new): interpreter vs native vs WASM
on a battery of boundary values (MIN / MIN+1 / -1 / 0 / 1 / MAX-1 /
MAX).

## Risk mitigation

The `.loftc` bump + migration tool are the biggest risk.  Sub-plan
`02a-migration-tool-design.md` opens if the tool requires more than a
~200-line implementation.  Migration tool ships on the SAME commit as
the format bump — no interim state.

## Budget

**480-720 minutes** across the phase.  If the opcode replumb in
`src/fill.rs` surfaces parity bugs, open
`02b-fill-rs-i64-parity.md` as a sub-phase.

## Deliverables

- All `c54_*` (non-G) tests un-ignored.
- Migration tool committed with round-trip fixture.
- `.loftc` version bumped.
- PROBLEMS.md / QUALITY.md entry updated.
