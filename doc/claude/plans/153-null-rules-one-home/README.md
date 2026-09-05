<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 153 — Null rules: one home each, and the refusal at the point every escape ends up

Tracker: [@PLN153](https://github.com/loft-lang/plans/issues/153).

## Status

**Open — census not yet run.**  The null MODEL is decided and not reopened here: @PLN102's
[keystone](../102-stability-contract/keystone-null-model.md) chose **B** (an in-band sentinel
for scalars, out-of-band absence for references and for a struct stored inline), frozen in
[DESIGN_DECISIONS.md § C90](../../DESIGN_DECISIONS.md), with @PLN25 (the dense element
model) and @PLN116 (`x?`) shipped on top of it.  What this plan changes is the rules'
REPRESENTATION IN CODE: of the 18 `@FR-N-*` rules in [types.md](../../formal/types.md) three
are cited (7 sites); the four `@FR-L-Null*` rules in [layout.md](../../formal/layout.md) are
cited from 60 sites that the rule-led walk CITED without converging — and the bug review
scored that walk **no effect**: null/sentinel rose from 17.9 % to 25.0 % of filed bugs
([BUG_REVIEW.md](../../BUG_REVIEW.md), 2026-08, third row).  Citing is the receipt, not the
conversion.

## Goal

Every `@FR-N-*` and `@FR-L-Null*` rule names ONE predicate or emitter that decides it, every
site that asks the question reads that home, and a value of a non-null type that would be
observably null is REFUSED at the one point every escape ends up — the store into a `τ` slot.
Done when the null/sentinel share of filed bugs falls in the bug review after the plan's
watermark, or the residual is characterised as a different mechanism.

## Effort + design

- **Effort:** H — six phases, each an afternoon to a day, none straddling a PR.
- **Design:** ~ — the model is decided (C90); the homes are to be located, and phase 3's
  chokepoint is the one design call (§ Open design questions).
- **Value category:** S (silent failure).  A sentinel that collides with a real value, or a
  `τ?` that reaches a `τ` slot undischarged, answers wrong with nothing said.
- **Last touched:** 2026-09-05.

## Why null is the hard family, measured

The 2026-09-05 evaluation of the rule-led walk queue (QUALITY.md B7q–B7s and the report that
followed) put null apart from every other family for three structural reasons:

1. **One fact, two representations.**  Nullability is a type former (`Optional`, the N
   rules) and a layout sentinel (the L-Null rules).  Every question is asked twice, at
   different levels, and no predicate answers both.
2. **No chokepoint by construction.**  `τ?` is discharged at the point of use — `??`, `?`,
   `match`, or a store into a non-null slot — so the question is asked wherever a value is
   read.  `scripts/ir_walker_audit.py optional` measures it: **703** functions discriminate on
   a `Type` variant and **352** of them are opaque to a wrapped shape (QUALITY.md B6p, B6s).
   The `?` survives at DECLARATION time and at LVALUE places; both defects of the B7h walk
   sat exactly there.
3. **A family the code does not represent.**  Fifteen N rules have no citation, so the walk
   method has nothing to split.  A rule with zero sites is not walked; it is first located.

The precedent that says what moves such a class is generics: a REFUSAL at the one point every
escape ends up (the type-variable row at allocation) took its share from 15.6 % to 4.0 %,
where folding onto a fact and citing did not.  Null has no such point today; phase 3 builds it.

## Composition matrix — Stage A

The axes this plan's cells vary, written as `/tmp` probes on `--interpret` first and graduated
to `tests/scripts/` as each phase's regression suite; every cell green on both backends:

| axis | domain |
|---|---|
| type kind | `integer` · narrow (`u8`…`u32`, `i8`, `i16`) · `float`/`single` · `boolean` · `character` · `text` · reference · struct stored INLINE (`__nullable<S>`) · `vector<τ>` · keyed |
| position | local · field · element · tuple member · argument · return · capture · literal |
| spelling of nullability | declared `x: τ?` · inferred join (`N-Join`) · `v[i]` (`N-Index`) · `a / b` (`N-Div`) · `e as τ?` (`N-Cast?`) · a library return typed `τ?` |
| discharge | none · `?? d` · `x?` · `match null` · `== null` test · store into a non-null slot |
| backend | `--interpret` · `--native` |

`python3 scripts/matrix_axes.py file <guard.loft>` is run on every guard this plan writes;
the axes it reports unreached are the cells still to build, not a note.

## Sub-arcs — small steps, each able to go red on its own

`Verify` names the comparison that would go RED if the phase were done wrong.

| # | Item | Verify | Status |
|---|---|---|---|
| **0** | **Probe first.**  Is `τ??` constructible today (`N-Idem` — `Type::optional` in `data.rs` is the candidate home), and is `N-Intro` the only null-direction conversion in `⤳`?  A corpus census with an env-gated report, since `[profile.dev.package.loft]` compiles a `debug_assert` OUT of the library. | The report over the 1247-file corpus reads 0 for `τ??` — and a probe that constructs one by hand makes it read 1, so the zero is not vacuous. | Open |
| **1** | **Census: one home per N rule.**  For each of the 18, the predicate or emitter that decides it today.  Candidates: `N-Coal` → `parser/operators.rs::build_null_coalesce_default`; `N-Default` → the `x?` lowering + `Data::has_default`; `N-Store`/`N-Decl` → the `(N-Store)` teeth (`keys.rs`: the DN3 gate, the call-arg gate, the heap gate); `N-Prop` → `nullflow_enabled()`; `N-Join` → the inferred-assignment join; `N-Match` → the null arm; `N-Div`/`N-Arith`/`N-Cast`/`N-Cast?` → operator typing ([float-null-domain-typing.md](../102-stability-contract/float-null-domain-typing.md)); `N-Dense` → element storage; `N-Parse` → folded into `N-Cast`; `N-Index`, `N-Reserve`, `N-Store` already cited. | Per rule, ONE probe pair on both backends — a program where the rule must hold and one where its negation must be refused — green BEFORE the `@FR-` citation is added.  `rule_tags.py check` then reports 18/18 cited; a rule whose only evidence is the citation is the B6u failure and does not count. | Open |
| **2** | **`N-Prop` has 10 `nullflow_enabled()` sites in 4 files.**  Which question does each ask — propagate, gate, or warn?  Fold the fact-reading half onto one predicate; the per-site residue stays where it is per-site. | `introspect` output (IR, bytecode, Rust) byte-identical over the 1247-file corpus against the committed compiler (the B7r/B7s method), under the default AND under `LOFT_NO_NULLFLOW=1`. | Open |
| **3** | **The `N-Store` refusal at ONE point.**  Today the teeth sit at the local slot, the field, the return, the index, the call argument and the heap half as separate gates, each a spelling; a nullable reaching a non-null slot through a position none of them covers is answered wrong in silence.  One check where every store passes — the `⇐` lowering, whose ten push sites and six admission lists B6t already measured — is the chokepoint. | The Stage A matrix, position × type kind × discharge, with an `@EXPECT_WARNING` / `@EXPECT_ERROR` cell wherever a nullable meets a non-null slot undischarged and a silent cell wherever it is discharged; `make falsify` against the current build names the cells that pass silently today. | Open — the one design call |
| **4** | **The `optional` screen, ranked.**  The 352 opaque functions ordered by *can an undischarged value reach here*: declaration reads (a field's, a local's, a return's declared type) and lvalue places first, use-path sites last.  Each function in the top tier either peels through `base()` or is shown unreachable by a probe cell. | The gated `optional` audit row in QUALITY.md moves DOWN and every moved function has a cell; a peel added with no cell is the B6u receipt and is not counted. | Open |
| **5** | **`@FR-L-Null`'s 45 citations converged.**  The two questions B6u split — *what value means absent in this storage?* (the per-type sentinel table frozen in C90, `Stores::is_null`) and *is this the same storage?* (`base()`) — each have a home; every citation either reads it or is removed as a redundant spelling. | The site count goes DOWN, and where a change is a fold the emission is byte-identical over the corpus; where it is a fix, a probe cell. | Open |
| **6** | **Re-measure.**  `make bug-review` on the null/sentinel class after the plan's watermark against the window before it. | The class falls, or the residual names a mechanism this plan did not touch and a follow-on plan is filed for it. | Open |

## Phase ordering

0 before 1 (a probe that kills the census cheaply).  1 before 2 and 3 (the homes have to be
known before they are folded or moved).  2 and 3 are independent of each other.  4 and 5 can
interleave with 3 and are the long tail; 4's top tier is worth doing before 3's matrix is cut,
because the declaration-time sites it names are where the matrix's undischarged cells land.
6 last, and only after a bug-review window has passed.

## Open design questions

1. **Where exactly is "the one point every escape ends up" for a store?**  B6t counted ten
   `⇐` push sites and six admission lists.  Phase 3 either finds the one lowering they share
   or first folds them — which is a phase of its own if the count is right.
2. **Does the heap half share the chokepoint?**  A nullable reference into a non-null field is
   `OpSetDbRef`-shaped, a scalar is `OpSetInt`-shaped; loft#1313 gave the heap half its own
   gate.  One check with two emitters, or two checks?
3. **`N-Store` is a WARNING except for narrow widths.**  Whether the warning becomes an error
   at the freeze is COMPATIBILITY.md's one-directional question and is NOT decided here; the
   plan lands the check where a later flip is one line.

## Cross-arc dependencies

- **@PLN152** (`status:next`) — `??` and `!` reaching the narrow widths.  Its steps 5–7 read
  the narrow-width `N-Store` ERROR and its message; phase 3 must keep that refusal where it
  is and its text what @PLN152 advertises.
- **@PLN102** (finished) — the keystone decision B and the null-flow flip
  ([nullflow-flip-plan.md](../102-stability-contract/nullflow-flip-plan.md)); phase 2 folds
  the sites that flip left behind.
- **@PLN25** (finished) — the dense element model; `N-Dense`'s home is its storage decision.
- The rule-led walk records this plan continues: QUALITY.md B6p, B6s (the `optional`
  screen), B6u (`@FR-L-Null`), B7h (`@FR-L-Null-Tag`).

## See also

- [formal/types.md](../../formal/types.md) — the N rules; [formal/layout.md](../../formal/layout.md)
  — the L-Null rules; [formal/README.md](../../formal/README.md) § When to reach for this doc.
- [keystone-null-model.md](../102-stability-contract/keystone-null-model.md) — the decided
  representation; [DESIGN_DECISIONS.md § C90](../../DESIGN_DECISIONS.md) — the frozen table.
- [STABILITY_METHOD.md § The rule-led walk](../../STABILITY_METHOD.md) — the method; the
  2026-09-05 evaluation in the session record that put null apart from lifetime.
- [BUG_REVIEW.md](../../BUG_REVIEW.md) — the null/sentinel class rows this plan is measured by.
- `scripts/ir_walker_audit.py optional`, `scripts/matrix_axes.py`, `make falsify` — the
  instruments; `LOFT_LOG=type_timeline:<var>` and `LOFT_DUMP_TYPES=1` for a single cell.
- [@PLN153](https://github.com/loft-lang/plans/issues/153) — this plan's issue.
