<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 152 — Let the existing spellings reach the types they cannot reach today

## Status

**Steps 0–3 are DONE and `??` now reaches the narrow widths** — `x = (x + 10) ?? 255` into a
`u8` answers `255` on both backends, where it was refused before. Steps 3–7 open. Shipped along the way:
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

Frequency does not enter it. A case a programmer cannot write correctly is worth fixing
whether or not many programmers reach it — the limitation is arbitrary, and removing
arbitrary limitations is the job (§ Why this is worth doing).

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

## Why this is worth doing, and separately, what it risks

**The justification is not frequency.** A programmer who reaches for a `u8` and writes
arithmetic on it cannot write correct code today: the failure is unobservable and the
substituted value is not theirs. That is an **arbitrary limitation** — arbitrary because
nothing about the language requires it, only the absence of a mechanism — and loft exists to
carry that kind of load for the programmer, not to hand it back
([GOALS.md](../../GOALS.md)).

The doctrine is already on the record, in the maker's own words under
[C79](../../DESIGN_DECISIONS.md), and the entry notes that it generalises:

> *"If we cannot fulfil what a programmer asks for us that should be an error instead of
> silently different semantics … otherwise we will have to support this seemingly
> undefined/strange behaviour into our future."*

`x: u8 = 250; x += 10` asked for one thing and silently got another. C79's own answer was to
refuse rather than substitute; here refusing is not available — C80 keeps the program running
and this plan adds no error — so the same principle takes its other form: **make the
substitution visible and choosable.** Rare or not, that is the case being fixed.

### What the rarity DOES tell us: the risk is near zero

Narrow widths are barely used for arithmetic. Counted across the tree:

| where narrow-typed `.loft` files live | files |
|---|---|
| `tests/scripts` | 52 |
| `doc/claude` (prose, not code) | 37 |
| `tests/fixtures` · `tests/docs` · `tests/integration` · `tests/leak_cases` | 17 |
| `tools/` | **2** |
| `lib/` | **0** |

The owner reports the same outside this repo: almost no narrow-width code in active use, the
exception being hex vectors carrying many `u8` cases as **limited template values without
arithmetic**. Those write no `??` and no `!`, so they stay unmarked and their emission is
byte-identical. **The population that could be disturbed is approximately empty** — which is
what a measured blast radius is for, and it is the only thing this count decides.

It also makes **S5** a bound rather than a gate: measuring the cost where the bit is live
records a number, but little is live for it to slow down.

**And the low usage may itself be the defect's shadow.** A type family that cannot be used
safely for arithmetic gets used only for storage and template values — exactly the usage
described. If so, this plan does not repair a construct people use; it makes one usable.
**Falsifier:** land it, and if narrow-width arithmetic still does not appear, the types were
niche and this reading was wrong.

## Hard constraint: the bit lives BESIDE THE EXPRESSION, and is never stored

**Nothing is stored anywhere.** Not in a struct, not in a collection element, not in a
variable's slot. The bit exists while an expression is being evaluated, is consumed by the
thing that validates it, and is gone. Storage layout is untouched by construction — no
store-format change, nothing for @PLN97's layout hash to notice, and no way for an
implementation to drift into widening anything.

That matters because narrow types exist for **density**, measured on 200 000 elements:

| | store capacity | per element |
|---|---|---|
| `vector<u8>` | **0.362 MB** | 1 byte |
| `vector<integer>` | **5.909 MB** | 8 bytes |

**16×** — 20 MB against 320 MB at the scale these types are chosen for. A bit stored beside
every element would give back half of that, so the constraint is not a preference; it is the
reason the types exist.

### What this permits, and what it forbids

The rule decides a question this plan had left open, and it decides it against one of the
sketches that motivated the plan:

```loft
x = (x + 10) ?? 255;                  // ✅ the bit rides the expression, `??` consumes it
if !(x + 10) { … }                    // ✅ `!` reads the bit of the expression beside it

f.i += 100;                           // ✅ ADJACENT FORM — the `if` directly follows,
if !f.i { … }                         //    so the bit lives in a temp across the pair

f.i += 100;                           // ❌ a statement intervenes: nothing carries the
log("…");  if !f.i { … }              //    status that far without storing it
```

**The adjacent form is admitted**, and it is still storage-free: when an `if` testing the
assigned place is the *very next* statement, the compiler can see both together and keep the
bit in a temp across exactly that window. Nothing is written to a struct, an element, or a
variable slot — the window is statically known and bounded at two statements.

