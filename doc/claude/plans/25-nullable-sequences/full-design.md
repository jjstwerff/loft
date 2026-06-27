<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 — the full design: one value model where `null` is a real, typed thing

> **The canonical, consolidated design.** Supersedes the keep-vs-drop history in
> [README.md](README.md) and the 2026-06-20 "default = nullable" decision. The
> corrected invariant was reached in [storage-vs-access-nullability.md](storage-vs-access-nullability.md)
> and formalised in [`../../formal/types.md` § Nullability](../../formal/types.md);
> the build order is [implementation-steps.md](implementation-steps.md). This doc
> ties them into one design an agent can execute from cold, and **designs the one
> remaining blocker** (the dense borrowed-view over-free — [#465](https://github.com/loft-lang/loft/issues/465)).
>
> **Why this exists (the steer):** the store-lifetime blockers that parked this work
> are now mostly cleared (the crawler crash + leaks fixed). loft wants a *principled
> value model* — not a patch-fest of per-site special-cases for "empty vs absent".
> Every recent vector/null bug (#465, the `??`-literal panic, Family N, #462) is a
> symptom of one missing foundation. This design installs the foundation; the
> symptoms come out in the wash.

---

## The one invariant

> **`vector<τ>` is dense and uniform for every `τ` (including a generic `N`). A
> value's nullability is carried only by an explicit `?` in its type (`τ?`); a
> lookup's partiality is carried only by the fallible operations (`(N-Index)`,
> `(N-Div)`, …) that synthesise `τ?`. There is no implicit rewrite of the container
> and no implicit unwrap of an optional.**

From this one rule, every case a probe never tested behaves correctly *for the same
reason*: type formation commutes with substitution (parametricity holds), and every
"put a value into a slot that admits null" is the single conversion `τ ⤳ τ?`
(`(N-Intro)`). It is the **integer model applied to null** — one type former, the
representation (dense vs tagged) derived from whether `τ` is `?`, exactly as integer
width is derived from range. The fix is therefore *formalisation, not invention*: add
the missing rules, delete the unformalised default-rewrite.

The full type-level statement is **`formal/types.md` § Nullability** (`N-Opt`,
`N-Idem`, `N-Dense`, `N-Intro`, `N-Index`, `N-Div`, `N-Parse`, `N-Arith`, `N-Cast`,
`N-Cast?`, `N-Coal`, `N-Match`) with deviations `DN1`–`DN4` tracking where today's
code still breaks them.

## The root defect being removed

The shipped default (`main`) makes `vector<S>` mean `vector<__nullable<S>>` — a
*rewrite*, default-on, `not null` opting out. That rewrite is **not stable under type
substitution**:

```
elaborate(vector<S>)        = vector<__nullable<S>>     (S known struct → rewritten)
elaborate(vector<N>)[N:=S]  = vector<S>                 (N generic → NOT rewritten)
⟹  ⟦vector<N>⟧[N:=S] ≠ ⟦vector<S>⟧      — parametricity broken
```

That single unsoundness is the shared root of:

| Symptom | Why the rewrite causes it |
|---|---|
| Generic HOF (`walk<N>(…, fn(N)->vector<N>)`) can't be written | `vector<N>` won't unify with the rewritten `vector<__nullable<T>>` |
| **Family N** (element over-promotion) | construction must Some-wrap at every syntactic position; re-asserted per site, not by the type |
| **#462** crawler SIGSEGV | the tagged-element nested stores the slot-reuse UAF fed on |
| **#465** dense/borrowed-view over-free (the residual) | the borrow→deep-copy decision is keyed off the `__nullable<S>` *shape*, re-derived per site |
| empty `[]` vs `null` indistinguishable | both lower to `Value::Null`; the match/return delivery cannot tell empty from absent |

The conflation at the heart: the rewrite made **storage** nullable only to give
**indexing** a null to return. Those are orthogonal — storage-nullability is a
property of the element *type* (`τ?`, parametric); access-partiality is a property of
the *indexing rule* (`(N-Index): v[i] ⇒ τ?` for any `τ`). Decouple them and all five
collapse.

## Re-assertion count — the design-protocol tell

| | Today (rewrite) | After (one rule) |
|---|---|---|
| materialise (Some-wrap) | re-asserted per position (assign/field/match ✅; return/arg/HOF 🔴) + a struct-literal PEEK | ONE conversion `τ ⤳ τ?`; positions inherit it. Family N **deletes** |
| container nullability | default-rewrite, substitution-unstable | no rewrite; `vector<τ>` uniform + parametric |
| OOB null | faked via storage | `(N-Index)` carries it; storage free of it |
| borrow→copy on return | keyed off `__nullable` shape across 4 delivery sites | the carried `deps` ownership fact (see § Ownership) |

`N × silence` is the brittleness, known before any code: today the materialisation
invariant is silently omittable at the return/arg/HOF positions (a wrong result, not a
compile error). The cure is to **collapse N to 1** — one conversion rule the positions
inherit — and to **make omission loud** where it can't (the discharge check `N-Store`
turns "forgot to handle null" into a compile error).

## Load-bearing claims — probed, verdicts recorded

(From [storage-vs-access-nullability.md](storage-vs-access-nullability.md) § Probe
verdicts, 2026-06-25 — the design is **validated**, not asserted.)

| Claim | Verdict | Evidence |
|---|---|---|
| 1 — parametricity / generics hold under dense | ✅ | the existing `plan25_e2_generics` carve-out *is* the admission the rewrite isn't substitution-stable; dense deletes the carve-out |
| 2 — `v[i] ??` consumers survive dense (the cost-decider) | ✅ both backends | access-nullability already independent of storage; `v[5] ?? d`, `t[99] ?? -1` fire on dense `not null` vectors |
| 3/4 — sparse-storage reliance (migration size) | ✅ ~zero | crawler: 461 vector decls, 0 `not null`, 732 `??` (access — survive), 0 genuine element `= null`; libs: 0 sparse writes |
| 5 — implicit `τ? ⤳ τ` unwrap | ⏳ tighten in Phase 3 (DN2) | low-risk; audit during build |

**Build-validated (env-gated dense flip, then the real flip on `2026-07-mac`):** Family
N fixed both backends; **#462 SIGSEGV fixed** (`QUEST OK`, no crash — dense removes the
tagged-element nested stores the UAF fed on); the half-flip artifacts gone; full suite
**2542 run · 2535 passed · 7 failed**. The 7 are the *known, bounded* remainder below —
not surprises.

