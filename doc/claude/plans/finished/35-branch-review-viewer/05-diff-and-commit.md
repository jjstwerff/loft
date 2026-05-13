<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 05 — Diff and commit views

**Status:** Open

## Goal

Land the two routes that turn the viewer into a real review
tool:

1. `/diff/<path>` — render the unified diff of `<path>`
   against `main`.
2. `/commit/<sha>` — render a commit's message + per-file
   diffs.

Plus a `[Rendered ¦ Diff vs main]` toggle on every file page
(top-right corner) that flips between phase 03's rendered view
and the diff view.

The output of this phase is a **complete review surface**:
from the dashboard, click any changed file → see what
changed; click any recent commit → see the full commit; flip
between rendered + diff for any file.

## What ships

### Routes

| Method | Path | Handler |
|---|---|---|
| GET | `/diff/<path>` | Renders `tools/viewer/state/diffs/<safe>.diff` as syntax-coloured HTML |
| GET | `/commit/<sha>` | Renders `tools/viewer/state/commits/<sha>.diff` (extended refresh.sh dumps these) |
| GET | `/file/<path>` | (UNCHANGED from phase 03) But the page now includes the toggle button |

### Refresh-script extension

`tools/viewer/refresh.sh` extends to dump per-commit diffs:

```bash
# 6. Per-recent-commit diffs (last 20 commits)
COMMITS_DIR="$STATE/commits"
rm -rf "$COMMITS_DIR" && mkdir -p "$COMMITS_DIR"
git log --pretty=%H -20 | while read -r sha; do
  git show "$sha" > "$COMMITS_DIR/$sha.diff"
done
```

`git show` includes the message + diff in one output —
exactly what the commit page renders.

### Diff renderer module

`tools/viewer/src/diff_render.loft` (~150 lines):

```loft
pub fn render_unified(diff_text: text) -> text {
    // Parse line-by-line, classify each:
    //   "diff --git ..."       → file header (h2)
    //   "@@ -a,b +c,d @@ ..."  → hunk header (highlighted)
    //   "+..."                 → added line (green background)
    //   "-..."                 → deleted line (red background)
    //   " ..."                 → context line (no background)
    //   "\\ No newline ..."    → context note
    // Wrap each line with appropriate <span class="diff-add">, etc.
    ...
}
```

No regex needed — the line classifier is a single
`starts_with` chain.  HTML escape every line content; preserve
leading whitespace.

### Commit-page renderer

For `/commit/<sha>`:
1. Read `state/commits/<sha>.diff`.
2. Split on `diff --git` markers into per-file sections.
3. The first section (before any `diff --git`) is the commit
   header (sha, author, date, message).
4. Render the header with prose styling; render each file
   section with `render_unified`.
5. Wrap in the standard layout.

### Toggle button on file pages

Top-right of every `/file/<path>` page:

```html
<nav class="view-toggle">
  <a href="/file/{path}" class="active">Rendered</a>
  <a href="/diff/{path}">Diff vs main</a>
</nav>
```

The link to `/diff/<path>` only renders if a per-file diff
exists (i.e., the file is in `state/diffs/`).  Files
unchanged on this branch hide the toggle.

### Rendered diff for `.md` files (v1.5 polish)

For markdown files, offer a third option: "Rendered diff"
that renders `git diff --word-diff` semantically — show the
old + new prose side-by-side with deletions struck through
and additions highlighted.

This is a **stretch goal** for phase 05.  v1 of phase 05
ships the unified-diff path.  Side-by-side rendered diff
becomes a polish item in phase 07 if there's appetite.

### Sidebar status hooks

The sidebar tree (from phase 01) gains status badges from
the dashboard's `state/changed.json` data: every file in the
tree that's listed as changed gets a small `M`/`A`/`D` badge
next to its name.

## Critical files

| Path | Action |
|---|---|
| `tools/viewer/refresh.sh` | UPDATED: dump per-commit `.diff` files to `state/commits/` |
| `tools/viewer/src/diff_render.loft` | NEW (~150 lines) |
| `tools/viewer/src/route.loft` | UPDATED: `/diff/<path>`, `/commit/<sha>` handlers + toggle on `/file/<path>` |
| `tools/viewer/src/state.loft` | UPDATED: `read_diff(path)` and `read_commit(sha)` |
| `tools/viewer/src/style.loft` | UPDATED: `.diff-add`, `.diff-del`, `.diff-context`, `.diff-hunk`, `.diff-file-header`, `.view-toggle` |
| `tools/viewer/src/tree.loft` | UPDATED: status badges on tree entries |

## Existing functions / tooling to reuse

- **`html.escape`** (phase 02) for diff line content.
- **State readers** from phase 04 — extended for the new
  per-commit / per-file paths.
- **`text.split`** for per-file diff parsing.

## Test surface

- `curl -s http://localhost:8765/diff/doc/claude/PROBLEMS.md`
  returns HTML with `class="diff-add"` lines (the P259/P260/P261
  rows added on `demo_dev`).
- `curl -s http://localhost:8765/commit/cfad6274` returns the
  P260 commit's message + per-file diffs.
- File page for `src/parser/vectors.rs` shows the toggle;
  toggle to diff view shows the P260 hunk.
- File page for an unchanged file (e.g.,
  `src/lexer.rs` if untouched on this branch) hides the
  toggle.
- Dashboard's "Recent commits" links to `/commit/<sha>`;
  click navigates correctly.
- Sidebar shows `M` badges next to changed files in `doc/`,
  `src/`, etc.

## Verification

End-to-end on `demo_dev`:

```bash
$ make view-refresh && make view &
$ curl -s http://localhost:8765/diff/doc/claude/PROBLEMS.md \
  | grep -c 'class="diff-add"'
# ≥ 3 added rows for P259/P260/P261

$ curl -s http://localhost:8765/commit/cfad6274 | grep -c 'P260'
# ≥ 1 in the message; more in the diff

$ # Open browser → http://localhost:8765/
$ # Click "src/parser/vectors.rs" → see Rendered view
$ # Click "Diff vs main" toggle → see the P260 + P259-commit-2 hunks
$ # Click sha "cfad6274" from Recent commits → see commit message + diff
```

## Risks

| Risk | Mitigation |
|---|---|
| Huge diff (>10K lines) renders slowly or blows the page | Cap diff render at first 5000 lines; "show full diff" link if needed |
| `git show` output for binary files (e.g., committed images) breaks parser | Detect "Binary files differ" line and render a "binary file changed" stub instead of raw |
| Toggle button placement competes with sidebar/breadcrumbs | Top-right corner, fixed position; CSS keeps it out of the content flow |
| Per-file diff for a renamed file isn't accessible by either old or new path | Refresh script tracks renames separately (`git diff -M --name-status`) and writes diffs under both paths; accept slight duplication |
| Side-by-side rendered diff is more work than expected | Defer to phase 07 polish |

## Cross-references

- [Phase 04 — git state wrapper](04-git-state-wrapper.md) — provides the dump format this phase consumes
- [Phase 03 — minimal markdown](03-markdown-minimal.md) — feeds the "Rendered" side of the toggle
- [README § Phases](README.md#phases) — placement of this phase in the arc
