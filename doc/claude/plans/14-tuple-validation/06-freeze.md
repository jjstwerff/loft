<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 06 — Matrix freeze + doc reconciliation

**Status: open**

## Goal

After phases 01–05 land, reconcile the documentation surface so the
matrix in [00-matrix.md](00-matrix.md) and [README.md](README.md) is
the single source of truth for tuple coverage.  No code change in
this phase — only doc updates and the close-out commit.

## Doc updates

| File | Change |
|---|---|
| `doc/claude/TUPLES.md` § Known limitations | Remove rows closed by phase 04 (T1.8c) and, if lifted, phase 05 (T1.11a).  Each remaining row carries a phase-or-decision pointer. |
| `doc/claude/TUPLES.md` § Non-goals | Reflect the phase-05 decision verbatim. |
| `doc/claude/TUPLES.md` § Deferred work | Drop entries closed by this plan (T1.8c, possibly T1.11a, possibly E6 folding). |
| `doc/claude/PLANNING.md` § T1.x | Mark T1.8c completed; T1.11a per phase-05 decision; E6 folded. |
| `doc/claude/DESIGN_DECISIONS.md` | New entries from phase 04 (E6 folding) and phase 05 (Keep path, if chosen). |
| `doc/claude/CHANGELOG_TECHNICAL.md` | Plan-14 close summary: cells covered, semantics decided, harness shipped. |
| `CHANGELOG.md` | One user-facing line: "Tuples cross-validated under interpreter and `--native` across {N} cells." |
| `doc/claude/plans/README.md` | Plan 14 moves from "current initiatives" to "finished initiatives".  Date stamp + commit hash. |
| `doc/claude/plans/14-tuple-validation/` | Whole subdirectory moves to `doc/claude/plans/finished/14-tuple-validation/` per the README convention. |

## Matrix verification

A small mechanical check before the close-out commit:

```bash
# Every cell in the matrix table must be either PASS:test_name,
# FIX:phase, or CLOSED:reason — no "TBD", no "open", no blank.
grep -E '^\| \*\*E[1-7]' doc/claude/plans/14-tuple-validation/00-matrix.md \
    | grep -v -E 'PASS:|FIX:|CLOSED:'
# Expect: empty output.

# Every test name in the matrix exists in the test suite.
grep -oE '[a-z][a-z0-9_]*' doc/claude/plans/14-tuple-validation/00-matrix.md \
    | grep -E '^e[0-9]_d[0-9]_' \
    | sort -u \
    | while read name; do
        grep -rqE "fn $name|cross_mode!\($name" tests/ \
            || echo "missing: $name"
      done
# Expect: empty output (no missing).
```

If either grep produces output, the close-out commit is blocked
until reconciled.

## Acceptance

- The two grep checks above produce empty output.
- TUPLES.md, PLANNING.md, DESIGN_DECISIONS.md, CHANGELOG_TECHNICAL.md
  all match the final matrix.
- Plan 14 subdirectory moved to `finished/`.
- `doc/claude/plans/README.md` table updated.
- `make ci` green.

## Out of scope

- Any new test cells.  Phase 06 is doc-only.
- Performance work (a tuple benchmark suite would be a separate
  initiative, triggered by plan-06 phase 9 if needed).
- Coroutine / iterator-protocol cells (separate plans).

## Cross-references

- [README.md](README.md) — plan goal + acceptance
- [00-matrix.md](00-matrix.md) — the source of truth
- [doc/claude/plans/README.md](../README.md) — finished-initiatives
  table format
