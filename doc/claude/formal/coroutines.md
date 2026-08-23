<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/coroutines.md — small-step semantics for generators (strict)

**Catalogue:** @F34 (coroutines / generators, 0.8.3), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the small-step relation for
> loft's **generators**: a function returning `iterator<T>` whose body may `yield`. It extends
> [operational.md](operational.md) (control flow), [heap.md](heap.md) (the suspended frame is a
> heap store), and [iteration.md](iteration.md) (a `for` over a generator advances it). It is the
> written contract for the "coroutines" part operational.md's D-op-1 named unwritten — and the
> single area where the two backends differ **most**: the interpreter suspends by serialising a
> frame, native compiles a resumable **state machine**.
>
> Scope: single-value `yield` (CO1.1–CO1.6, shipped 0.8.3). `yield from` (delegation) is deferred
> to 1.1+ ([COROUTINE.md](../COROUTINE.md) CO1.4) and is not specified here.

## The model in one line

A generator is a function whose **entire call stack at the point of `yield` is preserved** on the
heap (STACKFUL), so `yield` may sit inside a helper called from the generator. Calling it does
**not** run the body — it allocates a suspended frame and returns it as an `iterator<T>` value;
the body runs, in slices, on each advance.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩` and [heap.md](heap.md)'s heap `H`.

- **`fr`** — a suspended **coroutine frame**: a heap value ([heap.md](heap.md) `H-Alloc`) holding
  the generator's saved call stack (all locals of the generator AND of any active nested calls)
  plus a **resume point** `pc` (where to continue). An `iterator<T>` value IS a reference to `fr`.
- **`state(fr)`** ∈ `{ suspended@pc, running, done }`.
- `advance(fr)` runs the frame from its resume point until the next `yield` or the end.

---

## Rules

### Calling a generator suspends immediately

```
  (G-Call)   ⟨g(args), ⟨ρ, H⟩⟩ → ⟨fr, ⟨ρ, H'⟩⟩
               where g's return type is iterator<T>,  fr = fresh frame with g's args bound,
                     state(fr) = suspended@entry,  H' allocates fr.  The BODY DOES NOT RUN YET.
```

**In words.** Calling a generator function is **not** a normal call — it does not execute the
body. It allocates a suspended frame ([heap.md](heap.md)), binds the arguments, sets the resume
point to the entry, and returns the frame as an `iterator<T>` value. Nothing the body would do
(a side effect, a `yield`) has happened yet; the first slice runs on the first advance.

### `next` / a `for` advance runs one slice, up to the next `yield`

```
  (G-Next)   ⟨next(fr), ⟨ρ, H⟩⟩ → ⟨v, ⟨ρ, H'⟩⟩
               when state(fr) = suspended@pc and advance(fr) reaches `yield v`:
                 v is produced, the FULL stack (generator + nested calls) is saved into fr,
                 state(fr) := suspended@(after that yield),  H' = the updated frame.
  (G-Done)   ⟨next(fr), ⟨ρ, H⟩⟩ → ⟨done, ⟨ρ, H'⟩⟩
               when advance(fr) reaches the generator's end without a further yield:
                 state(fr) := done.  Every later next(fr) is `done` (idempotent exhaustion).
  (G-Return) a generator has NO `return`.  Values leave a generator only through `yield`, so
             `return e` in a function whose return type is `iterator<T>` is a STATIC error —
             the value has nothing it could mean — and so is a bare `return;`, because
             ending early is what `break` does and the body then reaches its end (G-Done).
             The rule is asked at BOTH spellings of the same act: the `return` keyword, and
             a body whose TAIL is a value.
```

**In words.** Advancing a generator resumes its saved stack and runs until the **next** `yield`,
which produces one value and re-suspends — so a generator computes lazily, one value per advance,
and its side effects happen interleaved with the consumer's. When the body runs off the end
without yielding again, the iterator is **done**; advancing a done iterator stays done (it never
restarts and never faults).

### `yield` produces one value and suspends the whole stack

```
  (G-Yield)  inside advance(fr), ⟨yield v, ⟨ρ, H⟩⟩ suspends:
               v becomes next's result, ρ (the generator's locals AND every active nested
               call's frame) is serialised into fr, and control returns to the consumer.
               Execution resumes at the statement AFTER this yield on the next G-Next.
  (G-YieldDepth)  `yield` is valid at ANY call depth within the generator (stackful): a
                  `yield` inside a helper `h()` called from g suspends g's WHOLE stack, not
                  just h's frame.
