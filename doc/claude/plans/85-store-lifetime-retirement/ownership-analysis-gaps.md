<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Ownership analysis — validation + the exact gaps (what the compiler must do)

The Stage-1 `Owned|Borrowed|Join` classifier (`src/use_analysis.rs`) is the INPUT the
Stage-3 compiler fix reads. Before wiring any free site we must know its gaps — *"we
can only detect what we need to do in the compiler once we know the exact gaps in the
analysis"* (user). This is that validation.

**Instrument:** [`fuzz/classify_vs_runtime.py`](fuzz/classify_vs_runtime.py) correlates,
per probe cell, the **classification** (the `OWN fn=…` dump) against the **runtime
over-free outcome on BOTH backends** (CRASH/LEAK, with compile-errors + asserts
separated out — they are not over-free signals). Run over the generated 54-cell matrix
(`grammar_gen.py`) + the 28 `probes/over-free-sweep/` probes + the 5 `462-*` repros.

## The headline: the analysis is SOUND (zero misses)

> **Across all 87 probe cells, every value that actually over-frees (CRASH or LEAK) is
> classified `Join` or `Borrowed` — NEVER pure `Owned`.** The analysis never tells the
> compiler "free this" about a live borrow. `SOUND MISSES: NONE`.

This is the load-bearing property: the classifier is a SOUND foundation. The gaps below
are about **completeness** (surfacing the free SITES) and **precision** (carrying the
base, scoping), not soundness.

## The gap map (none-churn matrix; classification × runtime)

| shape (struct) | class at escape | interp | native | reading |
|---|---|---|---|---|
| **elem_accumulate** | `pick:ret=Join` | **CRASH** (UAF) | clean | flag-OK; **interp-only** over-free |
| **match_return** | `deliver:ret=Join` | **CRASH** (UAF) | clean | flag-OK; **interp-only** over-free |
| **local_source** | `one:reassign(chosen) prior=Owned rhs=Join` | **LEAK** | **LEAK** | flag-OK; **both backends** |
| field_return / field_local / field_reassign / nested_field | `…:ret=Borrowed` | clean | clean | safe borrow — "don't free" is correct |
| index_read | `deliver:ret=Join` | clean | COMPILE-ERR | Join accurate; native bug (below) |
| if_return | — | COMPILE-ERR | COMPILE-ERR | doesn't type-check (below) |
| *all scalar* | `ret=Owned` / `ret=Join` | clean | clean | scalar never over-frees |

Key refinement: **`elem_accumulate` and `match_return` over-free on INTERP only — native
is already correct** (right values, no leak). Only `local_source` leaks on both. So native
is a correctness REFERENCE for two of the three shapes — mirror what it does, don't invent.

## The exact gaps — and the compiler action each unblocks

> **UPDATE — Gaps A + B are now CLOSED (Stage 1.5, still inert).** `use_analysis::free_sites`
> surfaces both missing free sites, each with the freed value's class + the borrow base; the
> `ownership_surfaces_free_sites` test pins them, and the free sites now correlate EXACTLY with the
> over-free outcome (see § Stage 1.5 below). The original gap text is kept for the record.

### Gap A — the analysis classifies VALUES; two of three fixes act at a FREE SITE it doesn't surface
The over-free decision lives at a free site, and the analysis currently exposes only two of
the three site kinds:

