<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# DRAFT — the provenance paragraph (T3.1)

> **Status: draft, not published.** Deliberately NOT placed in `README.md`: this
> is a biographical claim about the author, and putting unapproved wording about a
> real person into the public README is not an agent's call. On approval it goes
> into the README's *How loft is built* section (which B4 already moved up).

## Why this paragraph exists

A first-time visitor's prior is "abandoned weekend hobby language". Every unusual
design decision — record-oriented collections, no `let`, four execution modes —
reads as a *quirk* without provenance and as a *deliberate trade-off* with it.
That context is currently stated nowhere.

It also strengthens the bus-factor argument rather than competing with it: the
agents execute a design that took a decade to converge; they did not invent it.

## Constraints (from the review, and they are the point)

- **One statement, plain.** No superlatives, no credential adjectives
  ("battle-tested", "decades of experience"). Restraint is what signals seniority.
- **No employer names, no domains.** The previous languages stay anonymous by
  design — "niche DSL for a lease company" is not a flex and the author does not
  want it surfaced.
- The **niche→general arc is the story**: someone who spent decades inside the
  limits of special-purpose languages building the general-purpose one he always
  wanted.

## Draft A — the review's wording, lightly tightened

> Loft is the fourth language its author has built. The previous three were
> production domain-specific languages in industry — the kind that quietly run a
> business for decades and never get a name. Loft is the first general-purpose
> one, and it carries ten years of design iteration. The AI agents write most of
> the code; the taste and the trade-offs are forty years of programming distilled
> into the docs they work from.

## Draft B — shorter, arc first

> Loft is the fourth language its author has built, and the first general-purpose
> one. The previous three were production domain-specific languages — the kind
> that quietly run a business for decades and never get a name — and the limits he
> kept hitting inside them are what this one is trying to answer. Ten years of
> design went in before the agents started writing code; what they work from is
> the design, not the other way round.

## Notes for the decision

- **B is more specific about the "why", A is more specific about the "how".** B's
  last sentence does the bus-factor work directly ("what they work from is the
  design"); A's does it more softly.
- "Forty years of programming" is the only number in A that is a credential
  rather than a fact about the work. If the restraint rule is applied strictly it
  probably goes; if it stays, it is the one place a reader can calibrate.
- Neither draft names a domain or an employer. Please check both against your own
  bar for that — it is easier for you to spot an implicit tell than for me.
