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
  (L-FnRef)  a bare function name f (in a value position / fn-typed context) is a fn-ref value —
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
  (L-CapRef)     capturing a `&T` parameter (calls.md F-ParamRef) captures its POINTEE: the
                 `&` is a channel to the CALLER's slot, so the share-or-copy question is asked
                 of what it points at.  A `&S` / `&vector<τ>` is then SHARED by (L-CapHeap) —
                 the same DbRef either way, so a field write, an element write and an append
                 from inside the closure all reach the caller — and a `&integer` / `&text` is
                 COPIED at creation by (L-CapScalar).  The `&` itself does NOT survive into
                 the closure: a write that would replace what it points AT is D-clo-18.
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

**OPEN: 3.**
- **D-clo-18** — a `&` SCALAR parameter written from inside a closure is REFUSED, where
  `(L-CapRef)` + `(F-ParamRef)` together say the write should reach the caller. The value
  lives in the caller's slot and `(L-CapScalar)` gives the closure a copy of it, so there is
  no shared record for the write to land in; making it work needs the REF itself in the
  closure record plus a write-back, which the cell machinery (the mechanism that makes the
  same shape work for a plain local) cannot supply — reads in the enclosing body would then
  see the cell while the caller still sees its own slot. Refusing is deliberate and is the
  half of loft#1276 that is not a fix: before the refusal the program COMPILED and answered
  quietly wrong (`fn bump(p: &integer) { g = fn() { p += 1; }; g(); p = p + 10; }` on `n = 5`
  answered 15 where 16 is correct — the closure's increment dropped through a parameter whose
  whole purpose is the write-back). Every other `&` capture shape is closed. Reject twin
  `tests/scripts/1276-reject-a-ref-parameter-a-closure-cannot-write.loft`
- **D-clo-20 — CLOSED 2026-09-02 (loft#1281), as a REFUSAL.** `(F-ParamHeap)` makes a
  whole-value rebind of a heap PARAMETER local to the callee, and a rebind written inside a
  CLOSURE that captures that parameter reached the CALLER instead:
  `fn repl(p: vector<integer>) { g = fn() { p = [7,7]; }; g(); }` left the caller's `[1,2]`
  as `[7,7]`, on both backends, with nothing reported, while the identical rebind written
  without the closure correctly left it alone. Every heap kind did it — vector, keyed and
  struct alike — and every right-hand side: a literal, a call, another local.

  The two rules meet here and the code followed only one. `(L-CapHeap)` is right that the
  closure and the callee body see one collection; what does not follow is that the caller
  does. The closure record holds a COPY of the parameter's DbRef, so the rebind lowered to a
  clear plus a refill of the store that copy names — and `(F-ParamHeap)` makes that store the
  caller's. A capture has no route back to the parameter SLOT, which is the binding
  `(F-ParamRebind)` rebinds.

  It is REFUSED now, which is the call `D-clo-18` makes for the `&`-scalar shape and for the
  same reason: making it mean what it says needs the binding reachable from inside the
  closure PLUS a write-back, and the cell machinery that gives a mutated captured SCALAR
  exactly that cannot serve a heap value — reads in the enclosing body would then see the
  cell while the caller still sees its own slot. Measured rather than assumed: repointing
  the capture slot alone does not fix it, because the callee reads its own slot directly
  (`t_6vector_len(p(0))`, not a read through the record), so a repoint moves the wrong answer
  from the caller to the callee instead of removing it.

  The refusal is narrow, and each exclusion still works: a captured LOCAL (no caller to reach
  past), a `&` parameter (`(L-CapRef)`, where the write-back is the point), a scalar or text
  parameter (the cell machinery), and every mutation-THROUGH — `p += [x]`, `p[i] = v`,
  `p.clear()`, a field write — which `(L-CapHeap)` and `(F-ParamGrow)` require to reach the
  caller. Reject twin `tests/parse_errors.rs::a_closure_cannot_replace_a_captured_heap_parameter`
  (all three right-hand sides, vector + hash + struct); the shapes it must not reach are
  `tests/scripts/1281-a-closure-cannot-replace-a-captured-parameter.loft`, which cannot hold
  the refused spelling because the fixed compiler will not parse it
- **D-clo-7** — a lambda's `??`-default store leaks one store per call where the borrow arm's witness cannot be NAMED and the call has nothing to witness either: TWO store-bearing captures, whose return dep names `__closure` and not which slot; that entry's value half, its BOUND-return leak half, its ARGUMENT-witness half, its single-CAPTURE witness and its literal-`null` argument are all closed (loft#1248, loft#1245)
- **D-clo-14** — a closure's `??` at a COLLECTION return leaks its mint arm; the over-free half (the lift emptied the caller's own vector) is closed, and declining the unguarded lift was the only cure correct on both backends (loft#1257)

The full register — these entries in full, plus every closed one with its dates and
issue numbers — is the companion [closures-history.md](closures-history.md).

## Conformance

- **Both forms capture identically (`L-Fn`)** — `x=10; [1].map(|y| { y+x })[0]` is `11`, and the
  long form `[1].map(fn(y:integer)->integer{y+x})[0]` is also `11`; a captured heap value is shared
  (`b.v=8; [1].map(|z| { z+b.v })[0]` is `9`). A non-capturing `[1,2,3].map(|x| { x*2 })` is
  unchanged (`2`). (Guard `tests/scripts/85-short-lambda-capture.loft`.)
- **Capture semantics (`L-CapScalar` / `L-CapHeap`)** — a captured scalar reads its
  creation-time value; a captured struct reads its *current* field value (`b.v=9` ⇒ `9`).
- **First-class (`L-Escape`)** — a closure returned from a function, or stored in a struct field,
  works: `mk(7)()` is `7`; `h.f()` is `42`.
- **A fn-ref reaches every CONTAINER (`L-Escape`, measured 2026-08-22)** — vector element by
  literal and by `+= [f]`, keyed-collection value, struct-enum variant payload read
  per-variant, and struct-in-vector all carry one and call it back out, on both backends.
- **…and a place that ALREADY holds one takes a new fn-ref (`L-Escape`, D-clo-3)** — a live
  local, a live tuple member (guard
  `tests/scripts/fn-ref-reassignment-tops-up-the-pair.loft`), and a struct field, a vector
  element, an element's field, a field's element and a `&`-parameter's field (guard
  `tests/scripts/fn-ref-assigned-into-a-field.loft`), from a bare name, an inline lambda
  (capturing or not), a non-capturing local and a call — including over a field that already
  owns a closure record, and 200 times in a loop without the store growing. A source the
  LITERAL refuses (an `if`/`match` arm, P215; a capturing source into a collection, #247)
  is refused identically here, by the same diagnostic.
- **No-crash on an un-inferrable stored lambda (D-clo-2)** — `g = |y|{…}; xs.map(g)` now emits a
  clean "cannot infer" diagnostic on both backends, not a panic (guard
  `tests/leak.rs::dclo2_stored_short_lambda_map_no_crash`). The same diagnostic covers
  `any` / `all` / `sort_by` / `filter`: it fires at the LAMBDA, not per combinator.

Closures are a full first-class contract: construction, every container measured above, and
re-assignment into a place that already holds one. What a closure may not do is bounded by
two decisions rather than by gaps — one capture shape per fn-ref attribute, and no capturing
closure inside a collection (#247/@P213) or inside a struct that a collection holds (#318).
