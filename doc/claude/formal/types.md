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
  min/max). `integer` = `Integer[i64::MIN+1, i64::MAX]` (symmetric — the reserved null
  sentinel `i64::MIN` is EXCLUDED from the value range; see
  [operational.md `(E-Null)`](operational.md)); `u8` = `Integer[0, 255]` (a *non-null* `u8`;
  a nullable `u8?` reserves `255` for null, so its non-null values are `[0, 254]`); etc.
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
> admits it. `not null` still parses (kept for source back-compat) but is **deprecated**: it
> is a semantic no-op (non-null is already the default) and, since #546, WARNS ("`not null` is
> deprecated and has no effect… delete `not null`") rather than staying silent. Null enters
> values only through the **fallible operations**
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
  (N-Default)  Γ ⊢ e ⇒ τ?,  has_default(τ)           ⟹   Γ ⊢ e? ⇒ τ            (@PLN116)
               the TYPE's default discharges:  e?  ≡  e ?? construct_default(τ).
               `has_default(τ)` is a STATIC side-condition — where it fails, `e?` is a
               COMPILE error, never a runtime one (§ Defaults below).  The pairing is the
               mnemonic: `??` = the default YOU give, `?` = the default the TYPE gives.
  (N-Match)    match e { null ⇒ …,  x ⇒ …(x:τ)… }      eliminates τ?, binds the τ arm
  (N-Store)    storing  e:τ?  into a  τ  slot without discharge is REJECTED — a WARNING for
               most τ (the null is representable-and-distinct in τ's non-null form), a hard
               ERROR only for narrow widths (u8…u32, § Null-flow below, where the null would
               collide with a real value); discharge first (`?? d` / `match`) either way

  inference — declared vs inferred storage (the "by definition vs by use" split)
  (N-Decl)     a DECLARED slot `x: τ` is a COMMITMENT: `x = e` checks `e ⇐ τ`. If e:τ? it is
               `(N-Store)`-illegal — declaring non-null FORBIDS a later nullable write.
  (N-Join)     an INFERRED `a = e₁ … a = eₙ` (no annotation) has type `⨆ᵢ τᵢ`, made OPTIONAL
               iff some `τᵢ` is optional.  `?` rides the SAME join as integer width `(I-Join)`
               — `integer ⊔ integer? = integer?`.

  @PLN102 (SHIPPED, default-on since 2026-07-11 — see § Null-flow, the general laws below):
   · (N-Prop)   null PROPAGATES through arithmetic (a:τ? op b ⇒ τ?), across every type.
   · (N-Parse)  FOLDS INTO (N-Cast): a parse is a cast — `s as τ` asserts (non-null), `s as
                τ?` checks.  The auto-`τ?` reading above is superseded.
   · (N-Store)  becomes a WARNING (not ill-typed) EXCEPT narrow widths (u8…u32), where the
                null collides with a real value and it stays a hard error.
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

> **The one decided exception — overflow arithmetic ([C85](../DESIGN_DECISIONS.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--)).**
> `a+b` / `a*b` / `a-b` stay typed **non-null `integer`** (forcing `integer?` on every
> arithmetic op would poison the common path to guard a fault that essentially never fires),
> yet on overflow they write the reserved `i64::MIN` sentinel into that non-null slot — which
> then reads as `null`. So a non-null `integer` slot *can* observably hold `null` after an
> overflow: `(N-Store)` never fires because the op is typed non-null. This is a **deliberate,
> bounded soundness edge** (parallel to a non-null `float` holding a `NaN`), not a deviation to
> close — the reachable-fault ops (`/`, `%`, `v[i]`, parse) stay `τ?` as above.

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

| base `τ` | `τ?` null representation (the reserved value, per width) |
|---|---|
| `Integer` (8-byte) | in-band sentinel `i64::MIN` |
| a narrow `Integer` (`u8`/`i8`/`u16`/`i16`/`i32`) | in-band sentinel = its top stored value (`u8` → `255`); excluded from `τ?`'s non-null range |
| `Bool` | in-band sentinel `255` |
| `Char` | in-band sentinel codepoint `0` (collides with a literal `'\0'`) |

> **The sentinel is the type's, not each reader's — measured 2026-08-22.** loft#1014 made
> every site that WRITES a character agree on codepoint 0; five sites that READ one, or put
> a character on the wire, still spelled it their own way, and `Stores::is_null`'s character
> arm was unreachable as well as wrong. An absent character therefore serialised as a SPACE,
> and `to_json()` emitted the loft literal `'q'` — not JSON — so a struct with a character
> field could not round-trip through loft's own parser. On the wire a character is now a
> one-character JSON **string**; a number is still read as its codepoint. Guarded by
> `tests/scripts/character-across-the-json-surface.loft` on both backends, and the reason
> the interpolation channel still differs (concatenation skips a bare `'\0'`) is the
> collision this row names. QUALITY.md § `character` on the JSON surface.
| `Float` / `Single` | in-band sentinel = a reserved `NaN` |
| a reference | out-of-band `nullref` (a reserved `DbRef`; no collision) |
| a struct `S` as a `vector` element | the tagged **`__nullable<S>`** enum (discriminant + payload; no collision) |

> The in-band scalar sentinels are **observable, reserved values** — the base type's null is
> one specific bit-pattern, excluded from the non-null range (`(E-Null)`). This is the
> deliberate zero-overhead choice ([the null-model keystone](../plans/102-stability-contract/keystone-null-model.md),
> option B); its cost — a nullable narrow type cannot store its one reserved value — is a
> documented limitation, and the collision sites that could silently *produce* a sentinel are
> guarded (D-op-null-2). References and struct-in-vector already avoid the cost with an
> out-of-band tag.

So `bytes_for(τ?)` and the sentinel-vs-tag choice are *consequences* of `τ`, never declared
— the same discipline as `bytes_for_range` on integers. **Parametricity holds**: `τ?` and
`vector<τ>` formation commute with substitution (`⟦N?⟧[N:=S]=⟦S?⟧`,
`⟦vector<N>⟧[N:=S]=⟦vector<S>⟧`), because nothing is rewritten on the element's *shape*.

(Design + the evidence that the old implicit default-nullable rewrite broke parametricity
and forced materialisation: [../plans/25-nullable-sequences/storage-vs-access-nullability.md](../plans/25-nullable-sequences/storage-vs-access-nullability.md).)

### Defaults — `construct_default` and the `x?` discharge (@PLN116)

`(N-Default)` discharges a `τ?` with **the type's own** default. That default is ONE partial
function on types, shared with the `S{}` zero value (one home per fact,
[Goal E](../GOALS.md)) — so `x?` and `S{}` can never disagree about what a `τ` defaults to.

```
  construct_default : τ ⇀ value        PARTIAL — has_default(τ) is exactly its domain

  (D-Scalar)   construct_default(Integer[r]) = 0            (every width)
               construct_default(Float)   = 0.0
               construct_default(Single)  = 0.0f
               construct_default(Boolean) = false
               construct_default(Character) = '\0'
  (D-Text)     construct_default(text)      = ""
  (D-Coll)     construct_default(vector<τ>) = []            (likewise every keyed collection)
  (D-Opt)      construct_default(τ?)        = null          an optional's default IS null
  (D-Enum)     construct_default(Enum E)    = the FIRST-DECLARED variant of E
  (D-Rec)      construct_default(S)         = S{f₁ = d₁ … fₙ = dₙ}
                 where dᵢ = the field's `= expr` when given, else construct_default(τᵢ)
                 REQUIRES  ∀i. has_default_field(fᵢ)

  the two places the domain STOPS — has_default = FALSE, so `x?` (and `S{}`) is a COMPILE error
  (D-NoRef)    has_default(&τ) = FALSE
                 a bare reference / non-null DbRef has no zero: there is no "the null pointer"
                 in a language whose storage is non-null by default.
  (D-NoEnumF)  has_default_field(f : E) = FALSE  when E is a BARE (non-optional) enum and f
               carries no `= expr`.
                 An enum's 0 IS its null (variants are 1-based), so a non-null enum field may
                 not silently zero-fill to it; and choosing a variant as a record's default is
                 a real decision the author must make.  Fix by supplying the field, giving it
                 `= <variant>`, or typing it `E?` — which then defaults to `null` by (D-Opt).
```

**In words.** Every type either has one obvious zero or it has none, and `x?` is exactly
"discharge with that zero". Scalars go to `0`/`false`/`'\0'`, `text` to `""`, collections to
empty, a record to itself with every field defaulted. Two things genuinely have no zero, and
for those `x?` does not compile — you must say what you mean with `??` or `match`.

**The partiality is a TYPE rule, not a runtime one.** This is what keeps `x?` consistent with
"no *runtime* errors, ever" ([C80](../DESIGN_DECISIONS.md)): `x?` never fails at run time
because the cases where no default exists are rejected at *compile* time. `has_default` is a
static well-definedness condition on the operator, in the same family as `(N-Cast)`'s
provable-fit requirement — not a check that can fire on a value.

**A bare enum discharges positionally — say it out loud.** `(D-Enum)` makes `x?` on an enum
mean *the first-declared variant*, so **reordering the variants silently changes what `x?`
does**, while `x ?? Colour.Red` is order-independent. That asymmetry is deliberate (a bare
enum has to default to *something*, and first-declared is the only choice that needs no extra
declaration), but it means `?` on an enum trades an explicit choice for a positional one.
Prefer `??` where the variant matters. Note the contrast with `(D-NoEnumF)`: a *bare* enum
discharges to its first variant, yet an enum *field inside a record* refuses to — because
defaulting a whole record silently is a much bigger claim than defaulting one expression.

**At a text parse, `?` composes with the CHECKED cast, not the asserting one.** A bare
`s as integer` on text is ill-typed under `(N-Cast)` — a parse cannot be *asserted* — so
`(s as integer)?` is rejected by the inner cast, before `(N-Default)` is ever reached; its
premise `Γ ⊢ e ⇒ τ?` simply does not hold. The composable form is the checked cast first:

```loft
(s as integer?)?          // integer   — checked cast ⇒ integer?, then `?` ⇒ 0 on a bad parse
s as integer ?? 0         // integer   — the assert-or-default form `(N-Cast)` licenses
s as integer              // COMPILE ERROR — an assertion a parse can't discharge
```

This is not a gap in `(N-Default)`: `?` discharges the parse result exactly like any other
`τ?`. Only the *asserting* spelling is refused, and it is refused for reasons that have
nothing to do with `?`.

**Falsifying programs.**

```loft
// (D-Enum) — obeying the rule and reordering the enum disagree
enum Colour { Red, Green, Blue }
c: Colour? = null;
c?                        // Red.  Swap Red/Green in the declaration and this becomes Green.

// (D-NoEnumF) — a record whose enum field has no default cannot itself default
struct Pixel { tint: Colour, x: integer }
p: Pixel? = null;
p?                        // COMPILE ERROR: `tint` is a bare enum with no `= expr`
                          // fixes: `tint: Colour = Colour.Red`, or `tint: Colour?`

// (D-Opt) — an optional defaults to null, so `?` on it is a no-op, not an unwrap
o: integer? = null;
o?                        // 0        (τ = integer here — `?` discharges the outer optional)

// (D-Rec) — the default is structural, not a shared instance (see operational.md E-Default)
struct P { x: integer, y: integer }
q: P? = null;
q?                        // P{x:0, y:0}
```

### Null-flow — the general laws, across EVERY type (@PLN102, 2026-07-11)

The `(N-*)` rules are stated on `τ` and hold for **every** type, not just `integer`. The null
model is ONE model: each type reserves exactly one null ([C90](../DESIGN_DECISIONS.md); table
below), and four general laws govern how a null is *produced*, *propagated*, *asserted away*,
and *stored*. They are checked **throughout the stack** by a cross-type conformance matrix
(each type × each law, both backends), so a gap in any one type is a caught deviation, not a
silent hole.

**Shipped (2026-07-11, default-on — @PLN102 #559; `nullflow_enabled()` in `src/keys.rs`, opt-out
`LOFT_NO_NULLFLOW`):** (N-Prop), the (N-Store) warn/error split, the (N-Domain) generalisation,
and folding (N-Parse) into (N-Cast) are live across every type, verified both backends. Float
`/`/`%` and the domain-partial functions (`sqrt`, `ln`, `log`, `asin`, `acos`, `pow`) now type
`τ?` exactly like integer `/`/`%`: a variable-divisor `1.0 / b` stored into a non-null `float`
WARNS (the (N-Store) split above, not a uniform hard error); and a text parse `s as float` is
now a hard COMPILE ERROR ("a text parse `as float` may fail — use `float?`"), superseding the
old auto-`τ?` reading. Design record:
[../plans/102-stability-contract/float-null-domain-typing.md](../plans/102-stability-contract/float-null-domain-typing.md).

```
(N-Domain)  a PARTIAL OPERATION that can yield the reserved null from a REACHABLE input types
            its result τ?  —  ÷0 / %0 (Integer / Float / Single); sqrt(<0), ln·log(≤0),
            asin·acos(∉[-1,1]), pow(neg ^ frac) (Float / Single); v[i] / s[i] OOB (ANY element
            τ).  Non-null when the input is PROVABLY in-domain (constant / range / guard) — the
            same "provably-fits" elision as (N-Arith) / (N-Cast).  Generalises (N-Div) /
            (N-Index) to every type + partial op.  Runtime = null + continue (C80); the reserved
            null is the VALUE, NEVER a runtime error.

(N-Prop)    an operation with a NULLABLE operand whose runtime carries the null through types
            its result nullable:  a:τ? op b  ⟹  τ?  (either operand).  Arithmetic on
            Integer / Float / Single (the sentinel / NaN propagates), text `?`-concat, etc.
            The type tracks the propagation the runtime ALREADY performs (verified: `n+5`,
            `5-n`, `abs(n)` on a null n stay null).  C85 is the COMPLEMENT — non-null operands
            stay non-null; a sentinel PRODUCED by overflow is a result, not a propagated INPUT.

(N-Cast)    an explicit cast `as τ` is an ASSERTION → non-null τ (compile error if the fit is
            not provable — use `as τ?` / `?? d`).  A text→numeric PARSE is a cast, so it obeys
            (N-Cast) / (N-Cast?): `s as float` asserts (non-null), `s as float?` checks (→
            float?), `s as float ?? d` is assert-or-default.  This SUPERSEDES the old auto-`τ?`
            reading of (N-Parse): the `?` on a cast is the programmer's explicit choice, never
            inferred, for EVERY τ.  (An OPERATION's `?` is inherent (N-Domain); a CAST's is not.)

(N-Store)   storing e:τ? into a non-null τ slot is —
            · a WARNING (nudge, compiles + runs, the slot holds null) when the null is
              REPRESENTABLE-AND-DISTINCT in τ's non-null form: a stored null reads back as null,
              spreadsheet model intact;
            · a hard ERROR when the null sentinel is a VALID non-null value of τ (a collision),
              so the null cannot be stored faithfully — discharge (`?? d`) or widen the slot to
              `τ?`.
            Representability is a property of τ (table).  REFINES the old uniform-error
            (N-Store): every type warns EXCEPT the narrow widths, which error.
```

**Per-type null + store verdict** — the verdict follows the *representability* test, not
per-type taste ([C90](../DESIGN_DECISIONS.md) fixes the reserved value):

| τ | reserved null | distinct in the NON-null form? | `τ?` → `τ` store |
|---|---|---|---|
| `integer` (i64) | `i64::MIN` — reserved even non-null (`[MIN+1, MAX]`) | yes | **warn** |
| `float` / `single` | `NaN` | yes (`NaN` ≠ any real) | **warn** |
| `boolean` | `255` (three-state, C73) | yes (non-null = 0 / 1) | **warn** |
| `character` | codepoint `0` / NUL — reserved even non-null (`0 as character` reads null) | yes | **warn** |
| `text` | out-of-band (heap) | yes | **warn** |
| reference | out-of-band `nullref` | yes | **warn** |
| struct in `vector` | tagged `__nullable<S>` | yes | **warn** |
| narrow `u8`/`i8`/`u16`/`i16`/`i32`/`u32` | top width value — reserved ONLY in the `τ?` form | **no** — non-null uses the full width (`255` is a real `u8`) | **error** |

The narrow widths are the **sole error case**: they are the only types whose non-null form
spends the whole width on real values (C90 gives them a sentinel only in `τ?`, to keep the
range full), so a null cannot sit in a non-null narrow slot. Every other type reserves its
null distinctly even non-null — the C85 in-band-sentinel property — so a stored null is
observable and a warning suffices. (Composite `τ?` — tuples, multi-field aggregates — inherits
its elements' out-of-band tags and warns; per-element vs whole-tuple nullability is an open
gap, see [../plans/102-stability-contract/formal-audit.md](../plans/102-stability-contract/formal-audit.md).)

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

### Pattern captures (@PLN35, SHIPPED)

> **@PLN35 · SHIPPED.** These rules were written spec-first, ahead of the code; the code
> landed with phases 1–7 + PC1–PC5 ([matching.md § Rules — PEG patterns](matching.md)) and now
> obeys them — verified both backends: an alternation binding a name in only SOME branches
> reads `null` in the branch that does not bind it (`P-Alt-Diff`), and a capture inside `(a)?`
> reads `null` when the optional is absent (`P-Opt-Ty`). Pinned per-phase by the @PLN89 oracle;
> design: [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

A pattern capture binds a name to a matched sub-result. **The headline is that this introduces NO
new type former** — every capture lands on `τ`, `τ?`, or `vector<τ>`, unified by the SAME join `⊔`
and nullable-join `(N-Join)` that already run the integer / nullability model above.

```
  (P-Cap-Ty)     Γ ⊢ (name:p)  binds  name : τ         where Γ ⊢ p's result ⇒ τ
  (P-Alt-Same)   (a | b) both bind name : τ_a, τ_b     ⟹   name : τ_a ⊔ τ_b   (the join; if τ_a ⊔ τ_b
                 is undefined ⟹ STATIC ERROR "alternatives bind name at incompatible types")
  (P-Alt-Diff)   name bound in only SOME alternatives  ⟹   name : τ?           (via (N-Join))
  (P-Opt-Ty)     a capture inside (a)?                 ⟹   promoted to τ?       (via (N-Opt))
  (P-Rep-Ty)     the capture inside (a)* / (a)+        ⟹   vector<τ>            (via (N-Dense))
  (P-Rest-Ty)    ..name over a vector<τ> subject      ⟹   name : vector<τ>
```

**In words.** A named capture takes the type of whatever its sub-pattern produced. When two
alternatives bind the same name, the two types are joined (`integer` and `u8` → `integer`; no join
⟹ a compile error, exactly like an incompatible integer join). A name only *some* branches bind, or
a capture inside an optional, becomes nullable (`τ?`) — the same optional-join `(N-Join)` an
inferred `a = null; a = 5` uses. A repetition or `..rest` collects into a `vector<τ>`. So PEG
capture typing is a new *source* of the types loft already has; `match` also stays a `τ?` eliminator
(`(N-Match)` is unchanged — a `null` / `x` arm still discharges an optional).

---

## Deviations

OPEN: **0** — `D-Opt-Zero` is CLOSED (2026-08-24, below); the @PLN25 nullability flip (DN1–DN6) is CLOSED (2026-07-02); D1/D2/D4 closed by
fix/reconciliation.  The **@PLN102 DN3-Float extension** (below) is also CLOSED — SHIPPED
default-on 2026-07-11 (#559): float `/`/`%` and the domain-partial float functions type `τ?`
exactly like integer `/`/`%`.  Every DN1–DN6 + DN3-Float entry is CLOSED, retained as the
record.  Per-situation mitigation catalogue:
[../plans/25-nullable-sequences/DN1-MITIGATION.md](../plans/25-nullable-sequences/DN1-MITIGATION.md).

### D-Opt-Zero — CLOSED (2026-08-24): a nullable field defaults to its BASE ZERO, not null

`(D-Opt)` says `construct_default(τ?) = null` — *"an optional's default IS null"* — and
`(D-Rec)` composes it: a field with no `= expr` takes `construct_default` of its own type.
The code does not do that for a scalar or text base. Measured on **both** backends, so this
is a rule/code divergence and not a backend split:

```loft
struct S { a: integer?, b: text?, c: boolean?, d: Colour? }
s = S {};                       // a=0   b=''   c=false   d=null
```

Only the enum base answers `null`. `data::to_default` handles it as
`Type::Optional(inner) => to_default(inner, data)` and states the choice outright: base-zero
is *"the settled design call"* (@PLN25), because a bare `null` would fall to the `_` arm and
render as native unit into a scalar slot (E0308) — and the field is still writable to null
through an explicit `= null`.

**So the disagreement is real and deliberate, which is the awkward part**: a decision was
taken in code and `(D-Opt)` was never updated to match, exactly as
[`formal/layout.md`'s `L-Tuple`] was left naming a function a rename had removed. The
doctrine says the CODE changes to match the rules, so this stays OPEN rather than being
written away — but the decision behind it has a stated reason and a plan number, and the
resolution is a design call the owner should make, not a silent edit in either direction:

- amend `(D-Opt)` to state base-zero for a scalar/text base and `null` for the rest, or
- change `to_default` and give the E0308 problem a different answer.

**Resolved by the owner (2026-08-24): the rule stands — a nullable value in a field is null
at the start.** `to_default`'s `Optional` arm now builds the base type's null SENTINEL
through the new `data::to_null`, which is the `OpConv…FromNull` op for that base. Those ops
already existed and already produced the same sentinels as the runtime's
`Stores::set_default_value_nullable` (`i64::MIN`, `255` for the tri-state boolean,
`char::from(0)`), so the two paths now agree instead of contradicting each other.

The E0308 objection was real but misdirected: a bare `Value::Null` does render as native
unit, and the answer is the TYPED null op rather than the base's zero. Nothing needed
inventing.

**What the old behaviour rested on, checked:** the code cited *"an omitted field gets the
zero value for its type (LOFT.md § constructors; 06-structs.loft locks it)"*. `LOFT.md` has
no such section and does not contain that sentence, and `06-structs.loft` declares no
nullable field at all. The only thing actually locking it was one half of
`issues::issue_332_nullable_narrow_field_null_roundtrip`, written on the strength of those
two citations; it now asserts the rule instead, across omitted / assigned / re-nulled for
`i16?`, `i32?` and `integer?`.

**Blast radius: one test in 4 434.** Verified on both backends for `i8? u8? i16? u16? i32?
integer? float? single? boolean? text? character?` and an enum, a reference and a vector —
and the non-nullable defaults (`0`, `false`, `""`) are unchanged.

⚠ `character?` is the one base whose null is indistinguishable from its zero: both are
`char::from(0)`, which is also what the runtime writes (its content-type-6 arm ignores
`nullable`). Consistent between the two paths, so not a divergence — but it means
`character?` cannot represent absence distinctly. Separate question, not opened here.

Found by citing: the `D-*` family has one home (`data::to_default` + `Data::has_default`),
and writing *"enforces `@FR-D-Opt`"* on it is what forced the comparison.

### DN-SE-inline — CLOSED (2026-08-22): a nullable struct-enum in INLINE storage
The representation rule above derives `τ?`'s null from `τ`'s storage, and gives a reference the
out-of-band `nullref`. A struct-enum is carried as a `DbRef`, so `Shape?` takes the reference
sentinel — which loft#1065 measured it did NOT: several sites answered "what is this type's
null" without telling a struct-enum from a value enum, and it took the value enum's `255`
BYTE into a handle slot. `--interpret` then read the slot back as a live ref (its own
store-lifetime guard fired) and `--native` refused to compile (`non-primitive cast: u8 as
DbRef`). **Closed for a LOCAL, a parameter and a return** by discriminating `Enum(_, true, _)`
from `Enum(_, false, _)` at each site, plus `base()` where a shape was read without peeling
`Optional` (six sites; guard `tests/scripts/1065-nullable-struct-enum.loft`).

**INLINE storage closed too** (loft#1071) — a struct FIELD and a `vector<Shape?>` element are
a four-byte record pointer inside the holder's record, which cannot hold the twelve-byte
sentinel at all. The rule says the representation follows the base type's STORAGE, and it does
here: absence is pointer `0`, which is what the field prime already writes. Three sites had to
agree on that one word — the construction (`Box { s: null }`), the assignment (`b.s = null`,
which had been silently a no-op), and the test, which must read the stored WORD because
`OpGetField` answers a sub-reference whose own record is the HOLDER's and so is never null. No
`__nullable<…>` tag was needed: a record pointer already has an in-band absent value, exactly
as a narrow scalar does, so the element rides the `Optional` marker like a scalar element.

Iteration closed with it: `for e in v { e == null }` binds a sub-reference to the element
SLOT, so it reads that slot's word. Which of the two a `Var` is turns on what it VIEWS,
followed through the DEP CHAIN — a loop variable's own dep names itself, and only its
declaration's dep names the vector, so reading the first link answers no.

This entry is also the answer to the "OPEN: 0" line above having been too strong: the rule was
written, and the code disagreed with it for a whole type former, in both directions at once.

### DN1 — CLOSED (2026-07-02): scalar / field storage is non-null by default
`(N-Dense)` now holds for scalars + struct fields, not just vector elements: a plain
`integer`/`text`/`bool`/`float`/`char` field/local/return is NON-null by default (nullability
rides `?`/`Optional`), and `not null` is redundant — stripped from in-tree source; the parser
still ACCEPTS it for backward compat pending the registry republish (task #4), and (since #546)
WARNS "deprecated and has no effect" rather than staying silent. `a.x = null`
on `x: integer` is now a compile error ("declare it `integer?`"). Landed: the DN1 default flip
(PR #471, default-on; `LOFT_PLN25_OFF` opts the whole model out) + the F2 field-attribute flip for
ALL scalars + the `not null` source strip + parser no-op. Enforced by `(N-Store)`/`(N-Decl)` at
return / field / typed-store / index / call-argument sites — the last (a nullable `τ?` passed
into a non-null PARAMETER) closed 2026-07-16 (`callarg_nstore_enabled`, `src/keys.rs`; the
identical warn/error split at the `process_call_args` chokepoint, opt-out
`LOFT_NO_CALLARG_NSTORE`). Both backends.

### DN2 — CLOSED (2026-07-02): no implicit `S? ⤳ S` unwrap
The implicit `Enum(__nullable<S>) ⤳ Reference(S)` (and scalar `τ? ⤳ τ`) unwrap is gone — a `τ?`
cannot reach a `τ` slot without a discharge. Verified: `x: S? = …; y: S = x` (no `?`) is a compile
error ("cannot change type from S to S?"); `?? default` / `match` are the only elims
(`(N-Coal)`/`(N-Match)`). Closed by the `(N-Store)` / `change_var` teeth (DN1) + the DN5 `as`-cast
closure. Both backends.

### DN3 — CLOSED (2026-07-02): fit-failing ops type `τ?`  ·  overflow-arith = decided edge
A fit-failing op now TYPES its result `τ?` (the runtime already nulled per `E-Uncomp`/C80), so an
un-discharged result stored into non-null storage is a compile error — the developer must guard,
`?? d`, or declare the target `τ?`. **DONE:** integer `/` and `%` (the division root-cause fix — the
`handle_operator` arithmetic-branch wrap); `v[i]` / `s[i]` indexing (the index flip, default-on,
with const / iter-var / `if i < len` guard fit-proofs); and **text→numeric parse** (`(N-Parse)`:
`s as integer` / `as float` / `as single` now type `integer?` / `float?` / `single?` — a bad parse
is a reachable fault, exactly like `÷0` and OOB). The `as` handler in `parser/operators.rs` wraps a
`Text`-source numeric cast in `Optional`; discharge with `?? d`, `as τ?`, or `match`. In-tree
consumers migrated (lib/lexer number/escape accessors already return `τ?`; the test scripts +
audience-demo servers `?? 0`). The runtime "may produce null" warnings are RETIRED (the type +
`(N-Store)` is the enforcement). Regressions: `25-division-nullable.loft`, `25-index-nullable.loft`,
`inc17_text_to_integer_requires_as` + `102` reject twins; both backends. **DECIDED EDGE (not a
deviation) — overflow arithmetic** `a*b`/`a+b`/`a-b` stays NON-null: overflow → the null sentinel +
continue (C80, no trap), NOT `τ?`. The fault is extraordinary (operands ~3×10⁹) while the op is
ubiquitous, so forcing discharge on all arithmetic is disproportionate + (given no traps) would
block a game over a fault its player never hits —
[DESIGN_DECISIONS C85](../DESIGN_DECISIONS.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--). Range-tracking keeps provably-fit multiplies exact.

### DN4 — CLOSED (2026-07-02, F5 cutover): `as` to a narrower type enforces the range
`400 as u8` was UB (the cast asserted the *type* but left an out-of-range value in a `u8`
slot). Now per `(N-Cast)`/`(N-Cast?)`: **`as u8` requires a PROVABLE fit** (`400 as u8`, and
`b: integer; b as u8`, are **compile errors** — "use `as u8?`"), and **`as u8?` is the CHECKED
cast** (value or `null`, never out-of-range) — a pure parse-time range-guard desugar (`OpLeInt`
+ `if` + `OpConvIntFromNull`, no new runtime op; the guard types as a full nullable integer so
the `i64::MIN` sentinel keeps full width). Range-tracking makes masked values provably-fit, so
`(x & 255) as u8` / `(non-neg) % c as u8` need no `?`. **Enforcement is UNCONDITIONAL** — the
interim `LOFT_NO_DN4` opt-out (which reverted to the silent width-tag) is RETIRED, so DN4 is
consistent with its nullness sibling DN5. Validated both backends (`tests/dn4_cast.rs`: the
value matrix, the compile-error cases, and the opt-out-retired guard).

### DN5 — CLOSED (2026-07-02, F3): `as τ` no longer launders `null` / `τ?` into a non-null scalar
`null as integer`, `x:integer? as integer`, `(a/b) as integer`, `v[i] as integer` used to
type-check and store `null` into a non-null slot (bypassing `(N-Store)`). Now a `Null`/`Optional`
source cast to a non-null scalar (no `?`) is a **compile error** directing to `as τ?` (checked →
`null`) or `?? d`; `as τ?` types `Optional<τ>` so the laundering stays closed downstream
(`z: τ = (e as τ?)` still requires a discharge). This is the **nullness dimension of DN4** — the
two are ONE domain-containment fit-check at the `as` chokepoint (`operators.rs`): a scalar
target's domain is its value RANGE × nullness, and `null` is the reserved out-of-domain element
(so `is_narrowing_int` peels `Optional` — a nullable source no longer slips past the range
check). A `null as S` heap ref stays legal (`is_non_null_scalar` is scalar-only). Regression:
`tests/scripts/102-expected-errors.loft` twins + the `25-*-nullable.loft` accept paths.

### Refinement (2026-07-10): implicit checked narrowing into a NULLABLE narrow target
DN4's `as`-required rule is for a **non-null** narrow target. Coercing an integer / `integer?`
into a **nullable** narrow target (`Optional<narrow>`, e.g. `u8?`) needs **no explicit `as`**:
`convert` routes it through the same `dn4_checked_cast` range-guard (in-range → the value,
out-of-range → `null`). This is sound without ceremony because the target is nullable — an
out-of-range value becomes a VISIBLE `null`, never the silent truncation that `as u8` into a
non-null slot would be. Only nullable narrow targets are affected; a non-null `u8` still
requires an explicit `as`. Guard: `tests/scripts/25-nullable-narrow-implicit-checked.loft`
(both backends).

### DN6 — CLOSED (2026-07-02, F4): the inferred `null`-join widens to `τ?` instead of rejecting
Per `(N-Join)` an INFERRED `a = null; a = 5` (no annotation) now infers `a : integer?` — the join
of `null` and the scalar — via `change_var_type` (`variables/mod.rs`). `var_tp == null` is
inherently the inferred case (a variable cannot be annotated `null`), so the widen never overrides
an explicit non-null contract: annotated `a: integer = null` still rejects, as does the reverse
`a = 5; a = null`; a widened `τ?` into a non-null slot still requires a discharge. **Scoped to
INLINE scalars** (Integer/Boolean/Float/Single/Character): the retroactive widen reuses the slot
the first `= null` allocated, sound only when Null and `τ?` share it (an in-slot sentinel).
`Text` — the one heap-backed scalar — is EXCLUDED (its Null slot is not a text?-heap slot;
widening it underflowed `fn_return`'s discard / native E0308), so a text null-start must annotate
`s: text? = null`. Regression: `tests/scripts/25-null-join.loft` + reject twins in
`102-expected-errors.loft`.

### DN3-Float — CLOSED (2026-07-11, shipped default-on, @PLN102 #559): float/single `/`, `%`, and domain-partial functions type `τ?`

> This is the **float instance** of the general **§ Null-flow laws** (N-Domain / N-Prop /
> N-Cast / N-Store) above — those laws hold for every type; this entry records the float
> classification (which ops) and the conversion set.

DN3 typed integer `/`/`%`, indexing, and text-parse `τ?`, but its division wrap is **gated
to `Type::Integer`** (`src/parser/operators.rs:2300`), so float/single `/`/`%` — and the
domain-partial float functions — keep a **non-null** `float`/`single` return while
producing null (a reserved NaN) at runtime. The type therefore **lies** about a null the
integer side already surfaces: `f.g = 1.0 / b` and `f.g = ln(-1.0)` store null straight
into a non-null `float` field with no diagnostic, where the integer `s.f = 10 / y`
equivalent is a compile error. Return types freeze at contract 1, so the honest signature
is chosen now (pre-freeze-only).

**Rule.** A float/single op types its result `τ?` **iff it can yield the reserved NaN-null
from an input a normal program reaches** — the DN3 boundary read across to floats, with
[C85](../DESIGN_DECISIONS.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--)
(overflow → non-null) as its complement:

- **`τ?`:** `/` and `%` (÷0 — mirror of integer `/`; the existing `divisor_provably_nonzero`
  proof keeps `x / 2.0` and a guarded `if b != 0.0` non-null); `sqrt` (arg `< 0`);
  `ln`/`log`/`log2`/`log10` (arg `≤ 0`); `asin`/`acos` (arg outside `[-1, 1]`); `pow` (base
  `< 0` ∧ fractional exp — resolved 2026-07-11: a genuine domain error, folded in). A
  **provably-in-domain argument blocks the `τ?`** — not just a literal constant (`sqrt(4.0)`,
  `sqrt(PI)`, `ln(2.0)`) but any expression the sign/interval lattice proves in-domain
  (`sqrt(dx*dx + dy*dy)`, `sqrt(max(x, 0.01))`, `asin(clamp(e, -1, 1))` all stay non-null,
  @PLN102 #581, 2026-07-16 — [design](../plans/102-stability-contract/soften-nullflow-discharge.md));
  an argument that is **not** provably in-domain is `τ?` (bare `sqrt(x)` on an unknown-sign `x`
  stays nullable). A constant *out-of-domain* argument (`sqrt(-1.0)`) may warn
  *"always null"*, the parallel of the existing constant-`/0` warning.
- **Non-null (C85-style decided edge):** `sin`/`cos`/`tan`/`atan`/`atan2`/`exp`/`abs`/
  `ceil`/`floor`/`round` — a *finite* argument is always finite; NaN arises only from an
  `±inf` argument, itself reachable only through a C85 overflow, so forcing `?` on the
  ubiquitous op is disproportionate.

**Runtime is UNCHANGED** — null + continue (C80); **no runtime error is added** (owner:
*"a null is fine, errors never"*). Two type-level rules do the work, refining DN3 **across
the shipped integer model** (@PLN25) too — the runtime *already* propagates the null
sentinel through arithmetic (`n+5` / `n*5` / `5-n` / `abs(n)` on a null `n` all stay null,
both backends), so the type only has to stop lying:

- **(N-Prop) — nullability propagates through arithmetic.** An arithmetic op with any
  nullable operand yields a nullable result: `integer? + integer → integer?`,
  `float - float? → float?` (either operand position). Already true for `text? + text`; now
  uniform over `integer` / `float` / `single`. **C85 is untouched** — non-null × non-null
  stays non-null (overflow → sentinel silently, no `?` forced); propagation fires only when
  an operand is *already* nullable, so the two compose (non-null arithmetic stays non-null;
  a null, once present, stays visible).
- **(N-Warn) — a nullable into a non-null slot is a WARNING, not an error — EXCEPT narrow
  width types.** The relaxation applies where the target's null pattern is available in its
  *non-null* form: `integer` (reserves `i64::MIN` even non-null — a stored null reads back as
  null), `float`/`single` (NaN), `text` (out-of-band). There the program still compiles + runs
  (the slot holds null) and the warning nudges toward `?? d` / `match` — a warning because a
  hard error would break every existing non-null store of a `sqrt` / float-`/` result
  (compatibility) and Goal F reserves warnings as the programmer-billing channel; this RELAXES
  DN3's current integer *hard error* to a warning. **Narrow width integers**
  (`u8`/`i8`/`u16`/`i16`/`i32`/`u32`) spend their whole width on real values (a non-null `u8`
  holds `255`), so they have no spare bit-pattern for null — a null there is unrepresentable
  and would silently corrupt to a real value. They **keep the hard error** (`?? d`, or widen
  the target to `u8?`). Split principle: *warn iff the null is representable-and-observable in
  the non-null slot* — the in-band-sentinel property C85 already relies on. Narrow stores
  already error (DN1/DN4/DN5), so keeping them costs zero compatibility.

Together, a `float?` rides through inference, arithmetic, comparison, interpolation, and
calls, and is nudged only at a non-null STORAGE site (field / return / explicitly-typed
local / call-argument). Mechanism (shipped): the
`div_nullable` gate was extended to `Float`/`Single` (the nullable runtime peers
`OpDiv{Float,Single}Nullable` + the `??`-swap, phase 4f.5); the domain-partial function return
types in `default/01_code.loft` are `float?`/`single?`. **Shipped, default-on 2026-07-11
(@PLN102 #559), verified both backends; the pow / domain-proving forks and the conversion set
are recorded in
[../plans/102-stability-contract/float-null-domain-typing.md](../plans/102-stability-contract/float-null-domain-typing.md).**

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
