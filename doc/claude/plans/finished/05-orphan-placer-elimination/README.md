<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN05 — Orphan-placer elimination

## Status — 2026-04-23: closed

Reference for the SHIPPED slot-assignment behaviour lives in
[`../../../SLOTS.md`](../../../SLOTS.md) (§ "Cross-scope `Set` +
Insert-rooted bodies" line ~79, § "Plan-04 / @PLAN05 status (closed)"
line ~141, § P185 entry line ~219, summary line ~242).  This file is
a closure record only.

**Landed:** Phases 1a + 1b + 2 + 2c.

| Commit | Phase | Change |
|---|---|---|
| `e0a020f` | 1a | `process_scope` handles `Value::Insert` at function-body root |
| `494e5c7` | 1b | Exhaustive IR traversal + cross-scope `Set` handling |
| `309e0f4` | 2 | `place_orphaned_vars` deleted (~150 LOC retired) |
| `f74f78c` | 2c | P185 tests un-ignored; marked fixed in `PROBLEMS.md` |

**Dropped:** Phase 2b (invariant **I8 — orphan-iterator-alias** in
`validate.rs`).  With `place_orphaned_vars` gone, the bug class I8
would catch is structurally prevented — defensive invariant with no
driving bug doesn't pay its complexity cost.  Revisit only if a
future slot-reuse aliasing regression surfaces.

## Goal (achieved)

Delete `src/variables/slots.rs::place_orphaned_vars` by extending V1's
main walk to reach every variable.  Eliminate the bug class
structurally rather than continuing to point-fix the orphan placer
(P178 was already a point fix; P185 was the next).

## Realised value

Orphan-probe count over the cleanup arc:
- Pre-Phase 1a: 6 shapes surfaced
- Post-Phase 1a: 4 shapes
- Post-Phase 1b: 0 shapes
- Post-Phase 2: catch-net retired

P178 + P185 closed in the same arc.  No regressions on the
`tests/slot_v2_baseline.rs` fixtures.

## Sibling plans

- [`../04-slot-assignment-redesign/`](../04-slot-assignment-redesign) —
  parent plan that scoped V2 (retracted as replacement for V1) and
  produced the SPEC.md / walkthroughs.md / 00a-audit.md design
  archive @PLAN05 leaned on.
- The invariant set I1–I6 from V2 survives in `src/variables/validate.rs`
  and runs against V1's output as a shadow validator.

## See also

- [`../../../SLOTS.md`](../../../SLOTS.md) — slot-assignment reference
  (algorithm, two-zone design, diagnostic tools, @PLAN04/05 closure
  notes, the per-fixture status table)
- [`../../../PROBLEMS.md`](../../../PROBLEMS.md) — P185 row (closed
  by this plan)
- `src/variables/slots.rs::process_scope` / `place_large_and_recurse` —
  the extended main-walk site
- `src/variables/validate.rs` — invariant home (I1–I6 active; I8
  scoped + dropped)