**Over-unification caught by the build (recorded, not hidden):** the doc had claimed
"Family A folds into the dense fix." FALSE — dense fixes N + #462 but **not** Family A
(the `?? <vector-literal>` codegen panic), which is an orthogonal Layer-2 codegen bug.
*Family A is now fixed independently on `main`* (the work-ref slot fix, commit
`6ea779be`, regression `tests/scripts/440`). The build was the last probe and it
corrected the design — kept here as the worked example.

---

## The value model (what changes, in words)

1. **Storage dense by default, nullable explicit.** `vector<T>` stores inline `T` for
   all `T` incl. generic `N`. A genuinely-sparse sequence is written `vector<T?>` (the
   existing `__nullable<T>` representation, now only when *written*). The default-on
   rewrite (`e2_rewrite_enabled` / the `sub_type` vector arm + the inferred-literal
   PEEK + the comprehension twin) is **deleted**; the `__nullable<T>` machinery stays
   for the explicit `T?` case.
2. **Indexing is partial: `v[i] ⇒ τ?`** regardless of storage (`N-Index`). `v[i] ?? d`,
   `match`, bounds-checked use all keep working *unchanged for consumers* — the `??`
   sites are ACCESS, not storage. A sparse `vector<τ?>` gives `v[i] ⇒ τ?? ≡ τ?`.
3. **`τ?` is a first-class, parametric type former** — intro `τ ⤳ τ?` (the ONE
   materialisation chokepoint, `N-Intro`), explicit elim (`??` / `match`, `N-Coal` /
   `N-Match`). No implicit `τ? ⤳ τ` (`DN2`).
