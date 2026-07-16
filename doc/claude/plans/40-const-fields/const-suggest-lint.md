<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Enhancement — library-scoped `const`-field suggestion lint

**Status: design, not started.**  Spun off from @PLN40 + the const ecosystem
uptake.  Should become its own plan / `loft-lang/features` issue before build.
The **second half of the "const ergonomics" arc** — pairs with
[literal-expected-type.md](literal-expected-type.md).

## Goal

A lint that finds struct fields which are only ever set at construction and
suggests marking them `const` — automating the manual survey + agent uptake that
const-hardened the libraries.  **Library-scoped**: aggressive on library packages,
absent on application programs (owner policy: const matters for libs, not for
throwaway app code — see the `libs-maximize-const-fields` memory).

## The candidate rule (grounded in the verified enforcement semantics)

A struct field is a **const-suggest candidate** iff it is never the target of a
field-WRITE anywhere the compiler can see, and it is eligible.

A "field write" — the thing that disqualifies — is exactly what `validate_write`
(`src/parser/expressions.rs:3449`) already rejects for a const field:

| Form | Disqualifies? | Why |
|---|---|---|
| `t.f = …` (reassign) | **yes** | rebinds the field |
| `t.f += […]` (append / compound) | **yes** | const rejects it (verified — `+=` lowers through the write path) |
| `t.f[i] = …` (element/index write) | **no** | mutates contents, allowed under const |
| `T{ f: … }` (construction) | **no** | the write-once site |

So a `+=`-grown accumulator (`Args.options`, `Scene.meshes`, `CellSnap.*`) is **not**
a candidate under current semantics.  (If the open `+=`-as-contents design question
is resolved to ALLOW append on const fields, this rule widens — see Dependencies.)

**Eligible** = not already `const`, not `virtual`/`computed` (`Attribute.constant`),
not a key field (`!mutable`), not a hidden/synthetic field.

## The one insight — reuse the enforcement chokepoint

`validate_write` already visits **every** field write.  The lint is the *complement*
of what const enforcement rejects: accumulate the set of `(struct, field)` pairs
ever written via `=`/`+=`; any eligible field NOT in that set is a candidate.  Same
data, same chokepoint — the suggestion and the enforcement can never disagree.

The dataflow for "written anywhere in the unit" already exists: the ownership oracle
(`src/ownership_cfg.rs`, `src/use_analysis.rs`, @PLN94) and the write-site visiting
in `validate_write`.  Reporting joins the existing lint family
(`variables/mod.rs::test_used` unused-vars, `use_analysis::dead_store_accesses` the
@PLN107 dead-store lint) — all `Level::Warning`, "evaluated not asserted".

## The soundness gate (why it is a suggestion, not an auto-fix)

A `pub` struct's field can be reassigned by a **consumer the compiler is not
looking at**.  Single-unit analysis cannot prove a pub field is never written
elsewhere, so suggesting `const` on it is unsound *to auto-apply* — accepting it
could break a consumer (the cross-library version of `hex_world.world_load`).

Resolution: the lint **suggests** (a warning); the author judges.  For a library
that is exactly right — the author owns the contract and knows whether a consumer
mutates the field.  Never auto-rewrite; never hard-error.

## Safe small steps

| # | Step | Verify | Why safe |
|---|---|---|---|
| 0 | **Cross-check the analysis against the survey.**  Build a read-only "written-fields" collector (reuse `validate_write`'s visiting to accumulate `(struct, field)` write targets, excluding index writes).  Its complement = candidate fields. | Run it on `hex_world`, the net libs, `hex_terrain` — the candidate set must MATCH what the manual survey/agents found (net = every field; hex_world.Cell; etc.). | No code emitted — pure analysis; the survey is the oracle |
| 1 | **Candidate pass behind a flag.**  Compute eligible-never-written fields; store the list.  Nothing reported yet. | Builds; suite green (inert). | Gated, reports nothing |
| 2 | **Emit suggestions as warnings** (opt-in `loft lint --suggest-const` / `LOFT_SUGGEST_CONST`).  Message: `field 'x' of struct 'T' is never reassigned — consider marking it 'const'` at the field-decl position. | On a lib with known candidates → warns exactly on them; on an already-const'd lib → zero; positions correct. | Opt-in, advisory; can't break a build |
| 3 | **Library-scope it.**  Fire only when the package being compiled is a library (`loft.toml [library]` / `build_phase`), not for an application entry point. | Absent on an app program; present on a lib. | Scoping only narrows where it fires |
| 4 | **Noise controls.**  Exclude already-const / virtual / key / hidden fields (step-0 eligibility).  Add a per-field opt-out marker for an intentionally-mutable-but-currently-set-once field (so a lib author can silence a false positive without adding `const`). | A silenced field stops warning; a genuine candidate still warns. | Suggestions only; author retains control |
| 5 | **Default-on for library builds** (owner policy).  Start opt-in, soak on the already-const'd libs (should be near-zero warnings after the uptake), then flip default-on for library packages with an opt-out env (`LOFT_NO_SUGGEST_CONST`). | Library `make ci` shows the expected (small) suggestion set; app builds unaffected. | Flip only after step 0–4 prove the candidate set is accurate and quiet on const'd libs |
| 6 | **Wire into authoring.**  Add "const every never-reassigned field" to `LIBRARY_CHECKLIST.md`; optionally a CI advisory gate for library repos. | Checklist + advisory check present. | Docs / advisory only |

## Dependencies / open questions

- **Pairs with [literal-expected-type.md](literal-expected-type.md).**  Accepting a
  suggestion can force a construct-then-fill → literal refactor that hits the
  literal-field type-inference gap.  Land that enhancement first so "accept the
  suggestion" stays frictionless.
- **The `+=`-on-const design question.**  Const currently rejects `t.v += […]`
  (append).  If that is changed to allow append as contents mutation, the candidate
  rule in this lint widens to include `+=`-grown accumulators — which are common
  library builder fields, so it materially raises how much const a lib can carry.
  Decide that BEFORE step 5 (default-on), so the suggestion set is stable.
- **Whole-program mode (later).**  A registry-wide pass could prove a pub field is
  never written by ANY consumer, upgrading the suggestion from "author judges" to
  "provably safe" — out of scope for v1.

## See also

- [README.md § Consumer survey](README.md) — the manual survey this lint automates
- `src/parser/expressions.rs:3449` (`validate_write`) — the write chokepoint to reuse
- `src/variables/mod.rs::test_used`, `src/use_analysis.rs` (`dead_store_accesses`) — the lint family to join
- the `libs-maximize-const-fields` owner-policy memory (also → `LIBRARY_CHECKLIST.md`)
