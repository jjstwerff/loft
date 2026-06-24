<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ROADMAP.md — the path to a spec-conformant implementation

The single ordered view of every **open deviation** across `formal/*` — the gaps between
the rules each area states and what the code does today — sequenced into the order to
resolve them. Detail stays in each area doc (this is order/size/direction only, like
[STABILITY_ROADMAP.md](../STABILITY_ROADMAP.md) for stability).

## The principle

> **The code changes to match the rules.** That is the default for every row below.
> But the spec is a *hypothesis*, not scripture: where a rule turns out to be wrong or
> missing a real detail, the **rule** is what changes (flagged **spec-may-adjust**). Most
> rows are code→spec; a few are decisions where the spec itself is on the table.

Closing a row means the implementation obeys the rule (then the deviation entry is deleted),
**or** the rule is corrected and the row becomes a decided edge (moved to
[INCONSISTENCIES.md](../INCONSISTENCIES.md) / [DESIGN_DECISIONS.md](../DESIGN_DECISIONS.md)).

## Distance today

| area | open | what's left |
|---|---|---|
| [types.md](types.md) | 1 | D2 — the integer model is i64 end-to-end (`Value::Int` IR change) |
| [binding.md](binding.md) | 1 | D-bind-7 — reject a bare `&a;` statement |
| [grammar.md](grammar.md) | 3 | precedence-in-the-grammar-doc + 2 decisions (`**` right-assoc landed) |
| [operational.md](operational.md) | 3 | the shared interp/native semantics (oracle) |
| [ownership.md](ownership.md) | 5 | the `deps` borrow checker (the big one) |

Type, binding, and grammar are nearly done; operational and ownership are the long arcs.

---

## Phase A — turnkey (days, no new design)

Cheap, well-scoped, no plan needed — clear the easy distance first.

| # | deviation | change | direction |
|---|---|---|---|
| A1 | **D-bind-7** | extend the prefix-`&` parser guard (already rejects 9 expression positions) to the bare-statement position `&a;` → binding.md reaches **0** | code→spec |
| A2 | **D-gram-1** | lift the 12-level precedence ladder + left-associativity from the parser into [LOFT.md](../LOFT.md)'s user-facing grammar | code→spec (doc) |
| A3 | **D-op-3** | carry "trap-suppressing context" (`?? ` operand) as one threaded fact, not a per-site flag — same shape as the D1 hint consolidation just landed | code→spec |

## Phase B — three decisions (a sentence each; may change the spec, not the code)

These are **spec-may-adjust** — your call resolves them, then they close or reclassify.

| # | deviation | the decision | likely outcome |
|---|---|---|---|
| ~~B1~~ | ~~**D-gram-3**~~ **DONE** | `**` is now **right**-associative (`2**3**2 == 512`) — the maker-centric call (don't carry a surprise). | code→spec, landed; `tests/issues.rs::power_is_right_associative` |
| B2 | **D-gram-2** | is loft's surface deliberately **not context-free** (speculative backtracking + lexer modes)? | **spec-may-adjust**: almost certainly a *decided edge*, not a deviation — reclassify (don't chase a CFG) |
| B3 | **D-gram-4** | the `&` bitwise/reference overload | mostly closed by @PLN87 (prefix `&` now rejected in expression positions); its residual **is** A1 (D-bind-7) — fold/close once A1 lands |

## Phase C — tracked projects (have or need a plan; weeks)

The real weight. Each is a `loft-lang/plans` issue, sequenced.

| # | deviation(s) | project | plan | size |
|---|---|---|---|---|
| C1 | **D2** | integer model i64 end-to-end — widen `Value::Int` to i64 (the IR change; the two false-bottoms are ruled out) | **[@PLN88](https://github.com/loft-lang/plans/issues/88)** | M–L |
| C2 | **D-own-3** | typed `Deps` (replace the overloaded `Vec<u16>`) — the substrate the rest of ownership reads | H2 ([DEPS_INVENTORY.md](../DEPS_INVENTORY.md)); *no issue yet* | M |
| C3 | **D-own-1, D-own-2, D-own-5, D-own-4** | the `deps` borrow checker: ownership computed once per binding/path; free/copy/move derive from one `deps` fact; `&`-borrow source tracked in `deps`; reverse the #415 copy-on-bind stopgap | **[@PLN85](https://github.com/loft-lang/plans/issues/85)** | L (the north star) |

## Phase D — the operational oracle (decided: differential)

| # | deviation(s) | project | direction |
|---|---|---|---|
| D1 | **D-op-1, D-op-2** | **DECIDED (2026-06): a differential oracle** — run a growing corpus on BOTH backends and assert they agree (value / trap / stdout / leak); the operational.md rules guide coverage. Turns the interp/native divergence class (D4/#433) from a coverage lottery into a caught failure. Switchable later to an executable shared semantics; the rules reuse either way. *Needs a plan issue (none yet).* | code→spec (the chosen model) |

---

## Resolving order, in one line

**A1 → A2 → A3** (clear types/binding/grammar to near-zero) **·** **B1/B2/B3** (decide, cheap) **·**
then the tracked arcs **C1 (@PLN88) · C2 (typed Deps) → C3 (@PLN85)** in that dependency order **·**
**D1** (operational oracle) last, after its goal is decided.

## What is NOT on this list (already clean or decided)

- types.md D1/D3/D4/D5, binding.md D-bind-0..6 + doc — **closed in code this cycle** (with tests).
- A row that turns out **spec-may-adjust** leaves `formal/` and becomes a decided edge — it is
  *resolved*, not deleted-by-fix. The deviation count is "distance from the current spec," and
  the current spec is allowed to be wrong.
