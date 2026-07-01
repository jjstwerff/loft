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
- **alias** — heap reference-default: a binding to a heap value shares the source;
  field/element mutation writes through, with no `&` needed.

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

### `&` vs reference-default — where the annotation is load-bearing

```
  (B-HeapAlias)   a heap-typed binding ALREADY aliases the source (reference-default),
                  so for a heap field/element mutation the `&τ` annotation is REDUNDANT
                  (the W4 lint, not an error).
  (B-ScalarCopy)  a scalar-typed binding COPIES; the `&τ` annotation is exactly what
                  makes a scalar binding a link instead of a copy.
```

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
> One known **deferred** case (not a leak): `&`-write-back from a CALL/var RHS needs ownership
> transfer — `tests/issues.rs` ignored `pln87_amp_writeback_from_call_writes_back`, tracked in
> the @PLN87 plan, consistent with `&` on a non-lvalue being rejected.

---

## Conformance

The rules' falsifying programs are the ladder lock-ins (`pln87_link_l*`); the north star
`a=3; b=&a; b=4; a==4` is `B-Ref-Write` (D-bind-2). As `loft2` lands a rung its lock-in
flips to PASS and the matching deviation is **deleted** here. D-bind-0 is the deepest:
closing it (a real `&τ` reference type) makes the others fall out of the type rather than
out of per-site flags. When OPEN reaches 0, `&`-binding is formal and feeds the deferred
`deps`/borrow `ownership.md`.