4. **Scalars/fields too.** `x: integer?` is the optional scalar; the fallible ops
   (`/`, `%`, overflow, `parse`, `as`) synthesise `τ?` per `N-Div`/`N-Arith`/
   `N-Parse`/`N-Cast?`. `not null` becomes a no-op (non-null is the default), then is
   retired from the parser.
5. **`null` is the universal "no representable result"** — never wrap, never saturate
   (that is UB). Nullability is range-driven: an op is non-null when its result
   provably fits, `τ?` when it could miss.

The runtime null representation **already exists** (`/0→null`, overflow→null, OOB→null,
scalar null sentinels — all in `fill.rs`), so this is mostly *type-level* work (carry
`τ?`, enforce discharge) plus **one new runtime op**: the checked cast `as τ?`
(`N-Cast?`).

---

## The build — EXPAND → MIGRATE → CONTRACT → TIGHTEN → CLEANUP

The ordering principle (full detail in [implementation-steps.md](implementation-steps.md)):
never flip a default before the code is ready, so *as much as possible passes the whole
way*. Each phase ends green (or a known, bounded, measured set). Never carry two phases'
breakage at once.

| Phase | What | Breakage | Status |
|---|---|---|---|
| **0 EXPAND** | `T?` parses (vectors + scalars), `τ?` former + `N-Intro` — additive | zero | vectors ✅; scalars ⬜ |
| **1 MIGRATE** | annotate every genuinely-nullable site `?` *while default still nullable* (no-op) | zero | the test-saver; partly skipped → the 4 nullable-feature failures |
| **2 CONTRACT** | flip the default to dense/non-null; retire the PEEK + comprehension synth | the MIGRATE misses (1-char `?` each) | vectors ✅ (live on `2026-07-mac`); scalars ⬜ |
| **3 TIGHTEN** | discharge + cast strictness — `DN2` (drop implicit unwrap) → `DN4` (`as` fit / `as τ?`) → `DN3` (fallible ops typed `τ?`, `N-Store` makes un-discharged null a compile error) | the one measured break; **lands last** | ⬜ |
| **4 PARALLEL** | the ownership over-free (#465, § below) + Family A (✅ done) | — | #465 ⬜ (the blocker) |
| **5 CLEANUP** | strip `not null` from all `.loft`, then from the parser (in that order) | zero (Phase 2 made it a no-op) | ⬜ |

**Current landable state of the rewrite (on `2026-07-mac`):** vectors-dense is built
and live; 7 tests fail = "Phase 2 ran ahead of Phase 1": **4** nullable-feature tests
(do Phase 1 — annotate `vector<T?>`; mechanical) **+ 3** the #465 ownership over-free
(Phase 4, below). No revert needed — the disciplined recovery is *complete Phase 1* +
*fix #465*, then the vectors half is the first mergeable nullability checkpoint.

---

## The remaining blocker — #465, the dense borrowed-view over-free (Phase 4, step 6)

This is the one piece that was only a hypothesis; here it is designed.

### What it is
Under dense storage, a function that **returns a borrowed view of an element/field**
(`return table[idx] ?? m_none()`, `match c { Filled{items} => items, _ => [] }`)
**over-frees the source** — `len(t)` drops 2→1, the holder corrupts (interp), or the
returned store is freed-then-reused (the #462-family SIGSEGV under churn). On `main`
(nullable storage) the same shape *leaks* or aliases; under dense it *over-frees*.
Same bug class, opposite sign — because it lives in a **different relation** than
nullability: ownership (`OWNERSHIP_MODEL.md`), which the value-model design deliberately
does **not** fold in.

### Why dense exposes it
The borrowed-view→deep-copy decision was **keyed off the `__nullable<S>` enum shape**.
A dense element doesn't trip that shape test, so the return takes the *adopt* (alias)
path and frees the source. The detection is the wrong fact: it asks "is this the tagged
shape?" when it should ask "**does this return value borrow a still-live source?**"

### The invariant (one sentence)
> **A returned value whose `deps` name a still-live source (an argument, or a local
> that outlives the return) is COPIED into the return buffer by value, and that
> borrowed source is NEVER freed by the copy; a returned value that OWNS its store is
> moved (renamed onto the buffer). The choice is read from the carried `deps`
> ownership fact — not re-derived from the value's representation shape.**

### Re-assertion count — the real defect
The copy-vs-alias decision is today **re-derived across four delivery sites**
(`vec_match_candidate`, `classify_vector_delivery`, `ref_return`,
`materialize_vector_arms_into`), each reading a *different proxy* (the `__nullable`
shape, the type-dep `ls`, the tail syntactic form). #465 is exactly a case that falls
between them: a match-arm binding borrows an arg, but the borrow lives in the binding's
own `deps` while `ls` carries only the buffer work-ref — so no site sees it, and the
alias ships. **N = 4, silent.** This is the same `deps`-fused-into-`Type` root the whole
store-lifetime class shares (`OWNERSHIP_MODEL.md`, Cluster A / H10).

### The fix direction (NOT a 5th special-case)
Consolidate the four sites onto **one** chokepoint that consults the carried `deps`
ownership fact: *borrows-a-live-source ⇒ copy + don't-free-source; owns ⇒ move*. Make
the deep-copy fire for a returned dense element by asking the ownership question, not
the shape question. The empty-arm half (`_ => []` delivering empty, not the
`Value::Null` that today blocks `vec_match_candidate`) falls out once `[]` is a real
dense empty vector rather than the null sentinel — i.e. it is *fixed by the value-model
flip itself*, not by delivery-code patching.

### Build gate (mandatory — this is the corruption-risk path)
Matrix-first (`loft-codegen` + `engineering-rigor`), because a wrong move **over-frees
the caller's buffer → UAF/silent corruption**, which a green suite does not catch.
Boundary matrix across **{enum-field-view, struct-field-view, whole-arg, index-read,
local-view} × {match-arm, if-arm, direct-return} × {returned, returned+buffer-churn}**,
asserting **value + length + leak on BOTH backends**, plus `LOFT_WATCH_STORE` for the
store-reuse collision (#426B's hazard — index-read views stay aliased until the
store-reuse substrate is proven safe). Design the chokepoint on the page, probe each
claim, *then* build. The dense-flip revert is the standing reminder of what coding
before the design costs.

---

## Branch & merge discipline

- The dense rewrite lives on **`2026-07-mac`** (gate-inert history → now live default
  there). `main` is still nullable-default. **`main` is the release branch and must not
  receive this mid-flight** (the tree fails 7 under the live dense default until Phase 1
  + #465 land).
- **Merge only at green phase boundaries.** First mergeable checkpoint = vectors green
  (Phase 1 annotate the 4 + Phase 4 #465). Then scalars (Phase 0→2), then Phase 3
  (measured tighten), then Phase 5 (cleanup) — each ending green.
- **Already on `main` (no action):** Family A (the `??`-literal panic, `6ea779be`); the
  native conditional-reassign retbuf leak fix (`603bce54`). These are the value-model
  family's *codegen* siblings, landed independently.
- `#465` stays open (the borrowed-view residual); it closes with Phase 4.

## What "done" means

The canonical incoherence probe yields a real, null-safe absent element on both
backends with no gate:

```loft
struct Item { name: text, value: integer }
items = `[ {"name":"a","value":1}, null, {"name":"c","value":3} ]` as vector<Item?>;
//  len(items) == 3 ;  items[1] == null  is true ;  present items keep their fields
```

…and a generic `walk<N>(root: N, expand: fn(N)->vector<N>, max)` type-checks and runs
for `N := struct / integer / text / nested` — the parametricity the rewrite broke. At
that point `null` is one principled, typed thing across scalars, fields, and sequences;
`?` is the only nullability marker; and #465 / Family N / the empty-vs-null conflation
are gone by construction, not by patches.
