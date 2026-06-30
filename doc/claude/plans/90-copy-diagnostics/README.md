<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 90 — copy diagnostics: make every silent structure copy visible

Tracker: [@PLN90](https://github.com/loft-lang/plans/issues/90).
Full design + failure-path enumeration: [COPY_DIAGNOSTICS.md](../../COPY_DIAGNOSTICS.md).

## Status

**Phase 1 COMPLETE; phase 2 started (avoidable-vs-forced classification).**

**Phase 1** — the decision covers every structure-copy emission ([phase1-inventory.md](phase1-inventory.md)).
`LOFT_COPY_DUMP` is the runtime ground truth; the verdict (`use_analysis`, **route 1** —
extend its domain, since the facts live only in the post-parse pass) classifies all four
copy idioms: var-buffer · construction/field-append · return-buffer · `OpCopyRecord`. Parity
proven on the corpus; all diagnostic-only `Copy` rows (no `ElidePlan`, no codegen change).

**Phase 2** — each `Copy` row carries a 4-way `VerdictRow.class` (the warning bucket):
**Eliminated** (`Borrow`) · **Avoidable** (warn — a borrow would be sound, blocked only by
analysis weakness; the north-star worklist) · **Implicit** (SILENT — the copy is inherent to
the model: a struct/enum field or vector slot *owns* its data, e.g. `S { f: src }`, `v[i] =
e`) · **Forced** (informational — owned by circumstance: temporary source, later mutation).
`LOFT_MATERIALIZE_DUMP` shows `bucket=…` per row + a `MAT-WORKLIST avoidable=N implicit=N
forced=N` tally. On the corpus: `field_return` → AVOIDABLE (its elimination IS the @PLN85 P4
borrow-correctly fix); **construction / `OpCopyRecord` → implicit (NOT warned — it is what
the programmer asked for)**; `assign_field` → eliminated. **North-star primary:** the warning
serves elimination — drain bucket 2 (avoidable) into bucket 1 (auto-elide), stay quiet on
bucket 3, never copy "just because" ([COPY_DIAGNOSTICS.md § North-star](../../COPY_DIAGNOSTICS.md)).
Suite byte-identical (issues 746, use_analysis 16). NEXT: emit the user-facing lint off the
bucket (warn only Avoidable; located; opt-in), and/or start draining bucket 2 (the P4 fix).

**North-star (do not lose it):** the warning is the *instrument*, not the goal — the goal is
the compiler **automatically not copying** when it can prove a borrow is safe (we never copy
"just because"). loft already has the engine: a `Borrow` verdict → `ElidePlan` → the borrow
rewrite elides the copy (var-buffer ships today). Every copy sorts into three buckets:
**(1) auto-eliminated** (no warning — grow this), **(2) avoidable** (warned — and this set
IS the worklist for bucket 1), **(3) forced** (warned, informational). See
[COPY_DIAGNOSTICS.md § North-star](../../COPY_DIAGNOSTICS.md). Notably, **field-return is
bucket 2** — eliminating it is fixing @PLN85 P4 (borrow-correctly), which closes the loop
back to that work.

NEXT — **phase 2**: classify each Copy row avoidable vs forced (bucket 2 vs 3 — the
elimination worklist), then emit the user-facing lint off it (with the `&` / restructure
hint, opt-in first). Coarse var/source attribution in some rows is to be sharpened there.

Wanted **before @PLN85 closes**: we often miss that a copy is happening, and that blind
spot is shaping what we build (the @PLN85 owned-copy match-return synthesis manufactures
copies on a near-universal pattern). Surfacing copies first changes those decisions.

## Goal

Make **every deep copy of a heap structure visible** — never silent. loft aliases heap
values by default; a deep copy of a record / vector-of-records is the fallback the
compiler emits when it cannot prove an alias is safe, and it is invisible today. Surface
it, with its reason, classified avoidable vs forced.

**Invariant:** every emitted structure copy is decided at one place (the copy-vs-borrow
verdict) and surfaced there with its reason — so a copy is never silent and a warning
never fires without an actual copy.

It is a **perf/behaviour lint, not a borrow checker** — never an error, never an
annotation, lowering unchanged; compatible with the decided "internal, invisible
ownership" model ([OWNERSHIP_MODEL.md § Internal and invisible](../../OWNERSHIP_MODEL.md)).
Three reasons to warn on *every* structure copy regardless of static size: the cost is
runtime-sized and unbounded ("hundreds of MB just to be sure"); conservative copies are
invisible and often avoidable; a copy silently changes behaviour (independent value breaks
the alias-default). It also doubles as **our** worklist — each avoidable copy is a
currently-invisible borrow-analysis gap we might still fix.

## Phases

1. **Coverage (load-bearing first).** Make the copy-vs-borrow decision the *sole arbiter
   consulted by every structure-copy emission*. Today `use_analysis::Verdict` (with a
   `reason`) decides only vector-copy bindings, while `OpCopyRecord` / copying
   `OpAppendVector` are emitted at ~20 scattered sites (struct construction, pass-by-value,
   assignment) that bypass it. Extend the verdict's domain, or route all emission through
   one `emit_structure_copy` chokepoint that consults it. *A warning is only as complete as
   the decision — so this is the first deliverable, not the diagnostic.*
2. **Diagnostic.** Emit the lint off the decision: avoidable vs forced, with the verdict's
   reason and an existing-lever hint (`&` / restructure). Opt-in lint first; promote to a
   default warning once borrows are the norm.
3. **Explicit copy-intent.** The surface form to opt into an independent copy and silence
   the lint — the inverse of `&`.

## Cross-arc dependencies

- **@PLN85 (ownership / over-free).** Soundness rides on the `deps` / `use_analysis`
  verdict. Prerequisite is **borrow-correctly, not the owned-copy synthesis**: the borrowed
  match-return should compile as a true borrow (no return-buffer ABI), so borrows are the
  default and a copy is the rare, warned exception. The @PLN85 `jo_copy_borrowed_arm_yield`
  synthesis is the *wrong* tool here (it manufactures the very copies this plan warns on);
  keep it only as the fallback for the genuine forced case (a temporary subject that cannot
  outlive the return).

## Open questions

- Coverage-first vs scoped-warning-first (recommendation: coverage first; a partial warning
  that looks complete is worse than none).
- Default severity: opt-in lint vs default warning.
- Explicit-copy surface syntax (phase 3).

## See also

- [COPY_DIAGNOSTICS.md](../../COPY_DIAGNOSTICS.md) — the design.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the decision substrate + the
  "internal, invisible, no borrow checker" position.
- `src/use_analysis.rs` — the copy-vs-borrow `Verdict` + `reason` this hangs off.
