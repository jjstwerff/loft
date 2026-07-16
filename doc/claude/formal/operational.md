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
> (`match` + exhaustiveness; + PEG patterns and the two new ops `OpMatchAnchor` / `OpMatchRevert`,
> @PLN35 SPEC-FIRST), [tuples.md](tuples.md), [closures.md](closures.md) (lambdas /
> closures / fn-refs), [formatting.md](formatting.md) (`"{x}"` interpolation + value→text
> rendering), and [interfaces.md](interfaces.md) (interfaces + generics — a static/typing area).
> Every sibling file is now at **0 own deviations** (closures' D-clo-1/2 closed 2026-07-04;
> formatting + interfaces written 2026-07-05). The operational contract is now written across the
> whole family — nothing is left to "the interpreter is the spec" except the differential-oracle
> meta-deviation (D-op-1) itself.

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
  (E-Null)   `null` is a value, represented IN-BAND by a specific reserved bit-pattern per
             scalar width — `integer` = `i64::MIN`; a narrow int = its top value (`u8` = 255);
             `float`/`single` = a reserved NaN; `character` = codepoint 0; a reference =
             `nullref`.  That pattern is a REAL, OBSERVABLE value, and it is RESERVED: it is
             EXCLUDED from the non-null range of `τ` (so a non-null `integer` is
             `[i64::MIN+1, i64::MAX]`, symmetric).  No legitimate non-null value equals it.
             Both backends MUST agree on the abstract value AND on the reserved pattern per
             width (the pattern is part of the observable contract, not a private encoding).
```

**In words.** A value is "done" — it doesn't evaluate further. `null` is a real value, not a
separate state; each scalar **width** reserves ONE bit-pattern for it (an `integer` null is
the smallest `i64`; a `u8` null is 255; a `float` null is a reserved NaN). That pattern is
**in-band and observable** — it is a value in the same slot — so the non-null value **range
excludes it** and no real value can silently be confused with null. (This corrects the earlier
claim that "how a backend encodes the sentinel is its business": the encoding is in-band, so it
IS observable and part of the frozen contract — see the null-sentinel keystone,
[plans/102-stability-contract/keystone-null-model.md](../plans/102-stability-contract/keystone-null-model.md).)
The cost — a *nullable* narrow type cannot store its one reserved value (`u8?` has no 255,
`integer?` no `i64::MIN`, `character?` no `'\0'`) — is a deliberate, documented limitation of
the in-band model, not a silent one.

### Evaluation order — left to right

```
  (E-Left)   in a binary form `e₁ op e₂` (op NOT short-circuiting), reduce e₁ to a value
             first, then e₂:
                 ⟨e₁, σ⟩ → ⟨e₁', σ'⟩   ⟹   ⟨e₁ op e₂, σ⟩ → ⟨e₁' op e₂, σ'⟩
                 ⟨v₁ op e₂, σ⟩ → ⟨v₁ op e₂', σ'⟩   when   ⟨e₂, σ⟩ → ⟨e₂', σ'⟩
  (E-And)    `e₁ && e₂` reduces e₁ first; if e₁ is false the whole form is false and e₂ is
             **NOT** evaluated (short-circuit); otherwise the form reduces to e₂.
  (E-Or)     `e₁ || e₂` reduces e₁ first; if e₁ is true the whole form is true and e₂ is
             **NOT** evaluated; otherwise the form reduces to e₂.
