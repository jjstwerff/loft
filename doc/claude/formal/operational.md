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
> (overflow / divide-by-zero — under C80, these yield **null and continue**; `??` is the
> null-fallback). Scope note: this covers the scalar core (values, arithmetic, the
> uncomputable→null discipline, evaluation order, assignment, `if`, sequencing). Heap/store
> steps, iterators, and coroutines are **not yet** written — the interpreter remains their
> spec (D-op-1).

## Notation

- `σ` — the **store/environment** (variable ⟼ value, plus the heap).
- `⟨e, σ⟩` — a **configuration**: expression `e` to evaluate in store `σ`.
- `⟨e, σ⟩ → ⟨e', σ'⟩` — one **small step**.
- `v` — a **value**: an `integer` (64-bit), `float`, `boolean`, `character`, `text`, a
  heap reference, or **`null`**.
  (There is no trap/halt step in the core: an uncomputable result is the value `null`, not a
  halt — see `E-Uncomp`. The only runtime halts are the *explicit* `panic`/`assert` in
  dev/test, which are statements outside this scalar core.)

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

### Arithmetic — uncomputable yields null (the spreadsheet model)

```
  (E-Op)        ⟨v₁ op v₂, σ⟩ → ⟨v, σ⟩          where v = v₁ op v₂ is representable
  (E-Uncomp)    ⟨v₁ op v₂, σ⟩ → ⟨null, σ⟩       where the result is NOT computable — `v₁ op v₂`
                                                overflows the type, or op is `/`/`%` with
                                                v₂ = 0.  The result is **null**; evaluation
                                                CONTINUES (it never halts).
  (E-NullArg)   any op with a `null` operand produces `null` (null is contagious),
                EXCEPT comparisons, which compare against the sentinel.
```

**In words.** Arithmetic gives the obvious result when it fits. When it *can't* — overflow,
divide/modulo by zero — it yields **null** and the program **keeps running**; it does not
halt. This is the **spreadsheet model** ([DESIGN_DECISIONS.md C80](../DESIGN_DECISIONS.md)): a
cell that can't compute shows null and never stops the other cells. A fault is *local* — it
degrades one value, never the whole run. The same holds for every uncomputable step (an
out-of-bounds index, a deref of an absent value): null, continue.

### `??` — a non-null fallback (no trap mode)

```
  (E-Coalesce)   ⟨e ?? d, σ⟩ → ⟨v, σ⟩   if  e → v  with v ≠ null
                 ⟨e ?? d, σ⟩ → ⟨d, σ⟩   if  e → null
```

**In words.** `??` supplies a fallback for a null: `(a * b) ?? 0` is "a*b, or 0 if it couldn't
compute." There is **no** context-dependent "trap-suppression mode" any more — an op yields
null whether or not it sits under `??` (C80); `??` just decides what to do with that null.
(This is what closes the old D-op-3.)

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
- **Status:** OPEN — **direction chosen (2026-06): a differential oracle.**
- **Removal:** build a **differential oracle** — run a growing program corpus on BOTH
  backends and assert they AGREE (value / null / halt / stdout / leak); these rules stay the
  written contract that GUIDES the corpus (what behaviour to cover), not a third
  implementation. A mismatch is then a divergence caught before ship, and every fixed
  divergence grows the corpus. *Chosen for now over an executable shared semantics (both
  backends conforming to one definition) — switchable to that later; these rules are reused
  either way.*  Open follow-up: a plan issue for the oracle + corpus (none yet).

### D-op-2 — interp/native divergences are test-caught, not definition-caught
- **Violates:** E-Op / E-Uncomp / the shared-contract premise
- **Where:** the two backends are kept in agreement by the suite, so a divergence ships
  until a test happens to exercise it. **#433** is the canonical case: a program the
  interpreter evaluated fine failed to *compile* natively (`E0308`), i.e. the backends
  disagreed on a program both should accept — caught by a test, not by the definition.
- **Effect:** every codegen fix this session (the bool-arg E0308, the `__native_tail_ret`
  lift) was a backend disagreeing with the interpreter; under a shared semantics each is a
  definitional error, found before shipping.
- **Status:** OPEN — downstream of D-op-1 (the differential oracle).
- **Removal:** the differential oracle (D-op-1) makes "interp and native disagree on a
  program both accept" a *caught* failure (run-both-and-compare), not a coverage lottery —
  the corpus, not luck, decides what is exercised.

### D-op-4 — the runtime traps/halts on uncomputable, instead of null-and-continue
- **Violates:** E-Uncomp (the spreadsheet model — [DESIGN_DECISIONS.md C80](../DESIGN_DECISIONS.md))
- **Where:** today an overflow / divide-by-zero / out-of-bounds index **traps**, and in
  development HALTS the run (the old C66 dev-mode behaviour); the trap-vs-null choice still
  rides on a static `??`-position flag. E-Uncomp says the result is **null** and evaluation
  CONTINUES in every mode, with no trap-suppression flag (so `??` is purely the null-fallback).
  `panic` / `assert` are **not** in scope — those are explicit developer signals that keep
  their dev/test-halt + production-log-and-continue split (C66); only the *implicit* calculation
  faults move.
- **Effect:** one bad calculation can stop a development run, where the model says it degrades
  a single value and the rest of the program still computes — a misbehaving entity never
  freezes the world.
- **Status:** OPEN — the implementation gap the C80 decision opened (the rule moved; the code
  has not). Subsumes the former D-op-3 (the trap-suppression flag disappears with the trap).
- **Removal:** every uncomputable arithmetic / index / deref yields null + continue (the
  existing production sentinel path) in ALL modes, **silently** (no per-fault log by default —
  too spammy); drop the `??` trap-suppression mode (`??` is then purely the null-fallback).
  - **Mode-independent** (C80 point 5): implicit faults behave **identically** in dev / test /
    production — there is no dev-vs-production split for them; only `panic`/`assert` keep the C66
    split. **Mechanism:** the implicit-fault sites must take the null+continue path
    *unconditionally* — i.e. NOT consult `dev_soft_halt_enabled()` (today `raise_recoverable`
    halts under dev-soft-halt, which is why OOB still stops a test run). `panic`/`assert` keep
    `raise` + the dev-soft-halt; only the implicit faults move to an always-null variant.
  - **Tested via the debug log, not a halt:** because the faults are silent, the suite enables
    the opt-in **debug log level** (normally invisible) to VALIDATE that an expected uncomputable
    fired and produced null while the rest of the program kept running. The debug-log
    infrastructure is part of this deviation's implementation — it is the observation mechanism
    that replaces the dev-halt for asserting fault behaviour in tests.

---

## Conformance

The pinned rules are checkable directly: `5 / 0` traps (`error: divide by zero`); `a + 1`
at `a = i64::MAX` traps; `(i64::MAX + 1) ?? 0` is `0` (E-Coalesce suppresses the trap);
`integer` null is `i64::MIN`. D-op-1/D-op-2's falsifier is any program where the
interpreter and `--native` disagree — e.g. #433's cbor `read_value` (interp `20`, native
E0308 pre-fix). When the rules become the shared oracle, that disagreement is the
definitional error, and this doc is the definition it fails against.
