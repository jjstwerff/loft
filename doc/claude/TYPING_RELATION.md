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

> **Rough spot R2 — width lives outside `⤳`.** `is_equal` collapses every
> `Integer(spec)` to one type, so `(C-Refl)` makes *any* integer flow to *any* other at
> the relation level; width is then re-imposed OUTSIDE the relation, as a diagnostic (§3).
> So "does `u8` convert to `i64`?" has two answers depending on which function you ask —
> the relation says yes unconditionally; `convert` says yes-but-maybe-an-error.
>
> The collapse in `is_equal` is **not** itself the bug: there is one integer type
> (`integer`), identified by its range, and `u8`/`i16`/… are notation for a narrow range
> (see [formal/types.md § the integer model](formal/types.md)). `is_equal` answering the
> width-free "is this `integer`?" is correct. The rough spot is that `⤳` for integers
> *bottoms out in that collapse* instead of being **range containment** — so width has no
> home in the relation and gets re-derived per-site (from `forced_size` in `convert`, from
> `range()` in codegen). The fix folds width INTO `⤳` as `[a,b] ⊆ [c,d]`, with
> `forced_size` a derived storage cache, not a width source.

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
  The earlier #433 fix (`block_needs_i64_widen`: widen a narrow value-block at a
  return/assign **seam**) was the *codegen* patch for the visible-seam case. The front-end
  join is now **landed**: an inferred multiply-assigned local widens to `⨆ᵢ τᵢ` (the
  `(I-Join)` rule), so the residual closes at the type, not the seam. Guarded by
  `tests/scripts/433-ijoin-multiply-assigned.loft`; an annotated `x: u8` stays constrained.

> **Rough spot R3.** Width is *derived* in two places that must agree by hand:
> `convert`/`is_narrowing_int` (from `forced_size`) and `narrow_int_cast` in codegen (from
> `range()`). `is_equal` ignoring width is **correct** — the base-type question carries no
> width — not a third authority. #433 and its residual were both "these two derivations
> disagreed." Making `⤳` range containment, with `forced_size` a derived cache, removes the
> disagreement surface.

## 4. The rough spots, collected

| id | rough spot | the rule that would close it |
|----|------------|------------------------------|
| R1 | four `*_hint` side-channels for one idea | the checking judgment `Γ ⊢ e ⇐ τ` (§1) |
| R2 | width lives outside `⤳` — re-derived per-site, not range containment | `⤳` = range containment; `forced_size` a cache (§2) |
| R3 | width derived two ways (convert `forced_size` / codegen `range()`) | `⤳` range containment; `forced_size` a cache (§3) |

None of these is a runtime/memory red flag — they are **front-end** rough spots, which
is why [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) structurally misses them
(it is scoped to runtime/memory/codegen). This doc is their home.

## 5. What to do with this

This is a **lens**, not a migration plan. Concretely, the cheap wins it points at:

1. ~~**Add `(T-Join)`**~~ **DONE** — an inferred multiply-assigned local now widens to the
   join of its writes (the `(I-Join)` rule), closing the #433-residual at the type. Guarded
   by `tests/scripts/433-ijoin-multiply-assigned.loft`; see
   [formal/types.md](formal/types.md) (was deviation D4).
2. **Collapse the four `*_hint` fields into one checking-mode parameter** threaded
   through the expression parser. Mechanical; removes R1 and makes the next literal
   position correct by construction.
3. **Make `⤳` on integers range containment** (`[a,b] ⊆ [c,d]`) so width has one home:
   `is_narrowing_int` reads it and `forced_size` becomes a derived storage cache, not a
   width source. `is_equal` keeps its (correct) width-free collapse. Larger; the now-landed
   `(I-Join)` rule proves the range-as-truth model out. This is formal/types.md D2/D3/D5.

Defer anything ownership/`deps`-shaped until @PLN85/@PLN87 close — per
[FORMALIZATION.md](FORMALIZATION.md) § Recommendation, the type's own contents are
still moving, and a typing relation over a moving type is premature.
