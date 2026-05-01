# Phase 03a — Retrospective

**Status:** OPEN

**Kind:** Retrospective (no code; produces durable memory entries)

**Trigger:** Last phase among 02 / 03 lands.  May fire after just
phase 02 if 02a decided to skip 03.

**Time budget:** 1 day max.

## Why this phase exists

Plan 10 was a structural bet: replace precise dep-tracking-driven
cleanup with mechanical scope-walk + runtime no-op + Drop safety
net.  The retrospective captures whether the bet paid off, in
durable form, so future cleanup-handling design benefits.

## Questions to answer

### The bet
1. Did the scope-walk actually close P203 cleanly?
2. Did dep-tracking simplification materialise as predicted in
   phase 00 survey?
3. Did suppression list stay manageable, or grow unwieldy?

### Performance
4. Did the extra OpFreeRef calls cause measurable slowdown?
5. If yes — was the slowdown worth the correctness gain?

### Defence in depth
6. Did phase 03 (Drop safety net) catch anything in practice, or
   was it pure theatre?
7. If something landed before phase 03 that exposed a missing
   scope-walk case — Drop saved it?

### Spillover
8. Did plan 10 close any side-effect bugs (file-related tests
   that quietly worked despite the regression)?
9. Did plan 10 close part of P204?  Document the residual gap.

### Lessons
10. What pattern worked here that future cleanup designs should
    repeat?
11. What didn't work that should be avoided?
12. Was phase 00 survey worth the day budget, or could it have
    been compressed?

## Output

### Memory entries (primary)

Write 3-5 durable memory entries.  Examples:

- **Cleanup-via-scope-walk pattern (project memory)**: "Plan 10
  replaced precise dep-tracking-driven OpFreeRef emission with a
  mechanical scope-walk over locals at every block close.  Worked /
  didn't.  Apply when cleanup correctness has resisted multiple
  precise fixes."

- **Runtime no-op fast-path (feedback memory)**: "OpFreeRef gained
  early-return for already-freed slots.  Cost: ~Nns per extra call.
  Made the scope-walk safe by absorbing extra calls.  Pattern:
  always pair conservative emission with permissive runtime."

- **Suppression-list approach (feedback memory)**: "Plan 10 used a
  hardcoded SUPPRESSION_LIST for owned-by-callee cases.  After
  N entries, was sustainable / fragile.  Future emission rewrites
  should consider [whatever turned out better]."

- **Drop safety net (feedback memory)**: "FileHandle::Drop catches
  emission gaps after the fact.  Worth the refactor / overkill.
  Apply for OS-resource types; not needed for pure heap refs."

- **Plan 10 + plan 09 interaction (project memory)**: "Plans 10
  (scope-exit) and 09 (per-Op emitter) ran in [some] order.
  Order [mattered/didn't] because [reason].  Document in next
  multi-plan effort."

### Updates to other plans / docs

- **PROBLEMS.md**: confirm P203 closure; update P204 with residual-
  gap status.
- **LIFETIME.md**: final contract documentation.
- **CHANGELOG_TECHNICAL.md**: summary entry for plan 10.

### If plan ended early

If phase 02a decided to skip phase 03, document the reason and
whether to revisit Drop safety net later.

## Decision criteria

This phase doesn't decide anything for plan 10 itself — that's
done.  It decides:

| Finding | Action for future |
|---|---|
| Scope-walk pattern broadly successful | Apply in future cleanup-emission rewrites; reference plan 10 as template. |
| Pattern worked for files but cascaded for other ref types | Document the boundary; future plans pick the pattern selectively. |
| Pattern didn't work or caused new bugs | Save lessons; future cleanup designs avoid this approach. |

## Findings

_(populate at end of plan 10)_

## Memory entries written

_(list of file names / titles created in this retrospective)_
