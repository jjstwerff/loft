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

### Arity — every required parameter must be supplied (static)

```
  (F-Arity)   a call `f(a₁…aₖ)` is WELL-FORMED (checked before the program runs) only if every
              parameter is filled.  A parameter is filled by a positional argument, a named
              argument (`name: e`), or — when no argument targets it — a DEFAULT: a `= e` default
              (bound to e) or a nullable type `τ?` (bound to null).  A parameter with NEITHER a
              default nor a nullable type is REQUIRED: omitting it is a COMPILE ERROR (too FEW
              arguments), and supplying more arguments than parameters is a COMPILE ERROR (too
              MANY).  A compiler-inserted slot (a return buffer) is not a user parameter and is
              exempt from the requirement.
```

**In words.** Every parameter must get a value. A parameter is optional only if it declares a
default (`= e`) or is nullable (`τ?`, which defaults to `null`); otherwise it is required, and
omitting it is a compile error — `missing argument for parameter '…' — the call supplies too few
arguments` — just as passing extra arguments is (`Too many parameters`). loft does **not** silently
fill a missing argument: an earlier "defaulted-null" lenience (a missing argument quietly became
`null`/empty) was **removed** (2026-07-17), because it was a footgun — a missing function-typed
argument filled a broken value and crashed the stdlib, and a missing scalar silently read `null`.
Named arguments may fill parameters out of order and skip a middle parameter that carries a default
(`f(a: 1, c: 3)` when `b` has one). Verified both backends. (Arity is about the *count*; a supplied
argument's *nullability* is checked separately — passing a `τ?` value into a non-null parameter
warns, or errors for a narrow width: [types.md](types.md)'s `N-Store` rule at the call-argument
site.)

### The call binds parameters and yields the return value

```
  (F-Call)   ⟨f(v₁…vₙ), σ⟩ → ⟨r, σ'⟩
               where a fresh frame binds pᵢ per F-Param* below, body runs to a return value r,
               and the frame is dropped (its owned locals freed, heap.md H-Free).
  (F-Return) `return e` exits the current call with e; a function whose body ends in an
             expression returns that expression (the implicit tail return).
  (F-Drop)   …unless the function is DECLARED with no return type: then a value-typed tail
             expression is a STATEMENT.  It is evaluated for its effects, its value is
             discarded, and the function returns nothing.
  (F-Block)  the same rule one level in: a `{ … }` block ending in an expression YIELDS that
             expression, wherever the block stands, and the value flows out to whatever reads
             it.  It is discarded only where the BLOCK itself is a statement — a `;`-terminated
             one, a loop body, or the body a function drops by F-Drop.
  (F-Rec)    a call to the same (or a mutually-recursive) function gets its OWN fresh frame —
             recursion is ordinary, bounded only by the stack.
```

**In words.** Calling runs the body in a fresh frame with the parameters bound, and the value the
body returns is the call's result; the frame's own temporaries are released when it returns.
`return e` leaves early; a function whose last statement is an expression returns it without an
explicit `return`. Recursion is just a call with a fresh frame.

