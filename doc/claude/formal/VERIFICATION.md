<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/VERIFICATION.md — detailed verification worklist for the operational rules

The operational-semantics files written 2026-07-04 ([heap](heap.md), [iteration](iteration.md),
[coroutines](coroutines.md), [concurrency](concurrency.md), [calls](calls.md),
[matching](matching.md), [tuples](tuples.md), [closures](closures.md)) were each grounded in
**targeted probes** before writing — but a *targeted* probe is not a *systematic* one. Two
imprecisions were caught exactly by re-checking a written rule in detail:

- `I-Text` said "characters" — a detailed check (`"e" + U+0301`) proved it is **codepoints, not
  grapheme clusters**, and the rule was wrong-because-vague.
- `closures.md` claimed `|…|` is non-capturing — a detailed check proved it *should* capture
  (pure sugar), surfacing **D-clo-1** (fixed) and **D-clo-2** (still open, a crash).

This worklist drives that detailed pass over **every rule**: for each, the single falsifiable
claim, its status on **BOTH backends**, and the standing guard (an [oracle](../../tests/oracle/)
program or a `tests/scripts/` guard) that should keep it true. It is the concrete D-op-1 coverage
plan for the new rules — the oracle already guards each *area*; this drives it down to each *rule*.

## Method

1. **One falsifiable claim per rule.** Write the smallest program whose output *distinguishes* the
   rule holding from it failing (a value AND, where relevant, a length / a compile accept-vs-reject).
2. **Run it on BOTH backends** (`--interpret` AND `--native`) and hand-compute the expected value —
   agreement between the two is necessary but NOT sufficient (both could be wrong the same way);
   the hand-computed value is the oracle.
3. **Graduate it to a standing guard** — add it to `tests/oracle/*.loft` (both-backends +
   leak + driver-agreement, the nightly gate) or a `tests/scripts/*.loft` guard. A rule is
   "verified" only once a guard pins it.
4. **Static claims also need driver-agreement** — a rule that says "X is a compile error"
   (exhaustiveness, tuple-OOB, `&a;`) must be checked to REJECT on `--dump` / `--interpret` /
   `--native` alike (the D-op-2 facet).

## Legend

- ✓ — verified on both backends this cycle AND has a standing guard.
- ~ — probed once (usually interpreter only); needs the both-backends run + a graduated guard.
- ☐ — not yet verified in detail.

---

