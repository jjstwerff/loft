<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# V2 Slot Allocator — Specification

Status: **draft**.  Owner: plan-04 Phase 1.  Last updated: 2026-04-22.

This is the full specification for the single-pass slot allocator.
It satisfies the hard constraint from [README.md](README.md) — no
branches on variable size, scope kind, or set cardinality.

V1 (the current two-zone allocator) is being retired.  V2 is not a
V1 replacement that tries to mimic it — V2 is the allocator going
forward.  V2's placements are the new truth; V1's historical
placements are not a target.  In practice V2 produces tighter
layouts than V1 on most functions (smaller or equal `hwm`) because
the single pool permits cross-scope dead-slot reuse that V1's
per-scope zone split prohibits.  That's a feature, not a
divergence to account for.

Practical consequence for `tests/slot_v2_baseline.rs`: the 24
`.slots(…)` specs currently lock V1's placements.  They were
harvested from V1 to characterise the *old* behaviour for the
audit; Phase 2 re-harvests them from V2 and locks the new layouts.
The fixtures themselves — the `.loft` snippets and their
rationales — survive unchanged; only the numeric specs shift.  See
Phase 2's `02-parallel-impl.md` for the harvest / re-lock step
order.

---

## 1. Input / output

### Input

A flat vector of intervals, one per live local and per work-ref:

```rust
pub struct LocalInterval {
    pub var_nr: u16,
    pub live_start: u32,   // from intervals.rs::compute_intervals
    pub live_end:   u32,   // inclusive; loop-carry extension already applied
    pub size:       u16,   // size(var.type_def, Context::Variable)
    pub kind:       SlotKind,
}

pub enum SlotKind {
    /// Inline storage: integers, booleans, characters, DbRef handles,
    /// struct fields.  Slot is reusable by any other Inline interval
    /// whose live range does not overlap.
    Inline,
    /// Allocation handle: Text (24 B String), owned vectors, owned
    /// structs backed by Store::alloc.  Slot reuse only permitted
    /// between `RefSlot` intervals of the same size and type
    /// discriminant, to preserve the drop-semantics V1 already relies
    /// on (OpFreeText/OpFreeRef at scope exit).
    RefSlot,
}
```

The caller (compile.rs) is responsible for:
- Running `compute_intervals` first — the allocator trusts `live_start`
  and `live_end` verbatim.
- Populating `kind` from `v.type_def` (`Text` / `Reference` / `Vector` →
  `RefSlot`; everything else → `Inline`).
- Excluding arguments (they are pre-placed in the frame prefix and
  given to the allocator as `local_start: u16`).

### Output

```rust
pub struct SlotAssignment {
    pub var_nr: u16,
    pub slot:   u16,      // offset from frame base
}

pub struct AllocatorResult {
    pub slots: Vec<SlotAssignment>,
    pub hwm:   u16,                       // function-level high-water mark
    pub per_block_var_size: HashMap<u16, u16>, // scope_nr → zone1-style pre-claim size
}
```

The `per_block_var_size` field is a **compatibility surface**, not a
zone-split re-emergence: for each Block/Loop node, it carries the
largest-slot-above-frame-base among the variables V2 placed in that
scope, so the existing bytecode codegen path
(`src/state/codegen.rs:1896`) can continue to emit its per-block
`OpReserveFrame(var_size)` unchanged.  The algorithm itself (§ 2)
never branches on scope kind; `per_block_var_size` is computed as a
post-pass summary of `slots`, one entry per unique `scope` in the
input.

Retiring per-block `OpReserveFrame` in favour of a single
function-entry `OpReserveFrame(hwm)` is a follow-up codegen change
(tracked as Gap 2 in `SPEC_GAPS.md`) — Phase 2 keeps the existing
codegen contract and lands the function-entry-only optimisation in
a separate commit after the equivalence harness is green.

---

## 2. Algorithm

The allocator is ~15 unconditional steps.  No branches on variable
size, scope kind, or set cardinality — only on `kind` (Inline vs
RefSlot), which is a *type-class* property of the interval, not a
placement dispatch.

