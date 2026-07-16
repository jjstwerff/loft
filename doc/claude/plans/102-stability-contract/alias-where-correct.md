<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Alias where an alias is correct — a safe refinement of C86 (copy-by-default)

> **Status: design (2026-07-16).** Keeps [C86](../../DESIGN_DECISIONS.md) (whole-value heap binds
> COPY; `&` is explicit write-through) as the *contract*, and adds the compiler intelligence to
> **alias instead of copy in the cases where an alias is correct** — safely, so the memory-model bug
> that drove the copy (#415's aliasing UAF) never returns. This is exactly C86's own "revisit when:
> widen `ElidePlan`". Written before the code (design-protocol § "write the doc before the code").

## The request, made precise

*"Make everything the same, but still make aliases where that is correct."* This has a real tension
that the design must resolve honestly, not paper over:

- **C86's contract is COPY.** `th = t.tr_h; th[i] = v` copies the field into `th`, so the write lands
  in a throwaway copy (the hex_terrain break). Write-through needs `th = &t.tr_h`.
- **Recovering that alias is OBSERVABLE.** After the bind, `t.tr_h` is read again and differs under
  copy (unchanged) vs alias (written through). So *"alias the hex_terrain bind"* and *"everything
  observably the same"* cannot both hold unconditionally.

The resolution is to split "an alias is correct" into two disjoint cases with different guarantees,
both gated on the same **safety** precondition:

1. **The alias is UNOBSERVABLE** — copy and alias produce identical observable behaviour. Aliasing
   here is a pure optimization; *everything stays the same*. (Tier 1.)
2. **The copy is a PROVABLE DEAD STORE** — the local is mutated then discarded unread, so the copy
   guarantees a lost write and has no correct meaning; the write-through is the only intent the code
   can have. (Tier 2 — the hex_terrain case.)

Tier 1 is "everything the same" in full. Tier 2 changes observable behaviour **only for programs the
copy already made a guaranteed no-op** — every *correct* program is unchanged, but it is a deliberate
C86 refinement, so it carries an owner decision (below).

## The one invariant (design-protocol step 1)

> **A whole-value bind aliases (write-through) instead of copying iff the alias is SAFE — the source
> store provably outlives every use of the local (no use-after-free) — AND copying is not the intended
> semantics: either the copy is unobservable (free optimization) or the copy is a provable dead store
> (the write-through is the only meaning the code can have). In every other case it copies (C86).**

The invariant is **sound by conservatism**, inheriting `use_analysis`'s existing discipline (*"can
only lose an elision, never produce a wrong borrow"*): if safety cannot be *proven*, it copies. So the
refinement can never reintroduce #415's aliasing UAF — the worst case is a missed alias (a copy that
was safe to elide), never an unsafe one. **This is what makes it landable near loft's #1 weakness.**

## The safety gate — the shared precondition (why #415 cannot return)

#415 switched field-read binds to copy because the alias was a **use-after-free**: `a = x.v` aliased
the field's store *without owning it*, so a later free of `x` dangled `a`. So an alias is *correct*
only when it is *safe*, and safety is precisely the store-lifetime fact loft already computes:

- **Provenance:** `use_analysis::Ownership::Borrowed { base }` already carries, for a bound value, the
  caller-visible source var whose store backs it. That is the alias target.
- **Lifetime:** the alias is safe iff `base`'s store outlives the local's last use — the local does
  **not escape** past `base` (the dead-store lint's existing *non-escaping* check, `warn_dead_stores`
  S4a) and `base` is not freed while the local is live (the `deps` / `reclaim_safe` fact).

Only inside this safe set does the design ever prefer an alias. Outside it, C86's copy stands
unchanged. The gate is one fact read at one place (below), not a per-site re-derivation.

## Tier 1 — unobservable aliasing (contract-clean, always-on)

Alias when copy vs alias is **indistinguishable** AND safe. Sufficient sound conditions:

- **Source dead after the bind** — the current `ElidePlan` last-use elision (already shipped). A
  mutation through the local reaches no observed read of the source.
- **Both source and local read-only after the bind** — with no mutation on either side, shared vs
  independent storage is unobservable. This is the **widening** (most binds are read-only), the
  concrete cash-out of C86's "widen `ElidePlan`".

Tier 1 is a pure optimization: **the observable result is byte-identical to copy-everywhere**, so it
is "everything the same" with no contract question. It does **not** touch hex_terrain — there the
source is read again after a mutation through the local, so the alias is observable and Tier 1
correctly declines it (copy stays).

## Tier 2 — write-through recovery (the hex_terrain case, a C86 refinement)

Alias when the copy is a **provable dead store** AND safe. The trigger already exists: the @PLN107
classifier `use_analysis::dead_store_accesses` returns per-local `(reads, write_targets)`, and
`warn_dead_stores` already isolates exactly the target case — an **`Owned`, non-escaping** local with
`reads == 0 && write_targets > 0` (mutated, never read back). That is `th = t.tr_h; th[i] = v` with
`th` discarded: the copy guarantees a lost write, so the only intent the code can carry is
write-through. Tier 2 turns that **detected dead store into an alias** (the write reaches the field)
instead of only warning about it.

