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
> (merged into this branch); D-bind-7, its last residual, was fixed that cycle. **D-bind: 0
> open** — `B-Ref-Reshape` (2026-08-05) declines a container disturbance under a live `&`, and
> all three of its disturbances are enforced. This doc's SECOND axis,
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
                     (`x + &y`, and `1 + &y` — the position does not matter), a collection
                     element (`[&a]`), a call/format argument (`f(&a)` passes a value, not
                     a `&`-prefixed expression), a condition (`if &a > 0`), a bare
                     statement (`&a;`), a `return &a;`, a COMPOUND-assignment right-hand
                     side (`b += &a` — it mutates `b`, it does not give `b` a reference
                     type, so there is no binding to annotate), an assignment TARGET
                     (`&x = 3`).  There is NO `&` operator; permitting `&` as a
                     value-level prefix is precisely what lets the reference leak into
                     contexts that then mis-elaborate.
  (B-Ref-StoredRef)  the ONE position outside a `&τ` binding where a prefix `&` is legal
                     is a struct-literal field whose declared type is `reference<τ>`:
                     `Linked { link: &pool[i] }`.  That is a DIFFERENT type former from
                     `&τ` — `Type::Reference`, a stored cross-store pointer, versus
                     `Type::RefVar`, the stack link the rest of this doc is about — and
                     the field's type is what admits the `&`, so a `&` in a field of any
                     other type is still B-Ref-AnnotationOnly's parse error.
  (B-Ref-NotTarget)  (instance of B-Ref-AnnotationOnly) `&x = 3` is an error — a type
                     annotation lives at a binding, not on an lvalue being written.
```

**In words.** You make a reference by writing `&` at a binding — `b = &a`, or a
`&integer` parameter — which gives `b` a link to `a`; it does *not* read a value out of
`a`. Because `&` is a type annotation and not an operator, it is allowed *only* there:
writing `&` anywhere else (`1 + &a`, `[&a]`, `f(&a)`, `&x = 3`, …) is a parse error. The
single other place the token is legal is a field declared `reference<τ>`, where it is a
different type former doing a different job (`B-Ref-StoredRef`).

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
                  fresh whole value.  The alias lasts as long as the PLACE: where the
                  container is DISTURBED (B-Disturb) while the view is still live, the
                  binding MATERIALISES — it is given its own copy, taken at the bind,
                  and the author is told — so writes through it stop reaching the
                  container (@PLN130 F2/F4/F8).  A plain bind already copies, so this
                  is consistent with what it meant; a `&` gets B-Ref-Reshape instead.
  (B-View-Base)   a projection off a BORROWED base is a VIEW at EVERY element type — not only
                  a struct-typed one.  `for b in bv { c = b.vecf; … }` aliases exactly as
                  `c = b.strf` does, and so does a tuple element.  Ownership of the BASE is the
                  axis: off an OWNED base a COLLECTION projection copies (B-Copy, `af = bx.v`)
                  while a STRUCT projection views (B-View); off a BORROWED base everything
                  views.  `classify_vec_bind`'s `depend().is_empty()` is where the parser asks
                  it, and @PLN25 p379 depends on the write-through (`cells = sc.v;
                  cells[i] = h`).
  (B-View-Depth)  a vector INDEX read (`a = vv[0]`) and a NESTED field read
                  (`c = o.inner.v`) are VIEWS whatever the element type — #426's RESOLUTION,
                  whose FILED premise (*"these must COPY"*) was recorded as the wrong read:
                  under the reference-default model a binding to a heap value aliases the
                  source, in-place mutation writes through, and the view survives a source
                  realloc.  Guarded by `85-store-lifetime-reference-default-views.loft` and
                  `294-vector-element-view-semantics.loft`.
  (B-Disturb)     three events END the place a reference names, and they are the same
                  three for every rule below: REMOVING from the container (`v.remove(i)`
                  renumbers every later position — collections.md Col-Remove),
                  RE-KEYING an element (writing a key field: the record moves, or
                  becomes reachable by no key), and REASSIGNING the container itself
                  (`bx = T{…}` leaves the place with nothing to point at).  Overwriting
                  a place is NOT disturbing it: `o.inner = Box{…}` writes INTO the place
                  `o.inner` already occupies, so a view of it survives.
  (B-Ref-Reshape) DISTURBING a container while a `&` reference into it is still LIVE is
                  a COMPILE-TIME ERROR.  These are the shapes where B-Ref-Alias could
                  not hold, and declining them is what makes B-Ref-Alias unconditional
                  with no runtime machinery.  Taking a `&` is the author's OWNERSHIP
                  DECISION, and this is its consequence: loft will not quietly downgrade
                  the reference to a copy, so where it cannot honour the write it
                  declines the program (C79, revisited 2026-08-05).  LIVENESS is the
                  condition, not existence — `c = &v[0]; c.n = 1; v.remove(0);` is fine,
                  because the reference is dead at the disturbance.  The disturbance may
                  be in this frame or in anything the frame CALLS (`f(v[i], v)` where
                  `f` removes from its container parameter, at any depth).
                  A plain LOCAL bind is exempt and keeps compiling — it materialises
                  (B-View).  A plain PARAMETER is NOT exempt: it aliases the caller's
                  element exactly as a `&` one does (calls.md F-ParamHeap), so the rule
                  keys on the aliasing relation, not on the token.
```

