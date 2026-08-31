# formal/collections-history.md — the deviation register for [collections.md](collections.md)

> **The rules are next door.**  [collections.md](collections.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

- **`C-Order`** (hash bucket-walk) — already a decided edge in concurrency.md; `Col-Order` references it.
- **`D-key-1`** (keyed slice = iterator) — a shipped decided edge (the value-position crash was fixed to a
  clean diagnostic, RELEASE.md 2026-07-04); formalized as `INV-KeyedSlice`, not an open deviation.
- **INV-Superset** — a deliberate design decision (raw Morton interval), not a deviation; record as an edge
  with a DESIGN_DECISIONS cross-link.
- **Candidate OPEN (verify):** the per-query scratch-vector allocation for spatial slices (CAVEATS.md notes
  it as the next efficiency lever) — a performance note, likely NOT a formal deviation.

OPEN: **0** — `D-col-null` was opened and CLOSED the same day (2026-08-28, below).

### `D-col-null` — OPENED AND CLOSED (2026-08-28, loft#1120): two answers to *"is this collection null?"*

`(Col-Lookup)` and `(N-Index)` make an absent element that type's null, and `(E-Coalesce)` makes
`e ?? d` yield `d` for exactly that null.  One value, one null, one answer — and the tree carried
two, each right about the half the other got wrong.

`??` asked `OpConvBoolFromRef` (`rec != 0`).  That reads the encoding a MISSED LOOKUP uses and
nothing else, so a nullable collection FIELD — whose read is a sub-reference carrying the HOLDER's
record — was "present" whatever the slot contained: the default was unreachable, and a `hash` /
`index` field then dereferenced the record the absent slot names and stopped the run.  `==  null`
asked `OpVectorIsNull`, which reads the handle sentinel and the slot word but called a record-less
DbRef present, so `vv[9] == null` answered `false` for an index plainly out of range.  `spatial`
and `trie` were in neither list: the coalesce's hand-written variants named `Vector`/`Sorted`/
`Hash`/`Index` only, so they fell to the generic convert, which hands back the bare handle —
`--interpret` read twelve pointer bytes as a boolean and `--native` would not compile the `if`.

Closed by giving the question ONE implementation: `vector::is_absent_collection` answers ABSENT for
a DbRef that reaches no slot (the missed-lookup encoding it used to call present), and the coalesce
asks `Parser::collection_is_null` — the lowering `== null` already used — through
`is_collection_type`, which names every kind including `Radix` and `Trie`.  The condition position
(`if c`) shares that lowering and was wrong in the same three ways.

⚠ **The oracle under the neighbouring `OPEN: 0`s could not see this.**  Five guards already covered
nullable collection fields (`909`, `917`, `920`, `922`, `936`) and every one of them writes `?? []`
— and empty is what the wrong answer looks like, so each cell agreed with itself.  A default whose
length differs from both the empty and the present arm is what separates them; that is what
`tests/scripts/1120-one-null-question-for-a-collection.loft` writes, over six collection kinds ×
{null, empty, filled} × {field, element field, parameter, handle, lookup} × {`??`, `== null`, `if`}.