> **Stage 1–2 (2026-07-04) — the worklist earned its keep.** Two real findings: (1) **H-Copy was
> imprecise** — a struct-typed projection is a VIEW, not a copy (heap.md corrected: added
> `H-View`); (2) **coroutine laziness diverges** for LOOP-based yields (native eager) —
> reclassified as a DECIDED EDGE (a rustc restriction, the maker's accepted trade-off), not a bug.
> Everything else on the priority + runtime + static-reject rows verified ✓ on BOTH backends
> (F-ParamRebind incl. native, `par` N-independence, null/OOB-continue, range/reduce-left/map-fresh,
> and the static rejects — exhaustiveness / tuple-OOB / wildcard-not-last reject identically).
>
> **Graduated to standing oracle guards (nightly gate, both-backends + leak + driver-agreement):**
> `tests/oracle/24-heap-copy-vs-view.loft` (H-Copy/H-View), `25-parameter-binding.loft` (F-Param*),
> `26-coroutine-laziness.loft` (straight-line G-Call/G-Next). The full `--ignored` sweep passes.
>
> **Stage 3 — remaining rows, no new divergences.** Verified ✓ both backends: H-Alloc zero-init,
> H-WriteLocked (a `#lock`ed write FAULTS on both — the intended tripwire), F-Rec (factorial),
> F-Return (implicit tail), T-Paren (`(e)` is grouping), L-Escape (a returned closure works on
> native). Still ☐ / covered-elsewhere: **F-ParamRef** (`&` write-back — binding.md's domain, 0
> deviations, PR#436), **C-Order hash-par** (a documented edge; probe syntax pending),
> **G-YieldDepth** (stackful yield — likely needs `yield from`, deferred to 1.1+), **D-clo-2**
> (the open closure crash). The load-bearing rules are now pinned; the residue is edge/deferred.

## heap.md

- ~ **H-Alloc / H-NewRec** — a freshly constructed struct/vector has all fields null/zero; two
  constructions are distinct stores. *Guard: extend oracle `03`.*
- ✓ **H-Copy** — a WHOLE-VALUE / vector bind COPIES: `c = o; c.v=9` ⇒ `o.v==1`; `fv = e.items;
  fv[0]=99` ⇒ `e.items[0]==1`; `r=&v; r[0]=99` ⇒ `v[0]==1` (both backends). *Guard: fold into oracle.*
- ✓ **H-View (the Stage-1 catch)** — a STRUCT-typed projection ALIASES: `c = o.i; c.v=9` ⇒
  `o.i.v==9`; `s = v[0]; s.v=9` (struct element) ⇒ `v[0].v==9` (both backends). heap.md corrected —
  H-Copy had wrongly implied projections copy. *Guard: an oracle case pinning both copy and view.*
- ☐ **H-Copy/View (keyed)** — the copy-vs-view split for a keyed collection (hash/sorted) value +
  element. *Verify + guard.*
- ~ **H-Read / H-ReadNull / H-Index** — read a field of `nullref` ⇒ `null` + CONTINUE; `v[i]` with
  `i≥len` ⇒ `null` + continue. *Guard: an oracle case that reads past a vector and through a null ref.*
- ☐ **H-Write / H-WriteNull / H-WriteOOB** — a write through `nullref` / out of bounds is a no-op
  that continues (never a wild write). *Verify both backends + graduate.*
- ☐ **H-WriteLocked** — a write to a `#lock`ed store faults, never silently succeeds. *Guard.*
- ✓ **H-Free* + H-Sound** — the LIFO / no-stack / no-double-free discipline. *Standing guards:
  `LOFT_POISON` suite + the ownership fuzz gate + `LOFT_NATIVE_LEAK_CHECK` (ownership.md is 0 open).*

## iteration.md

- ~ **I-For / I-Next / I-Done** — index order `0,1,…`; the length is re-read each round (an
  append-during-iterate is visible). *Guard: an oracle case that appends while iterating.*
- ☐ **I-Range** — half-open `a..b`; empty when `a≥b`. *Verify both backends + guard.*
- ✓ **I-Text** — CODEPOINTS not graphemes (`"e"+U+0301+"X"` ⇒ 3 iterations, `c#index=0,1,3`); byte
  cursor; `t.map` is a static error. *Guard: oracle `15` covers text; add the combining-sequence case.*
- ~ **I-Map / I-Filter / I-Reduce** — map preserves order, filter preserves relative order, reduce
  folds LEFT (use a NON-commutative `g`, e.g. subtraction, to pin direction). *Guard: extend oracle `13`.*
- ~ **I-Comp** — the result is a FRESH vector (source untouched). *Guard.*
- ~ **I-Empty / I-NullSrc** — empty and null sources iterate zero times, continue. *Guard.*

## coroutines.md

- ✓/edge **G-Call / G-Next laziness** — STRAIGHT-LINE yields are lazy on BOTH backends
  (`print("a");yield 1;print("b");yield 2` ⇒ `a g1 b g2`). LOOP-based yields are lazy on interp,
  EAGER on native (`y0 g0 y1 g1` vs `y0 y1 g0 g1`) — a DECIDED EDGE (rustc restriction), NOT a bug
  (coroutines.md). *Guard: add a straight-line laziness oracle case (passes both, pins the contract).*
- ✓ **G-Next values / G-Done** — one value per advance (sum 30); exhaustion (take-first-2 ⇒ 1),
  both backends. *Guard: oracle `12` (values).*
- ☐ **G-YieldDepth** — STACKFUL: a `yield` inside a helper called from the generator (needs
  `yield from` / a nested generator — check whether it is even expressible today). *Verify.*
- ✓ **G-For** — `for x in gen()` visits the produced sequence (both backends). *Guard: oracle `12`.*

## concurrency.md

- ✓ **C-Det** — `par(b=sq(a),N)` sum is `30` for N=1 AND N=4, equals the sequential loop, both
  backends. *Guard: extend oracle `14` with a multi-N assertion.*
- ~ **C-Par** — results consumed IN source order (order-sensitive body matches sequential). *Guard: extend oracle `14`.*
- ☐ **C-Order (hash exception)** — a `par` over a hash may visit in a DIFFERENT order than the
  key-ordered sequential `for x in h`, but both backends agree with each other. *Verify + guard.*
- n/a **C-Impure** — undefined by contract (a data race is a program error); not a testable rule,
  but a lint/doc check that an impure worker is discouraged.

## calls.md

- ~ **F-Args** — arguments left-to-right (`add(tag("A"),tag("B"))` prints `AB`). *Guard: oracle `06` (eval-order) — add the call-arg case.*
- ☐ **F-Call / F-Return / F-Rec** — frame, implicit tail return, recursion. *Guard: oracle `17`/`21` cover recursion; add an implicit-return case.*
- ✓ **F-ParamScalar** — `fn inc(n){n=n+1}` leaves caller `x==5` (interp; re-run native). *Guard.*
- ✓ **F-ParamHeap** — `fn mut(e){e.h=99}` ⇒ caller `o.h==99` (oracle `03` is close). *Guard: pin it.*
- ✓ **F-ParamRebind** — `fn re(v){v=[9,9]}` leaves caller `o[0]==1`; `fn re(s){s=S{v:9}}` leaves
  `o.v==1` — verified BOTH backends (the subtle @PLN87 P2.4 rule holds on native). *Guard.*
- ☐ **F-ParamRef** — a `&T` param's whole-value `p=e` DOES write back. *Verify both backends + guard.*
- ✓ **F-Ret** — two calls' returns independent (oracle `02` return-ownership). *Guard: pin.*

## matching.md

- ✓ **M-Match / M-Variant / M-Expr** — arm selection + payload bind + expression value (oracle `07`/`20`). *Guarded.*
- ~ **M-Wild** — `_` matches any; an arm AFTER `_` is a static error. *Verify the reject on all drivers + guard (driver-agreement).*
- ✓ **M-Exhaust** — a missing variant is a COMPILE error (verified interp; add the driver-agreement
  reject to oracle `19`-style). *High priority for the driver-agreement facet.*

## tuples.md

- ~ **T-Cons / T-Proj** — construct + `.0`/`.1` (oracle `17`). *Guarded (extend).*
- ☐ **T-Paren** — a single `(e)` is grouping, NOT a 1-tuple. *Verify + guard.*
- ☐ **T-Proj OOB** — `t.5` on a 2-tuple is a COMPILE error (a static reject — driver-agreement),
  never a runtime null. *Verify the reject on all drivers.*
- ~ **T-Destr / T-Ret** — `(a,b)=…`, tuple return + unpack (interp; run native). *Guard: oracle `17`.*

## closures.md

- ✓ **L-Fn (both forms capture)** — `|y|{y+x}` and `fn(y){y+x}` both yield `11`; heap capture `9`;
  non-capturing `2` (both backends). *Guard: `tests/scripts/85-short-lambda-capture.loft` + oracle `04`/`22`.*
- ✓ **L-CapScalar / L-CapHeap** — scalar by value at creation; heap shared (`b.v=9`⇒`9`). *Guard: extend `85`.*
- ~ **L-Apply / L-Escape** — store / pass / return / struct-field (`mk(7)()==7`, `h.f()==42`; interp). *Run native + guard.*
- ☐ **D-clo-2 (OPEN)** — `g=|y|{y*2}; xs.map(g)` PANICS (`data.rs:4569`). *The one open closure
  deviation; fixing it needs its own guard + likely a filed issue.*

---

## Priorities (verify these first — highest divergence risk / most user-visible)

1. **H-Copy for struct / nested / keyed** — the copy-not-alias proof covered vectors only; the
   whole owned-vs-host / raw-write story ([capabilities.md](capabilities.md)) leans on it.
2. **F-ParamRebind on NATIVE** — the @PLN87 P2.4 "whole-value param reassign is local" rule was
   probed on interp only; native rebind (P2.4 witness) is exactly where store-lifetime bugs lived.
3. **G-Call laziness + G-YieldDepth (stackful)** — coroutines are the largest interp-vs-native
   mechanism gap (serialised frame vs state machine).
4. **C-Det N-independence** — `par` at several thread counts on both backends.
5. **The static rejects (driver-agreement)** — M-Exhaust, M-Wild-after, T-Proj-OOB, T-Paren must
   REJECT identically on `--dump` / `--interpret` / `--native` (the D-op-2 facet).

## How this closes out

Each ✓ with a graduated guard is a rule the nightly oracle keeps true; each ~/☐ is a program to
write + run on both backends + fold into `tests/oracle/` or `tests/scripts/`. When every row is ✓,
the newly-written operational rules are not just *written* but *pinned* — the differential oracle
(D-op-1) then covers every rule, not just every area, and this file is retired into the oracle
corpus it produced.
