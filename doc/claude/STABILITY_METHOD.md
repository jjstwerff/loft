<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STABILITY_METHOD.md — find dual invariants, move algorithms to their data, then de-duplicate

The working method for turning a grown codebase into a stable one.  It is
[GOALS.md](GOALS.md) Goal E ("one home per fact", robustness by subtraction)
expressed as a **procedure with three separated passes**.  The separation is
the point: each pass produces a complete artifact before the next starts, so
the survey is never invalidated by the repairs, and the repairs are never
improvised without the survey.

The method exists because of a measured failure class.  Six bugs fixed on
2026-06-10/11 (#313, #314, #316, #318, #322, #323, #328) shared one anatomy:
**a single fact was implemented in two or more places, and the places
disagreed** — a flag and a layout answering "is this field split?"
differently per parse order (#313); five sites each deciding "who frees this
store" (#316/#323); a cache manifest claiming to cover inputs the parser
loaded behind its back (#322); the parse erasing pointer-ness that the docs
and the layout still asserted (#328).  None of these were typos or logic
slips — each was a *structural* defect: the invariant had no single home, so
the copies drifted.

## Pass 1 — the sweep (find and document; do not fix)

Walk the whole body of code with one question: **which facts are asserted in
more than one way?**  The tell-tale shapes:

- a **flag and a derived structure** that answer the same question (a
  `*_d_nr` marker vs the registered layout);
- a **parse-time decision re-derived at codegen time** (or at cache-load
  time, or on the native backend);
- **two encodings for one value** (a null sentinel and a zero default);
- **one field carrying several meanings** (a deps list that is liveness here,
  ownership there, a type marker elsewhere);
- a **document asserting semantics the code does not implement** (the spec is
  a home too).

For every find, write a catalog entry (the live catalog:
[STABILITY_SWEEP.md](STABILITY_SWEEP.md)) containing four things:

1. **The invariant, in one sentence** — the fact itself, stated so a reader
   can check any site against it ("a struct field's layout is whatever
   `fill_database` registered — nothing else may answer layout questions").
2. **Every home** — each place the code (or a doc) asserts, caches, or
   re-derives the fact today, with `file:line`.
3. **The natural home** — which *data structure* the invariant belongs to.
   This names where the algorithm will eventually live (pass 2), and is the
   one judgment call in the entry: the home is the structure whose lifetime
   and mutation already match the fact's (layout facts live with the layout;
   ownership facts live with the store allocator; encoding facts live with
   the type that is encoded).
4. **The probe and its verdict** — a minimal program that makes the homes
   disagree, run on both backends.  A probe that breaks becomes a GitHub
   issue plus an `#[ignore = "stability-sweep: #NNN"]` test; a probe that
   holds is recorded as "probed, held" — coverage is a result too.

**No fixing during the sweep.**  A mid-sweep fix re-shuffles the ground being
surveyed: it moves homes, invalidates recorded line numbers, and — worse —
spends the fresh diagnostic context on one instance instead of the class.
The discipline mirrors the matrix-first debugging rule: the urge to fix is
the signal the survey is not finished.

## Between the passes — fix the known bugs first

**Fix as many open bugs as possible BEFORE the pass-2 rewrite (user,
2026-06-11).**  Each fix sharpens the contract between the routines the
relocation will touch: a routine whose edge cases are correct documents its
own obligations, while a buggy one leaves the mover guessing which
behaviours are contract and which are accident.  The sweep's findings list
is therefore also the fixing queue — work it down (ordinary bug-fix rigor,
one issue at a time) until what remains is exactly the structural moves
pass 2 exists for.

## Pass 2 — move each algorithm to its data structure

For each catalog entry, relocate the deciding logic INTO the natural home
named by the entry: the data structure whose state the invariant describes.
After the move, every former site *asks* the home instead of *re-deriving*
the answer.

This is the structural fix, and it is different from "deduplicate the code":
two textually different sites cannot be merged while each owns part of the
decision, but both collapse trivially once a method on the right structure
answers the question.  Worked precedents:

- `Parser::fn_ref_field_is_split` (#313) — read/write shape stopped
  consulting a parse-order-mutable flag and started asking the registered
  layout: the layout is the structure whose state IS the answer.
- `free_named`'s cascade (#323) — capture lifetime moved into the store
  allocator's free path; the scope analysis now only decides *not to emit*
  a free, never *who owns*.

A move is complete when the old sites contain **no remaining copy of the
decision** — only calls.  Each move is an ordinary change with the ordinary
gates (probes from pass 1 re-run as its verification matrix; both backends).

**When to run pass 2 (user, 2026-06-11): in a quiet stretch — when there is
not much rewrite activity in flight.**  Relocations cut across the same
files feature branches touch, so running them concurrently multiplies merge
conflicts and re-introduces drift while homes are mid-move.  The catalog is
deliberately durable for this: it waits, fully specified, until a low-churn
window (typically right after a release ships, before the next dogfood wave
starts), and each entry names everything needed to execute the move cold.

## Pass 3 — remove the duplications

Only now delete: the flags nobody reads, the re-derivations that became
calls, the second encodings, the dead guards that compensated for drift
between homes.  Deletion is last because pass 2 made it *safe* — each
removal is of something demonstrably unused (the usage-sentinel test from
the engineering-rigor skill applies: route the suspect through a loud
chokepoint, run the suite, delete on silence after a positive control).

The pass-3 deliverable is negative diff with the pass-1 probes still green.
Goal E's check applies verbatim: the robust version is the shorter one.

## Why three passes and not one

Fixing during the hunt optimises locally: each fix is reasonable, but the
catalog ends up describing a tree that no longer exists, the natural homes
get chosen one-at-a-time without seeing the whole family, and duplications
get "fixed" by patching both copies (which *preserves* the dual home).  The
separation forces the three different judgments to each happen with full
information: *what exists* (pass 1), *where it belongs* (pass 2), *what can
go* (pass 3).

## Relation to the rest of the method stack

- [GOALS.md](GOALS.md) Goal E — the destination this method walks toward;
  § "Stability trumps features" governs what to do when a sweep finding is
  better closed by rejection than by consolidation (→ C74/C75 precedents).
- The **engineering-rigor skill** — supplies the per-finding instruments
  (boundary matrix, usage sentinel, falsification probes).
- [DESIGN_PROTOCOL.md](DESIGN_PROTOCOL.md) — pass 2's moves are designs;
  load-bearing ones get the protocol (name the invariant, count re-assertion
  sites — the catalog already did both — then probe to falsify).
- [STABILITY_SWEEP.md](STABILITY_SWEEP.md) — the live pass-1 catalog and
  work list.

---

## Where the method points next

[STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) (2026-06-11) applies this
method's lens to *designs* instead of routines: the eight structures the
bug history says will keep manufacturing bugs (H1 analysis-dependent
arity is the headline), each with sized mitigation work and a landing
order.  Treat it as the input queue for the next pass-2-style quiet
window.