```

**In words.** `yield v` hands `v` to whoever advanced the iterator and freezes the generator
exactly where it is — including any helper functions it was in the middle of calling (the
stackful property). On the next advance it thaws and continues from the statement right after the
`yield`, with every local restored. `yield` is rejected by the compiler outside a generator
function (a `yield` where the return type is not `iterator<T>` is a static error, not a runtime
one).

### A `for` over a generator is I-For over `next`

```
  (G-For)    for x in g(args) { body }
               ≡  fr := g(args) ;                    (G-Call — suspended)
                  loop { r := next(fr) ;             (G-Next — run one slice)
                         if r = done { break } ;     (G-Done)
                         x := r ; body }
```

**In words.** Consuming a generator with `for` is exactly [iteration.md](iteration.md)'s loop,
with `next(fr)` as the cursor's `next` and `done` as the stop signal. So a generator is
interchangeable with a vector at the `for` site — the difference is only that its elements are
computed lazily, on demand, rather than read from a store.

---

## Deviations

OPEN: **0** (2026-08-23) — **D-cor-1 opened and closed the same day.**

> **D-cor-1 — CLOSED (2026-08-23) — `return` in a generator was accepted and DISCARDED.**
> `(G-Call)` makes any `-> iterator<T>` function a generator and the model gives a returned
> value no meaning, but nothing said so and nothing refused it.  Both spellings of the same
> act were accepted:
>
> ```loft
> fn make() -> iterator<integer> { return counting(1); }   // the keyword
> fn make() -> iterator<integer> { counting(1) }           // the tail value
> ```
>
> An author delegating to another generator got an **EMPTY sequence** on `--interpret`,
> exit 0, no diagnostic — and a panic inside `alloc_coroutine` ("RefCell already borrowed")
> on `--native`.  `(G-Return)` above now states the rule and both spellings are refused,
> naming `break` (early end) and `for v in g() { yield v; }` (forwarding).
>
> ⚠ **The message names `break` because the other candidate was MEASURED and does not
> work.** A bare `return;` was refused only as the generic *"Expect expression after
> return"* — the check reads "the declared type is not Void" as "a value is required" — and
> it looked like a capability the refusal was removing.  It is not one: `detect_lazy_for`
> (`generation/coroutine.rs`) rejects a `return` outright, so such a generator falls back to
> the eager buffer, whose factory cannot emit a mid-body return either (rustc E0308 — the
> factory's type is `Box<dyn LoftCoroutine>`).  Enabling it in the parser alone would have
> shipped a shape that runs on one backend and does not compile on the other, which is the
> divergence the refusal exists to prevent.  Naming an untested cure in a diagnostic is
> worse than naming none.
>
> Guards: `tests/scripts/generator-return-carries-no-value.loft` (what must work, both
> backends) and three cells in `102-expected-errors.loft`.

One thing the VERIFICATION worklist surfaced (2026-07-04) is a **decided edge**, not
a deviation — recorded here so it is not mistaken for a bug:

> **DECIDED EDGE — NARROWED (loft#836, slice 1, 2026-08-10): native is eager only for a loop body
> whose resume point one state cannot encode.** Laziness (G-Call / G-Next) holds for
> **straight-line** yields on both backends, and now for a loop whose body ends in ONE
> unconditional `yield`: the loop is lowered to a header+body state pair, the cursor persists in
> the coroutine struct, and one advance runs one iteration. So
> `for i in 0..2 { print("y{i} "); yield i }` consumed by `for x in gen() { print("g{x} ") }` is
> `y0 g0 y1 g1` on BOTH backends, and an early-`break`-consumed billion-iteration loop-generator
> stops when its consumer does — it no longer runs unboundedly on native. Eagerness was never a
> rustc restriction; it was the absence of a loop-to-state-machine transform, which is the same
> shape `async`/`await` uses and compiles on stable Rust.
>
> What REMAINS eager, and why each needs a resume point a single state cannot express: more than
> one yield per iteration (re-entry must land at the yield that suspended), a yield inside an
> `if`/`match` (the resume point depends on the branch), a nested loop (a cursor per level), a
> `continue`, and a statement AFTER the yield (the iteration would have to resume mid-body).
> A yield of a tuple / fn-ref rides the `next_into` channel, which has no suspend point of its
> own; a yield of a struct / vector builds its record into a work local that is not persisted, so
> lowering it lazily would leak one record per yield. Those keep the eager buffer, and their
> **values agree**; the observable difference stays side-effect interleaving.
>
> ⚠ **One of those reasons fires on code the author did not write, measured 2026-08-23.** A
> generator that writes a **text field of a heap parameter** is eager:
>
> ```loft
> struct T { s: text }
> fn g(t: T) -> iterator<integer> { for i in 0..5 { print("p{i} "); t.s += "x"; yield i; } }
> //  --interpret : p0 g0 p1 g1          --native : p0 p1 p2 p3 p4 g0 g1
> ```
>
> It has ONE yield, and the yield IS the author's last statement — so it reads as a shape the
> lazy lowering admits. In the IR it is not: a text field write allocates a temp and its
> cleanup lands after the suspend —
> `OpSetText(t, 0, _field_1); yield i; OpFreeText(_field_1);` — so `detect_lazy_for` sees
> *"a statement AFTER the yield"* and demotes the whole generator. The same shape with an
> **integer** field is lazy on both backends (`OpSetInt(…); yield i;` — nothing trails), as are
> a text LOCAL written in the loop, a text-typed yield, and a text field READ.
>
> **This is the second time a generated cleanup op has silently demoted a generator**: the
> `detect_yield_from` comment above records the first, where matching the op count exactly
> *"silently pushed every `yield from` onto the eager path the moment that free appeared"*.
> That site learned to tolerate a trailing free; this one has not. The fix the pair implies is
> to hoist a dead temp's free above the suspend — `_field_1` is written into the record by the
> preceding `OpSetText` and never read again, so freeing it before the yield is
> semantics-preserving — rather than to widen the recogniser to accept arbitrary trailing
> statements. Not implemented; it is coroutine-lowering work, and it belongs with the
> remaining CL-9 slices under loft#836.
> Slices 2-4 of [COROUTINE.md § Design: lazy loop yields (CL-9)](../COROUTINE.md#design-lazy-loop-yields-cl-9)
> close the rest. Tracked as [loft#836](https://github.com/loft-lang/loft/issues/836).

- **Conformance is otherwise differential, and this is the hardest case** — the two backends
  implement suspension by the most different mechanisms (interp: serialise the frame to a heap
  store; native: a compiled resumable state machine, still eager for the shapes named above).
  The @PLN89 differential oracle (D-op-1) carries `12-coroutine-generator`, but it checks VALUES
  only — which is exactly why it reported full agreement across the eager/lazy difference.
  `tests/oracle/26-coroutine-laziness.loft` pins the **interleaving** for both the straight-line
  and the lazy-loop shapes, and since 2026-08-23 it ASSERTS that order rather than only
  comparing the two backends' output: it records each event into a heap-struct parameter, which
  a generator can write across a yield. Giving it that assertion is what found the text-field
  cell above — the trace was written as text first, and recording it made the generator eager.
- **`yield from` is out of scope** — delegation (CO1.4) is deferred to 1.1+; when it lands it
  extends `G-Yield` (a delegated yield forwards the sub-generator's values) and gets its own
  rule + oracle case. Until then a `yield from` is a parse-level unsupported form, not an
  unspecified runtime behaviour.

---

## Conformance

- **Lazy, one-per-advance (`G-Call` / `G-Next`)** — a generator's side effects interleave with the
  consumer's, one value per advance. STRAIGHT-LINE yields obey this on both backends
  (`print("a"); yield 1; print("b"); yield 2` → `a g1 b g2`). LOOP-based yields interleave on the
  interpreter (`y0 g0 y1 g1`) but run EAGERLY on native (`y0 y1 g0 g1`) — the decided edge above,
  not a divergence to fix.
- **Stackful (`G-YieldDepth`)** — a `yield` inside a helper called from the generator produces
  the value and resumes correctly past the helper — the same sequence on both backends.
- **Exhaustion (`G-Done`)** — a finite generator produces its sequence then reports done; further
  advances stay done (no restart, no fault).
- **Interchangeable at `for` (`G-For`)** — `for x in gen() { … }` and `for x in vec { … }`
  visit their elements by the same loop; swapping a generator for the equivalent vector changes
  only timing, not the values or their order.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a
generator's produced sequence, laziness, or exhaustion is the definitional error this doc names.
