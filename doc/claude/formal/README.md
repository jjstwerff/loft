<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ — loft's strict formal definition (rules + tracked deviations)

This directory is the **strict** formal definition of loft. Each document covers one
area and has exactly two parts:

1. **Rules** — the formal definition we *want*: the judgments / relations / grammar
   loft is meant to satisfy, written tightly enough to be checked against. This is the
   **target**, not a description of today's code.
2. **Deviations** — every place the *current implementation* breaks a rule above,
   numbered (`D1`, `D2`, …), each with: the rule it violates, where it lives, the
   user-visible effect, and a status. **The deviation list is meant to shrink to
   zero over time** — closing a deviation means making the implementation obey the
   rule (often a bug fix or a refactor), then deleting its entry here.

> **The rules do not change to match the code. The code changes to match the rules.**
> A new edge that the rules can't express is a signal the *rule* is wrong (fix the
> rule); a place the code disobeys a sound rule is a *deviation* (fix the code).

## Reading guide

These docs are dense (they are a spec), but every rule is meant to be readable. How to
get through them:

- **Read the prose first.** Each substantial block of formal rules is paired with an
  **"In words"** reading in plain English. The formal rule is the precise version; the
  prose is the one to read first. If the two ever disagree, the prose is the mistake
  (fix it).
- **The notation, explained once:**
  - `Γ ⊢ e ⇒ τ` — "expression `e` *has* type `τ`" (the parser works the type out itself).
  - `Γ ⊢ e ⇐ τ` — "`e` is *checked against* an expected type `τ`" (the `τ` is pushed in
    from the surrounding code).
  - `τ ⤳ σ` — "a value of type `τ` is *accepted where* `σ` is expected", with no cast.
  - `⊔` — the *join*: the smallest type that contains both (for integers, the wider range).
  - `(Name)` in front of a rule is just its label, so a deviation can cite it.
- **The examples are the anchor.** Each area ends with *falsifying programs* — tiny
  snippets where obeying the rule and obeying today's code disagree. Read those to see
  what a rule actually buys.
- **A deviation (`Dn`) is a known gap, not a bug report** — "the code breaks this rule,
  here" — tracked to be removed, then deleted.

## Relationship to the working docs

These formal docs are **new and separate**; the existing planning/analysis docs are
unchanged and stay where they are:

| doc | role |
|---|---|
| [FORMALIZATION.md](../FORMALIZATION.md) | the **lens** — why formalizing is worth it, per-layer readiness, ranked rough spots |
| [TYPING_RELATION.md](../TYPING_RELATION.md) | the **analysis** of the type/conversion area — rough spots R1–R3, recommendations |
| [STABILITY_REDFLAGS.md](../STABILITY_REDFLAGS.md), [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md), … | the runtime/ownership working records |
| **`formal/*` (here)** | the **strict spec** — the rules, and the deviation list to drive to zero |

The lens docs answer *"is this worth doing and where does it hurt?"*; these answer
*"what exactly is the rule, and which lines break it?"* They cite each other but do
not duplicate: a deviation entry links to the lens analysis instead of re-explaining it.

## Areas

The **static** areas are all at **0 open deviations** (2026-07-04). The **operational** area is
one small-step contract split across files: the scalar core (operational.md) plus the heap,
iteration, coroutines, and concurrency (added 2026-07-04) — these hold **0 open deviations of
their own** and shrink operational.md's single meta-deviation (D-op-1: conformance is *differential*
via the @PLN89 oracle, not a second executable definition).

