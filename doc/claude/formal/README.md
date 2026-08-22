<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ — loft's strict formal definition (rules + tracked deviations)

This directory is the **strict** formal definition of loft. Each document covers one
area and has exactly two parts:

1. **Rules** — the formal definition we *want*: the judgments / relations / grammar
   loft is meant to satisfy, written tightly enough to be checked against. This is the
   **target**, not a description of today's code.
2. **Deviations** — every place the *current implementation* breaks a rule above,
   numbered (`D1`, `D2`, …), each with: the rule it violates, where it lives, the
   user-visible effect, and a status. **The deviation list is meant to shrink to
   zero over time** — closing a deviation means making the implementation obey the
   rule (often a bug fix or a refactor), then deleting its entry here.

> **The rules do not change to match the code. The code changes to match the rules.**
> A new edge that the rules can't express is a signal the *rule* is wrong (fix the
> rule); a place the code disobeys a sound rule is a *deviation* (fix the code).

## When to reach for this doc

**Before** the work, not after — these are the moments the rules decide something you would
otherwise deliberate about:

- **An issue calls something "a design call"**, or offers "two ways to close it". Check whether
  the rule is already written; if it is, the choice is not open. loft#1002 was filed as *"the
  choice is a design call, which is why this is filed rather than fixed"* — while
  [collections.md](collections.md) already carried `(Slice-Open) xs[(x,y)..]  open outward walk
  from a point`. The rule was written and the shipped tail was the deviation, so only one of the
  two "ways" was ever admissible.
- **You are about to ship a REFUSAL** (*"X is not supported"*). A rule may say it must work, which
  makes the refusal a deviation to record rather than a decision to make. loft#1006's
  `&(text, …)` refusal reads against [binding.md](binding.md)'s `B-Ref-Alias` (`&` makes ANY
  binding — *scalar OR heap* — a live link) and `B-Ref-Uniform` (no operation is special-cased).
- **You are about to change the observable semantics of a shipped surface.** The rule tells you
  which direction is the fix and which is the regression.
- **The two backends disagree.** The differential oracles named in each doc's Deviations section
  are the enforcement layer; a divergence is usually a deviation with a name already.

And the doctrine cuts the other way too, per the rule above: an edge the rules **cannot express**
means the *rule* is wrong and wants extending — that is not a licence to leave the code as-is.

