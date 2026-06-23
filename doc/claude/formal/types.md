<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/types.md — type system & conversion relation (strict)

> **Rules then deviations** (see [README](README.md)). The rules are the target loft's
> front end should satisfy; the deviations are where today's code breaks them, to be
> driven to zero. Analysis/rationale lives in [../TYPING_RELATION.md](../TYPING_RELATION.md)
> (the lens) — entries here link to it instead of re-explaining.

## Notation

- `Γ` — typing context (variable ⟼ type bindings).
- `τ, σ` — types (the `Type` enum, `src/data.rs`).
- `&τ` — a **reference type**: the type of a variable that is a live link to a τ-lvalue
  (read/write-through). A type constructor; its introduction and link semantics live in
  [binding.md](binding.md). In *this* doc it appears only in the conversion relation —
  it reads through to its referent (`C-Ref`).
- `Integer[a,b]` — an integer type with closed value range `[a, b]` (the `IntegerSpec`
  min/max). `integer` = `Integer[i64::MIN, i64::MAX]`; `u8` = `Integer[0, 255]`; etc.
- `τ ⤳ σ` — **conversion**: a value of `τ` is accepted where `σ` is expected, with no
  explicit cast. This is the *only* implicit coercion in the language.
- `⊔` — the **join** (least type containing both); for integers, the range-union's
  enclosing `Integer[min a c, max b d]`.

---

## Rules

### Judgments — bidirectional, two modes only

```
  (T-Syn)   Γ ⊢ e ⇒ τ        e synthesises its own type τ
  (T-Chk)   Γ ⊢ e ⇐ τ        e is checked against an expected τ
  (T-Sub)   Γ ⊢ e ⇒ τ ,  τ ⤳ σ   ⟹   Γ ⊢ e ⇐ σ
```

**In words.** loft works out a type in one of two directions: either the expression tells
us its type (`⇒`, *synthesis*), or the surrounding code already expects a type and we
check the expression against it (`⇐`, *checking*). `(T-Chk)` is the **single** carrier of
"the expected type" — there is exactly one checking mode, pushed structurally into
sub-expressions (literals, lambda bodies, variant references, read targets). There is no
other channel for an expected type.

```
  (T-Chk-Vec)    Γ ⊢ [e₁ … eₙ] ⇐ vector<τ>   ⟸   ∀i. Γ ⊢ eᵢ ⇐ τ
  (T-Chk-Lam)    Γ ⊢ (\x.e) ⇐ fn(σ)→τ         ⟸   Γ, x:σ ⊢ e ⇐ τ
  (T-Chk-Var)    Γ ⊢ V ⇐ Enum E               ⟸   V ∈ variants(E)
```

### Conversion `τ ⤳ σ` — width folded in

```
  (C-Refl)    τ ⤳ τ
  (C-Never)   Never ⤳ τ
  (C-Tuple)   (σ₁…σₙ) ⤳ (τ₁…τₙ)        ⟸   ∀i. σᵢ ⤳ τᵢ
  (C-Var)     Reference(S) ⤳ Enum(E)   ⟸   S ∈ variants(E)            (and the
              dual Enum(__nullable<S>) ⤳ Reference(S), and plain Enum ⤳ Integer tag)
  (C-Int)     Integer[a,b] ⤳ Integer[c,d]   ⟸   [a,b] ⊆ [c,d]         (see I-*)
  (C-Ref)     &τ ⤳ σ   ⟸   τ ⤳ σ      (a reference reads through to its referent; there
              is NO  σ ⤳ &τ  — a reference is made only by `&` at a binding, never coerced)
```

