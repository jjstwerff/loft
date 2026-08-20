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
  (T-Ref)     a parameter may be declared `&(τ₁, …, τₙ)`.  It denotes the CALLER's tuple: a
              projection `p.i` reads the caller's element and an assignment `p.i = e` writes it,
              both through the tuple's stored reference at the element's own offset — the same
              `(ref, offset)` pair an ordinary struct FIELD uses (binding.md B-Ref).
  (T-Ref-El)  every τᵢ must be one of `integer` (any width), `float`, `single`, `character`,
              `boolean` — the types laid out for that pair.  Any other element type is a STATIC
              error naming the offending type; it is never a runtime fault and never an ICE.
```

**In words.** `fn sw(p: &(integer, integer)) { t = p.0; p.0 = p.1; p.1 = t }` swaps the caller's
tuple in place — that is what a reference tuple is for. The admitted element types are exactly
the scalars the element opcodes are laid out for, and the boundary is enforced at the SIGNATURE,
so a program either compiles and behaves identically on both backends or is refused where it is
written.

A `text`, collection, struct or function-reference element is refused. This is a layout
limitation, not a missing opcode: `OpGetText` / `OpSetText` exist and take the same
`(ref, offset)`, but a reference tuple's storage is not a record with a text slot the way a
struct is. Use a **struct** instead — its fields of any type write through a `&` parameter —
or take the tuple by value and return a new one. The refusal message says both.

---

## Deviations

OPEN: **1**, and bounded by the oracle note below.

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

> **D-tup-2 — OPEN (2026-08-20) — the admitted-element rule is not asked at every construction
> site.** `T-Ref-El` names which element types a `&(…)` admits, and `data::ref_tuple_element_ok`
> is the single list that answers it. The *signature* path consults it
> (`parser/definitions.rs`), so a `&(text, text)` PARAMETER is refused with the rule's message.
> A `&`-annotated **local** is built at a second site (`parser/expressions.rs`) that never asks:
>
> ```
> a = ("p", "q");
> b: &(text, text) = a;
> b.0 = "x";
> ```
>
> reaches codegen and dies as an internal compiler error on BOTH backends
> (`RefTuplePut: unsupported element type Text`, `state/codegen.rs`). With a struct element it
> reads *"Store access out of bounds … the reference is corrupt"*, and even the ADMITTED
> `&(integer, integer)` reaches an index-out-of-bounds in `database/allocation.rs` — so this is
> not one bad element type, it is the whole `T-Ref` rule going unenforced on the local path.
>
> The fix the rule asks for is to move the check to the **type-construction chokepoint** where a
> `RefVar(Tuple)` is formed, rather than adding a second call beside the first — a second call
> site would be the same shape as the three lists D-tup-1 collapsed.

- **Conformance is differential** — tuples are enforced across the two backends by the @PLN89
  oracle (D-op-1): `17-tuples-recursion` carries construction, projection, destructuring, and
  tuple returns, precisely because the native layout (a synthetic `__tuple<…>` struct, inline
  bytes) differs from the interpreter's. A divergence in element order, value, or type is caught
  there.
- ⚠ **…but the oracle's elements are all `(integer, integer)`.** It carries no `text`, and that
  gap is measured, not theoretical: this doc read `OPEN: 0` through **two** live tuple deviations
  that the differential it leans on could not see — loft#1004 (a tuple's `text` element written
  one index too high: silent wrong element, silent lost write, SIGSEGV) and loft#1005 (a tuple
  `text` parameter that would not compile on `--native` at all). A `text` element is the first
  place the native layout stops being inline bytes, so it is exactly where a layout differential
  earns its keep. Widening `17-tuples-recursion` to a heap element type is the fix; until then
  the zero above is bounded by what the oracle covers.

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