**This is observable** — hex_terrain goes from "0 land cells" (lost write) to correct heights — but
only for programs where the copy was already a guaranteed no-op. No *correct* program is affected: if
the local is read after the mutation, it is not a dead store and Tier 2 does not fire (copy stays,
value semantics preserved).

### The fork Tier 2 forces (owner decision — do not pick silently)

Tier 2 has a genuine downside that Tier 1 does not, and it collides with C86's stated rationale
(*"variables are their own thing; no spooky action; you don't have to remember how they're
constructed"* — C86 § Rationale). The bind's semantics become **non-local** (they depend on whether
the local is read later), and a dead store that was really a *different* bug (a **missing read-back**)
would be silently converted into a source mutation instead of being surfaced. Three ways to spend
that, in increasing alignment with "it just works":

| Option | Behaviour on the dead-store case | Trade-off |
|---|---|---|
| **A — copy + steer (status quo+)** | copy (write lost); `warn_dead_stores` nudges "use `&`" | contract-clean, fully local, but hex_terrain stays wrong until the author edits |
| **B — alias + steer (recommended)** | **alias (write-through) AND** emit the arc-C recommended-idiom steer *"loft treated this as write-through; write `&t.tr_h` to make it explicit"* | recovers the intent (correct output) AND announces it → the spookiness is *removed by making it non-silent*; ties to [arc C](recommended-idiom-channel.md) |
| **C — alias, silent** | alias (write-through), no diagnostic | maximal "just works", but fully spooky + can silently mutate the source when the real bug was a missing read |

**Recommendation: B.** It is the sweet spot the request reaches for — *"still make aliases where
correct"* — while paying off C86's anti-spookiness constraint through **announcement** rather than
suppression: the alias happens (correct result), and the steer makes the write-through explicit and
teaches `&`. It reuses arc C's channel exactly (a Goal-F warning on owned source; never a break). A is
the safe fallback if the owner prefers strict locality; C is rejected (silent source-mutation is the
one genuinely dangerous cell).

## Re-assertion count (design-protocol step 2) — N = 1

The alias-vs-copy choice is made in **one place** — `use_analysis`'s verdict feeding
`scopes::elide_borrows` (the `ElidePlan`). Both tiers extend that single computation; codegen keeps
*reading* the plan, never re-deriving it. The safety gate is one fact (`Ownership::Borrowed { base }`
+ non-escape), the dead-store trigger is one fact (`dead_store_accesses`). There is no per-bind-site
spray: one analysis, one plan, one codegen consumer — exactly the shape C86 already ships, widened.

## Falsification — how it breaks (design-protocol steps 3–4)

- **Claim: "the safety gate makes an unsafe alias impossible."** The gate is sound-by-conservatism
  (proven-safe-or-copy). Falsification target: a boundary matrix over the store-lifetime axes that
  drove #415 — a bound field mutated after its `base` struct is freed / reassigned / escapes via a
  return; each must **copy** (no alias), verified on both backends under `LOFT_POISON` +
  `LOFT_NATIVE_LEAK_CHECK`. A single unsafe alias here is the #415 regression; this matrix is the
  gate.
- **Claim (Tier 1): "the alias is unobservable."** Falsify with the distinguishing observations: (a)
  mutate the source then read the local; (b) mutate the local then read the source. Any program that
  can make either observation must **not** be Tier-1-aliased (it falls to Tier 2 or copy). Positive
  control: a read-only bind aliases and is byte-identical; a source-then-mutate-then-local-read bind
  copies.
- **Claim (Tier 2): "a dead store has no correct copy meaning."** The near-miss (over-unification
  guard, step 4): a local that *looks* discarded but escapes via a projection/return/closure is **not**
  a dead store — `warn_dead_stores`'s non-escape + `Owned`-only filters already exclude those (a
  `Borrowed` view's write already propagates, so it is not even a copy). Falsification target: an
  escaping-then-mutated local must NOT alias-via-Tier-2 (it is not a dead store); a genuinely
  discarded one does. The @PLN107 S4a exclusions are the reused guard — do not re-implement them.
- **Claim: "this preserves the contract."** True for Tier 1 (unobservable) and for every correct
  program under Tier 2. **False** for a program that is a provable dead store *and* relies on the
  source staying unmutated — but such a program relies on a guaranteed lost write (a no-op's
  non-effect), which is not a *functioning* behaviour. Still, under the absolute-compat promise this
  is an observable change to *some* extant program text, so **if landed post-freeze it is
  contract-keyed** (C4): copy under the old contract, alias-and-steer under the new. Pre-freeze it is
  a straight refinement. This is the compat lever the owner-decision above must set.

## The safe small steps

Inert-first, each verifiable before the next. Tier 1 is a pure optimization (no contract question);
Tier 2 lands gated so its observable change is measured before any default.

