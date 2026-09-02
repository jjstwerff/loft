<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 152 — Carry validity beside the value (one null mechanism instead of three)

## Status

**Phases A and B are DONE; C is next.** loft encodes *"this operation could not produce a
value"* in three parallel places that do not agree with each other. This plan replaces them
with one mechanism: an operator computes **`(value, valid)`**, and the pair **collapses at
named boundaries** under a four-row rule — out-of-band *in flight*, in-band *at rest*, so
nothing about storage changes.

- **A** — the interpreter cost, which was the design's kill gate: **under 1 %**. Passed.
- **B** — the boundary matrix and the proposed rules: the value collapse is **already
  uniform across every boundary**, and **overflow is the only fault that can reach a slot
  with no spare code**. Both change what the remaining phases have to do; see
  [MEASUREMENTS.md](MEASUREMENTS.md), including a correction to this plan's own premise.

Tracker: [@PLN152](https://github.com/loft-lang/plans/issues/152).

## Goal

One rule — every operation yields a value and a validity bit; the bit collapses into the
slot's representation at each named boundary and that collapse is the only place a
diagnostic lives — replacing the per-operator `τ?` negotiation, the 14 `Op*Nullable`
duplicates, and the in-band sentinel's per-operand tests.

## Effort + design

- **Effort:** MH
- **Design:** ~ (the mechanism is settled; the collapse table's narrow-width row and the
  interpreter representation are open — see § Open design questions)
- **Last touched:** 2026-09-02

## The three encodings this replaces

1. **The type** — `τ?`, plus a per-operation negotiation about which ops earn it: `/`
   and `%` do (DN3), `+ - *` do not (C85), narrow-width `+=` does not (`(E-Uncomp-NN)`).
2. **The operator** — 14 `Op*Nullable` variants in `default/01_code.loft`
   (`OpAddIntNullable`, `OpDivFloatNullable`, `OpGetVectorNullable`, …), plus 6 `_nn`
   Rust variants that exist only to skip the cost encoding 3 imposes.
3. **The value** — an in-band reserved bit-pattern per width, tested on every operand:
   `ops::op_add_long` compares both operands against `i64::MIN` before it can add.

Measured behaviour today (`--interpret`, 2026-09-02) — one question, four answers:

| site | type says | runtime does | diagnostic |
|---|---|---|---|
| `b.n = v[i]` (non-null field) | non-null | holds null | warning |
| `x: integer` overflow | non-null | holds null | none |
| `f: float` div-0 | non-null | holds null (NaN) | none |
| `b: u8 = 250; b += 10` | non-null | **holds `0`** | none |
| `a: integer = 2; a = v[i]` | non-null | — | **hard error** |

**The fact already exists and is discarded.** `ops::checked_long!`'s `None` arm knows the
operation failed — it calls `note_integer_overflow(…)` and then returns `i64::MIN`,
throwing the fact into the sentinel encoding. This plan returns it instead of discarding
it, which is why the producer side is small.

## The collapse rule (the deliverable)

At a boundary, when the flag says invalid:

| target | action |
|---|---|
| `τ?` | write null, no report |
| non-null `τ`, width has a spare code | write the sentinel (reads as null) — today's `integer` / `i32` / `float` behaviour |
| non-null `τ`, width has **no** spare code (`u8` `i8` `u16` `i16`, and `u32` whose code is at the top) | write the type's default **and report** — the `0` stops being silent |
| the expression was discharged (`??`, `?`, `match`) | flag consumed, no report |

**Boundaries** (the collapse-point set): store to a local, struct-field write, collection
element write, call argument, return, `par` merge, serialisation. The flag **never crosses
one** — signatures keep `τ?`, storage keeps C90's sentinels. That bound is what makes the
design affordable, and it is sufficient, because `b += 10` is one expression.

## Impact on the formal definition

The rules move, and this is the half to review first — the mechanism is only worth
building if the rule set it leaves is smaller than the one it replaces.

### `formal/types.md` — the null-flow laws

| rule | impact |
|---|---|
| `(N-Domain)` | **Narrows.** A partial operation no longer needs to type `τ?` to be honest, because validity rides beside the value. The type obligation survives **at boundaries only** — a *function* that can answer absence still declares `τ?`. |
| `(N-Prop)` | **Reframed.** In flight, propagation is the flag's disjunction, not the type's. The type-level clause survives for values that cross a boundary. |
| `(N-Store)` | **Replaced** by `(E-Collapse)`. The warn-for-most-types / error-for-narrow-widths split disappears — it exists only because the type had to carry a fact the value could not. |
| `(N-Decl)` `(N-Join)` `(N-Coal)` `(N-Default)` `(N-Match)` `(N-Cast)` `(N-Cast?)` `(N-Dense)` `(N-Reserve)` | **Unchanged.** The discharge forms gain a second reading — they consume the flag as well as eliminating the type — but their statements stand. `(N-Reserve)`'s scope becomes explicitly *storage*. |

### `formal/operational.md` — evaluation

| rule | impact |
|---|---|
| `(E-Uncomp)` | **Restated.** An uncomputable operation yields `(value, invalid)`; the observable null appears at the collapse, not inside the op. The spreadsheet behaviour is unchanged. |
| `(E-Uncomp-NN)` | **Replaced.** It becomes row 3 of `(E-Collapse)` and gains a report. This is the rule that produces loft#1296's plausible `0`. |
| `(E-Null)` | **Split by position.** Its in-band claim is true *at rest* and false *in flight*. This is what finally makes E-Null's "private / unobservable" wording honest — the keystone recorded that wording as dishonest and settled for a doc fix; here it becomes true of the half it describes. |
| `(E-Report)` | **Generalised.** The collapse site is the one reporting home, instead of today's unguarded-div0 special case. |

**New rules** (`operational.md`):

- `(E-Valid)` — every operation yields a pair `(value, valid)`.
- `(E-VProp)` — validity propagates by disjunction: an operand that is invalid makes the
  result invalid.
- `(E-Collapse)` — the four-row table above. The new keystone rule.
- `(E-Boundary)` — the collapse-point set, enumerated and closed.

**Deviations.** `D-op-9` (loft#1265, closed 2026-09-01) left its central question open in
its own closing note — *"whether the checking form is emitted always or only under the
flag … is a measurement, not a judgement."* Phase A **is** that measurement, and a carried
flag makes the checking form free rather than conditional. `D-op-1` / `D-op-2` (no shared
operational semantics; backend divergence test-caught rather than definition-caught) are
*moved, not closed*: the collapse rule gets one home in the `#rust` template that feeds
both backends, so this class of divergence becomes unexpressible for these ops.

### `DESIGN_DECISIONS.md`

| decision | impact |
|---|---|
| [C85](../../DESIGN_DECISIONS.md) — overflow arithmetic types non-null | **Outcome preserved, mechanism replaced.** C85 exists because forcing `integer?` on every add *"poisons the common path"* — a complaint about the **type**. A flag is invisible in signatures, so the user-facing outcome (no `??` on ordinary arithmetic) survives while the exception it needed disappears. Needs a revision note, not a reversal. |
| [C90](../../DESIGN_DECISIONS.md) — one reserved bit-pattern per nullable scalar | **Scope narrows to storage.** The per-type reserved-value table stays for *slots*; it stops applying to *computed* values, so a computed `u8` 255, `i64::MIN`, `'\0'` and a real `NaN` are all representable in flight. C90's table is named part of the contract-1 freeze, so **this amendment must land pre-freeze.** |
| [C80](../../DESIGN_DECISIONS.md) — the spreadsheet fault model | **Unchanged and reinforced.** A fault stays a *value, not a control-flow event*; the flag is value-flow, nothing unwinds, no cleanup blocks appear. |
| DN3 / DN3-Float (`formal/types-history.md`) | **Simplified.** "Which operations type `τ?`" stops being a negotiated per-op list. |

### `COMPATIBILITY.md`

- The **reporting** version of row 3 adds no error, so it is legal even after contract 1.
- Changing what a narrow-width overflow *answers* (`0` → null) **is** a semantic change,
  so that variant is pre-freeze-only. Phase E decides which, and the decision is the one
  place this plan can spend the one-way door.
- Narrowing C90's scope to storage is likewise pre-freeze.

## Composition matrix — Stage A

The axes this change actually touches, per [README § composition axes](../README.md):
**type-kind** (wide scalar · narrow scalar · `float`/`single` · `character` · `boolean` ·
`text`), **storage context** (local · struct field · vector element · argument · return ·
captured), **access** (the collapse is a *store*, so both directions per axis), **null /
sentinel** (spare-code vs no-spare-code widths — the axis that splits the collapse table),
and **backend**. Construction path matters at one cell only (a literal operand is
statically valid).

The cell is *(operation × operand validity × target type × target nullability × backend)*,
hand-computed, asserting **value AND the report channel AND leak**. Derive the axis
coverage with `python3 scripts/matrix_axes.py`, not by hand — an axis named in a closing
paragraph is not an axis measured by the cells. Probes graduate to
`tests/scripts/152-validity-collapse.loft`.

## Sub-arcs

| Item | Source | Verify | Status |
|---|---|---|---|
| **A** — measure the interpreter cost: carry the flag on the integer arithmetic ops | [MEASUREMENTS.md](MEASUREMENTS.md) | one binary, `LOFT_FLAG=0/1/2`, interleaved; a control that must not move and a double-traffic mode that must | **DONE — passed, <1 %** |
| **B** — write the collapse table as proposed rules; build the boundary matrix | [`probes/`](probes/) | 19 value cells × 2 backends + a refusal probe and a diagnostic probe, each with its own control | **DONE — see [MEASUREMENTS.md](MEASUREMENTS.md)** |
| **C** — carry the flag through the five integer arithmetic ops, both backends, **sentinel still decides** | `ops::checked_long!` | always-on assert `flag_invalid ⟺ value == sentinel` at every collapse point, full suite, both backends (the parallel run) | Open |
| **D** — flip the collapse decision to the flag for the widths that already answer null | § the collapse rule | `introspect` + full suite **identical** to C; C's assert still holds | Open |
| **E** — narrow widths: the collapse site reports (loft#1296) | § COMPATIBILITY | a Rust test counting the stderr notice — `make falsify` cannot score a diagnostic; matrix cells for `u8`/`i8`/`u16`/`i16`/`u32` flip | Open |
| **F** — retire the duplicates: the 14 `Op*Nullable` and the `_nn` variants the flag subsumes | `default/01_code.loft` | `introspect` byte-identical IR before/after across the corpus + `fill_rs_up_to_date` | Open |
| **G** — provenance: widen the flag to carry the clearing site, opt-in | `STRONG_POINTS.md` § 12 | a probe where a null born deep in a call chain is reported with its originating line | Open |
| **H** — rewrite the formal rules, amend C85 / C90, close what closes | § Impact on the formal definition | `scripts/rule_tags.py check`; the formal conformance corpus | Open |

## Measurements

Phases A and B are measured; the numbers, the method and the corrections they
forced live in [MEASUREMENTS.md](MEASUREMENTS.md). Headlines: the flag costs
**under 1 %** on the interpreter (A), the value collapse is **already uniform across
every boundary** so Phase D is a no-op (B), and **overflow is the only fault that can
reach a slot with no spare code** — every sibling is refused, so C85 is the single
reachability door (B).

## Proposed rules (Phase B deliverable)

Draft for `formal/operational.md`; Phase H lands them.

```
  (E-Valid)     ⟨e, σ⟩ ⇓ (v, ok)      every operation yields a value AND a validity bit.
                                      ok = false iff the operation could not produce a
                                      representable result (E-Uncomp's condition).
  (E-VProp)     (v₁,ok₁) op (v₂,ok₂) ⇒ (v, ok₁ ∧ ok₂ ∧ ok_op)
                                      validity propagates by conjunction; an invalid
                                      operand makes the result invalid.  In flight this
                                      REPLACES the in-band sentinel test: the operand's
                                      value carries no null, its bit does.
  (E-Boundary)  a value crosses a BOUNDARY at: a store to a declared local, a struct-field
                write, a collection-element write, a call argument, a return, a `par` merge,
                and serialisation.  The bit does NOT cross — it collapses here.
  (E-Collapse)  at a boundary with target τ, when ok = false:
                  τ is nullable            ⇒ write null;                    no report
                  τ non-null, spare code   ⇒ write the sentinel (reads null); no report
                  τ non-null, no spare code⇒ write default(τ)              AND REPORT
                  discharged (?? / ? / match) ⇒ bit consumed;              no report
                A DECLARED local additionally rejects at compile time per (N-Decl); the
                collapse rule governs the runtime answer, not that static commitment.
```

`(E-Uncomp-NN)` becomes row 3. `(E-Null)`'s in-band claim holds below `(E-Boundary)` and is
false above it — which is what makes its "private / unobservable" wording true of the half it
describes.

## Phase ordering

1. **A before anything.** It is the cheapest phase and the only one that can kill the
   design. Native gets the flag nearly free — it is a bit the CPU already sets — but the
   interpreter needs a parallel flag stack or a widened eval slot on the hot dispatch loop.
2. **B before C**, so the matrix exists before the mechanism moves and can witness that
   nothing regressed.
3. **C → D → E** is the parallel-run sequence: build beside, compare exactly, then swap,
   then change one answer deliberately.
4. **F after D**, never before — the duplicates are the fallback while the flag is unproven.
5. **G and H** are independent of each other and can follow in either order.

## Open design questions

1. ~~**The interpreter representation.**~~ **ANSWERED by Phase A** — a byte-per-slot shadow
   indexed by `stack_pos` costs under 1 %, so the clean design stands and the
   static-approximation fallback is not needed. Two follow-ons it raises instead: the
   shadow must be **bounds-check-free** (a fixed-size array plus a mask removed two `jbe`
   per op), and **where the field sits in `State` matters more than the flag traffic does**
   — a 24-byte addition moved every benchmark 2–4 % on layout alone.
2. **Row 3 of the collapse table** — report-and-default (compat-safe forever) or
   answer-null (a semantic change, pre-freeze only). Phase E, and it is the one-way door.
3. **Does the flag survive a coroutine yield or a `par` merge, or collapse there?** Both are
   boundaries under `(E-Boundary)` as drafted; confirm against `formal/coroutines.md` and
   `formal/concurrency.md` before H.
4. **Provenance width** — a bit is a flag; a word is a flag plus an origin. Whether the
   origin rides in release builds or only under an env gate is a Phase-G measurement.

## Cross-arc dependencies

- **@PLN102** — `keystone-null-model.md` weighed **A** (out-of-band in storage) against
  **B** (in-band in storage) and chose B. This is a third option neither considered, and it
  avoids the three objections that killed A: a `τ?` wider than `τ`, `vector<τ?>` losing
  density, a store-layout break. The keystone is not reopened; its scope is narrowed.
- **@PLN25** — the null-flow model whose `(N-*)` laws this plan rewrites.
- **`PERFORMANCE.md` § `_nn`** — the NonNull lattice becomes unnecessary; retire that
  section's staging plan in Phase F rather than building it.

## See also

- [`formal/types.md`](../../formal/types.md) § Null-flow laws · [`formal/operational.md`](../../formal/operational.md) § E-Uncomp / E-Report
- [`plans/102-stability-contract/keystone-null-model.md`](../102-stability-contract/keystone-null-model.md) — the A-vs-B decision this extends
- [`DESIGN_DECISIONS.md`](../../DESIGN_DECISIONS.md) C80 · C85 · C90
- [`COMPATIBILITY.md`](../../COMPATIBILITY.md) § The error surface is one-directional
- loft#1296 · loft#1297 · [@PLN152](https://github.com/loft-lang/plans/issues/152)
