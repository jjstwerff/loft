#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which chapters of the language reference owe a human read — and which have MOVED.

The reference is what we promise the people who use loft: it ships in all four release
bundles and on the docs site, and a reader takes it as the definition.  Whether it is
still TRUE is not a property a script can decide.  `release-checklist.py`'s `A-pdf*`
checks establish that the document is whole and current -- every chapter present, built
from live inputs, stamped with this version -- and none of them reads a sentence.  A
chapter can be freshly generated, correctly versioned, structurally perfect, and
describe behaviour the language stopped having two releases ago.

So the review is by hand, and the only real question is when.  Done on tag day it is a
day of reading under time pressure, which is how it turns into a skim.  This makes it
**continuous** instead, with the same watermark idea LIBRARY_DOC_REVIEW.md uses: each
chapter records the commit it was last read through, and a chapter is only back on the
list once its SOURCE has moved past that.  Review a chapter the week its topic changes
and the tag-day list is short by construction.

    scripts/reference-review.py              # the worklist
    scripts/reference-review.py --verbose    # + the commits behind each MOVED chapter

The watermark table lives in `doc/claude/REFERENCE_REVIEW.md` and is edited by hand when
a chapter is read.  It is the ONE home for "reviewed through": a second machine-readable
copy would be a second list of the same fact, and would drift the moment someone updated
only the prose.
"""

from __future__ import annotations

import argparse
import datetime
import os
import pathlib
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DOC = os.path.join(ROOT, "doc", "claude", "REFERENCE_REVIEW.md")

# The chapters that are NOT topics, and the hand-written source each is rendered from.
# `gendoc` reads all four with `if let Ok(read_to_string(..))`, so each is a real file
# with its own history -- which is exactly what a watermark needs.
FIXED_CHAPTERS = [
    ("doc/install.html", "Getting Started"),
    ("doc/00-vs-rust.html", "vs Rust"),
    ("doc/00-vs-python.html", "vs Python"),
    ("doc/roadmap.html", "Roadmap"),
    ("default", "Standard Library"),
]


def sh(*args: str) -> tuple[int, str]:
    try:
        p = subprocess.run(args, cwd=ROOT, capture_output=True, text=True, timeout=60)
        return p.returncode, (p.stdout + p.stderr).strip()
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return 1, ""


def chapters() -> list[tuple[str, str]]:
    """(source path, chapter title) for every level-1 part of the reference.

    Derived, never listed: the topic set is whatever `tests/docs/` holds, keyed the way
    `documentation::gather_topics` keys it (`@NAME`, skipping `00-*`), so a topic added
    tomorrow appears here without anyone maintaining a second list.
    """
    out: list[tuple[str, str]] = []
    docs = os.path.join(ROOT, "tests", "docs")
    for entry in sorted(os.listdir(docs)):
        path = os.path.join(docs, entry)
        if not entry.endswith(".loft") or entry.startswith("00-") or not os.path.isfile(path):
            continue
        name = ""
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if line.startswith("// @NAME: "):
                    name = line[len("// @NAME: ") :].strip()
        out.append((f"tests/docs/{entry}", name or entry))
    out.extend(FIXED_CHAPTERS)
    return out


def watermarks() -> dict[str, tuple[str, str]]:
    """The table in REFERENCE_REVIEW.md, as data: source -> (reviewed, commit)."""
    marks: dict[str, tuple[str, str]] = {}
    if not os.path.isfile(DOC):
        return marks
    in_table = False
    with open(DOC, encoding="utf-8") as f:
        for line in f:
            if re.match(r"^\| *chapter source *\| *reviewed through *\|", line):
                in_table = True
                continue
            if in_table:
                if re.match(r"^\|[ :\-]*-[ :\-]*\|", line):
                    continue
                if not line.startswith("|"):
                    in_table = False
                    continue
                cells = [c.strip().strip("`") for c in line.split("|")[1:-1]]
                if len(cells) >= 3 and cells[0]:
                    marks[cells[0].rstrip("/")] = (cells[1], cells[2])
    return marks


def source_commit(path: str) -> str:
    """The last commit that touched this chapter's source.

    Recorded as the watermark rather than `HEAD`, for two reasons.  It is MEANINGFUL --
    the row names the change the reviewer actually read, not whatever unrelated commit
    happened to be checked out that afternoon.  And it is STABLE: re-marking a chapter
    nobody has touched writes the same value, so the table only changes when the answer
    does.
    """
    code, out = sh("git", "log", "-1", "--format=%h", "--", path)
    return out.strip() if code == 0 and out.strip() else "HEAD"


def write_watermark(source: str, title: str, commit: str | None) -> str:
    """Add, replace or remove one row of the table in REFERENCE_REVIEW.md.

    Editing the doc rather than a sidecar keeps the one-home rule the table is built on:
    the reviewer, the aid and the release checklist all read the same rows, so there is
    no second copy to drift.  `commit=None` removes the row.
    """
    text = pathlib.Path(DOC).read_text(encoding="utf-8")
    lines = text.splitlines()
    head = next(
        (i for i, l in enumerate(lines) if re.match(r"^\| *chapter source *\|", l)), None
    )
    if head is None:
        sys.exit(f"{DOC}: no watermark table to write to")
    start = head + 2  # header + separator
    end = start
    while end < len(lines) and lines[end].startswith("|"):
        end += 1
    rows = {}
    for line in lines[start:end]:
        cells = [c.strip().strip("`") for c in line.split("|")[1:-1]]
        if len(cells) >= 3 and cells[0]:
            rows[cells[0]] = (cells[1], cells[2])
    if commit is None:
        if source not in rows:
            return f"{source} had no row"
        rows.pop(source)
        verdict = f"removed the watermark for {source}"
    else:
        today = datetime.date.today().isoformat()
        rows[source] = (today, commit)
        verdict = f"{title} — validated at {commit} ({today})"
    body = [
        f"| `{k}` | {v[0]} | `{v[1]}` |" for k, v in sorted(rows.items())
    ]
    pathlib.Path(DOC).write_text(
        "\n".join(lines[:start] + body + lines[end:]) + "\n", encoding="utf-8"
    )
    return verdict


def moved_since(commit: str, path: str) -> list[str]:
    """Commits touching `path` after `commit` — the reason a chapter is back on the list."""
    code, out = sh("git", "log", "--oneline", f"{commit}..HEAD", "--", path)
    if code != 0:
        return ["(cannot resolve that commit — check the watermark)"]
    return [l for l in out.splitlines() if l.strip()]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--verbose", action="store_true", help="list the commits behind each MOVED chapter")
    ap.add_argument(
        "--done",
        metavar="SOURCE",
        help="record a chapter as validated at its source's current commit "
        "(e.g. --done tests/docs/07-vector.loft)",
    )
    ap.add_argument("--undo", metavar="SOURCE", help="remove a chapter's watermark row")
    args = ap.parse_args()

    pop = chapters()
    if args.done or args.undo:
        by_src = {src: title for src, title in pop}
        for target, commit in ((args.done, "set"), (args.undo, None)):
            if not target:
                continue
            if target not in by_src:
                print(f"not a chapter source: {target}", file=sys.stderr)
                print("  run with no arguments to see the list", file=sys.stderr)
                return 2
            print(
                write_watermark(
                    target,
                    by_src[target],
                    source_commit(target) if commit == "set" else None,
                )
            )
        pop = chapters()
    marks = watermarks()
    known = {src for src, _ in pop}

    never, moved, current = [], [], []
    for src, title in pop:
        mark = marks.get(src)
        if mark is None:
            never.append((src, title))
            continue
        commits = moved_since(mark[1], src)
        if commits:
            moved.append((src, title, mark, commits))
        else:
            current.append((src, title, mark))

    # A row matching no chapter is REPORTED, not dropped: it means a chapter was renamed
    # or removed and the table still claims it was reviewed, which reads as coverage.
    stale_rows = [k for k in marks if k not in known]

    print(f"Reference review — {len(pop)} chapters\n")
    if never:
        print(f"NEVER REVIEWED ({len(never)}) — no watermark row:")
        for src, title in never:
            print(f"  {title:<34} {src}")
        print()
    if moved:
        print(f"MOVED since its watermark ({len(moved)}) — owes a re-read:")
        for src, title, mark, commits in moved:
            print(f"  {title:<34} {src}")
            print(f"      reviewed through {mark[1]} ({mark[0]}) — {len(commits)} commit(s) since")
            if args.verbose:
                for c in commits:
                    print(f"        {c}")
        print()
    if stale_rows:
        print(f"STALE ROWS ({len(stale_rows)}) — a watermark for something that is not a chapter:")
        for k in stale_rows:
            print(f"  {k}")
        print()
    print(f"{len(current)}/{len(pop)} chapters reviewed at their current source.")
    if never or moved:
        print(
            f"\nRead the {len(never) + len(moved)} above against the language as it "
            f"behaves NOW, then record each in doc/claude/REFERENCE_REVIEW.md."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
