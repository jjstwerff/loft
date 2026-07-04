<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/calls.md — small-step semantics for function calls (strict)

**Catalogue:** @F3 (scalar/call core), @PLN87 (parameter binding), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the small-step relation for a
> **function call and return** — argument evaluation, parameter binding, the frame, and the
> result. It is the most load-bearing form operational.md left unwritten. It extends
> [operational.md](operational.md) (eval order, `return`), [heap.md](heap.md) (the parameter
> binding + return value are heap facts), and [binding.md](binding.md) (a `&` parameter is the
> explicit write-back). Every rule below is a **user-visible contract** verified on both
> backends, not a test artefact.

## The one thing to get right

loft's parameter passing is **not** uniform "by value" or "by reference" — it is by TYPE:

- a **scalar** parameter is a **copy** (mutating it never touches the caller);
- a **heap** parameter (struct / vector / collection) is **shared** — a field/element
  **mutation THROUGH it is visible to the caller** — BUT a **whole-value reassignment** of the
  parameter rebinds **locally** (no write-back).

This is exactly the fact [capabilities.md](capabilities.md)'s raw-write rule rests on ("a write
whose root is a parameter mutates the caller"), and it is the contract a user must know to reason
about a call's effects.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩` and [heap.md](heap.md)'s heap `H`.
`f(p₁…pₙ) { body }` is a function; a call is `f(a₁…aₙ)`.

---

## Rules

### Arguments evaluate left to right, before the body

```
  (F-Args)   in f(a₁, …, aₙ), reduce a₁ to a value first, then a₂, …, then aₙ — left to right,
             each fully, BEFORE the body runs.  Any side effect in an argument happens in that
             order (operational.md E-Left, lifted to n-ary calls).
```

**In words.** All arguments are evaluated, left to right, before the function body begins — so
`add(tag("A"), tag("B"))` prints `A` then `B`, then calls `add`. A call is strict (call-by-value
in the evaluation-order sense: arguments are values before the body sees them); loft has no lazy
arguments.

### The call binds parameters and yields the return value

```
  (F-Call)   ⟨f(v₁…vₙ), σ⟩ → ⟨r, σ'⟩
               where a fresh frame binds pᵢ per F-Param* below, body runs to a return value r,
               and the frame is dropped (its owned locals freed, heap.md H-Free).
  (F-Return) `return e` exits the current call with e; a function whose body ends in an
             expression returns that expression (the implicit tail return).
  (F-Rec)    a call to the same (or a mutually-recursive) function gets its OWN fresh frame —
             recursion is ordinary, bounded only by the stack.
```

**In words.** Calling runs the body in a fresh frame with the parameters bound, and the value the
body returns is the call's result; the frame's own temporaries are released when it returns.
`return e` leaves early; a function whose last statement is an expression returns it without an
explicit `return`. Recursion is just a call with a fresh frame.

### Parameter binding — by TYPE, not uniform

```
  (F-ParamScalar)  a scalar parameter (integer/float/single/boolean/character) is bound to a
                   COPY of the argument.  `p = e` inside the body is a local update; the caller's
                   value is UNCHANGED.
  (F-ParamHeap)    a heap parameter (a struct / vector / keyed collection) is bound so that the
                   parameter and the caller's argument share the same store: a MUTATION THROUGH
                   the parameter — `p.field = v`, `p[i] = v`, `p.field += …` — IS VISIBLE to the
                   caller.
  (F-ParamRebind)  a WHOLE-VALUE reassignment of a heap parameter — `p = [..]`, `p = other` —
                   rebinds p LOCALLY (a fresh backing); it does NOT write back to the caller
                   (@PLN87 P2.4).  The distinction is mutate-through (visible) vs replace (local).
  (F-ParamRef)     a `&`-typed parameter (binding.md) is the EXPLICIT write-back channel: a
                   whole-value `p = e` on a `&T` parameter DOES write through to the caller.
```

**In words.** What a call can do to its arguments depends on the type. A **scalar** argument is
copied — the callee can never change the caller's number. A **struct/vector** argument is
**shared**: if the callee writes a field or element (`e.hp = 0`, `v[i] = x`), the caller sees it —
this is deliberate (no deep copy on every call, and it is how a mod "manages the entities it is
given"). But **replacing** the whole value (`v = [1,2]`) only rebinds the callee's local name; the
caller keeps its value. To get write-back on a whole-value assignment, the parameter must be
declared `&T` ([binding.md](binding.md)) — that is the one explicit channel. (Verified: `e.h=99`
in a callee ⇒ caller sees `99`; `n=n+1` on a scalar ⇒ caller unchanged; `v=[9,9]` on a plain
vector param ⇒ caller unchanged.)

### The return value is independent

```
  (F-Ret)    the value a function returns is a FRESH, independent value: a caller that mutates
             one call's result does not affect another call's result, nor any of the callee's
             (now-dropped) locals.  A function that returns a whole heap value hands out an
             OWNED value (heap.md H-Alloc / the return-buffer), never a view of a local.
             EXCEPTION: a `&T` return (binding.md) is an explicit borrow of an addressable input.
```

**In words.** Two calls to the same function give you two independent results — mutating one never
shows up in the other (verified: `a = mk(); a[0]=99; b = mk()` leaves `b[0]==1`). A function never
leaks a view of its own local: the returned heap value is owned by the caller. The only borrow you
can get back is an explicit `&T` return, which binding.md governs.

---

## Deviations

OPEN: **0** (a *rules* doc — it shrinks operational.md's D-op-1, adds no code deviation).

- **Conformance is differential** — call/return is enforced across the two backends by the
  @PLN89 oracle (D-op-1); recursion, nested calls, and struct returns are in its corpus
  (`17-tuples-recursion`, `21-deep-recursion-large-data`, `08-nrvo-mixed-return-paths`). The
  parameter contract (`F-Param*`) is exactly what the ownership register (ownership.md, 0 open)
  and the sandbox raw-write rule ([capabilities.md](capabilities.md), 0 open) are built on, so it
  has the strongest standing cross-checks in the spec.

---

## Conformance

- **Arg order (`F-Args`)** — `add(tag("A"), tag("B"))` prints `AB` before returning.
- **Scalar copy (`F-ParamScalar`)** — `fn inc(n){ n=n+1 }` leaves the caller's `x==5`.
- **Heap mutate-through (`F-ParamHeap`)** — `fn mut(e: E){ e.h=99 }` makes the caller's `o.h==99`;
  `fn f(v){ v[0]=99 }` makes the caller's `orig[0]==99`.
- **Heap reassign is local (`F-ParamRebind`)** — `fn re(v){ v=[9,9] }` leaves the caller's
  `o[0]==1`; only a `&`-param would write back.
- **Return independence (`F-Ret`)** — `a = mk(); a[0]=99; b = mk()` leaves `b[0]==1`.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on argument
order, a parameter's effect on the caller, or a return's independence is the definitional error
this doc names.
