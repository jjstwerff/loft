<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/binding.md — reference types & `&` (strict)

> **Rules then deviations** (see [README](README.md)). The governing rule: **`&` is a
> TYPE ANNOTATION, not an operator.** `&τ` is a *reference type* (a live link to a
> τ-lvalue); the `&` belongs to the **variable's type**, fixed at its binding — it is
> not something the expression grammar applies per use. The deviation list **is** the
> @PLN87 ladder (built in the `loft2` worktree, branch `tuxedo-work2`) plus the gap
> between this type-level model and the current expression-level handling.
>
> ⚠️ The model here **supersedes** the "`&` = reassignment write-back" framing still in
> [OWNERSHIP_MODEL.md § The law](../OWNERSHIP_MODEL.md) and the
> [@PLN87 plan](../plans/87-reference-default-binding.md) on `main` (D-bind-doc). Design
> home: [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md), [DESIGN_DECISIONS.md C77](../DESIGN_DECISIONS.md).
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
because it lives in the type.

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

OPEN: **8** — D-bind-0 (the model gap), D-bind-7 (the parse leak, ⚑ vital), the @PLN87
ladder L1–L6, + the doc reconciliation. Each ladder entry links to its runnable lock-in
in `loft2` (`tests/issues.rs`, `pln87_link_l*`, currently `#[ignore]`).

### D-bind-0 — `&` is handled as an expression operator, not a type annotation
- **Violates:** B-RefType / B-RefType-OfVar
- **Where:** the parser treats `&` at the EXPRESSION level — a base-case marker
  (`loft2 operators.rs`, `amp_pending`) recorded per assignment (`expressions.rs`,
  `amp_bindings`). It "works kind of as an operator but not truly"; the linkage rides
  on a per-expression flag, not on the variable's type.
- **Effect:** the rules above cannot be read off the variable's type alone; `&`-ness is
  scattered across parse sites instead of being one type fact.
- **Status:** OPEN — the structural one; the ladder rungs below are easier to land on
  top of the expression-level handling, but the clean end-state is a reference TYPE.
- **Removal:** represent `&τ` as the binding's **type** (a reference type the variable
  carries), so reads/writes/mutations dispatch on the type, not an expression flag.

### D-bind-7 — `&` in a non-binding position is not rejected at parse time  ⚑ vital
- **Violates:** B-Ref-AnnotationOnly
- **Where:** because `&` is parsed as an expression prefix (D-bind-0), it is accepted in
  any expression position and only fails *downstream* with an unrelated error instead of
  a clean "`&` is only a reference-type annotation" parse error.
- **Effect (measured, this branch):**
  | source | wanted | actual |
  |---|---|---|
  | `x = 1 + &a` | parse error at `&` | `error[E0308]: mismatched types` |
  | `v = [&a]` | parse error at `&` | `error: Unknown variable '_elm_1'` (internal) |
  | `f(&a)` (format/call arg) | parse error at `&` | `error[E0308]: mismatched types` |
  | `&a;` (bare) / `if &a > 0` | parse error at `&` | `error[E0308]: mismatched types` |
- **Status:** OPEN — the parse leak the VITAL rule forbids; a symptom of D-bind-0.
- **Removal:** accept `&` only in reference-type-annotation positions (type / bind site)
  and reject it elsewhere *in the parser*, with a message naming the rule — not as an
  accidental type mismatch. Closing D-bind-0 (a real `&τ` type) makes this hold by
  construction; until then, an explicit parse-site guard enforces it.

### D-bind-1 — a scalar `&τ` variable copies instead of reading live  (ladder L1)
- **Violates:** B-Ref-Read
- **Where:** a scalar binding lowers to a COPY; only a HEAP source binds a live view today.
- **Effect:** `a = 3; b = &a; a = 5; b` → `3` (want `5`).
- **Status:** OPEN — `pln87_link_l1_scalar_live_read`.
- **Removal:** a scalar `&τ` variable holds a live reference to the source's slot.

### D-bind-2 — a scalar `&τ` write does not reach the source  (ladder L2, NORTH STAR)
- **Violates:** B-Ref-Write
- **Effect:** `a = 3; b = &a; b = 4; a` → `3` (want `4`).
- **Status:** OPEN — `pln87_link_l2_scalar_write_through`.

### D-bind-3 — struct-field reference blocked by the #415 copy-on-bind  (ladder L3)
- **Violates:** B-Ref-Lvalue (struct field) + B-HeapAlias
- **Where:** #415 makes a STRUCT vector-field read COPY on bind ([OWNERSHIP_MODEL.md:152](../OWNERSHIP_MODEL.md)) — a store-lifetime stopgap contradicting reference-default; the design's end state reverses it.
- **Effect:** `s.x = 3; b = &s.x; b = 4; s.x` → `3` (want `4`).
- **Status:** OPEN — gated on the **#415 reversal** (substrate stream). `pln87_link_l3_field_write_through`.

### D-bind-4 — scalar vector-element reference does not write through  (ladder L4)
- **Violates:** B-Ref-Lvalue (vector element) + B-Ref-Write
- **Effect:** `v = [10, 20]; c = &v[0]; c = 99; v[0]` → `10` (want `99`) for a SCALAR element.
- **Status:** OPEN — `pln87_link_l4_element_write_through`.

### D-bind-6 — a `&τ` parameter is not a link to the caller's lvalue  (ladder L6)
- **Violates:** B-Ref-Intro / B-Ref-Write across a call
- **Effect:** `fn f(b: &integer){ b = 4 }  … f(&a); a` → `3` (want `4`). Replaces the old "`&`-param write-back" with the uniform type-carried link.
- **Status:** OPEN — `pln87_link_l6_param_write_through`.

### D-bind-doc — canonical docs still describe the superseded write-back framing
- **Violates:** B-RefType (the model's own statement of itself)
- **Where:** [OWNERSHIP_MODEL.md § The law](../OWNERSHIP_MODEL.md) ("`&` to reassign back") + the [@PLN87 plan](../plans/87-reference-default-binding.md) write-back/P2 framing, both on `main`. The correction is committed on `loft2`/`tuxedo-work2`, not yet merged.
- **Status:** OPEN — closes when @PLN87 reaches `main`.
- **Removal:** rewrite OWNERSHIP_MODEL § The law to the type-annotation / bind-site-link framing on merge; this doc becomes the spec it points to.

---

## Conformance

The rules' falsifying programs are the ladder lock-ins (`pln87_link_l*`); the north star
`a=3; b=&a; b=4; a==4` is `B-Ref-Write` (D-bind-2). As `loft2` lands a rung its lock-in
flips to PASS and the matching deviation is **deleted** here. D-bind-0 is the deepest:
closing it (a real `&τ` reference type) makes the others fall out of the type rather than
out of per-site flags. When OPEN reaches 0, `&`-binding is formal and feeds the deferred
`deps`/borrow `ownership.md`.
