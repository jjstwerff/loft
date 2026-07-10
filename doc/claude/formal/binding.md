<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/binding.md — reference types & `&` (strict)

**Catalogue:** @F21 (references `&T`). Roadmap: @PLN87.

> **Rules then deviations** (see [README](README.md)). The governing rule: **`&` is a
> TYPE ANNOTATION, not an operator.** `&τ` is a *reference type* (a live link to a
> τ-lvalue); the `&` belongs to the **variable's type**, fixed at its binding — it is
> not something the expression grammar applies per use. The @PLN87 ladder (built in the
> `loft2` worktree, branch `tuxedo-work2`) **realises this model** and landed via PR#436
> (merged into this branch); the deviation list is now **closed (OPEN: 0)** — D-bind-7,
> the last residual, was fixed this cycle.
>
> The model here is now also the one in [OWNERSHIP_MODEL.md § The law](../OWNERSHIP_MODEL.md) —
> @PLN87 rewrote it to the bind-site-link framing (the old "`&` = reassignment write-back"
> framing is gone). Design home: [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md),
> [DESIGN_DECISIONS.md C77](../DESIGN_DECISIONS.md).
> This doc is the **binding surface**; the `deps`/borrow *checker* (lifetimes) stays in
> the deferred `ownership.md`.

## Notation

- **lvalue** — an assignable location: a variable `x`, a struct field `s.x`, a vector
  element `v[i]`.
- **`&τ`** — the **reference type**: the type of a variable that is a live link to a
  τ-lvalue (read- **and** write-through). A type constructor, like `vector<τ>`.
- **alias** — a binding that SHARES the source (field/element mutation writes through).
  A plain bind is NOT an alias (it copies, `B-Copy`); you get one with `&` (`d = &v`,
  `B-Ref-Alias`), or for free when reading a struct-typed projection (`B-View`).

---

## Rules

### `&τ` is a type; `&` is its annotation (NOT an operator)

```
  (B-RefType)        `&τ` is a TYPE — a live link to a τ-lvalue.  It is part of the
                     type system (a type constructor), written with a leading `&`.
                     There is NO `&` operator in the expression grammar.
  (B-RefType-OfVar)  a variable or parameter HAS type `&τ` for its whole lifetime; the
                     reference-ness is a property of the VARIABLE (its type), fixed at
                     the binding — never a per-expression operation.
```

**In words.** `&integer` is a *type* — the type of a variable that is a live link to some
integer, the same way `vector<integer>` is the type of a vector. The `&` belongs to the
variable's type and stays there for the variable's whole life; it is never an action you
perform on a value. There is no `&` operator.

### Introducing a reference (at a binding, not by evaluating an expression)

```
  (B-Ref-Intro)      a `&`-annotated binding gives the bound variable type `&τ` LINKED
                     to an lvalue:  fn f(b: &integer)  ·  b = &a  (b : &(typeof a)).
                     It records "b's type is a link to a" — it does NOT evaluate `a`
                     into a fresh value.
  (B-Ref-Lvalue)     the linked source is an lvalue (variable / field / element).
  (B-Ref-AnnotationOnly)  ⚑ VITAL.  `&` occurs ONLY as a reference-type annotation — in a
                     type (`&τ`) or at the bind site that gives a variable that type.  A
                     unary `&` in ANY other position is a PARSE ERROR: an operand
                     (`x + &y`), a collection element (`[&a]`), a call/format argument
                     (`f(&a)` passes a value, not a `&`-prefixed expression), a condition
                     (`if &a > 0`), a bare statement (`&a;`), an assignment TARGET
                     (`&x = 3`).  There is NO `&` operator; permitting `&` as a
                     value-level prefix is precisely what lets the reference leak into
                     contexts that then mis-elaborate.
  (B-Ref-NotTarget)  (instance of B-Ref-AnnotationOnly) `&x = 3` is an error — a type
                     annotation lives at a binding, not on an lvalue being written.
```

**In words.** You make a reference by writing `&` at a binding — `b = &a`, or a
`&integer` parameter — which gives `b` a link to `a`; it does *not* read a value out of
`a`. Because `&` is a type annotation and not an operator, it is allowed *only* there:
writing `&` anywhere else (`1 + &a`, `[&a]`, `f(&a)`, `&x = 3`, …) is a parse error.

### Using a `&τ` variable — the link is carried by the type

```
  (B-Ref-Read)    reading a `&τ` variable yields the source's CURRENT value (live):
                      a = 3; b = &a; a = 5;  b == 5
  (B-Ref-Write)   writing a `&τ` variable writes the SOURCE — the NORTH STAR:
                      a = 3; b = &a; b = 4;  a == 4
  (B-Ref-Uniform) a `&τ` variable is used EXACTLY like a τ variable — read, write,
                  field/element mutate — and every operation goes through the link via
                  the EXISTING mutation code.  The TYPE carries the linkage; no
                  operation is special-cased and the mutation code is unchanged.
```

**In words.** A linked variable is a window onto its source: read it and you see the
source's current value; write it and the source changes (`a=3; b=&a; b=4` leaves
`a==4`). You use it like any normal variable — the link is invisible at the use site
because it lives in the type. (In the type system, this read-through is the conversion
rule `C-Ref` in [types.md](types.md): a `&τ` is accepted wherever a `τ` is.)

### `&` vs the default — a plain bind COPIES; `&` links; a struct projection views