> **An "OPEN: 0" line is a claim to re-measure, not a fact.** It is only as strong as the
> conformance corpus underneath it. [tuples.md](tuples.md) read `OPEN: 0` while loft#1004 (a
> tuple's `text` element written one index too high) and loft#1005 (a tuple `text` parameter that
> would not compile on `--native`) were both live tuple deviations — because the oracle it leans
> on, `tests/oracle/17-tuples-recursion.loft`, is all-`(integer, integer)` and carries no `text`
> at all. Before trusting a zero, read what its oracle actually covers.
>
> [ownership.md](ownership.md) read `OPEN: 0` for six weeks with loft#1029 live, for the same
> reason from a different angle: its Join corpus
> (`tests/scripts/1019-join-owned-arm-owner.loft`) moves four axes — which side owns, arm
> count, `if` vs `match`, free function vs method — and holds the ARGUMENT SPELLING fixed at a
> variable in every cell. The runtime witness that closes the Join only covered an argument the
> call site could NAME, so a literal argument leaked and no cell asked. Moving that one axis
> turned up **six more leaking spellings** — a field, a nested field, a vector element, a
> vector-typed field, `??`, and an `if` in argument position — none of them in the issue.
> **A corpus that varies the subject exhaustively still proves nothing about the axis it never
> varies** — count what is held fixed, not what is swept. (Closed 2026-08-20; the axis is now
> swept by `tests/scripts/1029-inline-argument-borrow-source.loft`.)
>
> [closures.md](closures.md) read `OPEN: 0` with a crash live, and the held-fixed axis there
> was not the subject at all but the **moment**. Its `L-Escape` corpus checks three
> destinations — a local, a struct field, a return — and every one of them writes into a
> place being INITIALISED, so the axis it never varies is *first-Set vs re-Set*. Assigning a
> fn-ref into a place that already held one kept the eight-byte form against a twenty-byte
> slot: `g = inc` panicked on `--interpret` while `--native` ran the same program, so it was
> a backend SPLIT that neither backend alone could witness. Sweeping the CONTAINER — vector
> element, keyed value, struct-enum payload, nested struct — came back entirely clean; the
> find was on the axis nobody had thought of as an axis. (D-clo-3, 2026-08-22.)
>
> [iteration.md](iteration.md) read `OPEN: 0` while every vector combinator was broken over
> a TUPLE element — `map` answering a packed DbRef as `343597383710`, `filter` segfaulting,
> `--native` refusing to compile. Its conformance corpus runs map/filter/reduce entirely on
> `vector<integer>`; the text cell is a different SOURCE kind, not a vector of text, so the
> ELEMENT TYPE is never varied at all. That is now the THIRD doc whose zero rested on an
> all-scalar corpus, after interfaces.md (scalar instantiation) and tuples.md (no `text`) —
> which makes "what does this corpus instantiate at?" the first question to ask of any
> conformance list, not a per-doc accident. (D-iter-1, loft#1074, 2026-08-22.)
>
> **And a zero can fail a third way, with no oracle or corpus involved.**
> [tuples.md](tuples.md) read `OPEN: 0` again on the day it closed D-tup-1 by collapsing three
> disagreeing element lists into one — while D-tup-2 was live, because the surviving list is
> only consulted at one of the two sites that construct a `&(…)`. Single-sourcing a rule makes
> the answers agree; it does not make anyone ASK.
>
> So a zero rests on three things, and each has to be checked separately: what the corpus
> COVERS, which axes it holds FIXED, and whether every site that makes the decision actually
> reaches the rule.

## Reading guide

These docs are dense (they are a spec), but every rule is meant to be readable. How to
get through them:

- **Read the prose first.** Each substantial block of formal rules is paired with an
  **"In words"** reading in plain English. The formal rule is the precise version; the
  prose is the one to read first. If the two ever disagree, the prose is the mistake
  (fix it).
- **The notation, explained once:**
  - `Γ ⊢ e ⇒ τ` — "expression `e` *has* type `τ`" (the parser works the type out itself).
  - `Γ ⊢ e ⇐ τ` — "`e` is *checked against* an expected type `τ`" (the `τ` is pushed in
    from the surrounding code).
  - `τ ⤳ σ` — "a value of type `τ` is *accepted where* `σ` is expected", with no cast.
  - `⊔` — the *join*: the smallest type that contains both (for integers, the wider range).
  - `(Name)` in front of a rule is just its label, so a deviation can cite it.
- **The examples are the anchor.** Each area ends with *falsifying programs* — tiny
  snippets where obeying the rule and obeying today's code disagree. Read those to see
  what a rule actually buys.
- **A deviation (`Dn`) is a known gap, not a bug report** — "the code breaks this rule,
  here" — tracked to be removed, then deleted.

## Relationship to the working docs

These formal docs are **new and separate**; the existing planning/analysis docs are
unchanged and stay where they are:

| doc | role |
|---|---|
| [FORMALIZATION.md](../FORMALIZATION.md) | the **lens** — why formalizing is worth it, per-layer readiness, ranked rough spots |
| [TYPING_RELATION.md](../TYPING_RELATION.md) | the **analysis** of the type/conversion area — rough spots R1–R3, recommendations |
| [STABILITY_REDFLAGS.md](../STABILITY_REDFLAGS.md), [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md), … | the runtime/ownership working records |
| **`formal/*` (here)** | the **strict spec** — the rules, and the deviation list to drive to zero |

The lens docs answer *"is this worth doing and where does it hurt?"*; these answer
*"what exactly is the rule, and which lines break it?"* They cite each other but do
not duplicate: a deviation entry links to the lens analysis instead of re-explaining it.

## Areas

The **static** areas are at **0 open deviations** (2026-07-04) except two: **layout.md**
(2026-07-07), the store byte-layout contract, at **1 open** (`D-layout-1`, the #477 version-guard
gap, mechanism-shipped pending a durable-store consumer); and **binding.md** briefly (2026-08-05 — `D-bind-9`, opened and closed the same day when `B-Ref-Reshape` gained its other two disturbances). The **operational** area is
one small-step contract split across files: the scalar core (operational.md) plus the heap,
iteration, coroutines, concurrency, calls, matching, tuples, closures (2026-07-04), and the last
two — **text formatting** and **interfaces/generics** (2026-07-05) — so the operational contract is
now written across the *whole* family. Each sibling holds **0 open deviations of their own** and
shrinks operational.md's single meta-deviation (D-op-1: conformance is *differential* via the
@PLN89 oracle, not a second executable definition).