```text
 1. Sort `intervals` by (live_start, var_nr).  Stable on ties.
 2. Initialise `placed: Vec<(slot, size, kind, live_start, live_end)>`
    and `hwm = local_start`.
 3. For each interval `I` in order, do steps 4–8:
 4.   candidate = local_start.
 5.   For each `p` in `placed` (in any order):
 5a.     If `I.live_end < p.live_start` or `I.live_start > p.live_end`,
          `p` is disjoint in time — ignore (reuse OK).
 5b.     Else (live-overlap): if `p.kind != I.kind`, or
          `I.kind == RefSlot && p.size != I.size`, slot reuse is
          forbidden.  Treat as a hard spatial block: if
          `[candidate, candidate+I.size)` overlaps `[p.slot, p.slot+p.size)`,
          bump `candidate` to `p.slot + p.size` and re-run step 5.
 5c.     Else (live-overlap, compatible kind and — for RefSlot —
          matching size): live overlap + spatial overlap both bad.
          If `[candidate, candidate+I.size)` overlaps
          `[p.slot, p.slot+p.size)`, bump `candidate` to
          `p.slot + p.size` and re-run step 5.
 6.   Emit `SlotAssignment { var_nr: I.var_nr, slot: candidate }`.
 7.   Push `(candidate, I.size, I.kind, I.live_start, I.live_end)`
      into `placed`.
 8.   `hwm = max(hwm, candidate + I.size)`.
 9. Return `AllocatorResult { slots, hwm, per_block_var_size }`.
```

No alignment step.  V1 packs variables densely — a 1-byte bool at
slot 4 is followed by an 8-byte variable at slot 5 (observed in
fixture 5, `sibling_scopes_share_frame_area`: `cond+1=4 v+8=5`).
V2 matches this packing.  Runtime reads use the value-size'd
opcodes (`OpGetInt`, `OpGetFloat`, …) that handle arbitrary byte
offsets; no alignment constraint is observable from loft
semantics.

### Why this is uniform

Steps 4–8 do not branch on variable size, scope kind, or set
cardinality.  The only two conditions are:
- Live-range *overlap* (step 5a) — a purely numerical comparison on
  `live_start` / `live_end`, regardless of what the variable is.
- Kind compatibility (step 5b) — a property of the *type class*,
  not the allocator's placement strategy.  A `RefSlot`'s storage
  carries extra drop-time semantics (`OpFreeText`, `OpFreeRef`) that
  the runtime observes; mixing `Inline` and `RefSlot` in one slot
  would corrupt those semantics regardless of whether the uniform
  allocator knows about them.

The `SlotKind` axis is the structural analogue of V1's hard-coded
`Type::Text` branch at `slots.rs:236` — but it is a single, typed
input field with a documented runtime contract, not a dispatch on
"if this variable is Text, handle it specially here."

### Formula vs branch

The reading of `size` in step 5c and the size comparison in step 5b
are *formula* reads (the allocator consumes these scalars to compute
an offset), not *branches* (the allocator does not switch between
different placement strategies based on them).  This matches the
"ergonomic exceptions" in [README.md's hard constraint](README.md) —
reading size to run interval-graph colouring is one branch-free
formula.

---

## 3. The three design questions, resolved

### 3.1 Loop-scope lifetime — V2's algorithm is scope-agnostic; codegen contract preserved in Phase 2

**Decision.**  V2's algorithm (§ 2) does not branch on "is this
variable in a loop scope?"  Sibling-block reuse and loop-carry
preservation fall out of liveness intervals alone (see rationale
below).  To keep Phase 2's blast radius small, V2 populates
`per_block_var_size` as a compatibility surface so the existing
bytecode codegen path can continue to emit per-block
`OpReserveFrame(var_size)` without change.  Retiring the per-block
reserves and moving to a single function-entry
`OpReserveFrame(hwm)` is a separate, smaller commit after the
equivalence harness is green.

**Why.**  Liveness-based colouring already captures the two
invariants that the per-block `OpReserveFrame` / `OpFreeStack`
currently enforce at runtime:

1. *Sibling-block slot reuse.*  Two if-arms with disjoint lifetimes
   share a slot because their intervals don't overlap, not because
   `OpFreeStack` restores TOS at the end of each arm.  See
   fixtures 5 (`sibling_scopes_share_frame_area`) and 16
   (`nested_if_block_branches`).
2. *Loop-carried value preservation.*  A variable defined before a
   loop and read inside it gets its `last_use` extended to the end
   of the loop by `intervals.rs:110–132`.  Colouring will not reuse
   its slot for an internal loop variable because they are
   interval-live simultaneously.  See fixture 19 (`for_loop_two_loop_locals`)
   and fixture 10 (`parent_refs_plus_child_loop_index`).

The cost is a potentially larger `hwm` on functions where
block-local storage could have been recycled across sibling blocks
that don't *actually* overlap under liveness (rare: our 26 fixtures
show no such case).  A guardrail on this is spelled out in § 5.

