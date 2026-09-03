<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 2026-09-03 — the bind/copy thread on `tuxedo-stability-impact`

One session's work on branch `tuxedo-stability-impact`, all of it pushed. Written so the
thread can be picked up cold. The through-line: **four issues that are one mechanism** — a
site deciding a shape by matching the `Type` (or `Value`) variant BARE, so a wrapped or
branched form reaches none of the paths written for it and the default stands.

## What landed, and what each cost

| issue | what it was | state |
|---|---|---|
| **#1316** | `reference<T>?` was silently an INLINE COPY, and a cyclic one failed layout | FIXED, both backends |
| **#1315** | `revalidate-libs` could never print `0 COMPILE-BREAK` | FIXED, gate green |
| **#1319** | a NULLABLE whole-value bind ALIASED its source | FIXED, both backends |
| **#1321** | a JOIN right-hand side ALIASES its source | **attempted, REVERTED** — see below |

Cherry-picked from `../loft` (`loft-b9`'s ownership/closures thread), all green here:
`25be4533`, `e949f943`, `f992fb05`, `c6239cbf`, `7c92d713`, `29743c5c` — D-clo-14, D-own-8
Face A, D-clo-7, and the register corrections that follow them.

## The mechanism, stated once

`τ?` is `Optional(τ)` — the same storage behind a nullability marker (`@FR-L-Null`). A site
that matches `Type::Reference(..)` or `Type::Vector(..)` without `base()` answers "no" for the
wrapped form, and whatever the site was choosing does not happen. The register has now
recorded this five times (D-layout-2, D-layout-3, D-layout-4, D-bind-13, D-bind-15), each time
closing on "all N sites", each time with siblings left. **A census of the parser plus
`typedef.rs` finds 103 bare `Type::Reference` matches, 21 of which read a field's declared
type** — that number is a QUEUE, not a defect count (most read the synthetic `__nullable<S>`'s
own payload, where no `Optional` can arrive), but it is the population the next instance comes
from.

## #1316 — a `?` on a pointer field

`@FR-L-Null` vs `@FR-L-Null-Tag` divide on a property of the type: a τ that reserves a null
VALUE keeps its bytes, and only a struct stored INLINE needs a tag. A stored reference
reserves `nullref`. The field rewrite gave it the tag anyway, so `reference<Leaf>?` and
`Leaf?` laid out identically — the `?` silently replaced a shared pointer with a copy (`11`
where a pointer prints `22`), and on a self-referencing struct the inline form has no finite
size, so a linked list's terminator could not be declared at all.

`@FR-L-Null-Which` is new and records what decides the split (the `u16::MAX` share marker);
`synth_nullable_struct_fields` is its one home. loft#1313's suppression is deleted with it,
and the notice it unblocked had the same defect one layer up — `Type::name` renders a pointer
field as the bare struct name, so the cure read `Node?`, which does not compile. Also closed
`@FR-B-Ref-StoredRef`'s field-order half (D-bind-14): the `&` gate accepted only `;`/`}`, so
`Trail { link: &pool[0], id: 7 }` was refused for its comma.

## #1315 — an instrument whose zero was not zero

Not a design gap: the matrix policy was written TWICE, and the local script was missing the
workflow's skip of the `loft` package (the compiler is not a library revalidated against
itself). Three quieter disagreements came out with it — the known-broken map, the `subpath`
default, and whether a YANKED version may be validated (it may not; the workflow had no
filter). `scripts/revalidate_matrix.py` is the single source both read, with a `--self-test`
wired into `make ci` that gives each rule an input it must act on.

## #1319 — a nullable bind aliased

Four sites matching the variant bare. The second half is the one to remember: **a copy must
not turn ABSENCE into EMPTINESS.** A null source has to leave the destination null, not
holding the store the copy allocated for it. Both mechanisms already existed —
`Stores::vector_replace` gained the guard `replace_keyed` has carried since loft#1150, and the
record bind routes through `OpBindOrCopy` with the source as its own witness.

`OpCopyRefOrNull` was tried first and is WRONG here: it binds `Stores::null()`, whose
`store_nr` is a REAL slot with `rec == 0`, while `x == null` on a record lowers to
`OpRefIsNull`, which tests `store_nr == u16::MAX`. Two spellings of absence that agree for the
element read it was written for and not for a bound local.