```
  (B-Copy)        a PLAIN bind COPIES the source — a scalar (`d = a`) AND a heap
                  WHOLE-VALUE (`d = v`, `d = self.data`): the bound variable is
                  INDEPENDENT, and mutating it does NOT reach the source.  This is
                  [heap.md](heap.md) H-Copy (`fv = e.items; fv[0]=99` leaves `e.items[0]`).
  (B-Ref-Alias)   the `&τ` annotation makes ANY binding — scalar OR heap — a live LINK
                  to the source instead of a copy.  `d = &v` / `d = &self.data` ALIAS the
                  vector: `d[i] = x` (and `d += …`) write THROUGH to the source, which is
                  NON-OWNING (the source frees the store).  `&` is how you OPT INTO
                  aliasing; without it a heap bind copies.  This is B-Ref-Write for a
                  vector lvalue.
  (B-View)        a STRUCT-typed PROJECTION (`s = o.inner`, `e = v[i]` where the element
                  IS a struct) is a VIEW that aliases WITHOUT `&` ([heap.md](heap.md)
                  H-View: `c = o.i; c.v=9` ⇒ `o.i.v==9`) — the one place aliasing is the
                  default, because a struct projection names an interior place, not a
                  fresh whole value.
```

**In words.** Binding copies by default — a scalar and a whole vector alike (`d = v`
gives you an independent copy). Writing `&` at the bind turns it into a live link, so
`d = &self.data; d[i] = x` writes through to the source (a game can grab a sub-vector and
mutate it in place). The one exception is reading a *struct-typed* field or element
(`o.inner`, `v[i]`): that is a view onto the interior, and mutating it is already
visible — no `&` needed there.

### Pattern captures (@PLN35, SPEC-FIRST · planned, NOT yet implemented)

> **@PLN35 · SPEC-FIRST** — the target for how PEG match-pattern captures alias the subject
> ([matching.md § Rules — PEG patterns](matching.md)), written ahead of the code. Design:
> [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

```
  (P-Cap-View)   a SINGLE structural capture that names an INTERIOR place of the subject (a struct
                 field, a struct-typed element) is a VIEW (B-View / heap.md H-View): it aliases
                 WITHOUT `&`, and carries the subject's borrow-dep (`Deps::frame1(subject)`) so both
                 backends agree on free.
  (P-Cap-Fresh)  a `...rest` sub-slice and a repetition `(a)*` accumulator are FRESH vectors
                 (heap.md H-Alloc), INDEPENDENT of the subject (B-Copy / iteration.md I-Comp).
```

**In words.** Binding a single interior piece of the matched value — a field, a struct element — is
a *view* onto that place, exactly like reading `o.inner` today (`B-View`): no copy, and a mutation
writes through. A `...rest` tail or a repetition's collected vector is instead a *fresh* vector,
independent of the subject — the same "fresh result vector" a comprehension builds (`I-Comp`) — so
mutating a captured `rest` never touches the original. This split keeps the cheap case cheap while
avoiding an interior-sub-slice lifetime that neither backend models cleanly.

---

## Deviations

OPEN: **0**. The @PLN87 ladder (L1–L6), the model + doc reconciliation (PR#436), and the
last residual D-bind-7 are all closed and verified below.

> **Landed via @PLN87 / PR#436 (verified, closed):**
> - **D-bind-0** — `&τ` is now `Type::RefVar` (a reference type the variable carries); `&` is
>   no longer a general operator (a dedicated diagnostic rejects it elsewhere). Reads/writes
>   dispatch on the variable's RefVar type, not a per-expression flag.
> - **D-bind-1 / D-bind-2 (NORTH STAR)** — scalar live read + write-through: `a=3; b=&a; b=4;
>   a==4` → verified on interp **and** native.
> - **D-bind-3** — struct-field reference write-through: `b=&s.x; b=4; s.x==4` (the #415 gate
>   no longer blocks it).
> - **D-bind-4** — vector-element reference: `c=&v[0]; c=9; v[0]==9`.
> - **D-bind-6** — `&`-parameter link: `fn f(b:&integer){b=4}; f(a); a==4` → both backends.
> - **D-bind-doc** — `OWNERSHIP_MODEL § The law` rewritten to "heap aliases by default; `&`
>   binds a live REFERENCE"; the write-back framing is gone.
> - **D-bind-7 (the last residual ⚑ vital position)** — a bare `&a;` statement (and a
>   block-final `{ &a }`, the same leak) is now parse-rejected. The fix sits at the statement
>   chokepoint, `parser/expressions.rs::parse_assign`: a statement that BEGAN with a prefix
>   `&` whose `&` was not consumed by an assignment is the non-binding use the rule forbids.
>   The `operators.rs` guard clears `amp_pending` whenever it has already reported the `&`
>   (sub-expression / non-place), so the flag is still set at the chokepoint only in the
>   unreported bare/block-final case; a `started_with_amp` gate keeps a leaked flag from a
>   nested `&(…)` parse from mis-firing. Verified on interp **and** native; `pln87_d_bind_7_*`
>   in `tests/parse_errors.rs` (bare statement · bare field statement · block-final). The
>   caret points at the `&`.
>
> The former deferred case has **landed**: `&`-write-back from a CALL/var RHS
> (`fn f(o: &Obj){ o = mk() }`) now routes the RHS through a transferable owned temp, so the
> write-back reaches the caller (`a.x == 9`) — verified on interp **and** native. The parse
> rejection is gone; `tests/issues.rs::pln87_amp_writeback_from_call_writes_back` is an active,
> passing test (no longer `#[ignore]`d).

---

## Conformance

The rules' falsifying programs are the ladder lock-ins (`pln87_link_l*`); the north star
`a=3; b=&a; b=4; a==4` is `B-Ref-Write` (D-bind-2). As `loft2` lands a rung its lock-in
flips to PASS and the matching deviation is **deleted** here. D-bind-0 is the deepest:
closing it (a real `&τ` reference type) makes the others fall out of the type rather than
out of per-site flags. When OPEN reaches 0, `&`-binding is formal and feeds the deferred
`deps`/borrow `ownership.md`.
