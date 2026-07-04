<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/tuples.md — semantics for tuples (strict)

**Catalogue:** @F (tuples), @PLN89 (differential oracle). Reference: [TUPLES.md](../TUPLES.md).

> **Rules then deviations** (see [README](README.md)). This is the relation for **tuples** —
> anonymous positional products: construction, element projection, and destructuring. It extends
> [operational.md](operational.md) (eval order, assignment) and [calls.md](calls.md) (a tuple is
> a first-class argument / return). Every rule is a **user-visible contract** verified on both
> backends.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩`. A tuple value is an ordered
`(v₁, …, vₙ)` with `n ≥ 2`; its type is `(τ₁, …, τₙ)`. `t.i` is the `i`-th element (0-based, a
compile-time index).

---

## Rules

### Construction — positional, left to right, at least two elements

```
  (T-Cons)   ⟨(e₁, …, eₙ), σ⟩   evaluates e₁, …, eₙ LEFT TO RIGHT (operational.md E-Left) into
             the tuple value (v₁, …, vₙ),  n ≥ 2.
  (T-Paren)  a single parenthesised expression `(e)` is NOT a tuple — it is just grouping.  A
             tuple needs ≥ 2 comma-separated elements.
```

**In words.** `(3, 7)` builds a 2-tuple, evaluating the elements in source order. Tuples are
anonymous (no declared type name) and positional — the elements can be of different types
(`(integer, text)`). A lone `(e)` is ordinary parenthesisation, not a 1-tuple; the minimum tuple
width is 2.

### Projection — `.i` reads the i-th element

```
  (T-Proj)   ⟨t.i, σ⟩ → ⟨vᵢ, σ⟩         where t = (v₀, …, vₙ₋₁) and 0 ≤ i < n; i is a COMPILE-TIME
             index (a literal), and its type is τᵢ.  An out-of-range i is a STATIC error.
```

**In words.** `t.0` is the first element, `t.1` the second, and so on — the index is a literal
fixed at compile time (not a runtime value), so its element type is known statically and an
out-of-range `.i` is a compile error, never a runtime null. (Verified: `(3,7).0` is `3`,
`.1` is `7`.)

### Destructuring — bind the elements positionally

```
  (T-Destr)  ⟨(x₁, …, xₙ) = e, σ⟩ → bind each xᵢ to the i-th element of the tuple value of e.
             The arities must match (n names for an n-tuple), positionally.
```

**In words.** `(a, b) = (5, 9)` binds `a = 5`, `b = 9` — a positional unpack. It composes with a
tuple-returning call: `(x, y) = pair()` unpacks the returned tuple directly (verified: `2 3`).

### Tuples as call arguments and returns

```
  (T-Ret)    a function may return a tuple type `(τ₁, …, τₙ)`; the returned tuple is an
             INDEPENDENT value (calls.md F-Ret), commonly unpacked at the call site by T-Destr.
```

**In words.** A tuple is a first-class value — you can return one (`fn pair() -> (integer,
integer)`), pass one, and unpack it at the caller. Returning a tuple is the idiomatic
"return two things," and the result is independent like any return (calls.md).

---

## Deviations

OPEN: **0** (a *rules* doc — it shrinks operational.md's D-op-1, adds no code deviation).

- **Conformance is differential** — tuples are enforced across the two backends by the @PLN89
  oracle (D-op-1): `17-tuples-recursion` carries construction, projection, destructuring, and
  tuple returns, precisely because the native layout (a synthetic `__tuple<…>` struct, inline
  bytes) differs from the interpreter's. A divergence in element order, value, or type is caught
  there.

---

## Conformance

- **Construct + project (`T-Cons` / `T-Proj`)** — `t = (3, 7); t.0` is `3`, `t.1` is `7`.
- **Destructure (`T-Destr`)** — `(a, b) = (5, 9)` binds `a=5, b=9`.
- **Tuple return + unpack (`T-Ret` + `T-Destr`)** — `fn pair() -> (integer,integer) { (2,3) }`,
  `(x, y) = pair()` binds `x=2, y=3`.
- **Static index (`T-Proj`)** — `t.5` on a 2-tuple is a compile error, not a runtime null.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a
tuple's element order, values, or a projection is the definitional error this doc names.