| doc | area | status |
|---|---|---|
| [types.md](types.md) | type system + conversion relation (incl. integer width) | **0 open** — @PLN25 value/null model landed (DN1–DN6 + D2 closed); the @PLN102 null-flow generalisation (N-Prop/N-Domain/N-Cast/N-Store incl. call-arg + DN3-Float) SHIPPED default-on, verified both backends |
| [binding.md](binding.md) | reference types & `&` (the bind-site link law) + the `const` immutability axis | **1 open** (D-bind-11: `&(τ, …)` admits only SCALAR elements, against B-Ref-Alias/B-Ref-Uniform — the two backends represent a reference tuple differently and `text` is the first element where that shows; loft#1006) — **B-Ref-AnnotationOnly is now total** (D-bind-10, 2026-08-09): a `&` that was the LAST operand of an expression (`b = 1 + &a`, `b += &a`, a block-final `1 + &a`, `S { x: &a }`) used to compile, because the guard peeked only the token AFTER the operand; `B-Ref-StoredRef` records the one legal non-binding position, a `reference<τ>` field. **B-Ref-Reshape** landed (@PLN130 F9, loft#779): disturbing a container while a `&` reference into it is LIVE is a compile error, for all three of `B-Disturb`'s events (removal, re-key, container reassignment). It is the first application of C79's 2026-08-05 *decline-what-we-cannot-implement-safely* revisit, whose reason is forward compatibility: an error can be dropped later, a silently different semantics cannot. Also closed: `&` is a TYPE ANNOTATION (`&τ` = `Type::RefVar`), @PLN87 ladder L1–L6 + D-bind-7 closed; the @PLN40 two-level `const` model (Const-Bind/Value/…) shipped, and D-const-1 (enum-variant const) closed via @PLN102 K1 — enforced identically to struct fields, both backends |
| [grammar.md](grammar.md) | concrete grammar + operator precedence | **0 open** — the 12-level precedence ladder written; the prefix-`&`/infix-`&` overload + non-CFG surface resolved as decided edges (C81/C82) |
| [operational.md](operational.md) | small-step semantics — the scalar core | **rules complete for the core, 2 open** — values/null sentinels, left-to-right order, uncomputable→null (C80) + `??`, state steps; the 2 open are the META deviation D-op-1/2 (differential-not-definitional conformance), inherited by every operational file below |
| [heap.md](heap.md) | store steps — alloc / read / write / **copy** / free | **rules written (2026-07-04), 0 own** — the `DbRef`/`Store` model; the whole-value COPY (C86); `H-Materialise` (a view falls back to a copy when its place is destroyed under it, @PLN130); the LIFO free discipline whose soundness is ownership.md; conformance via the oracle (D-op-1) |
| [layout.md](layout.md) | the store BYTE layout — `layout(τ)` (widths, offsets, packing, the reference encoding) | **rules written (2026-07-07), 1 open** — the FORMAT counterpart to heap.md's steps (it defines the `field_offset` heap.md reads at); one format (RAM = disk); nullability is a sentinel, not a layout (`L-Null`); **D-layout-1** (no version guard on persisted bytes, #477) is **mechanism-shipped** — the golden test + the `.dschema` sidecar — pending a durable-store consumer to auto-invoke it (@PLN97) |
| [iteration.md](iteration.md) | `for`, ranges, text iteration, the map/filter/reduce/comprehension combinators | **rules written (2026-07-04), 0 own** — index-cursor `for`, deterministic combinator order, fresh result vector; conformance via the oracle |
| [coroutines.md](coroutines.md) | generators — `yield` / `next`, stackful suspension | **rules written (2026-07-04), 0 own** — lazy one-value-per-advance; straight-line yields lazy on both backends, and so is a loop body that ONLY yields; a loop body with a SECOND statement is eager on native (a DECIDED EDGE — rustc restriction, loft#836); conformance via the oracle |
| [concurrency.md](concurrency.md) | `par` — the one parallel construct | **rules written (2026-07-04), 0 own** — a parallel map consumed in source order; determinism CONDITIONAL on a pure worker; conformance via the oracle |
| [calls.md](calls.md) | function call & return — args, parameter binding, the frame | **rules written (2026-07-04), 0 own** — args left-to-right; scalar params by-value, heap params share (mutate-through visible, whole reassign local, `&` writes back); returns independent |
| [matching.md](matching.md) | `match` — enum-variant dispatch + payload binding | **rules written (2026-07-04), 0 own** — an expression; struct-payload patterns bind by name; `_` is the final catch-all; **compile-time exhaustiveness** (a missing variant does not compile) |
| [tuples.md](tuples.md) | tuples — construct / project / destructure | **1 open** (D-tup-1: no rule for `&(…)`, so the composition of two specified features is unspecified — see D-bind-11) — positional products (n≥2); `.i` a compile-time index; `(a,b) = …` destructuring; tuple returns. ⚠ its differential oracle is all-`(integer, integer)`: the doc read `0 open` through loft#1004 and loft#1005, both `text`-element deviations it could not see |
| [closures.md](closures.md) | lambdas / closures / fn-refs — capture + apply | **0 open** (2026-07-04) — the `fn(){}` and `\|…\|` forms capture IDENTICALLY (pure sugar, D-clo-1); first-class (store/pass/return/escape); scalar-by-value / heap-shared capture; a stored un-inferrable short lambda in `map` is now a clean diagnostic, not a crash (D-clo-2) |
| [formatting.md](formatting.md) | text formatting — `"{x}"` interpolation + value→text rendering | **rules written (2026-07-05), 0 own** — arbitrary-expression interpolation, `{{`/`}}` escape, per-type render (null → `"null"`, char-0 → nothing), the width/align/pad/precision/radix specs, and fault-safe interpolation (`{a/b}` → `null(/0)`, never a halt); one rendering sink → backend parity; plus `F-Target` (@PLN124, 2026-08-09) — the same template builds a VALUE when checked against a type defining `lit`/`hole_*`; conformance via the oracle |
| [interfaces.md](interfaces.md) | interfaces (traits) + generics — bounds, satisfaction, monomorphization | **rules written (2026-07-05), 0 own** — `interface I { fn m(self: Self,…) }`, STRUCTURAL satisfaction (no `impl`), bounded `fn f<T: I>(…)`, parser-side monomorphization (one copy per concrete type → both backends identical), static satisfaction check (`'C' does not satisfy interface 'I': missing m`); compile-time only (no dynamic dispatch / inheritance / associated types — decided edges) |
| [collections.md](collections.md) | collection kinds (`vector`/`hash`/`sorted`/`index`/`spatial`/`trie`), indexing & slicing | **SCOPE (2026-07-10)** — not yet rules: it inventories the shipped behaviour, names each rule with its anchor, and lists what must be both-backends-verified before it graduates to the normal form at 0 deviations. **`Slice-Open`/`Slice-Cap` now HOLD (2026-08-19, loft#1002)** — the open spatial slices answered the Z-order tail against a rule that already said *outward walk*, and open question 4 (`:n` exact-count) is answered: exactly n from any origin |
| [ownership.md](ownership.md) | the `deps` / borrow **checker** (lifetimes) — distinct from binding.md's surface | **0 open** (2026-07-04) — D-own-1/2/3/4/5 ALL CLOSED; every store-lifetime decision reads the one total `deps` fact. The soundness proof heap.md's free rules rest on |
| [capabilities.md](capabilities.md) | sandbox **admission** — what a restricted caller may do (call / parameter / field / mutation rights) | **0 open** (2026-07-04) — the 6-rule judgment `P;ctx ⊢ e ✓` fully enforced; D-cap-1/2/3 CLOSED, each with a RED/GREEN adversarial pair. Cites ownership.md/heap.md for the owned-vs-host fact |

## Roadmap

[ROADMAP.md](ROADMAP.md) is the single ordered view of every open deviation across the
areas — sequenced into the order to resolve them, each flagged **code→spec** (the default)
or **spec-may-adjust** (where the rule itself is the decision). Start there to see the path
to a spec-conformant implementation; the per-area docs hold the detail.

[VERIFICATION.md](VERIFICATION.md) is the companion worklist for the operational rules
written 2026-07-04 (heap / iteration / coroutines / concurrency / calls / matching / tuples /
closures): per-rule, the single falsifiable claim, its both-backends status, and the standing
guard that pins it. Where ROADMAP tracks *deviations to close*, VERIFICATION tracks *rules to
pin* — the concrete plan to drive the differential oracle down from every area to every rule.

## Deviation entry format

```
### Dn — <one-line name>
- **Violates:** <rule id(s)>
- **Where:** <file:symbol> (the site(s) that break it)
- **Effect:** <user-visible symptom / issue refs>
- **Status:** OPEN | IN PROGRESS (<branch/PR>) | CLOSED (<commit> — then delete)
- **Removal:** <the change that makes the code obey the rule>
```

A CLOSED deviation is **deleted**, not kept — `git log` is the history. The count of
OPEN deviations per doc is the area's distance from formal.
