<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/types.md — type system & conversion relation (strict)

**Catalogue:** @F3 (scalar types), @F4 (width integers), @F5 (type conversions), @F1 (null / Optional). Roadmap: @PLN88, @PLN25.

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

See **§ Nullability** below for `τ?` (the optional former) and the rules that introduce /
eliminate it; indexing (`(N-Index)`) is one of the fallible operations that synthesise it.

### Conversion `τ ⤳ σ` — width folded in

```
  (C-Refl)    τ ⤳ τ
  (C-Never)   Never ⤳ τ
  (C-Tuple)   (σ₁…σₙ) ⤳ (τ₁…τₙ)        ⟸   ∀i. σᵢ ⤳ τᵢ
  (C-Var)     Reference(S) ⤳ Enum(E)   ⟸   S ∈ variants(E)            (and plain
              Enum ⤳ Integer tag).  NB: the null INTRO `S ⤳ S?` is `(N-Intro)`; there is
              NO implicit `S? ⤳ S` unwrap (that is `(N-Store)`-illegal — discharge via `??`)
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

### Nullability — the optional former `τ?`

> **One former, representation derived — the integer model applied to null.** `τ?` is the
> *optional* type ("a `τ`, or `null`"). **Storage is non-null by default**: a binding,
> field, or `vector` element of type `τ` never holds `null` — `τ?` is the *only* way a slot
> admits it. `not null` is accepted but a **no-op** (non-null is already the default; kept
> for source back-compat). Null enters values only through the **fallible operations**
> below, which synthesise `τ?`; it leaves only through an explicit **discharge** (`??` /
> `match`). There is no implicit unwrap.

```
  formation
  (N-Opt)      τ wf  ⟹  τ? wf          τ? is a type for any τ
  (N-Idem)     τ?? ≡ τ?                 optional is idempotent — no double-null
  (N-Dense)    vector<τ> stores τ       elements are non-null unless written vector<τ?>

  introduction (a non-null value flows into an optional slot)
  (N-Intro)    τ ⤳ τ?                   the ONLY null-direction conversion in ⤳

  "no representable result" → τ?.  null is the UNIVERSAL doesn't-fit / undefined value —
  NEVER wrap, saturate, or an out-of-range value (that would be UB).  Nullability is
  RANGE-DRIVEN: an op is non-null when its result provably fits, τ? when it could miss.
  (N-Index)    Γ ⊢ v ⇒ vector<τ>, Γ ⊢ i ⇒ Integer   ⟹   Γ ⊢ v[i]  ⇒ τ?       (OOB — no elem)
  (N-Div)      Γ ⊢ a,b ⇒ Integer                     ⟹   Γ ⊢ a/b, a%b ⇒ Integer?  (÷0 undefined)
  (N-Parse)    Γ ⊢ parse_τ(s)                          ⟹   Γ ⊢ …     ⇒ τ?           (invalid)
  (N-Arith)    Γ ⊢ a,b ⇒ Integer,  op ∈ {+,-,*}      ⟹   Γ ⊢ a op b ⇒ Integer[r]
               where r = the range-arithmetic of the operands.  NON-null if r ⊆ i64 (the
               common bounded case — no `??`); else Integer? (overflow → null).
  (N-Cast)     Γ ⊢ e ⇒ Integer[s]   ⟹   Γ ⊢ (e as τ) ⇒ τ   REQUIRES s ⊆ range(τ).  If the
               fit is NOT provable it is a COMPILE ERROR — `as τ` is an assertion of fit; the
               honest form for a maybe-miss is `as τ?`.  (`b: integer; b as u8` errors; `400
               as u8` errors — provably can't fit.)
  (N-Cast?)    Γ ⊢ (e as τ?) ⇒ τ?    the CHECKED cast — value if it fits, else null.  Always
               legal; NEVER yields an out-of-range value.  (`b as u8? ⇒ u8?`.)

  elimination (discharge — REQUIRED; there is NO  τ? ⤳ τ)
  (N-Coal)     Γ ⊢ e ⇒ τ?,  Γ ⊢ d ⇐ τ                ⟹   Γ ⊢ (e ?? d) ⇒ τ
  (N-Match)    match e { null ⇒ …,  x ⇒ …(x:τ)… }      eliminates τ?, binds the τ arm
  (N-Store)    storing  e:τ?  into a  τ  slot is ILL-TYPED — discharge first

  inference — declared vs inferred storage (the "by definition vs by use" split)
  (N-Decl)     a DECLARED slot `x: τ` is a COMMITMENT: `x = e` checks `e ⇐ τ`. If e:τ? it is
               `(N-Store)`-illegal — declaring non-null FORBIDS a later nullable write.
  (N-Join)     an INFERRED `a = e₁ … a = eₙ` (no annotation) has type `⨆ᵢ τᵢ`, made OPTIONAL
               iff some `τᵢ` is optional.  `?` rides the SAME join as integer width `(I-Join)`
               — `integer ⊔ integer? = integer?`.
```

> **The case that proves the declaration means something** (`do we allow a:integer=2; a=v[i]`?):
>
> | code | result | why |
> |---|---|---|
> | `a: integer = 2;  a = v[i]` | **type error** | `a` declared non-null `(N-Decl)`; `v[i]:integer?` can't store `(N-Store)` — write `a = v[i] ?? 0` or declare `a: integer?` |
> | `a = 2;  a = v[i]`          | `a : integer?` | inferred → `(N-Join)` widens to optional |
>
> Exactly parallel to declared integer width (`a: u8 = 2; a = big` also errors — a declared
> type is a commitment; an inferred one widens by join).

**In words.** Nullability is a property of **operations, not storage**. `null` is the
**one universal value for "no representable result"** — division by zero, out-of-bounds
index, failed parse, integer overflow, and a cast that doesn't fit **all yield `null`**,
never a wrapped / saturated / out-of-range value. That single choice is what **roots out
the UB class**: a slot of type `τ` never holds a non-`τ` value — it either fits (and the
op is non-null) or it's `null` (and `(N-Store)` forces you to discharge it). We do **not**
fake non-null on an op that can miss.

Nullability is **range-driven**, so the discharge burden is proportional to *real* risk,
not theoretical: `a op b` and `e as τ` are **non-null when the result provably fits** the
target range — `b=4; b*100 ⇒ Integer[400]` fits i64, so it's non-null, **no `??`** — and
`τ?` only when the range could miss (a narrowing `as`, a declared-narrow slot, a genuinely
i64-overflowing product). So there is **no `??` "after every `a*x`"** — only where the op
can actually fail to produce an in-range value. `(N-Intro)` is the one implicit
null-direction step; the reverse is never implicit, so a null can't be lost by accident.

This unifies nullability with the **integer range model into one system**: a value's range
decides its storage width *and* whether a fit-failure yields `null`. Overflow-to-null is
therefore the *correct* runtime behavior (loft already does it for `a*b`); the work is to
**type** it (`Integer?` when the range exceeds i64) and require discharge — and to fix
`as` so `400 as u8` yields `null`, not the current `400`.

**Representation is derived, exactly like integer width.** `τ?` has one identity
(optional-of-τ); how the `null` is *stored* follows from the base type and is not part of
the type:

| base `τ` | `τ?` null representation |
|---|---|
| `Integer` / `Bool` / `Char` / `Float` | the value-slot **null sentinel** (`i32::MIN`, `255`, …) |
| a struct `S` as a `vector` element | the tagged **`__nullable<S>`** enum (discriminant + payload) |

So `bytes_for(τ?)` and the sentinel-vs-tag choice are *consequences* of `τ`, never declared
— the same discipline as `bytes_for_range` on integers. **Parametricity holds**: `τ?` and
`vector<τ>` formation commute with substitution (`⟦N?⟧[N:=S]=⟦S?⟧`,
`⟦vector<N>⟧[N:=S]=⟦vector<S>⟧`), because nothing is rewritten on the element's *shape*.

(Design + the evidence that the old implicit default-nullable rewrite broke parametricity
and forced materialisation: [../plans/25-nullable-sequences/storage-vs-access-nullability.md](../plans/25-nullable-sequences/storage-vs-access-nullability.md).)

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

OPEN: **6** (the nullability flip + UB-rooting — §25 in progress). Per-situation
mitigation catalogue: [../plans/25-nullable-sequences/DN1-MITIGATION.md](../plans/25-nullable-sequences/DN1-MITIGATION.md).

### DN1 — scalar / field storage is still nullable-by-default
The `(N-Dense)` / non-null-default rule holds for `vector` elements (flipped: dense default,
`vector<τ?>` opt-in) but **not yet for scalars and struct fields**: a plain `integer` field
still admits `null` (verified: `a.x = null` on `x: integer` succeeds), and `not null` is the
opt-out rather than a no-op. Falsifier: `struct A { x: integer }  …  a.x = null` type-checks
today; under the rule it must require `x: integer?`. Closes when the scalar default flips +
`not null` becomes the no-op.

**Status (2026-07-01): the flip is the active @PLN25 step-f, prototyped + blast-radius-measured.**
The `keys.rs` default-on flip (`LOFT_PLN25_OFF` opts out) + the `n_store_violation` enforcement of
`(N-Store)`/`(N-Decl)` at return / field / typed-store / vector-index sites are validated; the
scalar-vector-element + `character?` + narrow-field-in-vector native gaps are closed. Landing is
gated on: (a) resolving the remaining red tests (measured, small); (b) fixing the `change_var`
local-null message (it wrongly suggests `as`; must name `integer?` — see the mitigation doc §2);
(c) migrating the stdlib `min`/`max`/`clamp` dead null-prop + removing the `STD_SOURCE` exemption;
(d) closing DN5 + DN6 below. Full sequence: [DN1-MITIGATION.md](../plans/25-nullable-sequences/DN1-MITIGATION.md).

### DN2 — implicit `S? ⤳ S` unwrap still exists
Code still performs the implicit `Enum(__nullable<S>) ⤳ Reference(S)` unwrap (the old
`(C-Var)` dual), violating `(N-Store)` / the no-implicit-elim rule — a `τ?` can reach a `τ`
slot without a `??`/`match`. Falsifier: an `S?` value assigned to an `S` binding with no
discharge. Closes when the unwrap is removed and `(N-Coal)`/`(N-Match)` are the only elims.

### DN3 — fit-failing ops warn + propagate instead of yielding `τ?`
`a / b`, `a % b`, `parse_*`, and overflowing `a*b` already produce **null at runtime**
(correct per the model — null is the doesn't-fit value, `E-Uncomp`/C80), but the **type**
doesn't carry it: they're typed non-null and only *warn* ("division may produce null"),
so an un-discharged null flows into non-null storage. Falsifier: `b = a / x` with no `??`
type-checks (warns) today; under `(N-Div)`/`(N-Arith)`+`(N-Store)` it must be `… ?? 0` or
`b: integer?`. The runtime is right; the **typing + discharge** is the gap. Its blast
radius (fit-failing results stored into non-null without `??`) is the gating measurement.

### DN4 — `as` to a narrower type doesn't enforce the range (UB)  ·  IMPLEMENTED behind `LOFT_DN4` (default off); cutover pending
`400 as u8` yields **400** by default — the cast asserts the *type* but leaves an
out-of-range value in a `u8` slot (UB: a `u8` holding 400). Per `(N-Cast)`/`(N-Cast?)` the
two honest forms are: **`as u8` requires a PROVABLE fit** (so `400 as u8` and `b: integer;
b as u8` are **compile errors** — "use `as u8?`"), and **`as u8?` is the CHECKED cast**
(value or `null`, never out-of-range). Falsifier: `(400 as u8) == 400` is true by default;
with `LOFT_DN4` set, `400 as u8` does not compile and `400 as u8?` is `null`.

**Status (2026-06-28):** the rule is implemented behind the `LOFT_DN4` flag and validated
on both backends (`tests/dn4_cast.rs`; the value matrix, the compile-error cases, and the
flag-off-is-inert guard). `as τ?` is a pure parse-time range-guard desugar (`OpLeInt` + `if`
+ `OpConvIntFromNull`) — **no new runtime op and no runtime error**; the result types as a
full nullable integer (the null sentinel `i64::MIN` needs full width — typing the guard as
the narrow `τ` made native `i64::MIN as u8 == 0` and lost the null). One item now gates the
**default-on cutover**: **migrate the remaining in-tree sites** to `as τ?` (measured at
stdlib 0, crawler 0, tests/scripts 30/9 *before* range-tracking — now fewer, since masked
casts are exempt). Gate (1) — **range-tracking** so a masked value is provably-fit — is
**DONE for `&`/`%`** (always-on; `x & 255` types `integer(0,255)`, `(non-neg) % c` types
`[0,|c|-1]`, both sound + suite-green): `(x & 255) as u8` no longer needs `as u8?`. The
`(N-Arith)` range arithmetic for `+`/`-`/`*` (overflow → `Integer?`) remains, and lands
with DN3. Deviation **shrinks but stays open** until cutover. The integer-range sibling of DN1–DN3's
null work — same "no claim without enforcement."

### DN5 — `as τ` launders `null` / `τ?` into a non-null scalar (the nullness sibling of DN4)
`as` currently strips nullability without enforcing fit, so it **bypasses `(N-Store)`**: `null as
integer`, `x:integer? as integer`, and `return null as integer` all type-check and store `null`
into a non-null `integer` (verified: `a: integer = null as integer` yields `a == null`). Per
`(N-Cast)` — "`as τ` requires a PROVABLE fit; the honest maybe-miss form is `as τ?`" — a `Null`/
`Optional` source into a non-null scalar target must be a **compile error** directing to `as τ?`
(checked → `null`) or `?? d`. Falsifier: `(null as integer) == null` is `true` today; under
`(N-Cast)` it does not compile. This is the **nullness dimension of DN4** (which is the range
dimension) — one rule, one fit-check, one gate. *Explicit* opt-in (you must write `as integer`),
so a tightening not a foundation crack; close it AFTER the scalar flip (DN1) lands, scoped to
`target is_non_null_scalar ∧ source ∈ {Null, Optional}` (a `null as S` heap ref stays legal).

### DN6 — inferred `null`-join is rejected instead of widening to `τ?`
Per `(N-Join)` an inferred `a = null; a = 5` (no annotation) must infer `a : integer?` — the join
of `null` and `integer`, made optional. The implementation instead **rejects** it (`change_var`:
"cannot change type from null to integer") rather than joining. Falsifier: `a = null; a = 5; a ?? 0`
is `(N-Join)`-legal (`a : integer?`) but errors today. This is the *inferred* escape valve the
model promises — nullable **for free** where no annotation committed you to non-null, the
ergonomic counterpart to `(N-Decl)`'s rejection of annotated `a: integer = null`. Closes when
`change_var` joins `Null ⊔ τ = τ?` for an unannotated local.

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