**Anything further apart is not**, because past the next statement there is nothing left to
carry the status but the place itself, and the place is storage.

The bit is therefore an evaluation-time value with a bounded lifetime, not a stored one — and
everything hard about a per-variable design stays gone: no marker on `Variable`, no
pass-1/pass-2 split (the hazard this tree has been bitten by), no companion slot, no
propagation rule for `b = a`, and no way to damage layout even by mistake.

### The adjacency rule polices itself, with a lint that already ships

The obvious hazard is that adjacency is invisible: insert a line between the assignment and
the `if`, and the check silently stops working. **It does not go silent — an existing warning
catches it.** Measured today, on both the adjacent and the non-adjacent shape:

```
warning[redundant-null-negation]: '!' on a 'not null' integer(0, 255) is always false
```

So the design's job is to make that warning **stop firing in the fused position and keep
firing everywhere else**. Then moving a line restores it, and an author who writes the check
too far from the assignment is told that it is always false rather than left believing it
works. The guard rail exists; it needs teaching, not building — and it gives S3 an exact
gate rather than a judgement call.

## Mechanism

A fit-failing operation evaluates to its value **and** a validity bit, for the five widths
that cannot represent their own failure. The bit is available only to a validator in the same
expression:

- **`??`** consumes it — the author's fallback replaces the type's default, routed into
  `OpRangeDefault`'s `dflt`, which is already the fallback slot (Phase E).
- **`!`** reads it — *did this fail to produce a value?* — either of the expression it is
  applied to, or of the assignment in the immediately preceding statement when it names that
  same place.

Where neither appears, the bit is never produced and emission is byte-identical: that is the
opt-in claim, and it is checkable with an `introspect` diff rather than argued.

Ordinary arithmetic is untouched. `integer`, `float` and `single` keep a sentinel, so their
failure is already a value and they never produce a bit — which is what bounds the cost, and
why Phase A's +0.5–0.8 % (measured on two benchmarks that are plain `integer` throughout)
does not transfer ([ARC-B-DESIGN.md](ARC-B-DESIGN.md)).

The predicate this hangs off already exists and must be extended rather than re-spelled:
`IntegerSpec::reserves_sentinel_unconditionally`, reached through `uncomputable_default`, is
exactly *"can this type represent its own failure?"* — Phase E proved it is the single home by
reading its answer off `dflt`. 173 sites already ask a narrow-width question, so the hazard
is a duplicate list, not the branch ([ARC-B-DESIGN.md](ARC-B-DESIGN.md)).

## Composition matrix — Stage A

Axes: **width** (the five in scope; `integer`/`i32`/`float`/`single` as controls that must not
move), **spelling** (`??` · `!` · `== null` · none), **seam** (local compound assign · field ·
element · argument · return), **fault shape** (out-of-range vs landing on the sentinel — two
different paths, per Phase C), and **backend**. The before-half exists as
[`probes/`](probes/) and [`probes/axis/`](probes/axis/); a cell with no spelling written must
be byte-identical to today, which is the opt-in claim and the thing the matrix must prove.
One axis is not about values at all: **stored SIZE**, asserted with `store_memory()` on a
large narrow collection, because a design that passed every value cell while doubling
`vector<u8>` would have broken the reason the type was declared.

## Sub-arcs — small steps, each able to go red on its own

Cut against the two bounds ([loft-plan-workflow](../../../../.claude/skills/loft-plan-workflow/SKILL.md)
§ Cutting a phase): **upper** — the old path and the new one both run and compare exactly;
**lower** — the step can fail for a real reason without the next one. Every step's opt-in
claim has the same shape of check, an `introspect` diff over a corpus that writes neither
`??` nor `!`, so a step that quietly changed ordinary emission cannot pass.

