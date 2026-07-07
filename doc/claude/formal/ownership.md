<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ownership.md — the `deps` ownership / borrow system (strict; register at `OPEN: 0`)

**Catalogue:** @F21 (references `&T`), @I60 (deps / lifetime tracker) — Goal E. Roadmap: @PLN85, @PLN87.

> **Rules then deviations** (see [README](README.md)). The rules below are loft's
> ownership model, and as of 2026-07-04 the deviation register is at **`OPEN: 0`** — all
> five `D-own-*` deviations are CLOSED on the **shipped path** (@PLN85 store-lifetime,
> @PLN87 the `&` law both landed), validated by the @PLN89 differential oracle + the
> `program_ownership` fuzzer. This is *validation, not a machine-checked proof*: the
> `Join` fact still resolves through a runtime witness, and the pre-fact shape-scans
> survive under opt-out as differential-control machinery. The residual is not a
> correctness deviation but the *substrate* — the fact is computed flow-INSENSITIVELY.
> [@PLN94](../plans/94-cfg-ownership-dataflow/) has now built the flow-SENSITIVE
> replacement as an independent oracle that runs BESIDE the shipped analysis (a machine
> check on every `cargo test`, `tests/ownership_oracle.rs`); its abstract-interpretation
> soundness is the path from *"validated"* to *"machine-checked"* — see
> §"Machine-checkable soundness" below for the proof skeleton (one open lemma).
>
> The rules are loft's borrow checker. **Rust is the reference model.** Beacon + rationale:
> [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md); the typed-`deps` design:
> [DEPS_INVENTORY.md](../DEPS_INVENTORY.md). This doc is the **checker** (lifetimes /
> free placement); the **surface** (`&τ`, reference-default) is [binding.md](binding.md).

## Notation

- **owner** — the binding or slot responsible for freeing a heap store. Exactly one at a
  time.
- **borrow / alias** — a value that *refers to* a store it does not own (a parameter, a
  field/element read, a `&τ` link). It must not free, and must not outlive its source.
- **`deps`** — the per-binding fact recording what it owns and what it borrows from. The
  one fact every store-lifetime decision reads. (Today a `Vec<u16>`; see D-own-3.)
- **transfer / move** — handing ownership to another binding (e.g. a return). The giver
  stops owning; it must not free what it moved.

---

## Rules

> The model is **sound** (no use-after-free, no double-free, no leak) **and complete**
> (computed for *every* binding, every path). The five invariants:

```
  (O-Owner)     SINGLE OWNER.  Every heap store has exactly one owner at any moment.
  (O-Move)      MOVE ON RETURN.  A returned heap value's ownership transfers to the
                caller's binding; the callee never frees what it transfers.  If the return
                *borrows* a parameter, the return type records it (`{Attr(param)}`) and the
                caller COPIES to obtain its own store.
  (O-Borrow)    BORROW TRACKING.  A value aliasing another (param / field / element / `&τ`)
                carries the source in its `deps`; the borrower is skip-free; the single
                owner frees once.
  (O-Derived)   FREE PLACEMENT IS DERIVED, NOT DECIDED.  Free a local iff it owns its store
                and does not transfer it out — once, at scope exit.  No per-site heuristic.
  (O-Complete)  PER BINDING, PER PATH, COMPLETE.  Every binding, including every `match`/`if`
                arm — a set-and-reconcile, not a single-variable structural walk.
```

**In words.** One thing owns each piece of heap, and it's the only thing that frees it.
When you return a heap value you *give it away* (the function stops owning it); if you
only return a view into an argument, the type says so and the caller makes its own copy.
Anything that just borrows is tracked but never frees. Crucially, *where* to free is
**computed** from these facts, not guessed per code-site — and it's computed for **every**
binding on **every** branch, not just the easy ones.