**In words.** `τ ⤳ σ` is loft's *only* automatic conversion — "a `τ` value is fine where a
`σ` is wanted, no cast needed." The rules list the safe cases: the same type; a `Never`
(a `return`/`break`, which fits anywhere); tuples element-by-element; a struct used as one
of an enum's variants (and the nullable/tag duals); and an integer into a *wider* integer.
`(C-Int)` means **width lives inside `⤳`**: an integer flows into another integer iff its
range fits. There is no separate width authority. `is_equal` answers only the *width-free*
base-type question ("is this `integer`?" — correctly *yes* for every width); `convert` and
codegen read width from this one relation, never their own copy (see the integer model
below). `(C-Ref)` threads in
references: a `&τ` variable *has* type `&τ` and reads **through** to a `τ`, so it is
usable wherever a `τ` is, and `τ`'s own conversions then apply (e.g. `&u8` → `integer`).
The reverse never holds — you cannot coerce a plain value into a reference; a `&τ` is made
only by a `&` annotation at a binding. The link's introduction and its write-through
semantics live in [binding.md](binding.md); here it is just one more thing `⤳` accepts.

### The integer model — one type; the rest is notation

> **`integer` is the only integer type.** Its identity is its value range `[min, max]`
> (plus `not_null`). `u8` / `i8` / `u16` / `i16` / `i32` are **not** distinct types — they
> are **notation** for an `integer` whose range fits in fewer than 8 bytes. The value
> semantics have no per-width type; a width is a *consequence* of the range.
>
> **Storage width is derived, never declared.** The bytes a value occupies are a pure
> function of its range — `bytes_for_range(min, max)`: `[0,255]` ⟹ 1 byte, the full range
> ⟹ 8. `IntegerSpec.forced_size` is a **cache of that function**, not an independent fact:
> it must always equal `bytes_for_range(min, max)`. It is a storage hint, not a value-type
> (the field's own doc says exactly this).
>
> **Why this settles the width question.** Width has one home — the range, read through
> `⤳` / `(C-Int)`. So `is_equal` collapsing every `Integer(_)` to one type is **correct**
> ("is this `integer`?" carries no width); narrowing (`I-Narrow`) and the codegen cast both
> **derive from the range**, not from `forced_size`. A too-narrow range is then
> unambiguously a bug — undersized storage means a silent overflow, with no second
> authority to mask it — which is *why* range inference (`I-Join`, …) must be sound. The
> deviations D2/D3/D5 are the spots where the code still treats `forced_size` (or a
> per-site copy) as a width authority instead of a derived cache of the range.

### Integer width

```
  (I-Sub)     Integer[a,b] <: Integer[c,d]   ⟺   [a,b] ⊆ [c,d]
              and  <:  is exactly the implicit  ⤳  on integers (C-Int).
  (I-Widen)   widening (a superset target) is implicit.
  (I-Narrow)  narrowing (a non-superset target) is NOT implicit: it needs either
                – an explicit  e as σ , or
                – e is a literal whose value ∈ range(σ).
  (I-Lit)     an integer literal n  has every type Integer[a,b] with a ≤ n ≤ b
              (it checks at the expected width; it does not force i64).
  (I-Join)    a variable assigned e₁ … eₙ in a scope has type  ⨆ᵢ τᵢ  where τᵢ are the
              synthesised assignment types.  (Its width is the join of all writes,
              never just the first/narrowest.)
```

**In words.** An integer's type is its value *range*. A narrower integer fits a wider one
for free (`u8` flows into `integer`); the other way round (wider into narrower) needs an
explicit `as`, unless the value is a literal that plainly fits. A literal takes whatever
width is expected of it. And a variable written in several places gets the type big enough
for *all* its writes (the join) — not just its first one. That last point was the
#433-residual; `(I-Join)` is now implemented for inferred locals (an inferred local widens
to the join when a write would not fit; an annotated `x: u8` stays constrained), guarded by
`tests/scripts/433-ijoin-multiply-assigned.loft`.

### Coercion closure

```
  (C-Only)    ⤳ is the only implicit coercion.  Every other type change is an explicit
              op or cast and appears in the syntax.
```

---

## Deviations

