<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 04 — Case D: aliased mutating (rejected)

**Status: open**

## Goal

Implement Case D per [README § Case D](README.md#case-d--aliased-mutating-rejected)
and [DISCUSSION § Phase 3](DISCUSSION.md#phase-3--diagnostic-emission).

Case D is the genuine aliased-state case: closure mutates a
capture, escapes its construction scope, AND the outer scope
still reads or writes the capture after the closure's
construction site.  The two reads see inconsistent state at
mismatched lifetimes; the language rejects the program with a
clear diagnostic naming the four positions involved.

## What ships

When phase 03's liveness check returns YES for any mutated
capture, classify as D and emit a diagnostic.  No lowering;
parsing fails.

The diagnostic must name four positions per
[DISCUSSION § Phase 3](DISCUSSION.md#phase-3--diagnostic-emission):

1. The closure's body site (where mutation was written).
2. The captured binding's defining site.
3. The post-construction outer use that triggered the
   case-D classification.
4. The closure's destination site (where it escapes).

Two emission paths, ship the cheapest first:

### Path (a): inline-position fallback (ship first)

A single `diagnostic!` call with all four positions inlined into
the message body.  Matches existing P213 / P215 / P227
diagnostic shape; no DiagEntry change.  Example:

```
error: closure captures `count` (defined at line 3:5) by-value
       but body mutates it (line 5:9), the closure escapes via
       return (line 7:5), AND outer scope reads `count` after
       construction (line 6:23).  Aliased state across
       mismatched lifetimes is not allowed.  Either:
         - move the outer use BEFORE the closure construction
         - or wrap `count` in `Mutable<T>` for explicit
           shared ownership (see lib/mutable; not yet shipped)
  --> snippet.loft:5:9
```

### Path (b): rustc-style multi-caret diagnostic (deferred)

Extends `DiagEntry` in `src/diagnostics.rs` with optional
secondary positions (~75 LOC additive change).  Renders four
separate carets with their own arrows.  Better UX but larger
infrastructure change.

Phase 04 ships path (a); path (b) is a separate follow-up if
the inline format proves hard to scan.

## Test surface

`tests/parse_errors.rs` (case-D rejection cells follow the
same pattern as P257's parse-time rejection):

```
d_int_capture_with_outer_read_after    // problematic() — log_info(count) after closure ctor
d_text_append_with_outer_use           // s = ""; cl = fn() { s += "x" }; print(s); cl
d_struct_field_mutated_then_used       // p.x mutated in closure, then read in outer
d_post_construction_assign             // closure mutates count; outer assigns count after
d_outer_use_in_branch_after_ctor       // if cond { read(count) }; closure
                                       //   — the if-branch counts as a post-ctor use
```

Each cell uses `code!(...).error("…closure captures `<name>`…")`
asserting the inline-position diagnostic exactly.  Pin the four-
position format so Path (b)'s future migration is a deliberate
diagnostic-shape change (not a silent breakage).

Plus a "rejection-then-fix" pair for one cell:

```
d_problematic_pattern_rejected         // case D rejects
d_problematic_pattern_fixed_by_reorder // moving the outer read BEFORE closure ctor → case B passes
```

## Critical files

| File | Change |
|---|---|
| `src/parser/closure_analysis.rs` | Branch on case=D in classifier; call `emit_case_d_diagnostic` instead of synthesising the closure |
| `src/parser/vectors.rs::synthesize_closure_record` | Skip synthesis when case=D (parser has already errored) |
| `src/diagnostics.rs` | OPTIONAL — Path (b) secondary-position support |
| `tests/parse_errors.rs` | 7 case-D cells |

## Verification

- All 7 d_* cells emit the expected error string exactly.
- The "fixed by reorder" cell compiles + runs (case B classification).
- No false-positive Case D on a Case B body (regression net via
  phase 02's b_* cells still green).
- No false-positive Case D on a Case C body (phase 03's c_* cells).
- `make ci` green.
- The error message names all four positions; future contributor
  can reorder code based on the message alone.

## Risks

| Risk | Mitigation |
|---|---|
| Diagnostic text is locked in by the cells; future changes need cell updates | This is INTENTIONAL — the cells are the contract.  When the diagnostic format changes (e.g., Path (b) ships), update the cells in the same commit. |
| False-positive Case D rejects valid Case B/C code | Phase 06 retrofit catches false-positives at the application layer (TTT v6, plan-36).  Phase 04 cells include the "fixed by reorder" cell as a specific guard against the dominant false-positive shape (where the analysis treats a not-actually-aliasing read as aliasing). |
| Case-D diagnostic doesn't suggest `Mutable<T>` workaround until phase 05 ships | Phase 04 inline-position message says "wrap in `Mutable<T>` for explicit shared ownership (see lib/mutable; not yet shipped)" — the suggestion is conditional on phase 05.  When phase 05 ships, update the message to drop the "not yet shipped" qualifier. |
| Path (a) inline format is unreadable for nested cases | Phase 04 ships path (a) as the regression contract; the cells lock in the format.  If the format proves bad in plan-36 retrofit (phase 06), file Path (b) as a follow-up.  Cells make the diagnostic-shape upgrade traceable. |

## Cross-references

- [README § Case D](README.md#case-d--aliased-mutating-rejected)
- [README § Diagnostic shape](README.md#diagnostic-shape)
- [DISCUSSION § Snippet 4](DISCUSSION.md) — paper-trace of case D rejection.
- `src/diagnostics.rs::DiagEntry` — extension point for Path (b).
- [P257 fix](../../PROBLEMS.md#257) — same parse-time-rejection pattern (vector capture into closure).
