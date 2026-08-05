<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# F9 — a `&` link always writes through; reshaping under one is REFUSED (D-bind-8)

**Status — DESIGN, not started.** Tracker: [loft#779](https://github.com/loft-lang/loft/issues/779) ·
deviation [`formal/binding.md` D-bind-8](../../formal/binding.md) · lock-ins
`d_bind_8_*` in `tests/parse_errors.rs`.

## The rule

Two halves. The first is already in the spec; the second is the maker's decision of 2026-08-05
and is a spec ADDITION.

1. **B-Ref-Alias, unchanged and unconditional** — *"the `&τ` annotation makes ANY binding a live
   LINK to the source"*, so every write through a `&` reaches the source. Restated by the maker:
   *"A reference to any structure variable or otherwise should allow for writing it too. In all
   cases."*
2. **NEW — removing from a container while a `&` link into it is open is REFUSED AT COMPILE
   TIME.** Maker, 2026-08-05: *"The removal of anything from a structure (vector for example)
   that has an open `&` relation (for us an edge case) should be forbidden on compile time."*

Together these are total and need **no runtime machinery at all**: a `&` always writes through,
because the one program shape where it could not is rejected before it runs. This is the rustc
bargain in loft's spelling — where rustc refuses a mutation while a borrow is live, loft refuses
the removal; and it is affordable precisely because the maker classes it an **edge case**, so no
real program pays for it.

**What does NOT change: @PLN130 F2 stays exactly as shipped for a PLAIN bind.** `c = v[0]` copies
(or mimics a copy), so materialise-plus-advice remains right for it — maker: *"When we copy data
… then we obviously do not want write through."* The whole arc is about `&` only, and the two
paths must end up disjoint:

| binding | container reshaped while it is live | outcome |
|---|---|---|
| plain `c = v[0]` | yes | materialise + advice (F2, shipped, unchanged) |
| `c = &v[0]` | yes | **compile-time error** |
| `c = &v[0]` | no | live link, writes through (already correct) |

## Root cause — the `&` is thrown away before any decision can use it

`c = &v[0]` and `c = v[0]` compile to **byte-identical IR**:

```
c(1):ref(Box)["v"] = OpGetVector(v(1), 16i32, 1i32);      // both spellings
```

(verified with `loft introspect`; the only diff is line numbers). For a **struct-typed**
projection the two were equivalent under B-View — both alias — so the parser drops the `&` as
redundant, silently. Harmless until F2 made a view materialise on a reshape: from that moment the
spellings stopped being equivalent, and the discarded `&` became load-bearing information that no
longer exists.

D-bind-0 closed on *"`&τ` is now `Type::RefVar`, a reference type the variable carries"*. True for
**parameters** (they render `&vec<ref(N)>`); not true for a local struct projection.

**Measured today** (probe 38 / `LOFT_VAR_TABLE`), all three silently dropping the write:

| | shape | today | why |
|---|---|---|---|
| **(a)** | `c = &v[0]; v.remove(2)` — element does NOT move | 11, write dropped | materialise fires on a `&`, nothing marks it as one |
| **(b)** | `c = &v[2]; v.remove(0)` — element moves 2→1 | 33, write dropped | same |
| **(c)** | `&` param, callee does the `remove` | 33, **no diagnostic at all** | materialise does not fire (a `&` param IS distinguishable); nothing adjusts the link |

Under the decided rule **all three become compile-time errors**, so the fix direction is one
mechanism (detect + refuse) rather than three.

## What the refusal decision removes from this design

An earlier draft of this file had a *"make the link follow its element"* step — emit
`if link.pos > removed.pos { link.pos -= elem_size }` at each removal. It is deleted. It was the
only step with real risk (runtime arithmetic per link, a disjointness requirement against the
plain path, and an undecided sub-case for a link to the *removed* element). The refusal answers
all of that by construction, and the undecided sub-case evaporates: there is no "link to a
removed element" if the removal is rejected.

Recorded because the reasoning is worth not re-deriving: following was *feasible* — a vector stays
DENSE (F3 decision 1), so the new position is arithmetic rather than a lookup, and the live-link
set is already computed. It is simply not worth it for an edge case.

## Steps

Each is independently landable, has its own gate, and leaves the tree green.

### Step 1 — make F2's materialise LIVENESS-aware ✅ DONE (2026-08-05)

**Shipped.** `collect_views_to_materialise` now answers both causes and the flat
`reshaped_containers` set is gone; `tests/scripts/148-view-liveness-across-reshape.loft` is the
CI guard, `probes/39-f2-liveness-boundary.loft` the 12-cell boundary. The merge blocker is
cleared. Details in § Step 1, as shipped, below.

Before any `&` work: F2's reshape path keyed on the **container** and was **order-blind**. It
materialised any view of a container reshaped anywhere in the function, even one that is dead
long before the removal. Measured, plain bind, both backends:

```loft
c = v[0];
c.n = 99;          // the view is DEAD from here
v.remove(2);
// v[0].n == 11 on this branch;  == 99 on the installed mainline binary
```

**This is a regression F2 introduced on this branch**, not a pre-existing gap — mainline has no
materialise and lands the write. Two closure-bar violations at once: a **lost write**, and
**untrue advice** (*"`v` is modified while `c` is in use"* when `c` is not in use). F2 is not on
`main`, so nothing has shipped — which is exactly why this must be fixed before the branch merges.

The fix is available and already built: **F8's `collect_views_to_materialise` is keyed on the
VIEW and is liveness-aware** (it walks blocks in order and marks only a view live across the
event). The reshape cause simply does not route through it — it still consults the flat
`reshaped_containers` set. Route it through the same walk, treating a `remove` / `#remove` of the
container as the event instead of a re-establishment.

- **Fixes:** the lost write and the false advice for plain binds; reduces over-copying.
- **Also the prerequisite for step 3**, whose refusal condition is *"a `&` link LIVE across the
  removal"* — the same liveness question.
- **Gate:** `f2_dead_view_before_removal_keeps_writing_through` (lock-in, `tests/parse_errors.rs`)
  flips to PASS; `145`, `146`, `201`, `774`, `147` unchanged; the advice fires only where the view
  really is live; both backends; `LOFT_STRICT_STORES` clean; full suite green. Re-run probe 03–07
  and 29, which are F2's own evidence — they must stay green, and they are the cells that prove
  the liveness walk did not go too far the other way.
- **Risk: low-medium.** It narrows when a copy happens, so the failure direction is a stale view
  coming back — which probes 03–07 and 29 are exactly the guard for.

#### Step 1, as shipped

**The design was right about the destination and wrong about the vehicle.** F8's walk is keyed
on the VIEW and walks blocks in order, but *order is not liveness*: it marks a view bound before
the event, which is exactly what cell L1 (`c = v[0]; c.n = 99; v.remove(2)`) is. Routing the
reshape cause through it unchanged would have fixed L5 and L8 and left the headline regression
in place. So the walk gained the missing half rather than merely a second caller:

> A disturbance only **SHAKES** the open views of that container. A later read or write of a
> shaken view is what **condemns** it. A view whose last use precedes the disturbance keeps its
> alias and writes through.

That is one rule for both causes, so `reshaped_containers` is deleted as a field and the
per-statement helper feeds the same walk; `views_to_materialise` became a
`HashMap<u16, ViewCause>` so the two advice lines still read differently (`Reshaped` wins when
both apply). The cause split is now carried as data instead of re-derived at the strip site from
*"is the container in the reshaped set"*, which was the thing that could disagree with itself.

**One measurement decided the shape of the walk, and code-reading would not have found it.**
The first version shook and then read uses over the WHOLE statement before recursing into it —
so for the function's top-level block it re-scanned every statement inside, and `c.n = 99`
(which runs *before* the removal) counted as a use *after* it. L1 stayed broken with the
analysis "working". One env-gated print of the condemning statement named the block instantly;
the fix is to handle uses at LEAF granularity and let the recursion carry order. Kept as the
reason `walk_stmt` looks the way it does: the whole-statement shake survives only for `Loop`
(where the next iteration really does put the disturbance before the body's uses) and for forms
the walk cannot descend into (a `match` arm), where coarse-in-both-directions is the safety net.

| | before | after |
|---|---|---|
| L1 dead before the removal | 11, write lost | **99** |
| L5 bind in a block that closed | 11 native / 99 interp — a backend split | **99** both |
| L8 removal before the bind | 11, write lost | **99** |
| L10 one dead + one live view | 11022, both stripped | **99022** |
| L2/L3/L4/L6/L7/L9 (F2's own evidence) | unchanged | unchanged |
| advice lines emitted | 13, four of them untrue | **9**, all on views genuinely in use |

Probes 03–07, 26, 28, 29, 30, 35, 36, 37 and 38 re-run clean on both backends;
`LOFT_STRICT_STORES` reports nothing on probe 39, so no cell passes by leaking.

**Known lower bound, recorded rather than papered over:** a view bound inside a nested block and
used on a LATER iteration of an enclosing loop is not tracked, because the frame closes with the
block. A disturbance anywhere inside a loop shakes every view held from *outside* it before the
body is walked, so the ordinary loop shapes are covered; this one keeps today's behaviour.

### Step 2 — give `&` a representation that survives to the check (INERT)

Mark the binding when the parser sees `&` at a projection bind — a flag on the variable, **not** a
type change. Nothing reads it yet.

- **Why a flag, not `Type::RefVar` for locals.** Making a local RefVar would re-route every read
  and write through the double indirection params use — slower on every access, which is exactly
  what loft's own advice warns about for a redundant `&` param — and would touch every codegen
  dispatch site. A marker bit keeps today's single-indirection representation and informs only the
  new check. Cheapest thing that carries the fact.
- **Gate (behaviour-preserving refactor, per the loft-codegen skill):** emitted IR **and**
  generated native Rust **byte-identical** before/after, on a corpus with `&` locals, `&` params
  and plain projections. The flag is observable only via `LOFT_VAR_TABLE` / `LOFT_DEBUG_F8`. Full
  suite green.
- **Risk: low.** Adds a fact, changes no decision.

### Step 3 — refuse a removal while a `&` link into that container is LIVE (same function)

At each `remove` / `#remove` site, if any `&`-marked link into that container is live across it,
report a compile-time error naming both the removal and the link.

- **Liveness is the condition, not existence** — the rustc rule. A `&` link that is dead before
  the removal is no conflict, and `c = &v[0]; c.n = 1; v.remove(0);` must keep compiling. The F2
  liveness walk (`collect_views_to_materialise`, added for F8) already computes exactly
  "is this binding live across this operation", so this reuses it rather than inventing one.
- **Also exempt `&` from the materialise here.** Once the dangerous shape is refused, a surviving
  `&` link is by definition not live across a removal, so it must keep its dep and write through.
  This is safe *only* in combination with the refusal — on its own, exempting `&` would turn
  case (b)'s dropped write into a write to the vacated slot, trading a lost update for possible
  corruption. That pairing is the single most important ordering constraint in this design.
- **Fixes:** (a) and (b).
- **Gate:** both same-function lock-ins report the error, with the caret on the removal; `145`,
  `146`, `201`, `774`, `147` unchanged (they pin plain binds and must not move); a positive cell
  where the link is dead before the removal still compiles and still writes through; both
  backends; `LOFT_STRICT_STORES` clean; full suite green.
- **Risk: medium**, and it is a COMPATIBILITY risk rather than a correctness one — see below.

### Step 4 — the cross-frame case (c)

The callee removes from a `&vector` param while the caller holds a `&` link into that container.
Two places the check can sit, and the caller is the honest one:

- **At the call site (preferred).** The caller can see that an argument is a projection of another
  argument (`shift(v[2], v)`) and that it holds a live `&` link into `v`. Needs a per-definition
  fact *"this callee removes from `&` param k"* — prototyped as `reshaped_ref_params` during the
  audit, then reverted with the reasons recorded at `collect_reshaped_containers`.
- **In the callee (fallback, coarser).** A `&`-element param plus a removal from a `&`-container
  param is suspicious without knowing they are related — it would refuse some sound programs, so
  it is second choice.

One frame deep only. A reshape two calls down keeps today's behaviour and gets its own
`#[ignore]`d lock-in, which is a lower bound in the safe direction.

- **Fixes:** (c), probe 26, probe 38 cell A1; closes loft#779.
- **Gate:** the third lock-in reports; probes 26 and 38 green on both backends; probe 38 graduates
  to `tests/scripts/`.
- **Risk: medium.**

### Step 5 — close the deviation and state the rule

- Add the refusal to `formal/binding.md` as a **rule** (it is a spec addition, not a correction) —
  suggested name **B-Ref-Reshape** — beside B-Ref-Alias, with its own conformance lock-ins.
- Delete D-bind-8; `OPEN` returns to **0**.
- Replace the *"known gap"* wording in
  [OWNERSHIP_MODEL.md § A view lasts as long as the thing it names](../../OWNERSHIP_MODEL.md) and
  the C86 lifetime paragraph in [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) with the shipped
  two-row rule (plain → materialise; `&` → refused).
- `LOFT.md`: the user-facing paragraph gains the `&` row, since the error is something an author
  will meet.

## The compatibility question, which must be answered before step 3 lands ⚑

**Adding an error can stop compiling a program that compiles today** —
[COMPATIBILITY.md](../../COMPATIBILITY.md), and *"compatibility is ABSOLUTE"*. The argument that
this is nonetheless allowed:

- Every program the error rejects is **already silently wrong** — it loses a write with no
  diagnostic. Refusing it converts silent breakage into a compile-time message, which is the
  direction the closure bar demands, not a regression.
- It is an **edge case** by the maker's own classification, so the blast radius is small.

That argument still has to be checked against the contract-freeze machinery rather than assumed:
`CONTRACT_VERSION` and whether this counts as a pre-freeze error-add (D-const-1 in `binding.md`
was one, and recorded as such). **Do this check first** — if the freeze has passed, the refusal
needs the edition-style valve (@PLN113 contract-keying) instead of landing unconditionally.

## Deliberately out of scope

**A `&` link across a keyed-collection RE-KEY.** F4 treats a key-field write as a reshape and
materialises the view; for a `sorted` the record genuinely moves. Same class, and under the
decided rule it should presumably be refused too — but a re-key is a *write*, not a removal, so
the maker's sentence does not literally cover it. Needs its own decision. Recorded so nobody
assumes step 3 covered it.

## Reading order for the next session

1. This file.
2. `formal/binding.md` § D-bind-8 — the rule, the falsifying programs, the root cause.
3. `probes/38-refparam-view-across-reshape.loft` — the measured boundary as cells, including the
   two CONFORMANT ones (B1, C1) that localise the defect to the frame boundary.
4. `tests/parse_errors.rs::d_bind_8_*` — the three lock-ins.
