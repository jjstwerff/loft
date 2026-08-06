#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN131 — how much of loft's diagnostic surface carries a code (and so can carry a fix).

Scans every emission form in `src/` and reports coverage by level, the complete list of
uncoded warnings and advice, and where the uncoded errors live.

The warning/advice list is the one worth reading. An error stops the build, so its reader is
already acting on it; a warning fires on a program that WORKS and can be ignored forever —
that is the reader a suggestion changes the behaviour of, and the set is small enough to
finish.

Usage:  python3 tools/diag_inventory.py [--json]
"""

import json
import os
import re
import sys
from collections import Counter

ROOT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "src")

# Every form that reaches `Diagnostics`. Adding an emission helper without adding it here
# makes this under-report, which is the one way the numbers can lie.
FORMS = [
    re.compile(r"\bdiagnostic!\s*\("),
    re.compile(r"\bdiagnostic_at!\s*\("),
    re.compile(r"\bspecific!\s*\("),
    re.compile(r"\.add_at_coded\s*\("),
    re.compile(r"\.add_at\s*\("),
    re.compile(r"\.add\s*\(\s*(?:crate::diagnostics::)?Level::"),
    re.compile(r"\bself\.err(?:_coded)?\s*\("),
]
LEVEL = re.compile(r"Level::(Fatal|Error|Warning|Advice|Debug)")
CODE = re.compile(r'code\s*=\s*"([a-z0-9-]+)"|_coded\s*\(\s*[^,]*,\s*(?:Some\()?"([a-z0-9-]+)"')
USER_LEVELS = ("Fatal", "Error", "Warning", "Advice")


def scan():
    """Every user-facing emission site, as dicts."""
    rows = []
    for dirpath, _, files in os.walk(ROOT):
        for fn in sorted(files):
            if not fn.endswith(".rs"):
                continue
            path = os.path.join(dirpath, fn)
            rel = os.path.relpath(path, os.path.dirname(ROOT))
            text = open(path, encoding="utf-8").read()
            lines = text.split("\n")
            # Stop at the first `#[cfg(test)]`: a test's diagnostic is not a surface.
            m = re.search(r"^#\[cfg\(test\)\]\s*$", text, re.M)
            cut = text[: m.start()].count("\n") if m else len(lines)
            for i, line in enumerate(lines[:cut]):
                if not any(rx.search(line) for rx in FORMS):
                    continue
                window = "\n".join(lines[i : i + 8])
                lv = LEVEL.search(window)
                cm = CODE.search(window)
                rows.append(
                    {
                        "file": rel,
                        "line": i + 1,
                        "level": lv.group(1) if lv else "?",
                        "code": (cm.group(1) or cm.group(2)) if cm else None,
                        "message": message(lines, i),
                    }
                )
    return [r for r in rows if r["level"] in USER_LEVELS]


def message(lines, i):
    """The first string literal at or after the emission — the diagnostic's prose.

    A wider window than the level/code scan on purpose: loft's longer messages are written
    as backslash continuations across many lines, so a window sized for the call itself
    finds an opening quote and no closing one, and reports `?` for exactly the diagnostics
    that had the most to say.
    """
    window = re.sub(r"\s+", " ", " ".join(lines[i : i + 24]))
    m = re.search(r'"((?:[^"\\]|\\.){6,})"', window)
    if not m:
        return "?"
    t = re.sub(r"\s+", " ", m.group(1).replace("\\", ""))
    return t[:88] + "…" if len(t) > 88 else t


def main():
    rows = scan()
    if "--json" in sys.argv:
        json.dump(rows, sys.stdout, indent=1)
        return
    coded = sum(1 for r in rows if r["code"])
    print(f"{len(rows)} user-facing diagnostic sites; {coded} carry a code "
          f"({100 * coded / len(rows):.1f}%)\n")
    print("by level:")
    for lv in ("Advice", "Warning", "Error", "Fatal"):
        g = [r for r in rows if r["level"] == lv]
        if not g:
            continue
        c = sum(1 for r in g if r["code"])
        print(f"  {lv:8s} {c:3d}/{len(g):3d}  ({100 * c / len(g):.0f}%)")

    print("\nuncoded warnings and advice — the set worth finishing:")
    for r in rows:
        if r["level"] in ("Warning", "Advice") and not r["code"]:
            print(f"  {r['level']:7s} {r['file']}:{r['line']}  {r['message']}")

    print("\nuncoded errors, by file (a map, not a work-list — code an error when it")
    print("gets a fix, since a code is a frozen surface):")
    c = Counter(r["file"] for r in rows if r["level"] in ("Error", "Fatal") and not r["code"])
    for f, n in c.most_common():
        print(f"  {n:4d}  {f}")


if __name__ == "__main__":
    main()
