<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Implementation steps — dense-default nullability, ordered to minimise test breakage

The model is in [`../../formal/types.md` § Nullability](../../formal/types.md) (`N-*` rules,
deviations DN1–DN4) and the design in [storage-vs-access-nullability.md](storage-vs-access-nullability.md).
This doc is **the build order**, sequenced so that *as much as possible passes the whole way*.

## The ordering principle: EXPAND → MIGRATE → CONTRACT → TIGHTEN

A default flip breaks tests in proportion to how much un-migrated code the flip surprises.
So we **never flip a default before the code is ready for it**:

1. **EXPAND** — make `T?` *available* without changing any default. Purely additive → **zero
   breakage**.
2. **MIGRATE** — annotate every genuinely-nullable site with `?` *while the old default still
   means nullable*. Because the site is already nullable, adding `?` is a **behaviour-preserving
   no-op → zero breakage**. This is the step that makes the flip safe — it pre-positions every
   nullable site so the flip doesn't surprise it.
3. **CONTRACT** — flip the default to dense/non-null. The only breakage is sites that needed
   nullable but were *missed* in MIGRATE → minimal, and each is a one-character `?` fix.
4. **TIGHTEN** — turn on the discharge/`as`-fit strictness (`N-Store`, `N-Cast`). This is the
   one genuinely-breaking step (every un-discharged fallible result), so it goes **last** and
   is measured before it lands.

Each phase ends green (or with a known, bounded, measured set). Never carry two phases' breakage
at once.

---

## Phase 0 — EXPAND: `T?` available, defaults unchanged  ·  zero breakage

- Parse postfix `T?` for vector elements (`vector<S?>`) **and** scalars/fields (`x: integer?`).
  Accepting new syntax that nothing uses yet breaks nothing.
- Type system: `τ?` is the optional former; `(N-Opt)`, `(N-Idem)`, `(N-Intro) τ ⤳ τ?`. No
  default changes, no discharge enforcement yet — `τ?` simply *coexists* with today's behaviour.
- **Test impact: none.** Gate: full suite stays green.
- Status: vectors **done** (postfix `vector<S?>` parses + works). Scalars **done** — `integer?`
  / `text?` / `S?` parse in every type position (field, param, return, `as`-cast) via a postfix
  `?` consumer wrapping `parse_type` (the named-type chokepoint; the vector element `?` is
  consumed earlier in `sub_type_inner`, so it isn't stolen). Today a no-op (plain types are
  already nullable); for Integer the `?` records `not_null:false` so it survives the Phase-2
  flip, other scalars accept-and-ignore until Phase 2 adds their optional representation.
  Regression: `tests/scripts/25-scalar-optional-syntax.loft`. Full suite green.

## Phase 1 — MIGRATE: annotate nullable sites with `?`  ·  zero breakage  ← the test-saver

- While the default is STILL nullable, add `?` to every site that genuinely needs null: the
  `plan25_e2_*` nullable tests (`vector<S?>`), any field/element that stores `null`, the lib
  consumers found by the survey. Under the old nullable-default these annotations are no-ops.
- Survey first (cheap): grep for element/field `= null`, `for x in v` null-use — the genuinely
  sparse sites (the vector survey found ~0 in crawler; scalars need their own).
- **Test impact: none** — annotating an already-nullable site changes no behaviour. This is the
  whole point: every nullable site is `?`-marked *before* the flip can surprise it.
- Gate: full suite still green, now with the nullable sites explicit.

## Phase 2 — CONTRACT: flip the defaults  ·  minimal breakage

- Vector element default → dense + retire the inferred PEEK + comprehension twin. **(done)**
- Scalar/field default → non-null; `not null` → accepted no-op (DN1).
- **Test impact: only the MIGRATE misses** — a nullable site that Phase 1 didn't annotate now
  errors; fix = add the `?` it should have had. Bounded; each is one character.
- Gate: green after sweeping the misses. (Current vectors-only state: 4 nullable-feature tests
  still failing = Phase-1 work not yet done for them; doing Phase 1 takes 7→3.)

## Phase 3 — TIGHTEN: discharge + cast enforcement  ·  the measured breakage, LAST

Order within, least-to-most breaking:
- **DN2** — remove the implicit `τ? ⤳ τ` unwrap. Breakage: code relying on silent unwrap.
- **DN4** — `as τ` requires provable fit (error → `as τ?`); `as τ?` is the checked cast.
  Breakage: out-of-range casts (`400 as u8`); fix = `as τ?` / `?? d` / mask.
