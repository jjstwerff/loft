<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 04 — Slot assignment redesign

## Status — 2026-04-23: closed

All phases landed.  Plan moved to `plans/finished/`.  See also the
companion [`plan-05`](../05-orphan-placer-elimination/README.md)
which retired `place_orphaned_vars`.

**Final shape — three successive refits, no V1 retirement:**

- **Phase A (`9f759ee`) — landed.**  V1 revert + invariant **I7 —
  scope-frame consistency** in `validate.rs`.  Converts the
  `Incorrect var X[slot] versus TOS` runtime-panic class for
  zone-1 placements into a compile-time `[I7]` diagnostic.
- **Phase B.1 (`9f759ee`) — landed.**  Positional init
  primitives `OpInitRefSentinel(pos)` + `OpInitCreateStack(pos,
  dep_pos)` added alongside the existing `OpInitText(pos)` /
  `OpInitRef(pos)` (from 2h.1).
- **Phase B.2 (`bea156a…5e35948`) — landed.**  Rewired every
  codegen call site + parser-emitted compound op via
  `generate_call` interception; deleted `OpText` (−1 opcode).
  The three remaining compound ops (`OpConvRefFromNull`,
  `OpNullRefSentinel`, `OpCreateStack`) became dead runtime code
  but kept their dictionary entries so parser `cl("OpName", …)`
  still resolves.
- **Phase B.3 atomic bundle (`06a8d14`) — landed.**  Single
  function-entry `OpReserveFrame(frame_hwm)`; per-block
  `OpReserveFrame(block.var_size)` deleted; slot-move + gap-fill
  removed from `gen_set_first_at_tos`.  `gen_set_first_*` helpers
  rewritten slot-aware.  `generate_set`'s first-allocation
  dispatch collapsed to a single `gen_set_first_at_tos` call.
  `generate_block` drop-tail compensation re-reads a `Var(inner)`
  when the tail is `Drop(Var(_))` and the block result is
  non-Void (papers over a parser-emitted IR pattern the previous
  per-block reserve implicitly covered).
- **Phase B.3 follow-up v2 (`f47cc93`) — landed.**  `par(b = …, N)`
  loop body treats `b` as a parse-time alias, not a runtime
  variable; `build_parallel_for_ir` uses `create_unique` +
  `set_name` to register `b` for body parsing, then
  `replace_var_in_ir` substitutes every `Value::Var(b_var)` with
  the element accessor at parse time.  Fixes the name-sharing
  regression where two par blocks collapsed onto one `b_var` via
  `Function::add_variable`'s name-keyed early-return.  See
  [`b3-par-inline.md`](b3-par-inline.md) for the design and
  [`b3-par-type-lie.md`](b3-par-type-lie.md) for the shelved v1
  attempt's post-mortem.
- **Phase B.4 — landed.**  `SLOTS.md` frame-layout section
  rewritten for the single-entry-reserve design; CHANGELOG entry
  added alongside plan-05's close-out.

**Retracted / deferred out of scope:**
- The original single-pass V2 allocator driving codegen (2h, 2v,
  03-switch) — both attempts hit a shared failure mode on
  outer-scope / inner-Set placements.  V2 remains a shadow
  validator.
- The `OpConvRefFromNull` / `OpNullRefSentinel` deletion attempt
  (2026-04-23) was reverted: replacing the parser's `cl(...)`
  emissions with decomposed Block IR broke native codegen (the
  compound ops' `#rust"..."` templates had no equivalent on the
  positional init ops).  Compound ops stay as dictionary-only
  dead runtime code.

See [`05-orphan-placer-elimination/`](../05-orphan-placer-elimination/README.md)
for the companion plan that retires `place_orphaned_vars` by
extending V1's main walk.  Both plans together retire every
brittle-patch target this initiative originally set out to
structurally fix.

**What survives (the permanent state):**
- V1 continues to drive codegen.
- V2 shadow validator under `LOFT_SLOT_V2=validate` — I1–I6 green
  on the corpus; a meaningful correctness gate for future V1 edits.
- Invariants I1–I7 in `validate.rs` run at end of every codegen
  pass in debug / test builds against V1's output.
- SPEC.md / walkthroughs.md / fixture-catalogue remain as
  reference.

---

## Original goal (superseded)

Replace the current two-zone slot allocator plus orphan-placement
post-pass with a single algorithm that assigns every local variable
a slot in one deterministic pass.  The current design has produced a
recurring class of safety bugs (P178, P185, and likely more) that
each required a targeted patch on top of the existing heuristics.
The redesign removes the heuristics rather than patching them one
at a time.

## Context

