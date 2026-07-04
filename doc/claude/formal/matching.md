<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/matching.md — semantics for `match` (strict)

**Catalogue:** @F3 (enum/match core), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the relation for the `match`
> expression — enum-variant dispatch with payload binding. It is the second control form
> [operational.md](operational.md) pins only half of (`if`, not `match`). It extends
> operational.md (control flow, expressions) and [heap.md](heap.md) (an enum value is a tagged
> heap value; a variant pattern reads its payload). Every rule is a **user-visible contract**
> verified on both backends.
>
> A `match`'s headline guarantee is **compile-time exhaustiveness**: a `match` that forgets a
> variant does not compile. That is a promise to the user, checked before the program runs.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩`. An enum value `v` has a **variant
tag** and, for a struct-payload variant, named payload **fields**. A `match` is
`match e { pat₁ => b₁, …, patₙ => bₙ }`; a pattern is a unit variant `V`, a struct-payload
variant `V { f₁, … }`, or the wildcard `_`.

---

## Rules

### `match` is an expression that selects the first matching arm

```
  (M-Match)   ⟨match e { pat₁ => b₁, … }, σ⟩ → ⟨e', σ⟩          when e → e'   (scrutinee first)
              ⟨match v { pat₁ => b₁, … }, σ⟩ → ⟨bₖ[binds], σ⟩
                where k is the SMALLEST index whose patₖ matches v, and binds is patₖ's bindings.
  (M-Expr)    match is an EXPRESSION: every arm body bᵢ has the match's result type, and the
              selected arm's value is the whole match's value (feeds directly into `r = match …`).
```

**In words.** `match` first reduces the scrutinee to a value, then picks the **first** arm (top to
bottom) whose pattern matches, binds that pattern's variables, and evaluates its body — the body's
value **is** the match's value, so `r = match c { … }` is normal (verified: it returns `100`).
Only the selected arm runs.

### Patterns — unit, struct-payload with field binding, wildcard

```
  (M-Unit)     pattern V     matches an enum value whose variant is the unit variant V.
  (M-Variant)  pattern V { f₁, …, fₘ }  matches a value whose variant is V, BINDING each fⱼ to the
               corresponding payload field of v (by name), in scope for that arm's body.
  (M-Wild)     pattern _     matches ANY value; it is the catch-all.  It must be the LAST arm —
               an arm after `_` is a STATIC error (unreachable).
```

**In words.** A unit variant (`Dot`) matches by tag alone; a struct-payload variant
(`Circle { r }`, `Box { w, h }`) matches by tag AND binds its payload fields by name into the
arm, so `Circle { r } => r * r` uses the matched value's `r` (verified: `25` for `r = 5`). The
wildcard `_` matches everything and is the default — it must come last, because any arm written
after it could never run (loft rejects that at compile time).

### Exhaustiveness is checked at compile time

```
  (M-Exhaust)  a match on an enum must cover EVERY variant — each variant by its own arm, or a
               trailing `_`.  A match missing a variant is a STATIC ERROR
               ("match on E is not exhaustive — missing: …"), NOT a runtime fault.
```

**In words.** The compiler proves a `match` handles every case: if you add a variant to an enum,
every `match` that forgot it stops compiling with a precise "missing: …" message (verified). This
is the load-bearing guarantee — a `match` can never fall through to nothing at runtime, so there
is no "unmatched value" runtime error in loft's model; the exhaustiveness is discharged
statically, before the program runs.

---

## Deviations

OPEN: **0** (a *rules* doc — it shrinks operational.md's D-op-1, adds no code deviation).

- **Conformance is differential** — `match` dispatch is enforced across the two backends by the
  @PLN89 oracle (D-op-1): `20-nested-enum-match` and `07-enum-match-dispatch` carry struct-payload
  variants, recursive walks, and matches whose arms return different variants, precisely because
  the native tag dispatch + payload layout differ from the interpreter's. A divergence in which
  arm fires, or in a bound payload value, is caught there.
- **Exhaustiveness is a STATIC judgment** — so it also participates in the oracle's
  *driver-agreement* facet (D-op-2): `--dump` / `--interpret` / `--native` must agree that a
  non-exhaustive match is rejected.

---

## Conformance

- **Arm selection + payload bind (`M-Variant`)** — `match Sh::Circle { r: 5 } { Dot => 0,
  Circle { r } => r*r }` is `25`.
- **Wildcard default (`M-Wild`)** — `match C::D { A => 1, _ => 0 }` is `0`; an arm after `_` is a
  compile error.
- **Exhaustiveness (`M-Exhaust`)** — `match c { A => 1 }` over `enum C { A, B }` does NOT compile
  ("missing: B"); adding a `B => …` arm or a trailing `_` makes it compile.
- **As an expression (`M-Expr`)** — `r = match c { A => 100, B => 200 }` binds `r` to the arm's
  value (`100`).

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on which
arm a `match` selects, on a bound payload value, or on whether a match is exhaustive is the
definitional error this doc names.
