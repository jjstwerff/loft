<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 05 — D3 decision: tuples in struct fields

**Status: shipped 2026-05-11.  6/7 D3 cells green on both backends
(E1, E1 update, E1n, E2, E3, E5).  Decision: LIFT — already shipped
by Plan-06 phase 4d, which removed the `Type::Tuple` rejection in
`parse_field` and routes tuple-field storage through the synthetic
`__tuple<…>` struct positions.  E4 (closure-element tuple AS A
struct field) hits a separate native codegen projection bug — the
runtime `(u32, DbRef)` fn-ref tuple needs `.0` projection before
the `as i32` cast for the struct-field-write path, and P196's
projection patch didn't extend to this nested case.  Filed @P251;
e4_d3 cell parked behind it.**

## Goal

Decide — in writing, with reviewer sign-off — whether to **lift
T1.11a** (today's parser rejection of `struct S { t: (A, B) }`) or
**keep the rejection and record it as a closed-by-decision** in
DESIGN_DECISIONS.md.  No more "deferred" status for D3 cells after
this phase.

If lifted: the phase ships parser + codegen + scope cleanup for tuple
fields and re-validates every E1–E5 row in the D3 column under the
[cross-mode harness](00-matrix.md#cross-mode-harness).

If kept: the phase records the rationale (compile error stays in
place; the matrix marks every D3 cell as `CLOSED:rationale`) and the
plan closes one phase early.

## Decision (recorded 2026-05-11)

> **Decision: LIFT** — already shipped by Plan-06 phase 4d.
> Reviewer sign-off date: 2026-05-11.
>
> **Rationale:** the lift was implemented on a prior arc.  The
> rejection in `src/parser/definitions.rs::parse_field` was
> already removed; struct field types of `Type::Tuple` route
> through `parser/mod.rs::set_field_check`'s Tuple arm
> (line 2372) which emits per-element write ops at the
> synthetic `__tuple<…>` struct's element positions, and field
> READS go through `get_val`'s Tuple arm using the same offset
> table.  The phase 05 spike was therefore a verification pass:
> instantiate, read, update (whole-tuple and per-element),
> nested, with `text` / `Reference` / `integer not null` /
> mixed elements — all observed identical between interpreter
> and `--native`.
>
> The "Keep" alternative was already foreclosed by the prior
> lift; reverting it would break `tests/parse_errors.rs:797-803`
> (which documents the lifted state) and the `__tuple<…>` storage
> machinery already exercised by P189b's vector-of-tuple element
> access.
>
> **E4 (closure as a tuple-element of a struct field) corollary:**
> @P251 opens a follow-up for the native projection bug surfaced
> during this phase — the codegen path that writes a tuple
> containing a fn-ref into a struct field doesn't project the
> runtime `(u32, DbRef)` fn-ref tuple's `.0` before the `as i32`
> cast (P196 only fixed the direct fn-ref-as-struct-field path,
> not the wrapping-tuple case).  E4_d3 cell parks behind @P251.

## Feasibility spike (precedes any larger commit)

Before opening the implementation arm, run a tightly-scoped spike:

1. Remove the `Type::Tuple` rejection in `src/parser/definitions.rs::
   parse_field` (one or two lines).
2. Write a minimal test that declares `struct S { t: (integer,
   integer) }`, instantiates `s = S { t: (3, 7) }`, reads `s.t.0` and
   `s.t.1`, and asserts.
3. Compile and run.  Three possible outcomes:
   - **Compiles + runs correctly under both modes** → low-effort lift,
     opt for "Lift" decision and proceed to implementation arm.
   - **Compiles + interp passes + native diverges** → medium-effort
     lift; native codegen needs tuple-field-specific work.  Decision
     hinges on appetite.
   - **Compile error elsewhere (record-writer / record-reader / scope
     cleanup)** → high-effort lift; touches store layout
     primitives.  Decision likely "Keep" unless a real consumer needs
     it now.

The spike is one commit on a throwaway branch (or unstaged, depending
on outcome); its purpose is to inform the Decision section, not to
ship.

## Lift path — implementation outline

Only opens if the Decision section selects "Lift".

### 5a — parser

- `src/parser/definitions.rs::parse_field` accepts `Type::Tuple`.
- `parse_field_init` accepts a tuple literal initialiser.
- Constructor syntax `S { t: (a, b) }` parses.

### 5b — record layout

- The struct record's field offset for a tuple field is its inline
  byte size (sum of `element_size`).  Confirm `definitions.rs`
  field-offset arithmetic uses `Type::Tuple::element_size` (not just
  primitive-element width).
- `gen_set_field_*` for a tuple field emits per-element `OpPut*` at
  `record_offset + element_offsets[i]`.

### 5c — scope cleanup

- The struct record's owned-element walker (`get_free_vars` or
  similar) descends into tuple fields and emits `OpFreeRef` for each
  owned element (text, reference, closure-dep) inside the tuple.
- The record's `Drop` path (`OpDeleteStruct` etc.) walks tuple
  fields' owned elements before freeing the record.

### 5d — D3 cells under the harness

For each row E1, E1n, E2, E3, E4, E5: write `e<ROW>_d3_field_*`
cells.  Two cell families:

- **Read-back**: declare struct, instantiate with tuple field, read
  `s.field.i`, assert.
- **Update**: declare struct, instantiate, then `s.field = (new_a,
  new_b)`, read back, assert.

Each cell runs under cross-mode; cross-mode equivalence is mandatory.

### 5e — match on tuple field

- `match s.field { (a, b) -> ... }` works (T1.9 pattern unchanged;
  the subject is a `TupleGet` of a struct-field read).

## Keep path — closed-by-decision wording

Only opens if the Decision section selects "Keep".

DESIGN_DECISIONS.md gets an entry like:

```
## C-XX — Tuples not allowed in struct fields (T1.11a)

Decision: tuples remain prohibited as struct field types.
Rejection: `parse_field` in `src/parser/definitions.rs` continues to
emit "tuple types are not allowed as struct field types".

Rationale: <filled in based on spike outcome>.  Common rationale
candidates include:
- Storage layout: tuple inline vs DbRef-to-tuple encoding ambiguity
  in record writer.
- Lifetime tracking: owned-element walker would need tuple-field
  recursion adding complexity disproportionate to demand.
- No current consumer: plan-06 phase 9 and the rest of the matrix
  rows D1/D2 cover all use cases identified to date.

Workaround: declare a named struct for the would-be tuple field
(`struct Pair { a: integer, b: integer }`).  Equivalent storage,
better readability for non-trivial fields.

Trigger to revisit: a use case that cannot be expressed with a named
struct (none known as of 2026-05-04).
```

The matrix in [00-matrix.md](00-matrix.md) marks every D3 cell as
`CLOSED:C-XX` with a link to this entry.

## Acceptance

**Lift path:**
- Decision section filled in.
- 6 D3 cells (one per row E1–E5 plus E1n) green under cross-mode.
- TUPLES.md "non-goals" updated to remove "tuples in struct fields".
- PLANNING.md § T1.11a marked completed.
- `make ci` green.

**Keep path:**
- Decision section filled in.
- DESIGN_DECISIONS.md entry written and committed.
- Matrix updated.
- TUPLES.md "non-goals" remains; gains a cross-reference to the new
  C-XX entry.
- `make ci` green (no test changes; only docs).

## Risks

| Risk | Mitigation |
|---|---|
| Lift path balloons to large-effort work as record-layout primitives need touching | The feasibility spike (above) sets the scope before the Decision lands.  If the spike shows >3 days of work, decision likely "Keep". |
| Lift path passes interp but fails native silently | Cross-mode harness catches divergence per cell.  No cell ships green under one mode only. |
| Keep path documents a rationale that the next contributor disagrees with | DESIGN_DECISIONS.md entries are appendable — a future reversal updates the entry rather than re-arguing from scratch.  The "trigger to revisit" section names a concrete bar. |

## Out of scope

- Tuple-as-vector-element (E7) and other non-goal element types.
- Tuple-returning functions (T1.8a) for D3 — covered by @PLAN06 phase
  9a.

## Cross-references

- [PLANNING.md § T1.11a](../../../PLANNING.md)
- `src/parser/definitions.rs::parse_field`
- `tests/parse_errors.rs::tuple_in_struct_field_rejected`
- [TUPLES.md § non-goals](../../../TUPLES.md)
- [DESIGN_DECISIONS.md](../../../DESIGN_DECISIONS.md)
