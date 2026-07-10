<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/matching.md — semantics for `match` (strict)

**Catalogue:** @F3 (enum/match core), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the relation for the `match`
> expression — enum-variant dispatch with payload binding. It is the second control form
> [operational.md](operational.md) pins only half of (`if`, not `match`). It extends
> operational.md (control flow, expressions) and [heap.md](heap.md) (an enum value is a tagged
> heap value; a variant pattern reads its payload). Every rule is a **user-visible contract**
> verified on both backends.
>
> A `match`'s headline guarantee is **compile-time exhaustiveness**: a `match` that forgets a
> variant does not compile. That is a promise to the user, checked before the program runs.
>
> **@PLN35 extension (planned, SPEC-FIRST):** the § *Rules — PEG patterns* below adds sequence /
> alternation / optional / repetition / capture patterns as the **target** for the PEG build (not
> yet shipped). It generalises this exhaustiveness guarantee to `M-Total`, so the promise survives
> patterns that can *fail*.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩`. An enum value `v` has a **variant
tag** and, for a struct-payload variant, named payload **fields**. A `match` is
`match e { pat₁ => b₁, …, patₙ => bₙ }`; a pattern is a unit variant `V`, a struct-payload
variant `V { f₁, … }`, or the wildcard `_`.

---

## Rules

### `match` is an expression that selects the first matching arm

```
  (M-Match)   ⟨match e { pat₁ => b₁, … }, σ⟩ → ⟨e', σ⟩          when e → e'   (scrutinee first)
              ⟨match v { pat₁ => b₁, … }, σ⟩ → ⟨bₖ[binds], σ⟩
                where k is the SMALLEST index whose patₖ matches v, and binds is patₖ's bindings.
  (M-Expr)    match is an EXPRESSION: every arm body bᵢ has the match's result type, and the
              selected arm's value is the whole match's value (feeds directly into `r = match …`).
```

**In words.** `match` first reduces the scrutinee to a value, then picks the **first** arm (top to
bottom) whose pattern matches, binds that pattern's variables, and evaluates its body — the body's
value **is** the match's value, so `r = match c { … }` is normal (verified: it returns `100`).
Only the selected arm runs.

### Patterns — unit, struct-payload with field binding, wildcard

```
  (M-Unit)     pattern V     matches an enum value whose variant is the unit variant V.
  (M-Variant)  pattern V { f₁, …, fₘ }  matches a value whose variant is V, BINDING each fⱼ to the
               corresponding payload field of v (by name), in scope for that arm's body.
  (M-Wild)     pattern _     matches ANY value; it is the catch-all.  It must be the LAST arm —
               an arm after `_` is a STATIC error (unreachable).
```

**In words.** A unit variant (`Dot`) matches by tag alone; a struct-payload variant
(`Circle { r }`, `Box { w, h }`) matches by tag AND binds its payload fields by name into the
arm, so `Circle { r } => r * r` uses the matched value's `r` (verified: `25` for `r = 5`). The
wildcard `_` matches everything and is the default — it must come last, because any arm written
after it could never run (loft rejects that at compile time).

### Exhaustiveness is checked at compile time

```
  (M-Exhaust)  a match on an enum must cover EVERY variant — each variant by its own arm, or a
               trailing `_`.  A match missing a variant is a STATIC ERROR
               ("match on E is not exhaustive — missing: …"), NOT a runtime fault.
