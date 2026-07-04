<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/operational.md — small-step semantics for the stable core (strict)

**Catalogue:** @F38 (arithmetic safety), @F1 (null model), @F3 (scalar core) — the Goal-D backend contract. Roadmap: @PLN28, @PLN89.

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
> null-fallback). Scope note: this file covers the scalar core (values, arithmetic, the
> uncomputable→null discipline, evaluation order, assignment, `if`, sequencing). The rest of
> the operational semantics is split into sibling files (all 2026-07-04): [heap.md](heap.md)
> (store alloc/read/write/copy/free), [iteration.md](iteration.md) (`for` + combinators),
> [coroutines.md](coroutines.md) (generators), [concurrency.md](concurrency.md) (`par`),
> [calls.md](calls.md) (function call/return + parameter binding), [matching.md](matching.md)
> (`match` + exhaustiveness), [tuples.md](tuples.md), [closures.md](closures.md) (lambdas /
> closures / fn-refs — written WITH 2 open deviations, the only operational file that has them).
> Still NOT written — the interpreter remains their spec (tracked under D-op-1): **text
> formatting** (`"{x}"` interpolation) and **generics/interfaces** (a static/typing concern →
> types.md).

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

### Observability — report a fault only where it is UNGUARDED

```
  (E-Report)   an UNGUARDED uncomputable divide/modulo-by-zero ALSO emits a Warn-level
               log (`divide_by_zero`) — the "no guard" signal — before yielding null.
               A GUARDED site (the operand of `??` / a following null-check) emits the
               silent `*Nullable` op and reports NOTHING (the guard owns the null).
               Integer OVERFLOW is silent at every site (the null IS the signal — also
               the rustc-release default); the value is null, never a wrapped wrong answer.
```

**In words.** The fault stays a *value*, never a halt (E-Uncomp), but loft is not blind to it:
an uncomputable you did **not** defend — a bare `a / 0` — also writes one Warn log so it is not
invisible, while a site you *explicitly* defended (`a / b ?? 0`) is silent because you already
said how to handle it. Overflow is silent everywhere — common enough that a per-site log would
be spam, and the null result already shows it. The Warn is **silent on a default CLI run** (no
logger attached) and surfaces when a logger is — which is how a test *validates* the fault fired
(see `runtime_logging.rs::prod_divide_by_zero_logs_and_continues`). The opt-in `--dev-soft-halt`
debug flag still surfaces these recoverable faults (uniformly: div0, overflow, OOB) for one-shot
breakage triage — it is an explicit debugging tool, NOT a dev/test/prod mode, so it does not break
E-Uncomp's mode-independence.

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

OPEN: **2**

### D-op-1 — there is no shared operational semantics; the interpreter is the spec
- **Violates:** the premise of this doc (a single evaluation relation both backends obey)
- **Where:** `src/state/` (the interpreter) is the de-facto definition; `src/generation/`
  (native) is a *separate* generator. No rules above are mechanically checked against
  either — they are a written contract the code is *supposed* to meet.
- **Effect:** correctness for native means "matches the interpreter on the tests we ran",
  not "obeys the semantics". The unwritten parts (heap/store steps, iterators, coroutines)
  have no spec but the interpreter's code.