- **DN3** — type fit-failures (`/`,`%`,`[]`,`parse`, overflow) as `τ?`; `(N-Store)` makes an
  un-discharged null into a non-null slot an **error**. **Biggest blast radius** — every
  `b = a / x` etc. without a `??`.
- **Measure before landing DN3**: count sites assigning a fit-failing result into non-null
  storage without `??` (the gating number). Migrate by adding `?? d` / `as τ?` / a mask.
- Gate: green after the discharge migration. This is "I'm willing to rewrite the tests" — and
  it's exactly here, last, after everything else is already green.

## Phase 4 — parallel fixes (not on the critical path)

- **Step 6** — the dense borrowed-view over-free (`150-i306`, `85-borrowed-view`): a Layer-3
  ownership bug the dense flip exposes; matrix-first (`OWNERSHIP_MODEL.md`). Blocks vectors going
  fully green independently of the nullability typing.
- **Family A** — `??`-with-a-vector-literal default lowering to `()` (pre-existing codegen).

## Phase 5 — CLEANUP: retire `not null` from the parser  ·  ordering is load-bearing

Once non-null is the default everywhere, `not null` carries no meaning. Remove it — but in
this order, or ~680 existing `not null` sites become parse errors at once:

1. **Strip `not null` from all `.loft` source** — stdlib (`default/*.loft`, 3), tests
   (`tests/**`, ~275), libs + consumers (crawler ~403). Mechanical sweep; by Phase 2 `not null`
   is already a no-op, so deleting it is **behaviour-preserving → suite stays green**. Do it
   per-area, re-running the targeted suite after each.
2. **Verify no source uses it** — `grep -rn "not null"` over all `.loft` is clean.
3. **Remove `not null` from the parser** — `definitions.rs`: the scalar `not null` parse
   (~line 1470), the vector arm's `has_keyword("not")`, and the keyed arms' accept-as-no-op
   (`index`/`hash`/`sorted`/`spacial`). After this `not null` is a **syntax error** — the
   retirement is enforced, not just conventional.
4. **Docs/skills** — drop `not null` from `LOFT.md`, `loft-write` skill, `formal/types.md`
   notation, and any examples; `?` is the only nullability marker left.

Test-safety: steps 1–2 are no-ops (green throughout); step 3 only "breaks" code that step 2
proved nonexistent. Pure subtraction — the end state has one nullability notation (`?`), no
non-null marker, because non-null is simply the default.

---

## Reconciliation with the in-flight work (branch `fix-crawler`, all UNCOMMITTED)

This rewrite did not start clean — it grew out of the crawler dogfood wave, and three
threads currently coexist uncommitted on `fix-crawler`. Naming them keeps the breakage
legible and the merge order safe.

### The three threads

1. **#460 / #461 — crawler-wave native/packaging bugs (FIXED, green, separable).**
   `src/main.rs` (#460 entry-package auto-native skip) + `src/native_lib.rs` (#461 cdylib
   type-index fingerprint) + their regressions in `tests/n3_use_native.rs`, plus the
   `tests/exit_codes.rs` moros adjustment. These are **independent of the nullability rewrite**
   and pass on their own — they can be committed/merged first, at any time.
2. **#462 + the dense-vector flip — the nullability rewrite's vectors-half (LIVE, breaking).**
   `src/parser/definitions.rs` (dense default + postfix `?`) + `src/parser/vectors.rs` (PEEKs
   retired). The dense default is **live in the tree**, which is what fixes #462 (the SIGSEGV)
   as a side effect. This is **Phase 0 + Phase 2 done, Phase 1 skipped** — hence the breakage.
3. **Design + investigation docs (no code risk).** `formal/types.md` (the `N-*` rules), the
   `@PLN25` design + this plan, the `@PLN85` field-map / site-inventory / cluster-462 / probes.

### Phase status of the in-flight code (where thread 2 actually is)

