<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 152 — Let the author choose the fallback, instead of taking the type's default

## Status

**Rewritten 2026-09-02 (owner call).** The plan opened as *"carry validity beside the
value"*; four measurement phases retired that mechanism and shipped the one real defect it
was aimed at. What is left, and is now the whole plan, is the owner's ask: **a fit-failing
operation should be able to take a fallback the AUTHOR chose — `?? 255` — instead of
silently taking the type's default.** Opt-in: write nothing and behaviour is unchanged.

Retired: the validity flag (Phase A measured it a **net +0.5–0.8 % slowdown**, so its own
performance argument is refuted) and phases F–H that depended on it. Shipped from the
measurement phases: loft#1305 and loft#1306. The record is in
[MEASUREMENTS.md](MEASUREMENTS.md).

Tracker: [@PLN152](https://github.com/loft-lang/plans/issues/152).

## Goal

`?? <value>` supplies the fallback for a fit-failing operation on a declared-narrow slot,
where today only `uncomputable_default`'s choice is possible.

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
| **P2** — decide the SPELLING for the narrow case (§ open questions), and write it down before implementing | § open questions | a design note the owner signs off; no code | Open |
| **P3** — route an author-written fallback into `OpRangeDefault`'s `dflt` at the compound-assign seam | `parser/expressions.rs::compound_range` | the axis matrix's narrow cells answer the author's value; **every cell with no `??` is unchanged** | Open |
| **P4** — the remaining seams: field, element, argument, return | `guard_declared_range` | one cell per seam on both backends; `integer`/`i32` controls unmoved | Open |
| **P5** — a non-constant fallback, or a decision that it stays constant | `dflt` is `const integer` | either a cell with a variable fallback, or a recorded decision saying why not | Open |
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

## See also

- [MEASUREMENTS.md](MEASUREMENTS.md) — phases A/B/C/E, and the two corrections they forced on
  this plan's own premise.
- [`formal/types.md`](../../formal/types.md) § Null-flow laws · `(E-Uncomp-NN)` in
  [`formal/operational.md`](../../formal/operational.md) — the rule that names the default.
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C85 (why arithmetic stays non-null), C80.
- [COMPATIBILITY.md](../../COMPATIBILITY.md) — why opt-in is not pre-freeze-bound.
- Shipped from the retired phases: loft#1305, loft#1306.
