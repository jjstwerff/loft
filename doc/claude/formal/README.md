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

| doc | area | status |
|---|---|---|
| [types.md](types.md) | type system + conversion relation (incl. integer width) | **started** |
| [binding.md](binding.md) | reference types & `&` (the bind-site link law) | **mostly landed** — `&` is a TYPE ANNOTATION (`&τ` = `Type::RefVar`), not an operator; the @PLN87 ladder L1–L6 (scalar/field/element/param read+write-through) landed via PR#436 and is verified; **1 deviation** left (D-bind-7: a bare `&a;` statement not yet parse-rejected) |
| [grammar.md](grammar.md) | concrete grammar + operator precedence | **started** — the 12-level precedence ladder (left-associative except `**`, now right-assoc) the written grammar omits; the prefix-`&` (reference) vs infix-`&` (bitwise-and) overload; 3 deviations |
| [operational.md](operational.md) | small-step semantics for the stable core | **started** — the scalar core as the shared interp/native contract (values/null sentinels, left-to-right order, the **uncomputable→null** discipline [the spreadsheet model, C80] + `??` fallback, state steps); 3 deviations (no shared semantics → divergence is structural, #433; the trap→null runtime change is D-op-4) |
| [ownership.md](ownership.md) | the `deps` / borrow **checker** (lifetimes) — distinct from binding.md's surface | **started (aspirational)** — the 5 sound+complete invariants we steer toward (the rules), with the active @PLN85/@PLN87 migration as the 5 deviations. Written deliberately ahead of the implementation: the target steers the work |
| [capabilities.md](capabilities.md) | sandbox **admission** — what a restricted caller may do (call / parameter / field rights) | **started (aspirational + conditional)** — a 6-rule authorization judgment `P;ctx ⊢ e ✓`, gated on a sandbox profile (vacuous for trusted code); the call gate is enforced, field rights + the parameter lock are the 3 deviations (@PLN86 F4–F7). Cites ownership.md for the owned-vs-host predicate |

## Roadmap

[ROADMAP.md](ROADMAP.md) is the single ordered view of every open deviation across the
areas — sequenced into the order to resolve them, each flagged **code→spec** (the default)
or **spec-may-adjust** (where the rule itself is the decision). Start there to see the path
to a spec-conformant implementation; the per-area docs hold the detail.

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
