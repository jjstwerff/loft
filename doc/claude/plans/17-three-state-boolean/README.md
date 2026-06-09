<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 17 — Three-state boolean (true / false / null)

## Status

Open — design ready, no implementation yet.  `boolean` is today the **only**
common-value scalar whose zero-value collides with its null sentinel (the null
sentinel for `boolean` *is* `false`).  Every other scalar — `integer`, `float`,
`text`, plain `enum` — distinguishes "zero value" from "absent".  This plan makes
`boolean` join that model: three-state in data (false / true / null) unless a field
is `not null`, with `null` collapsing to `false` at the single boolean-logic
chokepoint.  Tracked as [@PLN17](https://github.com/loft-lang/plans/issues/17).

## Goal

A nullable `boolean` distinguishes `null` from `false` everywhere it is stored,
copied, or compared, and collapses to `false` only when consumed as a truth value —
so a `hash`/`index` map to `boolean` can express absent vs false vs true.

## Effort + design

- **Effort:** M — touches the null model; the risk is *coverage of the coercion
  sites* and the native-marshalling boundary, not representation room.
- **Design:** ~ (partial) — the invariant is clear and three claims are already
  confirmed by code-read; the remaining load-bearing claims need falsification
  probes (Stage A) before any code.
- **Last touched:** 2026-06-09

## The invariant (Design Protocol 1, step 1)

> A `boolean` value that is not `not null` has **three runtime states** —
> `false` = byte `0`, `true` = byte `1`, `null` = byte `255` — reusing the
> byte-storage sentinel scheme plain enums and narrow ints **already** use.  The
> third state is **preserved** by storage, assignment, copy, `==` / `!=`, and
> `== null`; it **collapses to `false`** at the *one* truthiness chokepoint that
> every forced context routes through.

Why this is a *consistency fix*, not a new feature: a `boolean` is stored in one
byte (`data.rs:1055` — `Type::Boolean | Type::Enum(_, false, _) => 1`), the same
storage class as a plain enum, and byte `255` is *already* the universal null
sentinel for that family (`store.rs:1756`, `fill.rs:1221`).  The third state
physically exists and is reserved; boolean is the lone type whose read/compare path
flattens the byte to a 2-state Rust `bool`.

## Re-assertion sites — the prospective tell (Design Protocol 1, step 2)

The design is correct **iff** `null → false` is enforced at *one* chokepoint, not
re-stated per context.  Every "forced context" must route a value through the same
truthiness coercion:

`if` · `while` · `assert` · `&&` · `||` · `!` · for-`if` filter · match guard ·
ternary-style `if` expression.

If each compiles its own "is-this-true" test, that is **N silent re-assertion
sites = the brittleness, known now**.  The cure is to confirm (or build) a single
coercion every site emits.  Early read: `if` lowers to `OpGotoFalseWord`
(`codegen.rs:737`) whose impl `goto_false` reads the byte as `bool` (`fill.rs:302`,
`!= 0`) — so **255 currently reads as `true`**.  The coercion belongs at this op and
its peers; Stage A must enumerate every consumer of a boolean-as-truth-value and
prove they reduce to one site (or a small, named set), **no narrower, no wider**.

## Composition matrix — Stage A (REQUIRED, before any code)

Write these as `/tmp` probes on `--interpret` first; the feature is done only when
every cell is green on **both** backends and the probes graduate to
`tests/scripts/`.  Axes: **value** `{false, true, null}` × **context**.

| Context | false | true | null | Expected after fix |
|---|---|---|---|---|
| `if x` / `while x` / `assert(x)` | skip | run | **skip** | null coerces to false |
| `!x` | true | false | **true** | `!null` = `!false` = true |
| `x && y`, `x \|\| y` | logic | logic | **false-coerce** | coerce-at-context, not Kleene |
| `x == false` | true | false | **false** | null distinguishable (the point) |
| `x == true` | false | true | **false** | " |
| `x == null` | false | false | **true** | the null test |
| `x == y` (bool == bool) | raw byte | raw byte | raw byte | C4: distinguishability free if raw compare |
| stored field read (nullable) | false | true | **null** | round-trips 255 |
| stored field read (`not null`) | false | true | n/a | unchanged — 2-state |
| nullable field **default-init** | — | — | **null** | flips from today's `false` |
| `hash`/`index`/`sorted` map → bool | false | true | **null** | the real-consumer trigger |
| `{x}` format / `{x:…}` | "false" | "true" | **?** | decide null rendering |
| native `#rust fn(bool)` / `-> bool` | false | true | **false at boundary** | marshal coercion |
| `vector<boolean>` element | false | true | **null** | byte-packed element round-trip |
| closure capture of nullable bool | false | true | **null** | capture preserves third state |

Extract the **real-consumer probe** verbatim from the `hash → boolean` shape the
agent's confusion pass hit (issue body) — real extraction catches classes the
synthetic cells miss.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A** — Stage A matrix | probes for every cell above, both backends; record current vs expected | Open |
| **B** — representation | nullable bool field/element default-inits to `255`; storage + stack round-trip `255` (C5/C6) | Open — blocked on A |
| **C** — truthiness chokepoint | `null → false` at the one forced-context site set (`OpGotoFalseWord` + peers); prove `&&`/`\|\|`/`!`/match-guard route through it (C3) | Open — blocked on A |
| **D** — comparison + null test | `==`/`!=`/`== null` distinguish `255` (likely free if raw-byte — C4) | Open — blocked on A |
| **E** — native marshalling | `#rust` `bool` params/returns coerce `null → false` at the boundary | Open — blocked on A |
| **F** — format rendering | decide + implement `{nullable_bool}` output | Open |
| **G** — backward-compat scan | grep suite + stdlib for `== false` / default-false reliance; fix or document | Open |
| **H** — docs + graduate | LOFT.md null table; graduate probes to `tests/scripts/`; record `&&`/`!` decision in `DESIGN_DECISIONS.md` | Open — last |

## Phase ordering

1. **A first** — the matrix is the spec.  Map current behaviour at every cell on
   both backends; the diff is the work list.  Do not design B–F until A is read.
2. **B + C together** — representation and the coercion chokepoint are the core; a
   `255` that stores but reads as `true` is worse than today.  Land both or neither.
3. **D** — should be near-free if `==` is a raw byte compare (confirm in A).
4. **E** — the native boundary is the most likely wrong-result (not clean-break)
   site; audit every `#rust` signature touching `bool`.
5. **F + G** — rendering + the compat scan; G gates the release call.
6. **H** — docs, regression graduation, decision record.

## Open design questions

1. **`&&` / `||` / `!` — coerce vs Kleene.**  Recommend **coerce-at-context**
   (`null → false`): simpler, matches "forced context", and keeps `if x`/`if !x`
   backward-compatible.  Kleene three-valued logic (null propagates as "unknown")
   is far more surface and contradicts the framing — decline unless A surfaces a
   real consumer that needs it.
2. **`== false` distinguishing null.**  Under a raw-byte compare `null == false` is
   naturally `false` — keep it (it *is* the distinguishability win); `== null` is
   the null test.  Decision needed only if A shows `==` is not a raw compare.
3. **Default-init flip under feature-freeze.**  Nullable bool field default flips
   `false → null`.  `not null` fields are unaffected (the escape hatch).  G's scan
   sizes the blast radius; the truthy idiom (`if field`) is preserved regardless.
4. **`{nullable_bool}` rendering.**  Likely `"null"` (mirroring other nullable
   scalars) — confirm against existing `{nullable_int}` behaviour in A.
5. **Stack width.**  A local assigned from a nullable source (`b = h[k].flag`) must
   carry the third state — confirm the stack slot round-trips `255` (C6).

## Over-unification guard (Design Protocol 1, step 4)

The cleanest claim — *"boolean becomes exactly a 2-variant plain enum, so it's all
free"* — is the one to attack.  Enums have **no** `&&` / `||` / `!`, no native
`bool` marshalling, no `{b}` truthy formatting, and are not the canonical `if`
subject.  Each of those is a site the enum analogy does **not** cover; the matrix
(B/C/E/F rows) is exactly the set of operations boolean has that enums don't, and
the build is what proves the chokepoint actually covers them with one mechanism.

## Cross-arc dependencies

- **`plans/1-integer-width-discipline/`** — sibling null-model / S-tier plan; same
  "make a scalar's null discipline consistent" flavor.  No code dependency; shared
  reviewer context.
- **`DESIGN_DECISIONS.md` § C69** (`!x` is a null test on non-booleans) — adjacent,
  **no conflict**: this plan touches boolean `!`; C69 governs non-boolean `!`.
  H must record the boolean-`!` semantics so the two read as one coherent story.
- The S-tier **collection-validation** plan (`plans/future/20`) overlaps the
  `hash/index/sorted → bool` matrix rows — coordinate the keyed-collection cells.

## See also

- `doc/claude/LOFT.md` § Null representation — the sentinel table this plan changes.
- `doc/claude/DESIGN_PROTOCOL.md` — the design discipline this README applies.
- `doc/claude/plans/README.md` § The composition axes — what the Stage A matrix varies.
- [@PLN17](https://github.com/loft-lang/plans/issues/17) — the tracker issue (identity).
