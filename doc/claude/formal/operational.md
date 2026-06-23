<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/operational.md — small-step semantics for the stable core (strict)

> **Rules then deviations** (see [README](README.md)). This is a small-step evaluation
> relation for loft's **stable scalar core**, written for one purpose: to be the **shared
> contract both backends must satisfy**. Today the interpreter (`src/state/`) *is* the
> de-facto spec and the native generator (`src/generation/`) is a separate implementation
> kept in agreement only by tests — so a disagreement is a test gap, not a definitional
> error. These rules turn that around: a program where the two backends step differently
> is **by definition** a bug in whichever one disobeys.
>
> Rough spot #3 from [FORMALIZATION.md](../FORMALIZATION.md). Pinned-behaviour sources:
> [LOFT.md § null](../LOFT.md) (in-band sentinels) and [LOFT.md § Arithmetic safety](../LOFT.md)
> (overflow / divide-by-zero trap; the `??` suppression). Scope note: this covers the
> scalar core (values, arithmetic, the null/trap discipline, evaluation order, assignment,
> `if`, sequencing). Heap/store steps, iterators, and coroutines are **not yet** written —
> the interpreter remains their spec (D-op-1).

## Notation

- `σ` — the **store/environment** (variable ⟼ value, plus the heap).
- `⟨e, σ⟩` — a **configuration**: expression `e` to evaluate in store `σ`.
- `⟨e, σ⟩ → ⟨e', σ'⟩` — one **small step**.
- `v` — a **value**: an `integer` (64-bit), `float`, `boolean`, `character`, `text`, a
  heap reference, or **`null`**.
- `⟨e, σ⟩ ↯ r` — a **trap**: evaluation halts with reason `r` (a runtime error). A trap is
  terminal; nothing steps out of it.

---

## Rules

### Values and null

```
  (E-Val)    a value v does not step (it is a normal form).
  (E-Null)   `null` is a value, represented in-band by a per-type SENTINEL — e.g.
             `integer`'s null is `i64::MIN`.  Two configs that agree on the abstract
             value (incl. null) MUST agree, regardless of how a backend stores the sentinel.
```

**In words.** A value is "done" — it doesn't evaluate further. `null` is a real value, not
a separate state; each type reserves one bit pattern for it (an `integer` null is the
smallest `i64`). The semantics talk about the *abstract* value; how a backend encodes the
sentinel is its business, but the value it computes must match.

### Evaluation order — left to right

```
  (E-Left)   in a binary form `e₁ op e₂`, reduce e₁ to a value first, then e₂:
                 ⟨e₁, σ⟩ → ⟨e₁', σ'⟩   ⟹   ⟨e₁ op e₂, σ⟩ → ⟨e₁' op e₂, σ'⟩
                 ⟨v₁ op e₂, σ⟩ → ⟨v₁ op e₂', σ'⟩   when   ⟨e₂, σ⟩ → ⟨e₂', σ'⟩
```

**In words.** Operands evaluate left first, then right — so any side effects (a call that
mutates the store) happen in source order. Both backends must use this order.

### Arithmetic, and the trap discipline

```
  (E-Op)        ⟨v₁ op v₂, σ⟩ → ⟨v, σ⟩          where v = v₁ op v₂ is representable
  (E-Trap)      ⟨v₁ op v₂, σ⟩ ↯ r               where v₁ op v₂ overflows, or op is `/`
                                                or `%` with v₂ = 0.  r names the operator
                                                and the operands ("integer overflow:
                                                2147483647 + 1", "integer division by zero").
  (E-NullArg)   any op with a `null` operand produces `null` (null is contagious),
                EXCEPT comparisons, which compare against the sentinel.
```

**In words.** Arithmetic gives the obvious result *when it fits*. When it would overflow
the type, or divide/modulo by zero, it does **not** quietly return a wrong number or a
null — it **traps**: the program halts with a message naming the operator and operands.
This is the safety guarantee, and both backends must trap on exactly the same inputs.

### The `??` trap-suppression mode

```
  (E-Coalesce)   ⟨e ?? d, σ⟩ : evaluate e in TRAP-SUPPRESSING mode —
                   – if e → v (v ≠ null):           e ?? d  →  v
                   – if e → null, OR e would trap:  e ?? d  →  d
                 The mode is STATIC: an arithmetic op is trap-suppressing IFF it is the
                 direct operand of `??`.  Elsewhere (E-Trap) applies.
```

