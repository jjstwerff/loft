<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# FORMALIZATION.md — a formal definition as a rough-spot lens

> **Thesis.** A full formal definition of loft (grammar + static + dynamic +
> ownership semantics) is **premature as a deliverable** — the ownership/`deps`
> model is still aspirational and mid-migration, so writing it down now would
> canonize the very holes the team is closing. But the *act* of formalizing is
> valuable on its own: a rule you cannot write cleanly marks a rough spot. This
> doc records what that lens reveals, layer by layer, so the rough spots are named
> once instead of re-discovered as bugs.
>
> Companion to [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) (the runtime/memory
> "re-derived fact" map) and [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md) (the
> ownership beacon). This doc is the *front-end* counterpart: it adds the two
> rough spots the red-flag map structurally misses, because that map is scoped to
> runtime/memory/codegen, not syntax/types.

## Per-layer readiness

| Layer | Verdict | Why |
|---|---|---|
| 1. Grammar | describable, in flux | Informal EBNF exists ([LOFT.md](LOFT.md) § Summary of grammar) but defers all operator precedence + context-sensitivity to the backtracking parser |
| 2. Type system | describable, in flux | The `Type` enum is enumerable, but the typing *relation* is implemented-only (the `convert`/`cast`/`can_convert` trio + a conversion table), and `Deps` is fused INTO the type and mid-migration |
| 3. Dynamic semantics | not ready (encoding sub-layer ready) | "The interpreter is the spec" — no operational rules; but null-sentinel encoding + overflow trapping ARE pinned in prose ([LOFT.md](LOFT.md) § null / integer) |
| 4. Ownership / `deps` | not ready | [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md) self-labels "aspirational… not fully implemented," with open holes in its own table |
| 5. Inconsistencies | specifiable now | Mostly decided-and-guarded edges; one genuinely open design gap (const on struct fields, [INCONSISTENCIES.md](INCONSISTENCIES.md)) |
| 6. Stability | not ready (ownership) / ready (rest) | Store-lifetime class reopened 2026-06-21 (@PLN85); the rest is low-bug |

## The rough spots, ranked by leverage

1. **Ownership/`deps` is fused into `Type` — the deep root.** Every heap-type
   variant carries a `Deps` list, computed by heuristics N times rather than as one
   fact. You cannot write a stable typing relation while the type's own contents are
   still moving. This is already the active work (red-flag clusters A/C, @PLN85;
   the `&`-binding law @PLN87). Everything below is partly downstream of it.

2. **The typing / conversion relation is a *table*, not rules.** There are no
   typing judgments; the closest artifact is the conversion trio + an 11-pair table.
   That ad-hoc-ness is where the integer-coercion bugs live. Evidence:
   - **#432** — an untyped vector literal kept 8-byte stride into a `vector<u8>`
     parameter; the fix had to add a *fourth* expected-type side-channel
     (`vector_hint`, alongside `lambda_hint`/`enum_hint`/`read_target_type`). Four
     parallel channels for "the expected type" is the smell: one rule should carry it.
   - **#433 residual** (reported 2026-06-23) — a local whose width is decided
     **across branches** (`arg = 0` … `arg = bytes[i]` … `arg = arg*256 + bytes[k]`)
     stays `u8` natively instead of widening to the integer join, so it overflows and
     mismatches the `i64` it is later used as (cbor `read_value`, ~10× E0308). The
     #433 seam fix (widen a narrow value-block at a return/assign) does NOT reach a
     variable whose *type* is inferred narrow across branches. A principled "the type
     of a multiply-assigned local is the join of its assigned types" rule is what is
     missing.

3. **No shared operational semantics → interp/native divergence is structural.**
   The interpreter is the spec; the native backend is a *separate* generator kept in
   agreement by tests. **#433** itself was an interp-vs-native divergence (native
   E0308, interpreter fine), and its residual still is (`--interpret` + `loft test`
   stay green; only the native binary breaks). A small-step semantics both backends
   must satisfy would make that divergence a definitional error, not a test-caught one.

4. **The grammar under-specifies precedence and is not context-free.** The informal
   EBNF collapses all binary operators into one rule — real precedence lives only in
   `src/parser/operators.rs`. The lexer's Formatting mode and speculative backtracking
   (type-vs-variable, struct-init-vs-block) make the surface context-sensitive. Stable,
   but unwritten: reasoning about precedence today means reading the parser.

5. **Local irregularities.** `Enum(u32, bool, Deps)` overloads a bool for
   value-vs-reference; narrow-int nullability shrinks the value range to steal a
   sentinel; `const` silently does not apply to struct fields (the one genuinely open
   design gap, [INCONSISTENCIES.md](INCONSISTENCIES.md)).

## The meta-insight

The formalization lens **confirms the red-flag map and adds to it**. Spots 1, 3, and
the null-codec are already in [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md). But
spots **2 (the front-end typing/conversion relation)** and **4 (grammar precedence)**
are NOT — the red-flag map is scoped to runtime/memory/codegen. #432 is the proof: it
was not any red-flag cluster; it was an unwritten typing rule. So a formalization pass
is a *complementary* instrument to the red-flag sweep, not a duplicate.

## Recommendation — formalize as a layer-by-layer instrument, not a deliverable

- **Highest leverage: write the typing / conversion relation as actual rules**
  (spot 2). It would force the four expected-type side-channels into one rule and
  surface the remaining coercion gaps (the next #432 / #433-residual) as *unprovable
  cases* before they ship.
- **Write a small-step operational semantics for the stable core** (spot 3) as the
  shared contract interp and native must both satisfy — turning interp-vs-native
  parity from a test invariant into a definitional one.
- **Pin the grammar with precedence** (spot 4) — cheap, mechanical, removes a
  "read the parser" tax.
- **Defer the ownership-model formalization** (spot 1) until @PLN85 / @PLN87 close.
  At that point [OWNERSHIP_MODEL.md](OWNERSHIP_MODEL.md)'s named invariants become the
  soundness theorems to formalize *against*, not guesses to write down.