```

**In words.** Operands evaluate left first, then right — so any side effects (a call that
mutates the store) happen in source order. Both backends must use this order. The **only**
exception is the short-circuiting logical operators `&&`/`||` (and their `and`/`or` spellings):
they reduce the left operand, and evaluate the right operand *only* when the left has not
already decided the result — verified on both backends. Every other binary op (arithmetic,
comparison, `??`) evaluates both operands under E-Left.

### Arithmetic — uncomputable yields null (the spreadsheet model)

```
  (E-Op)        ⟨v₁ op v₂, σ⟩ → ⟨v, σ⟩          where v = v₁ op v₂ is representable
  (E-Uncomp)    ⟨v₁ op v₂, σ⟩ → ⟨null, σ⟩       where the result is NOT computable — `v₁ op v₂`
                                                overflows the type, or op is `/`/`%` with
                                                v₂ = 0.  The result is **null**; evaluation
                                                CONTINUES (it never halts).
  (E-NullArg)   any op with a `null` operand produces `null` (null is CONTAGIOUS),
                EXCEPT comparisons, which are DEFINITE against the reserved pattern and
                UNIFORM across every scalar type:
                  `null == null` → true;  `v == null` / `null == v` → false (v non-null);
                  `!=` is the exact complement of `==`;
                  ordering (`<` `>` `<=` `>=`) places `null` at the LOW extreme —
                  `null < v` → true, `v < null` → false, `null < null` → false —
                  the SAME for `integer`, `character`, `float`, `single`, `boolean`.
```

**In words.** Arithmetic gives the obvious result when it fits. When it *can't* — overflow,
divide/modulo by zero — it yields **null** and the program **keeps running**; it does not
halt. Comparisons are the exception to contagion: they let you *test* for null (`x == null`)
and give a **total order** with null sorting first, and this is **uniform across scalar
types** — `null == null` is always true, never type-dependent. (`float`/`single` null was a
NaN, so `null == null` used to be false and ordering unordered — deviation D-op-null-1, CLOSED
by keystone step 2 (2026-07-10); both are now uniform with the integer/char behavior.) This is
the **spreadsheet model** ([DESIGN_DECISIONS.md C80](../DESIGN_DECISIONS.md)): a
cell that can't compute shows null and never stops the other cells. A fault is *local* — it
degrades one value, never the whole run. The same holds for every uncomputable step (an
out-of-bounds index, a deref of an absent value): null, continue.

**Float `==` is exact, never epsilon.** For two *non-null* floats, `==`/`!=` compare the IEEE
values exactly — `1.0 == 1.0000000001` is **false**, `0.1 + 0.2 == 0.3` is **false** (the sum is
`0.30000000000000004`), and `!=` is the exact complement of `==`. There is no tolerance band. The
ordering operators (`<` `<=` `>` `>=`) agree with it: among non-null floats they form a **total
order** — NaN cannot occur (it is represented as null, D-op-null-1), so exactly one of `a < b`,
`a == b`, `a > b` holds for every pair, and no value is ever both `<` and `==` its neighbour.
`single` (32-bit) behaves the same. Verified both backends.

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
  (E-Asgn)   ⟨x = v, σ⟩ → ⟨v, σ[x ↦ v]⟩                 (the LHS place reduces first —
                                                        left-to-right — THEN the RHS, by E-Left)
  (E-Seq)    ⟨v ; s, σ⟩ → ⟨s, σ⟩
  (E-IfT)    ⟨if true { s } else { t }, σ⟩ → ⟨s, σ⟩      (and E-IfF for false)
```

**In words.** A variable steps to its stored value; an assignment reduces its left-hand
place first (left-to-right), then its right side, then updates the store; a sequence drops a
finished statement; an `if` picks the branch
its (already-evaluated) condition selected. Standard — pinned here only so both backends
share them.

---

## Deviations

OPEN: **2** (D-op-1/2; the null-model keystone deviations D-op-null-1/2 both CLOSED 2026-07-10 by
keystone steps 2–3. Opened 2026-07-10 by the @PLN102 pre-freeze audit —
[the null-model keystone decision](../plans/102-stability-contract/keystone-null-model.md).)

### D-op-null-1 — CLOSED (2026-07-10, keystone step 2): float/single null comparison now uniform
- Was: `float`/`single` null (a NaN) made `null == null` **false** and ordering unordered, where
  integer/char null is reflexive and orders low — violating `(E-NullArg)`'s uniformity.
