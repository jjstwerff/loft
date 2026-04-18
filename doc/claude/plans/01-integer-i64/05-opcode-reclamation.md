# Phase 5 — C54.E: reclaim 32-bit-arithmetic opcodes

Status: **not started** — blocked by Phase 2 and Phase 4.

## Context

After C54.A every `integer` slot is i64, and the `Op*Long` arithmetic
family becomes duplicate.  Per QUALITY.md § 464-474: delete
`OpAddLong`, `OpMulLong`, `OpEqLong`, … from `default/01_code.loft`
and `src/native.rs`'s registry.

Current opcode count: 254 / 256.  Reclaiming ~26 opcode slots unlocks
**O1 superinstruction peephole rewriting** (currently deferred
indefinitely in ROADMAP.md).  This is the knock-on prize that makes
this whole initiative pay for itself beyond the safety fix.

## Scope

- Enumerate all `Op*Long` arithmetic opcodes: `OpAddLong`, `OpSubLong`,
  `OpMulLong`, `OpDivLong`, `OpModLong`, `OpEqLong`, `OpLtLong`,
  `OpLeLong`, `OpGtLong`, `OpGeLong`, `OpNeqLong`, plus their null
  variants if any.  Target: ~26 slots.
- Delete from `default/01_code.loft` operator definitions.
- Delete from `src/native.rs` native function registry.
- Delete from `src/fill.rs` handler list.
- Ensure `src/parser/operators.rs` dispatches all integer arithmetic
  through `Op*Int` (now i64).
- Renumber remaining opcodes if needed to close the gap, or leave
  gaps and update the total.

**Keep**: the bytecode-constant family (`OpConstTiny` / `Short` / `Int`
/ `Long`).  Those are stream-payload-width optimisations, not
register-width-specific.

## Pre-flight check

Before deleting, grep every reference:

```bash
grep -rn "OpAddLong\|OpSubLong\|OpMulLong\|OpDivLong\|OpModLong\|OpEqLong" src/ default/ tests/ lib/
```

Any user-code reference means either the stdlib sweep (Phase 4)
missed something or a user program needs migration.

## Test plan (from QUALITY.md § 472-474)

Un-ignore:
- `c54e_opcode_budget_reclaimed` (counts the opcode table size +
  asserts the new budget).
- `c54e_long_arithmetic_still_works` (post-deletion, `long` / `l` code
  that went through migration still runs).
- `c54e_loftc_pre_c54_invalidated` (old caches referencing dropped
  opcodes fail cleanly).

## Risk

Opcode renumbering risks `.loftc` cache invalidation for every user.
Acceptable because Phase 2 already bumped the format version.

## Budget

**180-240 minutes** — mostly mechanical, but the renumbering +
cross-reference step is fiddly.

## Deliverables

- Opcodes deleted from every source of truth.
- Opcode count reduced (target 228 / 256 or thereabouts).
- Tests un-ignored.
- ROADMAP.md § Deferred indefinitely: O1 superinstruction peephole
  moved from "blocked on opcode budget" to "unblocked, available for
  prioritisation".
