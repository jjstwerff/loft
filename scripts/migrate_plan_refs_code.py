#!/usr/bin/env python3
"""Migrate stale plan-directory PATH references in CODE files after the @PLN27 flat
migration.

The @PLN27 migration flattened `doc/claude/(lib_)plans/{future,deferred}/<NN>-slug/`
to flat issue-numbered `doc/claude/(lib_)plans/<PLN>-slug/`, but its ref-rewriter
(`scripts/migrate_plan_refs.py`) only touched `.md` files.  Bare path mentions in
code COMMENTS (`.rs`, `.loft`, …) still point at the old `future/`/`deferred/`
locations.  This fixes those.

The old->new mapping is recovered by SLUG matched against the CURRENT flat plan
tree — NOT by git renames: the dir slug is preserved across the move (only the
number + the `future/`|`deferred/` segment change), and slug-matching is robust to
how git recorded each move (some were add+delete, not detected renames) and works
even post-merge.  `finished/` was NOT migrated, so a ref into `finished/` — or any
slug with no flat match (e.g. a pre-existing wrong path) — is left untouched and
reported, never guessed.

Default: dry run (report only).  Pass `--apply` to write.
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip())

# A stale ref anywhere on a line: <tree>/(future|deferred)/<NN>-<slug>.  `\b` +
# alternation order keep `lib_plans` from being matched as a bare `plans`.
REF = re.compile(r"\b(lib_plans|plans)/(?:future|deferred)/\d+-([a-z0-9][a-z0-9-]*)")


def slug_maps():
    """slug -> '<NN>-<slug>' for the CURRENT flat dirs, one map per tree.

    Only flat `<digits>-<slug>` dirs count; the `future/`/`deferred/`/`finished/`
    subdirs don't start with a digit, so they're excluded by construction.
    """
    maps = {"plans": {}, "lib_plans": {}}
    for tree in maps:
        base = ROOT / "doc" / "claude" / tree
        if not base.is_dir():
            continue
        for d in base.iterdir():
            m = re.match(r"\d+-(.+)$", d.name)
            if d.is_dir() and m:
                maps[tree][m.group(1)] = d.name
    return maps


def main():
    apply = "--apply" in sys.argv
    maps = slug_maps()
    unmapped = {}  # 'tree/.../slug' -> count
    total = 0
    files_changed = 0

    for rel in subprocess.check_output(["git", "ls-files"], text=True).split():
        # `.md` is already migrated by scripts/migrate_plan_refs.py; generated
        # artifacts (the tracker-index golden, lint/test baselines) are rebuilt by
        # their own tools (`make index`, the lint script) — never hand-edited here.
        if rel.endswith((".md", ".baseline")):
            continue
        if rel.startswith(("tests/golden/", "index/")) or Path(rel).name == ".lint_comments_baseline":
            continue
        fp = ROOT / rel
        if not fp.is_file():
            continue
        try:
            text = orig = fp.read_text()
        except (UnicodeDecodeError, OSError):
            continue

        hits = [0]

        def sub(m):
            tree, slug = m.group(1), m.group(2)
            newdir = maps.get(tree, {}).get(slug)
            if newdir is None:
                key = f"{tree}/(future|deferred)/…-{slug}"
                unmapped[key] = unmapped.get(key, 0) + 1
                return m.group(0)
            hits[0] += 1
            return f"{tree}/{newdir}"

        text = REF.sub(sub, text)
        if text != orig:
            print(f"  {'rewrote' if apply else 'would rewrite'} {rel} ({hits[0]} ref(s))")
            total += hits[0]
            files_changed += 1
            if apply:
                fp.write_text(text)

    verb = "applied" if apply else "dry run"
    print(f"\n{verb}: {total} ref(s) across {files_changed} file(s)")
    if unmapped:
        print("UNMAPPED (left as-is — finished/ or a pre-existing wrong path):")
        for key, n in sorted(unmapped.items()):
            print(f"  {key}  ({n})")
    if not apply:
        print("rerun with --apply to write.")


if __name__ == "__main__":
    main()