**In words.** `??` is the escape hatch. An arithmetic operation written *directly* under
`??` does not trap on overflow or divide-by-zero — instead it yields `null`, which `??`
then replaces with the fallback. So `(a * b) ?? 0` is "a*b, or 0 if it overflows." The
suppression is decided by syntax (is this op the direct operand of `??`?), so the *same*
`a * b` traps in one place and falls through in another — a context-dependent evaluation
mode both backends must reproduce identically.

### State steps

```
  (E-Var)    ⟨x, σ⟩ → ⟨σ(x), σ⟩
  (E-Asgn)   ⟨x = v, σ⟩ → ⟨v, σ[x ↦ v]⟩                 (the RHS reduces first, by E-Left)
  (E-Seq)    ⟨v ; s, σ⟩ → ⟨s, σ⟩
  (E-IfT)    ⟨if true { s } else { t }, σ⟩ → ⟨s, σ⟩      (and E-IfF for false)
```

**In words.** A variable steps to its stored value; an assignment reduces its right side
then updates the store; a sequence drops a finished statement; an `if` picks the branch
its (already-evaluated) condition selected. Standard — pinned here only so both backends
share them.

---

## Deviations

OPEN: **3**

### D-op-1 — there is no shared operational semantics; the interpreter is the spec
- **Violates:** the premise of this doc (a single evaluation relation both backends obey)
- **Where:** `src/state/` (the interpreter) is the de-facto definition; `src/generation/`
  (native) is a *separate* generator. No rules above are mechanically checked against
  either — they are a written contract the code is *supposed* to meet.
- **Effect:** correctness for native means "matches the interpreter on the tests we ran",
  not "obeys the semantics". The unwritten parts (heap/store steps, iterators, coroutines)
  have no spec but the interpreter's code.
- **Status:** OPEN — the structural rough spot #3.
- **Removal:** grow these rules to cover the core, and treat them as the oracle both
  backends are tested against (a differential harness keyed to the rules, not to each
  other).

### D-op-2 — interp/native divergences are test-caught, not definition-caught
- **Violates:** E-Op / E-Trap / the shared-contract premise
- **Where:** the two backends are kept in agreement by the suite, so a divergence ships
  until a test happens to exercise it. **#433** is the canonical case: a program the
  interpreter evaluated fine failed to *compile* natively (`E0308`), i.e. the backends
  disagreed on a program both should accept — caught by a test, not by the definition.
- **Effect:** every codegen fix this session (the bool-arg E0308, the `__native_tail_ret`
  lift) was a backend disagreeing with the interpreter; under a shared semantics each is a
  definitional error, found before shipping.
- **Status:** OPEN — downstream of D-op-1.
- **Removal:** the differential oracle of D-op-1 makes "interp and native step differently"
  a definitional failure, not a missing test.

### D-op-3 — trap-vs-null is context-dependent and lives only in code
- **Violates:** E-Coalesce being a clean, written rule
- **Where:** whether `a op b` traps or yields null depends on whether it is the direct
  operand of `??` — a static parser fact (the trap-suppression flag), implemented per-site
  rather than derived from one rule. Adjacent to the integer-width re-derivation in
  [types.md D5](types.md).
- **Effect:** a new syntactic position that *should* suppress (or must not) is decided by
  whoever wires it — the same shape as the four `*_hint` channels (types.md D1).
- **Status:** OPEN.
- **Removal:** carry "trap-suppressing context" as one fact threaded by E-Coalesce, not a
  per-op flag.

---

## Conformance

The pinned rules are checkable directly: `5 / 0` traps (`error: divide by zero`); `a + 1`
at `a = i64::MAX` traps; `(i64::MAX + 1) ?? 0` is `0` (E-Coalesce suppresses the trap);
`integer` null is `i64::MIN`. D-op-1/D-op-2's falsifier is any program where the
interpreter and `--native` disagree — e.g. #433's cbor `read_value` (interp `20`, native
E0308 pre-fix). When the rules become the shared oracle, that disagreement is the
definitional error, and this doc is the definition it fails against.
