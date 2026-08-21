<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/interfaces.md — semantics for interfaces & generics (strict)

**Catalogue:** @F25 (generics), @F26 (interfaces & bounds), @PLN89 (differential oracle).
Reference: [INTERFACES.md](../INTERFACES.md), [TYPING_RELATION.md](../TYPING_RELATION.md).

> **Rules then deviations** (see [README](README.md)). This is the relation for loft's
> **compile-time polymorphism**: an `interface` (a set of method signatures), **structural
> satisfaction** (a type meets an interface by having the methods — no `impl`), a **generic**
> function bounded by interfaces, and **monomorphization** (one specialised copy per concrete
> type). It is primarily a *static* area (extends [types.md](types.md)); dispatch is a call
> ([calls.md](calls.md)). Every rule is a **user-visible contract** verified on both backends.

## Notation

Uses [types.md](types.md)'s `Γ ⊢ e ⇒ τ` (e *has* type τ). An **interface** `I` is a finite set of
method signatures `fn m(self: Self, p̄) -> R`. A **type variable** `T` is a name bound by a
generic header; a **bound** `T: I₁ + … + Iₖ` constrains it. `C ⊨ I` reads "concrete type `C`
**satisfies** interface `I`". `[T ↦ C]` is the substitution of a concrete type for a type variable.

---

## Rules

### Interface declaration — signatures only, `self: Self`

```
  (G-Iface)   interface I { fn m₁(self: Self, p̄₁) -> R₁  …  fn mₙ(self: Self, p̄ₙ) -> Rₙ }
              declares I as the set {m₁ … mₙ} of method SIGNATURES.  Each method's first
              parameter is `self: Self` (Self = the implementing type, filled in per instance);
              the interface has NO bodies.  An operator method may use sugar
              `op <tok> (self: Self, …) -> R`, which names the method `OpCamelCase` (e.g. `<` ⟶ OpLt).
```

**In words.** An interface is a named list of method shapes a type must provide — for example
`interface Ordered { op < (self: Self, other: Self) -> boolean }`. It states *what*, never *how*
(no method has a body). `Self` is a placeholder standing for whatever concrete type ends up
satisfying it. Operator requirements are written with `op <` and desugar to the canonical operator
method name.

### Satisfaction — structural, at the use site

```
  (G-Sat)   C ⊨ I   iff for every  fn m(self: Self, p̄) -> R  in I,  a concrete function m with
            receiver type C and signature [Self ↦ C](p̄ -> R) is VISIBLE at the point of use.
            No `impl` declaration is written or needed — having the methods IS satisfying.
```

**In words.** A type satisfies an interface exactly when the required methods exist for it — loft
reads the functions in scope, it does not want an `impl` block. A `struct Box` with
`fn size(self: Box) -> integer` automatically satisfies `interface Sizable { fn size(self:
Self) -> integer }`. Built-in types satisfy the stdlib interfaces (`Ordered`, `Equatable`,
`Addable`, `Numeric`, `Scalable`, `Printable`) through their existing operators. Satisfaction is judged with
the functions visible *where the generic is used*, not where the interface was declared.

### Generic functions — a bounded type variable

```
  (G-Gen)   fn f<T>(x: …T…) -> …T…            introduces an UNBOUNDED type variable T.
            fn f<T: I₁ + … + Iₖ>(…)           bounds T: only a type C with C ⊨ Iⱼ for every j may
                                              instantiate f, and inside f the body may call exactly
                                              the methods the bounds Iⱼ provide on a T value.
```

**In words.** `fn total<T: Sizable>(xs: vector<T>) -> integer` is generic over any element type
that is `Sizable`; the body may call `.size()` on a `T` because the bound guarantees it. An
unbounded `<T>` may only move values around (store, pass, return) — with no bound there is no
method it is allowed to call on a `T`. Multiple bounds combine with `+`.

### Monomorphization — one specialised copy per concrete type, in the parser

```
  (G-Mono)   a call f(ā) with concrete argument types C̄ SPECIALISES f: the parser produces a
             per-C̄ copy of f with [T ↦ C] applied throughout (attribute, return, and body types,
             and every method call re-resolved to C's concrete function).  This happens ONCE, in
             the parser, before backend selection — so the interpreter and `--native` receive the
             SAME specialised IR.  There is NO runtime interface value and NO dynamic dispatch.
```

