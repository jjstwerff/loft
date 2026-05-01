# Phase 00a — Introspection: after scaffold

**Status:** OPEN

**Kind:** Review (no code; updates downstream plan files based on
what actually happened in phase 00)

**Trigger:** Phase 00 marked DONE.

**Time budget:** 1 day max.

## Why this phase exists

Phase 00 is the riskiest infrastructure step.  Hoisting every
Op-emission call site through `emit_op` while keeping byte-identical
output is genuinely hard.  Before committing to phases 01-04, we
need to know:

- How many iterations did each hoist step actually take?
- What `EmitCtx` helpers ended up necessary that weren't planned?
- Which call sites resisted the hoist and needed special treatment?
- Is the byte-identical-via-goldens approach holding up, or are we
  fighting the goldens more than the work?

Wrong answers here propagate downstream — phase 02 inherits all the
EmitCtx limitations and all the hoist patterns.

## Questions to answer

### Effort vs estimate
1. How many commits did phase 00 actually need vs the planned 7-9?
2. Which step needed the most iterations?  Why?
3. Time elapsed: budget was implicitly ~3-5 days; actual?

### Surface area surprises
4. List every helper added to `EmitCtx` during phase 00.  For each,
   one-line note: planned in advance / surfaced during step N.
5. Were any direct Op emissions in `dispatch.rs` impossible to
   route through `emit_op` and left as legacy direct calls?  List
   them and the reason.
6. Did the fn-ref dispatch arms in `emit.rs` need a special
   trait-impl shape that the plain `OpEmitter` doesn't cover?

### Golden corpus stability
7. How often did a step that was supposed to be byte-identical
   produce a diff that took non-trivial debugging to fix?
8. Did any test outside the corpus regress without the corpus
   catching it?  If yes — corpus is too narrow; flag for expansion.

### Hidden assumptions surfaced
9. Anything the plan assumed about the codebase that turned out
   different?

## Output

Update these files based on findings:

- **`02-param-adapter.md`**: if `EmitCtx::value_type(value) -> Type`
  helper turned out to require non-trivial plumbing, document
  it in step 2.1.
- **`03-parallel-emitter.md`**: if any of the planned helpers
  (`worker_fn`, `closure_shape`) need data from `EmitCtx` that
  isn't trivially accessible, update step 3.2's helper sketches.
- **`05-file.md`** through **`08-binary.md`**: if any custom
  emitter sketch references an `EmitCtx` helper that turned out
  hard, flag in the relevant phase.

If phase 00 went substantially over budget (>2× planned commits or
>2× planned days), add a top-level **risk update** to README.md and
recompute the phase budgets.

## Decision criteria

| Finding | Action |
|---|---|
| Phase 00 landed clean (≤1.3× budget, no surprise plumbing) | Continue to phase 01 as planned. |
| Phase 00 landed with 2-3 surprises that updated downstream phases | Continue to phase 01 with updated plans. |
| Phase 00 took >2× budget OR a hoist step proved structurally unsolvable | **Stop and revise.**  The simplification phases assume `emit_op` is the universal dispatch.  If that's not achievable, the whole plan needs rethink. |
| Goldens proved unmaintainable (frequent spurious diffs) | Replace golden corpus with a smaller invariant-check approach before phase 02. |

## Memory entries to save

If introspection surfaces a pattern that should persist beyond this
plan, save as a feedback or project memory.  Examples:

- "Plan 09 phase 00: hoisting `dispatch.rs` direct Op emissions
  through `emit_op` required N idiosyncratic emitters because of
  arg-shape variation.  Future codegen plans should expect similar
  surface-area divergence."
- "Plan 09 phase 00: golden corpus of M files proved sufficient /
  insufficient — adjust corpus selection rule for next plan."

## Output format

Append to this file under "Findings" — keep findings concise (1-2
sentences each, link to the actual diff if possible).

## Findings

_(populate after phase 00 lands)_
