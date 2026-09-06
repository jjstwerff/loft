<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN154 phase 3 — a handle whose record has moved

**Question.** Free is covered three ways (`LOFT_POISON`, `LOFT_UAF_GEN`,
`LOFT_STRICT_STORES`).  GROWTH is covered by nothing: a container that outgrows its
allocation is copied to a new record and the old one is freed, and every reference that named
the old one is silently wrong.  Can the shadow say so, at the read?

**Verdict — yes, and it reports on all three open issues in the class.**  loft#1373, #1377 and
#1384 each report exactly one site — the stale view — and the corpus is silent.

---

## The mechanism

Two halves, because the two facts live in different places.

**The move is recorded where both record numbers exist.**  `Store::resize`'s relocating
branch is `claim` / `copy` / `delete`; afterwards the old number is a free block like any
other, and a frame slot naming it is indistinguishable from one naming a slot that was simply
reused.  So the branch logs `(store, old, new)` — which is why `Store` now carries its own
`store_nr`, the piece phase 0 predicted this plan would owe.

**The scan is exact, and that is what phases 1-2 bought.**  After an operator that moved
something, the dispatch loop walks the live frame and reads only the slots the shadow already
says are the BASE of a handle — no treating every aligned word as a possible reference, and no
false hits off a number that happens to look like a record id.  A slot naming a moved record
has its tag's FAMILY changed to stale, keeping its width and index, so the read check finds it
in the same comparison as everything else.

Three properties are deliberate:

* **The whole live frame, not the top one.**  A view handed down a call is read in the
  callee's frame and lives in the caller's; probe `e` below is that case, and a top-frame scan
  misses it.
* **At the END of the operator.**  That is the first moment the containers that legitimately
  track the move have finished updating themselves — probe `c` reads the grown vector's own
  handle and must stay silent.
* **The mark is not the report.**  A slot is marked at the move and reported only if it is
  READ while stale; a slot rewritten in between is simply retagged.  Probes `a`, `b` and `c`
  all mark three to five slots and report none.

## The falsification — all three are OPEN issues, so HEAD is the broken build

| issue | shape | answer at HEAD | the shadow |
|---|---|---|---|
| **#1373** | `d: S = v[0]` then 200 appends | `d=4294967296` | 1 site: `get_var<DbRef> … names record 3, which has MOVED` |
| **#1377** | `b = w[0]` on a `vector<vector<integer>>` | `len=0` | 1 site |
| **#1384** | `e = w.a[0]`, a view of a FIELD's element | `e=4294967296` | 1 site |

One site each, and it is the view — not the container, not a temporary.

## The false-positive matrix

Five probes, one axis moved each, run on `--interpret`:

| probe | what moves | expected | measured |
|---|---|---|---|
| **a** the view is bound AFTER the growth | order of bind vs. growth | silent | silent, `d=1`; 3 slots marked, 0 reported |
| **b** the view is re-bound each iteration | rebinding | silent | silent, `d=1`; 5 marked, 0 reported |
| **c** the CONTAINER's own handle is read after the growth | which handle is read | silent | silent, `first=1 len=201`; 3 marked, 0 reported |
| **d** the growth stays INSIDE the allocation | whether a record moves at all | silent, and NO relocation | silent, and the relocation line is absent — the negative control |
| **e** the view is read inside a CALLEE | which frame holds the slot | reports | reports, at the `show(d)` line |

Probe **d** is the one that makes the rest mean something: it proves the mechanism speaks only
when a record actually moves, rather than being silent because nothing is armed.

## Reading a silent run

`LOFT_VERIFY_STACK_TRACE=1` names every handle-tagged frame slot the scan read and the
relocations it compared against.  The summary's `N record relocation(s); M frame slot(s) named
one` answers half the question — whether anything moved, and whether any slot named it — and
the trace answers the other half, which slot.  It is also what found the scan's own addressing
bug: the shadow is indexed absolutely (`rec * 8 + fld`) and `Store::addr` takes the FIELD, so
counting the record twice made every "handle" it printed a `store=8 rec=3414097922`.

## Corpus

**1106 of 1177 programs clean at HEAD on `--interpret`, zero reports** — the same count as
phases 1-2, so the growth half added no false positive of its own.  The remaining 71 exit
before an operator runs (refusal tests) or abort.

## Reproducing

```bash
LOFT_VERIFY_STACK=1 loft --interpret <program>
LOFT_VERIFY_STACK=1 LOFT_VERIFY_STACK_TRACE=1 loft --interpret <program>   # which handles
```

## What it costs when it is off

Nothing beyond phases 1-2's `+0.9 %`: measured `+1.4 %` against the tree before the shadow
(`48d1229f`) on the same 4 M-iteration field-write loop, and the difference is a store field
and code layout rather than work — the loop relocates no record, so the log is never written
and the scan never runs.  The relocation log is guarded on a shadow having actually been
ARMED, not merely on the variable being set, so a `--native` run neither accumulates moves
nobody drains nor closes with a reassuring "no … reads": it says the shadow is
interpreter-only, beside the profiler's announcement of the same limit (loft#865).
