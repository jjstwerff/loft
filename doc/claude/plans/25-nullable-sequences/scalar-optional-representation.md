<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 — the `τ?` representation for scalars (DECIDED)

The Phase-2 scalars blocker (recorded in [implementation-steps.md](implementation-steps.md)):
only `Type::Integer` carries a null-flag (`not_null`); `Boolean`/`Float`/`Single`/
`Character`/`Text`/`Reference`/`Enum` have **no type-level optional marker**. When the
default flips to non-null, `text?`/`S?` need a way to *say* "nullable" that the unmarked
type no longer means. This doc decides how.

## Decision: `Type::Optional(Box<Type>)` — one optional former, compile-time only

A single new `Type` variant wraps any base type. `integer?` is `Optional(Integer{…})`,
`text?` is `Optional(Text(..))`, `S?` is `Optional(Reference(S,..))`, and so on.

- **(N-Opt)** holds for any `τ` by construction (`Optional(τ)` is always a type).
- **(N-Idem)** is enforced in the constructor: `Optional(Optional(τ)) → Optional(τ)` —
  no double-null.
- The constructor also normalises `Optional(Never)`/`Optional(Null)` to avoid junk.

## Why this option (the decisive criterion: loud omission)

The competing option — a per-variant `nullable: bool` flag — **fails silently**: a
`match tp { … }` that forgets to read the flag still compiles and is wrong at runtime.
That is exactly the brittleness we remove, not add (hidden invariant, silent violation).

A **new Type variant makes every exhaustive match a COMPILE ERROR until it handles
`Optional`** — so a site that forgets nullability fails *loudly at build time*. The
representation enforces its own coverage. This is the same principle DN4/`(N-Cast)` apply
to casts ("no claim without enforcement"), applied to the type representation itself.

## Why it costs nothing at runtime (aligns with "runtime never errors")

`Optional` is a **compile-time distinction only**. Storage stays **sentinel-based** —
`i64::MIN` for integer, null `DbRef` for references, char 0, etc. (already in `fill.rs`).
`Optional(τ)` and `τ` have the **same runtime layout**; there is no wrapper allocation and
no `__nullable<S>` enum synth for scalars. (That synth stays **vector-element-only**, where
dense packing genuinely cannot use a sentinel for an arbitrary struct.) So `Optional` adds
**zero runtime cost and zero runtime errors** — the compile side gets tidy, the runtime
keeps degrading null and continuing.

The DN4 measurement corroborates the sentinel model: stdlib **0** and crawler **0**
not-provably-fit narrowing casts — production code already lives within ranges, so the
type-level nullability is pure compile-time bookkeeping over a representation that already
exists.

## Reconciling `IntegerSpec.not_null`

`not_null` today does double duty — **nullability** AND **bounds-validity** (a non-null
`u8` must report `not_null:true` so `255` is accepted). Untangle them:

- **Bounds always apply.** An `IntegerSpec` *is* its range; `u8` = `Integer{0..255}`.
- **Nullability moves to the wrapper.** `u8?` = `Optional(Integer{0..255})`.
- **Drop `not_null` from `IntegerSpec`** once `Optional` lands — a follow-up *inside*
  DN1/DN3, not DN4. Until then DN4 may keep emitting the existing `not_null:false` for
  `as τ?` results; that result type migrates to `Optional(Integer)` when `Optional` lands.

## Rejected alternatives

- **(a) per-variant `nullable: bool`** — silent-omission brittleness (above) + polarity
  split (`Integer.not_null` vs others' `nullable`). Rejected.
- **(c) reuse `__nullable<S>` synth for scalars** — an enum wrapper where a sentinel
  already exists: heavyweight and the wrong tool. The synth exists for dense vector
  elements that *can't* sentinel an arbitrary struct; scalars don't have that problem.
  Rejected.

## Landing approach (when DN1/DN3 build it — not DN4)

1. Add `Type::Optional(Box<Type>)` with the idempotent constructor.
2. The compiler flags every exhaustive `match` — route the nullability-**agnostic**
   majority through a normaliser (`tp.peel_optional() -> (&Type, bool)` / `tp.base()`),
   so only the discharge / store / cast checks read the optional bit. The compile errors
   are the worklist; none can be silently skipped.
3. `(N-Store)`/`(N-Decl)`/`(N-Coal)`/`(N-Match)` read the optional bit; everything else
   peels it.

DN4 needs **none of this** — it is integer-only (`IntegerSpec` already carries range +
`not_null`), so it ships first and independently; see
[copy-elision-design.md] siblings and `formal/types.md` § DN4.
