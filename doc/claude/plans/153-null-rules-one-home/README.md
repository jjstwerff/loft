<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 153 — Null rules: one home each, and the refusal at the point every escape ends up

Tracker: [@PLN153](https://github.com/loft-lang/plans/issues/153).

## Status

**Active — phase 0 done (2026-09-05), phase 1 in progress.**  The null MODEL is decided and not reopened here: @PLN102's
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
| **0** | **Probe first.**  Is `τ??` constructible today (`N-Idem` — `Type::optional` in `data.rs` is the candidate home), and is `N-Intro` the only null-direction conversion in `⤳`?  A corpus census with an env-gated report, since `[profile.dev.package.loft]` compiles a `debug_assert` OUT of the library. | The report over the 1247-file corpus reads 0 for `τ??` — and a probe that constructs one by hand makes it read 1, so the zero is not vacuous. | **Done** 2026-09-05 — § Phase 0 result |
| **1** | **Census: one home per N rule.**  For each of the 18, the predicate or emitter that decides it today.  Candidates: `N-Coal` → `parser/operators.rs::build_null_coalesce_default`; `N-Default` → the `x?` lowering + `Data::has_default`; `N-Store`/`N-Decl` → the `(N-Store)` teeth (`keys.rs`: the DN3 gate, the call-arg gate, the heap gate); `N-Prop` → `nullflow_enabled()`; `N-Join` → the inferred-assignment join; `N-Match` → the null arm; `N-Div`/`N-Arith`/`N-Cast`/`N-Cast?` → operator typing ([float-null-domain-typing.md](../102-stability-contract/float-null-domain-typing.md)); `N-Dense` → element storage; `N-Parse` → folded into `N-Cast`; `N-Index`, `N-Reserve`, `N-Store` already cited. | Per rule, ONE probe pair on both backends — a program where the rule must hold and one where its negation must be refused — green BEFORE the `@FR-` citation is added.  `rule_tags.py check` then reports 18/18 cited; a rule whose only evidence is the citation is the B6u failure and does not count. | Open |
| **2** | **`N-Prop` has 10 `nullflow_enabled()` sites in 4 files.**  Which question does each ask — propagate, gate, or warn?  Fold the fact-reading half onto one predicate; the per-site residue stays where it is per-site. | `introspect` output (IR, bytecode, Rust) byte-identical over the 1247-file corpus against the committed compiler (the B7r/B7s method), under the default AND under `LOFT_NO_NULLFLOW=1`. | Open |
| **3** | **The `N-Store` refusal at ONE point.**  Today the teeth sit at the local slot, the field, the return, the index, the call argument and the heap half as separate gates, each a spelling; a nullable reaching a non-null slot through a position none of them covers is answered wrong in silence.  One check where every store passes — the `⇐` lowering, whose ten push sites and six admission lists B6t already measured — is the chokepoint. | The Stage A matrix, position × type kind × discharge, with an `@EXPECT_WARNING` / `@EXPECT_ERROR` cell wherever a nullable meets a non-null slot undischarged and a silent cell wherever it is discharged; `make falsify` against the current build names the cells that pass silently today. | Open — the one design call |
| **4** | **The `optional` screen, ranked.**  The 352 opaque functions ordered by *can an undischarged value reach here*: declaration reads (a field's, a local's, a return's declared type) and lvalue places first, use-path sites last.  Each function in the top tier either peels through `base()` or is shown unreachable by a probe cell. | The gated `optional` audit row in QUALITY.md moves DOWN and every moved function has a cell; a peel added with no cell is the B6u receipt and is not counted. | Open |
| **5** | **`@FR-L-Null`'s 45 citations converged.**  The two questions B6u split — *what value means absent in this storage?* (the per-type sentinel table frozen in C90, `Stores::is_null`) and *is this the same storage?* (`base()`) — each have a home; every citation either reads it or is removed as a redundant spelling. | The site count goes DOWN, and where a change is a fold the emission is byte-identical over the corpus; where it is a fix, a probe cell. | Open |
| **6** | **Re-measure.**  `make bug-review` on the null/sentinel class after the plan's watermark against the window before it. | The class falls, or the residual names a mechanism this plan did not touch and a follow-on plan is filed for it. | Open |

## Phase 0 — result (2026-09-05)

**`τ??` is not constructible from a loft program, and the corpus reads 0 with an instrument that can read 1.**

*The former.*  `Type::optional` (data.rs:1781) is `(N-Idem)`'s home: `Optional(Optional(τ))
→ Optional(τ)`, and `Optional(Never | Null)` stays what it was.  The type parser's postfix `?`
(definitions.rs:2606) builds through it, and `pln25_optional_enabled()` is default-ON, so the
spelling `integer??` cannot nest by syntax.

*The routes around it.*  Thirteen sites construct `Type::Optional(Box::new(…))` directly
(`grep -rn "Type::Optional(Box::new" src/`).  Eleven are structure-preserving rewrites
(`rewrite_type_opt`, `rewrite_unknown`, the IR deserialiser, two tests) or wrap a provably bare
inner — a fresh `Text`, a fresh `Vector`, a `.base()`, a `Reference` just built.  ONE can nest:
`typedef.rs:184–191` peels a field's `?` off an `Unknown` forward reference, resolves the stub
to the alias's `returned`, and re-wraps (`set_attr_type_keeping_optional`, :233).  If the alias
resolves to `τ?` the field becomes `τ??`.  The hand-built probe for that route is
`struct S { f: Maybe? }` with `type Maybe = integer?` declared AFTER it, measured:
**it built one.**  The first write to the field was refused with the type spelled out — *"Cannot
assign to field 'f' of type integer??"* — and the same through a chain of two forward aliases.
Fixed on the spot: the re-wrap now goes through `Type::optional`, so the route is idempotent
like every other (`typedef.rs:233`, cited `@FR-N-Idem`).  Guard
`tests/scripts/153-a-forward-alias-field-keeps-one-optional.loft` (two route cells, one
declared-before control).  The per-probe census had read 0 for this program before the fix —
VACUOUSLY, because the program was refused before `scopes::check` ran and the three census
lines it printed were the stdlib loads'; running the probe for its own output is what showed
the type.  The sweep counts a refused file as `failed`, never as a 0, for exactly this reason.

*The census.*  `src/null_census.rs` — an observer beside `ownership_cfg::oracle` in
`scopes::check`, gated on `LOFT_NULL_CENSUS`, walking every declared return, attribute and
variable type through `Type::for_each_child` and counting an `Optional` directly under another.
Env-gated rather than a `debug_assert` because `[profile.dev.package.loft]` compiles those OUT.
`scripts/null_census_sweep.sh` runs it over the corpus: **`TOTAL nested=0 files=1258 failed=0` — every file reached `scopes::check`, none carried a `τ??`**.
Non-vacuity: `null_census::tests::a_hand_built_nested_optional_reads_one` — a hand-built
`Optional(Optional(Integer))` reads 1, at depth through a `Vector` still 1, a triple reads 2.

**Three resolution defects on the same route, found by the guard, not by the census.**  Each
is one fact held in two places, which is the plan's thesis met on its first afternoon:

- **F1 — a pass-1 operator error on a wrapped stub, never retracted.**  Pass 1 defers an
  operator whose operand is unresolved (`parser/mod.rs::…`, *"a genuinely unresolvable operand
  reaches this same site on pass 2"*), but the test was `Type::is_unknown`, which sees through
  `Vector` and nothing else, so `Optional(Unknown(alias))` — a forward alias behind a `?` —
  was judged settled, resolved against `unknown?`, and refused with *"No matching operator
  '==' on 'unknown?' and 'integer'"* for a program pass 2 resolves.  Fixed with
  `Type::has_unknown`, a walker over the `Type` keystone (exhaustive by construction), used at
  the deferral; `is_unknown` keeps its meaning at the settledness guards, where a wrapper over
  a stub IS a written type that must not be overwritten (91 callers, none touched).
  Measured: the nullable-reference alias `f: MP?` under `!=` was the same defect.
- **F2 — an annotated local's `?` overwritten by its first assignment.**  `x: Maybe? = 5` with
  `Maybe` declared below: the declaration is `Optional(Unknown)`, the assignment's
  `change_var_type` found no arm for a declared type carrying a stub and fell through to the
  unconditional write, the slot read `integer`, `resolve_unknown_stub` found no stub left to
  fill, and pass 2's `integer?` was refused as *"cannot change type from integer to
  integer?"*.  `LOFT_LOG=type_timeline:x` showed all three writes.  Fixed with the mirror of
  the existing loft#1073 arm: a declared type with an unresolved component is a placeholder,
  not a baseline, and is kept for the stub rewrite to fill.
- **F3 — a forward nullable-reference alias field lints as 'not null'.**  `struct S { f: MP }`
  with `type MP = P?` below: storage is the tagged `__nullable<P>` and `S { f: null }` reads
  null back correctly, so the VALUE is right — but `redundant-null-check` fires on `s.f != null`
  because it reads `matches!(ctp, Optional)` on the field-read type, and for the forward case
  that type is the STORED spelling (`__nullable<P>`), not the source spelling
  (`Optional(Reference(P))`) the declared-before case carries.  A wrong diagnostic, not a wrong
  value; it is the stored-vs-source spelling question `(L-Null-Which)` names, so it is phase 5's
  first cell rather than a spot fix here (probes `f3`, `f7`).

Guard: `153-a-forward-alias-field-keeps-one-optional.loft` — c1/c2 (the `τ??` route and a
chain), c3 (declared-before control), c4 (the annotated local, F2); F1 is what lets c1/c2 use
`==` at all.  ⚠ The first cut of c2 "passed" through interpolation — `"{s.f}"` formats an
`unknown?` value at runtime without type-checking it — and only `==` exposed the stub; a cell
that reads a value must read it through an operator the type checker owns.

**`(N-Intro)` is NOT the only null-direction edge in the code's `⤳`, and that is phase 3's
premise stated by the compiler.**  `Parser::convert` (parser/mod.rs:4389) has both directions:
- `should = Optional(inner)` recurses on the base — `τ ⤳ τ?`, `(N-Intro)`.  ✓
- `is_type = Optional(inner)` ALSO recurses — `return self.convert(code, inner, should)` — the
  implicit `τ? ⤳ τ` UNWRAP the rules say does not exist (`types.md`: *"there is NO implicit
  `S? ⤳ S`"*).  Its own comment says why: `convert` services comparisons (`x == null`) too, so
  the `(N-Store)` teeth *"must live at the STORE / decl / index sites"*.  So the rule is
  enforced by the ABSENCE of a refusal at every site that is not a store — the scattered-teeth
  shape phase 3 exists to replace, and the reason a position no gate covers answers wrong in
  silence.
- `implicit_checked_narrow` admits `Integer[s] ⤳ τ?` for a NARROW nullable target as a
  NULL-PRODUCING conversion (the value becomes null when it does not fit) — `(N-Cast?)` made
  implicit, not `(N-Intro)`.  It gets a cell of its own in phase 3's matrix.
- The `__nullable<S> ⤳ S?` arm is a representation change under `(L-Null-Tag)`, null-preserving
  in both directions; not a null-direction edge.

**What this settles for the phases after it.**  Phase 1's `N-Idem`/`N-Opt` home is
`Type::optional`, with this census as their evidence — citing them there is honest because the
zero was measured.  Phase 3 starts from `convert`'s unwrap arm, not from the store sites: the
question is which sites may keep the unwrap (comparison, `match`, `??`'s subject) and which
must refuse it, and the answer is a list, not a per-site flag.

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