- **Status:** OPEN — **the oracle is BUILT and growing (@PLN89).** `tests/differential_oracle.rs`
  runs `tests/oracle/*.loft` (17 programs) on BOTH backends and asserts they AGREE on stdout
  (value/null), exit code (halt), and leak-freedom, with a positive control proving the detector
  fires.  **2026-07-04 coverage push** — the corpus now spans the divergence-prone areas where the
  two backends use the most different mechanisms: coroutines/generators (native state machine vs
  interp suspend), collection combinators (map/filter/comprehension), parallel reductions (par
  dispatch vs sequential), text (Rust String vs interp store), keyed collections (hash/sorted walk
  order + storage), and tuples/recursion — plus the two graduated cross-backend bugs (10/11).  **2026-07-04 — the DRIVER-AGREEMENT scope addition landed**: well-typedness is one static
  judgment, so `--dump` (pure parse+typecheck) / `--interpret` / `--native` must agree on
  accept-vs-reject; `statically_rejected()` (empty-stdout guard so a runtime panic isn't mistaken
  for a static reject) makes the #433 class — interp accepts what native rejects at rustc — a
  first-class caught property.  The oracle now catches real divergences in practice — three
  found this cycle: **#495** (runtime-Join over-free, FIXED), **#500** (native E0308 on a
  nested-ncc optional-text return, FIXED), **#501** (`.map`/`.filter` on a vector literal
  receiver, FILED).  Corpus is **23 programs** spanning coroutines / collections / parallel /
  text / keyed collections / tuples / nullability / nested enums / recursion / closures + the
  graduated bugs.  **NIGHTLY CI GATE WIRED (ci.yml, commit `971150dd`)**: the full
  `--ignored` sweep runs on the 03:00 UTC schedule + push-to-main (Linux-only, never on a PR),
  failing the nightly on any cross-backend divergence — the manual `-- --ignored` run is now
  a standing automatic guard.  Stays OPEN (the deviation closes only when a shared executable
  semantics replaces "the interpreter is the spec", or is reconciled): the corpus keeps growing.
- **Removal:** build a **differential oracle** — run a growing program corpus on BOTH
  backends and assert they AGREE (value / null / halt / stdout / leak); these rules stay the
  written contract that GUIDES the corpus (what behaviour to cover), not a third
  implementation. A mismatch is then a divergence caught before ship, and every fixed
  divergence grows the corpus. *Chosen for now over an executable shared semantics (both
  backends conforming to one definition) — switchable to that later; these rules are reused
  either way.*

### D-op-2 — interp/native divergences are test-caught, not definition-caught
- **Violates:** E-Op / E-Uncomp / the shared-contract premise
- **Where:** the two backends are kept in agreement by the suite, so a divergence ships
  until a test happens to exercise it. **#433** is the canonical case: a program the
  interpreter evaluated fine failed to *compile* natively (`E0308`), i.e. the backends
  disagreed on a program both should accept — caught by a test, not by the definition.
- **Effect:** every codegen fix this session (the bool-arg E0308, the `__native_tail_ret`
  lift) was a backend disagreeing with the interpreter; under a shared semantics each is a
  definitional error, found before shipping.
- **Status:** OPEN — downstream of D-op-1 (the differential oracle).  The oracle now covers
  BOTH facets of "backends disagree": the run-both-and-compare (value/halt/leak) for a program
  both ACCEPT, and — as of 2026-07-04 — the accept-vs-reject *driver-agreement* for the #433
  facet itself (interp accepts a program native rejects at rustc). Closes with D-op-1.
- **Removal:** the differential oracle (D-op-1) makes "interp and native disagree on a
  program both accept" a *caught* failure (run-both-and-compare), not a coverage lottery —
  the corpus, not luck, decides what is exercised.

> **D-op-4 — CLOSED (formalize4), so it is deleted from the list above.** The runtime no
> longer traps/halts on an uncomputable: div/mod-by-zero and integer overflow yield the null
> sentinel and continue on BOTH backends (E-Uncomp + E-Report), OOB already complied, and
> `NullDereference` was never raised. Guard: `tests/scripts/184-i333-div-zero-null-continues.loft`.
> The `??` trap-suppression mode is gone behaviourally (the `*Nullable` op split is now dead
> code — a separable cleanup). Kept as a one-line tombstone because it reshaped two rules
> (E-Report's logging policy + the C80 refinement); see `git log` for the full entry.

---

## Conformance

The pinned rules are checkable directly: `5 / 0` is **null** and execution continues (an
unguarded site also logs a `divide_by_zero` Warn); `a + 1` at `a = i64::MAX` is **null** and
continues; `(i64::MAX + 1) ?? 0` is `0` (E-Coalesce); `integer` null is `i64::MIN`.
D-op-1/D-op-2's falsifier is any program where the interpreter and `--native` disagree —
e.g. #433's cbor `read_value` (interp `20`, native E0308 pre-fix). When the rules become the
shared oracle, that disagreement is the definitional error, and this doc is the definition it
fails against.
