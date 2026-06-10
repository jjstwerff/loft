<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design Protocol 1 — A Design Is a Testable Hypothesis

> **Moved to a skill.** The protocol now lives as the **`design-protocol` skill**
> (`.claude/skills/design-protocol/SKILL.md`) — a self-contained, tree-agnostic
> sibling of the `engineering-rigor` skill, loaded on demand rather than carried in
> every context. It is the DESIGN-mode counterpart engineering-rigor routes to.
> Run `/design-protocol`, or it loads automatically when you are about to commit to
> a load-bearing design. This page is kept as a stable anchor for the doc-graph.

The first protocol **graduated** from the [Design Verification List](DESIGN_VERIFICATION.md)
(concern **C1 — brittleness over bugs**). In one line: *a design is a testable
**hypothesis** about an invariant, not a plan you execute* — name the one invariant,
count its re-assertion sites, build the cheapest probe that could **falsify** each
load-bearing claim, then build and validate against the written prediction; and for
exact-invariant domains where you cannot even form the invariant, flip to the
constructive instrument (plot a concrete instance of the *answer* and read the
invariant off it). The full method, evidence, and worked examples are in the skill.

## See also

- **`engineering-rigor` skill** — the synthesis + router; this protocol is its DESIGN-mode depth.
- [DESIGN_VERIFICATION.md § C1](DESIGN_VERIFICATION.md) — the incubator this graduated from.
- [GOALS.md](GOALS.md) Goal E — robustness by subtraction, the deep reason the short version is usually the robust one.
