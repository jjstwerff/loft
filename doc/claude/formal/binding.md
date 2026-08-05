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
> (merged into this branch); the `&`-ladder's own deviation list is **closed (D-bind: 0
> open)** — D-bind-7, the last residual, was fixed this cycle. This doc's SECOND axis,
> `const` (@PLN40, shipped), completes the binding table alongside `&`/copy/view — its
> deviation list is now **closed (D-const: 0 open)**; D-const-1 (enum-variant enforcement
> scope) was fixed via @PLN102 K1 (see § Deviations), unrelated to the `&`-ladder.
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
- **binding-const** / **value-const** — the two `const` positions (@PLN40): `const`
  before the NAME freezes the *slot* (no re-bind, contents stay mutable); `const` before
  the TYPE freezes the *value* (no through-write, whole-value rebind stays allowed). See
  `Const-Bind` / `Const-Value` below — orthogonal to `&`/alias, not a replacement for it.

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
  (B-Ref-Reshape) REMOVING from a container while a reference into it is still LIVE is
                  a COMPILE-TIME ERROR.  `remove` renumbers the positions inside the
                  container's store and a reference is pinned to one, so this is the
                  single shape where B-Ref-Alias could not hold; rejecting it is what
                  makes B-Ref-Alias unconditional with no runtime machinery.  LIVENESS
                  is the condition, not existence — `c = &v[0]; c.n = 1; v.remove(0);`
                  is fine, because the reference is dead at the removal.  The removal
                  may be in this frame or in anything the frame CALLS (`f(v[i], v)`
                  where `f` removes from its container parameter, at any depth).
                  A plain LOCAL bind is exempt and keeps compiling: it does not alias
                  across a reshape, because it materialises (@PLN130 F2).  A plain
                  PARAMETER is NOT exempt — it aliases the caller's element exactly as
                  a `&` one does, so the rule keys on the aliasing relation, not on the
                  token.
