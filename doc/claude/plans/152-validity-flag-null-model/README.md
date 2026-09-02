<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 152 — Let the existing spellings reach the types they cannot reach today

## Status

Open — design settled in shape, spelling constrained, nothing built. Shipped along the way:
loft#1305 and loft#1306. The measurement record that redirected this plan three times is in
[MEASUREMENTS.md](MEASUREMENTS.md); the arc-B design detail is in
[ARC-B-DESIGN.md](ARC-B-DESIGN.md).

Tracker: [@PLN152](https://github.com/loft-lang/plans/issues/152).

## Goal

**Let a programmer write code that handles the special type cases correctly — using the
spellings loft already has.**

- **No new arithmetic.** C80 and C85 stand. Operators do not change, results do not change,
  and a program that says nothing behaves exactly as it does today.
- **No new syntax.** `??` and `!` already mean *"give me a fallback"* and *"did this not
  happen"*. The work is making them REACH the cases where they currently cannot — not adding
  a construct.
- **Opt-in.** Absent those spellings, emission is byte-identical. Adding no error also keeps
  this outside [COMPATIBILITY.md](../../COMPATIBILITY.md)'s one-directional rule, so it is
  legal at any time rather than pre-freeze-only.

The test of this plan is one question: **can an author who cares write a correct handler,
without learning anything new?**

## What the two spellings already mean, and where they stop

Both already work — on the types that keep a spare code. Measured
([MEASUREMENTS.md](MEASUREMENTS.md)):

```loft
fn bump(v: integer, d: integer) -> integer { (v + d) ?? 42 }
bump(i64::MAX, 1)          // 42     — `??` supplies the author's fallback, at runtime
z: u8? = 250; z += 10;     // null   — `!z` is true: the failure is testable
```

| | `??` reaches it | `!` reaches it |
|---|---|---|
| `integer`, `i32` — keep a bottom code | **yes, today** | **yes, today** |
| `u8?`, `i16?`, … — a nullable narrow | yes | **yes, today** |
| **`u8`, `i8`, `u16`, `i16`, `u32` — non-null narrow** | **no** | **no** |

So there is no missing operator. There is a family of five types the existing operators
cannot see into, because every code in their range is a legitimate datum and their failure
has nowhere to live:

```loft
x: u8 = 250;  x += 10;   // x = 0 — and !x is false, x == null is false, x == 0 is true,
y: u8 = 200;  y += 10;   // y = 210      exactly as they are for a real, computed 0
```

`x` is fabricated and `y` is computed and **nothing in the program can tell them apart.**
That is the whole defect.

## What has to change, and what must not

**Change:** those five types gain somewhere for a failure to live, so `??` and `!` have
something to act on. That is the selective boolean — introduced only for a variable whose
status the program actually observes, never for arithmetic at large.

**Must not change:**

- **the surface** — `a: u8 = 250; a += 10; if !a { … }` is existing syntax throughout;
- **ordinary arithmetic** — `integer`, `float` and `single` keep a sentinel, so they never
  need a companion bit and their emission is untouched. This is what bounds the cost, and it
  is why Phase A's +0.5–0.8 % does not transfer: that measured a blanket bit on two
  benchmarks which are plain `integer` throughout ([ARC-B-DESIGN.md](ARC-B-DESIGN.md));
- **the default** — a program that observes nothing still takes the type's default, silently,
  exactly as today.

## Blast radius: measured, and near zero

Narrow widths are barely used for arithmetic anywhere. Counted across the tree:

| where narrow-typed `.loft` files live | files |
|---|---|
| `tests/scripts` | 52 |
| `doc/claude` (prose, not code) | 37 |
| `tests/fixtures` · `tests/docs` · `tests/integration` · `tests/leak_cases` | 17 |
| `tools/` | **2** |
| `lib/` | **0** |

And the owner reports the same shape outside this repo: almost no narrow-width code in active
use, the exception being the hex vectors, which carry many `u8` cases but as **limited
template values without arithmetic**.

That is a strong safety argument and a weak urgency argument, and both should be said:

- **Safety.** Template-value `u8` writes no `??` and no `!`, so it is unmarked, so its
  emission is byte-identical. The population that could be disturbed by this change is
  approximately empty, which is as good as a blast radius gets.
- **Urgency.** The defect is rarely hit, because the construct is rarely written.

**The hypothesis worth naming: the absence may be an effect of the defect.** A type family
that cannot be used safely for arithmetic — you cannot see a failure, you cannot choose the
fallback — gets used only for storage and template values, which is exactly the usage the
owner describes. If that is right, this plan is not repairing a construct people use; it is
making a construct usable. **Falsifier:** land it, and if narrow-width arithmetic still does
not appear, the types were simply niche and the hypothesis was wrong.

It also demotes **S5**: measuring the cost where the bit is live matters much less when
almost nothing is live. Keep it, but as a bound rather than a gate.

## Mechanism

A **per-variable marker**, which is a shape this codebase already uses: `Variable::amp_link`
carries a compile-time fact about one variable rather than changing the representation of
every access, and records that rationale in its own doc comment. `Variable` already holds
`const_binding`, `value_const`, `amp_link`, `uses` — *"is this variable's fit status
observed?"* is another fact of that kind.

1. Pass 1 observes that a `??` or a `!`/`== null` names a narrow non-null variable.
2. That variable is marked.
3. Pass 2 emits the status-carrying form **for marked variables only**; the bit lives in a
   companion slot — a frame slot on the interpreter, a Rust local on native. Both are
   per-variable, which is why both backends can carry it and why Phase A's stack shadow
   (which native had nowhere to put) is not what this is.

The predicate it hangs off already exists and must be extended rather than re-spelled:
`IntegerSpec::reserves_sentinel_unconditionally`, reached through `uncomputable_default`, is
exactly *"can this type represent its own failure?"* — Phase E proved it is the single home by
reading its answer off `dflt` instead of re-deriving it. 173 sites already ask a
narrow-width question, so the hazard here is a duplicate list, not the branch
([ARC-B-DESIGN.md](ARC-B-DESIGN.md)).

## Composition matrix — Stage A

Axes: **width** (the five in scope; `integer`/`i32`/`float`/`single` as controls that must not
move), **spelling** (`??` · `!` · `== null` · none), **seam** (local compound assign · field ·
element · argument · return), **fault shape** (out-of-range vs landing on the sentinel — two
different paths, per Phase C), and **backend**. The before-half exists as
[`probes/`](probes/) and [`probes/axis/`](probes/axis/); a cell with no spelling written must
be byte-identical to today, which is the opt-in claim and the thing the matrix must prove.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **S1** — pin what already works, since nothing does today: `??` on an `integer` overflow, `!` on a nullable narrow | § the two spellings | a `tests/scripts/` guard, falsified against a build with the coalesce stripped | Open |
| **S2** — `??` reaches a non-null narrow: route the author's value into `OpRangeDefault`'s `dflt`, which is already the fallback slot | `parser/expressions.rs::compound_range` | the narrow cells answer the author's value; **every cell with no `??` unchanged**, by `introspect` diff | Open |
| **S3** — `!` reaches a non-null narrow: the per-variable marker, set in pass 1, emitted in pass 2 | § Mechanism | a variable whose test appears textually AFTER its last store still answers, both backends, both passes; unmarked variables byte-identical | Open |
| **S4** — the remaining seams for both spellings: field, element, argument, return | `guard_declared_range` | one cell per seam per spelling; the controls unmoved | Open |
| **S5** — cost where the bit is LIVE (a bound, not a gate — see § Blast radius) | — | a narrow-width benchmark, which `bench/` does not currently contain and S5 must write | Open |
| **S6** — docs: the narrowing error already advertises `?? d`, so the diagnostic and the reference must agree with what ships | `DIAGNOSTICS.md` | the advertised cure works when followed | Open |

## Phase ordering

1. **S1 first** — cheap, and it protects behaviour that ships today pinned by nothing.
2. **S2 before S3.** `??` needs no new storage (the `dflt` slot exists), so it is the half
   that can land on its own and prove the opt-in claim before any marker machinery.
3. **S3 is the one with new machinery**, and the pass split is its hazard: the marker records
   a fact OBSERVED in pass 1, never a prediction about pass 2 — pass-stable data, which is
   what `amp_link` is, and a shape this tree has been bitten by before.
4. **S2 and S3 must both land before this is claimed done.** Choose-only cannot branch, log
   or count; detect-only cannot supply a value inline. Either alone leaves the case
   half-handleable, which is the state being complained about.
5. **S5 is a bound, not a gate.** The "no impact" claim is measured for ordinary arithmetic
   and unmeasured where the bit is live — but almost nothing is live (§ Blast radius), so this
   records a number rather than blocking on one.

## Open questions

1. **Does `!x` on a marked variable read the bit, or does the marked variable's null become
   representable?** They differ observably: the second makes `x == null` true and changes what
   `{x}` prints. The first is narrower and probably right, but it means `!x` and `x == null`
   could disagree on a marked variable, which is its own surprise.
2. **How far does the mark propagate?** `a: u8 = …; b = a; if !b { … }` — is `b` marked, and
   does assigning a marked variable carry the status? A rule that stops at the declaration is
   simple; one that follows the value is what an author would expect.
3. **Constant or expression fallback?** `dflt` is `const integer` today, so a constant is
   nearly free and an expression needs evaluating at the collapse.
4. **`u32`** — its spare code exists but sits at the top where no non-null read tests for it.
   Phase B measured it defaulting with the four; confirm it is not a third case.

## See also

- [MEASUREMENTS.md](MEASUREMENTS.md) — what phases A/B/C/E measured, and the corrections they
  forced on this plan's own premise.
- [ARC-B-DESIGN.md](ARC-B-DESIGN.md) — why the bit is structural, how much is needed, and the
  predicate it must extend.
- [`formal/types.md`](../../formal/types.md) § Null-flow laws ·
  [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C80, C85, C90.
- Shipped here: loft#1305, loft#1306.
