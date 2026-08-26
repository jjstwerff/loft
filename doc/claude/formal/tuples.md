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

### Reference tuples — `&(…)` writes the caller's elements in place

```
  (T-Ref)     a BINDING may be declared `&(τ₁, …, τₙ)` — a parameter, or a local written
              either way binding.md B-Ref-Intro allows (`b: &(…) = a`, `b = &a`).  It denotes
              the bound tuple itself: a projection `p.i` reads that tuple's element and an
              assignment `p.i = e` writes it, both through the tuple's stored reference at the
              element's own offset — the same `(ref, offset)` pair an ordinary struct FIELD
              uses (binding.md B-Ref).  For a parameter the tuple is the CALLER's; for a local
              it is the source variable's, and both are stack-backed, so the two positions are
              one mechanism and not two.
  (T-Ref-El)  every τᵢ must be one of `integer` (any width), `float`, `single`, `character`,
              `boolean` — the types laid out for that pair.  Any other element type is a STATIC
              error naming the offending type; it is never a runtime fault and never an ICE.
              The rule is asked wherever the `&` is WRITTEN, so a `&(…)` a signature refuses
              cannot be accepted at a local.
  (T-Ref-Src) the source of a `&(…)` local is a tuple VARIABLE.  A tuple ELEMENT or FIELD
              (`b = &v[0]`, `b = &s.pair`) is a STATIC error: a tuple place is read element by
              element into a fresh by-value tuple, so no place survives for the link to name.
              Declining is binding.md B-Ref-Reshape's rule — where the link cannot be honoured
              loft refuses the program rather than downgrading it to a copy.
```

**In words.** `fn sw(p: &(integer, integer)) { t = p.0; p.0 = p.1; p.1 = t }` swaps the caller's
tuple in place — that is what a reference tuple is for. The same annotation on a LOCAL means the
same thing (`a = (1, 2); b: &(integer, integer) = a; b.0 = 5` leaves `a.0 == 5`), because both
name a tuple sitting in a frame and reach it the same way. The admitted element types are exactly
the scalars the element opcodes are laid out for, and the boundary is enforced wherever the `&`
is written, so a program either compiles and behaves identically on both backends or is refused
where it is written.

The restriction belongs to the STACK-backed reference tuple this annotation builds. The
record-backed one a `for` loop binds over a `vector<(…)>` is a different construction reaching a
real record, and it admits any element type — `for t in [("a", "b")] { t.0 }` is correct on both
backends, and writing `t.0` there reaches the vector. Reading `T-Ref-El` as a fact about tuples
rather than about this binding is the mistake that boundary invites.

A `text`, collection, struct or function-reference element is refused. This is a layout
limitation, not a missing opcode: `OpGetText` / `OpSetText` exist and take the same
`(ref, offset)`, but a reference tuple's storage is not a record with a text slot the way a
struct is. Use a **struct** instead — its fields of any type write through a `&` parameter —
or take the tuple by value and return a new one. The refusal message says both.

---

## Deviations

OPEN: **0** (2026-08-26) — D-tup-3 opened and closed 2026-08-26; D-tup-2 closed the day the
rule it needed was written down.  Bounded by the oracle note below — **and D-tup-3 is what that
note was warning about**: it was found by giving an element a HEAP type, which this doc's
all-`(integer, integer)` oracle cannot express, so the zero above never covered it.

