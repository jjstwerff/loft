# Phase 6 — Spec: the integer arithmetic invariant + initiative close-out

Status: **not started** — final phase.

## Goal

Capture the landed C54 invariants in the user-facing documentation so
future work doesn't regress them.  Close out the initiative per the
`doc/claude/plans/` convention.

## The invariant statement

After C54 ships in full, loft guarantees:

> **"Integer arithmetic in loft never silently produces a wrong
> result."**
>
> - All arithmetic (`+`, `-`, `*`, `/`, `%`) operates on i64 registers.
> - Overflow (`i64::MAX + 1`, `i64::MIN - 1`), division by zero, and
>   modulo by zero produce a runtime error (C54.G) that reports file,
>   line, column, and the operator that tripped.
> - Storage widths are declaration-driven (`integer` = 8 bytes;
>   `i32` = 4 bytes; `integer limit(lo, hi)` sizes to fit).
> - Reads widen to i64; writes narrow with range checks.
> - `i64::MIN` is reserved as the null sentinel for unbounded integer
>   storage.  Users cannot accidentally produce this value via
>   arithmetic (it traps).  Explicit literal `-9_223_372_036_854_775_808`
>   is an error.

If Phase 0 picked G′ (unexpected), the trap semantics are replaced by
null-on-overflow + `??` composition.  Same invariant, different
mechanism.

## Doc edits

### `doc/claude/LOFT.md` — primitive types section

Find the table listing `integer` / `long` / `float` / `single` /
`character` / `boolean` / `text`.  Update:

- `integer` — **i64**; 8-byte storage for unbounded; narrowed
  storage for bounded (`limit(...)`).  Sentinel: `i64::MIN`.
- `i32` — explicit narrow alias: `integer size(4)`.  Arithmetic still
  widens to i64 in registers.
- `i8`, `u8`, `i16`, `u16`, `u32` — bounded, documented with
  byte-width + range.
- `long` — **deprecated** in 0.9.0, **removed** in 1.0.0.  Point at
  `--migrate-long`.

### `doc/claude/LOFT.md` — new section "Arithmetic safety"

Insert after the primitive-types section.  Covers:

1. The invariant statement above.
2. Overflow / div-zero behaviour (trap with good diagnostics under G;
   null-propagating under G′ if that was chosen).
3. Explicit `i32::MIN` / `i64::MIN` literal handling (reserved
   sentinel; literal is a compile-time error).
4. Cross-backend parity guarantee.
5. `??` composition (G′) — examples of arithmetic + fallback.  Omit
   under G.
6. Migration notes: 0.9.0 silently-wrong-result channel gone; 1.0.0
   `long` type removed.
7. Pointer to `--migrate-i64` + `--migrate-long` CLI tools.

### `doc/claude/CHANGELOG.md`

Under 0.9.0:

```
### Arithmetic safety — C54 initiative

- Integer arithmetic is now i64-wide.  Unbounded `integer` fields
  use 8-byte storage; `i32` is an explicit 4-byte alias.
- Overflow and divide-by-zero trap at runtime (or produce null under
  C54.G′) instead of silently returning `i32::MIN`.
- `u32` added as a stdlib type — closes the RGBA wrap trap.
- `long` type and `l` literal suffix deprecated.  Use `integer` /
  plain literals.  Run `loft --migrate-long <path>` to rewrite.
- Opcode budget reduced from 254/256 to ~228/256, unblocking future
  O1 superinstruction peephole work.
- `.loftc` bytecode cache format invalidated; recompiles
  automatically.
- Persisted-database migration: `loft --migrate-i64 <dbfile>`
  widens unbounded integer columns.

See `doc/claude/plans/finished/01-integer-i64/` for the full
initiative record.
```

Under 1.0.0 (future):

```
- `long` type and `l` literal suffix removed (hard error).  Users
  still on `long` code should run `loft --migrate-long` before
  upgrading.
```

### `doc/claude/PROBLEMS.md`

C54 was historically in `QUALITY.md` as active-sprint, not
`PROBLEMS.md`.  Add a closed entry:

```
| ~~C54~~ | Integer arithmetic silently produced wrong results on
`i32::MIN` / overflow / div-zero. | — | **Fixed 2026-XX** via the
01-integer-i64 initiative.  Phase 1 (G trap) + Phase 2 (i64 widen)
+ Phase 3 (u32) + Phase 4 (long deprecated) + Phase 5 (opcode
reclamation) landed in sequence.  See
`doc/claude/plans/finished/01-integer-i64/`. |
```

### `doc/claude/CAVEATS.md`

Find any entry referencing:
- `i32::MIN` sentinel behaviour
- silent overflow
- `long` as a workaround for large integers

Close each with a pointer to 0.9.0 / C54.

### `doc/claude/INCONSISTENCIES.md`

Close any entry about integer-width disparity between `integer` and
`long`, or about the i32::MIN sentinel being user-observable.

### `doc/claude/QUALITY.md`

- Strike the C54 entry.
- Strike the C54.A / B / C / E / G / G′ sub-entries.
- Add a pointer at the bottom: "C54 closed via initiative at
  `doc/claude/plans/finished/01-integer-i64/`."

### `doc/claude/RELEASE.md`

- Under "0.9.0 progress": note C54 complete.
- Under "1.0.0 blockers": add `long` final removal as a remaining
  item.

### `doc/claude/ROADMAP.md`

- 0.9.0 section: C54 complete — add detail line.
- 1.0.0 section: `long` removal.
- Deferred indefinitely: O1 superinstruction peephole now
  unblocked — upgrade to "available".

## Initiative close-out

Per the convention in `doc/claude/plans/README.md`:

```bash
git mv doc/claude/plans/01-integer-i64 doc/claude/plans/finished/01-integer-i64
```

Update `doc/claude/plans/README.md`:

- Remove `01-integer-i64` from "Current initiatives".
- Add to "Finished initiatives" with the close date and a one-line
  summary pointing at the doc landing places.

## Cross-reference updates

After the move, every reference to `doc/claude/plans/01-integer-i64/`
in the codebase (LIFETIME.md-style spec pointer, PROBLEMS.md,
CAVEATS.md, individual test comments referencing the initiative)
must be updated to `doc/claude/plans/finished/01-integer-i64/`.  Run:

```bash
grep -rn "plans/01-integer-i64" doc/ lib/ tests/ src/ | grep -v "plans/finished/"
```

Expect hits post-close; fix all of them in the same commit as the
directory move.

## Budget

**120-180 minutes** — documentation only.

## Deliverables

- LOFT.md type-reference section updated.
- LOFT.md new "Arithmetic safety" section.
- CHANGELOG.md 0.9.0 + 1.0.0 entries.
- PROBLEMS.md new closed entry.
- CAVEATS.md entries closed.
- INCONSISTENCIES.md entries closed.
- QUALITY.md C54 entries struck.
- RELEASE.md 0.9.0 progress updated.
- ROADMAP.md C54-related items updated.
- Initiative moved to `finished/`.
- All cross-references updated.

## Success criteria

1. A fresh user reading LOFT.md's "Arithmetic safety" section can
   predict the behaviour of `a + b` / `a / 0` / `x ?? default` on
   overflow.
2. A fresh contributor reading PROBLEMS.md sees C54 marked closed
   with a pointer to the finished initiative directory.
3. `grep -rn "plans/01-integer-i64" doc/ lib/ tests/ src/` returns
   only `plans/finished/` hits.
4. `RELEASE.md` 0.9.0 is clear on what changed for users.