**This is an INTERNAL system — it never rejects a program.** loft has no user-facing borrow
checker; the user writes naively and the compiler always finds a valid lowering, copying when
it cannot prove an alias is safe ([OWNERSHIP_MODEL.md § Internal and invisible](../OWNERSHIP_MODEL.md)).
That makes **`O-Complete` the load-bearing invariant**: an incomplete fact is not a compile
error the user fixes — it is a miscompile or a leak. So the failure mode to fear here is
*incompleteness* (D-own-2), not just unsoundness — the analysis must be **total**.

### The mechanism — one fact, derived everywhere

```
  (O-Deps)      every store-lifetime codegen decision — free placement, adopt-vs-copy,
                move-vs-clone, drop — DERIVES MECHANICALLY from the single `deps` fact.
                If a decision is re-derived by a codegen condition, that is the bug.
  (O-NoDiverge) because both backends translate the SAME `deps` facts, the interpreter and
                `--native` cannot diverge.  (This is the soundness side of
                [operational.md](operational.md)'s shared contract: O-NoDiverge is *why*
                E-Op/E-Trap agree across backends.)
```

**In words.** `deps` is the single source of truth. Every "do I free / copy / move this
store?" question is *answered by reading `deps`*, never re-worked out in the code
generator. And because both backends read the same answer, they can't disagree — which is
exactly what makes the operational rules hold on native as well as interp.

---

## Deviations

OPEN: **0** (2026-07-04) — **the ownership register is at zero.**  All five D-own
deviations are resolved: D-own-3 (typed `Deps`) CLOSED; D-own-4 RECLASSIFIED as the
decided edge C86 (whole-value binds copy; aliasing is a last-use elision —
`classify_vec_bind`); D-own-5 (the `&` borrow rides `deps`) CLOSED; **D-own-2
(O-Complete) CLOSED** (the ownership fact is total — oracle covers every value, the free
side reads it, the inherently-runtime Join completed per-path by the `_own_store`
witness; validated by the 6-shape sweep + full gates + the `program_ownership` fuzzer);
and now **D-own-1 (O-Deps) CLOSED** — an audit of every store-lifetime DECISION site
(dispatch.rs / state/codegen.rs / ops/ref_ops.rs / scopes.rs / control.rs) found the
free/copy/adopt/drop decisions read the ONE canonical fact
(`ownership_of` / `returns_borrowed_view` / `return_adopts_fresh_store`) on the shipped
path — the last inline shape-scan (the interp adopt-vs-deep-copy visible-ref-param scan)
was unified onto `return_adopts_fresh_store()` matching the native sibling (commit
`0234cbbb`).  **The floor (honest):** the pre-fact scans survive ONLY under the
`LOFT_NO_JOIN_OWN` opt-out (differential-control machinery, not shipped behaviour); the
runtime Join witnesses (`_own_store`/`OpBindOrCopy`) are inherently-runtime (spec-accepted,
not a re-derivation); and collapsing the return-ownership readers into ONE physical funnel
is code-DRY, not a re-derivation (each already reads the fact).  Those are reclassified as
non-deviation cleanup — the O-Deps SUBSTANCE (no shipped decision re-derives ownership; the
fact is carried and read everywhere) is met.  Validated: full suite 2601/2601 (env flakes
only), `native_scripts`, `LOFT_POISON`, the `ownership_fuzz_gate` control pairs, the
differential oracle, and the fuzzer.

### D-own-1 — CLOSED (2026-07-04): ownership is carried as one `deps` fact, read (not re-derived) per-site
- **Violated:** O-Derived / O-Deps
- **Where:** the store-lifetime bug class — `has_ref_params`, the return-source set, the
  free-suppress / return-buffer logic, etc. ([OWNERSHIP_MODEL.md § Why](../OWNERSHIP_MODEL.md)).
  Each fix added a codegen condition rather than completing a fact.
