<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Defer checklist — moving a plan to `deferred/`

Use this when a plan can't reach completion in the current arc but
the remaining work has a **concrete trigger** (not just "someday").
Sibling of [`_CLOSURE_CHECKLIST.md`](_CLOSURE_CHECKLIST.md) (for
fully shipped plans) and the [Authoring a new plan](README.md#authoring-a-new-plan)
section.

If a plan has no remaining triggers (everything is "won't do
without a driver"), it goes to [`../DESIGN_DECISIONS.md`](../DESIGN_DECISIONS.md),
not `deferred/`.  Deferral is for paused-with-intent, not for
abandoned.

## When to defer vs close vs leave-in-future

| Situation | Path |
|---|---|
| All phases shipped | [`_CLOSURE_CHECKLIST.md`](_CLOSURE_CHECKLIST.md) — move to `finished/` |
| Some phases shipped, others paused with trigger | **Partial defer** — this checklist (the README's Status table grows a SHIPPED / DEFERRED row split; canonical shape: plan-28, plan-12) |
| No phases shipped, all paused with trigger | **Full defer** — this checklist (move whole plan to `deferred/`) |
| Some phases shipped, others abandoned | Close the shipped parts via `_CLOSURE_CHECKLIST.md`; remaining design moves to `DESIGN_DECISIONS.md` |
| All phases waiting on a date (not a signal) | Stay in `future/`; nothing to defer.  ROADMAP carries it. |
| All phases waiting on user appetite (no concrete signal at all) | Move design to `DESIGN_DECISIONS.md`; remove plan dir |

The litmus test for "concrete trigger": can a future contributor,
six months from now, clearly identify the moment when this plan
should re-activate?  "When users complain about X" is concrete.
"When we feel like it" is not.

## Procedure (5 steps)

### Step 1 — Identify the boundary

For partial defers (the common case), separate:

- **Shipped phases**: which commits, what's in production today,
  where reference content lives now (or needs to move to).
- **Deferred phases**: which design content stays in the plan
  README (because it's needed when the trigger fires), what the
  trigger is.

Build a Status table at the top of the plan README with one row
per phase / tier / sub-arc and an explicit state column
(SHIPPED / DEFERRED / RETIRED).  Plan-28 and plan-12 are the
canonical shapes.

### Step 2 — Extract reference content for shipped parts

Apply the closure rule for the shipped portion only:

- **CREATE-AND-MOVE**: shipped phase introduced reference content
  that no `doc/claude/<NAME>.md` covers yet.  Create the doc
  section, move the content, leave a closure note in the plan
  README pointing at the new home.
- **TRIM-ONLY**: reference doc already covers the shipped state.
  Trim the plan README's design content for that phase to a
  closure note + cross-link.

Either way, the plan README ends up with:
- Status table at top showing SHIPPED / DEFERRED / RETIRED.
- Compressed closure narrative for shipped phases (with
  cross-links to reference homes).
- Full design content for deferred phases (kept; needed when
  trigger fires).

Length budget after defer: usually similar to before — content
trades places, doesn't shrink.

### Step 3 — Add a trigger row to `DEFERRED.md`

[`DEFERRED.md`](DEFERRED.md) is the trigger index.  Add a one-
line row:

```markdown
| [`deferred/<NN>-<slug>/`](deferred/<NN>-<slug>/) | One-line summary of what stays deferred | The concrete signal that re-activates |
```

The row points at the plan's README for full detail; the index
just makes the trigger discoverable via `grep -r "Trigger"
doc/claude/`.

**No row without a trigger.**  If you can't write a concrete
trigger, this isn't a defer — see the table above.

### Step 4 — Reclassify ROADMAP rows

For each ROADMAP row touched by the plan:

- **Shipped parts** — remove from ROADMAP (closure lives in
  CHANGELOG + git history per the maintenance rule).
- **Deferred parts that are roadmap-tracked** — leave on ROADMAP;
  update the plan path to point at `plans/deferred/<NN>-<slug>/`.
- **Deferred parts that are trigger-only** (no ROADMAP row
  expected until the trigger fires) — remove from ROADMAP.  The
  DEFERRED.md row is enough.

For the "Plans index by category" subsection at the bottom of
ROADMAP.md, move the plan from its category section to
acknowledgment that it's deferred (or remove it entirely if
trigger-only).

### Step 5 — `git mv` the plan directory

```bash
git mv doc/claude/plans/<NN>-<slug>           doc/claude/plans/deferred/<NN>-<slug>
# OR (from future/)
git mv doc/claude/plans/future/<NN>-<slug>    doc/claude/plans/deferred/<NN>-<slug>
```

Then update plans/README.md tables:

- Remove from "Current initiatives" or "Future initiatives"
  table.
- Add to "Deferred initiatives" table (with trigger summary +
  cross-link to DEFERRED.md row).

For lib_plans, the same pattern applies — `lib_plans/deferred/`
+ `lib_plans/DEFERRED.md` + `lib_plans/README.md` tables.

### Step 6 — Grep + rewrite incoming links

Same as the closure-checklist's most-skipped step:

```bash
grep -rn "plans/<NN>-<slug>\|plans/future/<NN>-<slug>" \
  CLAUDE.md doc/claude/ --include="*.md"
```

Rewrite each match to `plans/deferred/<NN>-<slug>/` (or to the
new reference home if the link was for shipped content that
moved to `doc/claude/<NAME>.md`).

## Lifecycle after defer

The plan stays in `deferred/` while ANY remaining phase has a
concrete trigger.

**When the trigger fires:**
1. Move the plan from `plans/deferred/` back to `plans/future/`
   (or directly into a current arc).
2. Remove the row from DEFERRED.md.
3. Add ROADMAP rows for the now-active work.
4. Update plans/README.md tables.
5. Update incoming links per Step 6 above.

**When a deferred plan ships its remaining phases:**
1. Apply [`_CLOSURE_CHECKLIST.md`](_CLOSURE_CHECKLIST.md) — move
   to `finished/`.

**When a deferred plan's remaining phases are reclassified as
won't-do:**
1. Move the design to `DESIGN_DECISIONS.md` with rationale.
2. Remove the plan directory.
3. Remove the DEFERRED.md row.

## Pitfalls

1. **"Will get to it later" is not a trigger.**  If the trigger
   is "when contributor appetite arrives" without a concrete
   signal (a bug pattern, a user request shape, a count
   threshold), the work belongs in DESIGN_DECISIONS.md, not
   deferred/.
2. **Partial defers need a clear boundary.**  Don't leave a plan
   with "Phase 3 is sort of done but not really documented."
   Either close Phase 3 properly (extract its reference content)
   or roll it back (drop it from "shipped" status).  Half-done
   phases corrupt the Status table.
3. **Don't forget Step 4 (ROADMAP reclassification).**  The
   shipped parts staying on ROADMAP is the most common drift; a
   future audit then thinks they're still open work.
4. **Cross-link rot is the second-most-common drift.**  Step 6
   catches it but only if you actually run the grep.

## Cross-links

- [`_CLOSURE_CHECKLIST.md`](_CLOSURE_CHECKLIST.md) — for fully
  shipped plans (move to `finished/`).
- [`_TEMPLATE.md`](_TEMPLATE.md) — for new plans.
- [`README.md § Three workflows`](README.md#three-workflows-for-todo-items--pick-the-lightest-that-fits)
  — when to use a plan vs the lighter `## Open work` flow.
- [`README.md § Light flow lifecycle`](README.md#light-flow-lifecycle---open-work-in-reference-docs)
  — the lighter analogue: deferring a row in a reference-doc
  `## Open work` section is just an inline annotation, no
  directory move.
- [`DEFERRED.md`](DEFERRED.md) — the trigger index this checklist
  feeds into.