`src/variables/slots.rs::assign_slots` is the single authority for
variable slot positions.  Its current shape (as documented in
[SLOTS.md](../../../SLOTS.md)):

- **Two zones.**  Small (≤ 8 B) variables go to zone 1 with greedy
  interval colouring and `OpReserveFrame`.  Large (> 8 B) variables
  and every loop-scope variable go to zone 2 with sequential IR-walk
  placement.  The split matches a real runtime distinction (loops
  can't reuse zone 1 across iterations) but the two halves have
  different placement algorithms that must agree on invariants like
  "no slot is shared with a still-live variable."
- **Special cases in the IR walk.**  Block-return, Insert preamble,
  Loop-scope, orphaned variables — each is handled by a separate
  branch in `process_scope` / `place_large_and_recurse`.
- **Orphan placer.**  `place_orphaned_vars` runs after the main walk
  for variables whose scope is not a Block/Loop node.  Started slot
  search from `0` before P178 → overlapped arguments; patched with
  `local_start` parameter.  Started without considering iterator-
  temporary liveness before P185 → overlapped a live text buffer;
  no patch yet.

The cumulative result is ~800 lines of slot-placement code, multiple
passes over the IR, and a documented pattern of "add a tactical
filter to `place_orphaned_vars` whenever a new aliasing bug is
reported."  P185's root cause is the third orphan-placement
regression in this class; the pattern makes another one likely.

## Why a redesign and not another patch

- **Classified safety bugs:** heap corruption / use-after-free, not
  just wrong values.  Any single-instance fix leaves the neighbouring
  configurations vulnerable until someone stumbles on them — exactly
  how P185 was found (generator script crashes mid-write).
- **Heuristic cost:** each patch adds a guard without retiring the
  code path that needs the guard.  The two-zone design, sequential
  IR-walk, and separate orphan placer will still coexist after any
  P185-specific fix, so the next aliasing edge case still has room
  to hide.
- **Test cost:** every new special case needs its own fixture.  The
  `p178_*`, `p185_*`, and SLOTS.md pattern-tests table would grow
  indefinitely under the patch-by-patch approach.

## Design direction (resolved)

Full specification: [**SPEC.md**](SPEC.md).
Per-fixture walk-throughs and invariant table: [**walkthroughs.md**](walkthroughs.md).
Spec-critique log: [**SPEC_GAPS.md**](SPEC_GAPS.md).

### Headlines

- **V1 is not V2's reference.**  V2's placements are the new truth.
  The correctness gate is invariant-based (SPEC § 5a): every
  function satisfies I1 (no-overlap) through I6
  (loop-iteration-safety), and the test suite stays green.  Byte-
  match against V1 is explicitly *not* a goal — V1 has legacy
  quirks (zone split, per-scope islands, Inline-size-match) that
  V2 is designed to replace, not replicate.
- **The algorithm is one pass, nine unconditional steps.**  Sort by
  `(live_start, var_nr)`, greedy-place with interval-overlap and
  `SlotKind` compatibility checks.  No branches on variable size,
  scope kind, or set cardinality.  `SlotKind` (Inline vs RefSlot)
  is the one structural axis, corresponding to the runtime's
  `OpFreeRef` / `OpFreeText` drop distinction.
- **Block-return aliasing handled by codegen, not the allocator.**
  V2 places `Set(v, Block([…, r]))` as two independent slots;
  codegen generalises the existing Text copy-path to every
  non-Inline block-return.  Removes a whole class of V1 bug
  (P122 frame-share family).
- **`per_block_var_size` preserved as a compatibility surface.**
  V2 outputs `(slots, hwm, per_block_var_size)` so the existing
  bytecode codegen path (`OpReserveFrame(var_size)` per block)
  is unchanged in Phase 2.  The function-entry-only
  `OpReserveFrame(hwm)` optimisation is a Phase 3+ cleanup.
- **Invariant testing replaces byte-match fixtures.**  The 24
  `.slots(…)`-locked fixtures in `tests/slot_v2_baseline.rs`
  become `.invariants_pass()` assertions in Phase 2.  The `.loft`
  snippets and structural rationales (`walkthroughs.md` § 4)
  survive; the numeric layout locks go.
- **P185 gets un-ignored.**  V1's layout for that fixture fails
  invariant I1 (overlap-on-aliased-slot).  V2 passes
  structurally.  The `#[ignore]` is removed in the same commit
  as V2's switchover.

### Hard constraint — uniform placement

**Every local goes through the same placement path.**  The redesign
is explicitly illegal if it:

- treats **small variables** (≤ 8 B — `integer`, `boolean`,
  `character`, enum values, references-as-handles) differently from
  **large variables** (text, vectors, structs, anything via
  `Store::alloc`);
- treats **individual variables** differently from **sets of
  variables** (all locals in a Block, all locals in a Loop, all
  orphaned locals, the args + return-address prefix);
- carries any branch of the form "if this variable's size is N" or
  "if this scope contains more than one variable" or "if this
  variable's scope-shape is X."

The algorithm must accept a flat list of `(live_interval, size,
alignment)` triples and emit `slot_position` for each, with the same
code path producing the result whether the input has one variable
or five hundred, and whether each one is one byte or one kilobyte.

This constraint is deliberately stronger than "simpler."  The
recurring bug pattern (P178, P185, and the P122p/q/r series before
them) is that size-based or shape-based branches accumulate filters
every time a new aliasing case surfaces.  Making it **illegal** for
the new algorithm to branch on size or group membership is the
structural guarantee that the next aliasing case has nowhere to
hide — it either breaks the one placement path (loud failure), or
it doesn't exist.

Ergonomic exceptions the constraint does NOT rule out:
- Reading size + alignment from the input triple is fine — the
  constraint is against *branching* on them, not against *reading*
  them (an interval-graph-colouring step that looks at size to
  pick the lowest non-overlapping slot is one branch-free formula,
  not "zone 1 vs zone 2").
- The runtime still has its own notion of frame layout
  (`OpReserveFrame`, return-address offset).  The allocator speaks
  to that contract through its output, not by simulating it
  internally with a size-based split.

### Carve-out — `SlotKind` for drop-opcode semantics

**Permitted:** exactly one structural axis — `SlotKind`
(`Inline` vs `RefSlot`) — reflecting runtime drop-opcode semantics
(no drop / `OpFreeRef` / `OpFreeText`).  Within the `RefSlot`
axis, size comparison is permitted to keep drop-opcode reads
type-compatible (a slot previously holding a 24-B `String` cannot
be reused by a 12-B `DbRef` even with disjoint lifetimes — the
scope-exit drop would read the wrong bytes).

**Scope of the carve-out (what it does NOT permit):**
- No branches on scope kind (Block / Loop / If / Match), set
  cardinality, IR shape, or "orphan-ness."
- No size-based dispatch *outside* the `RefSlot` reuse-safety
  check.  Inline slots of any size reuse freely when lifetimes
  allow.

**Why the carve-out is bounded.**  `SlotKind` has exactly two
values and three drop opcodes (none / `OpFreeRef` / `OpFreeText`).
Adding a third kind requires adding a real runtime drop opcode —
it cannot slip in as a quiet allocator patch.  Phase 4's lint
(`slot_allocator_has_no_size_or_shape_branches`) recognises the
one permitted `match self.kind` and rejects every other size /
shape branch.

**Why the carve-out is necessary.**  V2 cannot eliminate the
branch without one of:
- A runtime change so a single drop opcode handles all kinds
  (invasive, outside plan-04's scope).
- Pessimising to "no slot reuse ever" (throws away the whole
  point of the redesign).
- A two-pool design (one pool per kind, coloured independently) —
  loses the single-pool goal and re-creates a zone-split in all
  but name.

None of these is better than allowing one typed axis with a
documented runtime contract.

### Key design questions to resolve in Phase 1

- **Loop-scope lifetime.**  Loops currently need sequential
  placement because per-block `OpFreeStack` inside a loop corrupts
  the iteration.  Under the uniform-placement rule, loops and
  blocks must share the algorithm — so the runtime contract has
  to change: one function-entry `OpReserveFrame(hwm)` covers the
  whole function, and neither blocks nor loops `OpFreeStack`.  The
  cost is that block-scope slot reuse across sibling blocks has to
  come out of liveness analysis (which it already does) rather
  than out of `OpReserveFrame` / `OpFreeStack` bookkeeping.
- **Block-return aliasing.**  Today `Set(v, Block([..., result]))`
  implicitly writes `result` through `v`'s slot — treating the
  Block's locals as a set with shared placement.  Under the
  constraint, "the block writes through v's slot" has to be
  expressed in the IR (e.g. rewrite to `Block([..., Set(v,
  result)])`) rather than as a placement-time special case.
- **Arguments.**  The args + return-address prefix is currently a
  pre-reserved region that placement avoids.  Options that respect
  the constraint: (a) finalise arg positions by extending the same
  liveness graph to include args as "live from entry to last use";
  (b) keep arg placement as a runtime-layout concern owned by
  codegen, with the allocator producing slots that start at an
  offset the allocator doesn't know or care about.  Phase 1 picks
  one — no mixed approach.

## Ground rule — no regressions

Per [`plans/README.md`](../../README.md): every phase must preserve the
full test suite green.  No "rewrite everything then fix the fallout"
— each phase lands a single narrow change with a regression guard.

## Phases

| # | Phase | File | Status | Blocks |
|---|---|---|---|---|
| 0 | **Characterize** — lock current behaviour with tests (P178, P185, every SLOTS.md pattern as an explicit assertion), audit every `place_orphaned_vars` call site, produce a fixture catalogue. | [00-characterize.md](00-characterize.md) | ✅ done | — |
| 1 | **Design** — write `SPEC.md` with the single-pass algorithm, resolve the three design questions, walk the spec through every Phase 0 fixture by hand.  No code changes. | [01-design.md](01-design.md) | ✅ done | — |
| 2 | **Parallel implementation** — build V2 in `src/variables/slots_v2.rs`, add an equivalence harness behind `LOFT_SLOT_V2=validate`, iterate until the whole suite passes with the harness on.  Codegen still uses V1. | [02-parallel-impl.md](02-parallel-impl.md) | ✅ done | — |
| 2h | **Codegen refactor** — would have broken `OpText` / `OpConvRefFromNull` to accept a slot position, retired the codegen fixup, deleted V1, un-ignored P185. | [02h-codegen-refactor.md](02h-codegen-refactor.md) | ❌ retracted — see file header for why | — |
| 2v | **V2-drive (alternative to 2h)** — tried making V2 the authoritative allocator instead of the codegen-is-allocator pivot.  Same failure mode as 2h: V2's IR-walk doesn't scope-filter. | — | ❌ retracted | — |
| 3 | Original "switch" plan (V1→V2).  Never revisited after 2h/V2-drive both failed. | [03-switch.md](03-switch.md) | ❌ retracted | — |
| 4 | Cleanup (rewrite SLOTS.md for V2, add `slot_allocator_has_no_size_or_shape_branches` lint, move to `plans/finished/`). | [04-cleanup.md](04-cleanup.md) | ❌ retracted | — |
| 2+ | **Expanded invariant validation (close-out)** — add I7 scope-frame check to `validate.rs`.  Catches the `Incorrect var X[slot] versus TOS` runtime panic class at compile time. | — | ✅ landed `9f759ee` | — |
| B.1 | **Positional init primitives** — `OpInitRefSentinel(pos)`, `OpInitCreateStack(pos, dep_pos)` added as dormant opcodes alongside the existing `OpInitText` / `OpInitRef` (from 2h.1). | — | ✅ landed `9f759ee` | — |
| B.2 | **Compound-op decomposition** — rewire all codegen call sites + parser emissions to `OpReserveFrame(n) + OpInit*(n)`; delete `OpText` (−1 opcode).  The 3 remaining compound ops become dead runtime code. | — | ✅ landed `bea156a…5e35948` | — |
| B.3 | **Function-entry frame reserve** (slot-aware refactor of 4 `gen_set_first_ref_*_copy` paths + reassign branch; delete slot-move; single `OpReserveFrame(frame_hwm)` per function; delete 3 dormant compound ops). | [b3-function-entry-reserve.md](b3-function-entry-reserve.md) | ✅ landed `06a8d14`; wrap threading regression closed by B.3-follow-up (v2) | — |
| B.3-follow-up (v1) | **par() IR type-lie fix** (shelved) — correct `build_parallel_for_ir`'s first-pass placeholder. | [b3-par-type-lie.md](b3-par-type-lie.md) | ❌ attempted, insufficient — `b_var` is shared across par blocks via name-keyed `add_variable`; `set_type` oscillates. Post-mortem in doc. | — |
| B.3-follow-up (v2) | **par() loop variable as inline alias** — `b` becomes a parse-time alias for the `OpGetField(OpGetVector(…))` accessor, not a runtime variable.  No `b_var` means no name-sharing bug; the view-semantics of "borrowed read into `_par_results_N`" fit text/ref/struct returns as well. | [b3-par-inline.md](b3-par-inline.md) | ✅ landed — all 49 primitive-pair matrix tests pass; `wrap::threading` + `script_threading` pass; `issues` 500/500, `expressions` 119/119, `slot_v2_baseline` 27/27; `find_problems.sh --bg --wait` clean | — |
| B.4 | **Docs** — rewrite SLOTS.md frame-layout section; CHANGELOG entry. | — | ✅ landed (this commit) | — |

**Phase 0 artefacts:**
- 26 fixtures in `tests/slot_v2_baseline.rs` (24 passing, 2 `#[ignore]`-d — P185 and a par-codegen pre-existing issue).
- `SLOTS.md` § "Phase 0 Fixture Catalogue" + § "Scope shapes and orphan placement".
- `00a-audit.md` — 20 size/scope/shape dispatch points V2 must subsume.
- Side discovery: P186 (struct-typed block expressions rejected) — fixed inline; no longer blocks the redesign.

**Phase 1 artefacts:**
- [`SPEC.md`](SPEC.md) — allocator input/output, 9-step algorithm,
  three design decisions resolved, invariant-based correctness gate,
  implementation sketch.
- [`walkthroughs.md`](walkthroughs.md) — three end-to-end traces
  (P178, P185, `zone1_reuse_two_ints_same_block`) plus a per-fixture
  structural-rationale table mapping each fixture to the invariants
  it exercises.
- [`SPEC_GAPS.md`](SPEC_GAPS.md) — nine critical-review gaps,
  six resolved, one moot, one deferred, one pending user signoff
  on a README wording change (SlotKind carve-out).

**Phase 2 artefacts:**
- `src/variables/validate.rs` — extended `validate_slots` with
  invariants I2–I6 (distinct `[I1]`…`[I6]` panic prefixes);
  10 unit tests in `mod invariant_tests` covering each failure path.
- `src/variables/slots_v2.rs` — V2 algorithm per SPEC § 2, with
  5 walk-through unit tests.
- `src/scopes.rs` — `LOFT_SLOT_V2` shadow plumbing (`validate` /
  `report` / `drive` modes).
- `tests/slot_v2_baseline.rs` — 29 fixtures transitioned from
  `.slots(…)` layout locks to `.invariants_pass()`.
- `tests/testing.rs` — `.invariants_pass()` helper.
- [`02c-optimality-report.md`](02c-optimality-report.md) — corpus-wide
  O1 measurement: **99.8 % of 10,352 functions are byte-for-byte
  identical between V1 and V2**; 17 tighter, 2 looser, net −100 bytes.
  Zero invariant violations under `LOFT_SLOT_V2=validate`.

Phase 0 is the one that unlocks the rest — without an exhaustive
fixture catalogue there's no way to show Phase 2's equivalence
assertion is meaningful.

## Non-goals

- **Changing the runtime frame layout.**  The interpreter's stack
  representation (`Store::stack`, `OpReserveFrame`, `OpFreeStack`) is
  not in scope.  V2 must produce slot positions that slot into the
  existing runtime without opcode changes.
- **Native-codegen-specific slot logic.**  `src/generation/` consumes
  `stack_pos` as an opaque input; the redesign targets the
  interpreter path, and the native path inherits whatever V2
  produces.
- **Performance targets.**  Not a performance-driven rewrite.  Any
  throughput change (up or down) is acceptable if correctness and
  simplicity improve; the plan tracks wall-clock on `cargo test
  --release` only as a guardrail, not a metric.

## Success criteria

1. P178's and P185's regression tests un-`#[ignore]`'d and passing
   without per-case patches in the allocator.
2. `place_orphaned_vars` removed from the tree.
3. `src/variables/slots.rs` is under 800 lines (currently ~1676)
   *or* the new file has one exported entry point and a single
   inner helper (no "zone N" branching).
4. Every fixture from Phase 0 produces identical slot assignments
   under V2 (locked by Phase 2's equivalence assertion before the
   switch).
5. `tests/issues.rs` slot-related `#[ignore]` count stays at zero
   after Phase 3.
6. **Uniform-placement audit:** grep the post-switch `slots.rs`
   (and any files it delegates to) for branches on variable size,
   scope kind (Block vs Loop vs "orphan"), or set cardinality.
   Every hit either (a) has a rationale documented in the code
   explaining why it is NOT size/shape-based placement dispatch
   (e.g. an interval-graph colouring step that reads size to
   compute overlap), or (b) is a regression against the hard
   constraint and must be removed.  This check lands as a
   `tests/doc_hygiene.rs::slot_allocator_has_no_size_or_shape_branches`
   lint in Phase 4.

## Related

- [SLOTS.md](../../../SLOTS.md) — current design (will be rewritten in Phase 4).
- [PROBLEMS.md § P178](../../../PROBLEMS.md) — orphan-placer argument-area overlap.
- [PROBLEMS.md § P185](../../../PROBLEMS.md) — orphan-placer text-buffer overlap.
- `src/variables/slots.rs` — the subject.
- `src/variables/intervals.rs` — live-interval computation (already present, to be reused).
- `src/variables/validate.rs` — debug-only overlap validator (Phase 2's equivalence check will piggy-back on this).