**Phase 2 contract (conservative).**  Codegen is unchanged:
`src/state/codegen.rs:1896` still emits
`OpReserveFrame(block.var_size)` per block.  V2 populates
`block.var_size` from its placement output via this definition:

```text
frame_base(S) = local_start                               if S is the top-level scope,
                max over v with v.scope ∈ ancestors(S) of (v.slot + v.size),
                clamped to ≥ local_start,                 otherwise.

per_block_var_size[S] = max(0,
                            max_{v with v.scope == S}(v.slot + v.size)
                                                      - frame_base(S))
```

Where `ancestors(S)` is the set of scopes on the path from S up
to the root (excluding S itself).  `frame_base(S)` is the TOS at
block-S entry under V1's recursion model; V2 computes it as a
post-pass by walking the scope tree.

The `max(0, …)` clamp handles V2's cross-scope reuse: if every
var in scope S reuses an ancestor slot, S's reserve is 0 bytes
(no new stack needed at block-S entry, nothing to free at exit).
This is a strict generalisation of V1's zone1-sum, which could
never be 0 for a non-empty scope.

This is a post-pass summary of `slots`, not a placement decision.
The algorithm in § 2 never reads scope kind while placing;
`per_block_var_size` is computed afterwards for codegen's benefit.

**Follow-up contract (function-entry only).**  After the equivalence
harness is green:
- Delete the `OpReserveFrame(var_size)` emission in `generate_block`.
- Delete the `OpFreeStack(var_size)` emission in `generate_block`'s
  exit path.
- Emit `OpReserveFrame(hwm)` at function entry, `OpFreeStack(hwm)`
  at function exit.
- Teach `gen_set_first_at_tos` to handle `slot < stack.position`
  (direct `set_var` store into an already-reserved slot) instead
  of asserting `pos == stack.position`.

This follow-up is **outside the scope of Phase 2 as currently
planned.**  It is tracked in `SPEC_GAPS.md` Gap 2.

### 3.2 Block-return aliasing — no special case

**Decision.**  V2 does not model block-return aliasing.  For every
`Set(v, Block([…, r]))`, V2 places `v` and the block's final
expression on independent slots, chosen by normal interval
colouring.  There is no alias hint, no merged interval, no IR
rewrite.

V1 frame-shared `v` with the block's last op to save a copy at
codegen.  Under V2, codegen generalises the existing Text path to
every type.

**Concrete codegen refactor** (bounded, ~100 LOC in
`src/state/codegen.rs`):

- Today `gen_set_first_at_tos` (`codegen.rs:1021`) handles Text
  via `gen_set_first_text` (line 664), which emits `OpText` to
  pre-allocate `v`'s 24-byte buffer at `v`'s slot, then calls
  `set_var` to evaluate the RHS onto the eval stack and emit
  `OpAppendText` to copy into `v`'s slot.
- Non-Text first-assignment currently falls through to
  `self.generate(value, …)` at `codegen.rs:1138`, which relies on
  V1's frame-sharing assumption (`v.slot == block.TOS`).
