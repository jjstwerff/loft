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

> **Rough spot R1 — RESOLVED.** Four fields = four positions someone remembered to thread.
> The four are now consolidated into one `Parser.expected` field with shape-dispatching
> reader methods (`lambda_hint()`/`enum_hint()`/`vector_hint()`/`read_target_type()`) — one
> `⇐` channel, set once and read by shape. A new position pushes the same `expected` rather
> than adding a 5th field. See formal/types.md (was D1).

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
 (C-Bare)    Sorted/Hash/Index/Spatial(..) ⤳ Reference(bare collection)
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
| R1 | ~~four `*_hint` side-channels for one idea~~ **DONE** — one `Parser.expected`, shape-dispatched | done (§1) |
| R2 | ~~width lives outside `⤳`~~ **DONE** — `⤳` is range containment; residual: storage off i32 | done (§2) |
| R3 | ~~width derived two ways~~ **DONE** — parser/codegen agree via range; residual: storage off i32 | done (§3) |

None of these is a runtime/memory red flag — they are **front-end** rough spots, which
is why [STABILITY_REDFLAGS.md](STABILITY_REDFLAGS.md) structurally misses them
(it is scoped to runtime/memory/codegen). This doc is their home.

## 5. What to do with this

This is a **lens**, not a migration plan. Concretely, the cheap wins it points at:

1. ~~**Add `(T-Join)`**~~ **DONE** — an inferred multiply-assigned local now widens to the
   join of its writes (the `(I-Join)` rule), closing the #433-residual at the type. Guarded
   by `tests/scripts/433-ijoin-multiply-assigned.loft`; see
   [formal/types.md](formal/types.md) (was deviation D4).
2. ~~**Collapse the four `*_hint` fields**~~ **DONE** — consolidated into one
   `Parser.expected` field with shape-dispatching reader methods; the four set-sites push the
   one field, the readers filter by shape (lambda → `Type::Function`, enum → enum-context,
   vector → narrow-element vector, read-target → any). A new position pushes `expected`, not a
   5th field. (was deviation D1.)
3. ~~**Make `⤳` on integers range containment**~~ **DONE** — `is_narrowing_int` now decides
   by range containment (`[a,b] ⊆ [c,d]`), in agreement with codegen's `narrow_int_cast`, so
   signedness is visible and the parser/codegen split is closed (was D3/D5). `is_equal` keeps
   its (correct) width-free collapse. **Residual (D2):** the full integer is still flagged by
   `forced_size = None`, because `IntegerSpec` carries i32/u32 bounds, not i64. i64 bounds are
   the trigger, but the real work is migrating the narrow-storage layer (`Parts`, `Value::Int`,
   the storage ops, `usable_min`/`usable_max`) off i32 — ~36 sites, mapped in formal/types.md
   § D2 Removal (two carry a silent-truncation hazard; do it as one focused pass).

The ownership/`deps` deferral has since lifted: @PLN85/@PLN87 closed, so `deps` is now a typed,
total fact and the ownership rules are written ([formal/ownership.md](formal/ownership.md), 0 open).
The typing relation is no longer written over a moving type — this doc's R1–R3 are DONE, with only
the i64-storage migration (D2 → @PLN88) outstanding.
