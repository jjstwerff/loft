<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 04 — Struct-reference tuples (E5 × D1, D2) — closes T1.8c

**Status: open**

## Goal

Close the long-standing T1.8c (struct-ref tuple element move-vs-copy
semantics) bug by **deciding the semantics in writing first**, then
implementing it, then un-ignoring `tuple_struct_refs` and adding the
remaining E5 cells under the [phase-00 cross-mode harness](
00-matrix.md#cross-mode-harness).

## Decision (filled in during phase 04)

> **Decision:** _to be recorded here at the start of the phase_ —
> Move semantics (preferred per PLANNING.md § T1.8c) or copy + null.
> Reviewer sign-off date: ____.
>
> Rationale: ____.

The phase does not start writing code until this Decision section is
filled in.  TUPLES.md and DESIGN_DECISIONS.md get the same wording so
future readers don't re-litigate.

## Cells closed

| Cell | Test name | Notes |
|---|---|---|
| E5×D1 swap | `e5_d1_struct_ref_local` | Un-ignore `tuple_struct_refs` (`tests/expressions.rs:993`) |
| E5×D1 read after destruct | `e5_d1_struct_ref_swap` | `(q1, q2) = two_points(p1, p2); q1.x ...` |
| E5×D2 arg | `e5_d2_struct_ref_arg` | Pass tuple-of-Reference as arg, read fields back |
| E5×D2 return | `e5_d2_struct_ref_return` | `#[ignore = "T1.8a"]` + requires phase 04 fix |
| E5 mixed (Reference, integer) | `e5_d1_ref_int_local` | Mixed reference + scalar elements |
| E5 mixed (Reference, text) | `e5_d1_ref_text_local` | Reference + owned text |

## E6 — what about "structure values"?

Loft has no inline by-value struct distinct from `Type::Reference`.
A `struct Foo { ... }` declaration produces a record laid out in a
store; the loft-level "value" is a `Reference(struct_def, dep)` —
identical to E5.

Therefore E6 is folded into E5 in this plan and the matrix.  This
phase records the folding in TUPLES.md and DESIGN_DECISIONS.md so a
future contributor proposing a new `Type::StructValue` variant finds
the rejection on the first search.

## The fix (sketch — finalised after Decision section)

If **move semantics**:

- `scopes.rs:578` — tuple scope-exit gate: when destructured into
  variables of `Type::Reference`, the tuple's own elements are moved
  out; do not emit `OpFreeRef` for them.  Verify the destination
  variables get normal scope-exit cleanup.
- The destructured variables must carry `Type::Reference` (not
  `Type::Unknown`) so `OpFreeRef` fires at their scope exit.
  `parse_destructure` (in `src/parser/expressions.rs`) propagates the
  element type to each LHS name.

If **copy + null**:

- New opcode `OpNullTupleElem(var, offset)` that sets the DbRef's
  `store_nr` to the null sentinel.
- Codegen emits `OpNullTupleElem` for each Reference element at
  destructuring time.
- `OpFreeRef` on the tuple at scope exit becomes a no-op for the
  null'd elements.

The phase write-up locks one path before any code lands; the other is
recorded as "considered, rejected, here's why" in
DESIGN_DECISIONS.md.

## Snippets (illustrative)

```loft
// e5_d1_struct_ref_swap (current `tuple_struct_refs`, un-ignored)
struct Point { x: integer, y: integer }
fn two_points(a: Point, b: Point) -> (Point, Point) { (b, a) }

p1 = Point { x: 1, y: 2 };
p2 = Point { x: 3, y: 4 };
(q1, q2) = two_points(p1, p2);
print("{q1.x},{q2.x}\n");
assert(q1.x == 3 && q2.x == 1, "ref swap");
```

```loft
// e5_d2_struct_ref_arg
struct P { v: integer }
fn show(t: (P, P)) {
    print("{t.0.v},{t.1.v}\n");
    assert(t.0.v == 10 && t.1.v == 20, "ref arg");
}
a = P { v: 10 };
b = P { v: 20 };
show((a, b));
```

## Pre-flight

```bash
# Confirm tuple_struct_refs still fails today (use-after-free / wrong
# field) before the Decision section is filled in.
cargo test --release --test expressions tuple_struct_refs -- --nocapture 2>&1 | tail -30
```

The pre-flight output drives the Decision section — if the failure
mode is double-free under copy semantics, move semantics is preferred;
if it's a stale read after a destructure, copy + null may be needed.

## Acceptance

- Decision section filled in with reviewer sign-off.
- `tuple_struct_refs` un-ignored and green under `tests/expressions.rs`.
- 5 cross-mode E5 cells green; 1 ignored carrying the T1.8a tag.
- TUPLES.md "known limitations" SC-3 / SC-6 (DbRef element rules)
  reflects the implemented semantics.
- DESIGN_DECISIONS.md gains an entry "E6 / Type::StructValue —
  rejected; loft has no inline value structs distinct from
  Reference" with this phase's commit as the cross-reference.
- PLANNING.md § T1.8c marked completed with commit hash.
- `make ci` green.

## Risks

| Risk | Mitigation |
|---|---|
| Move semantics breaks an unrelated consumer that today relies on the tuple still being readable post-destructure | Pre-flight grep for tuple-then-read patterns in `tests/`, `default/*.loft`, and `lib/*`.  If any exist, the phase pivots to copy + null; document the pivot in the Decision section. |
| Copy + null introduces a new opcode at a time when the opcode space is near-saturated (254/256 used per CHANGELOG) | If we go copy + null, the implementation reuses an existing free DbRef-write opcode plus a literal null-store-nr operand instead of adding a new opcode.  Document the chosen encoding. |
| Native codegen handles the chosen semantics differently from interp | Cross-mode harness catches it on the first cell test.  Treat any divergence as a phase-blocker, not a follow-up. |

## Out of scope

- E5 D3 (Reference inside a struct field) — covered by phase 05.
- Closure-element tuples — phase 03.
- Tuple-returning functions for Reference elements — gated on T1.8a.

## Cross-references

- [PLANNING.md § T1.8c](../../PLANNING.md) — original bug
- [TUPLES.md § known limitations](../../TUPLES.md)
- `tests/expressions.rs:993` — `tuple_struct_refs`
- `src/scopes.rs:578-587` — tuple scope-exit stub
- `src/parser/expressions.rs::parse_destructure`
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) — destination for
  the "no Type::StructValue" entry