**In words.** When `total` is called with a `vector<Box>`, loft builds a `Box`-specialised copy of
`total` and dispatches `.size()` to `Box`'s concrete method. Because specialisation is a parser
step feeding one shared IR to both backends, generics behave identically under `--interpret` and
`--native` — the two cannot drift. This is static monomorphization, like Rust's, not a v-table.

### Satisfaction is checked at instantiation — a miss does not compile

```
  (G-Check)  at each instantiation f[T ↦ C], the checker verifies C ⊨ Iⱼ for every bound.  A
             missing method is a STATIC error — `'C' does not satisfy interface 'I': missing m` —
             and the program does NOT compile.  The check is at the USE, not the interface
             declaration (an unused interface constrains nothing).
```

**In words.** If you call a `T: Sizable` generic with a type lacking `size`, you get a compile
error naming the type, the interface, and the missing method — never a runtime failure. The check
fires where the generic is instantiated, so the same interface can be satisfied by different sets
of visible functions at different call sites.

### Scope — compile-time polymorphism only (decided boundaries)

```
  (G-Scope)  interfaces are COMPILE-TIME bounds, not runtime types.  Out of scope by design
             (each a decided edge, not a deviation): an interface-typed VALUE / variable
             (`x: I = …` — dynamic dispatch); interface INHERITANCE (`interface A extends B`);
             ASSOCIATED types; DEFAULT method bodies; a GENERIC method inside an interface; a
             FACTORY method (a `Self` return with no `self` parameter).
```

**In words.** You cannot store a value at its interface type and dispatch on it at runtime
(`x: Sizable = …` is rejected) — an interface only ever appears as a generic *bound*. Inheritance,
associated types, default bodies, generic interface methods, and no-`self` factory methods are
deliberately not in the language (see [INTERFACES.md § out of scope](../INTERFACES.md)); each is a
decided boundary, so it belongs here as a scope rule, not as a deviation to close.

---

## Deviations

