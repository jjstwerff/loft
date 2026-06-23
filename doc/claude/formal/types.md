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

### Coercion closure

```
  (C-Only)    ⤳ is the only implicit coercion.  Every other type change is an explicit
              op or cast and appears in the syntax.
```

---

## Deviations

OPEN: **1**

### D2 — the full integer is marked by `forced_size = None`, not a canonical maximal range
- **Violates:** the integer model (width should be the range alone)
- **Where:** `is_narrowing_int` (`src/parser/mod.rs`) now decides narrowing by **range
  containment** for a forced (narrow) target — the same range+sign test codegen's
  `narrow_int_cast` uses, so the two agree (this closed the old D3/D5). But it must still
  treat `forced_size = None` as "full integer → never narrows": `IntegerSpec`'s i32/u32
  bounds cannot represent the i64 range, and the full integer has several bound encodings
  (`signed32` max = i32::MAX, `wide` max = u32::MAX), so pure containment flags false
  narrowings there. `forced_size = None` is thus still a width *marker*, not a pure range
  fact. See [../TYPING_RELATION.md](../TYPING_RELATION.md) § R2.
- **Effect:** width is range-driven for narrow types — signedness is now correct (`i8` does
  not implicitly fit `u8`) and the parser agrees with codegen. The one residual: "is this
  the full integer?" rides on `forced_size`, not on the bounds. Guard:
  `d2_signed_narrowing_i8_to_u8_needs_cast` (tests/issues.rs).
- **Status:** OPEN — the residual after the D3/D5 close. Tracked as
  [@PLN88](https://github.com/loft-lang/plans/issues/88) (loft-lang/plans#88).
- **Removal — three layers; the bottom is an IR change (each found by attempting it):**
  1. *Storage migration — mechanical, builds clean.* `IntegerSpec` **i64 bounds** ripple into
     ~35 i32-assuming sites: `Parts::Byte/Short(i32)` (DB storage-type enum), the
     `byte`/`short`/`int` ops (`min: i32`), the i32 casts at storage boundaries; `range()`
     needs i128 (the full-i64 count overflows i64); `usable_min`/`usable_max` widen to i64;
     the `is_wide`/`*_template` predicates compare bound literals (`u32::MAX` → `i64::MAX`).
     Narrow-gated, so the casts type-check. **Not the obstacle.**
  2. *Two `integer` ranges.* The default `integer` keyword resolves to **`I32 = signed32()`**
     — an **i32**-range template with `forced_size = None` (`src/typedef.rs` `"integer" => I32`)
     — while **`I64 = wide()`** is a *separate* i64 template; the `forced_size = None` guard in
     `is_narrowing_int` treated them as interchangeable. Pure range containment correctly
     distinguishes them (`wide ⊄ signed32`), so a `wide` value (e.g. an `(I-Join)`-widened
     local) assigned to an `integer` (`signed32`) destination narrows — *"cannot narrow
     integer to integer"*. Unifying = `signed32() → wide()`, fix the predicates
     (`is_wide`/`is_signed32_template`/`is_wide_template`), and collapse `__cell_long` into
     `__cell_integer` (`src/parser/vectors.rs`). This too **builds clean**.
  3. *The bottom — the IR assumes the default integer is i32.* With the model unified to i64 it
     **breaks at runtime/codegen**: `Value::Int(i32)` (the IR integer node) cannot hold a large
     value (`9_000_000_000` → *"literal out of range for i32"*), and a default integer — now
     `wide`, `min = i64::MIN+1` — flowing into a narrow storage op has its min cast `as i32`,
     **silently truncating** `i64::MIN+1` (emitted Rust: `db.byte(-9223372036854775807, …)`).
     The boundary casts *mask* this; they do not fix it. Closing D2 therefore bottoms out in an
     **IR change** — `Value::Int` must carry i64 (or default integers route through a `Long`/i64
     IR path) — so the default integer is genuinely i64 end-to-end, not i32 with a wide static
     range bolted on. That is the true depth of "make `integer` 64-bit": a fundamental IR /
     codegen change with the full suite + both backends as the net, **not** a type-or-cast
     change. (Attempted; reverted at this boundary — the `as i32` truncation is silent
     store corruption, exactly what the discipline says not to ship.)

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
