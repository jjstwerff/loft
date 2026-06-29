<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 dense-default — RESUME HERE (cold-start handoff)

Single resume point for the dense-default value-model rewrite. Written so a fresh
session can pick up after a `/clear`. **Read order:** this file →
[full-design.md](full-design.md) (the consolidated design) →
[storage-vs-access-nullability.md](storage-vs-access-nullability.md) (the invariant +
probe verdicts) → [implementation-steps.md](implementation-steps.md) (the phase order) →
[`../../formal/types.md` § Nullability](../../formal/types.md) (the `N-*` rules).

---

## TL;DR — where we are (updated 2026-06-29)

- **Branch: `lima-default-borrow-elision`**, off `main`, **pushed**. 6 `@PLN25` commits
  ahead of `origin/main`, **no PR** (the single-final-PR discipline holds).
- **Suite is fully green: `2564 passed, 0 failed`** (184 skipped) — verified
  2026-06-29 via `find_problems.sh`. The branch is in a clean, mergeable-at-a-phase-boundary
  state.
- **The vectors half is DONE and on `main`.** `vector<S>` is dense (`main_vector<S>`,
  no `__nullable`); `vector<S?>` is the nullable opt-in; `v[1] == null` is true; the
  canonical incoherence probe is coherent on both backends. The #465 borrowed-view
  over-free is fixed. Merged via `#412` (gate flip + keyed-dense), `#467` (dense
  vectors + copy-vs-borrow elision, vectors-green checkpoint), `#468` (borrow elision
  default-on + Tier-1.5).