```

**In words.** The compiler proves a `match` handles every case: if you add a variant to an enum,
every `match` that forgot it stops compiling with a precise "missing: …" message (verified). This
is the load-bearing guarantee — a `match` can never fall through to nothing at runtime, so there
is no "unmatched value" runtime error in loft's model; the exhaustiveness is discharged
statically, before the program runs.

---

## Rules — PEG patterns (@PLN35, SPEC-FIRST · planned, NOT yet implemented)

> **@PLN35 · SPEC-FIRST.** Unlike everything above (shipped semantics, pinned by the oracle), the
> rules in THIS section are the **target** for the PEG match-pattern extension — written *ahead* of
> the code at the maker's direction, so the phased build in
> [../plans/35-match-peg/](../plans/35-match-peg/) is constructed to satisfy them. They are **not
> yet met by either backend.** Each is pinned per-phase by the @PLN89 oracle as its phase lands
> (worklist: [VERIFICATION.md § matching.md — PEG patterns](VERIFICATION.md)). Until a rule's phase
> ships, read it as design, not a guarantee. Overview + phase↔rule map:
> [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

PEG patterns generalise a *point* pattern (unit/struct variant, `_`) to a **sequence** that may
branch (`|`), skip (`?`), repeat (`*`/`+`), and **capture** sub-results — over a vector/slice or an
iterator. The load-bearing constraint is that they must **preserve `M-Exhaust`**: a structural
pattern can *fail*, so totality is re-secured by requiring a total final arm (`M-Total`).

### The pattern-match relation

An input is walked by a **cursor** `κ = ⟨i, src⟩` — [iteration.md](iteration.md)'s iterator: an
index `i` into a source `src`, with `elem(src,i)` / `len(src)` **null past the end, never a fault**
(`I-Done`). The relation:

```
  ⟨pat, κ, σ⟩ ⇓ Match(binds, κ')     pat matches, consuming κ→κ', binding binds
  ⟨pat, κ, σ⟩ ⇓ Fail                  pat does not match — κ and σ UNCHANGED (P-Atomic / INV-Pure)
```

```
  (P-Point)  a unit variant V, struct variant V{f…}, literal, `_`, or bare binding is a POINT
             pattern over one value (today's M-Unit/M-Variant/M-Wild lifted into ⇓).  A struct /
             variant FIELD may itself be a pattern (nested) — the recursion this extension adds.
  (P-Seq)    ⟨[p₁ … pₙ], κ⟩: run p₁ from κ→κ₁, …, pₙ from κ_{n-1}→κₙ; ANY pᵢ ⇓ Fail ⟹ the whole
             sequence ⇓ Fail (κ unchanged).  binds = ⋃ᵢ binds_i.
  (P-Whole)  an ARM's sequence pattern must consume the WHOLE input (κ' = ⟨len(src),src⟩); a proper
             PREFIX ⇓ Fail for arm-selection UNLESS the sequence ends in `..rest`, which absorbs the
             remainder.  (This is why `[a,b,c]` needs exact length today.)
  (P-Alt)    ⟨(a | b), κ⟩: try a from κ; if Match, that; else try b from the SAME κ.  Ordered choice
             — FIRST success wins; both Fail ⟹ Fail.
  (P-Opt)    ⟨(a)?, κ⟩: try a; on Match(bs,κ') that; on Fail ⟹ Match(bs↦null, κ) — succeeds with a's
             captures null, cursor UNMOVED.  (P-Opt never Fails.)
  (P-Rep)    ⟨(a)*, κ⟩: greedily match a from κ→κ₁→…; on the first Fail at κ_m ⟹ Match(collected, κ_m).
             `(a)+` = a then (a)*.  A separator `*(s)` is consumed between iterations, not captured.
             BOUNDED by len(src) for slices ⟹ terminates; for iterators, by `max_lookahead` (P-IterBound).
  (P-Cap)    ⟨name:p, κ⟩: run p; on Match(bs,κ') ⟹ Match(bs ∪ {name ↦ p's result}, κ').
  (P-Rest)   ⟨..name, κ=⟨i,src⟩⟩ ⟹ Match({name ↦ a FRESH vector of src[i .. len−t]}, ⟨len−t, src⟩),
             t = fixed patterns after the rest (H-Alloc — a new store, independent of src).
  (P-Multi)  a MULTI-PATTERN arm `pat_a, pat_b => body`: try pat_a from ⟨0,v⟩ (whole-match); else
             pat_b; the FIRST whole-match commits.  (P-Alt at arm granularity — no new cursor work.)
  (P-Atomic) ⟨pat,κ,σ⟩ ⇓ Fail ⟹ σ UNCHANGED, κ not advanced (INV-Pure).  Provisional captures from a
             failed attempt are NEVER observable — the arm body runs ONLY after a committed whole-match.
```

**In words.** A pattern either matches — moving the cursor forward and binding names — or fails,
leaving everything exactly as it was. A sequence runs its parts in order and fails as a whole if any
part fails; an arm's sequence must line up with the *entire* input unless it ends in `..rest`.
Alternation tries its branches left to right and takes the first that works; an optional either
matches or quietly binds its captures to null without moving; a repetition matches greedily and
stops at the first failure, collecting what it got. Crucially, a *failed* attempt is invisible — no
half-bound name, no half-moved cursor leaks to the next arm (`P-Atomic`), which is what makes
backtracking safe.

### `M-Exhaust` generalises to `M-Total` (the invariant this extension must not break)

```
  (M-Total)  total(pat):
               total(_) = total(bare name) = true
               total(V) / total(V{f…}) = true  iff every field sub-pattern is total
               total(sequence | alternation-not-covering | optional-in-required-pos | repetition |
                     length-constrained slice | literal) = false
             A match SATISFIES INV-Total (never fails to select an arm at runtime) iff EITHER
               (enum subject) its total arms cover every variant,  OR
               its FINAL arm's pattern is total (a `_`, a bare binding, or a full variant cover).
             A match with a non-total pattern and no total final arm is a STATIC ERROR
               ("match is not exhaustive — a structural pattern can fail; add a `_` arm").
```

**In words.** This is the one rule that keeps loft's promise that a `match` never falls through to
nothing. A PEG pattern *can* fail, so on its own it is not enough — the compiler requires a final
arm that always matches (`_`, a bare name, or a set of enum arms that jointly cover every variant).
With that final arm present, some arm always fires; without it, the program does not compile. It is
the exact generalisation of `M-Exhaust`: for a pure-enum match, nothing changes.

### Iterator inputs add the only new operational primitive

For a **vector/slice**, anchor/revert is just save/restore of `i` — pure
[operational.md](operational.md) assignment, **no new op**. For an **iterator** (a source that
cannot be re-indexed), a failed alternative must *replay* pulled items, so two ops are added — the
`Lexer::memory` + `links`-refcount model (`src/lexer.rs`):

```
  (P-Anchor)    OpMatchAnchor: push ⟨i, epoch⟩; while any anchor is live, next(it) APPENDS the pulled
                item to a memo buffer instead of discarding it.
  (P-Revert)    OpMatchRevert: pop the anchor, rewind i (replaying from the memo), drop bindings
                written after epoch.  The buffer clears when the anchor stack empties (refcount 0).
  (P-IterBound) a repetition over an iterator is bounded by `max_lookahead`; exceeding it is a
                DEFINED runtime error (never a hang) — preserving termination.
```

A side-effecting pull (a generator that mutates external state per item) cannot be reverted;
matching over such a source is UB-by-contract (documented in [../CAVEATS.md](../CAVEATS.md)) — the
same assumption `Lexer` makes about its token stream.

Captures follow [types.md § Pattern captures](types.md) (no new type former — `τ` / `τ?` /
`vector<τ>` via the join) and [binding.md § Pattern captures](binding.md) (a single interior capture
is a view; `..rest` / repetition are fresh vectors); the pattern grammar + precedence are in
[grammar.md § Pattern-operator precedence](grammar.md).

---

## Deviations

OPEN: **0** (a *rules* doc — it shrinks operational.md's D-op-1, adds no code deviation).

- **PEG patterns are SPEC-FIRST (@PLN35)** — the *Rules — PEG patterns* § is written ahead of the
  code, so it opens **no** deviation (there is no implementation yet to break a rule). The gap is a
  build obligation tracked in [VERIFICATION.md § matching.md — PEG patterns](VERIFICATION.md) and
  pinned per-phase by the @PLN89 oracle; each rule graduates to a ✓ there as its
  [plans/35-match-peg](../plans/35-match-peg/) phase lands.
- **Conformance is differential** — `match` dispatch is enforced across the two backends by the
  @PLN89 oracle (D-op-1): `20-nested-enum-match` and `07-enum-match-dispatch` carry struct-payload
  variants, recursive walks, and matches whose arms return different variants, precisely because
  the native tag dispatch + payload layout differ from the interpreter's. A divergence in which
  arm fires, or in a bound payload value, is caught there.
- **Exhaustiveness is a STATIC judgment** — so it also participates in the oracle's
  *driver-agreement* facet (D-op-2): `--dump` / `--interpret` / `--native` must agree that a
  non-exhaustive match is rejected.

---

## Conformance

- **Arm selection + payload bind (`M-Variant`)** — `match Sh::Circle { r: 5 } { Dot => 0,
  Circle { r } => r*r }` is `25`.
- **Wildcard default (`M-Wild`)** — `match C::D { A => 1, _ => 0 }` is `0`; an arm after `_` is a
  compile error.
- **Exhaustiveness (`M-Exhaust`)** — `match c { A => 1 }` over `enum C { A, B }` does NOT compile
  ("missing: B"); adding a `B => …` arm or a trailing `_` makes it compile.
- **As an expression (`M-Expr`)** — `r = match c { A => 100, B => 200 }` binds `r` to the arm's
  value (`100`).

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on which
arm a `match` selects, on a bound payload value, or on whether a match is exhaustive is the
definitional error this doc names.
