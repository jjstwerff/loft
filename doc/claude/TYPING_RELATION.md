<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# TYPING_RELATION.md — loft's typing & conversion relation, written as rules

> **The FORMALIZATION.md top-leverage instrument (rough spot #2).** loft has no
> typing judgments; the closest artifact is the `convert` / `cast` / `can_convert`
> trio plus an ad-hoc conversion table and **four** parallel "expected type"
> side-channels. This doc writes that relation down as rules — not to canonize it,
> but so the gaps show up as *unprovable cases* instead of as the next #432/#433.
>
> Grounded in `src/parser/mod.rs` (`convert` ~1552, `cast` ~1848, `can_convert`
> ~1934), `src/parser/operators.rs` (the `??` result-type rule ~1312), and the four
> hint fields on `Parser` (`lambda_hint`, `enum_hint`, `read_target_type`,
> `vector_hint`). Companion to [FORMALIZATION.md](FORMALIZATION.md) (the lens) and
> [INCONSISTENCIES.md](INCONSISTENCIES.md) (the decided edges).

## 0. The shape of the problem

Two symptoms say "the typing relation is implemented, not specified":

1. **Four expected-type side-channels.** `lambda_hint`, `enum_hint`,
   `read_target_type`, `vector_hint` are four ad-hoc encodings of *one* idea — "the
   type this expression is being checked against." #432 was fixed by adding the
   *fourth* (`vector_hint`); the smell is that there is no single "checking mode" to
   carry it.
2. **A conversion table, not judgments.** `convert` is a 30-branch cascade; whether
   `A` flows to `B` is decided by `is_equal` plus a pile of special cases, with the
   integer-narrowing *error* layered on top as a side effect.

The rules below name both so the missing cases are visible.

## 1. Judgment forms (the unification)

A principled front-end has exactly **two** modes (bidirectional typing):

```
  Γ ⊢ e ⇒ τ      synthesis  — e determines its own type τ
  Γ ⊢ e ⇐ τ      checking   — an expected τ is pushed into e
```

The four hint fields are all the **checking** mode τ for a specific syntactic
position, threaded by hand instead of by the judgment:

| side-channel        | the checking position it encodes                       |
|---------------------|--------------------------------------------------------|
| `lambda_hint`       | `Γ ⊢ (\x. e) ⇐ fn(σ)→τ` — param/return types of a lambda |
| `enum_hint`         | `Γ ⊢ Variant ⇐ Enum` — bare variant against its enum   |
| `read_target_type`  | `Γ ⊢ read(...) ⇐ τ` — the destination of a parse/read  |
| `vector_hint`       | `Γ ⊢ [e, …] ⇐ vector<τ>` — element type of a literal    |

> **Rough spot R1.** Four fields = four positions someone remembered to thread. The
> checking judgment `⇐` is total over the syntax; the side-channel set is not — the
> next literal position that needs an expected type is the next #432.

## 2. The conversion relation  `τ ⤳ τ′`  (implicit) and  `τ ⟶as⟶ τ′`  (explicit)

`τ ⤳ τ′` = "a value of `τ` is accepted where `τ′` is expected, no cast." Read off
`convert`/`can_convert` (the `convert` cascade is the implicit relation; `cast` adds
the `as`-sanctioned widenings/narrowings).

```
 (C-Refl)    τ ⤳ τ                              ─ is_equal(τ, τ′); NOTE: is_equal
                                                  treats ALL Integer(_) as one (R2)
 (C-Never)   Never ⤳ τ                          ─ return/break/continue
 (C-Rewr)    Rewritten(τ) ⤳ τ′  ⟸  τ ⤳ τ′       ─ strip inline-ctor wrapper (both sides)
 (C-Tuple)   (σ₁..σₙ) ⤳ (τ₁..τₙ)  ⟸  ∀i. σᵢ ⤳ τᵢ
 (C-Ref→En)  Reference(S) ⤳ Enum(E, ref)        ─ if S is a variant of E
 (C-Null)    Enum(__nullable<S>, ref) ⤳ Reference(S)   ─ dense-slot payload (@PLN25 E2)
 (C-EnTag)   Enum(E, value) ⤳ Integer(_)        ─ plain enum tag rides the scalar
 (C-Bare)    Sorted/Hash/Index/Spacial(..) ⤳ Reference(bare collection)
```

> **Rough spot R2 — the integer collapse.** `is_equal` collapses every
> `Integer(spec)` to one type, so `(C-Refl)` makes *any* integer flow to *any* other
> at the relation level. Width is then re-imposed OUTSIDE the relation, as a
> diagnostic (§3). So "does `u8` convert to `i64`?" has two answers depending on
> which function you ask — the conversion relation says yes unconditionally; `convert`
> says yes-but-maybe-an-error. A real rule would fold width INTO `⤳`.

## 3. Integer width — where #432 / #433 live

Width is not in `⤳`; it is a separate gate inside `convert`:

```
 (W-Widen)    Integer(s) ⤳ i64                         always (narrow → wide is free)
 (W-Narrow)   Integer(s) ⤳ Integer(s′),  s′ ⊂ s        ERROR unless:
                – the value is a literal that provably fits  (int_value_fits), or
                – an explicit `as s′`                         (cast, not convert)
```

`is_narrowing_int` keys on `forced_size`; `int_value_fits` is the constant-folding
escape hatch. The two open rules:

- **#432 — literal element width comes from the checking type.** `[10, 20, 30]`
  passed to a `vector<u8>` parameter must elaborate its elements at `u8`, not the
  default 8-byte stride. As a rule this is just `(W-checking)`:
  ```
    Γ ⊢ [e₁..eₙ] ⇐ vector<Integer(s)>   ⟸   ∀i. Γ ⊢ eᵢ ⇐ Integer(s)
  ```
  The fix threaded `vector_hint` to do exactly this — i.e. it hand-implemented one
  case of the checking judgment `⇐` that §1 says should be primitive.

- **#433-residual — a multiply-assigned local has the JOIN of its assignments.**
  ```
    arg = 0;            -- Integer(0,0)        narrow
    arg = bytes[i];     -- Integer(0,255)      u8
    arg = arg*256 + …;  -- Integer(0,65535)    wider
    … use arg as integer
  ```
  Native infers `arg : u8` from the first/narrowest assignment and never widens, so
  it overflows and E0308s against the `i64` use. The missing rule:
  ```
    (T-Join)   Γ ⊢ (x := e₁; … ; x := eₙ) ⇒  x : ⨆ᵢ τᵢ     where Γ ⊢ eᵢ ⇒ τᵢ
  ```
  The shipped #433 fix (`block_needs_i64_widen`: widen a narrow value-block at a
  return/assign **seam**) is the *codegen* patch for the case where the seam is
  visible; it does NOT compute `(T-Join)` for a variable whose declared/inferred
  type is narrow across branches. That join is the real fix and is **not yet a
  rule** — it is the front-end counterpart left open after the native seam patch.