> **D-tup-3 — OPENED AND CLOSED (2026-08-26, loft#1104) — a tuple element is a projection that
> the ownership machinery could not read as one.** `(T-Proj)` says `t.i` is element `i`, and for a
> heap element that means a `DbRef` into the store the element lies in — the same thing `b.s` and
> `v[0]` are. The @P290 borrow-vs-owned bracket could not see it, so a call whose return may
> borrow the argument kept its conservative answer and LEAKED one record per call, both backends:
>
> ```loft
> fn pick(s: S, c: boolean) -> S { if c { s } else { mk() } }
> fn f(c: boolean) -> integer { s = S { a: 7 }; t = (s, 9); r = pick(t.0, c); r.a }   // 1 record / call
> ```
>
> `pick(q, …)`, `pick(b.s, …)` and `pick(v[0], …)` were all clean. The bracket protects a store by
> naming it through a variable whose VALUE is a `DbRef`, and `view_root_slots` walks a projection
> chain to that variable using `is_projection_op` — which is keyed on `OpGetField` / `OpGetVector`.
> A tuple element is neither: it is `Value::TupleGet`, not a `Call` at all.
>
> **Two cures are unavailable, and which ones is the useful part.** Widening the op list cannot
> reach a shape that is not an op. Naming the TUPLE cannot work either — the bracket protects the
> store a `DbRef` variable points at, and a tuple is not a `DbRef`; its ELEMENT carries the store.
> So the argument is bound to a temp typed as the tuple element itself, deps and all, which is
> exactly the hand-written spelling that was always clean (`e = t.0; pick(e, …)`) and emits the
> same code — the argument loft#1029 used for the inline-construction family, one spelling over.
> Closed in `Scopes::scan_args` (`tuple_element_borrow_source`), gated exactly as its sibling is: a
> heap-carrying element, at a `returns_borrowed_view` callee, and nothing else — binding an
> argument reorders it relative to its left-hand siblings, which is a cost worth paying only where
> the alternative is a leak.
>
> ⚠ **The bare `t.0` is one cell of six, and the other five were found by moving the axes the
> first sweep pinned** — the chain's OP, the container the tuple sits in, and the index.
> `pick(t.0.s, …)`, `pick(t.0[0], …)` and `pick(t.1.s, …)` put a projection CHAIN above the
> element; `pick(t.0.0, …)` and `pick(vt[0].0, …)` read the element off something that is not a
> plain variable, which the parser lowers to a `tuple_tmp` block; and `pick(t.0.0.s, …)` is both
> at once, invisible until the block shape had a cure. **WHICH NODE gets the name is the whole
> distinction, because it decides the type the temp carries.** A chain is RE-BASED on the temp
> rather than bound: the ELEMENT's type is one the tuple declares, while the chain's RESULT type
> would have to be inferred, and a temp typed off the CALLEE'S PARAMETER instead carries no deps —
> it then reads as an OWNER of a store it only views, and the free that follows is a
> use-after-free rather than a leak (QUALITY.md § B6k).
>
> ⚠ **The class, and this is its fourth instance in a week: one notion, two spellings, one looked
> for.** A projection resolved by OP NAME cannot see the `TupleGet` spelling; the same blindness
> reaches `Parser::expr_borrows_local` (latent there — the deps leg covers what the op list
> cannot). The blindness is not findable from the symptom: searching for the spelling you DO match
> returns every site that gets it right, and the sites that get it wrong contain nothing to search
> for. `scripts/ir_walker_audit.py spellings` counts the class — **38 functions resolve a
> projection by op name and 4 handle the tuple spelling** (18 · 2 when this entry was first
> written; the SCREEN was widened, not the family — it could see two of the three ways Rust
> resolves an op here and was blind to the `data.def(d).name() == "OpGetField"` form every
> hand-spelled list in the tree uses). See `IMPLEMENTATIONS.md` § *One notion, how many
> SPELLINGS?*
>
> **Measured.** Nine cells, both backends, values identical before and after — this is a pure
> leak, so `--interpret` under `LOFT_STRICT_STORES=1` is the instrument and the assertions score
> nothing. On a control binary built at `9c1a0e4e` the two record-element cells report
> `kt=78 S1104×50` over 25 rounds each; after, clean, and clean under `LOFT_POISON=1` too.
> Emitted IR over the corpus: **no existing program changes** — only the guard. Controls: the
> three already-nameable spellings, the hand-written binding, a SCALAR tuple element (which
> carries no store and must not be bound) and a callee that does not return a borrowed view.
> Guard: `tests/scripts/1104-a-tuple-element-argument-borrow-witness.loft`, scored by the wrap
> harness's leak gate — `loft --tests` cannot fail it even with `LOFT_STRICT_STORES=1`.

> **D-tup-1 — CLOSED (2026-08-20) — the reference tuple has a rule.** This doc specified
> construction, projection, destructuring and returns and said nothing about `&(τ₁, …, τₙ)` —
> the composition of `&` ([binding.md](binding.md)) with a tuple. Both halves were specified and
> their composition was not, which is how the two backends came to represent it differently with
> nothing to catch them (`--native`: a Rust stack tuple by `&mut`; interpreter: a record through
> a DbRef), and how loft#1006 reached codegen as an internal compiler error.
>
> `T-Ref` / `T-Ref-El` above now state what a `&(…)` denotes and which element types it admits.
> Extending the rule is what the [README](README.md) doctrine asks for at an edge the rules
> cannot express, and writing it down is what showed the admitted set had been **three lists that
> disagreed**: the signature guard admitted `single` and a function reference that codegen then
> died on, and refused `boolean`, which every layer could always have handled. There is one list
> now (`data::ref_tuple_element_ok`), read by the guard and by both `RefTupleGet` / `RefTuplePut`
> arms, so the rule and the implementation cannot drift apart again. Measured on both backends
> across all five admitted element types plus the four refused ones. Tracked against binding.md's
> D-bind-11, which carries the measurement.
>
> ⚠ **The last sentence was too strong, and D-tup-2 below is why.** One list is necessary and was
> not sufficient: a list is only consulted where somebody calls it, and only one of the two sites
> that build a `RefVar(Tuple)` does.

> **D-tup-2 — CLOSED (2026-08-23) — the admitted-element rule is now asked at every
> construction site, and the local path it exposed is implemented.** `T-Ref-El` names which
> element types a `&(…)` admits and `data::ref_tuple_element_ok` is the single list that answers
> it, but only the *signature* path consulted it. `Parser::ref_var_type` is now the one place a
> `&` in source becomes a `Type::RefVar`, so the parameter, the annotated local and the inferred
> `b = &a` all ask it, and a `&(…)` a signature refuses cannot be accepted at a local. Guard
> `tests/scripts/reference-tuple-local-binding.loft` (what must work) +
> `102-expected-errors.loft` (the four refusals); proven to fail on a pristine tree at
> `1e9d7910` — 6 of 7 cells on `--interpret`, 7 of 7 on `--native`.
>
> ⚠ **The entry named the ICE, and the ICE was the mild half.** Measured across positions and
> element types rather than at the filed cell, the whole `&(…)` LOCAL was unimplemented, at every
> element type including the admitted ones, and the loudness varied with what the tuple happened
> to hold:
>
> | written | was |
> |---|---|
> | `b = &a` | the `&` was **DROPPED**: the IR typed `b` a plain tuple and copied it, so `b.0 = 5` left `a` untouched, silently, on both backends |
> | `b: &(integer, integer) = a` | typed a reference over a value — the interpreter read an ELEMENT as a store index (`(7, 9)` gave *"index is 9"*) and `--native` handed the user a raw rustc `E0308` |
> | `b: &(boolean, boolean) = a` | answered `truefalse` where the swap says `falsetrue`, **exit code 0** |
> | `b: &(float, float) = a` | answered `null` for a present element |
> | `b: &(text, text) = a` | the filed ICE |
>
> So the register read `OPEN: 1` against a `silent-wrong` and a wrong-answer cell that no
> deviation named, because the entry inherited the ICE from the report that raised it. **Both
> backends agreed on every one of those**, which is why the tuple differential the doc leans on
> (D-op-1) was structurally blind: the two implementations were wrong in the same way.
>
> The fix is the one the rule asked for — the chokepoint, not a second call beside the first —
> plus the mechanism the chokepoint then had to have something to admit: a tuple local lives in
> the FRAME, so it joins the scalars at `OpCreateStack`, which is exactly the stack ref a `&(…)`
> PARAMETER is already handed at its call site. Native represents the local link as the raw
> `*mut (…)` @PLN87 L1 gives every local link (raw so the source stays readable beside it, which
> is legal loft and not legal Rust borrowing), and two sites now read one predicate,
> `generation::is_raw_tuple_link`, to decide it — the element base and the call that forwards
> the local to a `&(…)` parameter.
>
> ⚠ **`T-Ref-El` is a fact about this BINDING, not about tuples.** Measured while picking the
> chokepoint: the record-backed `RefVar(Tuple)` a `for` loop builds over a `vector<(text, text)>`
> reads and WRITES its elements correctly on both backends. It reaches a real record, so the
> layout limitation the refusal exists for does not apply to it. Putting the gate in a universal
> `RefVar(Tuple)` constructor would have refused a shape that works — which is why the
> chokepoint is *the `&` written in source*, and why `T-Ref` now says stack-backed out loud.
>
> The one shape left refused rather than linked is a tuple PLACE (`b = &v[0]`, `b = &s.pair`),
> now `T-Ref-Src`. It used to bind silently to a COPY — `b.0 = 9` wrote the copy and the source
> was unchanged, with no diagnostic and both backends agreeing. B-Ref-Reshape settles what to do
> there: loft declines rather than downgrading a reference to a copy.

> **D-tup-3 — CLOSED (2026-08-20) — a nullable element at a tuple POSITION.** This doc
> specified construction, projection, destructuring and returns, and `types.md` @PLN25
> `(N-Decl)` specified that a non-null `τ` stored into a `τ?` slot is not a type change.
> Their composition was not specified, and `(N-Decl)` peeled one `Optional` at the TOP, so
> a `τ?` sitting at a tuple position was never seen: `c: (text?, integer) = ("c0", 3)` was
> refused as a declared LOCAL while the identical type was accepted as a RETURN (loft#1034).
>
> That is D-tup-1's shape a second time — two specified halves, an unspecified composition,
> two sites answering differently with nothing to catch them. `(N-Decl)` now reads
> element-wise (`Variables::decl_accepts`, recursive through nested tuples), and the
> assignment path routes a tuple target through the SAME `convert` the return position
> always used, rather than growing a second opinion beside it.
>
> ⚠ **The refusal was the loud half.** The silent half was that a `null` ELEMENT was never
> converted to the element type's sentinel — it stored the empty text and answered `false`
> to `== null`. A fix that only widened the typing check would have turned a compile error
> into a wrong answer, which is why the guard's null-element cell is load-bearing.
>
> Direction preserved: the widening is `τ → τ?` only, so `(text, integer) ← (text?, integer)`
> remains the `(N-Store)` violation.

- **Conformance is differential** — tuples are enforced across the two backends by the @PLN89
  oracle (D-op-1): `17-tuples-recursion` carries construction, projection, destructuring, and
  tuple returns, precisely because the native layout (a synthetic `__tuple<…>` struct, inline
  bytes) differs from the interpreter's. A divergence in element order, value, or type is caught
  there.
- ⚠ **…and it carries no NESTED tuple with a `fn(…)` inside it — a second axis, measured
  2026-08-22.** `t: ((fn(integer) -> integer, integer), text) = ((dbl, 1), "z")` — a program
  with no assignment anywhere in it — panicked `fn_call_ref: fn_var=16 < 20` on the
  interpreter and was refused by rustc on `--native`, while the cell that touched no
  function at all (reading the plain members beside it) failed hardest, with an ICE. Depth
  was the axis loft#1069's own fix held fixed: it taught the tuple literal that a fn-ref
  member is the whole 20-byte pair and read the TOP-LEVEL members only, so everything it
  repaired was broken again one level in. Three sites had that shallow reading — the
  interpreter's literal push, the native emitter's declared-slot hand-down (and its gate),
  and the native fn-ref reachability walk — and all three now decide with ONE predicate,
  `data::tuple_carries_fn_ref`, which sees through nesting. That it is one function and not
  three copies is the D-tup-1 lesson applied before it could bite: three lists that
  disagreed is exactly what loft#1006 was. Guard
  `tests/scripts/fn-ref-in-a-nested-tuple.loft`, proven to fail on a pristine tree on both
  backends. The two REFUSALS left at this position — a short lambda not inferred inside a
  nested literal, and a forward-referenced fn name not resolving in any tuple literal — were
  loft#1073, and are closed (2026-08-22, guard
  `tests/scripts/tuple-literal-member-fn-inference.loft`). Both were the same shape one level
  in: `(T-Chk)`'s push read the TOP-LEVEL members, so a member that merely CONTAINS a
  `fn(…)` seeded nothing; and `change_var_type` accepted a bare `Unknown` source as pass 1's
  placeholder but not the same fact inside a composite, so `(later, 1)` was measured against
  the declared type and refused — the mirror of loft#944, which made that statement about the
  variable's own type.
- ⚠ **…but the oracle's elements are all `(integer, integer)`.** It carries no `text`, and that
  gap is measured, not theoretical: this doc read `OPEN: 0` through **two** live tuple deviations
  that the differential it leans on could not see — loft#1004 (a tuple's `text` element written
  one index too high: silent wrong element, silent lost write, SIGSEGV) and loft#1005 (a tuple
  `text` parameter that would not compile on `--native` at all). A `text` element is the first
  place the native layout stops being inline bytes, so it is exactly where a layout differential
  earns its keep. Widening `17-tuples-recursion` to a heap element type is the fix; until then
  the zero above is bounded by what the oracle covers.
