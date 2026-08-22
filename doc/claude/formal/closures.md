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

OPEN: **1** (D-clo-3). Both lambda forms capture identically (D-clo-1), and the
stored-short-lambda combinator crash is now a clean diagnostic (D-clo-2) — both closed
2026-07-04. D-clo-3 is `L-Escape`'s *storage* half, opened 2026-08-22 by re-measuring the
`OPEN: 0` this line used to carry.

> **The re-measurement, and what the corpus was holding fixed (2026-08-22).** The
> Conformance section below verifies `L-Escape` at three destinations — a local, a struct
> field, and a return — and every one of them writes into a place being **initialised**.
> The axis it never varied is therefore not the container at all but *first-Set vs re-Set*,
> and on that axis a live crash was sitting under the zero: a fn-ref written by a
> NON-CAPTURING source (a bare name, or a lambda capturing nothing) lowers to the 8-byte
> d_nr while the slot is the 20-byte pair, and only the initialising paths topped it up.
> `g = inc` on a live `g` panicked `fn_call_ref: fn_var=16 < 20` on `--interpret` while
> `--native` ran the same program — a backend SPLIT, so neither backend alone could see it
> — and `t.0 = inc` panicked on one backend and handed the user a raw rustc E0308 on the
> other. Fixed at the three destination-aware sites (`set_var`, the `TuplePut` arms of both
> backends, and the native reachability walk), guarded by
> `tests/scripts/fn-ref-reassignment-tops-up-the-pair.loft`, which was confirmed to fail on
> a pristine tree on both backends.
>
> The rest of the destination sweep came back clean and is recorded here so it is not
> re-run: vector element (literal and `+= [f]`), keyed-collection value, struct-enum
> variant payload read per-variant, nested struct-in-vector, and an un-inferrable stored
> short lambda through `map`/`any`/`all`/`sort_by`/`filter` (D-clo-2's fix named
> `parse_map` alone, but the diagnostic fires at the LAMBDA, so it was never the
> single-site risk it looked like).

> **D-clo-3 — OPEN (2026-08-22).** `L-Escape` says a closure "may be stored in a variable
> or struct field". It may be stored in one — in a LITERAL. **Assigning** to a fn-typed
> struct field or vector element that already holds a value is refused on both backends
> (`h.f = inc`, `v[0] = inc`), and refused by the wrong rule: the fn-ref field read lowers
> to a `fn_ref_field_read` Block rather than the `Call`/`Var` place shapes the assignment
> dispatcher knows, so it falls through to *"Not implemented operation = for type
> function(…)"* — a message contradicted by the same field accepting the same value one
> line earlier. The underlying capability is the P215/@P213 deferral (a non-inline source
> has no closure record built for it), which is a shipped decision pinned by
> `tests/scripts/fn-ref-field-non-inline-refused.loft`; what is a defect is that the
> assignment case never reaches that decision's diagnostic. Tracked as loft#1072, which
> separates the small half (name the real reason) from the design half (support the write).
> Workaround, verified on both backends: rebuild the value — `h = Holder { f: inc, tag:
> h.tag }`.

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

> **D-clo-2 — CLOSED (2026-07-04).** A stored short `|x|` lambda whose types could not be inferred
> (assigned without a type context, `g = |y| { y*2 }`) got a GARBAGE signature (a `text`/`void`
> default), and passing it to `.map` built a `vector<void>` result → a panic at `data.rs:4569`
> (`def(u32::MAX)`). The root cause was a crash where a **clean diagnostic** was already the intended
> outcome (the same lambda used standalone / called directly already errors "Cannot infer type for
> lambda parameter"). Fix: `parse_map` now guards a `void`/`Unknown` return (or `Unknown` param)
> from an un-inferrable fn-ref and emits the guiding "pass it inline / use `fn(x: T) -> R`"
> diagnostic instead of building the invalid result vector. The inline `.map(|y| …)` form (which
> has the element-type hint) and the long `fn(y: T) -> R` form are unaffected. Regression guard:
> `tests/leak.rs::dclo2_stored_short_lambda_map_no_crash` (parses without panicking, the guard
> diagnostic fires); 625 lib + interp + native_scripts green. (Making it *work* — inferring the
> stored lambda's types from the later `.map` source — is cross-statement inference, a separate
> enhancement; the crash → clean error is the fix.)

---

## Conformance

- **Both forms capture identically (`L-Fn`)** — `x=10; [1].map(|y| { y+x })[0]` is `11`, and the
  long form `[1].map(fn(y:integer)->integer{y+x})[0]` is also `11`; a captured heap value is shared
  (`b.v=8; [1].map(|z| { z+b.v })[0]` is `9`). A non-capturing `[1,2,3].map(|x| { x*2 })` is
  unchanged (`2`). (Guard `tests/scripts/85-short-lambda-capture.loft`.)
- **Capture semantics (`L-CapScalar` / `L-CapHeap`)** — a captured scalar reads its
  creation-time value; a captured struct reads its *current* field value (`b.v=9` ⇒ `9`).
- **First-class (`L-Escape`)** — a closure returned from a function, or stored in a struct field,
  works: `mk(7)()` is `7`; `h.f()` is `42`. ⚠ Each of these INITIALISES its destination; the
  *re-*assignment half is D-clo-3 and is only partly satisfied — a live local and a live tuple
  member now take a new fn-ref (guard
  `tests/scripts/fn-ref-reassignment-tops-up-the-pair.loft`), a struct field and a vector
  element still refuse it (loft#1072).
- **A fn-ref reaches every CONTAINER (`L-Escape`, measured 2026-08-22)** — vector element by
  literal and by `+= [f]`, keyed-collection value, struct-enum variant payload read
  per-variant, and struct-in-vector all carry one and call it back out, on both backends.
- **No-crash on an un-inferrable stored lambda (D-clo-2)** — `g = |y|{…}; xs.map(g)` now emits a
  clean "cannot infer" diagnostic on both backends, not a panic (guard
  `tests/leak.rs::dclo2_stored_short_lambda_map_no_crash`). The same diagnostic covers
  `any` / `all` / `sort_by` / `filter`: it fires at the LAMBDA, not per combinator.

Closures are a full first-class contract for CONSTRUCTION and for every container measured
above. The open edge is writing one into a place that already holds a value — D-clo-3.
