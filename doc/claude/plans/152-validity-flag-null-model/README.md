<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 152 — A fit-failure the author can choose, and can see

## Status

**Rewritten 2026-09-02 (owner call).** The plan opened as *"carry validity beside the
value"*; four measurement phases retired that mechanism and shipped the one real defect it
was aimed at. What is left, and is now the whole plan, is the owner's ask: **let an author opt in to
correct code for the fit-failure edge**, which is currently hard or impossible to get right.
Opt-in throughout — write nothing and behaviour is unchanged, so no existing program moves
and no error is added.

Shipped from the measurement phases: loft#1305 and loft#1306. The record is in
[MEASUREMENTS.md](MEASUREMENTS.md).

**The validity boolean is back, scoped.** It was retired on two grounds; one still holds and
one did not survive arc B. Its PERFORMANCE argument is still refuted — Phase A measured the
carry at a net **+0.5–0.8 % slowdown**, so it is not an optimisation. But *"correctness does
not need it"* was true only of the defects known then. **Arc B cannot be built without an
out-of-band bit**, by construction, and § Why arc B needs a bit says exactly why and exactly
how much.

Tracker: [@PLN152](https://github.com/loft-lang/plans/issues/152).

## Goal

**Make the fit-failure edge handleable, without changing what happens when nobody handles
it.** This is not new semantics: C80 and C85 stand, the default stays the default, and a
program that says nothing behaves exactly as it does today. What is added is a way for an
author who *cares* about the edge to write correct code for it — which today is hard or
impossible.

That is the whole test of this plan. Not *"does the language do something better by
default"* — it should not — but: **can an author who cares write a correct handler?**

### What "a correct handler" needs, and why today's answer is no

Writing one takes two things, and the narrow non-null slot supplies neither:

| the author needs to… | today |
|---|---|
| **know it happened** | impossible — `x = 0` after a failure is indistinguishable from a computed `0`; `!x` false, `x == null` false |
| **choose what happens instead** | not reachable — `uncomputable_default` picks it, and `??` cannot see the failure because `260` is not a null |

Either one alone is insufficient, which is why this plan carries both arcs rather than
picking the cheaper. Detect-only cannot supply a value inline; choose-only cannot branch,
log, or count. **A design that answers one and not the other has not made the case
handleable** — it has made it half-handleable, which is the state that is hard to get right
today.

- **A — choose it.** `?? <value>` supplies the fallback where only the type's default is
  possible now.
- **B — see it.** A fit-failure on a non-null narrow slot can be tested for.

The escape that already exists is *declare it `u8?`*, and it is a real answer — but it is an
up-front decision that changes the type for every downstream reader, which is the "hard"
half of the owner's complaint rather than a refutation of it.

## Half of this already works — and the other half fails for a different reason

Measured 2026-09-02, and it decides the shape of the work:

```loft
fn bump(v: integer, d: integer) -> integer { (v + d) ?? 42 }

bump(big, 1)   ->  42     // the author's fallback, on a RUNTIME overflow
plain(big, 1)  ->  null   // C85's default, unchanged
bump(2, 3)     ->  5      // the ordinary path is untouched
```

So on a type that keeps a sentinel (`integer`, `i32`), `??` **already** catches an overflow
and supplies the author's value, with no static knowledge and no `redundant-coalesce`. That
half needs documenting and a guard, not building.

**The narrow widths fail for a reason that is not "the `??` is missing".** Their fault is
not a null at all:

| the fault | is it null? | can `??` see it? |
|---|---|---|
| `x: integer` overflows → the sentinel | yes | **yes — works today** |
| `x: u8 = 250; x += 10` → `260`, out of range | **no** — 260 is an ordinary number | **no** |

`260` never becomes null, so no coalesce can fire on it. The only thing that handles it is
`OpRangeDefault`'s `dflt` argument, and that argument is always compiler-chosen. **That is
the gap: not a missing operator, a fallback the author cannot reach.**

## B — what can be tested today, and the one shape that cannot

Measured ([MEASUREMENTS.md](MEASUREMENTS.md)):

| shape | today | testable? |
|---|---|---|
| `a: u8 = 300` — a literal that cannot fit | **compile error** | n/a — caught, not dropped |
| `x: u8 = 250; x += 10` | `x = 0`; `!x` false, `x == null` false, `x == 0` true — identical to a real zero | **no** |
| `z: u8? = 250; z += 10` | `z = null` | **yes — `if !z` already works** |

So the owner's `if !a { … }` sketch is already the right spelling and already works — on a
nullable slot. The gap is the **non-null narrow slot**, which by construction has no code for
null to occupy, so there is nothing for `!a` to observe. `x = 0` (fabricated) and `y = 210`
(computed) differ in nothing the program can reach.

## Why arc B needs a bit

**Five of the seven scalar widths cannot encode their own failure** — `u8`/`i8`/`u16`/`i16`/
`u32` answer `0`, where `i32` and `integer` answer `null` because they keep a bottom code back
([MEASUREMENTS.md](MEASUREMENTS.md)). On those five every code is a legitimate datum, so there
is no value that can mean *"this did not happen"*. Arc B needs a channel that is not the
value's own bits: the requirement is structural, not a preference.

It is **not** Phase A's bit, which sat beside every eval-stack slot and has no native
analogue. It is selective — introduced only in the expressions that need it — and
per-variable, which is a thing both backends already have.

**So ordinary arithmetic stays single-variable.** `integer`, `float` and `single` keep a
sentinel and never need a companion bit, which is what bounds the impact on real algorithms:
the cost lands on the five narrow widths, where the fault is frequent and the author asked to
handle it. Phase A's +0.5–0.8 % measured the blanket design on two benchmarks that are plain
`integer` throughout — under this design they would carry no bit and show no change, so that
number does not transfer ([ARC-B-DESIGN.md](ARC-B-DESIGN.md)).

How that is built, why the two candidate spellings differ in cost, and which existing
predicate it must extend rather than duplicate: [ARC-B-DESIGN.md](ARC-B-DESIGN.md).

## The home already exists

Phase E left the collapse at one site both backends share:

```
OpRangeDefault(val, lo, hi, dflt)
```

`dflt` **is** the fallback slot. It is filled today by `uncomputable_default(nullable, spec)`.
The whole feature is letting an author-written value fill it instead — which is why this is a
small change with an exact home rather than a new mechanism.

## Opt-in, and therefore not pre-freeze-bound

Writing nothing keeps today's behaviour, so this **adds no error and changes no existing
program**. Under [COMPATIBILITY.md](../../COMPATIBILITY.md) § *The error surface is
one-directional* that makes it legal at any time, including after contract 1 — unlike the
forced-discharge shape, which would have been pre-freeze-only. The opt-in framing is what
buys that freedom, and it is the reason to prefer it beyond ergonomics.

## Composition matrix — Stage A

Axes: **target width** (`u8`/`i8`/`u16`/`i16`/`u32` — and `integer`/`i32` as the controls
that must not move, since they already work), **fault shape** (out-of-range vs landing on the
sentinel — Phase B pinned this axis and Phase C showed they are different paths), **the
seam** (local compound assign · field · element · argument · return), **fallback kind**
(constant · variable · expression — `dflt` is `const integer` today, which bounds what is
expressible), and **backend**.

The before-half exists: [`probes/`](probes/) and [`probes/axis/`](probes/axis/). A cell where
no `??` is written must be byte-identical to today — that is the opt-in claim, and it is what
the matrix must prove rather than assert.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **P1** — pin what already works: `??` on a sentinel-bearing overflow, both backends, and that it is NOT reported redundant | § half of this already works | a `tests/scripts/` guard; it must FAIL if the coalesce stops firing, which `make falsify` can answer against a build with `??` stripped | Open |
| **P2** — decide BOTH surfaces together: the `??` spelling (arc A) and how a failure is made visible (arc B) | § open questions 1 and 5 | a design note the owner signs off; no code. The gate is that the two compose — a fallback and a test for it must not be two unrelated features | Open |
| **P3** — route an author-written fallback into `OpRangeDefault`'s `dflt` at the compound-assign seam | `parser/expressions.rs::compound_range` | the axis matrix's narrow cells answer the author's value; **every cell with no `??` is unchanged** | Open |
| **P4** — the remaining seams: field, element, argument, return | `guard_declared_range` | one cell per seam on both backends; `integer`/`i32` controls unmoved | Open |
| **P5** — a non-constant fallback, or a decision that it stays constant | `dflt` is `const integer` | either a cell with a variable fallback, or a recorded decision saying why not | Open |
| **B1** — the at-the-store test, if P2 picks it: the fit predicate as an expression where the operands are still in scope | § the spelling decides | a cell per width; the five that cannot represent failure must answer, and `i32`/`integer` must be unchanged (they already do) | Open |
| **B2** — the after-the-store test, if P2 picks it: a per-variable marker set in pass 1, emitted in pass 2, with the bit in a companion slot | § Selective, not blanket | a cell whose status test appears textually AFTER the last store still answers, on both backends and both passes; and every unmarked variable emits byte-identically (`introspect` diff), which is the whole claim of "selective" | Open |
| **P6** — document it: the narrowing error already advertises `?? d`, so the doc and the diagnostic must agree | `DIAGNOSTICS.md`, the reference chapter | the advertised cure works when followed | Open |

## Phase ordering

1. **P1 first.** It is cheap, it protects behaviour that already ships and is currently
   pinned by nothing, and it establishes the guard shape the later phases reuse.
2. **P2 before any code.** The spelling is the one irreversible part — a surface, once
   shipped, is frozen — so it is decided on paper and signed off, not discovered in a patch.
3. **P3 → P4** widens seam by seam, each with its own cell, because a seam that silently
   keeps the compiler's default would otherwise pass by looking unchanged.
4. **P5** is a scope valve: if a non-constant fallback needs a new op, it may be a separate
   plan rather than this one growing.
5. **B1 or B2, not both** — P2 chooses. B1 may need no IR change; B2 needs the stored bit and
   is the only arm to which Phase A's +0.5–0.8 % measurement transfers, so if P2 picks it,
   measure before building, the way A did.
6. **A and B ship together or the case stays half-handleable** (§ Goal). Landing the fallback
   without the test, or the test without the fallback, leaves exactly the state the owner is
   complaining about.

## Open design questions

1. **The spelling — this is the real question, and P2 exists for it.** `x += 10` has nowhere
   obvious to hang a fallback: `x += 10 ?? 255` parses as `x += (10 ?? 255)`, which coalesces
   the operand, not the result. Candidates: `x = (x + 10) ?? 255` (works grammatically, but
   today it is refused by the narrowing check — so the check has to learn that a `??` supplies
   the guard the message already asks for), a declaration-site default
   (`x: u8 = 250 ?? 255` — reads as the wrong thing), or an attribute on the type. **None is
   obviously right; that is why it is a phase.**
2. **Should the narrowing error's advertised cure be made true, or the message changed?** It
   says *"guard the value (`?? d`, mask, or an `if` range check)"* and `?? d` does not
   currently work there. One of the two has to move.
3. **Constant or expression?** `dflt` is `const integer`, so a constant is nearly free and an
   expression needs the fallback evaluated at the collapse. The owner's `?? 1.0` example is a
   literal, so P3 can ship constants and P5 decides the rest.
4. **Does the sentinel arm want it too?** Phase E made a sentinel collapse silently to the
   default on a narrow slot. If an author writes a fallback, it should presumably win there as
   well — one rule for both arms, not two.
5. **How is a non-null fit-failure made VISIBLE (arc B)?** The owner has not picked a
   notation, and neither has this plan. Three directions, none free:
   - **"declare it `u8?`"** — costs nothing, works today, and the diagnostic could simply
     advertise it. But it says a non-null narrow slot is *never* testable, which is a real
     answer only if that is acceptable.
   - **The fallback subsumes it** — with `?? 255` the author chose the value, so there is
     nothing to detect. This covers *do something useful* but not *know that it happened*,
     and it cannot express "I want to branch on the failure".
   - **A failing store yields a testable outcome** — the owner's `if !a = 300 { … }` sketch.
     This is the only direction that answers the question as asked, and § Why arc B needs a
     bit shows it is unavoidable for the five widths that cannot represent their own failure.
     Its cost splits by spelling: tested AT the store it may need no IR change; tested AFTER
     the store it needs a bit that persists.
   Deciding this is P2's job, alongside the `??` spelling, because the two surfaces should be
   designed together or they will not compose.

## See also

- [MEASUREMENTS.md](MEASUREMENTS.md) — phases A/B/C/E, and the two corrections they forced on
  this plan's own premise.
- [`formal/types.md`](../../formal/types.md) § Null-flow laws · `(E-Uncomp-NN)` in
  [`formal/operational.md`](../../formal/operational.md) — the rule that names the default.
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C85 (why arithmetic stays non-null), C80.
- [COMPATIBILITY.md](../../COMPATIBILITY.md) — why opt-in is not pre-freeze-bound.
- Shipped from the retired phases: loft#1305, loft#1306.
