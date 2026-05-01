# Phase 02a — Introspection: after scope-walk

**Status:** OPEN

**Kind:** Review (no code)

**Trigger:** Phase 02 marked DONE.

**Time budget:** 1 day max.

## Why this phase exists

Phase 02 is the load-bearing change — it replaces the
dep-tracking-driven cleanup emission with a mechanical scope-walk
and closes P203.  Before continuing to phase 03 (Drop safety net),
verify:

- P203 actually closed.
- Suppression list is sustainable.
- Dep-tracking simplification was as advertised.

## Questions to answer

### Did P203 close?
1. `repro_p203.loft` exits 0 under native?
2. Does the file actually exist on disk after the block close?
3. Are there file-related tests that *now* pass that didn't before?

### Suppression list health
4. How many entries in `SUPPRESSION_LIST`?  (Predicted from phase
   00 survey vs actual.)
5. Are all entries documented with a "why suppressed" comment?
6. Are any entries fragile (e.g., key-by-name when var names
   change)?  Flag for follow-up.

### Dep-tracking simplification
7. How much dep-tracking code retired in step 2.5?  (Predicted
   from phase 00 survey vs actual LOC.)
8. Are remaining dep-tracking consumers (aliasing / closure /
   parallel) cleanly separated, or did they leak into the cleanup
   path during phase 02?

### P204 cleanup-side
9. Did phase 02's scope-walk also close the cleanup-side of P204?
   (The Call-resolution side stays open.)
10. If yes — does P204's reproducer exit 0 under native?
11. If no — what's the residual gap?

### Performance
12. Did the extra OpFreeRef calls (now firing on every local) cause
    measurable runtime slowdown?  (The runtime no-op should make
    them ~ns each.)

```bash
cargo test --release --test wrap 2>&1 | tail -5
# Compare timing before/after phase 02 if possible.
```

## Output

Update these files based on findings:

- **`03-drop-safety-net.md`**: if phase 02 closed P203 cleanly,
  phase 03 is genuinely belt-and-suspenders (low priority).  If
  phase 02 has any residual gap, phase 03 becomes a fallback for
  the gap.
- **`README.md`**: update status table; if dep-tracking simplifica-
  tion came out smaller than expected, flag whether further
  retirements are tractable.
- **PROBLEMS.md (P204)**: if cleanup-side of P204 closed, update
  the entry to reflect partial closure.

## Decision criteria

| Finding | Action |
|---|---|
| P203 closed cleanly + suppression list ≤ 5 entries + dep-tracking simplified as expected | Continue to phase 03 (or skip if P203 is unambiguously closed and Drop safety net is overkill). |
| P203 closed but suppression list grew large (>10 entries) | Investigate — the scope-walk may be too coarse; consider a more selective approach (e.g., type-keyed walk). |
| P203 closed but dep-tracking simplification didn't materialise | Survey was wrong; revisit phase 00 findings. |
| P203 didn't close | Stop and diagnose.  The plan's central assumption is wrong; rethink before continuing. |
| Phase 02 took >2× budget | Re-budget phase 03; consider deferring it. |

## Memory entries to save

Examples:

- "Mechanical scope-walk for resource cleanup (instead of
  dep-tracking-driven emission): worked / didn't.  Apply pattern
  when fix attempts on a precise-emission system have repeatedly
  cascaded."
- "Suppression-list approach for codegen exceptions: keyed by N
  attribute / pattern.  Sustainable / fragile."
- "OpFreeRef no-op fast-path: extra calls cost ~Nns each in
  practice.  Acceptable / regrettable."

## Findings

_(populate after phase 02 lands)_
