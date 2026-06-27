<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Materialization algorithm — lifetime-driven copy / borrow / move, with a result-test before wiring

> **Status:** design (doc-first, `design-protocol`). The decision *algorithm* that
> consumes the [USE-analysis pre-work](use-analysis-prework-design.md) and produces the
> Borrow/Copy/Move verdict from **lifetime information**, plus the harness that lets us
> **test the verdicts in isolation — before they drive any codegen.** Sibling of
> [copy-elision-design.md](copy-elision-design.md) (the envelope) and
> [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) (the north star).

The two halves are deliberately separable: the algorithm emits a *verdict per binding*;
the harness *validates verdicts* while the program still runs the old (copy) code. We
only cut a verdict into codegen after it has survived the harness.

---

## 1. The lifetime model (what the algorithm reads)

loft already computes, per function, what the algorithm needs — this is not new
machinery, it is reading existing facts in one place:

- **Program-point order.** `compute_intervals` (`src/variables/intervals.rs`) assigns a
  monotonic sequence number to each IR node. Call it `seq(p)`.
- **Live interval.** Each var carries `first_def` and `last_use: u32`
  (`src/variables/mod.rs`). `live(v) = [first_def(v), last_use(v)]`. If `v` *escapes*
  (returned / stored into longer-lived heap / captured), `last_use(v) = ∞` (its value
  outlives the frame).
- **Scope nesting + free point.** Each var has a `scope`; a store is freed at its
  owner's scope exit. `free(o)` = `seq` of owner `o`'s scope-exit (or `o`'s
  reassignment). A **parameter**'s scope is the whole function ⇒ `free(param) = ∞`.
- **Owner graph (`deps`).** `owner(x)` follows `deps` transitively to the var that owns
  the store (empty `deps`), or to the parameter the chain borrows from. (`deps` empty ⇒
  owns; non-empty ⇒ borrows — `scopes.rs`.)
- **Mutation points.** `muts(S)` = the set of `seq` at which store `S` is written — a
  write to the field, to its owner, or through any tracked alias. Computed by the
  USE-analysis walk (pre-work §3). **Store generation** (`Store.generation`,
  `src/store.rs`) gives the dynamic counterpart used by the harness (§5).
- **Read-only flag.** `writes(v)` from the USE-analysis (`mutated`).

---

## 2. The divergence events as lifetime predicates

The copy-elision envelope (copy-elision §2) says borrow ≡ copy iff none of D1/D2/D3 can
occur. Each becomes a **lifetime query** over the model above, for a binding
`v = src.f` (or `v = src`):

| Event | Lifetime predicate that *forbids* it (must hold to borrow) |
|---|---|
| **D1** write through `v` | `¬writes(v)` — `v` has no write use |
| **D2** source mutated while `v` live | `muts(store(src.f)) ∩ (first_def(v), last_use(v)] = ∅` |
| **D3** `v` outlives source store | `last_use(v) < free(owner(src))` on every path (escape ⇒ `last_use=∞` ⇒ fails) |

So the whole decision is: *does `v`'s live interval sit strictly inside the source
store's live interval, with no mutation of that store in between, and no write through
`v`?* A pure interval-containment question — which is why lifetime information is the
right and sufficient spine.

---

## 3. The algorithm

```
materialize(binding  v = rhs)  ->  { Move(src) | Borrow(src) | Copy | OwnFresh }:

  match rhs_source(rhs):
    None ->                                   # literal / call-return / arithmetic: fresh storage
        return OwnFresh                        # unchanged from today

    Some(src):
      # ── MOVE: src is a temporary that dies INTO v (adopt its store; no copy) ──
      if is_temp(src) and last_use(src) == first_def(v):
          return Move(src)                     # = the existing NRVO adopt (#462)

      o = owner(src)
      # ── BORROW: v's live interval is contained in the source store's, unmutated ──
      if  not writes(v)                                            # ¬D1
      and muts(store(src.f)) ∩ (first_def(v), last_use(v)] == ∅    # ¬D2
      and last_use(v) < free(o):                                   # ¬D3 (escape ⇒ ∞ ⇒ false)
          return Borrow(src)                   # alias src.f's store, skip_free

      # ── COPY: the safe default ──
      return Copy
```

