<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 DN1 flip — invalidation catalogue + mitigation

When the scalar/field/return default flips to **non-null** (`LOFT_PLN25` default-on;
`LOFT_PLN25_OFF` opts out during the transition), a set of programs that type-checked
under the old *nullable-by-default* model become **invalid**. This doc is the complete
catalogue of those situations and, for each, **how it is mitigated**.

The formal grounding is [`formal/types.md` § Nullability](../../formal/types.md); every
row cites the rule it enforces. The one-line model:

> A slot of type `τ` never holds `null`. `τ?` is the only nullable form. `null` enters
> only through fallible ops (which synthesise `τ?`) and leaves only through an explicit
> discharge (`?? ` / `match`). **A declared type is a commitment; an inferred one widens
> by join.**

## The three mitigation strategies

Every invalidation is mitigated by exactly one of these. Choosing the right one per
situation is the point of this doc — *not everything should be auto-fixed*, and *some
things the compiler should fix silently*.

| Strategy | What it is | When it is right |
|---|---|---|
| **D — Diagnostic** | a precise error/warning that tells the human the exact edit | when the fix needs human intent (is nullable wanted? which default?) — the message *is* the mitigation |
| **M — Mechanical migration** | a source rewrite a tool / quick-fix can apply safely | when the edit is unambiguous (append `?` to a declaration that provably holds null) |
| **S — Semantic auto-fix** | the type system handles it silently, no source change | when the rules already specify the answer (inferred join widens to `τ?`) |

