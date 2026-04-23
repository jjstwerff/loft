<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 05 — Orphan-placer elimination

**Status:** planned.  Targeted follow-up after plan-04's retraction
(see [`../04-slot-assignment-redesign/README.md`](../04-slot-assignment-redesign/README.md)
§ Status).

**Goal:** delete `src/variables/slots.rs::place_orphaned_vars` by
extending V1's main walk to reach every variable, and add invariant
I8 (dep-chain-aware orphan-iterator-alias check) so P185's exact
failure shape is caught at compile time with a named panic instead
of a runtime SIGSEGV.  Un-ignore P185.

## Context

V1's main walk (`process_scope` + `place_large_and_recurse` in
`src/variables/slots.rs`) places variables whose declared scope
matches the Block/Loop node it's currently visiting.  Three IR
shapes currently fall through to `place_orphaned_vars`:

1. **Insert-rooted function bodies** — the IR root is
   `Value::Insert(...)`, not `Value::Block(...)`.  `process_scope`
   early-returns (line 74-76) and all locals become orphans.  P178
   shape.
2. **Parent-scope Set inside child-Block operators** — a variable
   whose `scope == parent` is Set inside a child's `operators` list.
   The parent's walk never enters the child Block's operators as
   parent-scope material; the child's walk skips it via the
   `v.scope == scope` filter (line 212).  P185 shape.
3. **Insert preambles with `__lift_N` / `__ref_N` / `__vdb_N`
   temporaries** — compiler-generated temps whose scope is the
   function body but which live inside an `Insert` node in a call
   argument position.

The orphan placer is a catch-all that places these by interval
colouring against already-placed variables.  Its bug history:
- P178 — `local_start = 0` overlapped the argument region; fixed with
  a `local_start` parameter floor.
- P185 — slot reuse missed that a live text accumulator holds a
  DbRef into the iterator-temp's store.  Still open.

Each patch to the orphan placer has been a point fix for a specific
shape.  Deleting it eliminates the bug class structurally.

## Approach

**Phase 0 — Characterize.**  Add fixtures for each orphan shape so
the main-walk extension has regression guards.  Share / reuse the
fixtures already in `tests/slot_v2_baseline.rs` where they exist;
add missing ones (e.g. P185 itself stays `#[ignore]`'d during
Phase 0).

**Phase 1 — Extend `process_scope` / `place_large_and_recurse`:**
- Recognize `Value::Insert` at function-body root (treat as synthetic
  Block with scope 1).  *(landed — Phase 1a, `e0a020f`)*
- Exhaustive IR traversal for `BreakWith / Iter / Tuple / TuplePut /
  Yield / Parallel`.  *(landed — Phase 1b, `494e5c7`)*
- Cross-scope `Set(v)` handling where `v.scope != walker_scope`
  (child-scope pre-init preamble in parent's operator list).
  *(landed — Phase 1b, `494e5c7`)*

Orphan-probe reduction:
- Pre-Phase 1a: 6 shapes surfaced.
- Post-Phase 1a: 4 shapes remained.
- Post-Phase 1b: 0 shapes remained.

Phase 1 complete.  Phase 2 gates the retirement of
`place_orphaned_vars`.

**Phase 2 — Delete and guard:**
- Delete `place_orphaned_vars` and its call site.  *(this commit)*
- Add invariant **I8 — orphan-iterator-alias** to
  `src/variables/validate.rs`: for each slot reuse across live
  intervals, walk dep chains to ensure no currently-live variable's
  value points into the reused slot's backing store.  Panic with
  `[I8]` prefix.  *(Phase 2b — next commit)*
- Un-ignore `tests/issues.rs::p185_slot_alias_on_late_local_in_nested_for`
  and `tests/slot_v2_baseline.rs::p185_late_local_after_inner_loop`.
  *(Phase 2c)*
- Mark P185 fixed in `doc/claude/PROBLEMS.md`.  *(Phase 2c)*

## References

Plan-04 artefacts (design archive, not driver):

- [`SPEC.md`](../04-slot-assignment-redesign/SPEC.md) — V2's
  intended algorithm (IR-walk, single pool).  Retracted as a
  replacement for V1, but the invariant set (I1–I6) survives and
  runs against V1's output.
- [`walkthroughs.md`](../04-slot-assignment-redesign/walkthroughs.md)
  — per-fixture structural rationale for P178, P185, and
  zone1-reuse patterns.
- [`00a-audit.md`](../04-slot-assignment-redesign/00a-audit.md) —
  size/scope/shape dispatch branches in V1's slots.rs.  Useful as a
  map of what the extended main walk must cover.

Code:

- `src/variables/slots.rs::place_orphaned_vars` — the target.
- `src/variables/slots.rs::process_scope` /
  `place_large_and_recurse` — the extension site.
- `src/variables/validate.rs` — where I8 lands.
- `src/variables/intervals.rs::compute_intervals` — current
  live-interval source; I8 adds dep-chain traversal on top.

## Ground rule — no regressions

Per [`plans/README.md`](../README.md): every phase lands a single
narrow change with `cargo test --release` green.  Never bundle the
main-walk extension with the orphan placer deletion.

## Non-goals

- **Zone split removal.**  V1's zone 1 / zone 2 split is not the
  source of the bugs; plan-04's uniform-placement constraint is
  retracted.  Plan-05 keeps V1's shape.
- **V2 retirement.**  V2 stays as a shadow validator indefinitely.
- **Placement-algorithm change.**  Main-walk extension is a
  coverage fix, not a redesign.
