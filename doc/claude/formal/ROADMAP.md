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
| [types.md](types.md) | 0 | ✓ closed — **@PLN25 value/null model landed (2026-07-02): DN1/DN2/DN3/DN4/DN5/DN6 CLOSED**, D2 closed (C83). DN3 fully closed: text→numeric **parse** now types `τ?` (`(N-Parse)`, reachable fault, like `÷0`/OOB). Overflow-arith (`a*b`) is a **decided edge** — non-null, overflow→null+continue (C85), NOT `τ?` |
| [binding.md](binding.md) | 0 | ✓ closed — D-bind-7 (reject bare `&a;` / block-final `{ &a }`) landed |
| [grammar.md](grammar.md) | 0 | ✓ closed — D-gram-1/3 landed; D-gram-2 (non-CFG) + D-gram-4 (`&` overload) resolved as decided edges → DESIGN_DECISIONS C81/C82 |
| [operational.md](operational.md) | 2 | D-op-1/2 the differential oracle (@PLN89) — D-op-4 the spreadsheet runtime (C80) is **CLOSED** (formalize4); the oracle SEED landed (`tests/oracle/`) |
| [ownership.md](ownership.md) | 5 | the `deps` borrow checker — **★ ACTIVE NOW (next days): the code-simplification exploration**, see [OWNERSHIP_MODEL.md § ACTIVE](../OWNERSHIP_MODEL.md#active--the-simplification-exploration-next-days-exploratory--revertable). Typed `Deps` (D-own-3) first |
| [capabilities.md](capabilities.md) | 3 | sandbox admission — call gate + field read/update/append **enforced** (@PLN86 F1–F6); remaining: the parameter `#default` lock (D-cap-1, @PLN86 6.9), the capturing-closure residual (D-cap-2), the owned-vs-host dependency on ownership D-own-2 (D-cap-3) |

Binding + grammar + **types are closed** — the @PLN25 value/null model landed (2026-07-02); DN3
fully closed with the text→numeric parse flip (`(N-Parse)` types `τ?`), overflow-arith reclassified
as a decided edge (C85, not a deviation). The active focus
for the next days is the **ownership simplification** — collapse the per-site `deps` thicket onto
the one beacon fact (OWNERSHIP_MODEL.md), exploratory + revertable, guarded by the @PLN89
differential oracle. The operational differential oracle (D-op-1/2) grows alongside it as the
safety net. **@PLN25 unblocks ownership** (GATE 1): the value/null model is settled, so the
ownership invariant can now be defined at `materialization_mode` (the two met there).

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
| C1 | **D2** | integer model i64 end-to-end. **AUDITED** ([plans/88-integer-i64.md](../plans/88-integer-i64.md)) — reframed: do NOT widen `Value::Int` (the runtime is already i64; `Int(i32)`/`Long(i64)` is a compact value-size encoding). The change is `IntegerSpec` bounds → i64 + template unify + an `int_const(i64)` keystone (compact `Int` if it fits, else `Long`). | **[@PLN88](https://github.com/loft-lang/plans/issues/88)** | M–L |
| C2 | **D-own-3** | typed `Deps` (replace the overloaded `Vec<u16>`) — the substrate the rest of ownership reads | H2 ([DEPS_INVENTORY.md](../DEPS_INVENTORY.md)); *no issue yet* | M |
| C3 | **D-own-1, D-own-2, D-own-5, D-own-4** | the `deps` borrow checker: ownership computed once per binding/path; free/copy/move derive from one `deps` fact; `&`-borrow source tracked in `deps`; reverse the #415 copy-on-bind stopgap | **[@PLN85](https://github.com/loft-lang/plans/issues/85)** | L (the north star) |
| C4 | **D-cap-1, D-cap-2** | capability admission: field read/update/append are **landed** (F3–F6); remaining is the `…#default` parameter lock — gate a non-default argument at a sandboxed call site (D-cap-1, 6.9) — plus group-existence + `member_access` IR persistence (6.8); and carry a capturing closure's host references into the reachable-set (D-cap-2). **D-cap-3 folds into C3** — `Cap-Own` reads ownership's owned-vs-host fact. | **[@PLN86](https://github.com/loft-lang/plans/issues/86)** (§7 6.8/6.9) | S–M |

## Phase D — the operational arc (two projects: oracle + spreadsheet runtime)

| # | deviation(s) | project | direction |
|---|---|---|---|
| D1 | **D-op-1, D-op-2** | **DECIDED (2026-06): a differential oracle** — run a growing corpus on BOTH backends and assert they agree (value / null / halt / stdout / leak); the operational.md rules guide coverage. Turns the interp/native divergence class (D4/#433) from a coverage lottery into a caught failure. Switchable later to an executable shared semantics; the rules reuse either way. **Scope addition (2026-07-02, routing feedback): DRIVER agreement — well-typedness is ONE static judgment, so accept/reject must agree across `--interpret` / `--dump` / `--native` / `--native-wasm` for every corpus program** (the reported `--dump` divergence traced to mixed binaries, but the property needs a guard, not an assumption; a first-pass abort already makes the *diagnostic set* phase-dependent). Tracked: **[@PLN89](https://github.com/loft-lang/plans/issues/89)**. | code→spec (the chosen model) |
| ~~D2~~ **DONE** | ~~**D-op-4**~~ | **BUILT the spreadsheet runtime** (formalize4). Div/mod-by-zero and integer overflow now yield the null sentinel and CONTINUE on both backends (`raise_recoverable` + `checked_long!`→`i64::MIN`); OOB already complied; `NullDereference` was never raised. Two refinements vs the original plan: an UNGUARDED div0 reports a Warn log (`E-Report`), overflow is silent (rustc-release default). The `??` trap-suppression mode is gone behaviourally (the `*Nullable` op split is now dead code — separable cleanup). Guard: `tests/scripts/184-i333-div-zero-null-continues.loft`. | code→spec (C80), landed |

---

## Resolving order, in one line

**~~A1~~ ~~A2~~ ~~A3~~** clears Phase A **·** **~~B1~~ ~~B2~~ ~~B3~~** all decided (D-gram-2/4 →
decided edges C82/C81; grammar.md at 0) **·** binding.md + grammar.md + **types.md (@PLN25 CLOSED —
DN1–DN6, D2 reconciled; DN3 parse-flip landed)** + **~~D2~~ (D-op-4 spreadsheet runtime,
formalize4)** now **closed** **·** NEXT: the tracked arcs
**C2 (typed Deps) → C3 (@PLN85)** ownership, and the operational **D1** (differential oracle, @PLN89)
— the only open deviations left.

## What is NOT on this list (already clean or decided)

- types.md D1/D3/D4/D5, binding.md D-bind-0..6 + doc — **closed in code this cycle** (with tests).
- A row that turns out **spec-may-adjust** leaves `formal/` and becomes a decided edge — it is
  *resolved*, not deleted-by-fix. The deviation count is "distance from the current spec," and
  the current spec is allowed to be wrong.
