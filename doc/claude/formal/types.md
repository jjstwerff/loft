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
> authority to mask it — which is *why* range inference (`I-Join`, …) must be sound.
> Narrowing now decides by range containment (`is_narrowing_int`), in agreement with
> codegen's `narrow_int_cast` — so signedness is visible and the old split is closed. The
> one residual (D2) is that the *full integer* is still marked by `forced_size = None`,
> because `IntegerSpec`'s i32/u32 bounds cannot yet hold the i64 range.

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

**Why `(I-Narrow)`'s `as` is consistent with the maker-centric center.** The explicit `as` is
a *write-time intent marker* at a **rare, deliberate edge** — idiomatic code (a well-built
library used as intended) needs **none**, the bar Rust's `usize`-index `as`-tax breaks by
construction and loft holds by having **one integer type, no special index type**. It is the
*right* moment to ask intent (you are in the editor, choosing to drop bits), not the wrong one
(runtime, or the common path). And it coexists with the spreadsheet model: a narrowing that
overflows *at runtime* yields **null and keeps running** (operational.md `E-Uncomp`, C80) —
`as` is compile-time intent, null is runtime keep-going. See
[GOALS.md § the wrong moment](../GOALS.md) and [DESIGN_DECISIONS.md C79/C80](../DESIGN_DECISIONS.md).

### Coercion closure

```
  (C-Only)    ⤳ is the only implicit coercion.  Every other type change is an explicit
              op or cast and appears in the syntax.
```

---

## Deviations

OPEN: **0**

### D2 — CLOSED by reconciliation (2026-06-24): `integer` = i64 is a *user-visible* contract met by a *compact* internal encoding

D2 was framed as a deviation to *remove* by widening the IR (`Value::Int` → i64) so the default
integer is "i64 end-to-end." That framing is **declined** — see
[DESIGN_DECISIONS.md C83](../DESIGN_DECISIONS.md#c83--the-internal-representation-follows-the-user-visible-contract-never-widen-storage-for-implementation-convenience).
The reconciliation:

- **The user-visible contract is met.** `integer` *is* i64 everywhere a user can observe it — a
  boundary matrix (graduated to `tests/scripts/438-integer-i64-user-visible.loft`) confirms a
  value above i32 range survives arithmetic (`* / % -`), bare literals, struct fields, vector
  elements, fn args/returns, comparison, negation, tuples, and field mutation, **identically on
  the interpreter and `--native`**. The runtime computes on `i64` throughout.
- **The internal model is *supposed* to be compact.** `Value::Int(i32)`/`Value::Long(i64)` is a
  deliberate value-size encoding (i32 for the small-value majority, i64 when needed), and
  `forced_size = None` marks the full i64 range. Per **C83** the internal representation *follows*
  the user-visible contract and is memory-bandwidth-conscious — it is **never widened for
  implementation convenience**. Blanket i64 storage would double every integer node/field for
  zero user-visible gain; the earlier "widen `Value::Int`" attempt was correctly **reverted** (it
  introduced a silent `as i32` truncation in a narrow storage path — solving the wrong problem).
- **The rule, restated to match the intended design:** *the default `integer` denotes the i64
  value range; storage uses the smallest sufficient encoding, with `forced_size = None` /
  `Long` as the full-range carriers.* Under this rule the code is **conformant** — `forced_size`
  as the full-integer marker is the intended encoding, not a width hack to remove. Narrowing is
  range-driven (this already closed D3/D5); signedness is correct (`i8` does not fit `u8`); the
  parser agrees with codegen. Guard: `d2_signed_narrowing_i8_to_u8_needs_cast` (tests/issues.rs)
  + the i64 user-visible regression above.

**If** a *user-visible* i64 truncation is ever found (a value a user can observe being clipped),
that narrow path is fixed — still without blanket widening (C83 § Revisit). The site audit in
[plans/88-integer-i64.md](../plans/88-integer-i64.md) remains the reference for any such targeted
fix. @PLN88's storage-rework rungs are **not** pursued (off the path per C83).

---

## Conformance check (how we know a deviation is real)

Each deviation should have a falsifying program — the case where obeying the rule and
obeying the code disagree. Examples on record:

- **D1 (CLOSED):** the four `*_hint` side-channels (`lambda_hint` / `enum_hint` /
  `vector_hint` / `read_target_type`) are consolidated into one `Parser.expected` field with
  shape-dispatching reader methods — one `⇐` channel, not four. (#432's `vector_hint` was the
  symptom of the sprawl.)
- **D4 (CLOSED):** the cbor `read_value` cross-branch `arg` (`arg=bytes[i];
  arg=arg*256+…`) inferred `u8` and overflowed. `(I-Join)` now widens it; the falsifier
  graduated to `tests/scripts/433-ijoin-multiply-assigned.loft`.

When a deviation closes, its falsifying program graduates to `tests/scripts/` and the
entry here is deleted.