- Under V2, extend the Text pattern to every type: for
  first-assignment to any `v` whose RHS is `Value::Block(…)`,
  pre-init `v`'s slot with the appropriate type-specific op
  (`OpConstInt 0` for integer, `OpConvRefFromNull` for
  `Reference`, `OpConstFalse` for bool, …), then route through
  `set_var`, which already emits the correct `OpPut<T>` /
  `OpPutRef` / `OpCopyRecord` / `OpAppendText` for every type.

`set_var` (line 2077) is already the uniform "evaluate value,
store into var's slot via appropriate OpPut" path; it handles
Integer, Text, Reference, Vector, Tuple, Enum, Function, and
Character.  The refactor is a routing change: detect
`value == Value::Block(_)` in `gen_set_first_at_tos` and call
`set_var` (with the pre-init prologue) instead of the fall-through
`generate`.

**No new opcodes.  No runtime contract change.**  The existing
bytecode semantics cover every case — only the emission site
shifts.

**Why this is clean.**  The frame-sharing in V1 is an optimisation
tangled with placement.  Separating them — V2 owns placement,
codegen owns the copy — keeps the allocator branch-free and the
runtime cost a single `OpCopyRecord` (or equivalent for small
types) per non-Text block-return site.  For a small number of
extra bytes of bytecode per site and no runtime allocation cost,
V2 no longer has to know about block-return at all.

**Interaction with the `Set(v, Block([..]))` slot-aliasing bug
family (P122-class).**  V1's frame-sharing is the mechanism most
of those bugs latched onto.  Removing it retires an entire bug
class: under V2 the block's locals and the outer target cannot
share a slot unless their intervals happen to be disjoint, in
which case the reuse is visibly correct.

### 3.3 Argument region — pick (b): pre-placed, allocator starts at `local_start`

**Decision.**  Arguments are placed by the parser / codegen before
the allocator runs; they occupy `[0, local_start)` in the frame.
The allocator takes `local_start: u16` as an input and never places
anything below it.  Argument intervals do NOT appear in the input
vector.

**Why.**  This matches V1's current behaviour (the `local_start`
parameter was the P178 fix) and keeps codegen's arg-placement logic
untouched.  Option (a) — "treat args as intervals from entry to last
arg-use" — would require dual-sourcing arg slots (parser assigns a
position, allocator confirms it matches), which is a contract
between two phases that V1 does not currently need.

The three argument-heavy fixtures from Phase 0 — P178
(`p178_is_capture_body`), fixture 15 (`fn_with_only_arguments`), and
fixture 13 (the Text-heavy Insert chain in
`sequential_lifted_calls`) — all pass identically under option (b)
because `local_start` already protects the arg region.

---

## 4. Walk-through tables

The per-fixture analyses live in a dedicated companion:
[`walkthroughs.md`](walkthroughs.md).

- **§ 1 (P178)** and **§ 2 (P185)** — full end-to-end traces of
  the algorithm, step by step, against the two regression fixtures.
- **§ 3 (`zone1_reuse_two_ints_same_block`)** — one sanity-check
  walk-through demonstrating the Inline-reuse path.
- **§ 4 (all-fixture summary)** — one row per fixture classifying
  V2 vs V1 as `match`, `divergence(OR)` (orphan-placer retirement),
  or `safe` (V2 produces a correct layout where V1 was unsafe).

Under the (b) alias-hint design (SPEC § 3.2), V2 reproduces V1
byte-for-byte on **every** layout-locked fixture.  The one behavioural
change is fixture #14 / P185: V2 picks a non-aliasing slot for `key`
where V1 currently UAFs.

---

## 5. Correctness gates for Phase 2

Before Phase 3 flips codegen to V2:

1. **All six safety invariants (I1–I6, § 5a) hold on every
   function in the test suite.**  `validate_slots` (extended in
   Phase 2) is the gate.
2. **Behavioural equivalence.**  Every `cargo test --release`,
   `make ci`, and `make test-packages` test passes under V2.
