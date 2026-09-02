<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 152 — Carry validity beside the value (one null mechanism instead of three)

## Status

**Phase A is DONE and the design PASSED its kill gate** (2026-09-02): carrying the
flag costs **under 1 %** on integer-arithmetic-heavy interpreter workloads
(+0.51 % `02_sum_loop`, +0.77 % `01_fibonacci`, **0.00 % on the control**) — see
§ Phase A result. The rest is open, nothing else built. Originally: design settled, nothing built. loft encodes *"this operation could not produce a
value"* in three parallel places that do not agree with each other, and the
disagreements are the defect class. This plan replaces them with one mechanism: an
operator computes **`(value, valid)`**, and the pair **collapses at named boundaries**
under a four-row rule. Validity is out-of-band *in flight*, in-band *at rest*, so
nothing about storage changes. Phase A is a measurement whose job is to kill the design
if the interpreter cost is real.

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
| **A** — measure the interpreter cost: carry the flag on the integer arithmetic ops | § Phase A result | one binary, `LOFT_FLAG=0/1/2`, interleaved; a control that must not move and a double-traffic mode that must | **DONE — passed, <1 %** |
| **B** — write the collapse table as proposed rules; build the boundary matrix as `/tmp` probes | § the collapse rule | the matrix's red cells match the known-issue list exactly; an unexpected red is a finding, an all-green falsifies the premise | Open |
| **C** — carry the flag through the five integer arithmetic ops, both backends, **sentinel still decides** | `ops::checked_long!` | always-on assert `flag_invalid ⟺ value == sentinel` at every collapse point, full suite, both backends (the parallel run) | Open |
| **D** — flip the collapse decision to the flag for the widths that already answer null | § the collapse rule | `introspect` + full suite **identical** to C; C's assert still holds | Open |
| **E** — narrow widths: the collapse site reports (loft#1296) | § COMPATIBILITY | a Rust test counting the stderr notice — `make falsify` cannot score a diagnostic; matrix cells for `u8`/`i8`/`u16`/`i16`/`u32` flip | Open |
| **F** — retire the duplicates: the 14 `Op*Nullable` and the `_nn` variants the flag subsumes | `default/01_code.loft` | `introspect` byte-identical IR before/after across the corpus + `fill_rs_up_to_date` | Open |
| **G** — provenance: widen the flag to carry the clearing site, opt-in | `STRONG_POINTS.md` § 12 | a probe where a null born deep in a call chain is reported with its originating line | Open |
| **H** — rewrite the formal rules, amend C85 / C90, close what closes | § Impact on the formal definition | `scripts/rule_tags.py check`; the formal conformance corpus | Open |

## Phase A result — measured 2026-09-02

**Verdict: the clean design survives.** The gate was *"under a few percent, build it;
around ten, fall back to computing the flag only where an op can be invalid."*

| bench | flag off | flag on | 2× traffic | flag cost | 2× cost |
|---|---|---|---|---|---|
| `02_sum_loop` (treatment — `AddInt` is the hot op) | 1383 ms | 1390 ms | 1405 ms | **+0.51 %** | +1.59 % |
| `01_fibonacci` (treatment — `MinInt`/`AddInt` + calls) | 12581 ms | 12678 ms | 12846 ms | **+0.77 %** | +2.11 % |
| `08_word_count` (**control** — text scanning) | 332 ms | 332 ms | 337 ms | **0.00 %** | +1.51 % |

Medians, `--interpret`, release build, 15 interleaved reps (5 for fibonacci). That is
**0.35 ns/op ≈ 1.1 cycles at 3 GHz** over `02_sum_loop`'s 20 M `AddInt` — physically
credible for three L1-resident, pipelined byte accesses, which is the check that the
number is a measurement rather than an artefact.

**The mechanism is confirmed in the disassembly, not just the timing.** In the flag arm
the operand sentinel tests are *gone* — baseline's `cmp $0x8000…,%rdx; je` (with `v1`'s
test folded into `neg; jo`) is replaced by `add (%rax),%rbx; seto %al`. The plan's claim
that *"the flag is a bit the CPU already sets"* is `seto`, at the instruction level.

**The method matters more than the number, and phases C/D inherit it.** The first three
attempts all failed, each for a different reason worth recording:

1. **A shared box.** Another checkout ran `rustc` + six `mold` linkers mid-run; load hit
   56 and `08_word_count` read 1017, 602, 457, 361, 340, 332, 310 across seven reps — a
   monotone settle, not a measurement.
2. **Separate binaries are not comparable at this effect size.** A build whose only change
   was *adding the shadow field* — ops untouched, the new code dead — moved every
   benchmark by **+2 to +4 %**, treatment and control alike. Later, a flag build made the
   text control **2.6 % faster**, which no change to integer arithmetic can do. Adding
   24 bytes to `State` shifts every field offset and the whole code layout, and that
   swamps what is being measured.
3. **So the measurement must be ONE binary with a runtime-selected path** — identical
   struct, identical layout, only the executed instructions differ. That is where the
   numbers above come from, and it is how C and D must compare before/after too.

The instrument was checked in both directions rather than trusted: the **control must not
move** (0.00 %) and a **double-traffic mode must** (+1.5 to +2.1 %). Both held. The
double-traffic arm scaled 2.7–3.1× rather than a clean 2×, and the control moved 1.5 % in
that arm, which bounds the residual noise at ~1.5 % — so the flag cost is quoted as
*under 1 %*, not to two decimals.

**What this does NOT measure — the honest residual.** The probe carried the flag through
`add` / `sub` / `mul` on integers. A full implementation also needs every *value-producing*
op to write its slot's flag (one store), because the shadow is indexed by stack position
and would otherwise read a stale entry. Scaling from the measured per-access cost, that is
roughly another **0.6–1 %**, putting a complete implementation near **1.5–2 %** — still
well inside the gate, but it is an estimate, and Phase C's parallel run is where it becomes
a measurement.

Probe artefacts are throwaway and were reverted; `src/fill.rs` is generated, so the real
change belongs in the `#rust` templates and the generator, not in that file.

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
