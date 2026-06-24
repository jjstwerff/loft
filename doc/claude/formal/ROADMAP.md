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
| [binding.md](binding.md) | 0 | ✓ closed — D-bind-7 (reject bare `&a;` / block-final `{ &a }`) landed |
| [grammar.md](grammar.md) | 0 | ✓ closed — D-gram-1/3 landed; D-gram-2 (non-CFG) + D-gram-4 (`&` overload) resolved as decided edges → DESIGN_DECISIONS C81/C82 |
| [operational.md](operational.md) | 3 | D-op-1/2 the differential oracle (@PLN89) · D-op-4 the spreadsheet runtime (C80) |
| [ownership.md](ownership.md) | 5 | the `deps` borrow checker (the big one) |

Binding is closed; type and grammar are nearly done; operational and ownership are the long arcs.

---

## Phase A — turnkey (days, no new design)

Cheap, well-scoped, no plan needed — clear the easy distance first.

| # | deviation | change | direction |
|---|---|---|---|
| ~~A1~~ **DONE** | ~~**D-bind-7**~~ | extended the prefix-`&` guard to the bare-statement position `&a;` (and block-final `{ &a }`, the same leak) at the `parse_assign` chokepoint → binding.md is **0**. `pln87_d_bind_7_*` in `tests/parse_errors.rs`. | code→spec, landed |
| ~~A2~~ **DONE** | ~~**D-gram-1**~~ | lifted the 12-level precedence ladder + associativity into [LOFT.md § Operators](../LOFT.md#operators) (fixed the stale table: `**`@10, `as`@11, assoc statement, unary-tightest note) and enumerated `binary_op` in § Summary of grammar | code→spec (doc), landed |
| ~~A3~~ **RESOLVED (subsumed)** | ~~**D-op-3**~~ | **obsolete — do NOT thread the flag.** The C80 decision (this cycle) eliminated trap-suppression as a concept: every uncomputable op yields null+continue in ALL modes, so the trapping variant *and* the `??`-position rewrite (`rewrite_outer_arith_to_nullable`, the per-site flag) both disappear. operational.md folded D-op-3 into **D-op-4** (`E-Coalesce`: "closes the old D-op-3"). The remaining work is D-op-4 (below), not a Phase-A consolidation. | spec-adjusted → D-op-4 |

## Phase B — three decisions (a sentence each; may change the spec, not the code)

These are **spec-may-adjust** — your call resolves them, then they close or reclassify.

| # | deviation | the decision | likely outcome |
|---|---|---|---|
| ~~B1~~ | ~~**D-gram-3**~~ **DONE** | `**` is now **right**-associative (`2**3**2 == 512`) — the maker-centric call (don't carry a surprise). | code→spec, landed; `tests/issues.rs::power_is_right_associative` |
| ~~B2~~ | ~~**D-gram-2**~~ **DONE** | loft's surface IS deliberately not context-free — accepted on purpose. | reclassified → decided edge, [DESIGN_DECISIONS C82](../DESIGN_DECISIONS.md#c82--lofts-surface-is-deliberately-not-context-free) |
| ~~B3~~ | ~~**D-gram-4**~~ **DONE** | A1 made prefix `&` total — keep one `&` token, disambiguated by position (like Rust). | reclassified → decided edge, [DESIGN_DECISIONS C81](../DESIGN_DECISIONS.md#c81---stays-one-token-disambiguated-by-position-bitwise-and-vs-reference) |

## Phase C — tracked projects (have or need a plan; weeks)

The real weight. Each is a `loft-lang/plans` issue, sequenced.

| # | deviation(s) | project | plan | size |
|---|---|---|---|---|
| C1 | **D2** | integer model i64 end-to-end — widen `Value::Int` to i64 (the IR change; the two false-bottoms are ruled out) | **[@PLN88](https://github.com/loft-lang/plans/issues/88)** | M–L |
| C2 | **D-own-3** | typed `Deps` (replace the overloaded `Vec<u16>`) — the substrate the rest of ownership reads | H2 ([DEPS_INVENTORY.md](../DEPS_INVENTORY.md)); *no issue yet* | M |
| C3 | **D-own-1, D-own-2, D-own-5, D-own-4** | the `deps` borrow checker: ownership computed once per binding/path; free/copy/move derive from one `deps` fact; `&`-borrow source tracked in `deps`; reverse the #415 copy-on-bind stopgap | **[@PLN85](https://github.com/loft-lang/plans/issues/85)** | L (the north star) |

## Phase D — the operational arc (two projects: oracle + spreadsheet runtime)

| # | deviation(s) | project | direction |
|---|---|---|---|
| D1 | **D-op-1, D-op-2** | **DECIDED (2026-06): a differential oracle** — run a growing corpus on BOTH backends and assert they agree (value / null / halt / stdout / leak); the operational.md rules guide coverage. Turns the interp/native divergence class (D4/#433) from a coverage lottery into a caught failure. Switchable later to an executable shared semantics; the rules reuse either way. Tracked: **[@PLN89](https://github.com/loft-lang/plans/issues/89)**. | code→spec (the chosen model) |
| D2 | **D-op-4** (subsumes ~~D-op-3~~) | **BUILD the spreadsheet runtime** — the C80 implementation gap (the rule moved; the code has not). Today an overflow / div-by-zero / OOB-index **traps**, and a dev run **halts**; the trap-vs-null choice rides on the static `??`-position rewrite. `E-Uncomp` wants null+continue in **all** modes, **silently** (opt-in debug log), with the `??`-rewrite dropped — `panic`/`assert` keep their C66 dev-halt/prod-log split (out of scope). Size **M+** (touches arith op dispatch, codegen, dev-halt behaviour); needs a plan issue (none yet). | code→spec (C80) |

---

## Resolving order, in one line

**~~A1~~ ~~A2~~ ~~A3~~** clears Phase A **·** **~~B1~~ ~~B2~~ ~~B3~~** all decided (D-gram-2/4 →
decided edges C82/C81; grammar.md at 0) **·** binding.md + grammar.md now **closed**, types.md at 1
**·** NEXT: the tracked arcs **C1 (@PLN88) · C2 (typed Deps) → C3 (@PLN85)** in that dependency
order **·** the operational arc **D1** (oracle, @PLN89) + **D2** (D-op-4 spreadsheet runtime, C80) last.

## What is NOT on this list (already clean or decided)

- types.md D1/D3/D4/D5, binding.md D-bind-0..6 + doc — **closed in code this cycle** (with tests).
- A row that turns out **spec-may-adjust** leaves `formal/` and becomes a decided edge — it is
  *resolved*, not deleted-by-fix. The deviation count is "distance from the current spec," and
  the current spec is allowed to be wrong.