`(F-Block)` is what `(F-Return)` and `(F-Drop)` are the two function-level cases of, and it
is written down because it did not hold: `fn f() -> integer { { 5 } }` answered `null` on the
interpreter and `0` on `--native`, and `fn g() -> integer { n = 5; { n } }` answered `5` on
one and `0` on the other. A block reaching the expression parser is parsed against `Void` —
a statement, as far as that site can tell — so its tail was dropped even where its value was
the function's. The type flowed out correctly the whole time, which is why the function
type-checked and nothing said anything (loft#1076).

`(F-Drop)` is the edge `(F-Return)` alone cannot answer, and it was written after the two
backends disagreed about it (loft#1075). `fn main() { store_persist_copy(h, "…") }` ends in a
call that returns a `boolean`, and `main` returns nothing — so "returns that expression" has
nowhere to put it. The answer is the one the language already had everywhere else: a value
expression in statement position is evaluated and discarded, and the tail of a void function is
a statement like any other. The tail still RUNS — the discard is of the value, not of the work —
and `f()` and `f();` are the same program. Nothing it owns survives the call (measured: a
heap-returning tail discarded 800 000 times holds a flat resident size).

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

**`F-ParamHeap` has a static consequence at the CALL SITE.** Because a heap parameter aliases the
caller's argument, handing a callee both a container and a reference INTO that container
(`f(v[i], v)`) gives it two names for the same store — and if `f` removes from the container
parameter, the write through the other one is lost. That call does not compile
([binding.md](binding.md) `B-Ref-Reshape`, @PLN130 F9 / loft#779). Note it is `F-ParamHeap`, not
the `&` spelling, that makes this a hazard: a PLAIN struct parameter aliases the caller's element
exactly as a `&` one does, so both spellings are refused. This is the only static rejection at a
call site besides `F-Arity`.

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

OPEN: **0**. Four deviations have been carried and closed (D-call-1 … D-call-4); otherwise
this is a *rules* doc — it shrinks operational.md's D-op-1 and adds no code deviation of its
own.

⚠ All three are the SAME rule, `(F-Return)` / `(F-Block)`, and all three were *"the tail's
value was dropped"*: D-call-1 dropped it because the block's type disagreed with the
signature, D-call-2 because a block reaching the expression parser is typed `Void`, and
D-call-3 because a var stood in for the tail expression and could not carry one of its
values. A zero here means no KNOWN survivor of that class, not that the class is closed —
each was found by moving an axis the previous one held fixed.

> **D-call-4 — OPENED AND CLOSED (2026-08-26, loft#1099).** `(F-Arity)` exempts a
> compiler-inserted slot from the user-facing requirement — *"a return buffer is not a user
> parameter"* — on the premise that the slot is THERE for every call. A two-pass parser owes
> that premise an invariant it does not state: **the compiler-inserted slots a function takes
> are fixed before any call to it is lowered.** A `-> text` function whose tail is a `match`
> with a `null` arm broke it:
>
> ```loft
> fn f(k: integer) -> text { a = "ab"; match k { -1 => null, _ => a } }
> ```
>
> ```
> H5 two-pass contract: def `n_f` (#710) grew a pass-2-only attribute `___acc_1`
> (pass1=2, pass2=3) that is not a documented lazy append — a real cross-pass divergence
> ```
>
> `do_if_acc` promotes a per-arm text accumulator and `text_return` makes it a hidden `&text`
> parameter, so the verdict decides ARITY. One of its terms reads the tail's INFERRED type,
> and that is not pass-stable: instrumented, it is `Optional(Text)` on pass 1 and `Text` on
> pass 2 from IR the two passes leave byte-identical. The accumulator was therefore minted on
> pass 2 alone, and the compiler aborted rather than lower a call against a signature that had
> moved. The `if` spelling of the same program was stable throughout, which is what says this
> is about the inference and not about the null arm.
>
> **The cure was already written down two blocks up, for the same hazard.** `do_tret_bind`
> promotes its own hidden `&text` buffer and carries a gate whose comment states the rule and
> the method: *"Rather than enumerate which tail shapes lower stably, make pass 2 FOLLOW pass
> 1: promote on pass 2 only if pass 1 already minted the `__tret` attribute."* `do_if_acc` now
> carries the twin (`def_has_acc_attr`). It generalises where a per-term repair would not:
> the unstable term is fixed for every tail shape at once, including ones nobody has hit.
>
> ⚠ **This is the second time in three days a fix was found by reading the code beside the
> defect rather than the defect.** loft#1096's belief was written in its own leg's comment,
> and this one's cure was written in its sibling's. `ownership.md`'s D-own-9 draws the first
> half of that lesson; this is the second.
>
> Guard `tests/scripts/1099-a-text-match-tail-with-a-null-arm.loft`, which fails on a
> pristine tree at `66fb9bb4` before it can run a single assertion (the parse aborts) —
> so its first job is to be a program the compiler accepts, and only then to check every
> arm's value on both backends. Controls: a DECLARED-nullable return, which keeps its
> accumulator on its own disjunct (loft#741 is what losing it costs), and a `match` with no
> null arm. Emitted IR over the corpus: **1 of 900** programs changes — the guard itself —
> so every existing text tail already answered the same on both passes.
>
> Two things it does NOT close, both measured and both pre-existing. A nullable tail into a
> non-null `text` return reports `(N-Store)` for the `if` spelling and stays SILENT for a
> `match` whose arm is the null literal — the same inferred type the gate could not trust is
> what the report reads, and the guard's two `@EXPECT_WARNING` lines record which cells
> speak. And a `match` arm that CALLS a `-> text?` function into a non-null `text` return
> compiles on `--interpret` and fails `--native` with `E0716` (loft#1100).

> **D-call-3 — OPENED AND CLOSED (2026-08-26, loft#1097).** `(F-Return)` did not hold for a
> COLLECTION tail join with a `null` arm:
>
> ```loft
> fn f(k: integer) -> vector<integer> { a = [1,2]; b = [3,4]; if k < 0 { null } else if k == 0 { a } else { b } }
> ```
>
> `f(-1)` answered `[1,2]` — `== null` read **false** while `len` read **2**, one value with
> two answers, on both backends and with no diagnostic. Two arms naming a local means there is
> a store to free before the return, so `scopes::free_vars` demotes the tail `if` to a
> STATEMENT and appends `Return(Var(ret_var))`: the expression still RUNS, and its value is
> discarded exactly as D-call-2's block tail was.
>
> `ret_var` comes from `returned_var_null_unified`, which folds a `null` arm onto its
> sibling's var — and states its own premise: *"the work-ref null-inits at function entry and
> a null arm never allocates into it, so `Return(Var(v))` yields the same null the sentinel
> did"*. True of a RECORD work-ref, which `gen_set_first_ref_null` sentinel-inits. **False of
> a collection**, whose owned local gets `OpInitRef` + `OpDatabase` and whose promoted buffer
> arrives ALIVE from the caller — so on the null path that var is a live, populated vector.
> `(E-Null)` is what it costs: the sentinel is a real, observable, RESERVED value, and a
> populated vector is not it. Closed by hoisting the tail's value to a temp when the fold
> lands on a collection (`scopes::free_vars`), the shape the null-arm RECORD join beside it
> already used — the frees still run between the value and the return.
>
> ⚠ **That same premise had already failed once, at a different site, and this is what makes
> it a class rather than a cell.** loft#1096 (`ownership.md` D-own-9, the day before) is
> `scopes::free_vars` reading *"a buffer not yet minted on this path is the null sentinel,
> which `free` ignores"* — the identical belief about a collection buffer's null-path
> contents, costing a use-after-free instead of a wrong value. **One wrong belief, two sites,
> two defects.** Grep the belief, not the symptom: any site reasoning that a collection slot
> holds the sentinel on a path that did not write it is suspect.
>
> Two more faults met at this tail and are fixed with it, both from the `Bind` leg's
> whole-tail copy `OpClearVector(buf); OpAppendVector(buf, <the join>)` — which answers the
> buffer on every path and evaluates the join AFTER the clear. An arm whose value IS the
> buffer answered what the clear had just emptied (`[]`), and an arm that had already
> delivered into the buffer was appended to itself and came back DOUBLED (`[3,4,5,3,4,5]`).
> Both are cured by the CONDITIONAL delivery that leg's own note names as what would close it
> — `materialize_vector_arms_into`, one arm at a time — plus leaving alone an arm whose value
> is a VIEW of the buffer, whose answer is already in it.
>
> Guard `tests/scripts/1097-a-null-arm-in-a-collection-tail-join.loft`: all three faults
> falsified on a pristine tree at `d98e60ef` (5 of 7 cells red), with a no-null-arm join and
> the RECORD family — where the fold's premise HOLDS — as the controls that keep the repair
> from widening. Fixes loft#1097. The leak left behind (a `match` tail needing a null arm, a
> local arm AND a literal arm, one store per call) is loft#1098: a lifetime fault with its own
> trigger, not this rule.

> **D-call-1 — OPENED AND CLOSED (2026-08-22).** `(F-Drop)` did not exist, and the edge it
> now names is where the two backends parted: a function DECLARED void whose body ends in a
> value ran on `--interpret` and would not compile on `--native`, which surfaced as a bare
> rustc `E0308` quoting a temporary `.rs` file under the message "native compilation failed
> (codegen bug)". `--native` is the default backend, so `loft t.loft` failed this way for an
> ordinary shape — a build-asset script whose last expression is a call returning `boolean`.
>
> Filed (loft#1075) as a design call between "emit it as a statement" and "refuse it", on the
> reading that the rules could not express the edge. Half of that was right: the RULE was
> missing, which is why `(F-Drop)` is written above. The choice was not open, because the IR
> had already made it — `parse_block` wraps a value-typed statement in `Value::Drop` when the
> enclosing function is declared void, so both backends receive `drop n_f();` and the discard
> is the shipped answer. What differed was the BLOCK's type: every statement but the last
> reaches the `t = Type::Void` at the foot of the statement loop, so a dropped TAIL left the
> block typed `boolean` in a function whose signature is `()`, and the native emitter takes
> the signature from the declared return and the trailing default value from the block's
> inferred type. The tell was next door — `f();` with a semicolon always worked on both
> backends, from the same IR, because the `;` sent the statement round the loop to that reset.
> One token deciding whether a program compiles is what says the block type, not the emitter,
> was the thing that was wrong.
>
> The fix is one statement — the block type follows the drop — and it repaired the
> interpreter too: a dropped struct-literal tail was held to program exit ("1 stores not
> freed"), which the same wrong block type had been keeping alive.
>
> It is GATED on the function-body context, and both attempts that were not are why. A
> `result` of `Void` reaches the drop meaning two different things, and only one of them is
> a decision. The other is a placeholder something else will fill in, and there are two of
> those: a LAMBDA declares no return type either, so its body carries the same `Void` while
> its return type is INFERRED from this very block type — flattening it gave every stored
> short `|x| { … }` a void return, which `parse_map` refuses with D-clo-2's *"cannot infer
> the type of the function passed to `map`"*; and a `{ … }` in STATEMENT position is parsed
> against `Void` even when it is the TAIL of an enclosing block, where it is the value that
> block yields — flattening that made `x = {{ …; n }}` infer void, which is the shape the
> Rust test harness writes around every `.expr(…)`. Both were found by the suite, not by
> reasoning.
>
> `unused_must_use` and `path_statements` also join the generated file's allow-list: a
> `#[must_use]` runtime op or a bare local reached as a statement is loft doing what the IR
> told it, and the warnings were reaching users quoting generated Rust — pre-existing on
> both trees, found by this matrix, and the same class of leak as the error. Guard `tests/scripts/void-fn-value-tail.loft`, confirmed
> to fail on a pristine tree at 655ff4dd with 13 `E0308`s on `--native` while `--interpret`
> ran it clean. Fixes loft#1075.

> **D-call-2 — OPENED AND CLOSED (2026-08-22).** `(F-Block)` did not hold: a `{ … }` block
> whose value someone reads dropped its own tail, so `fn f() -> integer { { 5 } }` answered
> `null` on `--interpret` and `0` on `--native`, and `fn g() -> integer { n = 5; { n } }`
> answered `5` on one backend and `0` on the other — silently, with the function
> type-checking, because the block's TYPE is its tail's type and only the value was thrown
> away. Every `{ … }` reaching `expression` is parsed against `Void` (a statement, as far as
> that site can tell), and the parse site cannot know which statement turns out to be the
> last one. The drop is undone after the statement loop, where the block's type is already
> the value it yields.
>
> Two boundaries, each measured rather than argued. **Depth**: a first version asked the
> question of the block one level DOWN and repaired `{ { 5 } }` while `{ { { 5 } } }` still
> answered null; asking it of the block's OWN tail holds at any depth. **Context**: the
> repair is restricted to a bare `{ … }`, the only context handed a `Void` it did not mean —
> a `for` / `while` / `parallel for` / `fields` body gets one because it IS a statement, and
> undoing the drop there leaked one store per round, reopening loft#725. Guard
> `tests/scripts/nested-block-in-value-position.loft`, confirmed to fail at the preceding
> commit on both backends. Fixes loft#1076.

- **Conformance is differential** — call/return is enforced across the two backends by the
  @PLN89 oracle (D-op-1); recursion, nested calls, and struct returns are in its corpus
  (`17-tuples-recursion`, `21-deep-recursion-large-data`, `08-nrvo-mixed-return-paths`). The
  parameter contract (`F-Param*`) is exactly what the ownership register (ownership.md, 0 open)
  and the sandbox raw-write rule ([capabilities.md](capabilities.md), 0 open) are built on, so it
  has the strongest standing cross-checks in the spec.

---

## Conformance

- **Arg order (`F-Args`)** — `add(tag("A"), tag("B"))` prints `AB` before returning.
- **A block yields its tail (`F-Block`)** — `fn f() -> integer { { 5 } }` is `5`, at any
  nesting depth, for every tail kind (a literal, a local, an operator expression, a call, a
  struct or vector literal) and every type; so is a block as an `if` arm, and one after a
  side-effecting statement. A block bound directly by an assignment (`x = { 5 }`) always
  was. The two legitimate discards are the controls: a `for` body's tail (loft#725) and a
  void function's (`F-Drop`). Guard `tests/scripts/nested-block-in-value-position.loft`,
  both backends.
- **Void tail discard (`F-Drop`)** — a function declared void whose body ends in a value runs
  the tail and returns nothing, for every tail type (boolean, integer, text, struct, vector,
  tuple, a narrow `u8`) and every tail shape (a call, a bare literal, an operator expression, a
  struct literal, an `if`, a nested block), in `main` and in an ordinary function alike. The
  `;` form and a block in VALUE position are the controls. Guard
  `tests/scripts/void-fn-value-tail.loft`, both backends.
- **Arity (`F-Arity`)** — `f(1)` for `fn f(a, b: integer)` (no default) → "missing argument for
  parameter 'b' … too few arguments"; `f(1, 2, 3)` for a 2-param `f` → "Too many parameters"; a
  `b = 5` default or a `b: integer?` nullable parameter may be omitted.
- **Scalar copy (`F-ParamScalar`)** — `fn inc(n){ n=n+1 }` leaves the caller's `x==5`.
- **Heap mutate-through (`F-ParamHeap`)** — `fn mut(e: E){ e.h=99 }` makes the caller's `o.h==99`;
  `fn f(v){ v[0]=99 }` makes the caller's `orig[0]==99`.
- **Heap reassign is local (`F-ParamRebind`)** — `fn re(v){ v=[9,9] }` leaves the caller's
  `o[0]==1`; only a `&`-param would write back.
- **Return independence (`F-Ret`)** — `a = mk(); a[0]=99; b = mk()` leaves `b[0]==1`.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on argument
order, a parameter's effect on the caller, or a return's independence is the definitional error
this doc names.