- **Effect:** the recurring store-lifetime bugs (Cluster A, #426, #429, …) — "N forests,
  one root". The class cannot be closed by more conditions.
- **@PLN85 note (2026-07-04):** the store-lifetime BUG class is retired (@PLN85 closed) —
  the load-bearing re-derivations are ELIMINATED (return-delivery + reassign thicket
  collapsed behind `classify_X`/`dispatch_X`; the `ownership_of` oracle default-on, 0/54
  over-free; the free side reads `returns_borrowed_view()`) and no re-derivation produces
  a live bug (closed by construction: fuzz/poison/DA + leak-gate).
- **@PLN90 note (2026-07-04):** the LAST per-site ownership re-derivation is now GONE —
  `scan_set`'s owned-vs-view TRACKER (`ref_rhs_ownership`) no longer re-derives from the
  RHS shape; it reads the ONE canonical `ownership_of` oracle (Owned → track; Borrowed
  AND Join → View, since a borrow/join reassignment displaces the prior owned store and
  must not be tracked as owned).  So O-Derived is SATISFIED: every store-lifetime
  decision now reads the one canonical fact, not a per-site shape scan.  Validated: full
  suite + `native_scripts` + DA + `LOFT_POISON` + differential oracle green; the p462
  conditional `?? m_none()` transition and the C86 copy-return cases all clean both
  backends.  **The D-own-2 residual is now CLOSED too** (see below): the `_ => Owned`
  tail is correct (it covers only fresh-owned / scalar / payload-less values, not a
  hole), the value-vs-bind gap is INERT for the free decision (the reassign pre-free +
  type-based scope-exit free cover it), and the inherently-runtime Join is completed
  per-path by the `_own_store` witness — so the ownership fact is TOTAL.  O-Derived:
  **CLOSED** — the re-derivation is deleted.  What stays under D-own-1 is only the
  *single-fact* unification: the free/copy/move decisions read the canonical fact at
  their chokepoints, but three cooperating mechanisms (the static oracle read + the
  runtime Join witnesses + the return-buffer machinery) are not yet ONE `deps` read.
- **Status:** CLOSED (2026-07-04) — the audit + `0234cbbb` unification landed the last
  shipped shape-scan onto the fact (see the header for the close + the honest floor).
  History below.  Landed: the return-delivery
  collapse is COMPLETE — `block_result` 459→328 lines, **45→21 helper calls**, the 15
  tail-shape classifiers down to ~3 genuinely-distinct entry guards; EVERY delivery
  mechanism routes through a pure `classify_X` selector + `dispatch_X` (vector
  `Delivery`, Reference `RefDelivery`, text `TextDep`, `ref_return`'s
  `classify_ret_promotion`); the #416/#448 cells folded; class swept dry over ~41
  probes.  The `ownership_of` oracle chokepoints are **DEFAULT-ON**
  (`keys.rs::join_own_enabled`; 54-cell over-free map 0/54 default).  And the FREE
  side began reading the canonical fact: `scan_set`'s #316 ownership tracker
  (`ref_rhs_ownership`) and codegen's owned-ref reassign gate now call
  `returns_borrowed_view()` instead of re-scanning the return deps inline (2026-07-04,
  both byte-identical over the 8 D-own-1/C86/462 corpora).
  **AUDIT 2026-07-04 — the consumption side is now ~fully fact-reading.** A sweep of
  every store-lifetime DECISION site (dispatch.rs, state/codegen.rs, ops/ref_ops.rs,
  scopes.rs, control.rs) found the free/copy/adopt/drop decisions read the canonical
  fact (`ownership_of` / `returns_borrowed_view` / `return_adopts_fresh_store`)
  everywhere but ONE genuine residual, plus two non-violations:
  - **THE ONE RESIDUAL — `state/codegen.rs:1786-1789`**: the interp `v = call()`
    deep-copy path still gates on an inline *visible-ref-param scan* to decide
    adopt-vs-deep-copy, while the NATIVE sibling (`dispatch.rs:405`) already reads
    `return_adopts_fresh_store()`.  For a fresh-return-with-ref-param callee
    (`fn mk_from(seed) -> Box { Box{..} }`) interp deep-copies where native adopts —
    same value + leak-clean on both, but a mechanism divergence.  Unifying it onto
    the fact is a COPY-ELIMINATION small-step (adopt instead of deep-copy), not
    byte-identical — best done as a dedicated @PLN90 slice on this most-reverted
    path, with the corpus+matrix gate, NOT rushed.
  - NOT violations: `dispatch.rs:403-404` (`.starts_with("n_")` / `code()!=Null` are
    call-KIND eligibility filters, the ownership decision reads the fact at 405);
    `scopes.rs collect_return_sources` (the return-source SET is the row-268 fact
    PRODUCER for the match/if union, not a consumption re-derivation).
  REMAINING: (1) the single copy-elim unification above + the architectural funnel of
  the 3 return paths (row 273) into one return-ownership computation — mechanical, no
  live bug; (2) the `??`-JOIN
  runtime witness (`OpBindOrCopy`/`OpFreeRefIfDistinct`/`_own_store`) is inherently
  runtime (the
  arm taken is unknown at compile time), not a re-derivation to delete.  D-own-5's
  `&`-borrow fact is CLOSED (folded).
- **Removal — DONE:** every free/copy/move reads `deps` (via `ownership_of` /
  `returns_borrowed_view` / `return_adopts_fresh_store`) on the shipped path; the
  per-site heuristics survive only under the `LOFT_NO_JOIN_OWN` opt-out (control
  machinery).  Non-deviation cleanup left: DELETE the opt-out scans once the differential
  controls retire, and collapse the return-ownership readers into one physical funnel
  (pure DRY — each already reads the fact).

### D-own-2 — CLOSED (2026-07-04, @PLN90): the ownership fact is TOTAL
- **Violated:** O-Complete
- **Where:** the row-100/102 holes — adopt-vs-copy for arbitrary borrowing returns; the
  general dep-driven caller copy. (The struct-field and value-`if`-return facets closed
  earlier — #415, a7.)
- **What CLOSES it — the analysis is now total, and validated total.**  O-Complete's
  failure mode is *incompleteness → a silent miscompile or leak* (line 64-66): a
  binding/path with NO computed ownership fact, falling back to a heuristic/stopgap.  That
  is now eliminated on three fronts:
  1. **The static fact is total and correct.**  `ownership_of` (use_analysis.rs) computes
     an `Own` for EVERY `Value`: `OpDatabase`/`OpNewRecord`/literals/scalars → `Owned`;
     a projection → `Borrowed{base}`; a user call → the interprocedural `call_ownership`;
     `??`/`if` → the `join` of its arms; block/insert → its tail.  The `_ => Owned` tail
     is not a hole — it covers only literals / scalar-void ops / payload-less control,
     which ARE fresh-owned or heap-irrelevant (verified against the classifier).
  2. **The free side READS that one fact** (the D-own-1 fold): `scan_set`'s #316 tracker
     (`ref_rhs_ownership`) is a pure `ownership_of` read — `Owned → Owned`, `Borrowed`/
     `Join → View`.  The three-valued gap is closed: `RefRhs::Unknown` is DELETED (dead
     once the oracle covers every value), so the free side is a total 2-valued read of
     the oracle, not a separate structural walk.
  3. **The inherently-runtime JOIN is completed per-path at runtime.**  Where a binding's
     ownership genuinely differs per path (`r = x; for { r = v[i] ?? x }` — owned copy on
     the empty path, a borrowed view once the ncc runs), a static per-binding fact CANNOT
     decide (the spec accepts this as inherently runtime, see D-own-1 residual (2)).  The
     `_own_store_<name>` witness (generation/, @PLN90 loft#495 / commits 44fd7d72 +
     a4bcad5b) is exactly the "set-and-reconcile across arms" O-Complete's removal
     criterion asks for — done at runtime: it tracks the store r actually owns, so BOTH
     the displaced-free and the scope-exit free release the owned store and never the
     view.  This is the last binding-shape whose free decision was previously incomplete.
- **The residuals — all COMPUTED and SAFE, not holes** (probed both backends,
  [plans/85 D-own-2-completeness.md § Sweep](../plans/85-store-lifetime-retirement/D-own-2-completeness.md)):
  (i) the **value-vs-bind gap** (`ownership_of(x)=Borrowed` for a `r = x` whole-value
  COPY that owns) is INERT for the free decision — the reassign pre-free + type-based
  scope-exit free release the displaced/final store regardless of the tracker's read;
  and for the transition class the witness's `is_var_copy` reads the bind as owned.
  (ii) the **deps-carried-join** (`r = pick(v,i)`, `pick = v[i] ?? Box{..}`) is a
  COMPUTED `Own::Join`, classified conservatively as a view — correct: the OWNED arm is
  materialised into the return buffer whose own lifetime frees it, so `r` views it (no
  leak / no double-free, both arms exercised).
- **Validated total:** the transition class swept dry over 6 shapes (2 live over-frees
  found + fixed, 4 safe), the value-vs-bind + deps-join residuals probed clean+poison,
  the full suite 2600/2600 (env flakes only), `native_scripts`, `LOFT_POISON`, native
  leak-check, DA, the differential oracle, AND the `program_ownership` fuzzer (3108 execs,
  0 findings — the "unfuzzed axis" concern discharged).  No binding/path produces a live
  miscompile; the analysis is total.
- **Not this deviation:** unifying the runtime witness + return-buffer machinery INTO the
  single `deps` read (rather than three cooperating mechanisms) is the *single-fact*
  ideal — that rides **D-own-1 (O-Deps)**, which stays open.  And the adopt-vs-view
  *optimisation* for a Join return (view is correct; adopt would save a copy) is
  copy-elimination — **@PLN90's LINT charter**, not an O-Complete correctness item.

### D-own-3 — CLOSED (2026-06-12, recounted into the register 2026-07-03): typed `Deps`
The dep list was a raw `Vec<u16>` overloading five meanings across two address spaces.
The H2 migration ([DEPS_INVENTORY.md](../DEPS_INVENTORY.md), steps 1–5) landed the
`Deps` newtype with named constructors at every creation site, space-checked queries
(`frame_vars` / `as_attr_indices`, debug space tags), and the `CALLEE_FRAME_BIT` VALUE
tag (0x8000) so the one cross-space provenance (the vectors.rs lambda propagation)
survives the IR codec unambiguously.  Residual (not a deviation): the newtype `Deref`s
to `Vec<u16>` for read convenience — writes go through the typed constructors.

### D-own-4 — RECLASSIFIED (2026-07-03, C86): the #415 copy IS the semantic; derive it, don't reverse it
The entry claimed the #415 struct-vector-field copy-on-bind was a stopgap contradicting
reference-default.  The reversal attempt found the premise false: on BOTH backends every
WHOLE-VALUE heap bind copies (`p = o`, `b = x`, `af = bx.v`) and only projections alias —
the written law, not the code, was wrong.  The maker's call
([DESIGN_DECISIONS C86](../DESIGN_DECISIONS.md#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule)):
whole-value binds COPY by contract; `p = o` becomes an alias only when the source is
provably dead afterwards — the rustc last-use rule, as an OPTIMIZATION
(`use_analysis::ElidePlan` is that analysis).  `O-Borrow` scopes to projections /
params / `&τ`.  (binding.md D-bind-3 was already closed — the old "blocks" claim was
stale.)  The implementable RESIDUAL — the copy/alias/elide decision at the bind site
derives from the ownership fact instead of the syntactic `struct_vec_field` branch —
folds into **D-own-1**.  **Narrowed 2026-07-03:** the decision is now the pure
`classify_vec_bind` selector (`VecBind`, parser/expressions.rs — byte-identical
extraction over the C86 bind corpus): the verdict reads the base var's
incrementally-maintained `deps` (the same fact `ownership_of` reconstructs post-parse
via its whole-body `Defs` walk — Owned ⇒ copy, Borrowed/Join ⇒ view; agreement
witnessed by `LOFT_MATERIALIZE_DUMP` over the corpus), and the ELIDE half is already
live post-parse (`elision_plans` → `scopes::elide_borrows`).  What remains of D-own-1
here: the mid-parse deps read and the post-parse oracle are two implementations of one
fact — they unify when ownership is carried as one typed `deps` fact end-to-end.

### D-own-5 — CLOSED (2026-07-03, folded): the `&` borrow now carries its source in `deps`
- **Was:** @PLN87's ladder L1–L6 realised live references ([binding.md](binding.md),
  verified), but the `&τ` borrow's source was carried by a side-flag (`skip_free` on the
  L5 heap whole-value alias), not the `deps` fact the checker reads.
- **The fold (executed):** the L5 bind (`p = &o`, the only `&` binder with a free
  decision) now types `p: &Reference(td, [o])` via the standard `depending()` carrier —
  free suppression derives from `owns = dep.is_empty()` (`scopes::get_free_vars`), the
  same O-Borrow read every other borrow uses; the `set_skip_free` side-channel at the
  bind is deleted.  Proof: the ladder introspects change ONLY in the type display
  (`&ref(Pair)` → `&ref(Pair)["whole"]`) — zero op changes, both backends green,
  leak-gated (434-pln87-scalar-reference, 28-references, 87-store-leaks).
- **Residual sliver (recorded under [D-own-1](#d-own-1)):** a scalar-place ref
  (`c = &v[0]`, `r = &s.x`) holds a DbRef into the source's store, but a scalar inner
  carries no `Deps` slot (`depending()` is the identity), so the link is not a readable
  fact — vacuous for FREE placement (the binder owns no store) but unavailable to any
  future lifetime check until `Deps` is carried type-wide (the D-own-1/D-own-2
  completion).

---

## Machine-checkable soundness — the @PLN94 flow-sensitive oracle (proof skeleton)

The register above is **validation, not a machine-checked proof**: the shipped fact is computed
flow-INSENSITIVELY (the join of all defs) and the `Join` case discharges through a runtime witness.
[@PLN94](../plans/94-cfg-ownership-dataflow/) builds the flow-SENSITIVE replacement — a monotone
dataflow fixpoint (`src/ownership_cfg.rs`) run BESIDE the shipped analysis as an independent oracle,
never driving codegen (SI-1). Being a textbook abstract interpretation, that oracle is the piece that
CAN carry a machine-checked proof. This section states the obligations, marks the discharged ones,
and frames the one real remaining lemma. It is a **skeleton** — the scaffolding for the proof, not
the proof.

**What is proved, and what is not.** The target is the **over-free** class only — no free of a store
the fact does not own (⇒ no use-after-free, no double-free of that store). It is **NOT** a no-leak
proof (under-free is a disjoint class the shipped leak-check owns — @PLN94's coexistence finding:
`LOFT_NO_JOIN_OWN` leaks past this oracle but not past the leak detector). And it proves the
**oracle**, not the codegen: the shipped path inherits the certificate only where the two agree,
which is why they run beside forever.

**(1) The abstract domain — DISCHARGED.** `OFact = ⊥ | Owned | Borrowed(b) | Join(b)` with meet `⊔`
is a join-semilattice (finite height ≤ 3), and `refines` is its partial order. *Proof:*
`ofact_meet_is_a_join_semilattice` + `ofact_refines_marks_precision_and_flags_the_unsound_direction`
(unit tests, `src/ownership_cfg.rs`).

**(2) The concrete property.** In [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩` with the
[heap.md](heap.md) store `H`, define at each program point the relation *owns(v)* = the var whose
binding is responsible for freeing `v`'s store (per **O-Owner**: exactly one). A free of `v` is
**sound** iff `owns(v) = v` at that point (**O-Derived**). Over-free = a sound-fact says `Owned`
where concretely `owns(v) ≠ v`.

**(3) The Galois connection.** `γ(Owned) =` { states where `owns(v)=v` }; `γ(Borrowed(b)) =` { states
where `v` aliases `b`'s store, `owns(v)=owns(b)≠v` } (**O-Borrow**); `γ(Join(b)) = γ(Owned) ∪
γ(Borrowed(b))` (runtime-dependent); `γ(⊥) = ∅`. `α` is the pointwise best abstraction. Obligation:
`γ` is monotone w.r.t. `refines` and `⊔` is its sound join — *straightforward from (1); to write.*

**(4) Local soundness of the transfer — THE ONE REAL OBLIGATION (open).** For each statement `s` and
its abstract transfer `T_s` (`src/ownership_cfg.rs::ownership_dataflow`), prove
`⟨s, σ⟩ → ⟨_, σ'⟩ ∧ σ ∈ γ(st) ⟹ σ' ∈ γ(T_s(st))` — each step over-approximates the concrete
store-ownership step. This is the substantive lemma, one case per RHS shape: `OpDatabase`/`OpNewRecord`
→ `Owned` (a fresh store, sound by heap.md `alloc`); a projection `OpGet*` → `Borrowed(base)` (a view,
**O-Borrow**); a call → the callee's `return_ownership` summary mapped to the caller's arg
(**O-Move**); the arm-meet `⊔` (**O-Complete**). The self-borrow `Borrowed(v)→Owned` and the `= null`
skip must each be shown to preserve the invariant (they mirror `get_free_vars`' @P302 and
null-declaration carve-outs).

**(5) Fixpoint soundness — from (1)+(4).** The round-robin least fixpoint over the CFG converges (**≤
n+2** passes, asserted SI-3) and, by (4) + monotonicity + Tarski, the per-block OUT-state soundly
over-approximates the concrete ownership at every reachable point. *Bound + convergence discharged;
the soundness step waits on (4).*

**(6) The check corollary.** With (5): if the oracle's check is **GREEN** — Check B finds no
unconditional `OpFreeRef(v)` with `st(v) = Borrowed`, and Check A finds no fact the shipped analysis
disagrees with in the unsound direction — then the emitted plan performs **no over-free** of the
covered classes. Contrapositive is the A1b catch: the `LOFT_NO_A1B` plan returns a store the fact
reads `Join`/`Borrowed` while the shipped fact reads `Owned` → RED (verified end-to-end,
`tests/ownership_oracle.rs`), a wrong plan every runtime gate passes.

**(7) Coexistence conclusion.** The proven oracle is a machine-checked *certifier*: on every program
where oracle and shipped analysis agree (empirically: 505-corpus + 54-cell fuzzer + 377 scripts, all
0 RED), the program carries a proof-backed over-free-freedom certificate; a residual disagreement
indicts one side for adjudication. This upgrades the register's *"validated"* to *"the flow-sensitive
fact is machine-checked sound (pending lemma (4)); the shipped analysis is certified per-program by
the proven oracle running beside it."*

**Obligation ledger.** DISCHARGED: (1) lattice, (5) fixpoint bound/convergence (SI-3), backend
fact-identity (SI-2, `tests/ownership_oracle.rs`), no-crying-wolf at corpus + fuzz scale (empirical).
OPEN: (3) `γ` monotonicity (small), **(4) local transfer soundness (the real work)**, then (5)'s
soundness step and (6) fall out. OUT OF SCOPE: no-leak (under-free — the leak detector's class); the
`Join` runtime-witness discharge (a separate `OpFreeRefIfDistinct` lemma); proving the shipped
8-mechanism analysis directly (the certifier sidesteps it).

## Conformance

This area's "falsifying programs" are the store-lifetime bugs themselves — each is a
program where the derived-free invariant (O-Derived) or completeness (O-Complete) fails
and a store leaks, double-frees, or a backend diverges. The area is **formal when OPEN
reaches 0**: when every store-lifetime decision is one `deps` read (O-Deps) over a complete,
typed fact, the bug class is closed by construction and `binding.md`/`types.md`'s
`deps`-fused rough spots (the `Deps`-in-`Type` fusion) resolve with it.