- Fixed at the single source: the `Op{Eq,Ne,Lt,Le}{Float,Single}` `#rust` bodies in
  `default/01_code.loft` (which drive BOTH the interpreter via `fill.rs` regen and native codegen)
  now treat a NaN operand as null definitely — `null == null` true, `!=` its exact complement,
  null orders at the low extreme — matching `(E-NullArg)` and the integer/char behavior. Verified
  both backends against the matrix; guard `tests/scripts/pln102-null-comparison-uniform.loft`. The
  conversion set (docs/tests on the old `x != x` NaN idiom → `== null`) was migrated in the same
  change.

### D-op-null-2 — CLOSED (2026-07-10, keystone step 3): collision sites report, no longer silent
- Was: an op whose true result is the reserved `i64::MIN` pattern (or an out-of-range shift/cast)
  silently masked, saturated, or nulled a real value — the silent-wrong class `(E-Null)` forbids.
- Fixed at the single `#rust` source (drives BOTH backends), mirroring `÷0` (report + null +
  continue):
  - **Shifts** (step 3a): `OpSLeftInt`/`OpSRightInt` report `ShiftOutOfRange` on an amount outside
    `[0, 64)` or a left shift landing on `i64::MIN` (`1 << 63`); null operands stay contagious.
  - **Casts** (step 3b): `OpCastIntFromFloat` reports `CastOutOfRange` on a float outside integer
    range (was: saturate to `i64::MAX`); `OpConvCharacterFromInt`/`OpCastCharacterFromInt` report on
    an invalid code point (was: silent NUL); `OpCastIntFromText` reports when a *valid* number parses
    to exactly `i64::MIN` (an unparseable text stays DN3-nullable → null, silently, unchanged). NaN
    floats and null integers stay contagious.
- Distinct from **C85** overflow of ordinary arithmetic, which is a decided edge and stays silent.
- Both backends; guards `tests/scripts/pln102-shift-collision-guard.loft` +
  `tests/scripts/pln102-cast-collision-guard.loft`; the conversion set was one assertion
  (`inf as integer` saturate → null in `02-floats.loft`).

### D-op-1 — there is no shared operational semantics; the interpreter is the spec
- **Violates:** the premise of this doc (a single evaluation relation both backends obey)
- **Where:** `src/state/` (the interpreter) is the de-facto *executable* definition;
  `src/generation/` (native) is a *separate* generator. The rules across this operational
  family — this file's scalar core plus [heap](heap.md) / [iteration](iteration.md) /
  [coroutines](coroutines.md) / [concurrency](concurrency.md) / [calls](calls.md) /
  [matching](matching.md) / [tuples](tuples.md) / [closures](closures.md), all written
  2026-07-04 and each at 0 own deviations — are a written contract the code is *supposed* to
  meet, but none is mechanically checked against either backend.
- **Effect:** correctness for native means "matches the interpreter on the tests we ran",
  not "obeys the semantics". As of 2026-07-04 the gap is **no longer missing rules** — the
  operational rules are now written for every core area (store alloc/read/write/copy/free,
  iteration + combinators, coroutines, `par`, calls, `match`, tuples, closures, text
  formatting, interfaces/generics — the last two added 2026-07-05). What remains is that those
  written rules are not enforced against a *single evaluation relation both backends share*:
  they GUIDE the differential oracle rather than mechanically defining agreement. Nothing is
  left "spec = the interpreter's code" now — only the differential-vs-definitional gap itself.
- **Status:** OPEN — **the oracle is BUILT and growing (@PLN89).** `tests/differential_oracle.rs`
  runs `tests/oracle/*.loft` (26 programs) on BOTH backends and asserts they AGREE on stdout
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
  receiver, FILED).  Corpus is **26 programs** spanning coroutines / collections / parallel /
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