- **The scalars + TIGHTEN half is IN-FLIGHT on this branch:**
  - **Scalars Phase 0 (EXPAND) — done.** `integer?` / `text?` / `S?` parse in every type
    position (decl, param, return, `as`-cast, nested). Today a no-op (plain types are still
    nullable). Regression: `tests/scripts/25-scalar-optional-syntax.loft`.
  - **DN4 (narrowing-cast enforcement) — done, default-ON** (opt-out `LOFT_NO_DN4`,
    `operators.rs:1674`). A narrowing integer cast of a not-provably-fit value is a compile
    error; `as τ?` is the checked form. DN4 is integer-range-only, needs none of the scalar
    `τ?` representation, so it **shipped ahead of the scalar default flip**. The error
    baseline caught a real silent overflow (`big as i32` was `10000000000`). Regression:
    `tests/dn4_cast.rs` (3) + the `tests/scripts/389-narrow-*` family.
  - **N-Arith range-tracking — done.** `&` and `%` narrow the static range so masked/modded
    values are provably-fit (feeds DN4's fit proof). Regression: `389-narrow-alias-ranges`,
    `389-narrow-vector-full-range`.
  - **Scalar `τ?` representation — DECIDED, NOT YET BUILT.** `Type::Optional(Box<Type>)`,
    compile-time only (storage stays sentinel-based). The design is in
    [scalar-optional-representation.md](scalar-optional-representation.md); the variant is
    **not in `src/data.rs` yet** — it is the first build step of the scalar half.
- **gridmesh 0.1.2 published (2026-06-29).** The DN4 cutover masked the gridmesh + hex_world
  fixtures and relocked `audience_crystal` to gridmesh 0.1.2; the suite was RED until that
  version was signed into the registry index. It is now published + signed (registry commit
  `056c08c`), which is why the suite is green.

## The one invariant (what the whole rewrite installs)

> `vector<τ>` is dense and uniform for every `τ` (incl. generic `N`); nullability is
> carried only by an explicit `τ?`; lookup-partiality only by the fallible ops
> (`v[i] ⇒ τ?`, etc.). No implicit container rewrite, no implicit unwrap. (The integer
> model applied to null — one former, representation derived.)

---

## Landed ledger (what is true on this branch, both backends)

| Area | Phase | State |
|---|---|---|
| Vectors `vector<S?>` | 0–2 EXPAND/MIGRATE/CONTRACT | ✅ on `main` (`#467`) |
| Borrow elision Tier-0 + Tier-1.5 | — | ✅ on `main`, default-on (`#468`) |
| #465 borrowed-view over-free | 4 | ✅ on `main` |
| Scalars `integer?`/`S?` syntax | 0 EXPAND | ✅ this branch (no-op) |
| DN4 `as τ` fit-check / `as τ?` | 3 TIGHTEN (early) | ✅ this branch, default-on |
| N-Arith `&`/`%` range-tracking | 3 support | ✅ this branch |
| Scalar `τ?` = `Type::Optional` | 0/2 prereq | 🔵 decided, **not built** |
| Borrow elision Tier 1 (local source) | — | 🔵 implemented, **parked off** by design |

---

## NEXT STEPS — concrete, in order, each with its validation gate

The remaining critical path to "done" (full null-model coherence) is the **scalars half**,
then **Phase 3 DN3/DN2**, then **Phase 5 cleanup**. Each step ends green; never carry two
phases' breakage at once.

### Step 1 — Build `Type::Optional(Box<Type>)` (the scalar `τ?` representation)
The Phase-2 blocker. A single compile-time optional former (storage stays sentinel-based →
zero runtime cost). A new `Type` variant is chosen precisely so every unhandled `match`
becomes a **loud compile error** (vs a silently-ignorable `nullable: bool`).
- **Where:** `src/data.rs` (the `Type` enum) + every `match Type` site the compiler flags.
  Reconcile with the existing `IntegerSpec.not_null` (`scalar-optional-representation.md`).
- **Validation:** additive only — `Optional` is constructed by nothing yet, so the gate is
  **suite stays at 2564/0** on both backends. The win is purely that the type *exists* and
  `cargo build` is clean (every `match` arm handled).

### Step 2 — Scalars Phase 1 MIGRATE: annotate nullable scalar/field sites with `?`
While the scalar default is STILL nullable, mark every site that genuinely holds null.
Under today's default these are **no-ops** — that is the point: pre-position them before the
flip can surprise them.
- **Survey first (cheap):** `grep` scalar/field `= null`, `?? `, `for x in …` null-use across
  `default/*.loft`, `lib/`, `tests/`, and the consumers. The vector survey found ~0 sparse
  sites in crawler; scalars need their own count — **record the number** (it is the Phase-2
  blast-radius estimate).
- **Validation:** **suite stays 2564/0** after annotation (no behaviour change). Annotating an
  already-nullable site changes nothing.

### Step 3 — Scalars Phase 2 CONTRACT: flip the scalar/field default to non-null (DN1)
The default flip. `IntegerSpec.not_null` default `false → true` (and the bool/char/text
analog); the plain-type parse stops meaning nullable; `not null` becomes an **accepted no-op**
(DN1).
- **Where:** `src/data.rs` (`not_null` defaults at the `IntegerSpec` constructors, lines
  ~97–167) + `src/parser/definitions.rs` (the scalar `not null` parse, ~line 1470 — consume +
  set nothing).
- **Validation:** the **only** breakage is Phase-1 misses — a nullable site that Step 2 didn't
  annotate now errors; fix = the one-character `?`. Gate: run `find_problems.sh --bg`, sweep
  each miss to `?`, re-run to **2564+/0** on both backends. Measured, bounded, each one
  character.

### Step 4 — Phase 3 TIGHTEN, the rest: DN2 then DN3 (the measured blast radius, LAST)
DN4 already shipped (above). Remaining, least-to-most breaking:
- **DN2** — remove the implicit `τ? ⤳ τ` unwrap in `convert()` (`parser/mod.rs:1585`). After
  this, `??` / `match` are the only ways down from `τ?`. Breakage: code relying on silent
  unwrap.
- **DN3** — type fit-failing ops as `τ?` (`/`, `%`, `[]`, `parse`, overflow) and make
  `(N-Store)` reject an un-discharged `τ?` into non-null storage. **Biggest blast radius** —
  every `b = a / x` without a `??`. The runtime already nulls (`fill.rs`); DN3 is type-level
  (carry `τ?`) + the discharge check.
- **MANDATORY — measure before DN3 lands:** count sites assigning a fit-failing result into
  non-null storage without `??` (the gating number). Migrate with `?? d` / `as τ?` / a mask.
- **Validation:** green after the discharge migration, both backends. This is the
  "willing-to-rewrite-tests" step — it lands last, after everything else is green, with the
  blast radius counted first.

### Step 5 — Phase 5 CLEANUP: retire `not null`  (ordering is load-bearing)
By now `not null` is a no-op everywhere. Remove it — **in this order**, or the **1015
occurrences across 300 `.loft` files** all become parse errors at once:
1. **Strip `not null` from all `.loft` source** (stdlib, `tests/**`, `lib/` + consumers).
   Mechanical, per-area, re-running the targeted suite after each — behaviour-preserving since
   it is already a no-op.
2. **`grep -rn "not null" --include='*.loft'` is clean** (0 occurrences) — the gate before
   touching the parser.
3. **Remove `not null` from the parser** (`definitions.rs`: scalar ~line 1470, the vector arm's
   `has_keyword("not")`, the keyed arms). After this `not null` is a **syntax error** — the
   retirement is enforced, not conventional.
4. **Docs/skills** — drop `not null` from `LOFT.md`, the `loft-write` skill, `formal/types.md`
   notation, examples. `?` becomes the only nullability marker.
- **Validation:** steps 1–2 keep the suite green throughout; step 3 only "breaks" code step 2
  proved nonexistent. End state: one nullability notation (`?`), no non-null marker.

### Step 6 — Land it: single PR to `main`
Per the `@PLN25` decision, the rewrite reaches `main` via **one final PR** when E2 is fully
coherent (scalars done + Phase 3 + Phase 5). The finishing PR carries `Closes @PLN25`. Before
opening: `git fetch`, confirm the head is a descendant of `origin/main`, full
`find_problems.sh` green on both backends.

---

## Parallel sub-thread (NOT on the critical path) — copy-vs-borrow elision

The performance face of the dense model ([copy-elision-design.md](copy-elision-design.md),
Cluster C / `OWNERSHIP_MODEL.md`). Tier-0 + Tier-1.5 are **default-on, on `main`**.
- **Tier 1 (local-struct source) is IMPLEMENTED but parked off** (`LOFT_ELIDE_T1`,
  `use_analysis.rs:423`). The design's own conclusion (2026-06-27 crawler dogfood):
  **do NOT cut it default-on** — it adds only 2 cold-path borrows, no measured local-sourced
  hot copy exists to capture; keep it as a turn-on-and-compare flag until a consumer surfaces
  one. So this is a *deliberate stop*, not unfinished work.
- **Tiers 2–3** (mutable source; unify assignment + return delivery onto one
  `materialization_mode` predicate — where this meets #465 / Cluster C) are **design-only,
  gated behind a measured need**. Do not build speculatively.

---

## Verify / probe commands

```sh
# dense default + nullable opt-in (the core correctness, already green):
loft --interpret /tmp/a.loft               # vector<S?>: v[1]==null -> true
loft introspect /tmp/a.loft | grep main_vector   # vector<S> -> main_vector<S> (dense)
# DN4 enforcement (default-on):
echo 'fn main(){ x = 400 as u8; print("{x}"); }' > /tmp/dn4.loft
loft --interpret /tmp/dn4.loft             # -> compile error: use `as u8?`  (LOFT_NO_DN4 opts out)
# full baseline (expect green: 2564/0):
./scripts/find_problems.sh --bg ; ./scripts/find_problems.sh --wait
```

## Facts a fresh session needs

- **`Type::Optional` is not in `src/data.rs` yet** — Step 1 builds it. The commit
  `885eab1d` only *decided* the representation.
- **The scalar default is still nullable** (`IntegerSpec.not_null` defaults to `false`,
  `data.rs:97–167`). Step 3 flips it. DN4 shipped before this because it is integer-range-only.
- **Family A** (`?? <vector-literal>` codegen panic) is fixed on `main` (`6ea779be`). Do NOT
  re-investigate.
- **#465** (dense borrowed-view over-free) is **fixed** and on `main` — it is no longer a
  blocker; the Tier-3 elision unification is its forward-looking sibling, not a re-fix.
- **Branch discipline:** keep `main` off this branch until the scalars half is green at a phase
  boundary; reach `main` via the single final PR (`Closes @PLN25`).
- **Do not build on `2026-07-mac`** — the abandoned enum-synthesis approach, long stale. The
  live `LOFT_E2_SYNTH` references are vestigial from it; the dense-default design (this branch)
  superseded it.