**In words.** Binding copies by default — a scalar and a whole vector alike (`d = v`
gives you an independent copy). Writing `&` at the bind turns it into a live link, so
`d = &self.data; d[i] = x` writes through to the source (a game can grab a sub-vector and
mutate it in place).

**The exceptions are not one but three, and stating only the first is what made three separate
correct behaviours read as bugs in one week** (D-bind-12's collection half, a nested field read,
a vector index read — all three filed against `B-Copy` and all three correct).  A projection is
a VIEW when *any* of these holds: its type is a STRUCT (`o.inner`, B-View); its base is
BORROWED, at every element type (B-View-Base); or the read is an INDEX or NESTED one
(B-View-Depth, #426).  What is left for `B-Copy` is a whole VALUE, a scalar, and a one-level
COLLECTION projection off an OWNED base — which is exactly `OWNERSHIP_MODEL § The law`'s
`af = bx.v`.

**The whole boundary is pinned in one place:**
`tests/scripts/bind-copies-or-views-the-whole-boundary.loft`, eleven cells, measured identical on
both backends.  Ask it rather than re-deriving: the cells existed before, scattered across four
files, and no single one said what the rule was.

Both kinds of alias last exactly as long as the place they name, and the three things
that end a place are the same for both (B-Disturb). What differs is the answer. A plain
view gets a **copy** and is told so: it already meant value semantics, so losing
write-through is consistent. A `&` gets an **error**, because it did not — the author
asked for a live link, and silently handing back a copy would make that request a lie.
That is the consequence of writing `&`: it is an ownership decision, so loft declines the
program rather than quietly changing what it means.

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

### Pattern captures (@PLN35, SHIPPED)

> **@PLN35 · SHIPPED.** These two rules were written spec-first, ahead of the code; the code
> landed with phases 1–7 + PC1–PC5 ([matching.md § Rules — PEG patterns](matching.md)) and obeys
> both — verified on both backends: a `[first, ..rest]` capture of a struct element writes
> THROUGH to the subject (`P-Cap-View`), and mutating the captured `rest` leaves the subject
> untouched (`P-Cap-Fresh`). Design:
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

OPEN: **1** (D-bind-11); D-bind-12 opened and CLOSED the same day. B-Ref-Reshape is enforced for all three of B-Disturb's events (D-bind-9,
opened and closed 2026-08-05); B-Ref-AnnotationOnly is enforced in every position, not
only the ones a leading `&` reaches (D-bind-10, 2026-08-09).