3. **Fixture suite transitioned.**  The 24 `.slots(…)`-locked
   fixtures in `tests/slot_v2_baseline.rs` have been converted
   to `.invariants_pass()` form (details in § 5a).  The `.loft`
   snippets and rationales remain; the numeric layout locks are
   gone.
4. **Optimality monitor.**  O1 (per-function `hwm` ≤ V1's) is
   reported for every function in the corpus.  Any regression is
   investigated; a regression does not, by itself, block Phase 3
   if the test suite is green, but the analysis must be recorded.
5. **P185 un-ignored.**  `p185_late_local_after_inner_loop`
   exercises a V1 use-after-free; under V2 it passes.  The
   `#[ignore]` attribute is removed in the same commit as V2's
   switchover.

---

## 5a. Invariant-based verification

V2 does not compare its output to V1's.  Correctness is checked by
running each output against a set of structural invariants that any
correct slot allocator must satisfy.  A layout is valid iff every
invariant holds; two valid layouts for the same function may differ
by slot number and still both be correct.

This replaces byte-match-against-V1 with a gate that stays
meaningful after V1 is retired and that generalises to any future
allocator rework (V3, native-codegen variants, etc.).

### Safety invariants (required for runtime correctness)

An allocator's output MUST satisfy all six.  A single failure on
any function blocks Phase 3.  Phase 2 extends
`src/variables/validate.rs::validate_slots` to check them.

- **I1 — No concurrent overlap.**  For every pair of variables
  `A`, `B` whose live intervals overlap AND whose scopes can be
  simultaneously live (`scopes_can_conflict` from today's
  validate.rs), `[A.slot, A.slot+A.size)` and
  `[B.slot, B.slot+B.size)` are disjoint.  *(This is today's
  single check; kept verbatim.)*

- **I2 — Argument isolation.**  For every non-argument variable
  `V` with `size > 0`, `V.slot >= local_start`.  *(Prevents the
  P178 class: orphan locals overlapping the argument region.)*

- **I3 — Frame boundedness.**  For every variable `V`,
  `V.slot + V.size <= hwm`.  *(Prevents writes past the reserved
  frame — would corrupt caller state.)*

- **I4 — Every defined variable is placed.**  For every `V` with
  `first_def != u32::MAX` and `size > 0`, `V.stack_pos != u16::MAX`.
  *(Catches regressions where the allocator silently skips a
  variable — the current `orphan` class of bugs.)*

- **I5 — Kind-consistency on overlapping-slot reuse.**  For any
  two variables `A`, `B` whose slot *ranges* overlap spatially
  (`A.slot < B.slot + B.size` AND `B.slot < A.slot + A.size`)
  AND whose live intervals are disjoint (reuse scenario):
  - `A.kind == B.kind`, and
  - if `A.kind == RefSlot`, then `A.slot == B.slot` AND
    `A.size == B.size` (full-range congruence).

  *(Preserves drop-opcode semantics: `OpFreeRef` / `OpFreeText`
  on a shared slot must fire on a value whose kind and size
  match the opcode's read.  The full-range congruence rule for
  RefSlot rules out partial overlaps like a 24-B Text at slot 4
  and a 12-B DbRef at slot 4 — identical start, different size,
  drop-opcode reads garbage past the smaller var's end.
  Inline vars can overlap partially when lifetimes are disjoint
  because Inline has no drop opcode — only runtime-reads of the
  smaller var see garbage, which is impossible since its
  lifetime ended.)*

- **I6 — Loop-iteration safety (defence-in-depth).**  For every
  loop scope `L` with seq range `[s, e]` and every pair `(V, W)`
  where `V.slot == W.slot`, at least one of:
  - both `V` and `W` are entirely inside `L` (`first_def >= s`,
    `last_use <= e` for both), or
  - both are entirely outside `L` (`last_use < s` or
    `first_def > e` for both), or
  - the two lifetimes are disjoint (covered by I1 then).

  *(Prevents iteration N+1 trampling a value written in iteration
  N.*

  *Note on redundancy:* if `compute_intervals` correctly extends
  `last_use` for loop-carried vars (`intervals.rs:110–132`), I1
  alone catches every real overlap — I6 only fires on
  `compute_intervals` bugs, not on allocator bugs.  It is kept
  as a defence-in-depth check: a regression in the interval pass
  that silently forgets loop-carry extension would still let V2
  produce a "correct by I1" layout that is catastrophic at
  runtime.  I6 catches such regressions before they reach
  codegen.*)

### Optimality invariants (diagnostic, not gates)

These are not blockers; they report on V2's performance relative
to V1.  Violations are flagged for investigation but do not stop
Phase 3.

- **O1 — hwm non-regression.**  For every function in the test
  corpus, `v2.hwm <= v1.hwm`.  V2's single-pool reuse is expected
  to be at least as tight as V1's per-scope zone split.  A
  regression signals either a spec bug (some V1 optimisation we
  missed) or an input difference (different `compute_intervals`
  output).
- **O2 — Aggregate frame size.**  Sum of `hwm` across all
  compiled functions in `cargo test --release` is ≤ V1's.

### How the invariants get checked

1. **Static (per-function).**  `validate_slots` runs after every
   allocator invocation in debug builds; an I1–I6 failure panics
   with a full variable table and IR dump (extends today's
   panic format).  This is the primary gate.
2. **Behavioural (per-test).**  `cargo test --release` and
   `make ci` remain the backstop — a program that compiles valid
   slots but miscomputes at runtime still fails.  Invariants
   catch misallocation; tests catch other bugs.
3. **Fixture suite (per-pattern).**  `tests/slot_v2_baseline.rs`
   drops the `.slots(…)` layout locks.  Each fixture keeps its
   `.loft` snippet and rationale; the assertion becomes
   `.invariants_pass()` (a new helper) which reruns `validate_slots`
   and returns `Ok` iff I1–I6 hold.
4. **Optional: randomised property test.**  A `proptest` / QuickCheck
   helper generates well-typed loft programs within size bounds,
   compiles under V2, and asserts I1–I6.  Targets high-branching
   patterns (nested loops, match arms, Insert preambles) where
   the fixture catalogue has sparse coverage.  This is a stretch
   goal for Phase 2 — not required for Phase 3 — but the framework
   cost is one `proptest` test file and a tiny type-respecting
   generator.

### Why this is strictly stronger than byte-match

Byte-match against V1 only catches V2 bugs on the *exact* inputs
V1 was run against.  A new program that neither V1 nor V2 has
seen could produce a broken V2 layout that byte-match cannot
catch.  Invariants I1–I6 hold for every possible input — the
check is universal.

Conversely, byte-match forbids legitimate V2 improvements (a
tighter layout is a "mismatch").  Invariants permit any valid
layout, so V2 is free to be better than V1.

### Minimum deliverable for Phase 3

- `validate_slots` extended with I2–I6 (I1 already present).
- `.slots(…)` calls in `tests/slot_v2_baseline.rs` replaced by
  `.invariants_pass()` before V2 flips on.
- One new `tests/slot_invariants.rs` integration test that
  exercises each invariant's failure mode with a hand-crafted
  bad `Function` (so a regression in `validate_slots` itself is
  caught).

---

## 6. What V2 still delegates to codegen / runtime

- **Frame reservation opcodes.**  `OpReserveFrame(hwm)` at function
  entry and `OpFreeStack(hwm)` at function exit — codegen reads
  `hwm` from V2's result.  Per-block `OpReserveFrame` /
  `OpFreeStack` go away (see § 3.1).
- **Return-address offset.**  The 4-byte slot immediately after the
  argument region is the return address.  Codegen owns this; V2 only
  sees the resulting `local_start`.
- **Native codegen (`src/generation/`).**  Reads `stack_pos` as an
  opaque u16.  V2's placements flow through unchanged — native
  codegen has no allocator logic of its own and does not need to
  change.
- **`compute_intervals` (`src/variables/intervals.rs`).**  V2 trusts
  the `first_def` / `last_use` values this pass computes, including
  the loop-carry extension at lines 110–132.  If V2's behaviour
  diverges from V1's on a particular fixture, the divergence is
  either in the colouring algorithm (§ 2) or in the IR rewrite (§ 3.2)
  — never in the intervals.  Any fixture that requires
  interval-analysis changes is out of scope for this redesign and
  gets flagged as a separate plan item.

---

## Implementation sketch for Phase 2

Phase 2 builds `src/variables/slots_v2.rs` behind
`LOFT_SLOT_V2=validate`.  The sketch:

```rust
pub fn assign_slots_v2(function: &mut Function, local_start: u16) -> u16 {
    let mut intervals: Vec<LocalInterval> = Vec::with_capacity(
        function.variables.len(),
    );
    for (i, v) in function.variables.iter().enumerate() {
        if v.argument || v.first_def == u32::MAX {
            continue;
        }
        let sz = size(&v.type_def, &Context::Variable);
        if sz == 0 { continue; }
        intervals.push(LocalInterval {
            var_nr: i as u16,
            live_start: v.first_def,
            live_end: v.last_use,
            size: sz,
            kind: slot_kind(&v.type_def),
        });
    }
    intervals.sort_by_key(|i| (i.live_start, i.var_nr));

    let mut placed: Vec<(u16, u16, SlotKind, u32, u32)> = Vec::new();
    let mut hwm = local_start;
    for i in &intervals {
        let mut candidate = local_start;
        'retry: loop {
            for p in &placed {
                if i.live_end < p.3 || i.live_start > p.4 { continue; }
                let end = candidate + i.size;
                let blocks_reuse = p.2 != i.kind
                    || (matches!(i.kind, SlotKind::RefSlot) && p.1 != i.size);
                if (blocks_reuse || candidate < p.0 + p.1 && end > p.0)
                    && candidate < p.0 + p.1
                {
                    candidate = p.0 + p.1;
                    continue 'retry;
                }
            }
            break;
        }
        function.variables[i.var_nr as usize].stack_pos = candidate;
        function.variables[i.var_nr as usize].pre_assigned_pos = candidate;
        placed.push((candidate, i.size, i.kind, i.live_start, i.live_end));
        hwm = hwm.max(candidate + i.size);
    }
    hwm
}
```

`slot_kind` maps `Type::Text | Type::Reference | Type::Vector(_, _) |
…` (all DbRef-handle types) → `RefSlot`, everything else → `Inline`.
No alignment bins — variables pack at arbitrary byte offsets.

The IR rewrite from § 3.2 lands in `src/scopes.rs` alongside
`scan_set` / `inline_struct_return`.  Phase 2's equivalence harness
runs both V1 and V2 on every test-compiled function, compares slot
positions, and asserts the correctness gates from § 5.

---

## Open questions flagged for Phase 2

- **Ref-slot size discriminant.**  Step 6c rejects RefSlot reuse
  when sizes differ.  V1 additionally requires the *type
  discriminant* to match (`slots.rs:422–423`).  Size alone should
  be sufficient because `size(Type)` already distinguishes Text
  (24 B), owned DbRef (12 B), and Vector (16 B).  Phase 2 verifies
  this by running the suite with discriminant-check off and
  discriminant-check on; if both pass, drop the discriminant check
  entirely.
- **Alignment.**  V2 does not align slots.  V1 packs densely and
  fixture 5 (`cond+1=4 v+8=5`) proves runtime reads handle arbitrary
  byte offsets.  If a future native-codegen pass needs aligned
  offsets for specific types, it can add the alignment as a
  separate pass over the `SlotAssignment` list without changing the
  allocator.
- **Zero-sized intervals.**  Arguments of type-only (zero-sized)
  variables are already excluded from the input (filter
  `v.first_def != u32::MAX` + `sz > 0`).  Phase 2 adds an assertion
  that the input vector never contains `size == 0` to catch
  regressions.
