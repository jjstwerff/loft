<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/closures.md — semantics for lambdas, closures, and fn-refs (strict)

**Catalogue:** @F22 (closures & value capture), @F23 (function references), @PLN89 (oracle).

> **Rules then deviations** (see [README](README.md)). This is the relation for loft's
> **first-class functions**: the two lambda forms, closure **capture**, function references, and
> application. It extends [calls.md](calls.md) (application is a call) and [heap.md](heap.md) (a
> capturing closure's environment is a heap record). Unlike the other operational files, this one
> has **open deviations**: the two lambda forms differ in capture, and one combinator path
> crashes. The Rules below are the **intended contract** (what a user should be able to rely on);
> the Deviations are exactly where today's implementation falls short — written so they can be
> driven to zero.

## The two forms — pure syntactic sugar (both capture)

loft has two lambda syntaxes, and (since 2026-07-04, D-clo-1 closed) they are **pure syntactic
sugar for the same thing** — both capture outer variables identically. The only difference is
ergonomics:

| form | captures outer locals? | ergonomics |
|---|---|---|
| `fn(p: T, …) -> R { body }` | **yes** | explicit parameter + return types; use anywhere |
| `\|p, …\| { body }` / `\|\| { body }` | **yes** | parameter types INFERRED from context (a `map`/`filter` callee's element type); no `->` return annotation |

A bare function name (`f`, not `f()`) is a **function reference** — a first-class value of type
`fn(T…) -> R`, a closure with an empty environment.

## Notation

Uses [calls.md](calls.md)'s call relation and [heap.md](heap.md)'s heap `H`. A **closure** is a
pair `⟨code, env⟩`: the lambda body plus a captured environment (the outer variables it names). A
**fn-ref** is a closure with an empty environment.

---

## Rules

### Construction — a closure captures the outer variables it names

```
  (L-Fn)     fn(p₁…pₙ) -> R { body }  AND  |p₁…pₙ| { body } / || { body }   both evaluate to a
             closure ⟨body, env⟩ where env captures every OUTER variable the body references but
             does not bind.  The two forms are equivalent modulo type-annotation ergonomics.
  (L-Ref)    a bare function name f (in a value position / fn-typed context) is a fn-ref value —
             a closure with an empty environment.
```

**In words.** Both `fn(y) -> integer { y + x }` and `|y| { y + x }` build a closure that
**captures** `x` from the surrounding scope — they are the same construct, differing only in that
the `|…|` form infers its parameter types from context (so it is the ergonomic `map`/`filter`
callback) while the `fn(){}` form spells them out (so it works where no type context is
available). A bare `f` (a function's name used as a value) is a first-class function reference.

### Capture semantics — scalar by value at creation, heap shared

```
  (L-CapScalar)  a captured SCALAR is captured BY VALUE at closure creation: the closure sees the
                 value the variable had when the closure was formed.
  (L-CapHeap)    a captured HEAP value (struct/vector) is SHARED: a mutation-through the source
                 AFTER capture is visible inside the closure (consistent with calls.md
                 F-ParamHeap — capture, like a call, shares heap state, copies scalars).
```

**In words.** A closure that captures an `integer x` freezes `x`'s value at the moment the closure
is built (verified: capture, then `x = 20`, still yields `10`). A closure that captures a struct or
vector shares it — mutating a field of the captured value afterwards shows up when the closure runs
(verified: `b.v = 9` after capture yields `9`). This mirrors the parameter contract in
[calls.md](calls.md): heap is shared, scalars are copied.

### First-class — store, pass, return; application is a call

```
  (L-Apply)   ⟨c(args), σ⟩   applies closure/fn-ref c: bind its parameters to args (calls.md
              F-Args/F-Param*), run body in ⟨code, env⟩, yield the return (calls.md F-Call).
  (L-Escape)  a closure is a VALUE: it may be stored in a variable or struct field, passed as an
              argument, and RETURNED from a function — a returned closure keeps its captures
              (it escapes cleanly).
```

**In words.** A closure is an ordinary value. You can put it in a variable or a struct field, pass
it to another function, and return it — and a returned closure still remembers what it captured
(verified: `fn mk(n) -> fn()->integer { fn()->integer { n } }`, then `mk(7)()` yields `7`; a
closure in a struct field `h.f()` yields `42`). Calling it is just a call ([calls.md](calls.md)),
with the closure's environment in scope.

---

## Deviations

OPEN: **1**. (The Rules above are the target; the remaining one is where the code breaks them.)

> **D-clo-1 — CLOSED (2026-07-04).** The `|…|` short form now captures outer variables exactly
> like the `fn(){}` form — the two are pure syntactic sugar (L-Fn), the maker's intent.
> `parse_lambda_short` gained the closure-param setup block its sibling `parse_lambda` already
> had (add the `__closure` attribute + set `closure_param` so the body reads captures from the
> closure record), and builds its public `Function` type from the DECLARED params only (excluding
> the hidden `__closure`, so a `.map(f)` arity check still sees one param). Inert for a
> non-capturing lambda (no captures ⇒ no closure record ⇒ the block is a no-op). Guard
> `tests/scripts/85-short-lambda-capture.loft` (scalar + heap capture, both backends); 625 lib +
> native_scripts + interp suite green. (Residual, minor: a zero-arg `|| { … }` closure *assigned
> then called* has a separate parse edge — the `.map`/inline capturing forms all work.)

### D-clo-2 — a stored `|…|` lambda passed to a combinator PANICS
- **Violates:** L-Apply / L-Escape (a lambda value must be applicable through a variable)
- **Where:** the combinator dispatch on a fn-ref VARIABLE whose value is a short lambda —
  `g = |y| { y*2 }; xs.map(g)` panics at `src/data.rs:4569` (`assertion left != right`). The
  INLINE `xs.map(|y| { y*2 })` works, and the LONG form `g = fn(y) -> … { y*2 }; xs.map(g)` works —
  only a *stored short lambda* passed to a combinator crashes.
- **Effect:** a crash (not a clean error) on a reasonable program. Both backends via the parser
  path; a minimal repro is `fn main() { g = |y| { y*2 }; r = [1,2,3].map(g); println("{r[0]}") }`.
- **Status:** OPEN — a real crash reproducible on `main`; should also be filed as a GitHub issue
  per the bug-filing policy (a minimal both-backends repro), with this deviation as its formal
  tracking.
- **Removal:** fix the fn-ref-variable combinator dispatch so a stored short lambda applies like
  the long form (which already works); add the repro to `tests/scripts/` as the guard.

---

## Conformance

- **Both forms capture identically (`L-Fn`)** — `x=10; [1].map(|y| { y+x })[0]` is `11`, and the
  long form `[1].map(fn(y:integer)->integer{y+x})[0]` is also `11`; a captured heap value is shared
  (`b.v=8; [1].map(|z| { z+b.v })[0]` is `9`). A non-capturing `[1,2,3].map(|x| { x*2 })` is
  unchanged (`2`). (Guard `tests/scripts/85-short-lambda-capture.loft`.)
- **Capture semantics (`L-CapScalar` / `L-CapHeap`)** — a captured scalar reads its
  creation-time value; a captured struct reads its *current* field value (`b.v=9` ⇒ `9`).
- **First-class (`L-Escape`)** — a closure returned from a function, or stored in a struct field,
  works: `mk(7)()` is `7`; `h.f()` is `42`.
- **The remaining open edge** — `g = |y|{…}; xs.map(g)` (D-clo-2, a stored short lambda passed to
  a combinator) is the one falsifying program this doc still tracks to zero.

When D-clo-2 closes, this area joins the rest at 0 open, and closures are a full, uniform
first-class contract.
