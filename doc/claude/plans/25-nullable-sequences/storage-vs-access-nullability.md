<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — separate STORAGE-nullability from ACCESS-nullability

> Re-examines the @PLN25 decision *"`vector<S>` is nullable by default"* (2026-06-20,
> `single-payload-refactor.md`) against new evidence: the crawler wave (Families N/A,
> #462) and the loft2 sandbox **walker** (`@PLN86 expand-walker.md` A1/A2). The
> decision is **not wrong about the capability** (sparse sequences are real) but is
> wrong about the **default**, and the default break is a genuine soundness defect.
> Written design-protocol-style: invariant, re-assertion count, load-bearing claims to
> probe, cost. **Nothing built yet** — this is the doc-before-code step.

## The fundamental flaw, named precisely

@PLN25 made `vector<S>` mean `vector<__nullable<S>>` (a *rewrite*, default-on, `not
null` opts out). That rewrite is **not stable under type substitution**, and a
type-former that isn't substitution-stable is unsound:

```
elaborate(vector<S>)        = vector<__nullable<S>>     (S is a known struct → rewritten)
elaborate(vector<N>)[N:=S]  = vector<N>[N:=S] = vector<S>   (N generic → NOT rewritten)
```

So `⟦vector<N>⟧[N:=S] ≠ ⟦vector<S>⟧`. **Parametricity is broken** — and that is
*exactly* why the generic walker `walk<N>(root, expand: fn(N)->[N], …)` cannot be
written (walker A1: "`vector<T>` unifies as `vector<__nullable<T>>`, which won't match
`vector<N>` under a generic HOF parameter"). It is also why Families N (element
over-promotion) and A (literal-default not materialised) exist: the rewrite forces a
*materialisation* (Some-wrap) at every construction site, and the materialisation is
re-asserted per syntactic position instead of by the type relation.

**The root conflation.** @PLN25 made *storage* nullable to give *indexing* a null to
return (`v[i]` out-of-bounds, sparse slots). But these are **orthogonal**:

| concept | question | where it belongs |
|---|---|---|
| **storage**-nullability | *can an element value be absent?* | the element TYPE (`?T`), parametric |
| **access**-nullability | *can this lookup miss?* (OOB) | the INDEXING rule, independent of storage |

Coupling them (nullable storage to serve OOB access) is what breaks parametricity,
forces materialisation (N, A), and creates the tagged-representation lifetime
complexity (#462, walker A2). Decouple them and all four collapse.

## How it interacts with the formal definition

The honest finding: **the default-nullable rewrite has NO formal rule.** `formal/types.md`
lists `(T-Chk-Vec) [e₁…eₙ] ⇐ vector<τ> ⟸ ∀i. eᵢ ⇐ τ` over a *uniform* `vector<τ>`, and
nullability only via `(C-Var)`'s `__nullable<S>` conversions. The rewrite is an
**unformalised implementation layer** — which is *why* it produces un-anticipated
breakage: there was no rule to check it against. Two consequences:

- **Family N is a DEVIATION from `(T-Chk-Vec)`**, not a formal gap: the code uses a
  struct-literal PEEK instead of `∀i. eᵢ ⇐ τ`. Conforming to the rule deletes it.
- **The fix is mostly formalisation, not invention.** It adds one missing rule and
  deletes the unformalised rewrite:

```
  (T-Index)   Γ ⊢ v ⇒ vector<τ>,  Γ ⊢ i ⇒ Integer[..]   ⟹   Γ ⊢ v[i] ⇒ ?τ
```

Access introduces `?τ` (the OOB-null), for ANY `vector<τ>` — this is where the null
@PLN25 baked into storage actually belongs. The runtime already supports it
(`OpGetVectorNullable` et al., 2026-05-11). `?τ` becomes a first-class type former;
`(C-Var)` already gives the intro `τ ⤳ ?τ` (materialise/Some-wrap), and the dual `?τ ⤳
τ` is tightened to **non-implicit** (requires `??`/`match` — implicit elimination of an
option is unsound; the current loose listing is a rough spot).

This mirrors the **integer model** that `formal/types.md` already settled: *one type
former; the representation is derived.* Integers — one `integer`, width derived from
range. Sequences — one `vector<τ>`, dense-vs-tagged derived from whether τ is `?`. No
shape-conditional rewrite in either.

## The invariant (one sentence)

> **`vector<τ>` is dense and uniform for every τ (including generic); a value's
> nullability is carried only by an explicit `?` in its type, and a lookup's
> partiality is carried only by `(T-Index)` — never by an implicit rewrite of the
> container.** Type formation then commutes with substitution (parametricity holds),
> and every "materialise a value into a slot" is the single conversion `τ ⤳ ?τ`.

## The design

1. **Storage dense by default, nullable explicit.** `vector<T>` stores inline `T` for
   all `T` incl. generic `N`. Genuinely-sparse sequences are written `vector<?T>`
   (the existing `__nullable<T>` representation, now only when *written*). Delete the
   default-on rewrite (`e2_rewrite_enabled` / the `sub_type` vector arm); keep the
   `__nullable<T>` machinery for explicit `?T`.
2. **Indexing is partial: `v[i] ⇒ ?τ`** regardless of storage. `v[i] ?? d`, `if let`,
   bounds-checked use all keep working — *unchanged for consumers*, because the `??`
   sites are ACCESS, not storage. A sparse `vector<?T>` gives `v[i] ⇒ ??T ≡ ?T`.
3. **`?τ` is a first-class, parametric type former** with intro `τ ⤳ ?τ` (the ONE
   materialisation chokepoint, `(C-Var)`) and explicit elim (`??`/`match`).
4. **The walker types trivially:** `walk<N>(root: N, expand: fn(N)->vector<N>, max)` —
   `vector<N>` is dense and uniform; `frontier[i] ⇒ ?N` discharged by the worklist;
   the null-skip the prototype dropped is moot (dense ⇒ no element-nulls). A1 gone.

## Re-assertion count (the design-protocol tell)

| Today | After |
|---|---|
| materialisation re-asserted per position (assign/field/match ✅; return/arg/HOF 🔴) + a PEEK | ONE rule `(T-Chk-Vec)` + `τ ⤳ ?τ`; positions inherit it. Family N+A **delete** |
| default-rewrite, substitution-unstable, breaks generics | no rewrite; `vector<τ>` uniform; parametric |
| OOB-null faked via storage | `(T-Index)` carries it; storage free of it |

#462 (the runtime slot-reuse UAF) **shrinks but does not vanish** — dense storage
removes most tagged-element nested stores (a big reduction in the lifetime surface),
but ownership is a *different* relation (`OWNERSHIP_MODEL.md`); don't fold it in.

## Load-bearing claims to PROBE before building (expect to falsify)

1. **Parametricity claim** — a generic `vector<N>` actually type-checks + runs for
   `N := S` (struct), `integer`, `text`, nested. (The walker is the consumer probe.)
2. **`v[i] ??` consumers survive untouched** — crawler's `enemies[i] ?? …`,
   `s.enemies[i] ?? s.enemies[0]` keep working under access-nullability with DENSE
   storage. **This is the migration-size claim** (the difference between "rewrite the
   `??` sites" and "rewrite only the genuinely-sparse declarations"). Probe a sample.
3. **Genuinely-sparse consumers** — anything that does `v[i] = null` / iterates nulls
   needs `vector<?T>`. Count them; that is the real migration cost, not every `??`.
4. **Cleanest-claim attack (over-unification guard):** does ANY current consumer rely
   on `for x in v` yielding nulls from default-nullable storage? If yes, dense-default
   changes its iteration type — find those before committing.
5. **`(C-Var)` dual must be tightened** — confirm nothing relies on implicit `?τ ⤳ τ`
   (it would hide a null). Probe the conversion table for silent option-unwrap.

## Cost / migration (you said you'll rewrite every test)

The migration is **smaller than the ~107 the E2 default-on note implies**, *if* claim 2
holds: most nullable use is `v[i] ??` (access) which is preserved; only declarations
that genuinely store null flip to `vector<?T>`. The honest unknown is claim 3's count.
The build order: (a) probe claims 1–5; (b) add `(T-Index)`, make `?τ` explicit, flip
the default in the `sub_type` chokepoint; (c) re-prove the field-map probes — N+A
should go green by construction; (d) migrate consumers (crawler, loft2 walker) and let
the dogfood loop surface the residual.

## Probe verdicts (2026-06-25) — design VALIDATED

| Claim | Verdict | Evidence |
|---|---|---|
| 1 — parametricity / generics | ✅ confirmed | `tests/plan25_e2_generics.rs` *already* carves generics out of the rewrite ("the generic parameter stays **dense** or `-> T` unification fails") — the carve-out IS the admission the rewrite isn't substitution-stable. Dense-default deletes the carve-out. |
| 2 — `v[i] ??` survives dense (cost-decider) | ✅ confirmed both backends | `v[5] ?? d` on `vector<S not null>` → default fires; `v[0]` → element. `t[99] ?? -1` on `vector<integer not null>` → -1; `t[1]` → 20. Access-nullability is already independent of storage. |
| 3 / 4 — sparse-storage reliance (migration size) | ✅ ~zero | crawler: 461 `vector<>` decls, **0** `not null`, **732** `??` (access, survive), **0** genuine element `= null` writes; libs: 0 sparse writes. No consumer relies on null-element storage. |
| 5 — implicit `?τ ⤳ τ` | ⏳ deferred | a tightening (require `??`/match); low-risk, audit during build. |

**Migration is a DEFAULT FLIP, not a rewrite of every test.** The ~107-test figure is for
pushing the rewrite *harder* (E2 default-on, incomplete access glue); we go the *other*
way (dense), which the 732 access sites already tolerate. Genuinely-sparse consumers
(none found yet) opt in with `vector<?T>`.

## Rewrite plan (sequenced, loft-codegen-gated)

The flip-point is **one chokepoint**: `e2_rewrite_enabled` / the `sub_type` vector arm
(`parser/expressions.rs:2345`, `definitions.rs`) — today *default = nullable, `not null`
opts out*. Steps:

1. **Syntax:** ensure `vector<?T>` (opt-IN nullable element) parses — today the opt-out
   is `not null`; the flip needs `?T` as the opt-in marker (verify/add).
2. **Flip the chokepoint:** `vector<T>` stays dense (no `__nullable<T>` synth) unless the
   element is written `?T`. Keep all `__nullable<T>` machinery for the explicit case.
3. **`(T-Index)`** already holds at runtime (claim 2) — confirm the *type* of `v[i]` is
   `?τ` for dense τ so `??`/`match` stay well-typed; add the formal rule to
   `formal/types.md`, delete the default-rewrite deviation.
4. **Re-prove** the field-map probes (`46N-*`, `46A-*`, `sib-nullcoalesce-*`) — N+A go
   green by construction (no rewrite → no materialisation mismatch).
5. **Dogfood:** rebuild crawler + the loft2 walker; let real use surface the residual
   (claim 5, generic-HOF inference — the walker's *other* gap, `Unknown field N.kids`).

## Build-probe results (2026-06-25) — env-gated dense flip `LOFT_DENSE_VECTORS`

A reversible measurement gate (`definitions.rs` vector arm: default dense unless
`not null`) — no `?T` syntax yet, so it makes ALL vectors dense. Findings:

| Target | dense-default result |
|---|---|
| **Family N** (`[fncall]`, `[ternary]` → vector<S>) | ✅ **FIXED** both backends — `main_vector<S>` dense, no `__nullable`, no over-promote |
| **#462** the sev:high SIGSEGV (crawler `questtest`) | ✅ **FIXED** — exit 0, `QUEST OK`, no crash. Dense removes the tagged-element nested stores the slot-reuse UAF fed on |
| **Family A** (`vv[i] ?? [literal]`) | 🔴 **NOT fixed** — still fails. A is a **separate Layer-2 codegen bug** (coalesce vector-literal default not materialised), independent of storage nullability |

**Design correction (over-unification caught by the build):** the doc claimed "A folds
into the Layer-1 fix." FALSE — dense-default fixes N + #462 but not A. A is its own
codegen fix (materialise the `??` vector-literal default). Keep it separate.

So dense-default's payoff is **N + #462** (the parametricity break and the crash); **A
is orthogonal** and fixed independently. This is the strongest evidence yet that the
@PLN25 default-nullable was the root of N and #462.

## Blast radius (2026-06-25) — full suite under the dense gate

**2542 run · 2529 passed · 13 failed (0.5%).** The 13 split into:

- **Genuine nullable-feature tests** (migrate to `?T`): `plan25_e2_json`
  (all_null_elements, null_leading, null_in_the_middle), `plan25_e2_hash`
  (null_in_shared_vector), `issues p143` (default-struct-return-from-nested-vector),
  `159-p385` (text-nullable-else-null). These *assert* nullable-element storage — they
  break by design and become `vector<?T>` once syntax exists.
- **Half-flip artifacts** (vanish under a COMPLETE flip): the `wrap`/`native` aggregate
  suites fail on a few scripts with `vector<Point> != vector<__nullable<Point>>`
  (`tests/docs/17-libraries.loft:82`) and `expected &vector<Item>, got
  vector<__nullable<Item>>` (`tests/scripts/11-vectors.loft:72`) — MIXED dense/nullable
  because the env gate flips only the **declared-type chokepoint** while the
  **inferred-literal PEEK** (`vectors.rs:1424`) + comprehension twin still synthesise
  `__nullable`. Retiring the PEEK in the real flip removes these.

So a *complete* dense flip is **~99.5% source-compatible**; the real migration is the
small nullable-feature set → `?T`. "Willing to rewrite every test" turns out to be
~5–10 test groups, not 107.

## BUILD DONE (steps 1–3) + step-5 suite finding (2026-06-25)

**Built and landed in the working tree:** `?S` opt-in syntax (`vector<?S>`), dense default
at the chokepoint, the inferred-literal PEEK + comprehension twin retired, `(T-Index)` in
`formal/types.md`. **Verified:** `vector<S>` dense; `vector<?S>` stores+reads null; keyed
`?` rejected; **Family N fixed both backends; #462 (the crawler SIGSEGV) fixed — `QUEST
OK`, no crash**; the half-flip artifacts gone.

**Full suite under the real flip: 2542 run · 2535 passed · 7 failed.** The 7 split:

- **4 nullable-feature tests** (`plan25_e2_json` all_null_elements / null_leading /
  null_in_the_middle; `plan25_e2_hash` null_in_shared_vector) — they assert nullable
  storage; **migrate their source to `vector<?S>`**. Mechanical.
- **2 store-lifetime tests + 1 native mirror** (`150-i306-view-return-ownership`,
  `85-…-borrowed-view-overfree`) — **a GENUINE dense-path regression, not a migration.**
  A borrowed-element return (`return table[idx] ?? m_none()`) now **over-frees the source
  element** under dense storage (`len(t)` 2→1; the Holder corrupts). 85's own comment names
  it: a borrowed-view return must **deep-copy**; under nullable storage that deep-copy
  fired, under dense it does not → over-free.

**This is the Layer-3 ownership relation (`OWNERSHIP_MODEL.md`), exactly the arc the design
said NOT to fold in.** Dense-default *trades* the #462 crash for a borrowed-view over-free
in the dense path — so the flip is the right DIRECTION (walker/N/#462/parametricity all
fixed) but **not landable until the dense borrowed-view deep-copy fires.** Hypothesis (to
matrix-investigate, not patch blind): the borrowed-view-detection that triggers the
deep-copy keyed off the `__nullable<S>` enum shape; a dense element doesn't trip it, so the
return frees the source. Fix = make the deep-copy fire for a returned dense vector element
too. This is the remaining blocker, and it's the @PLN85 ownership work — its own focused
matrix-first effort, not a tail-of-session patch.

> **Status (superseding below):** core rewrite BUILT + validated (N, #462, parametricity,
> formal). Remaining to land green: (a) migrate 4 nullable-feature tests to `?S`; (b) fix
> the dense borrowed-view over-free (Layer-3 ownership — the real blocker); (c) Family A
> (separate, deferred). The env gate is gone — dense is the live default in the tree.
> Original bounded-build plan:
> 1. Add `?T` opt-in syntax (`vector<?S>`).
> 2. Complete the flip — chokepoint dense-default AND retire the inferred PEEK +
>    comprehension synth (kill the half-flip artifacts); drop the env gate.
> 3. Formal: add `(T-Index) v[i] ⇒ ?τ`, delete the default-rewrite deviation.
> 4. Fix Family A separately (coalesce vector-literal default materialisation).
> 5. Migrate the ~5–10 nullable-feature test groups to `vector<?T>`; re-run suite.
> The env gate (`LOFT_DENSE_VECTORS`, `definitions.rs`) is the reversible seed of step 2.
