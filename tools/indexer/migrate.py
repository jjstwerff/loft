#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# tools/indexer/migrate.py — retroactive `@`-tagging pass for plan-37 phase 06.
#
# Walks `doc/claude/**/*.md` and rewrites bare-name tracker references to
# `@`-prefixed form:
#   - `P259` → `@P259`            (only when 259 is a valid PROBLEMS.md row ID)
#   - `plan-22` → `@PLAN22`       (only when 22 is a valid plan dir number)
#
# Conservative on purpose:
#   - Skips lines inside ```...``` or ~~~...~~~ fenced code blocks.
#   - Skips lines containing the `<!--noindex-->` marker.
#   - Skips `P\d+-R\d+` patterns (phase-N risk-M notation, not P-issues).
#   - Skips occurrences preceded by `@` (already tagged) or `[a-zA-Z0-9]`
#     (`2P259`, `nP259`, ...).
#   - Validates the numeric part against PROBLEMS.md / plan directories
#     before rewriting.
#
# By default runs as a DRY RUN — prints per-file change counts.  Pass
# `--apply` to write back in place.
#
# Usage:
#   tools/indexer/migrate.py              # dry-run, report counts
#   tools/indexer/migrate.py --apply      # rewrite files in place
#   tools/indexer/migrate.py --apply doc/claude/PROBLEMS.md   # one file

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def load_valid_pids() -> set[str]:
    """Row IDs from `| <N> |` lines at the start of PROBLEMS.md rows."""
    text = (REPO_ROOT / "doc/claude/PROBLEMS.md").read_text(encoding="utf-8")
    out: set[str] = set()
    for line in text.splitlines():
        m = re.match(r"^\| ([0-9]+) \|", line)
        if m:
            out.add(m.group(1))
    return out


def load_valid_plan_ids() -> set[str]:
    """Plan numeric IDs from `doc/claude/plans/(finished/|future/|deferred/)?NN-name`."""
    out: set[str] = set()
    plans_dir = REPO_ROOT / "doc/claude/plans"
    for entry in plans_dir.rglob("*"):
        if not entry.is_dir():
            continue
        rel = entry.relative_to(plans_dir)
        # Only direct children of plans/ or one of finished/, future/, deferred/
        parts = rel.parts
        if len(parts) == 1:
            name = parts[0]
        elif len(parts) == 2 and parts[0] in ("finished", "future", "deferred"):
            name = parts[1]
        else:
            continue
        m = re.match(r"^([0-9]+)-", name)
        if m:
            out.add(m.group(1))
            # Also accept unpadded form (plan-07 → 7) so refs without
            # leading zero validate.  Migration preserves the source
            # form, so both shapes work.
            out.add(str(int(m.group(1))))
    return out


P_PATTERN = re.compile(
    # Lookbehind excludes:
    #   `@`           — already migrated
    #   `[a-zA-Z0-9]` — compound identifier (`8d.P259`'s `.` is OK,
    #                   but `2P259` should be skipped)
    #   `/`           — URL / path component (`/tag/P259` is the
    #                   viewer's bare-form route; migrating would
    #                   break the URL)
    #
    # Numeric body requires 2+ digits — single-digit P-ids (`P1`,
    # `P2`, ..., `P9`) are heavily overloaded with non-P-issue
    # uses (PERFORMANCE.md "Design: P1"/"P2"/"P3", phase-N risk-M
    # notation as `P1-R3`, plan-N phase-M as `P5.2` for plan-17
    # phase 5.2).  Two-digit (`P54`) and longer are unambiguous
    # enough — the migrator validates the number against
    # PROBLEMS.md before rewriting, and 2-digit collisions are
    # rare in practice.
    r"(?P<lookbehind>(?:^|[^@a-zA-Z0-9/]))"
    r"P(?P<num>[0-9]{2,})(?P<suffix>[a-z]?)"
    r"(?P<lookahead>\b)"
)

PLAN_PATTERN = re.compile(
    # Same exclusions as P_PATTERN — `/tag/PLAN35` URLs etc.
    r"(?P<lookbehind>(?:^|[^@a-zA-Z0-9/]))"
    r"plan-(?P<num>[0-9]+)"
    # Only migrate when NOT followed by `-`.  Compound words like
    # `pre-plan-07-phase-4-step-4.8` shouldn't become
    # `pre-@PLAN07-phase-4-step-4.8` — the scanner would treat the  <!--noindex-->
    # whole `@PLAN07-phase-4-step-4.8` as a single sub-phase tag,  <!--noindex-->
    # which collides with the actual phase-tag convention
    # (`@PLAN07-04` style, not `@PLAN07-phase-4`).  <!--noindex-->  (illustrative example, not a live ref)
    r"(?![-0-9a-zA-Z])"
)


def backtick_spans(line: str) -> list[tuple[int, int]]:
    """Returns [(start, end)] of every inline-code span on the line.

    Skips backslash-escaped backticks.  Single-backtick spans only
    (loft docs don't use double-backtick spans for `` `…` `` content
    that itself contains a backtick).
    """
    spans: list[tuple[int, int]] = []
    i = 0
    n = len(line)
    while i < n:
        if line[i] == "`" and (i == 0 or line[i - 1] != "\\"):
            j = i + 1
            while j < n and line[j] != "`":
                j += 1
            if j < n:
                spans.append((i, j + 1))
                i = j + 1
                continue
        i += 1
    return spans


