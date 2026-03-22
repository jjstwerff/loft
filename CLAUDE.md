# Claude Code Instructions for the Loft Project

## Session start

Read `doc/claude/QUICK_START.md` before beginning any implementation work.

---

## Branch policy — MANDATORY

**Direct commits to `main` are not allowed.**

All changes — features, bug fixes, refactors, documentation updates — must land
on a feature branch and reach `main` only through a pull request.

The **currently active development branch** is `benchmark`.

### Why

`main` is the release branch.  Every commit on `main` is expected to be
releasable.  Direct commits bypass code review, CI, and the structured commit
sequence documented in `doc/claude/DEVELOPMENT.md`.  Feature branches keep
`main` clean and give each item a traceable history.

### Rules

1. **Never `git commit` directly on `main`.**  If you accidentally land on
   `main`, move the change to a feature branch before anything else.
2. **Never `git push` without an explicit user instruction** — see the
   [feedback memory](memory/feedback_no_github_automation.md) and the
   Remote CI section of `doc/claude/DEVELOPMENT.md`.
3. Create branches from the tip of `main` using the naming convention in
   `doc/claude/DEVELOPMENT.md` (e.g. `p1-1-lambda-parser`, `benchmark`).
4. Merging back to `main` is done via a GitHub pull request — not a local
   `git merge`.

---

## Key documentation

| File | Purpose |
|------|---------|
| `doc/claude/QUICK_START.md` | Session checklist — read first |
| `doc/claude/DEVELOPMENT.md` | Full branch / commit / CI workflow |
| `doc/claude/PLANNING.md` | Backlog with effort estimates |
| `doc/claude/PROBLEMS.md` | Known bugs — update when finding or fixing issues |
| `doc/claude/CODE.md` | Naming rules, function-length limits, null sentinels |
| `doc/claude/TESTING.md` | Test framework reference |
| `doc/claude/CHANGELOG.md` | Release history |
