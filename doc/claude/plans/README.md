# Plans

Multi-phase initiatives that span more than one session.  Each
subdirectory holds the README (goal + index) plus one markdown file
per phase.

## Conventions

- Subdirectory names are numbered (`NN-slug`) so they sort in the
  order they were opened.  The number is a monotonic counter — it
  does not imply priority.
- A new initiative opens with an `NN-slug/README.md` stating the
  goal, phase layout, and ground rules, plus a first phase plan
  file (conventionally `00-<first-phase>.md`).
- Every phase plan file begins with `Status: open | in-progress |
  done` so a fresh session can orient quickly.
- When an initiative is fully closed (all phases committed, no open
  follow-ups), move its entire subdirectory into `finished/`.
  That way `ls doc/claude/plans/` at a glance shows only live work.

## Current initiatives

| Dir | Initiative | Current phase |
|---|---|---|
| `00-inline-lift-safety/` | Eliminate silent memory corruption from inline struct-returning calls in expression contexts (P181 family). | Phase 0 — diagnostic (`00-p181-diagnostic.md`) |

## Finished initiatives

See `finished/` (empty at time of writing).

## One-off plans elsewhere

Per-session ephemeral plans not tied to a multi-phase initiative
live under `~/.claude/plans/` (flat, generated filenames).  Those
are not committed to the repo.