OPEN: **0** (a *rules* doc — the static satisfaction/monomorphization judgment; it adds no code
deviation and shrinks operational.md's D-op-1 on the dispatch side).

- **Conformance is differential + directly checkable** — satisfaction is a single static judgment,
  so accept/reject must agree across the drivers (D-op-1's driver-agreement facet). `G-Sat`/`G-Check`
  are checkable directly (a missing method rejects on both backends); the runtime behaviour of a
  monomorphized generic is pinned by `tests/scripts/86-interfaces.loft`, `tests/scripts/48-generics.loft`,
  `tests/scripts/1028-generic-null-typed-per-monomorph.loft` and
  `tests/scripts/1032-generic-iterator-return.loft`.

- **What `OPEN: 0` rests on here — "applied throughout" is the load-bearing phrase.** `(G-Mono)`
  says `[T ↦ C]` reaches *attribute, return, and body types, and every method call*. Four
  defects have now been the same omission: an operation whose choice is a function of `τ` was
  DECIDED while `τ` was still the type variable, and substitution then rewrote the type and left
  the choice behind — loft#1016 (`x?`'s default), loft#1020 (`x == null`), loft#1028 (a `null`
  literal's conversion), loft#1032 (the yield channel a `for` over a generator is paired with).
  Each was invisible to the oracle above, because both scripts instantiate
  over records; none of the three misbehaves at `T = <a struct>`, where a reference sentinel is
  the right answer anyway. loft#1028 is the sharpest reading of that gap: it made the two backends
  disagree — the interpreter answered a `text` monomorph the empty text, `--native` refused to
  compile the program — which is the one thing this section says monomorphization cannot do.
  A scalar instantiation is therefore one axis this doc's oracle was missing, and the count
  stays 0 only as long as the tests keep one.

  **The count is now six, and the two newest were found by sweeping the OPERATION rather
  than the type** (2026-08-22).  Both `1028-*` and `1032-*` sweep `T` across the scalars,
  but each sweeps ONE operation — the null, the yield channel — so the axis left fixed
  was *which operation the template decides*.  Moving it turned up two more the same day:

  - **The `??` null CHECK.**  `== null` was deferred by loft#1020; `??` asks the same
    question and was not.  It took the placeholder's own shape (a reference) and baked
    `rec != 0`, and the after-the-fact repair listed integer / text / float / single /
    enum and ended `_ => None`, so `boolean` and `character` fell through it.  `x ?? fb`
    LOOPED FOREVER at `T = boolean` and corrupted a record at `T = character` on
    `--interpret`; `--native` refused to compile either monomorph.  All three spellings
    were affected — `x ?? d`, `x?`, and `x ?? return d` — because all three reach the
    one check.
  - **The element READ.**  `wrap_vector_get_val` picks the value-extraction op from the
    element type and ended `_ => return code`, which reads as *"everything else is
    reference-shaped"* and was not: `character` and a VALUE enum both need unpacking.
    A template's `v[1]` handed back the address as the value — a garbage codepoint for
    `['a','b']`, `null` for `[Col::Blue, Col::Green]` — on BOTH backends, while the
    concrete twin was right.

  Both are now closed, the check by deferral and the read by an EXHAUSTIVE match (adding
  a `Type` variant fails the build there rather than joining the unhandled set).  The
  guard is `tests/scripts/generic-monomorph-null-and-element.loft`, which pairs every
  boolean and character cell with its hand-written twin — `(G-Mono)` as an assertion
  rather than as a claim.  The lesson generalises past this doc: **`_ => None` and
  `_ => return` are how a decision that is a function of `τ` goes missing quietly**, and
  a missing arm looks exactly like a deliberate one until something reads the answer.

  loft#1032 is the same reading a second time, and adds a **third** thing the oracle did not
  carry: a RETURN TYPE that is not the bare `T`. `substitute_type` had arms for `vector<T>`,
  `(T, T)` and `T?` and none for `iterator<T>`, in BOTH twins — the parser's and the variable
  table's — so a generic returning a generator kept the type variable in its return and in the
  handle its caller bound, while the loop variable beside it was substituted. `(G-Mono)` names
  the return explicitly, so this was a deviation and not a boundary; the rule did not move. The
  scalar axis is again what made it visible: at `T = text` or a struct the DbRef yield channel
  is the right answer anyway, so every cell of the new script passes before the fix at those
  types. Two of the three other defects the same repro surfaced were NOT monomorphization
  deviations at all — a forward call's back-patch and `--native`'s argument-hoist path each
  broke for a generator with no generic in the program — which is the loft#1029 lesson
  restated: a generic corpus is where such a thing becomes visible, not where it lives.

  The corpus is thin on a **second** axis, and loft#1029 is how that surfaced: it varies the
  instantiating TYPE and never varies how the ARGUMENT is spelled. Every call in both scripts
  binds its argument to a variable first, and a fresh-arm/borrow-arm join reached with anything
  else — a literal, a field, an element, a `??` — leaked a record on both backends until
  2026-08-20. That defect was NOT a monomorphization deviation — it reproduces with no generic in
  the program at all, so it is `ownership.md`'s to own (D-own-6, now closed) — but it was a
  generic corpus that made it visible, and the same omission would hide a monomorph-only variant
  of it here. `(G-Mono)`'s promise is that a specialised copy behaves as the hand-written
  concrete one would; an oracle that fixes the argument spelling cannot see the cases where it
  would not. The generic spelling itself is now a probe under
  `tests/scripts/1029-inline-argument-borrow-source.loft`'s finding and measured clean.
- **Test-hygiene note (resolved 2026-08-09):** `86-interfaces.loft::test_bounded_for_loop_struct`
  — a bounded `<T: Validatable>` for-loop over a struct vector calling a method per element — was
  commented out under a stale "crashes with P136 (use-after-free)" note. That bug is FIXED and the
  guard is live: `loft --tests tests/scripts/86-interfaces.loft` runs 11 functions including it,
  green. Only the trailing "Uncomment when fixed" comment beside it is left over.

---

## Conformance

- **Declare + structurally satisfy (`G-Iface` / `G-Sat`)** — `interface Sizable { fn size(self:
  Self) -> integer }` with `fn size(self: Box) -> integer` makes `Box ⊨ Sizable` — no `impl`.
- **Bounded generic dispatch (`G-Gen` / `G-Mono`)** — `fn total<T: Sizable>(xs: vector<T>) ->
  integer { s=0; for x in xs { s += x.size() } s }` over `[Box{2,3}, Box{4,5}]` is `26`, identical
  on both backends.
- **Satisfaction failure is static (`G-Check`)** — calling a `T: Sizable` generic with a `struct
  Bare` lacking `size` fails to compile: `'Bare' does not satisfy interface 'Sizable': missing size`.
- **No dynamic dispatch (`G-Scope`)** — `x: Sizable = Box{…}` is rejected; an interface names a
  generic bound, never a variable's type.

D-op-1's falsifier applies: any program a monomorphized generic evaluates differently on the two
backends — or that one driver accepts and another rejects — is the definitional error this doc names.