def in_span(pos: int, spans: list[tuple[int, int]]) -> bool:
    for s, e in spans:
        if s <= pos < e:
            return True
    return False


def migrate_line(line: str, valid_pids: set[str], valid_plans: set[str]) -> tuple[str, int]:
    """Returns (new_line, count_of_migrations)."""
    count = 0
    spans = backtick_spans(line)

    def sub_pid(m: re.Match) -> str:
        nonlocal count
        num = m.group("num")
        suffix = m.group("suffix")
        lookbehind = m.group("lookbehind")
        # Skip if the match is inside an inline-code span — those are
        # documentation examples (e.g., explaining what bare-name
        # form looks like), not real references.
        if in_span(m.start("num"), spans):
            return m.group(0)
        # Skip P\d+-R\d+ (phase-N risk-M notation) — look at what comes after.
        start_after = m.end()
        rest = line[start_after:]
        if re.match(r"^-R[0-9]+", rest):
            return m.group(0)
        if num not in valid_pids:
            return m.group(0)
        count += 1
        return f"{lookbehind}@P{num}{suffix}"

    def sub_plan(m: re.Match) -> str:
        nonlocal count
        num = m.group("num")
        lookbehind = m.group("lookbehind")
        if in_span(m.start("num"), spans):
            return m.group(0)
        if num not in valid_plans:
            return m.group(0)
        count += 1
        return f"{lookbehind}@PLAN{num}"

    line = P_PATTERN.sub(sub_pid, line)
    line = PLAN_PATTERN.sub(sub_plan, line)
    return line, count


def migrate_file(path: Path, valid_pids: set[str], valid_plans: set[str]) -> int:
    """Returns the migration count; rewrites file when migrations > 0."""
    text = path.read_text(encoding="utf-8")
    out_lines: list[str] = []
    in_fence = False
    fence_marker = ""
    total = 0

    for line in text.splitlines(keepends=True):
        stripped = line.lstrip()
        # Track fence state.  Treat ``` and ~~~ as separate fence flavors so
        # interleaved usage doesn't confuse the parser.
        if not in_fence:
            if stripped.startswith("```"):
                in_fence = True
                fence_marker = "```"
                out_lines.append(line)
                continue
            if stripped.startswith("~~~"):
                in_fence = True
                fence_marker = "~~~"
                out_lines.append(line)
                continue
        else:
            if stripped.startswith(fence_marker):
                in_fence = False
                fence_marker = ""
            out_lines.append(line)
            continue

        # Skip explicit noindex lines.
        if "<!--noindex-->" in line:
            out_lines.append(line)
            continue

        new_line, n = migrate_line(line, valid_pids, valid_plans)
        total += n
        out_lines.append(new_line)

    if total > 0:
        path.write_text("".join(out_lines), encoding="utf-8")
    return total


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Retroactive @-tagging for tracker references."
    )
    ap.add_argument(
        "--apply",
        action="store_true",
        help="Write changes in place (default: dry run, report counts only).",
    )
    ap.add_argument(
        "paths",
        nargs="*",
        help="Specific .md files to migrate (default: doc/claude/**/*.md).",
    )
    args = ap.parse_args()

    valid_pids = load_valid_pids()
    valid_plans = load_valid_plan_ids()
    print(
        f"loaded {len(valid_pids)} valid P-ids, {len(valid_plans)} valid plan-ids",
        file=sys.stderr,
    )

    if args.paths:
        files = [Path(p).resolve() for p in args.paths]
    else:
        files = sorted((REPO_ROOT / "doc/claude").rglob("*.md"))

    total_migrations = 0
    files_changed = 0

    for f in files:
        if not f.is_file():
            print(f"  skip {f} (not a file)", file=sys.stderr)
            continue
        if args.apply:
            n = migrate_file(f, valid_pids, valid_plans)
        else:
            # Dry run — count but don't write.
            text = f.read_text(encoding="utf-8")
            n = 0
            in_fence = False
            fence_marker = ""
            for line in text.splitlines(keepends=True):
                stripped = line.lstrip()
                if not in_fence:
                    if stripped.startswith("```"):
                        in_fence = True
                        fence_marker = "```"
                        continue
                    if stripped.startswith("~~~"):
                        in_fence = True
                        fence_marker = "~~~"
                        continue
                else:
                    if stripped.startswith(fence_marker):
                        in_fence = False
                        fence_marker = ""
                    continue
                if "<!--noindex-->" in line:
                    continue
                _, c = migrate_line(line, valid_pids, valid_plans)
                n += c
        if n > 0:
            rel = f.relative_to(REPO_ROOT)
            verb = "rewrote" if args.apply else "would rewrite"
            print(f"  {verb} {rel} ({n} refs)")
            total_migrations += n
            files_changed += 1

    if args.apply:
        print(
            f"\napplied: {total_migrations} refs across {files_changed} files",
            file=sys.stderr,
        )
    else:
        print(
            f"\ndry run: {total_migrations} refs in {files_changed} files would change",
            file=sys.stderr,
        )
        print("rerun with --apply to write changes.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
