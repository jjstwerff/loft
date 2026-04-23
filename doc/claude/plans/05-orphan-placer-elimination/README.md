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
  Block with scope 1).  *(landed)*
- When walking a child Block's operators, also check for parent-scope
  Sets and place them at the parent's frame base.  *(deferred)*
- When descending an Insert preamble, place its `__lift_*` / `__ref_*`
  temps at the enclosing function-scope frame base.  *(deferred)*

### Phase 1a — 2026-04-23 status

The Insert-root extension landed.  Orphan-probe instrumentation
showed it covers the P178 / P04 router shape (the canonical case).
Five residual orphan shapes remain:

- `_total_1(scope=3)` / `_gen_2(scope=4)` in a test fixture `f`.
- `_tv_1(scope=1, 24B text)` in a test fixture `f` (different from above).
- `_read_34 … _read_45` in scopes 45 / 53 / 61 / 62 of `n_main` of
  some test script — deeply-nested scopes never walked.
- `_idx_14(scope=2, 8B)` in `n_render_native` (graphics examples).

These surface via inner scopes that `process_scope` never reaches
because an outer wrapper (likely another Insert layer or a
for-loop-lowering Iter rewrite) intercepts before the scope-aware
walk.  Phase 1b will characterise each shape with a minimal
reproducer before extending the walker.

After **both** Phase 1a and Phase 1b land, `place_orphaned_vars`
should be unreachable on the full corpus.  Gate: `cargo test
--release` green with an `unreachable!()` marker replacing the
function body.

**Phase 2 — Delete and guard:**
- Delete `place_orphaned_vars` and its call site.
- Add invariant **I8 — orphan-iterator-alias** to
  `src/variables/validate.rs`: for each slot reuse across live
  intervals, walk dep chains to ensure no currently-live variable's
  value points into the reused slot's backing store.  Panic with
  `[I8]` prefix.
- Un-ignore `tests/issues.rs::p185_slot_alias_on_late_local_in_nested_for`
  and `tests/slot_v2_baseline.rs::p185_late_local_after_inner_loop`.
- Mark P185 fixed in `doc/claude/PROBLEMS.md`.

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