> **Rough spot R3.** Width lives in three places that must agree by hand:
> `is_equal` (ignores it), `convert`/`is_narrowing_int` (errors on it),
> `narrow_int_cast` in codegen (emits the `as`). #433 and its residual are both
> "these three disagreed." `(W-*)` + `(T-Join)` as the single source would remove the
> disagreement surface.

## 4. The rough spots, collected

| id | rough spot | the rule that would close it |
|----|------------|------------------------------|
| R1 | four `*_hint` side-channels for one idea | the checking judgment `Γ ⊢ e ⇐ τ` (§1) |
| R2 | `is_equal` collapses integer width; `⤳` then lies about it | fold width into `⤳` (§2) |
| R3 | width re-derived in is_equal / convert / codegen | `(W-Widen)`/`(W-Narrow)` + `(T-Join)` (§3) |

None of these is a runtime/memory red flag — they are **front-end** rough spots, which
is why [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) structurally misses them
(it is scoped to runtime/memory/codegen). This doc is their home.

## 5. What to do with this

This is a **lens**, not a migration plan. Concretely, the cheap wins it points at:

1. **Add `(T-Join)`** — infer a multiply-assigned local as the join of its assigned
   integer specs in the front end. Closes the #433-residual at the type, so neither
   backend needs a seam patch. Smallest, highest-confidence next step.
2. **Collapse the four `*_hint` fields into one checking-mode parameter** threaded
   through the expression parser. Mechanical; removes R1 and makes the next literal
   position correct by construction.
3. **Fold integer width into `is_equal`/`⤳`** so width has one authority. Larger;
   sequence after `(T-Join)` proves the join rule out.

Defer anything ownership/`deps`-shaped until @PLN85/@PLN87 close — per
[FORMALIZATION.md](FORMALIZATION.md) § Recommendation, the type's own contents are
still moving, and a typing relation over a moving type is premature.
