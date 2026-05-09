<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan closure checklist

Apply this when a plan ships and moves from `current` /
`future/` to `finished/`.

The closure rule (see [`README.md` § Closing a plan](README.md#closing-a-plan--documentation-must-move-out)):
**reference content moves OUT of the plan into its proper
home; the finished plan keeps only the closure record.**

**Not closing — deferring instead?**  See
[`_DEFER_CHECKLIST.md`](_DEFER_CHECKLIST.md).  Deferral applies
when remaining work has a concrete trigger but won't ship in
this arc; the shipped portion still follows the extraction
steps below for its reference content, but the directory moves
to `deferred/` rather than `finished/`.

## The 6 steps

### Step 1 — Identify reference vs closure-record sections

Read the plan README.  Tag each section as one of:

- **REFERENCE** ("how things work today after this plan
  ships") → moves out
- **CLOSURE RECORD** ("what was done, when, with what commits,
  what was retracted, why") → stays
- **HISTORICAL ARCHAEOLOGY** ("what was tried that didn't
  ship") → stays in the finished plan as historical record
  (don't move into reference; reference is for current
  truth)

### Step 2 — Pick the reference home

Two shapes:

**(a) CREATE-AND-MOVE** — no comprehensive reference doc
exists.  Create `doc/claude/<NAME>.md` (or
`lib/<name>/README.md` for library-scoped) with the extracted
content.  Canonical example: `31-html-export` →
`doc/claude/HTML_EXPORT.md` (created at extraction time).

**(b) TRIM-ONLY** — reference doc already exists and
covers the SHIPPED state comprehensively.  No content
movement needed; just trim the plan README and update
incoming links.  Canonical example:
`04-slot-assignment-redesign` → `SLOTS.md` was already
comprehensive.

If unsure which shape: read the candidate reference doc
and check if it covers the SHIPPED state.  If yes, TRIM-ONLY.
If no, CREATE-AND-MOVE.

### Step 3 — Trim the plan README

The trimmed README leads with:

```markdown
**Reference for the SHIPPED <feature> moved to
[`doc/claude/<NAME>.md`](../../../<NAME>.md).**  This file
is a closure record only.

## Status — YYYY-MM-DD: closed

[Brief outcome summary + commit chain]

## What did land

[Phase / commit table]

## What did NOT land (if anything was retracted)

[Post-mortem of retracted items + rationale]

## What's preserved as historical record (if multi-file plan)

[Names the sub-files and what each holds]

## See also

- [`doc/claude/<NAME>.md`](../../../<NAME>.md) — shipped
  reference (where you should be reading)
- [Companion plans, CHANGELOG, etc.]
```

Aim for 50-150 lines.  Closure records longer than 150 lines
usually still have reference content embedded.

### Step 4 — Grep for incoming links + rewrite

```bash
grep -rn "plans/<NN>-<slug>\|plans/finished/<NN>-<slug>\|plans/future/<NN>-<slug>" \
  CLAUDE.md doc/claude/ --include="*.md"
```

For each hit:
- If the link cites the plan for **design / how-it-works
  reference** → rewrite to point at the new reference home
  (`doc/claude/<NAME>.md`)
- If the link cites the plan for **closure-record reasons**
  (commit chain, post-mortem, archaeology) → keep pointing
  at the finished plan

The rule: docs link to reference; closure record is for
"what happened on this plan", not "how things work".

### Step 5 — Update CLAUDE.md doc index

If the closed plan had a CLAUDE.md doc index entry, update
it:

- **CREATE-AND-MOVE shape:** new entry for the reference
  doc; closed plan cited as secondary ("commits + build
  sequence").
- **TRIM-ONLY shape:** existing reference doc entry stays;
  closed plan entry can be removed (it's closure record;
  doesn't need top-level visibility).

### Step 6 — Update plans/README.md or lib_plans/README.md

Move the plan's row from the appropriate Future / Current
table to the Finished table.  Compress the Notes column to:

```markdown
| `finished/<NN>-<slug>/` | Closure record only.  Reference at [`<NAME>.md`](../<NAME>.md). | YYYY-MM-DD; closure-rule extraction YYYY-MM-DD. |
```

Long descriptions move to the reference doc + the closure
record.  The README table is an index; row text should be
≤ 2 lines for the average reader.

## Common pitfalls

1. **Skipping Step 4 (link rewrite).**  Most-likely cause of
   future drift.  The grep must run on `CLAUDE.md doc/claude/`
   AT MINIMUM; consider also `--include="*.rs"` for code
   comments that cite plans.

2. **Mistaking "what was tried" for reference content.**
   Retracted designs (V2 in plan-04, alternative approaches
   in WEB_SERVICES) are CLOSURE RECORD, not reference.
   Move them to the historical-record section in the trimmed
   README, not into the reference doc.

3. **Over-trimming.**  Closure record IS valuable: future
   contributors investigating "why was X tried and rejected"
   need the post-mortem.  Don't strip the "Why X retraction"
   section just because the plan is closed.

4. **Under-trimming.**  Plan README still feels like reference
   doc.  If you can't tell whether someone learning about the
   feature today should read the plan or the reference, trim
   more aggressively from the plan side.

5. **Forgetting CHANGELOG.md.**  User-facing release notes
   should mention the plan's outcome.  CHANGELOG_TECHNICAL.md
   gets the contributor-level detail.  These complement the
   closure record in finished/.

## Verification

After applying:

```bash
# 1. No incoming links left to the closed plan as a reference target
grep -rn "plans/finished/<NN>-<slug>\|plans/future/<NN>-<slug>" \
  CLAUDE.md doc/claude/ --include="*.md" \
  | grep -v "^doc/claude/plans/finished/<NN>-<slug>"

# 2. Reference doc exists + has the new content
ls doc/claude/<NAME>.md

# 3. Plan README is closure-record-shaped (≤ 150 lines, leads with "moved to")
wc -l doc/claude/plans/finished/<NN>-<slug>/README.md
head -5 doc/claude/plans/finished/<NN>-<slug>/README.md
```

## Examples

Two reference shapes shipped in this codebase:

| Plan | Shape | Reference home | Plan README delta |
|---|---|---|---|
| `31-html-export` | CREATE-AND-MOVE | NEW `HTML_EXPORT.md` (270 lines) | 542 → 79 lines (-85%) |
| `04-slot-assignment-redesign` | TRIM-ONLY | `SLOTS.md` (already comprehensive) | 394 → 100 lines (-75%) |