| shape | the over-free SITE | analysis surfaces it? | compiler action |
|---|---|---|---|
| `local_source` | the owned-slot **reassign** | ✅ `reassign_sites` gives `prior=Owned rhs=Join` | **READY** — wire `scopes.rs` free-placement (both backends leak) |
| `elem_accumulate` | the **append element source-free** (`OpCopyRecord` `0x8000` on `pick`'s Join return, in `collect`) | ❌ only `pick:ret=Join`, not "collect's append source-frees a Join" | needs **append-source-free site classification** → then fix interp's source-free |
| `match_return` | the **arm-return delivery** (`materialize_vector_arms_into` reassigns the buffer to a borrowed enum field) | ❌ only `deliver:ret=Join`, not the delivery site | needs **arm-return delivery site classification** → then fix interp's delivery |

So: **`local_source` can be wired NOW; `elem_accumulate` + `match_return` need the analysis
extended to surface their free sites first** (a Stage-1.5 increment — still inert, still
testable, before any emit change).

### Gap B — `Own::Borrowed`/`Join` carry no BASE
The Join fix is "materialise the borrow arm to owned at the escape" — which needs to know
WHICH store to copy. `Own::Borrowed` is currently opaque (Stage 1 dropped base tracking).
**The fact must carry the borrow base** (the projection's arg-0 / source var) before the
materialise can be emitted.

### Gap C — `Join` is over-flagged on clean cells (precision) — but closed by acting at the SITE
`match_return scalar` and `index_read` classify `Join` yet run clean. A value-level
"materialise every Join" would do needless work and risk regressing them. **Resolved by
Gap A's framing:** act at the FREE SITE, which only fires for record-element stores — so
scalar joins are never touched, no narrowing predicate needed. (Confirms the fix is
site-driven, not value-driven.)

## Adjacent bugs the sweep surfaced (NOT the over-free class; branch-internal)

Both around an empty `[]` literal used as a `vector<T>` default in a branch/coalesce tail
— a single likely root (empty-vector tail not typed/coerced to `vector<T>`):

1. **`if cond { v.rows } else { [] }`** → `error: expected vector<E>, got void on else`
   (the whole `if_return` shape fails to type-check). POISON-independent.
2. **`vv[i] ?? []`** → interp clean, **native `error[E0308]: mismatched types`** (the
   `index_read`/#426B shape; an interp↔native codegen divergence).

These block two probe shapes from even reaching the over-free analysis. Documented here
(branch-internal, stacked on @PLN25); not the join-ownership work — fold or file separately.

## Stage 1.5 — the free sites + base, surfaced (DONE, inert)

`use_analysis::free_sites(data, d_nr)` reports, per function (also dumped as `OWN fn=… free …`):
- **`AppendSource`** — each `OpCopyRecord(src,_,tp)` with the `0x8000` source-free bit, when `src`
  classifies `Borrowed`/`Join` (the `out += [pick()]` site). `elem_accumulate`'s is `Join`,
  `base=None` (the source is the inline `pick()` call → materialise = deep-copy the whole value).
- **`ParamDeliver`** — a heap-parameter return buffer reassigned to a **direct** `Borrowed`/`Join`
  projection (`base.is_some()`). `match_return`'s is `_mv_items_1 = OpGetField(e,…)`, `class=Borrowed`,
  `base=e` (the field to copy into the buffer).

The free sites now correlate **exactly** with the over-free outcome across the matrix:
`elem_accumulate`→`AppendSource`, `match_return`→`ParamDeliver`, `local_source`→the reassign site;
**every clean shape reports no site.**

**Precision finding (the `base.is_some()` filter on `ParamDeliver`):** `field_reassign`'s `best =
rows(b)` reassigns the retbuf via a CALL that delivers into it — which MATERIALISES a copy (best is
genuinely Owned), unlike `match_return`'s raw `OpGetField` projection (which aliases). The
discriminator is the base: a direct projection has a local base; a retbuf-materialising call has
none. Filtering `ParamDeliver` to `base.is_some()` drops the clean `field_reassign` false-positive
(which would otherwise mislead Stage 3 into NOT freeing an owned store → a leak) and guarantees every
reported site carries a usable materialise base. **Residual (Gap C, deferred):** `match_return scalar`
still reports `ParamDeliver` (same projection shape) though it runs clean — materialising it is safe
(a redundant copy); Stage 3 may gate on a record-element type if it matters.

## The unification — one oracle every site reads (in progress)

The per-site approach hit a wall at `elem_accumulate`: my codegen gate grew to a
**10-condition thicket** — itself the red flag `STABILITY_METHOD.md` warns about (a fact
re-derived through a conjunction of proxies). And the validation showed native's *own* inline
guard re-derives the same decision and gets the owned arm **wrong** (it materialises it →
`462-elem-accumulate-owned-branch-CLEAN` leaks on native). So both backends carry a wrong
per-site re-derivation; a third copy — however correct — is the wrong structure.

The fix is the OWNERSHIP_MODEL collapse: **one ownership oracle** every own-vs-borrow site
*reads* instead of re-deriving (the study's "single unification refactor"). The carried fact is
`Own { Owned | Borrowed{base} | Join{base} }` — the `base` is the var the value aliases, the
witness the `Join` runtime guard needs.

**Step 1 — DONE (inert):** `use_analysis::ownership_of` (the consolidation of Stage 1 `classify`
+ Stage 1.5 `borrow_base`) now folds the `base` into `Own` and resolves it
**interprocedurally** — a call's borrowed-view return maps the callee's borrowed param to the
CALLER's argument (`collect`'s `out += [pick(t,i)]` source resolves to base `t`, the witness).
Validated by `ownership_resolves_the_borrow_base` against hand-computed ground truth across the
shapes (`pick→Join(base=t)`, `deliver→Join(base=e)`, `nested→Borrowed(base=o)`, the `pick_cond`
reassign borrow arm → `pool`). Suite byte-identical (inert).
- KNOWN APPROXIMATION (pinned in the test): a whole-field / whole-arg return delivered through
  `__retbuf` resolves its base to `__retbuf`, not the true source (`getf`/`whole`). Harmless —
  clean field-return sites, never an over-free — but a precision gap a later fix should close.

**Step 2 — DONE (first chokepoint collapse, interp, `LOFT_JOIN_OWN`):** the interp first-bind
(`state/codegen.rs`, the `owned_ref` call-bind) now READS the oracle instead of the type-shape proxy.
A `Join` return bound into an owned slot emits `OpBindOrCopy` — the runtime arg-aliasing guard
**witnessed by the oracle's interprocedurally-resolved base** (collect's `t`): it ADOPTS the owned
`m_none()` arm (the source-free then frees it) and MATERIALISES the borrowed `t[i]` arm (the
source-free hits the copy; `t` intact). This FIXES BOTH arms of `elem_accumulate` on interp — the
thing the per-site re-derivation could not, because the witness needed the interprocedural base only
the oracle resolves. `join_own_fixes_elem_accumulate_interp` pins it; suite green flag-off; clean
shapes + scalar (`pick` returns Owned → no bind) + `local_source` all unaffected.
- The bytecode-stack discipline that bit twice: a PUSH op (`OpVarRef` witness) takes `var_pos` BEFORE
  `add_op` (reads at the pre-push position); the call result must be on the stack first so the
  witness offset accounts for it.

**Step 3 — DONE (native first-bind collapse, `LOFT_JOIN_OWN`):** `generation/dispatch.rs::output_set`
now reads the oracle too. For a `Join` return it replaces the buggy `_src.store_nr == _dst.store_nr`
guard with the oracle's base WITNESS: `adopt iff _src is null OR does not alias var_<witness>`, else
materialise. This fixes native's owned-arm leak (`owned-branch-CLEAN` was LEAK on native → clean).
**`elem_accumulate` is now correct on BOTH backends, both arms** — and both backends READ THE SAME
fact (the oracle), so they cannot diverge on it. `join_own_fixes_elem_accumulate_both_backends` pins
it; suite green flag-off.

**Next: `match_return` — DIAGNOSED (a different, deeper mechanism than the first-bind).** Both
backends ALIAS the retbuf to the enum field (`_mv_items_1 = OpGetField(e,4)`; native emits the same
`DbRef{..pos+4}` alias). The interp LEAK is NOT the delivery — `LOFT_LEAK_SITES` pins it to `cell`
(the `Filled` enum) + its `inner` vector in MAIN: because the returned vector aliases `cell`'s field,
interp's free analysis conservatively SKIPS freeing `cell`/`inner` (to keep the alias valid) → they
leak. Native aliases too but frees them anyway. So the chokepoint is the over-free class's LEAK
mirror (a borrow alias suppresses the owner's free), and the fix is to MATERIALISE the Filled arm —
copy `e`'s items into the cleared retbuf so `deliver` returns OWNED, breaking the alias and letting
`cell`/`inner` free normally. The `ParamDeliver` site (`Borrowed(base=e)`) is already surfaced by the
oracle; the fix is to MATERIALISE the aliasing Set (`_mv_items_1 = OpGetField(e,4)` → clear +
append-copy into the retbuf's own store) and strip the retbuf's `["e"]` dep so the return is owned.
Likely interp-only (native already frees correctly).

**HUNT COMPLETE (path fully traced + confirmed by `LOFT_TRACE_RR`):**
1. `parse_match_enum_field_bindings` (`control.rs:2715-2736`) creates the field binding
   `v_set(_mv_items_1, OpGetField(e, attr))` — CORRECTLY a borrowed view (`set_skip_free`, dep
   `["e"]`); right for a read-only use.
2. **`ref_return` (`control.rs:4965`, the promotion loop at ~5084) NRVO-RENAMES that borrowed-view
   local onto the retbuf** — `LOFT_TRACE_RR` confirms: `fn=n_deliver ls=[2] ls_tps=["_mv_items_1=
   Vector(.., Deps{items:[0]})"]` (dep `[0]` = borrows `e`), and the merged `ret` becomes
   `Deps{[1,0]}`. So `deliver` returns a BORROW of `e`. (`materialize_vector_arms_into` is NOT this
   path — it's never called for `deliver`; ref_return's `#306` dep-merge at `control.rs:5051` is.)
3. The borrowed return makes `main`'s free analysis conservatively SKIP freeing `cell`+`inner` (to
   keep the alias valid) → they leak on interp; native frees them anyway.

**THE DISCRIMINATOR (confirmed, surgical):** the promoted candidate's deps. `field_return`/
`nested_field` promote a PARAMETER (`b`/`o`) with EMPTY deps (the clean borrow-of-param return);
`match_return` promotes a LOCAL (`_mv_items_1`) with NON-EMPTY deps `[e]` (the problematic alias). So
"promoted candidate `tp(v).depend()` non-empty" separates them exactly.

**SKIP APPROACH RULED OUT (attempt 2, reverted).** Skipping the NRVO-rename for a non-empty-dep
candidate (`continue` in the promotion loop, like the `reassign_count ≥ 2` skip) does NOT route it
through a copy path — it just un-promotes: `deliver` then returns the borrow `_mv_items_1` (`["e"]`)
with `__retbuf` left UNFILLED → SIGSEGV (matrix `none`), worse than the leak. The reason:
`vec_match_candidate`/`materialize_vector_arms_into` (the copy delivery, `control.rs:731`) is FALSE
for `deliver` (it took the ref_return path, not the materialise path), so un-promoting leaves no one
to fill `__retbuf`. `match_return` therefore needs an **EXPLICIT materialise** — copy `_mv_items_1`'s
elements into `__retbuf` at the promotion (the fresh-buffer + `OpAppendVector` idiom) and yield
`__retbuf` — OR a fix to make `vec_match_candidate` fire for `deliver` so the existing materialise
delivery runs. This is the hardest site: TWO competing return-delivery paths (NRVO promotion vs
vector-arm materialise), and `deliver` falls into the one that can't materialise. Sites 1+2 stay
fixed on both backends; `match_return` is left at baseline (its broken attempts fully reverted).

**THE PRECISE DISCRIMINATOR — pinned in `ownership_pins_match_return_resisting_cases` (unit-tested
the new routine on the resisting cases, instead of flailing on the codegen).** Comparing the oracle
across match-return variants gives the grip:
| variant | promoted local (deps) | oracle RETURN | `ParamDeliver`? | runtime |
|---|---|---|---|---|
| `deliver` (Filled→items / else→[]) | `_mv_items_1` deps `[e]` | `Join(base=e)` | YES `Borrowed(base=e)` | leaks |
| `deliver2` (two field arms, both borrow `e`) | `_mv_xs_1` deps `[e]` | `Join(base=e)` | YES `Borrowed(base=e)` | leaks |
| `deliver3` (Filled arm builds a FRESH `o`) | `o` deps `[]` | `Join(base=o)` | **NO** | clean |
- **The `Join` RETURN verdict is NOT the fix signal** — it over-classifies `deliver3` as `Join(base=o)`
  (the retbuf-param approximation: the owned retbuf `o` classified as borrowed-of-itself), yet
  `deliver3` is runtime-clean and must NOT be touched.
- **The precise signal is the `ParamDeliver` FREE SITE** (a retbuf aliased to a borrowed enum-field
  view whose base is an EXTERNAL var `e`) — equivalently, the promoted candidate's NON-EMPTY deps.
  The genuinely-leaking arms have it; the fresh-build does not. (This is why the earlier skip's
  *discriminator* was right — it keyed on non-empty deps — and only its *action* was wrong.)
So the fix is now spec'd precisely: **materialise exactly the `ParamDeliver` arms** (copy the
borrowed field into the retbuf), leaving fresh-build arms (`deliver3`) untouched. The analysis side
is verified; the remaining work is purely the emit (the explicit copy at the retbuf promotion).

**EMIT — NATIVE COMPLETE, INTERP structure recovered, one residual (gated `LOFT_JOIN_OWN`,
off-default → suite byte-identical). Built FROM the proven IR (`deliver3`), not a hand-roll.**
`ref_return` (`control.rs`, before the promotion), for a borrowed-view match-field binding (`skip_free`,
non-empty deps, `v != buf_var`): delivers each arm into the SEPARATE `__retbuf` via the proven
per-arm machinery `materialize_vector_arms_into` (with a no-free for the `skip_free` borrowed binding
— it aliases the subject, doesn't own a store), marks the binding to **skip promotion** (in the
`#306` dep-walk AND the promotion loop, so the owned copy's return doesn't re-acquire the `["e"]`
borrow), and sets `returned = Vector(elm, Deps::attrs([buf_attr]))` (as `Delivery::Materialize` does).
The earlier hand-roll (`materialise_borrowed_return_local`, now deleted) reused the binding var AS the
buffer — the IR-diff against `deliver3` showed that was the bug (`["_mv_items_1"]`-typed vs the proven
`["__retbuf"]`). **NATIVE: fully clean** (462 + matrix `none`/`stress`, both arms). The IR now matches
`deliver3`'s structure: separate `__retbuf` buffer, the binding a local, copied per-arm.

**THE ONE INTERP RESIDUAL (precisely diagnosed by the `deliver3` diff — a LEAK, POISON→crash).**
`main` doesn't free the passed `cell` (the original leak). `deliver3` (clean, frees `cell`) is the
PROMOTION form `if {…fill buffer…}; return null` with `["??"]`; mine is the MATERIALISE form `return
if {…; __retbuf}` with `attrs` — returning the caller's OWN buffer, so the caller neither adopts nor
frees the argument.

**SYNTHESIS LANDED (WIP, gated `LOFT_JOIN_OWN`, off-default → suite byte-identical 746✓) — STRUCTURE
EXACT, one parser-internals blocker.** `jo_copy_borrowed_arm_yield` (in `parse_match`, after each
arm body): if the arm yields a `skip_free` vector field binding directly (`Filled { items } => {
items }`), it wraps the yield in an OWNED copy `{ o = []; o += items; o }` — a fresh local `o` created
with the OWNED element type (NOT the binding's `["e"]` type, which re-propagates the borrow), `o = []`
via the existing `vector_db` helper, `o += binding`, yield `o`. Done at PARSE time (re-parsed each
pass) so the EXISTING `ref_return` promotion then promotes `o`. **RESULT: the deliver IR is now
IDENTICAL to `deliver3`** — sig `["??"]`, var table (`0 e`, `1 __retbuf` marker, `2 _mv_items_1["e"]`
source, `3 arg _mvcopy_1` owned buffer), and the CALLER adopts (`r:vector["__ref_1"]`) + frees the
argument (`OpFreeRef(cell)`). Native fully clean; the simple case (`deliver` alone) is clean + value-
correct (asserts pass) on interp too. The earlier "POISON crash" was an INSTRUMENT ARTIFACT — `deliver3`
itself SIGSEGVs under `LOFT_POISON` on this shape.
**THE BLOCKER — CORRECTED (the "def-numbering" diagnosis below was WRONG; probing untangled THREE
separate things).**
1. **The matrix crashes were MASKING a PRE-EXISTING parser bug.** Defining `e_default` AND `filler`
   together (both DEAD code, never called) crashes interp **flag-OFF** — `deliver`+`e_default` alone is
   clean, `deliver`+`filler` alone is clean, only BOTH crash (`d_nr=u32::MAX`). EVERY matrix file
   carries both, so their "match_return CRASH" was largely this bug, not the over-free or my synthesis.
   → file as a separate issue (a 2-function parser def/type corruption).
2. **The "POISON crash" was an INSTRUMENT ARTIFACT** — `deliver3` itself SIGSEGVs under `LOFT_POISON`
   on this shape, so POISON verdicts on this family are unreliable.
3. **The REAL residual: the copy mechanism is broken, not the structure.** On a CLEAN over-free repro
   (`mr-clean`: borrowed-binding return + churn pressure, NO `e_default`/`filler`) the synthesis produces
   the exact `deliver3` IR but is **non-deterministic** (flips clean/crash) and a two-arm variant
   ASSERTs (wrong length). Cause: `o += items` (whole-vector append of a BORROWED, `skip_free` source)
   is SHALLOW on interp — `copyv` (`o += <OWNED param>`) is deep and survives churn. So `r` still
   aliases the subject. `deliver3` deep-copies element-wise via `?? e_default()` — but that needs a
   default constructor, NOT generally synthesisable. NEXT (the real task): either FIX the interp
   `OpAppendVector` to deep-copy a borrowed source (general improvement, see `fill.rs::append_vector`),
   or synthesise an element-wise deep copy without a default. (The earlier `ref_return` materialise
   block is vestigial — the synthesis supersedes it; clean up once the copy is deep.)

**CORRECTION — NOT a deep-copy issue either; it is a PRE-EXISTING non-deterministic corruption in
`o += <match-binding>` inside a match arm.** Reproduced in PLAIN loft (no synthesis, flag-off):
`match e { Filled { items } => { o: vector<E> = []; o += items; o }, _ => { [] } }` + churn crashes
interp **non-deterministically** (CRASH / clean / CRASH across identical runs — the uninitialised-mem /
UAF signature); native clean. Decisive isolations:
- A heap-free `struct E { hp: integer }` crashes IDENTICALLY → NOT the shallow heap/text copy
  (`vector_add`'s inline byte-copy was a red herring).
- The SAME whole-append in a PLAIN function (`copyf`: `fn(b: Box) { o = []; o += b.rows; o }`) is CLEAN
  → it is the MATCH-ARM context, not the append op.
- The element-loop (`deliver3`: `o += [items[x] ?? d]`) is CLEAN → it is the WHOLE-vector append
  specifically, in a match arm, with the `vector<ref(E)>` binding as source.
Repros: `match_return-emit/PREEXISTING-whole-append-in-match-nondeterministic.loft` (+
`CONTROL-whole-append-in-plain-fn-clean.loft`). A PRE-EXISTING interp bug independent of the @PLN85
synthesis (which merely GENERATES this pattern). NEXT: a dedicated debug session (boundary matrix on
whole-append-in-match-arm + `LOFT_LOG=minimal` to find the uninitialised/UAF op), OR sidestep by having
the synthesis emit the element-loop (proven clean). Lesson: several wrong targets were chased
(def-numbering, POISON, deep-copy) — each disproven by an isolation probe; isolate the variable
(heap-field, match-context, append-shape) BEFORE naming the fix.

**SETTLED — the analysis is SUFFICIENT; the fix is purely codegen STRUCTURE (synthesise `o`).**
Probed the question "can the analysis aid here?": the oracle classifies BOTH `deliver` (materialised)
and `deliver3` identically — `return=Join(base=buffer)` — so it already greenlights the delivery; it
is NOT the bottleneck. The decisive diff is the param/attr table:
- `deliver3` (clean): the buffer arg is a SEPARATE owned local `o` (promoted to the arg), with a
  DISTINCT `__retbuf` marker attr. Caller types `r:vector["__ref_1"]` → adopts the buffer + frees `cell`.
- mine: `__retbuf` is the arg AND the marker, CONFLATED (direct-into-buffer delivery). Caller types
  `r:vector` (empty) → frees nothing → `cell`/`inner` leak.
Getting `["??"]` to survive the `control.rs:5440` rebuild (pass-stable, via a var-property scan) is
necessary but NOT sufficient — the conflated PARAM STRUCTURE is the leak, and no return-dep value
fixes that. So `materialize_vector_arms_into`-into-`__retbuf` is a dead end for the borrowed-binding
case; only `deliver3`'s structure (separate owned local `o` promoted by the EXISTING promotion) makes
the caller adopt. **NEXT (the single remaining task): synthesise `o = []; o += <binding>; o` per
borrowed arm** and let the existing promotion build the `o`-arg + `__retbuf`-marker separation — the
one blocker is the fresh-vector `o = []` alloc helper (`OpDatabase`+`OpGetField` idiom; the
`Reference` analog is `materialize_return_into`). Captures + diffs:
[match_return-emit/](bytecode-comparisons/match_return-emit/README.md).

**LAYER PINNED (probe-driven, two earlier guesses corrected).** The interp crash is the
PARSER/IR-generation layer — the return/store **registration** structure — NOT interp execution and
NOT the tp:
- **Probe 1 (decisive):** `field_return` (`fn f(b: Box) -> vector { b.rows }`, the proven
  `copy_borrow_tail_into_retbuf` path) + churn runs CLEAN on interp. So interp executes
  "copy a borrowed vector into the retbuf" correctly — the bug is in MY IR, not the interpreter.
- **Probe 2 (`append_elem_tp` instrument):** ruled OUT the tp. Both programs compute
  `content(own-retbuf-vec-tp)` — mine `content(67)=65`, field_return `content(66)=64`, EACH correct
  for its own retbuf (the `66`/`67` gap is just per-program type registration). The `__vdb`/
  `OpReplaceVector` wrapper is also NOT needed — `copy_borrow_tail_into_retbuf` has neither and
  survives churn.
- **What's left:** `copy_borrow_tail_into_retbuf` delivers the borrow as the function TAIL and sets
  `returned = Vector(elm, Deps::attrs([buf_attr]))` + a typed `borrow_tail_copy` block — registering
  the retbuf as the OWNED return. My materialise appends inside a match ARM and strips deps but never
  establishes that registration, so under churn the store isn't tracked as the live return and a
  later allocation reuses it (the corruption).

**NEXT (corrected):** do NOT hand-roll the append. Route the borrowed match-arm delivery through
`copy_borrow_tail_into_retbuf` (or replicate its FULL output — clear + append + the `returned`/
block-deps registration), not just the bare `OpAppendVector`. The proven path is the spec; my
divergence from it (the missing registration) is the defect. The methodology lesson recurs: when a
proven sibling path exists, diverging from it IS the bug — match it, don't re-derive it.

Then flip default-on once all three sites are green on both backends + the full matrix + POISON. Then
fold the remaining own-vs-borrow re-derivations (the return-delivery thicket) onto the oracle as cleanup.

## Next step (per-site — SUPERSEDED by the unification above for sites 2/3)

1. **`local_source`** — ✅ DONE (commit `a639433d`, behind `LOFT_JOIN_OWN`). Root cause (nailed by an
   FRD runtime trace): `chosen = dflt()` move-adopts the fresh store into `chosen`; the caller's
   `__ref_2` retbuf keeps its null sentinel (it never owned the store), so the cleanup
   `FreeRefIfDistinct(__ref_2, chosen)` guards the wrong ref and the store orphans on reassign. Since
   `free` is store-level, a naive reassign-free is unsound (it would whole-store-free the pool). FIX:
   `use_analysis::displaced_owned_slots` flags `chosen`; `scopes.rs::scan_set` strips its `["pool"]`
   dep so the OWNED path deep-copies the borrow into its own store + frees it at scope exit (reusing
   the instances-1/2 `make_independent` pattern). Proven both backends; gate + fix tests in
   `tests/use_analysis.rs`.
2. **`elem_accumulate`** — DIAGNOSED (the loft-codegen "prove the working bytecode" step is done);
   implementation pending. **Interp-only** (native already correct). The divergence is the first-bind
   of a borrowed-view call return (`__lift_1 = pick(t,i)`, `pick -> M { t[i] ?? m_none() }`):
   - **Native** runtime-guards it (`generation/dispatch.rs::output_set` ~L358): `let _dst = old; let
     _src = pick(...); if _src.store_nr == MAX || _src.store_nr == _dst.store_nr { adopt _src } else {
     OpDatabase(_dst); OpCopyRecord(_src→fresh, NO source-free) }` — then the append source-frees the
     result. The owned arm (`m_none`) ADOPTS (the append's `0x8000` frees it correctly); the borrow
     arm (`t[i]`) MATERIALISES a copy (the append frees the copy; `t` intact).
   - **Interp** (`state/codegen.rs` `owned_ref` first-bind ~L1672): (a) gates the deep-copy on a
     type-shape proxy ("callee has a visible Reference/Enum param") that MISSES `pick`'s Vector-param
     borrow — study **instance 1**, fixed in native, NOT interp — AND (b) its deep-copy is an
     UNCONDITIONAL materialise (`OpDatabase` + `OpCopyRefOrNull`) with no adopt branch.
   - PROVEN (experiment, reverted): merely aligning the gate to `returns_borrowed_view()` fixes the
     borrow UAF (`CRASH→clean`) but LEAKS the owned arm (`clean→LEAK`) — the static `is_borrowed_view`
     suppresses the source-free uniformly (right for borrow, wrong for owned). A static gate cannot
     work — confirming the study's "the source-free is load-bearing for the owned branch". **The fix
     must mirror native's RUNTIME store-identity guard**, cleanest as a new interp op
     `OpBindOrCopy(src, pos, tp)` (adopt if `src.store_nr ∈ {MAX, old_v.store_nr}`, else fresh +
     deep-copy), emitted for a borrowed-view return in place of the unconditional deep-copy, gated
     `LOFT_JOIN_OWN`. Interp-only (native does it inline). Add via a `default/*.loft` `#rust` template
     + `make fill`; validate `462-elem-accumulate-{source-free,owned-branch-CLEAN}` + matrix + POISON
     on both backends.
3. **Then** `match_return` (interp `ParamDeliver` delivery) — surfaced by `free_sites`.
4. Flip default-on once all three + the full matrix are green on both backends and POISON-clean.