OPEN: **4**

### D1 — four expected-type side-channels instead of one checking judgment
- **Violates:** T-Chk (and its T-Chk-* instances)
- **Where:** `src/parser/mod.rs` — `Parser.lambda_hint`, `Parser.enum_hint`,
  `Parser.read_target_type`, `Parser.vector_hint`
- **Effect:** the checking mode is hand-threaded per syntactic position; a position
  nobody wired is the next #432 (the `vector_hint` channel was literally added *as*
  the #432 fix). See [../TYPING_RELATION.md](../TYPING_RELATION.md) § R1.
- **Status:** OPEN
- **Removal:** thread a single `expected: Option<Type>` (the `⇐` mode) through the
  expression parser; delete the four fields.

### D2 — `forced_size` is read as a width authority, not derived from the range
- **Violates:** the integer model (width lives in the range; `forced_size` is a cache)
- **Where:** `is_narrowing_int` (`src/parser/mod.rs:1467`) keys on `forced_size`; codegen's
  `narrow_int_cast` (`src/generation/mod.rs:571`) keys on `range()` — the one width,
  derived two ways. See [../TYPING_RELATION.md](../TYPING_RELATION.md) § R2.
- **Effect:** the two derivations can disagree at a narrow range with `forced_size = None`
  (exactly the #433 edge). `is_equal` collapsing every integer is **not** this bug — that
  is the correct width-free base-type answer (see the integer model).
- **Status:** OPEN
- **Removal:** `is_narrowing_int` reads range-containment (`I-Sub`); `forced_size` collapses
  to `bytes_for_range(min,max)`, consulted only at the storage seam, never as a width source.

### D3 — narrowing is a diagnostic bolted onto `convert`, not the range relation deciding
- **Violates:** I-Narrow (placement — the rule is correct, its home is wrong)
- **Where:** `src/parser/mod.rs::convert` ~1559 (`is_narrowing_int` + `int_value_fits`
  → an Error emitted *beside* the `is_equal` accept)
- **Effect:** the accept (base-type, via `is_equal`) and the width check sit apart, and the
  width check reads `forced_size` (D2) rather than the range. The single relation
  `⤳`/`(C-Int)` should itself accept-or-reject by range containment, not lean on a bolted-on
  diagnostic.
- **Status:** OPEN
- **Removal:** make `(C-Int)` (range containment) the accept/reject; the narrowing "error"
  is just "range not contained," with the literal-fits exemption part of the rule.

### D5 — integer width is derived in two places that must agree by hand
- **Violates:** the integer model (width has one home — the range)
- **Where:** `convert`/`is_narrowing_int` (derives width from `forced_size`) and
  `narrow_int_cast` in `src/generation/` (derives it from `range()`). (`is_equal` collapsing
  integers is the correct base-type answer, not a third authority.)
- **Effect:** #433 and its residual are both "these two derivations disagreed." Forward-bug
  generator until both read the range.
- **Status:** OPEN (closing D2 + D3 collapses this)
- **Removal:** both derive width from the range (`I-Sub`); `forced_size` is a cache used
  only for storage layout, never as a width source.

---

## Conformance check (how we know a deviation is real)

Each deviation should have a falsifying program — the case where obeying the rule and
obeying the code disagree. Examples on record:

- **D1:** `fn f(b: vector<u8>) -> integer { return (b[1] ?? 99); }` — interp obeys the
  rules, `--native` (pre-fix) obeyed the code and E0308'd. (#432, #433.)
- **D4 (CLOSED):** the cbor `read_value` cross-branch `arg` (`arg=bytes[i];
  arg=arg*256+…`) inferred `u8` and overflowed. `(I-Join)` now widens it; the falsifier
  graduated to `tests/scripts/433-ijoin-multiply-assigned.loft`.

When a deviation closes, its falsifying program graduates to `tests/scripts/` and the
entry here is deleted.
