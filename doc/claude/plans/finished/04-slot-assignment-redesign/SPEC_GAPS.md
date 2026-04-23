<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# SPEC.md — Gap List

Issues flagged by critical self-review of [SPEC.md](SPEC.md).  Each
entry is worked to closure; SPEC.md and walkthroughs.md are updated
in place as answers land.

Status key: **open** · **in progress** · **resolved** · **moot**
(superseded by a later design decision) · **deferred** (valid but
not Phase 2-blocking).

| # | Gap | Status | Outcome |
|---|-----|--------|---------|
| 1 | `validate.rs` invariant — does V2's output satisfy every check? | resolved | Yes.  The existing `find_conflict` check is purely spatial+temporal overlap with three permissive exceptions (same-name aliasing, sibling scopes, `skip_free`).  V2 satisfies it by construction.  Phase 2 extends `validate_slots` to check the full invariant set I1–I6 (SPEC § 5a). |
| 2 | Native-codegen reads `block.var_size` — does retiring per-block reserve break it? | resolved | `src/generation/` has **no** dependency on `var_size`; only the bytecode codegen path does.  V2 outputs `per_block_var_size` as a compatibility surface so bytecode codegen is unchanged.  Retiring per-block reserves (function-entry-only `OpReserveFrame`) is a follow-up codegen refactor, not a Phase 2 blocker. |
| 3 | RefSlot size-uniqueness — does `size(Type)` distinguish Text / Reference / Vector? | resolved | `Type::Text` is 24 B (unique); all other RefSlot types are 12 B and share the `OpFreeRef` drop opcode.  Cross-type reuse within the 12-B bucket is safe because `compute_intervals` walks the `OpFreeRef(v)` call and extends `v.last_use` to scope exit — the overlap check in step 5c therefore respects drop timing.  Size match alone is sufficient. |
| 4 | Argument-region contract — V2 inherits `local_start` correctly? | resolved | `local_start = sum(arg_sizes) + 4` computed in `scopes.rs:156–164`.  V2's input filter excludes `v.argument == true`, and step 4 unconditionally starts every candidate at `local_start`.  P178-class "orphan starts at slot 0" cannot recur at the algorithm level. |
| 5 | Block-return rewrite vs control-flow exits (Break / Continue / Return inside the block) | resolved | Control-flow exits short-circuit before any trailing `Set` runs, so the pre-rewrite and post-rewrite IR have identical runtime semantics.  However: the § 3.2 design evolved from (a) IR rewrite through (b) alias hint to (c) **no special case** — V2 no longer models block-return aliasing at all.  Codegen generalises the existing Text copy-path to every non-Inline block-return.  The outcome: one fewer V1 quirk for V2 to replicate, and one fewer bug class (the P122 frame-share family). |
| 6 | Eight more fixture traces to ground the "match V1" claims | resolved — direction changed | Traced fixtures #2, #5, #10 end-to-end.  Finding: V2 and V1 diverge on most fixtures because V1 has zone-first ordering, per-scope placement islands, and Inline-size-match-for-reuse — quirks V2 cannot reproduce without re-introducing the branches the redesign retires.  **Decision:** V1 is not V2's reference.  V2's placements are the new truth.  The correctness gate moves to invariant-based verification (I1–I6 in SPEC § 5a). |
| 7 | Empirical hwm slack (not a magic +32) | moot | V1 is not the reference; no "slack against V1" is needed.  V2's `hwm` is reported per-function and aggregated across the corpus as an **optimality** metric (O1 in SPEC § 5a), not a correctness gate.  A V2 regression over V1 is investigated but does not, by itself, block Phase 3 if behavioural tests pass. |
| 8 | O(n²) complexity bound acknowledged; worst-case function size | deferred | V2's algorithm is O(n²) in the interval count (per-interval linear scan of `placed`, with a retry loop bounded by the placed count).  Worst case: a function with `n` live intervals runs ≤ `n²` overlap checks.  For today's fixture corpus (max ~20 live intervals per `n_test`), this is trivial.  For the largest real functions (`make ci` covers all packages), Phase 2 adds a complexity measurement to the equivalence harness (record max `placed.len()` per function).  If any function exceeds 200 live intervals, Phase 2 revisits.  Not Phase 3-blocking in any currently-known function. |
| 9 | `SlotKind` carve-out vs README's hard uniform-placement constraint | resolved | User approved the carve-out (2026-04-22).  README's "Hard constraint" section now includes a § "Carve-out — `SlotKind` for drop-opcode semantics" permitting exactly one structural axis (Inline vs RefSlot) plus a size comparison *within* the RefSlot axis to preserve `OpFreeRef` / `OpFreeText` compatibility.  Phase 4's lint recognises the one permitted `match self.kind` and rejects every other size / shape branch. |

---

## Summary

Seven gaps resolved; one moot, one deferred.  Phase 2 is fully
unblocked.

The single-pool, scope-blind algorithm with invariant-based
verification is the current design.  Phase 2 work lands on SPEC § 5a
(extended `validate_slots`) and `tests/slot_v2_baseline.rs` fixture
rewrite (from layout locks to `.invariants_pass()`).

## Round-2 refinements (post-review pass)

After a second critical read, three spec tightenings landed:

- **SPEC § 3.2 — codegen refactor made concrete.**  Instead of the
  vague "codegen generalises the Text copy-path," § 3.2 now
  documents the exact routing change (detect `Value::Block` RHS in
  `gen_set_first_at_tos`, pre-init v's slot, route through
  `set_var`).  Confirmed bounded (~100 LOC, no new opcodes) by
  reading `src/state/codegen.rs:664–1138, 2077–2185`.

- **SPEC § 5a — I5 tightened.**  Old wording checked
  `A.slot == B.slot AND A.size == B.size`; missed partial-range
  overlap (24-B Text at slot 4 vs 12-B DbRef at slot 4, same
  start, different size).  New I5 checks "slot ranges overlap
  spatially" with a full-range-congruence rule for RefSlot reuse.

- **SPEC § 3.1 — `frame_base(scope)` and `per_block_var_size`
  defined.**  Previously used without definition; the formula
  now walks the scope tree (ancestors of S) and clamps to
  `local_start`.  The `max(0, …)` clamp handles V2's cross-scope
  reuse case (scope whose vars all reuse ancestor slots needs
  zero additional reserve).

- **SPEC § 5a — I6 redundancy called out.**  I6 is stated to be
  defence-in-depth: it only fires on `compute_intervals` bugs,
  not allocator bugs.  Kept because interval-pass regressions
  would silently corrupt loop-carried state.
