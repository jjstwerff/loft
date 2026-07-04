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

## The two forms — one captures, one does not

loft has two lambda syntaxes, and the difference is **capture**:

| form | captures outer locals? | use |
|---|---|---|
| `fn(p: T, …) -> R { body }` | **yes** — a full closure | store, pass, return, capture the environment |
| `\|p, …\| { body }` / `\|\| { body }` | **no** — params + globals only | the lightweight `map`/`filter` argument |

A bare function name (`f`, not `f()`) is a **function reference** — a first-class value of type
`fn(T…) -> R`, the non-capturing degenerate case.

## Notation

Uses [calls.md](calls.md)'s call relation and [heap.md](heap.md)'s heap `H`. A **closure** is a
pair `⟨code, env⟩`: the lambda body plus a captured environment (the outer variables it names). A
**fn-ref** is a closure with an empty environment.

---

## Rules

### Construction — a closure captures the outer variables it names

```
  (L-Fn)     fn(p₁…pₙ) -> R { body }   evaluates to a closure ⟨body, env⟩ where env captures every
             OUTER variable the body references but does not bind — its environment.
  (L-Short)  |p₁…pₙ| { body }  /  || { body }   evaluates to a NON-capturing lambda: its body may
             reference only its own parameters and GLOBAL functions (a fn-ref environment).
  (L-Ref)    a bare function name f (in a value position / fn-typed context) is a fn-ref value —
             a closure with an empty environment.
```

**In words.** Writing `fn(y) -> integer { y + x }` builds a closure that **captures** `x` from the
surrounding scope; writing `|y| { y + x }` builds a lightweight lambda that does **not** capture —
its body sees only `y` and globals. A bare `f` (a function's name used as a value) is a
first-class function reference. Which form to use is a real choice: the `|…|` form for a
`map`/`filter` callback that needs nothing from outside; the `fn(){}` form when the body must
close over a local.

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

OPEN: **2**. (The Rules above are the target; these are where the code breaks them.)

### D-clo-1 — the `|…|` lambda cannot capture, and says so unhelpfully
- **Violates:** L-Fn / L-Short (the two forms should differ in *ergonomics*, not leave a user
  stuck) — arguably the split is intended (a lightweight non-capturing lambda), but the boundary
  is a **bare error**, not a guided one.
- **Where:** `src/parser/vectors.rs::parse_lambda_short`. A `|y| { y + x }` referencing an outer
  local `x` fails with `Unknown variable 'x'` — the same message as a genuine typo, with no hint
  that the `fn(){}` form captures.
- **Effect:** a user who writes `xs.map(|y| { y + captured })` gets "Unknown variable", not "short
  lambdas don't capture — use `fn(y) -> … { … }`". The capability EXISTS (the `fn(){}` form), but
  the short form gives no path to it.
- **Status:** OPEN — decide whether the short form SHOULD capture (then implement it in
  `parse_lambda_short`, matching `parse_lambda`) or STAY non-capturing (then the diagnostic must
  guide to the `fn(){}` form). Either resolves it; the bare "Unknown variable" is the deviation.
- **Removal:** either extend `parse_lambda_short` to capture (its sibling `parse_lambda` already
  does), or emit a capture-specific diagnostic pointing at the `fn(){}` form.

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

- **`fn(){}` captures, `|…|` does not (`L-Fn` / `L-Short`)** — `x=10; f=fn()->integer{x}; f()` is
  `10`; `x=10; fn(y)->integer{y+x}` in a `map` yields `11`; the same body as `|y| { y+x }` is a
  compile error (D-clo-1).
- **Capture semantics (`L-CapScalar` / `L-CapHeap`)** — a captured scalar reads its
  creation-time value; a captured struct reads its *current* field value (`b.v=9` ⇒ `9`).
- **First-class (`L-Escape`)** — a closure returned from a function, or stored in a struct field,
  works: `mk(7)()` is `7`; `h.f()` is `42`.
- **The open edges** — `|y| { y+outer }` (D-clo-1) and `g = |y|{…}; xs.map(g)` (D-clo-2) are the
  two falsifying programs this doc tracks to zero.

When D-clo-1 and D-clo-2 close, this area joins the rest at 0 open, and closures are a full,
uniform first-class contract.