Precedence **Move > Borrow > Copy**. Every branch but Copy requires a *positive* proof;
anything the analysis cannot prove falls through to Copy (sound by default — pre-work §2).

**Tier-0 specialization (the `tile_at`/`edge_wall_raw` win).** When `o` is a **parameter**:
- `free(o) = ∞` ⇒ `last_use(v) < free(o)` is trivially true → **¬D3 free** for any
  non-escaping `v`;
- a read-only / by-value param is not mutated ⇒ `muts(store(src.f)) = ∅` → **¬D2** trivial.

So Tier-0 borrow collapses to: **`¬writes(v) ∧ ¬escapes(v) ∧ owner(src) is a read-only param`** —
exactly the accessor pattern, decided from `writes`/`escapes` + the owner-is-param fact,
no interval arithmetic needed. Later tiers turn on the full interval predicates (D2/D3)
for local and mutable sources.

---

## 4. The one invariant

> **A binding borrows iff its live interval is provably contained in its source store's
> live interval with no intervening mutation and no write-through; otherwise it copies
> (or moves a dying source).** The verdict is a function of *lifetimes*, computed once,
> never of RHS shape — and an unprovable containment is always answered Copy.

A case never tested borrows for the same reason the tested ones do: interval containment
is a total predicate over the lifetime model; an unseen code shape is classified by the
same containment test (and, if the walk can't model it, by the conservative default),
not by a shape enumeration that might omit it.

---

## 5. Testing the verdicts BEFORE wiring — the shadow oracle

The point the wiring-risk turns on: **we can check every Borrow verdict is correct while
the program still runs the COPY.** The copy we want to delete is the perfect oracle — at
`v`'s definition the copy *is* a snapshot of `src.f` at that instant. Three layers, none
of which changes program behavior:

### 5a. Static decision dump (verdict is what we expect)
`LOFT_MATERIALIZE_DUMP` prints one line per heap binding:
`file:line  v=<name>  verdict=Borrow|Copy|Move  reason=<predicate that decided>`.
A hand-labeled corpus (`tests/scripts/materialize/*.loft`, expected verdict per binding in
a sidecar / `@EXPECT` comment) asserts the algorithm computes the verdict we predicted.
This catches *logic* errors in the algorithm with zero runtime — the boundary matrix, at
the level of the decision.

### 5b. Dynamic shadow validation (verdict is actually SAFE) — runs while copying
Under `LOFT_MATERIALIZE_SHADOW` (test-only, zero-cost off), for every binding the
algorithm decided **Borrow**, the program *still copies* but additionally checks, at
runtime, that the borrow *would* have been safe — i.e. that the three predicates held on
this execution:

- **D1 — `v` stays read-only.** Watch writes to `v`'s (copy) store over `live(v)`; any
  write ⇒ the Borrow verdict was wrong (the static `¬writes(v)` missed an alias path).
  Mechanism: a shadow write-watch on the copy store; or, cheaper, assert `writes(v)` was
  false statically and corroborate by store `generation` deltas on `v`.
- **D2 — source unchanged across `live(v)`.** At `first_def(v)` we already hold the copy
  `C0` (= `src.f` at def). At `last_use(v)`, re-read the *live* `src.f` and compare to
  `C0` (value + length, or the cheaper `store.generation` recorded at def vs now). If it
  **differs**, a borrow would have made `v` observe the change → divergence from copy
  semantics → the Borrow verdict was unsafe. *This is a per-binding differential against
  the copy ground-truth, with the program semantics untouched.*
- **D3 — source store still alive at `last_use(v)`.** Assert `owner(src)`'s store is not
  freed at `seq(last_use(v))`. Under copy it never dangles; the shadow checks the owner
  liveness flag the borrow would have depended on.

A failed shadow assertion names the exact binding and predicate — **an unsafe verdict
caught with no borrow ever emitted.** The generation counter (`Store.generation`) already
exists for resize/realloc; the shadow extends it to bump on element writes too (gated by
the shadow flag, so production is untouched).

### 5c. The sweep gate (decisions are safe on real workloads)
Run the **shadow** over: the full test suite, and the crawler + moros_map dogfood (the
very workloads that surfaced the bottleneck). **Every Borrow verdict must pass the shadow
check on every execution.** A violation tightens the algorithm (more conservative) — it
never gets waved through. The cut-over (§6) is *gated on a clean shadow sweep*.

This is the requested property in full: the algorithm's **results are tested — statically
(5a) and dynamically against the copy oracle (5b) across real workloads (5c) — before a
single borrow is wired into codegen.**

---

## 6. Cut-over (only after the harness is clean)

1. 5a green: verdicts match the hand-labeled corpus.
2. 5b/5c green: zero shadow violations across suite + crawler + moros (Borrow verdicts
   are dynamically safe on every exercised path).
3. **Then** the elision rewrite (copy-elision §6 / pre-work §6) emits Borrow for the
   decided bindings.
4. Re-run with borrows LIVE: full suite + the §7 envelope matrix (value + length + leak,
   both backends) + crawler `surfacetest` perf gate. The non-elided paths' IR is
   byte-identical (`loft-codegen` refactor gate); only proven bindings change.

The shadow harness stays in the tree (behind its flag) as a standing regression
instrument: re-runnable on any future change to confirm no verdict went stale.

---

## 7. Falsification probes (design-protocol — before code)

Each row is the cheapest test that could prove a load-bearing claim **false**; expect to
falsify; run both backends.

| Claim | Probe | Required |
|---|---|---|
| containment ⇒ borrow-safe | `tile_at` shape: `v=s.f` read-only, param source | verdict Borrow; shadow clean; result correct |
| D1 catches write-through | `v=s.f; v[0]=9` | verdict Copy (writes(v)); if forced Borrow, shadow FIRES |
| D2 catches aliasing mutation | `v=s.f; s.f[0]=9; read v[0]` | verdict Copy; if forced Borrow, shadow D2 FIRES (live≠C0) |
| D3 catches escape/dangle | `v=s.f; return v` and `v=local.f; free local; read v` | verdict Copy; shadow D3 FIRES if forced |
| **cleanest claim (attack):** "interval containment is sufficient" | a mutation of `src.f` through an **untracked alias** inside `live(v)` | the static `muts` set MUST include it or the verdict MUST stay Copy — the shadow D2 is the backstop that catches a `muts` gap, proving we never ship a borrow the static set under-counted |
| shadow itself can fail (not vacuous) | a deliberately-wrong forced-Borrow on a mutated source | shadow MUST report a violation (a green shadow on a known-unsafe case = a broken harness) |

The last row is the harness's own falsifiability check (the matrix law: keep one
deliberately-red cell) — a shadow that never fires proves nothing.

---

## 8. Why this ordering is safe (the epistemic chain)

- **Soundness on unexercised paths** comes from the *static* side: conservative default
  (unknown ⇒ Copy) + the exhaustive-variant coverage sentinel (pre-work §8).
- **Correctness on exercised paths** comes from the *dynamic* shadow (5b/5c): the copy
  ground-truth falsifies any unsafe verdict the static analysis let through, *before* the
  borrow is emitted.
- **The residual** (an axis no probe and no dogfood path exercised) is handled the only
  way it can be: the shadow stays armed in-tree, so the first real execution that would
  have diverged fires the assertion instead of corrupting — the dogfood loop converting an
  unknown axis into a known one (design-protocol §residual).

So the algorithm is never trusted on assertion; it is trusted only where the copy oracle
has had a chance to refute it, and conservative everywhere else.