The trap to avoid: reaching for **M/S** where only **D** is sound (auto-widening an
*annotated* type silently defeats the annotation's promise — see §2).

---

## The catalogue

### 1. A declaration/return/element that legitimately holds `null` → migrate to `τ?`  ·  strategy **M**

The code *means* nullable; the old model let it omit the `?`. The fix is to add `?` to
the **declaration** that holds the null. Mechanical and safe — but it **ripples**:
consumers now see a `τ?` and may need their own discharge (§3).

| Site | Before → After | Rule | Message |
|---|---|---|---|
| fn return | `fn f() -> integer { … null … }` → `-> integer?` | (N-Store) | `` `null` cannot be stored into the return value of the non-null scalar type `integer` — declare it `integer?` to allow null `` |
| struct field | `struct S { x: integer }` (`S{x:null}` / `s.x=null`) → `x: integer?` | (N-Store) | `` … the field of the non-null scalar type `integer` … `` |
| vector element | `vector<integer>` (`v[i]=null`) → `vector<integer?>` | (N-Dense) | `` … the assignment target … `` |
| narrow field | `x: u8` / `x: integer limit(0,255)` → `x: u8?` / `…?` | (N-Store) | same as struct field |

Real examples from the migration (git history):

- `lib/lexer.loft` — 6 token accessors: `identifier -> text?`, `int/long_int -> integer?`,
  `get_float -> float?`, `get_single -> single?`, `constant_text -> text?` (`d620d77a`).
- `web.loft` — `try_recv -> text?` (`c9f512c5`).
- `tests/scripts/08-functions.loft` — `maybe_inc -> integer?`, `maybe_str -> text?`.
- `tests/scripts/292/299/32` — boolean/integer struct fields → `?`.
- `tests/scripts/389/407/inline-construct` — narrow-int fields → `u8?` / `integer limit(..)?`.
- `tests/scripts/25-nullable-sequences.loft` — `vi/vf/vt: vector<integer?/float?/text?>`.

**Auto-fixable?** *The declaration edit, yes* — a migration tool can append `?` to any
return-type / field / element type whose slot is written `null` (or an un-discharged
`τ?`). **The ripple, no** — once the return/field is `τ?`, its readers must discharge;
that is §3, which is *not* mechanical. So an auto-migrator should append the `?` **and
flag every consumer** for review rather than pretend the change is local.

### 2. An *annotated* local with a `null` init/reassign → `τ?` decl (NOT auto-widened)  ·  strategy **D**

```
a: integer = null;          // ERROR — declared non-null, assigned null
a: integer = 5;  a = null;  // ERROR — same
```

This is **(N-Decl)**: a declared `x: τ` is a *commitment*. It is the exact analogue of
`a: u8 = 300` (declared narrow, out of range) — nobody auto-widens *that* to `i32`, and
we must not auto-widen `integer` to `integer?` here. **The annotation must keep its
promise**, so this stays a **diagnostic**, not an auto-fix. The human writes
`a: integer? = null` if nullable was intended.

> ⚠️ **Open message bug (fix before the flip lands).** The local path routes through
> `change_var`, whose message is:
> `Variable 'a' cannot change type from integer to null; use a new variable name or cast with 'as'`
> This is wrong twice: (a) it does **not** name the real fix (`a: integer?`), and (b) it
> **suggests `as`** — which is the laundering hole of §6. It should emit the same
> `(N-Store)` message as the field/return sites ("declare it `integer?` to allow null"),
> and must NOT suggest `as`. **This is the single highest-value diagnostic fix in the flip.**

### 3. An un-discharged `τ?` stored into a `τ` slot → discharge (`??` / `match`)  ·  strategy **D** (auto only with narrowing)

```
got: text = "";
raw = try_recv();        // raw : text?
if raw != null { got = raw; }   // ERROR (N-Store): text? into text `got`
```

**(N-Store)**: the fix is `got = raw ?? ""` (or a `match`). This is **not safely
auto-fixable** — the compiler cannot invent the discharge default (`""`? `0`? an error?);
that is a semantic choice. So it is a **diagnostic** with a precise suggestion.

The *real* mitigation that removes most of these ergonomically is **flow-narrowing**
(after `if raw == null { continue }` / inside `if raw != null { … }`, narrow `raw` to
`text`), which is the standard nullable UX. That is a **separate feature** (deferred, see
[RESUME](RESUME.md)); with it, the `got = raw` above type-checks with no edit — a
**semantic auto-fix (S)** for the common guarded case. Until then: diagnostic + `??`.

### 4. `as τ` narrowing that doesn't provably fit the range (DN4)  ·  strategy **D**

```
400 as u8        // ERROR — provably can't fit; use `as u8?`
b: integer; b as u8   // ERROR — not provably in range; use `b as u8?`
```

**(N-Cast)** range dimension — already implemented behind `LOFT_DN4`, cutover pending.
Diagnostic + the checked form `as u8?` (yields `null` on miss). Not auto-fixed (the human
decides assert-fit vs checked). See `types.md` DN4.

### 5. stdlib null-propagation (`min`/`max`/`clamp`)  ·  strategy **M** (stdlib) + **D** (callers)

`default/01_code.loft`'s `min`/`max`/`clamp` (integer/single/float) do
`if !both || !b { return null; }` — dead under DN1 (their params are non-null). Currently
protected by the **`STD_SOURCE` exemption** in `n_store_violation` (a temporary scaffold).
At the flip: **drop the dead null-propagation** blocks (the stdlib migration) and remove
the exemption. A caller that relied on `min(null, x) == null` (e.g. `tests/…17-min-max-clamp`)
must stop passing `null` — a **diagnostic** at the call.

### 6. ⚠️ The `as` laundering hole — `null as τ` / `τ? as τ` (currently ACCEPTED)  ·  strategy **D**, ENFORCEMENT GAP

```
a: integer = null as integer;              // ACCEPTED today → a holds null (UNSOUND)
x: integer? = g();  y: integer = x as integer;   // ACCEPTED → y holds null (UNSOUND)
fn f() -> integer { return null as integer; }    // ACCEPTED → returns null (UNSOUND)
```

`as` currently strips nullability without enforcing fit, so it **bypasses every §1–3
guard**. Per **(N-Cast)** — "`as τ` requires a *provable fit*; the honest maybe-miss form
is `as τ?`" — a `null` / `τ?` source into a non-null scalar target must be a **compile
error** directing to `as τ?` (checked → `null`) or `?? d` (discharge). This is the
**nullness sibling of DN4** (§4, the range sibling); they are one rule and should share
one fit-check and one gate.

**Not a big problem in practice** (it is an *explicit* opt-in — you have to write
`as integer` — not a silent default), so it is a **tightening, not a foundation crack**.
But it must be closed for the flip to be sound, and it should land *after* the flip
(don't couple), scoped precisely: reject only when target `is_non_null_scalar` and source
is `Null`/`Optional`; `null as S` (heap ref) stays legal.

### 7. ⚠️ Inferred `null`-join not widened — `a = null; a = 5` (currently REJECTED)  ·  strategy **S**, IMPLEMENTATION GAP

```
a = null;  a = 5;      // ERROR today ("cannot change type from null to integer")
```

Per **(N-Join)** this should infer `a : integer?` (the join of `null` and `integer`,
made optional). It is the *inferred* escape valve the model promises — the ergonomic
counterpart to §2's rejection: you get nullable **for free** without annotating, exactly
where no annotation committed you to non-null. It is currently **not implemented**
(`change_var` rejects the null→τ transition instead of joining). This is a **semantic
auto-fix** to *build*: it makes the whole model less punishing without weakening a single
annotation. Higher ergonomic value than closing §6.

---

## Summary — what to auto-fix vs diagnose

| # | Situation | Strategy | Auto-fix the *source*? | Auto-fix the *semantics*? |
|---|---|---|---|---|
| 1 | decl/return/element holds null | **M** | ✅ append `?` (+ flag consumers) | — |
| 2 | annotated local `= null` | **D** | ❌ (annotation is a commitment) | — |
| 3 | `τ?` into `τ` slot | **D** | ❌ (default is a choice) | ✅ *with* flow-narrowing (deferred) |
| 4 | `as τ` range miss (DN4) | **D** | ❌ | — |
| 5 | stdlib null-prop | **M**+**D** | ✅ drop dead block | — |
| 6 | `as` null-launder (GAP) | **D** | ❌ | — (close the hole first) |
| 7 | inferred null-join (GAP) | **S** | — | ✅ **implement (N-Join) for null** |

**Two things the flip does not yet do**, both "enforcement/impl incomplete" (not "design
wrong"), sequenced *after* the flip lands:

1. **Close the `as` null-launder hole** (§6) — unify with DN4's fit-check.
2. **Implement (N-Join) for null** (§7) — the inferred-nullable auto-fix.

And **one message fix to do with the flip** (§2): the `change_var` local-null message must
stop suggesting `as` and name `integer?`.