- ⚠ **`(T-Cons)` says nothing about OWNERSHIP, and the third element type shows why that is a
  gap rather than a silence.** Given a heap LOCAL, a tuple literal stores its handle while a
  struct literal and a vector literal both COPY (`t = (vl, 9)` sees a later `vl[0] = 41`;
  `S { v: vl }` and `[vl]` do not, both backends). So a tuple element is aliased without the
  `&` that [binding.md](binding.md) `B-Copy` says aliasing requires — while `(T-Ref-El)` above
  REFUSES a collection element in the `&(…)` form that asks for it. Which of the two answers is
  the rule is an open design question (**loft#1102**); either way `(T-Cons)` owes a clause, and
  the `OPEN: 0` above does not cover this because the oracle carries no collection element
  either.

---

## Conformance

- **Construct + project (`T-Cons` / `T-Proj`)** — `t = (3, 7); t.0` is `3`, `t.1` is `7`.
- **Destructure (`T-Destr`)** — `(a, b) = (5, 9)` binds `a=5, b=9`.
- **Tuple return + unpack (`T-Ret` + `T-Destr`)** — `fn pair() -> (integer,integer) { (2,3) }`,
  `(x, y) = pair()` binds `x=2, y=3`.
- **Reference tuple (`T-Ref`)** — `fn sw(p: &(integer, integer)) { t = p.0; p.0 = p.1; p.1 = t }`
  swaps the CALLER's tuple: `(1,2)` reads back `2,1`. Verified on both backends for every
  admitted element type — `integer`, `float`, `single`, `character`, `boolean` — uniform and
  mixed (`&(integer, boolean, character)`), and at width 3 so the last element is reached
  (`tests/scripts/1006-reference-tuple-element-types.loft`).
- **Refused element types (`T-Ref-El`)** — `&(text, …)`, `&(fn() -> τ, …)` and a struct element
  are STATIC errors naming the element type, never an ICE
  (`tests/scripts/102-expected-errors.loft`).
- **Static index (`T-Proj`)** — `t.5` on a 2-tuple is a compile error, not a runtime null.

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on a
tuple's element order, values, or a projection is the definitional error this doc names.
