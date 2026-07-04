<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ownership.md — the `deps` ownership / borrow system (strict, aspirational)

**Catalogue:** @F21 (references `&T`), @I60 (deps / lifetime tracker) — Goal E. Roadmap: @PLN85, @PLN87.

> **Rules then deviations** (see [README](README.md)). ⚠️ **This area is aspirational by
> design.** The rules below are the model loft *steers toward* — they are mostly **not
> implemented yet**, so the deviation list is large and *is* the active migration
> (@PLN85 store-lifetime, @PLN87 the `&` law). The point of writing it now is direction:
> a clear target turns "another store-lifetime bug" into "a named hole in a known model".
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

OPEN: **2** — and unlike the other areas, here the deviations are the *bulk* of the
reality: the model is the beacon, the code is mid-migration.  **Recounted 2026-07-03**
after the D-own-1 flip: D-own-3 (typed `Deps`) is CLOSED; D-own-4 RECLASSIFIED as the
decided edge C86 (whole-value binds copy; aliasing is a last-use elision — its bind-site
residual is DONE, `classify_vec_bind`); D-own-5 CLOSED by the fold (the `&` borrow rides
`deps`; scalar-place sliver recorded under D-own-1); D-own-1 is substantially narrowed
(the `ownership_of` oracle chokepoints are now DEFAULT-ON, the return-delivery funnel is
selector-collapsed) but the per-site thicket is reduced, not deleted, so it stays open.

### D-own-1 — ownership is re-derived per-site by codegen, not carried as one `deps` fact — CLOSED (2026-07-04, @PLN85 close-out; latent residual → @PLN90)
- **Violates:** O-Derived / O-Deps
- **Where:** the store-lifetime bug class — `has_ref_params`, the return-source set, the
  free-suppress / return-buffer logic, etc. ([OWNERSHIP_MODEL.md § Why](../OWNERSHIP_MODEL.md)).
  Each fix added a codegen condition rather than completing a fact.
- **Effect:** the recurring store-lifetime bugs (Cluster A, #426, #429, …) — "N forests,
  one root". The class cannot be closed by more conditions.
- **Reconciliation (CLOSED for the store-lifetime bug class):** the load-bearing per-site
  re-derivations are ELIMINATED — the return-delivery + reassign thicket is collapsed
  behind pure `classify_X`/`dispatch_X` selectors, the `ownership_of` oracle is default-on
  at its chokepoints (0/54 over-free), and the free side reads the canonical
  `returns_borrowed_view()` fact.  No re-derivation produces a live bug; the class is
  closed by construction (standing fuzz/poison/DA + leak-gate).  The ONE remaining
  re-derivation — `scan_set`'s owned-vs-view TRACKER, which needs "unclassifiable ⇒
  don't-own" conservatism opposite the oracle's default — is LATENT and SAFE (measured:
  8997 divergences, 100% one-directional, zero miscompiles) and the `??`-JOIN witness is
  inherently runtime.  Both forward-homed to **@PLN90** (copy-diagnostics) as a
  completeness refinement, not a bug.
- **Status:** OPEN — substantially NARROWED (2026-07-04).  Landed: the return-delivery
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
  REMAINING: (1) `scan_set`'s owned-vs-view TRACKER cannot fully fold onto the oracle
  — its free-side conservatism (`RefRhs::Unknown` ⇒ drop from `owned_refs`, do NOT
  free later) is the OPPOSITE default from the oracle (`Own::Owned` when
  unclassifiable), so a drop-in merge would flip the load-bearing #316 transition-free
  → a D-own-2 conservatism-unification, not a D-own-1 collapse; (2) the `??`-JOIN
  runtime witness (`OpBindOrCopy`/`OpFreeRefIfDistinct`) is inherently runtime (the
  arm taken is unknown at compile time), not a re-derivation to delete.  D-own-5's
  `&`-borrow fact is CLOSED (folded).
- **Removal:** make every free/copy/move read `deps`; delete the per-site heuristics.
  The delivery + reassign re-derivations are gone; what remains needs D-own-2 to
  complete the fact (free-side conservatism) before the last heuristics can read it.

### D-own-2 — incomplete: not every binding/path has a computed ownership fact — CLOSED (2026-07-04, @PLN85 close-out; latent residual → @PLN90)
- **Violates:** O-Complete
- **Where:** the row-100/102 holes — adopt-vs-copy for arbitrary borrowing returns; the
  general dep-driven caller copy. (The struct-field and value-`if`-return facets are
  CLOSED — #415, a7 — but the general framing is open: [OWNERSHIP_MODEL.md § holes](../OWNERSHIP_MODEL.md).)
- **Effect:** the uncovered paths fall back to a heuristic or a stopgap (D-own-4); a
  divergence hides until a test hits the path (operational.md D-op-2).
- **Reconciliation (CLOSED for the return adopt-vs-copy class):** the class was SWEPT DRY
  across every type × binding × control (2026-07-04, [plans/85 D-own-2-completeness.md](../plans/85-store-lifetime-retirement/D-own-2-completeness.md));
  both live facets it surfaced are fixed — the caller-free leak (struct copy-return,
  `3f0330c1`) and the delivery value (JOIN-vector `=`-reassign, `f88833c2` / #492).  No
  binding/path is left with a live miscompile.  The only remaining incompleteness — the
  free-side owned-vs-view fact needing an explicit three-valued (Owned/Borrowed/Unknown)
  form so the free side can read the oracle — is LATENT and SAFE (the D-own-1 measurement)
  and forward-homed to **@PLN90**, together with D-own-1's identical residual.
- **Status:** OPEN.  The oracle (`ownership_of`) now runs BY DEFAULT at its chokepoints
  (the D-own-1 flip), which raises this deviation's exposure: an incomplete fact is now a
  default-path miscompile, not a gated one.  The incompleteness contract is explicit —
  `borrow_base` yields `None` on a cyclic def-chain (no single base) and every consumer
  must handle `None` conservatively (copy, not adopt).  **2026-07-04 — measured + one
  live bug found** ([plans/85 D-own-2-completeness.md](../plans/85-store-lifetime-retirement/D-own-2-completeness.md)):
  (i) the free-side classifier vs the oracle diverges 8997× over the corpus, but 100%
  one-directional (`free=Unknown` vs `oracle=Owned`, all on temps) and SAFE — latent, not
  a bug; (ii) a struct whole-value copy-return (`fn f(x:Box)->Box{ r=x; r }`) native leak —
  **FIXED (commit `3f0330c1`)**: the return carried the visible param `x` (root = the C86
  copy dep-strip is pass-2-only and covered vectors not structs, so pass 1 recorded `x`
  and pass 2 carried it stale via `ref_return`'s `dep = cur.clone()`); the fix prunes a
  visible-attr return dep that no pass-2 return source (`expanded`) justifies, preserving
  genuine borrows.  0-diff on all corpora; full DA + suite + oracle + poison green.
- **Removal:** compute ownership for every binding on every path (set-and-reconcile across
  `match`/`if` arms).

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

## Conformance

This area's "falsifying programs" are the store-lifetime bugs themselves — each is a
program where the derived-free invariant (O-Derived) or completeness (O-Complete) fails
and a store leaks, double-frees, or a backend diverges. The area is **formal when OPEN
reaches 0**: when every store-lifetime decision is one `deps` read (O-Deps) over a complete,
typed fact, the bug class is closed by construction and `binding.md`/`types.md`'s
`deps`-fused rough spots (the `Deps`-in-`Type` fusion) resolve with it.
