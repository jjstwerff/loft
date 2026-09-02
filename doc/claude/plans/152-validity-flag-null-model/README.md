<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 152 — Retire the silent default: a fit-failing narrow store must be typed, not defaulted

## Status

**Rewritten 2026-09-02 (owner call).** The plan opened as *"carry validity beside the
value"*; four phases of measurement retired that mechanism and left one thing genuinely
wrong, which is now the whole plan: **a fault reaching a narrow non-null slot takes the
type's DEFAULT — a legal, in-range value indistinguishable from a computed one.**

What the measurement phases settled, and why they redirected the plan, is in
[MEASUREMENTS.md](MEASUREMENTS.md). The short version: the defect list was already banked by
other work (B), the collapse site already existed and already reported (C), the one real hole
closed with two lines at that site (E, shipping loft#1305 + loft#1306) — and the flag itself
measured a **net +0.5–0.8 % slowdown**, so its performance argument is refuted by its own
Phase A. **The flag is retired.** Phases F–H that depended on it are dropped.

Tracker: [@PLN152](https://github.com/loft-lang/plans/issues/152).

## Goal

A fit-failing operation whose target is a declared-narrow slot types `τ?` and requires
discharge, so the language never silently substitutes a value the author did not choose.

## The rules already prescribe this

This is not a design proposal. [`formal/types.md`](../../formal/types.md) says it:

> `a op b` and `e as τ` are **non-null when the result provably fits** the target range …
> and `τ?` only when the range could miss (a narrowing `as`, **a declared-narrow slot**, a
> genuinely i64-overflowing product).
>
> Overflow-to-null is therefore the *correct* runtime behavior … **the work is to type it
> and require discharge**.

*A declared-narrow slot* is named explicitly. So `x: u8 = 250; x += 10` should type `u8?`
and demand a discharge; it instead types `u8`, takes `0`, and says nothing. **That is a
deviation from a written rule, not a decided edge** — which is what makes this plan
obligatory rather than optional.

## Why the narrow case is different from C85, precisely

[C85](../../DESIGN_DECISIONS.md) keeps `+ - *` typed non-null because forcing `integer?` on
all arithmetic *"poisons the common path to guard a fault that essentially never fires"*.
That is a **proportionality** argument, and it is calibrated for `i64`: overflow there needs
operands around 3 × 10⁹.

**For a `u8` the same fault fires at 256.** The exemption's premise is simply false at narrow
widths, so extending it there was a mis-application rather than a decision. C85 does not need
reversing — it needs its scope stated: the exemption is an i64 judgement.

This is also why the burden stays small. Nullability is **range-driven**, so range-tracking
already proves the fitting cases and they keep their non-null type: `(x & 255) as u8` and
`(non-neg) % c as u8` demand nothing today and will demand nothing after.

## Scope

**In** — the widths with no usable sentinel, which are exactly the ones that default:
`u8`, `i8`, `u16`, `i16`, and `u32` (whose spare code is the top one, which no non-null read
tests for).

**Out** — plain `integer` and `i32`. They keep a bottom code back, already answer `null`, and
C85 governs them unchanged. Also out: the validity flag, and any change to `OpRangeDefault`'s
runtime behaviour. **This plan changes what a fit-failing narrow op is TYPED, not what the
runtime does with it.** The collapse stays where Phase E put it; it simply stops being
reachable without the author having said what should happen.

## ⚠ Pre-freeze only

Requiring a discharge is **ADDING an error**, and
[COMPATIBILITY.md](../../COMPATIBILITY.md) § *The error surface is one-directional* says loft
may never add one after contract 1. So this lands before the freeze or not at all — and that
asymmetry is also the argument for doing it now rather than deferring: dropping the
requirement later is always legal, re-adding it never is.

## Composition matrix — Stage A

Axes this change touches: **target width** (`u8`/`i8`/`u16`/`i16`/`u32` in; `i32`/`integer`
as the must-not-move controls), **nullability** (`τ` vs `τ?` — the nullable spelling already
answers null and must stay burden-free), **the seam** (local compound assign · field store ·
element store · call argument · return · `par` merge · deserialisation), and **fit
provability** (provably-fits must stay non-null; provably-misses and cannot-prove must type
`τ?`). Backends: both, though this is a parse-time change and the runtime is untouched.

The existing probes are the before-half: [`probes/`](probes/) (19 value cells + the refusal
and diagnostic channels) and [`probes/axis/`](probes/axis/) (overflow shape × storage kind).
Their expectations change only in the cells this plan intends to move, which is the gate.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **N1** — measure the real conversion set: instrument the parser to count narrow-target seams where the fit is NOT provable, and confirm range-tracking already clears the provable ones | this README | the count, plus a hand-checked sample; a burden far above the ~16 files a first grep suggests kills the clean design and routes to a narrower seam set | Open |
| **N2** — type the result `τ?` at the compound-assign seam where the fit is not provable | § the rules | the axis matrix's narrow cells stop answering `0` and start demanding a discharge; the provably-fitting cells are UNCHANGED (the control that a blanket rule would fail) | Open |
| **N3** — the remaining narrow-target seams: field, element, argument, return | `guard_declared_range` / `compound_range` | one cell per seam, both backends; the `i32` and `integer` controls must not move | Open |
| **N4** — convert the in-tree sites, then the published-lib gate | `scripts/revalidate_libs_local.sh` | `make ci`, then the lib gate — a language change is not green until the published libraries build | Open |
| **N5** — rules + decision record: close the deviation, and state C85's exemption as an i64 judgement | `formal/types.md`, `DESIGN_DECISIONS.md` | `scripts/rule_tags.py check`; the deviation register moves | Open |

## Phase ordering

1. **N1 first, and it can kill the plan.** The whole case rests on the burden being
   proportional. If the un-provable set is large, the answer is a narrower seam set, not a
   blanket rule — and better to learn that from a count than from a converted tree.
2. **N2 before N3**: the compound assign is the seam the defect was filed at, and it is the
   one with an existing matrix.
3. **N4 cannot be skipped.** `make ci` green is not the bar for a language change; the
   published libraries are.
4. **N5 last**, so the rules record what shipped rather than what was intended.

## Open design questions

1. **What happens at a seam that cannot be typed** — a `par` merge, deserialisation, a store
   image read. Those write a narrow slot without an expression to type. The collapse still
   applies there; whether it should also report is N3's question, not N2's.
2. **Does `u32` belong with the four, or with `i32`?** Its spare code exists but is at the
   top, where no non-null read tests for it. Phase B measured it answering the default like
   the four; the rules describe it that way. Confirm it is not a third case.
3. **Is the discharge the right ergonomics, or should a narrow slot get a declared
   saturating/wrapping intent?** `x: u8 = 250; x += 10` demanding `?? 0` is honest but
   verbose where the author wanted saturation. Out of scope here — but if it is the real
   answer, N2 is the wrong shape, so it is worth asking before N2 rather than after.

## See also

- [MEASUREMENTS.md](MEASUREMENTS.md) — phases A/B/C/E: what was measured, and the two
  corrections it forced on this plan's own premise.
- [`formal/types.md`](../../formal/types.md) § Null-flow laws — the rule this plan makes hold.
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C85 (the i64 exemption), C90 (the in-band
  residual), C80 (the spreadsheet model, unchanged).
- [COMPATIBILITY.md](../../COMPATIBILITY.md) § The error surface is one-directional.
- Shipped from the retired phases: loft#1305, loft#1306.