| Phase | vectors | scalars |
|---|---|---|
| 0 EXPAND (`?` syntax) | ✅ done | ✅ **done** (`integer?`/`text?`/`S?` parse, no-op) |
| 1 MIGRATE (annotate `?`) | ✅ done (merged #467) | ⬜ survey + annotate |
| 2 CONTRACT (flip default) | ✅ done (merged #467) | ⬜ needs the `τ?` representation decision (below) |
| 3 TIGHTEN | ⬜ contract-changing — stability-line deadline | ⬜ |
| 5 CLEANUP (`not null`) | ⬜ | ⬜ |

> **The Phase-2 scalars blocker (representation decision).** Only `Type::Integer` carries a
> null-flag (`not_null`); `Boolean`/`Float`/`Single`/`Character`/`Text`/`Reference`/`Enum` have
> **no type-level optional marker** (they are nullable-by-default via runtime sentinels). So when
> Phase 2 flips the scalar default to non-null, `text?`/`S?` need a way to *say* "nullable" that
> the unmarked type no longer means. Three options to weigh before building Phase 2: (a) a
> `nullable: bool` on each scalar variant; (b) a unifying `Type::Optional(Box<Type>)`; (c) reuse
> the `__nullable<S>` synth (today vector-element-only). This is a load-bearing type-system design
> choice — settle it (design-protocol) before flipping. Phase 0 deliberately defers it: `?` is a
> no-op today, so EXPAND + MIGRATE proceed without it.

So the **7 failing tests are exactly "Phase 2 ran ahead of Phase 1"**: 4 nullable-feature tests
(fixed by doing Phase 1 — annotate `vector<S?>`) + 3 the step-6 ownership over-free (Phase 4).
Doing Phase 1 retroactively is the fix — **no revert of the flip is needed**.

### How the two plans converge

- **@PLN25 (nullable-sequences)** — its 2026-06-20 "default = nullable" decision is **superseded**
  by [storage-vs-access-nullability.md](storage-vs-access-nullability.md); this rewrite IS @PLN25's
  corrected end-state (dense default, `?` opt-in).
- **@PLN85 (store-lifetime retirement)** — the umbrella for the crawler wave; #462 is one of its
  clusters ([cluster-462](../85-store-lifetime-retirement/cluster-462-slot-reuse-uaf.md)), and the
  step-6 ownership over-free lives there.
- The dense flip is the **single change that closes @PLN85's #462 cluster AND realises @PLN25's
  corrected design** — the two plans meet here. Scalars/Phase-3/Phase-5 are pure @PLN25.

### Merge discipline (THE constraint)

`main` is the release branch; the tree currently **fails 7 tests under the live dense default**,
so the rewrite **must not reach `main` mid-flight**. Order:

1. **#460/#461 are landable now** (green, separable) — commit/PR them independently if desired.
2. **Vectors green** = Phase 1 (annotate the 4) + Phase 4 step-6 (ownership) → suite green with
   the dense vector default. This is the first mergeable nullability checkpoint.
3. **Scalars** (Phase 0→2) then **Phase 3** (the measured tightening) then **Phase 5** (cleanup)
   — each ending green; merge only at green phase boundaries, never mid-phase.

Net: the "breaking current plan" is thread 2 sitting at Phase 2-without-Phase-1; the disciplined
recovery is to *complete Phase 1*, not to back out — and to keep `main` off this branch until a
green phase boundary.

---

## Current state → immediate next actions (test-safest first)

The vector flip ran ahead to Phase 2, so the disciplined catch-up is:

1. **Phase 1 for the 4 failing nullable tests** — annotate them `vector<S?>` → 7 failing → 3.
   (Pure MIGRATE; zero risk; biggest green-restoring move available right now.)
2. **Step 6** — fix the dense borrowed-view over-free → vectors fully green.
3. **Phase 0/1/2 for scalars** — add `integer?` syntax (Phase 0), survey + annotate nullable
   scalar/field sites (Phase 1), then flip the scalar default + `not null` no-op (Phase 2).
4. **Phase 3** — measure, then DN2 → DN4 → DN3, adding `??`/`as τ?` to the boundary sites.

Everything before Phase 3 keeps the suite green (or one-character-fixable); Phase 3 is the only
deliberate, measured tightening, and it lands last.

---

## Code changes — the concrete sites (what actually changes, per phase)

### Phase 0 — EXPAND (parser + type, additive)
- **`?` postfix parse.** Vectors: `definitions.rs` `sub_type_inner` vector arm — **done** (the
  postfix `?` → `e2_nullable_elem`). Scalars/fields: the scalar type parse (`definitions.rs`
  ~`not null` site, line ~1470) must accept a trailing `?` and mark the type optional. Today
  neither `integer?` nor a scalar `?` parses.
- **`τ?` as a type + `(N-Intro)`.** `convert` (`parser/mod.rs:1585`) gains `τ ⤳ τ?` (a non-null
  flows into an optional). No other `convert` branch changes yet.

### Phase 1 — MIGRATE (source edits only, no compiler change)
- Annotate `vector<S?>` / `x: integer?` at the genuinely-nullable sites. Test/lib `.loft` edits.
  No `.rs` change.

### Phase 2 — CONTRACT (flip defaults)
- **Vector default → dense + retire PEEK** — `definitions.rs` vector arm + `vectors.rs:1424`/
  `:1733` PEEKs. **done.**
- **Scalar/field default → non-null** — the `IntegerSpec.not_null` **default** flips to `true`
  (and the analogous bool/char/text nullability). The plain-type parse stops meaning nullable.
- **`not null` → no-op** — `definitions.rs` scalar `not null` parse (line ~1470) + vector arm:
  consume the keyword, set nothing (non-null is already the default). Back-compat, zero meaning.

### Phase 3 — TIGHTEN (the strictness — the real work)
- **`as` rework (DN4) — `operators.rs:1567` `"as"` branch + `cast()` `parser/mod.rs:1881`.**
  Today the narrowing `as` just *returns the narrow type* and leaves the value untouched (the
  `400 as u8 → 400` bug — comment at `operators.rs:1574` says "value stays in the 8-byte slot").
  Change to the **two-form** rule:
  - `e as τ` (target non-null): **require provable fit** — if `range(e) ⊄ range(τ)`, emit a
    compile error ("use `as τ?`"). The existing `is_narrowing_int` (`operators.rs:1579`) becomes
    the *trigger for the fit check*, not a silent accept.
  - `e as τ?` (target optional): emit a **runtime range-check** op — value if it fits, else the
    `τ?` null. This is a NEW lowering (today there is no range-checking cast op); add an
    `OpCastChecked`/range-guard in `fill.rs` + `codegen.rs` (interp) and `generation/` (native).
  - Literal-doesn't-fit (`400 as u8`) → compile error at the same site (range known statically).
- **Fit-failing ops typed `τ?` (DN3).** The runtime already nulls; the TYPE must carry it:
  - division/mod: the `/`·`%` result type becomes `Integer?` (the producer in `operators.rs`
    arithmetic typing; the runtime null-on-zero in `fill.rs` already exists).
  - overflow: `+`,`-`,`*` result type is `Integer[r]`; when `r ⊄ i64`, the result type is
    `Integer?` (range computed in the arithmetic typing; runtime overflow→null already exists).
  - index: `v[i] ⇒ τ?` (the `OpGetVectorNullable` path already nullable — type it `τ?`).
  - parse: `parse_* ⇒ τ?`.
- **`(N-Store)` discharge check — `convert()` `parser/mod.rs:1585`.** Storing/binding/arg-passing
  a `τ?` where `τ` is expected is **rejected** (no implicit unwrap) unless discharged. This is
  the one check that turns the warnings into errors; it's where the blast radius lands.
- **DN2 — remove the implicit `τ? ⤳ τ` unwrap** in `convert` (the old `__nullable<S> ⤳
  Reference(S)` dual). After this, `??`/`match` are the only ways down from `τ?`.
- **`??` / `match` elimination** — `operators.rs` `build_null_coalesce_*` already lowers `??`;
  confirm it accepts the new `τ?` types (and Family A — the vector-literal default — is the
  pre-existing gap to fix here).

### Phase 4 — parallel
- **Step 6** — dense borrowed-view over-free: `state/io.rs` `do_copy_record` / the adopt-free
  path (`OWNERSHIP_MODEL.md`); matrix-first.
- **Family A** — `operators.rs` `build_null_coalesce_default`: materialise a vector-literal
  default instead of lowering it to `()`.

### Cross-cutting: the runtime null representation already exists
`/0→null`, overflow→null, OOB→null, and the scalar null sentinels are **already in `fill.rs`** —
so Phase 3 is mostly *type-level* work (carry `τ?`, enforce discharge) plus the **one new runtime
op** (the checked cast `as τ?`). We are not inventing runtime null; we are typing the null that
already happens and making the unhandled case a compile error.