| # | Step | What lands | Verify | E |
|---|---|---|---|---|
| 1 | **Safety oracle, exposed + matrixed.** Surface `alias_is_safe(local)` = `Ownership::Borrowed { base }` present AND local non-escaping AND `base` outlives the local (reuse `dead_store_accesses` non-escape + `reclaim_safe`). No codegen change — it only *reports*. | the #415 store-lifetime matrix: every "base freed/reassigned/escapes while local live" cell → `unsafe` (would copy); a plain field-read-then-read → `safe`. Both backends. Positive control: an injected unsafe shape reads `unsafe`. | M |
| 2 | **Tier 1 — widen `ElidePlan` to the read-only-both case (GATED, default off).** In the elision verdict, additionally alias a bind when `alias_is_safe` AND both source and local are read-only after the bind. Behind `LOFT_ALIAS_WIDEN` (opt-in). | byte-identical corpus (loft-codegen Mode B) with the flag OFF; with the flag ON, the read-only binds emit a borrow not a copy (introspect diff) and the **observable result is unchanged** on the full suite both backends. `--report-copies` shows the copy-count drop. | M |
| 3 | **Tier 1 default-on.** Flip `LOFT_ALIAS_WIDEN` default-on (opt-out) after the suite measures observably-identical. This is the "everything the same" deliverable — more aliases, zero behaviour change. | full suite + `native_scripts` byte-value-identical to pre-flip; leak-clean; the copy-count win recorded. | S |
| 4 | **Tier 2 — alias-the-dead-store (GATED, default off) + the arc-C steer (option B).** When `dead_store_accesses` flags an `Owned` non-escaping local `reads==0 & write_targets>0` AND `alias_is_safe`, emit a borrow AND the recommended-idiom steer "treated as write-through; write `&…`". Behind `LOFT_ALIAS_DEADSTORE`. | a hex_terrain-shaped fixture: OFF → 0-land-cells + the dead-store warning; ON → correct heights + the write-through steer, both backends, leak-clean. Escaping-local control → does NOT alias. | M |
| 5 | **Measure Tier 2's blast radius; owner sets the fork.** Run the full corpus + the registry libs under `LOFT_ALIAS_DEADSTORE`; enumerate every site whose observable output changes (all should be prior dead-store bugs). Present A/B/C to the owner with the measured set. **Do not default-on without the ruling** (+ the contract-key decision if post-freeze). | the changed-output set is enumerated and each is confirmed a prior lost-write bug (no correct program in it); `log()` it, no silent default. | S |
| 6 | **(On owner go) land Tier 2 per the ruling** — default-on-with-steer (B), or keep gated (A), or contract-keyed (C4) if post-freeze. Update OWNERSHIP_MODEL.md + C86's "revisit when" (now done: `ElidePlan` widened). | the ruling's behaviour is the suite's behaviour; C86/OWNERSHIP docs reconciled. | S |

**Shape:** Tier 1 (steps 1–3) is an unconditional win — the safe, contract-clean "make everything the
same, alias more". Tier 2 (steps 4–6) is the hex_terrain recovery, gated + measured + owner-decided,
because it is a genuine C86 refinement with a real trade-off. The safety oracle (step 1) is the
keystone — it is what guarantees #415 never returns.

## Relation to C86, arc C, and the freeze

- **C86 stays the contract.** This is its *"revisit when: widen `ElidePlan`"* clause cashed out — the
  semantic (copy-by-default) is unchanged; the compiler just realizes the alias in more of the safe
  set. Tier 1 needs no C86 change; Tier 2 refines the *dead-store corner* only, with an owner ruling.
- **Arc C is the delivery vehicle for Tier 2's non-spookiness** — the write-through steer is a
  `#`-free recommended-idiom warning on owned source (the [arc-C channel](recommended-idiom-channel.md)),
  so the alias is announced, not silent.
- **The freeze (arc E).** Tier 1 is contract-clean any time. Tier 2 pre-freeze is a straight
  refinement; post-freeze it is contract-keyed (C4) — copy under the old contract, alias+steer under
  the new — so no shipped program's observable behaviour changes without a contract bump.

## See also

- [DESIGN_DECISIONS.md § C86](../../DESIGN_DECISIONS.md) — the copy-by-default contract this refines
  (+ its "revisit when: widen `ElidePlan`").
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the bind law + `#415` (why the alias became a copy:
  the UAF the safety gate now prevents).
- [recommended-idiom-channel.md](recommended-idiom-channel.md) — arc C; Tier 2 option B's steer.
- Code-points: `src/use_analysis.rs` (`Ownership::Borrowed`, `dead_store_accesses`, `warn_dead_stores`
  S4a, the elision verdict) · `src/scopes.rs` (`elide_borrows` / `move_elide` — where a copy becomes an
  alias) · the `#415` store-lifetime guards (`tests/scripts/85-store-lifetime-field-read-copy.loft`).