```

**In words.** Binding copies by default — a scalar and a whole vector alike (`d = v`
gives you an independent copy). Writing `&` at the bind turns it into a live link, so
`d = &self.data; d[i] = x` writes through to the source (a game can grab a sub-vector and
mutate it in place). The one exception is reading a *struct-typed* field or element
(`o.inner`, `v[i]`): that is a view onto the interior, and mutating it is already
visible — no `&` needed there. And a reference stays valid for as long as it is used,
because the one thing that could invalidate it — removing from the container underneath
it — does not compile.

### `const` — the immutability axis (binding-const vs value-const, @PLN40, shipped)

`&` (above) and `const` are the two orthogonal axes of a binding: `&` opts a binding
*into* write-through aliasing; `const` opts a binding *out of* writes, at one of two
independent positions. The FIELD axis (`const v: T` / `v: const T` on a struct field) is
a struct-attribute property, documented in [LOFT.md § Fields](../LOFT.md) (the
four-quadrant table), not this doc's `&`-binding surface. The PARAM axis (`p: const T`) is
parameter-binding — see [calls.md](calls.md) (parameter binding) and
[LOFT.md § Functions](../LOFT.md) for the const-param prose; today only VALUE-const is
wired for parameters (binding-const params are not yet, see Const-Bind below). Design
source: [../plans/40-const-fields/const-model.md](../plans/40-const-fields/const-model.md).

```
  (Const-Bind)            `const` BEFORE THE NAME is binding-const: the SLOT never
                          re-points.  `const v: T` (field), `const x: T` (local); a
                          binding-const PARAMETER is not yet wired (Phase 1 shipped
                          value-const params + binding-const locals/fields only).
                          `t.v = other` is a compile error ("cannot reassign const
                          field '…' of struct '…' — const fields are
                          write-once-at-construction"); the CONTENTS stay mutable —
                          `t.v += […]` (append) and `t.v[i] = x` (element write) are
                          allowed.  The immutable-binding sibling of B-Copy: B-Copy's
                          copy is independent but still freely re-bindable;
                          Const-Bind additionally forbids re-binding it.
  (Const-Value)           `const` BEFORE THE TYPE is value-const: the VALUE is
                          read-only through this name.  `v: const T` (field),
                          `p: const T` (param), `x: const T` (local).  Every
                          through-write is rejected — a direct mutation
                          ("cannot mutate value-const field '…' of struct '…' — its
                          value is read-only (rebind with '=' to re-point, or drop
                          'const')") and a mutation reached BY DEREFERENCING THROUGH
                          a value-const field, `t.v[i] = …` / `t.r.x = …`
                          ("Cannot modify value-const field '…'; its value is
                          read-only") — but a WHOLE-VALUE rebind `t.v = other` is
                          ALLOWED (it re-points the slot; it does not touch the old
                          value).  The read-only dual of B-Ref-Alias: where `&` opts
                          a binding INTO write-through aliasing, `const` on the type
                          opts it OUT of every through-write.
  (Const-ScalarCollapse)  a by-value SCALAR (`integer` / `float` / `single` /
                          `boolean` / `character`) has no interior distinct from its
                          binding, so it freezes FULLY under EITHER axis:
                          `const n: integer` AND `n: const integer` both reject
                          `t.n = …` AND `t.n += …`.  (`text` is compound here, not
                          scalar — `const body: text` still allows `+=` append.)
  (Const-Compose)         `const v: const T` composes BOTH axes — Const-Bind rejects
                          the rebind, Const-Value rejects every through-write — so
                          the field is FULLY immutable: neither `t.v = other` nor
                          any mutation beneath it is accepted.
  (Const-ConstructExempt) construction lowers via a SEPARATE path (`Value::Insert`),
                          not the reassignment guard (`validate_write` /
                          `const_write_blocked`), so write-once is SET at
                          construction, not CHECKED there — `T{ v: 1 }` never
                          reaches the reassignment guard regardless of `v`'s
                          const-ness.
  (Const-VirtualReject)   `const virtual(...)` is a compile error: a
                          `virtual`/computed field is already read-only (no storage
                          to freeze), so `const` on it is redundant and rejected
                          rather than silently accepted.
```

**In words.** `const` is orthogonal to `&`: writing `&` at a bind opts INTO write-through
aliasing (`B-Ref-Alias`); writing `const` opts OUT of writes, at one of two positions.
Put `const` before the NAME (`const v: T`) and the *slot* freezes — the field can never be
re-pointed, but if the slot holds a collection or text you can still grow it in place
(`t.v += …`, `t.v[i] = x`); this is the builder shape (a `Mesh.verts`-style accumulator
grown after construction). Put `const` before the TYPE (`v: const T`) and the *value*
freezes instead — you may swap in a whole new value (`t.v = other`), but you can never
reach in and mutate the one that is there, at any depth (`t.v[i]=`, `t.v.x=`, `t.r.x=` are
all rejected). A plain scalar has no "interior" apart from its binding, so the two
positions collapse to the same fully-frozen behaviour for it (`Const-ScalarCollapse`). The
two axes compose (`const v: const T`) into a genuinely immutable field. Construction is
exempt by construction, not by a special case: a struct literal writes through a
different lowering path (`Value::Insert`) that the reassignment guard never sees, so
"write-once" really means "unchecked during construction, checked on every write after."

### Pattern captures (@PLN35, SPEC-FIRST · planned, NOT yet implemented)

> **@PLN35 · SPEC-FIRST** — the target for how PEG match-pattern captures alias the subject
> ([matching.md § Rules — PEG patterns](matching.md)), written ahead of the code. Design:
> [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

```
  (P-Cap-View)   a SINGLE structural capture that names an INTERIOR place of the subject (a struct
                 field, a struct-typed element) is a VIEW (B-View / heap.md H-View): it aliases
                 WITHOUT `&`, and carries the subject's borrow-dep (`Deps::frame1(subject)`) so both
                 backends agree on free.
  (P-Cap-Fresh)  a `..rest` sub-slice and a repetition `(a)*` accumulator are FRESH vectors
                 (heap.md H-Alloc), INDEPENDENT of the subject (B-Copy / iteration.md I-Comp).
```

**In words.** Binding a single interior piece of the matched value — a field, a struct element — is
a *view* onto that place, exactly like reading `o.inner` today (`B-View`): no copy, and a mutation
writes through. A `..rest` tail or a repetition's collected vector is instead a *fresh* vector,
independent of the subject — the same "fresh result vector" a comprehension builds (`I-Comp`) — so
mutating a captured `rest` never touches the original. This split keeps the cheap case cheap while
avoiding an interior-sub-slice lifetime that neither backend models cleanly.

---

## Deviations

OPEN: **0**. The @PLN87 ladder (L1–L6), the model + doc reconciliation
(PR#436), the residual D-bind-7 and D-bind-8 (closed below) are all verified;
@PLN40's Const-Bind / Const-Value / Const-ScalarCollapse / Const-Compose are shipped and
enforced for struct fields, parameters, and locals — and, since @PLN102 K1, for
**enum-variant fields** too (their one former residual gap, D-const-1, now closed).

> **D-bind-8 — CLOSED by adding B-Ref-Reshape (@PLN130 F9, [loft#779](https://github.com/loft-lang/loft/issues/779)).**
>
> B-Ref-Alias is unconditional — *"the `&τ` annotation makes ANY binding a live LINK to the
> source"* — and the code had one exception: three shapes where the write was silently
> DISCARDED, on both backends.
>
> ```loft
> // (a) the element does not even MOVE: `remove(2)` drops the last element.
> c = &v[0];  v.remove(2);  c.n = 99;      // v[0].n was 11 — the write was discarded.
> // (b) the element moves (index 2 -> 1) and the link does not follow it.
> c = &v[2];  v.remove(0);  c.n = 99;      // v[1].n was 33.
> // (c) the reshape is in the CALLEE, through a `&` parameter — and here with NO diagnostic.
> fn shift(target: &Box, all: &vector<Box>) { all.remove(0); target.n = 99; }
> shift(v[2], v);
> ```
>
> **The resolution is REFUSAL, not repair** (maker, 2026-08-05: *"The removal of anything from
> a structure (vector for example) that has an open `&` relation (for us an edge case) should
> be forbidden on compile time"*), so all three are now compile-time errors under the new rule
> **B-Ref-Reshape** above. That makes the pair total with no runtime machinery: a `&` always
> writes through, because the one shape where it could not is rejected before it runs. It is
> the rustc bargain in loft's spelling — rustc refuses the mutation while a borrow lives, loft
> refuses the removal — and it is affordable precisely because the maker classes it an edge
> case. Following the link instead (`if link.pos > removed.pos { link.pos -= size }`, which a
> dense vector makes arithmetic rather than a lookup) was feasible and was declined: not worth
> per-link runtime arithmetic for an edge case.
>
> A pre-freeze error-add. `manifest::CONTRACT_VERSION == 0`, and
> [COMPATIBILITY.md § The error surface is one-directional](../COMPATIBILITY.md) says loft may
> always DROP an error and after the freeze may never ADD one — so every place loft is too
> permissive is a last-chance-to-add, and every program this rejects was already silently
> wrong.
>
> **Two things measurement changed about the shape of the fix**, both recorded in
> `probes/40-reshape-refusal/README.md`:
>
> - the cross-frame half does **not** key on the `&` token. A plain struct PARAMETER aliases
>   the caller's element exactly as a `&` one does (cell X9: `fn w(t: Box) { t.n = 99 }` called
>   as `w(v[2])` writes 99 into the caller's `v`), and loft's own `warn_redundant_amp` advice
>   tells authors so — refusing only the `&` spelling would mean taking that advice trades a
>   compile error for a silent lost write. loft#779's own table asserted the opposite (*"plain
>   param copies (C86), so nothing to lose"*); that row is measurement-contradicted;
> - a plain LOCAL bind stays exempt for the opposite and equally measured reason: it does not
>   alias across a reshape, because @PLN130 F2 materialises it and says so.
>
> **Why D-bind-4 did not catch it.** Its lock-in is `c=&v[0]; c=9; v[0]==9` — no reshape. The
> rule was stated unconditionally and verified only in the simple shape, so a later change could
> narrow it without flipping any cell. The conformance lock-ins are now
> `b_ref_reshape_*` in `tests/parse_errors.rs` (six refused shapes and three positive ones) plus
> `tests/scripts/149-reference-survives-callee-reshape.loft`; `tests/scripts/145-…` and `774-…`
> pin the PLAIN-bind behaviour, which is unchanged.
>
> **Deliberately out of scope:** a reference across a keyed-collection RE-KEY. @PLN130 F4 treats
> a key-field write as a reshape and materialises the view, and for a `sorted` the record
> genuinely moves — same class, but a re-key is a *write*, not a removal, so the maker's
> sentence does not cover it. It needs its own decision.

> **Landed via @PLN102 K1 (verified, closed):**
> - **D-const-1 — enum-variant `const` / value-const fields are now enforced identically
>   to struct fields.**  `enum Shape { Circle { const radius: integer }, … }`; after
>   `if s is Circle { … }`, the direct write `s.radius = 9` is now REJECTED at parse time
>   (backend-independent, so no interp/native split). Root cause was that the field-write
>   guard resolved the field table via `Parts::Struct(fields)` only; the fix extends BOTH
>   the leaf-field block (`validate_write`) and the value-const chain-walk
>   (`lhs_frozen_through`) to also walk `Parts::EnumValue(_, fields)` — the variant def's
>   `attributes()[f_nr]` aligns with its `EnumValue` field order, so the const_field /
>   value_const checks apply unchanged (verified: the positive cells stay accepted, no
>   over-reach into a pattern-bound local copy). Diagnostics now name the owner as a
>   "variant". A pre-freeze error-add (`CONTRACT_VERSION` was 0). Regression: the boundary
>   matrix graduated to `pln40_enum_variant_*` in `tests/issues.rs` (negatives + the
>   over-reach guard) and the positive cells in `tests/scripts/40-const-fields.loft`, both
>   backends. The remaining laundering-via-local / -return / -generic scopes stay deferred
>   (Phase 3, post-1.0; see
>   [../plans/40-const-fields/const-model-phase2.md § Phase 3](../plans/40-const-fields/const-model-phase2.md)).

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

The `const` rules' falsifying programs are `tests/scripts/40-const-fields.loft` (positive
cells: construct/read/contents-mutation for every quadrant, struct **and** enum-variant)
plus the `pln40_const_*` / `pln40_vc_*` / `pln40_enum_variant_*` negatives in
`tests/issues.rs` — all graduated from the boundary matrix in
[../plans/40-const-fields/const-model.md](../plans/40-const-fields/const-model.md) and
[const-model-phase2.md](../plans/40-const-fields/const-model-phase2.md). D-const-1's
falsifier — the enum-variant write `s.radius = 9` after a `Circle` match — is now a pinned
regression (`pln40_enum_variant_const_reassign_rejected`), so a further regression fails
the suite.
