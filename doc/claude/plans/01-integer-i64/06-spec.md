# Phase 6 — Spec: the new integer arithmetic invariant

Status: **not started** — final phase.

## Goal

Capture the landed C54 invariants in the user-facing documentation so
future work doesn't regress them.

## What to write

### `doc/claude/LOFT.md` — type reference

- `integer` is i64.  Arithmetic is always i64.
- `i32` remains as an alias for `integer size(4)` — narrower storage,
  same arithmetic width.
- `u8` / `u16` / `u32` — bounded unsigned types with explicit sentinel
  reservation.  Document the sentinel + `not null` escape hatch.
- `long` is removed (0.9.0 deprecated, 1.0.0 hard error).
- Overflow / div-zero semantics — point at the "Arithmetic safety"
  section.

### `doc/claude/LOFT.md` — new section "Arithmetic safety"

- The invariant: "arithmetic in loft never silently produces a wrong
  result."
- G semantics (if chosen) — traps on overflow / div-zero; composes
  with error handling.
- G′ semantics (if chosen) — produces null on overflow / div-zero;
  composes with `??` and `?? return`.
- Explicit `i32::MIN` literals are preserved verbatim.
- Cross-backend parity guarantee.

### `doc/claude/CHANGELOG.md`

- 0.9.0: C54 initiative complete; arithmetic is safe; `long`
  deprecated; `u32` added.
- 1.0.0: `long` removed.

### `doc/claude/PROBLEMS.md`

- C54 moved from QUALITY.md active-sprint to "closed" with a pointer
  at this initiative's finished directory.

### `doc/claude/CAVEATS.md`

- Update any caveats that referenced `i32::MIN` sentinel or silent
  overflow.  Most likely several entries go from "caveat" to "closed".

### `doc/claude/INCONSISTENCIES.md`

- Any entry that called out the integer sentinel gets closed.

## Initiative close-out

- Move `doc/claude/plans/01-integer-i64/` into
  `doc/claude/plans/finished/` per convention.
- Update `doc/claude/plans/README.md` current-initiatives table.
- Update QUALITY.md — strike the C54 entry, reference the finished
  initiative directory.

## Budget

**120-180 minutes** — documentation only.

## Deliverables

- LOFT.md + CHANGELOG.md + PROBLEMS.md + CAVEATS.md + INCONSISTENCIES.md
  edits.
- Initiative moved to `finished/`.
- QUALITY.md updated.