> **D-bind-12 — CLOSED (2026-08-23) — the struct write-back was a real defect; the
> collection alias was the RULE being under-stated.** Filed as two halves; measuring the
> second one properly split them apart.
>
> **Half one — FIXED.** Writing back a value BOUND from a sibling element vanished from the
> IR entirely and leaked the record with it: `for p in w { hs = p.0; p.1 = hs; }` left
> `w[0].1` unchanged, on both backends. `move_elidable_source`'s last gate is *"owns a
> transferable store"*, read off `Uses::def_vdb` — whose own doc says *`v = OpGetField(vdb,
> 0, _)` where vdb is OpDatabase'd* and whose walk never checked the second half. So `hs =
> p.0`, a read of an EXISTING element through a borrow, counted as owning one, and
> `move_rewrite` dropped the `OpCopyRecord`. That drop is sound only when the source is
> CONSTRUCTED — its build ops are retargeted onto the destination — and `hs` has no build
> ops, so the copy WAS the write. `collect_uses` now enforces the documented condition once
> the whole body has been walked (the `OpDatabase` may not have been visited at insertion
> time). Precisely scoped: emitted IR is **byte-identical on 120 of 120 scripts**, and
> `857`'s own allocation count is unchanged at 27, so the pointer-bind it protects is
> untouched.
>
> **Half two — RESOLVED (2026-08-24): `B-View` was under-stated, and the missing clauses are
> now written as `B-View-Base` and `B-View-Depth`.** It is NOT the owner question this entry
> first called it: `OWNERSHIP_MODEL § The law` and #426's RESOLUTION had already decided it,
> and #426 records that its own filed premise (*"an index / nested read must COPY"*) was the
> wrong read. So the code was right and the rules doc was incomplete. The whole boundary — 11
> cells, both backends — is pinned by
> `tests/scripts/bind-copies-or-views-the-whole-boundary.loft`.
>
> **Original reading, kept because the mistake is the useful part:** `hv = p.0` on a
> COLLECTION element aliases, and the first reading scored that against `B-Copy`. Measured
> across the 2×2 off a BORROWED base, three of four projection cells are views:
>
> | construct | element type | behaviour |
> |---|---|---|
> | struct field | vector-typed | view |
> | struct field | struct-typed | view |
> | tuple element | vector-typed | **view** — the cell that was filed |
> | tuple element | struct-typed | copy |
>
> The implemented model is *a projection off a borrowed base is a VIEW; off an OWNED base it
> COPIES* — gated explicitly by `classify_vec_bind`'s `depend().is_empty()`, deliberate
> (`cells = sc.v; cells[i] = h` writing through is @PLN25 p379's point), and with its
> alternative measured to CORRUPT (#426, `185-nested-boolean-vector`). Verified in both
> directions: an owned base copies (`a = h.items` ⇒ `[1,2]`), a borrowed one views
> (`b = s.vecf` ⇒ `[9,9]`), and the p379 write-through reaches the source.
>
> `B-View` above states the view for a **struct-typed** projection only, so the rules cannot
> express a model the language depends on. Per [README](README.md) that means the RULE wants
> extending, not the code changing — **a rules question for the owner, deliberately not
> decided here**, since widening `B-Copy` instead would delete p379's idiom and re-enter
> #426.
>
> ⚠ **The fourth cell was the one deviation, and `B-View` already settled its direction —
> FIXED the same day.** A STRUCT-typed tuple element copied while its three siblings viewed,
> and `B-View` says a struct-typed projection IS a view, so there was no decision to make:
> the code had to move. The stored-tuple element read took the synthetic struct's attribute
> type VERBATIM, carrying neither the base's deps nor the base variable — so the bind typed
> as an OWNER while holding a handle into someone else's record, and was handed an
> `OpFreeRef` to match. Its two siblings already did it right and one says why: the
> plain-tuple site's P197 comment (*"without this, `a.v.0` returns a `Str` whose ptr points
> into a freed host"*), and `fields.rs`'s struct-field read, which carries the base deps AND
> `depending(base_var)`. All four projection cells are now views, the bind carries `["p"]`,
> and the spurious free is gone. Precisely scoped: emitted IR is unchanged on **80 of 80**
> tuple-bearing scripts (the only file that differs is the guard's own).
>
> **The consequence is pinned rather than left to be discovered:** a three-step swap through
> a bound element does NOT swap (`held` names the place), which is what its three siblings
> already did. `test_swap_through_a_view_does_not_swap` asserts that, and
> `test_swap_by_holding_the_value` shows the cure — hold the VALUE (a scalar/text local) and
> rebuild after the write.
>
> Guards: `tests/scripts/reference-tuple-heap-element-through-a-record.loft` — 8 cells, the
> two write-back ones proven to fail on a pristine worktree at `c3d18a5f` while the shapes
> that always worked (`p.1 = p.0`, a fresh literal) pass there, which is what made this read
> as *"writes to `p.1` are fine"*.

> ⚠ **Re-measured 2026-08-23, and the SECOND of the two named options is already running.**
> This entry says closing it needs *"either an op family that writes the STACK form through a
> DbRef, or backing a `&(…)` carrying heap elements with a real record"* — and the
> record-backed path is not hypothetical. A `for p in v` over `vector<(text, text)>` performs
> the EXACT swap `fn sw(p: &(text, text))` is refused for, correctly, on BOTH backends
> (`[("a1","b1")] → b1|a1`). So the open question is not *"can a reference tuple carry heap
> elements"* — it demonstrably can — but the narrower *"can a `&(…)` PARAMETER or LOCAL be
> given the record backing the loop path already uses"*, which D-tup-2 made pointed by
> deliberately making tuple locals STACK-backed so `&` works for scalars.
>
> The SIGSEGV below still reproduces (re-measured the same day with the `text` arms re-added),
> and its cause is now stated one level down: a `text` element on the STACK is a 16-byte `Str`
> — `{ ptr, len }`, a raw BORROW — while the record form is a 4-byte handle, so the record ops
> read a `Str` as a handle and get a corrupt record number. That is also why `fn f(s: &text)`
> WORKS while `&(text, text)` cannot: the `&text` parameter writes into the caller's 24-byte
> owned `String` via `OpClearStackText`/`OpAppendStackText` and the owner never changes,
> whereas a tuple's text element has no owner of its own on the stack.
>
> ⚠ **That working record-backed path had NO guard** — no script in the corpus wrote a `text`
> tuple element through it — so the evidence this entry now rests on was one refactor away
> from vanishing silently.
>
> **The remaining half is a REPRESENTATION choice, and the ops are what force it.** With the
> offset corrected, adding the `text` arms still SIGSEGVs — measured — because the two element
> paths speak different op families:
>
> | | addresses | representation of a `text` element |
> |---|---|---|
> | plain tuple (`OpPut*` + frame position) | a slot in the CURRENT frame | 16-byte inline stack form |
> | reference tuple (`OpSet*`/`OpGet*` + DbRef + offset) | any frame, via the link | 4-byte record handle |
>
> A callee must write the CALLER's frame, so only the DbRef family can reach it — and that
> family speaks the record form. Scalars are immune because an `i64` is 8 bytes in both. Closing
> this needs either an op family that writes the STACK form through a DbRef, or backing a
> `&(…)` carrying heap elements with a real record. That is the decision `D-tup-1` records as
> missing, and it is why the refusal stands meanwhile.
>
> `tuples.md` states no rule for `&(…)` at all, which is how a composition of two specified
> features went unspecified; see its Deviations note.
>
> ⚠ **Narrowed 2026-08-23 — a SECOND B-Ref-Alias violation was sitting behind this one, and
> it was not about element types at all.** This entry reads as *"`&(…)` works for scalars,
> and the open half is heap elements"*. Measured across POSITIONS instead of element types,
> the scalar half only worked at a PARAMETER: at a local, neither `b = &a` nor
> `b: &(integer, integer) = a` linked anything, at any element type. The first dropped the
> `&` and bound a copy — silently, on both backends — and the second typed a reference over a
> value, which the interpreter read as a store index and `--native` refused with a raw rustc
> `E0308` handed to the user. A `&(boolean, boolean)` local answered the un-swapped tuple with
> exit code 0. Fixed the same day (tuples.md D-tup-2, guard
> `tests/scripts/reference-tuple-local-binding.loft`): a tuple local is stack-backed, so it
> joins the scalars at `OpCreateStack` and B-Ref-Alias holds at every position for every
> admitted element type.
>
> **What stays open here is exactly the heap-element half**, and the table above is why. The
> entry's own framing — element types — is what hid a whole axis: a rule quantified over "ANY
> binding" is falsified by a POSITION as readily as by a type, and only one of those two was
> being swept.

> **D-bind-10 — CLOSED (2026-08-09) — the ⚑ VITAL rule was enforced for HALF of each
> expression.** The rule named `x + &y` as a parse error and grammar.md's D-gram-4
> declared the positional rule "total". Measured, four shapes compiled on both backends:
>
> ```loft
> b = 1 + &a;                         // an operand — the rule's OWN named example
> b += &a;                            // a compound-assignment RHS: not a bind site
> fn g(a: integer) -> integer { 1 + &a }   // a block-final tail value
> s = S { x: &a };                    // a struct-literal field of a NON-reference type
> ```
>
> **The mechanism, and why the sweep had to be over positions.** The guard
> (`operators.rs::parse_operators`, deepest precedence level) decided by peeking the token
> AFTER the `&`-operand: a `;` or `}` there meant "the `&` was the whole RHS". That proves
> nothing FOLLOWS the `&` — never that nothing PRECEDED it — so every shape where the `&`
> is the LAST operand of a larger expression passed. The one sub-expression test,
> `pln87_amp_in_subexpr_is_error`, puts the `&` at the HEAD (`b = &a + 1`), which is the
> single sub-expression position that peek did catch. One cell, one direction.
>
> **The fix supplies the other half.** The first primary of a binding RHS consumes an
> `amp_head` marker; a `&` reached after any operator, or inside a nested construct, sees
> it gone. The accept condition is now `terminates AND at head` — the pair is total. The
> head is opened in exactly three places: a plain `=` RHS (`parse_assign_op`), a statement
> start (so a bare `&a;` still reaches D-bind-7's own message), and a `reference<τ>` field
> value (`B-Ref-StoredRef`). Emitted IR + native Rust are byte-identical over the
> eight-shape accept corpus.
>
> **What the position sweep still missed, and the axis it held fixed.** The first fix
> rejected `S { x: &a }` for every field type — and broke `store_compact_b2.loft`, where
> `Linked { link: &pool[i] }` fills a `reference<Leaf>` field. Legality there is decided by
> the field's TYPE, not by the `&`'s position, and a sweep that varies position while
> pinning the type reads as complete and is not. That is `B-Ref-StoredRef`, previously
> unstated anywhere in this doc.
>
> A pre-freeze error-add (`manifest::CONTRACT_VERSION == 0`; [COMPATIBILITY.md § The error
> surface is one-directional](../COMPATIBILITY.md)) — every program it rejects was already
> silently dropping the `&` and binding a copy. Lock-ins: `pln87_amp_as_tail_operand_*`,
> `pln87_amp_as_compound_assign_rhs_*`, `pln87_amp_in_block_final_expression_*`,
> `pln87_amp_in_struct_literal_field_*`, `pln87_amp_in_return_statement_*` in
> `tests/parse_errors.rs`, with the ACCEPT half — every legal `&` position, each asserting
> the write reaches the source — in `tests/scripts/150-amp-head-position.loft`. The @PLN87 ladder (L1–L6), the model + doc reconciliation (PR#436), the residual
D-bind-7 and D-bind-8 (closed below) are all verified; @PLN40's Const-Bind / Const-Value /
Const-ScalarCollapse / Const-Compose are shipped and enforced for struct fields, parameters,
and locals — and, since @PLN102 K1, for **enum-variant fields** too (their one former residual
gap, D-const-1, now closed).

> **D-bind-9 — CLOSED same day (2026-08-05).** B-Ref-Reshape landed from the maker's sentence,
> which named REMOVAL, so the other two of `B-Disturb`'s events kept silently downgrading a `&`
> to a copy — measured on both backends, each with a *"copied out of"* advice line:
>
> ```loft
> c = &s[30];  c.key = 5;                                     // RE-KEY: s[5] was ABSENT
> c = &bx.inner;  bx = Mid { inner: Box { n: 22 } };  c.n = 99; // REASSIGN: bx.inner.n was 22
> ```
>
> Closing D-bind-8 while these held was an accounting error: the deviation named all three
> mechanisms of one rule and the sign-off covered one. Both now refuse, under the C79 principle
> (*decline what we cannot implement safely*) rather than as a second special case. The
> reassignment arm is the same liveness walk with the cause filter dropped; the re-key arm
> refuses at `note_key_field_write` where the base `is_amp_link`, and needs no liveness question
> because the key write IS the use. Lock-ins `b_ref_reshape_rekey_through_amp_link_is_error` and
> `b_ref_reshape_container_reassign_under_amp_link_is_error`, with their positive twins (a
> NON-key field still writes through; a reference dead before the reassignment still does).
>
> **The sweep that found them is the reusable part:** 14 shapes of `&` — whole struct, whole
> vector, element, field, nested, keyed non-key, keyed key, local reassign, callee reassign,
> `&` param mutate, `&` param rebind, loop, branch, overwrite-in-place — each asserting the one
> thing `&` promises, that the write reaches the source. Twelve honoured it; these two did not.
> A rule with more than one producer needs a sweep, not a cell.

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
> **What it did NOT close:** the other two disturbances. The maker's sentence named removal, so
> the RE-KEY and REASSIGNMENT causes were scoped out and still downgrade a `&` to a copy — now
> tracked as **D-bind-9** above, under the widened C79 principle rather than as an open question.

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