The filed matrix's `??` column turned out to be a DIFFERENT defect — measured, not assumed —
and is #1321.

## #1321 — open, and this is the state to resume from

`b = if c { a } else { [0, 0] }` ALIASES `a`, on both backends and on the shipped 2026.8.0.
`match` arms and `x ?? d` lower to the same shape. It is the COPY face of the binding whose
FREE face `loft-b9` closed in #1320; their fix does not touch it, measured.

**The rule is right and was implemented: a join is read ARM BY ARM** — copy where every arm
would have copied on its own, keep the view where some arm NAMES a place a view rule exempts.
Every matrix cell went green on both backends. It was then **reverted**, for one reason:

> **a CALL arm may return a BORROW.** `it = get(b) ?? d`, where `get` answers a view of its
> parameter, has no syntactic projection in any arm — so a walk over the IR shape says
> "copy", the caller owns the copy, and frees at scope exit what it borrowed. That is
> loft#974's guarded behaviour, caught by
> `accessor_borrow::an_accessors_returned_view_names_its_parameter`. Trading a silent alias
> for a wrong free is not an improvement.

**What the next attempt should do differently.** Not a syntactic walk over the IR. The
question is *does this arm's VALUE borrow?*, and the return TYPE already answers it — loft#974
is the change that put the dep there (`-> Y974?["b"]`). An arm whose type carries a dep keeps
its view; an arm whose type owns may be copied. That reads the same fact the oracle reads, one
level down.

Three more facts it will need, established and worth not re-deriving:

1. **Reading the JOIN rather than its arms is wrong.** `v[i]` is `τ?`, so `c = vv[0] ?? [0]`
   is a branch; copying it makes `@FR-B-View-Depth` unreachable for its own spelling, and
   `bind-copies-or-views-the-whole-boundary.loft` goes red on the cell that says so.
2. **A `??` HOISTS its subject into a temp** (`__ncc_N = vv[0]`), so the arm the walk reaches
   is a bare `Var` and the projection is one statement up.
3. **`use_analysis::is_projection_op` does not name `OpGetVectorNullable`**, while
   `generation::hoist::ELEMENT_ADDRESS_OPS` pairs it with `OpGetVector` for the same question.
   Two one-homes, one notion. Widening `is_projection_op` moves the ownership analysis at
   eight other sites — its own change, its own matrix.

And two facts that stay true: the destination's inherited **dep** is what closes every copy
path downstream (`owned_ref` requires `depend().is_empty()`, so the `OpBindOrCopy` arm written
for this shape is unreachable), and the **ownership oracle already reports the joined binding
`Owned`** while the shipped dep says borrowed. The analysis has the right answer; the shipped
fact does not.

**A decision that is deliberately NOT taken.** `b = if c { vv[0] } else { [0, 0] }` is a view
on one path and an owner on the other — the shape D-own-8 calls a defect. Making it a copy
means deciding that a branch produces a VALUE, which changes what `@FR-B-View-Depth` reaches
for a shipped spelling: a COMPATIBILITY decision with its own gate, not part of a bug fix.
Both agents working today reached the same reading; neither took it.

## Still open, unfixed, and not mine

`#1318` (a fn-ref `??` call with a call-result argument frees the caller's container after one
iteration — **a regression since 2026.8.0**, `silent-wrong`) is unowned and is the most
important of these. `#1322`, `#1323`, `#1324` are `loft-b9`'s filed-not-fixed side findings.

## Method notes worth keeping

* **The oracle is the reference route.** Twice this session the ownership oracle already had
  the right answer while the shipped fact did not. Ask `loft introspect --show-ownership`
  beside `LOFT_VAR_TABLE=<fn>` before theorising.
* **A filed issue is a hypothesis.** #1316's stated cause ("the `Optional` wrapper loses the
  marker") was wrong — the marker was intact. #1315's "the choice is a design call" was wrong
  — the call was already made, in the workflow. #1319's `??` column was a different defect.
  All three were settled by one measurement each, before any edit.
* **The falsification ratchet works.** `tests/falsified.baseline` caught the boundary file
  recording a control while still listed; the row had to be deleted for `make ci` to pass.
* **Poll loops match themselves through their LOG FILENAMES.** Two `until ! pgrep -f
  "[f]alsify"` waits spun forever because each other's command line contained
  `falsify1321.log`. The bracket trick does not protect against that.