| doc | area | status |
|---|---|---|
| [types.md](types.md) | type system + conversion relation (incl. integer width) | **0 open** — @PLN25 value/null model landed; DN1–DN6 + D2 closed |
| [binding.md](binding.md) | reference types & `&` (the bind-site link law) | **0 open** — `&` is a TYPE ANNOTATION (`&τ` = `Type::RefVar`); the @PLN87 ladder L1–L6 landed (PR#436); D-bind-7 (bare `&a;` reject) closed |
| [grammar.md](grammar.md) | concrete grammar + operator precedence | **0 open** — the 12-level precedence ladder written; the prefix-`&`/infix-`&` overload + non-CFG surface resolved as decided edges (C81/C82) |
| [operational.md](operational.md) | small-step semantics — the scalar core | **rules complete for the core, 2 open** — values/null sentinels, left-to-right order, uncomputable→null (C80) + `??`, state steps; the 2 open are the META deviation D-op-1/2 (differential-not-definitional conformance), inherited by every operational file below |
| [heap.md](heap.md) | store steps — alloc / read / write / **copy** / free | **rules written (2026-07-04), 0 own** — the `DbRef`/`Store` model; the whole-value COPY (C86); the LIFO free discipline whose soundness is ownership.md; conformance via the oracle (D-op-1) |
| [iteration.md](iteration.md) | `for`, ranges, text iteration, the map/filter/reduce/comprehension combinators | **rules written (2026-07-04), 0 own** — index-cursor `for`, deterministic combinator order, fresh result vector; conformance via the oracle |
| [coroutines.md](coroutines.md) | generators — `yield` / `next`, stackful suspension | **rules written (2026-07-04), 0 own** — lazy one-value-per-advance; straight-line yields lazy on both backends, but LOOP-based yields are eager on native (a DECIDED EDGE — rustc restriction, aspiration to fix); conformance via the oracle |
| [concurrency.md](concurrency.md) | `par` — the one parallel construct | **rules written (2026-07-04), 0 own** — a parallel map consumed in source order; determinism CONDITIONAL on a pure worker; conformance via the oracle |
| [calls.md](calls.md) | function call & return — args, parameter binding, the frame | **rules written (2026-07-04), 0 own** — args left-to-right; scalar params by-value, heap params share (mutate-through visible, whole reassign local, `&` writes back); returns independent |
| [matching.md](matching.md) | `match` — enum-variant dispatch + payload binding | **rules written (2026-07-04), 0 own** — an expression; struct-payload patterns bind by name; `_` is the final catch-all; **compile-time exhaustiveness** (a missing variant does not compile) |
| [tuples.md](tuples.md) | tuples — construct / project / destructure | **rules written (2026-07-04), 0 own** — positional products (n≥2); `.i` a compile-time index; `(a,b) = …` destructuring; tuple returns |
| [closures.md](closures.md) | lambdas / closures / fn-refs — capture + apply | **rules written (2026-07-04), 1 OPEN** — the `fn(){}` and `\|…\|` forms now capture IDENTICALLY (pure sugar — D-clo-1 CLOSED); first-class (store/pass/return/escape); scalar-by-value / heap-shared capture. One open: D-clo-2 (a stored short lambda passed to `map` PANICS at data.rs:4569) |
| [ownership.md](ownership.md) | the `deps` / borrow **checker** (lifetimes) — distinct from binding.md's surface | **0 open** (2026-07-04) — D-own-1/2/3/4/5 ALL CLOSED; every store-lifetime decision reads the one total `deps` fact. The soundness proof heap.md's free rules rest on |
| [capabilities.md](capabilities.md) | sandbox **admission** — what a restricted caller may do (call / parameter / field / mutation rights) | **0 open** (2026-07-04) — the 6-rule judgment `P;ctx ⊢ e ✓` fully enforced; D-cap-1/2/3 CLOSED, each with a RED/GREEN adversarial pair. Cites ownership.md/heap.md for the owned-vs-host fact |

## Roadmap

[ROADMAP.md](ROADMAP.md) is the single ordered view of every open deviation across the
areas — sequenced into the order to resolve them, each flagged **code→spec** (the default)
or **spec-may-adjust** (where the rule itself is the decision). Start there to see the path
to a spec-conformant implementation; the per-area docs hold the detail.

[VERIFICATION.md](VERIFICATION.md) is the companion worklist for the operational rules
written 2026-07-04 (heap / iteration / coroutines / concurrency / calls / matching / tuples /
closures): per-rule, the single falsifiable claim, its both-backends status, and the standing
guard that pins it. Where ROADMAP tracks *deviations to close*, VERIFICATION tracks *rules to
pin* — the concrete plan to drive the differential oracle down from every area to every rule.

## Deviation entry format

```
### Dn — <one-line name>
- **Violates:** <rule id(s)>
- **Where:** <file:symbol> (the site(s) that break it)
- **Effect:** <user-visible symptom / issue refs>
- **Status:** OPEN | IN PROGRESS (<branch/PR>) | CLOSED (<commit> — then delete)
- **Removal:** <the change that makes the code obey the rule>
```

A CLOSED deviation is **deleted**, not kept — `git log` is the history. The count of
OPEN deviations per doc is the area's distance from formal.