| # | Step | Verify — what goes red | Status |
|---|---|---|---|
| **0** | Probe: where must the opt-in gate live? | **DONE by inspection, no build needed** — `guard_declared_range` / `guard_compound_range` already receive the whole stored expression, so the gate lives at the store guard and needs no parse-time flag. | **Done** |
| **1** | Pin the three spellings that ship today and are guarded by nothing | `tests/scripts/152-the-fallback-and-failure-spellings-that-already-work.loft`; INERT against `origin/main` by design and recorded as such — it is a LOCK, and what moves it is a change made BY this plan. | **Done** |
| **2** | `??` reaches a non-null narrow — guard INSIDE the discharge, sentinel as its default | `tests/scripts/152-a-coalesce-chooses-the-fallback-for-a-narrow-slot.loft`, falsified at `4f229521` (the shape was refused before, so the guard cannot compile on the control) — both backends; the three probe channels unchanged. | **Done** |
| **3** | `??` at the remaining positions: field, element, argument, return, struct literal | `tests/scripts/152-the-fallback-reaches-every-position-a-narrow-store-takes.loft`, falsified at `4f229521`, both backends. **Came free with step 2** — wiring both seams covered all five; each cell uses a distinct fallback so none can pass on a neighbour's answer. | **Done** |
| **4** | ~~`!` in-expression~~ | **RETIRED — not implementable.** `x + 10` widens to `integer` and `260` is a good one, so a free expression has no narrow type, no range, and no failure to read. The failure is a property of a STORE ([MEASUREMENTS.md](MEASUREMENTS.md)). Folded into step 5. | **Retired** |
| **5** | **`!` in the adjacent form**: `f.i += 100;` then `if !f.i { … }` — now the ONLY spelling in which the question can be asked, since the assignment is what supplies the target | The lint is silent when fused and **fires one statement later**; `probes/diag` already scores that channel. Red if fusion reaches across an intervening statement, or misses an adjacent one. | Open |
| **6** | **Cost where the bit is live** (a bound, not a gate — § Why this is worth doing). | A narrow-width benchmark, which `bench/` does not contain and this step writes. Records a number; does not block. | Open |
| **7** | **Docs and diagnostics agree with what shipped** — the narrowing error already advertises `?? d`. | The advertised cure works when followed. Red if the message still prescribes something that does not work in the position it is offered. | Open |

## Ordering, and why each boundary is where it is

1. **Step 0 before anything.** It is throwaway and it can invalidate the whole shape for the
   price of one build — the cheapest phase in the plan and the one most likely to save work.
2. **Step 1 before any change**, so the three behaviours that ship today are pinned *before*
   the code that could break them exists. They are unguarded right now.
3. **Steps 2 and 3 split by seam, not by feature.** One seam proves the mechanism against an
   exact comparison; the rest are the same mechanism at more places, each with its own cell.
   Doing them together would mean a red cell could not be attributed to a seam.
4. **Step 4 is retired into step 5.** A free expression cannot fail, so the adjacent form is
   not one spelling of two — it is the only one in which the question can be asked without new
   syntax. Step 5 therefore also owns the lint behaviour step 4 was to have established.
5. **Steps 2–5 must all land before this is claimed done.** Choose-only cannot branch, log or
   count; detect-only cannot supply a value inline. Either alone leaves the case
   half-handleable, which is the state being complained about.

## Open questions

1. **Constant or expression fallback?** `dflt` is `const integer` today, so a constant is
   nearly free and an expression needs evaluating at the collapse.
2. **`u32`** — its spare code exists but sits at the top where no non-null read tests for it.
   Phase B measured it defaulting with the four; confirm it is not a third case.
3. **Does `!` on an expression conflict with its existing meaning?** `!` tests presence today,
   and on a non-null narrow expression it is currently always false — `redundant-null-negation`
   says so and would have to learn this case. Check that the lint and the new reading agree
   rather than one silently outranking the other.
4. **Double evaluation.** `if !(x + 10) { … } else { x = (x + 10) ?? 0; }` writes the
   expression twice, which C92 made *"evaluate the place exactly once"* for compound
   assignment precisely because double evaluation is a silent-wrong when the expression has
   effects. If that shape is the one authors will write, it wants a form that does not repeat
   the expression — which is a surface question for S2/S3, not an implementation detail.

**Resolved by the expression-local constraint** (recorded so they are not re-opened): whether
a marked variable's null becomes representable — there are no marked variables; and how far a
mark propagates through `b = a` — nothing propagates, because nothing is stored.

## See also

- [MEASUREMENTS.md](MEASUREMENTS.md) — what phases A/B/C/E measured, and the corrections they
  forced on this plan's own premise.
- [ARC-B-DESIGN.md](ARC-B-DESIGN.md) — why the bit is structural, how much is needed, and the
  predicate it must extend.
- [`formal/types.md`](../../formal/types.md) § Null-flow laws ·
  [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) C80, C85, C90.
- Shipped here: loft#1305, loft#1306.
