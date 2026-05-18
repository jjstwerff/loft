#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# tools/indexer/fix_broken_links.py — auto-fix broken markdown links
# surfaced by tools/indexer/scan.sh's broken_links bucket.
#
# For each broken link, try common repair patterns:
#   - off-by-one `../` (most common: README in `plans/<dir>/` cites
#     a top-level doc as `../X.md` but should be `../../X.md`)
#   - moved-to-finished/ (`plans/22-name/` → `plans/finished/22-name/`)
#
# Dry-run by default — prints what WOULD change.  Pass `--apply` to
# rewrite in place.

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


def load_links_bucket() -> dict:
    """Returns {target: [{file, line, anchor, context}]} from index/tags.json."""
    idx = REPO_ROOT / "index/tags.json"
    return json.loads(idx.read_text())["links"]


def load_broken_targets() -> set[str]:
    idx = REPO_ROOT / "index/tags.json"
    data = json.loads(idx.read_text())
    return {entry["target"] for entry in data.get("broken_links", [])}


def try_extra_dotdot(target: str) -> str | None:
    """If target is a missing path under doc/claude/plans/<dir>/X.md
    (would-be off-by-one), check whether adding one more `../` would
    resolve to an existing file.  Returns the FIXED resolved path
    or None."""
    # The broken target is the resolved-against-source path.  We
    # don't have direct access to "what was the original link
    # text"; we infer the off-by-one fix by:
    #   - if target's last K segments exist as `doc/claude/<rest>`,
    #     the link author meant the higher-level path.
    parts = target.split("/")
    # Try popping intermediate segments one at a time and checking
    # if the result exists.
    for i in range(len(parts) - 1):
        candidate_parts = parts[:i] + parts[i + 1 :]
        candidate = "/".join(candidate_parts)
        if (REPO_ROOT / candidate).is_file():
            return candidate
    return None


def relpath(source_dir: str, abs_target: str) -> str:
    """Compute target as a relative path from source_dir."""
    src = Path(source_dir).resolve() if source_dir else REPO_ROOT
    tgt = REPO_ROOT / abs_target
    try:
        return str(tgt.relative_to(src))
    except ValueError:
        # Walk up.
        rel_parts = []
        cur = src
        while not str(tgt).startswith(str(cur)):
            rel_parts.append("..")
            if cur == cur.parent:
                break
            cur = cur.parent
        if cur == REPO_ROOT and tgt == REPO_ROOT:
            return "."
        try:
            tail = str(tgt.relative_to(cur))
        except ValueError:
            tail = str(tgt)
        return "/".join(rel_parts + [tail])


# Manual overrides: known stale paths → correct paths.
MANUAL_FIXES: dict[str, str] = {
    # Plan-22 closeout drift
    "doc/claude/plans/22-mutable-closures/README.md":
        "doc/claude/plans/finished/22-mutable-closures/README.md",
    # Plan-35 closeout drift
    "doc/claude/plans/35-branch-review-viewer/README.md":
        "doc/claude/plans/finished/35-branch-review-viewer/README.md",
    "doc/claude/plans/35-branch-review-viewer/03-markdown-minimal.md":
        "doc/claude/plans/finished/35-branch-review-viewer/03-markdown-minimal.md",
}


# Targets to skip — known-intentional placeholders.
SKIP_TARGETS: set[str] = {
    "FOO.md",   # _TEMPLATE.md placeholder
}


def find_replacement(broken_target: str, source_file: str) -> str | None:
    """Returns the correct relative-path string to put in the source file,
    or None if we can't find a fix."""
    # 1. Manual override
    if broken_target in MANUAL_FIXES:
        new_abs = MANUAL_FIXES[broken_target]
    else:
        new_abs = try_extra_dotdot(broken_target)
    if new_abs is None:
        return None
    if not (REPO_ROOT / new_abs).is_file():
        return None
    # Compute relative path from source's directory
    src_dir = str(Path(source_file).parent)
    return relpath(src_dir, new_abs)


def rewrite_link_in_line(
    line: str, broken_target: str, source_file: str, replacement_rel: str
) -> tuple[str, bool]:
    """Find the broken link in `line` and rewrite the path portion.
    Returns (new_line, changed)."""
    # The broken link in the line is `[text](X.md)` or `[text](X.md#anchor)`.
    # We don't know the original link text — but we can find it by
    # matching `(X.md...)` patterns and resolving each one until we
    # hit the broken target.
    src_dir = str(Path(source_file).parent)

    def resolve(target_raw: str) -> str:
        # Same logic as scan.sh's awk
        target = target_raw
        while target.startswith("./"):
            target = target[2:]
        while target.startswith("../"):
            base = src_dir
            if "/" in base:
                base = base.rsplit("/", 1)[0]
            else:
                base = ""
            src_dir_local = base
            # Apply
            target = target[3:]
            src_dir_local = base
            # Reset src_dir to walked-up version for next iteration
            return resolve_inner(target, src_dir_local)
        if target.startswith("/"):
            return target.lstrip("/")
        if src_dir == "":
            return target
        return src_dir + "/" + target

    def resolve_inner(target: str, base: str) -> str:
        while target.startswith("./"):
            target = target[2:]
        while target.startswith("../"):
            if "/" in base:
                base = base.rsplit("/", 1)[0]
            else:
                base = ""
            target = target[3:]
        if target.startswith("/"):
            return target.lstrip("/")
        if base == "":
            return target
        return base + "/" + target

    # Find every `](X.md...)` match
    pattern = re.compile(r"\]\(([^)#\s]+\.md)(#[^)]*)?\)")
    changed = False
    new_line_parts: list[str] = []
    last = 0
    for m in pattern.finditer(line):
        target_raw = m.group(1)
        anchor = m.group(2) or ""
        if target_raw.startswith(("http:", "https:", "mailto:")):
            continue
        if target_raw.startswith("/"):
            resolved = target_raw[1:]
        else:
            resolved = resolve_inner(target_raw, src_dir)
        if resolved == broken_target:
            # Replace
            new_line_parts.append(line[last : m.start(1)])
            new_line_parts.append(replacement_rel)
            new_line_parts.append(anchor)
            new_line_parts.append(line[m.end() - 1])  # closing `)`
            last = m.end()
            changed = True
    new_line_parts.append(line[last:])
    return "".join(new_line_parts), changed


def main() -> int:
    ap = argparse.ArgumentParser(
        description="Auto-fix broken markdown links surfaced by phase 09."
    )
    ap.add_argument(
        "--apply",
        action="store_true",
        help="Write changes in place (default: dry run).",
    )
    args = ap.parse_args()

    idx = REPO_ROOT / "index/tags.json"
    if not idx.is_file():
        print("index/tags.json missing — run: make index", file=sys.stderr)
        return 1
    data = json.loads(idx.read_text())
    broken_links = data.get("broken_links", [])

    fixed = 0
    skipped = 0
    files_to_rewrite: dict[str, list[tuple[int, str, str, str]]] = {}
    # files_to_rewrite[file] = [(line_number, broken_target, replacement, original_link_text), ...]

    for entry in broken_links:
        target = entry["target"]
        if target in SKIP_TARGETS:
            skipped += 1
            continue
        for ref in entry["refs"]:
            file_str, line_str = ref.rsplit(":", 1)
            line_no = int(line_str)
            replacement = find_replacement(target, file_str)
            if replacement is None:
                skipped += 1
                continue
            files_to_rewrite.setdefault(file_str, []).append(
                (line_no, target, replacement, "")
            )

    for file_str, fixes in sorted(files_to_rewrite.items()):
        path = REPO_ROOT / file_str
        if not path.is_file():
            print(f"  warn: {file_str} doesn't exist", file=sys.stderr)
            continue
        text = path.read_text(encoding="utf-8")
        lines = text.splitlines(keepends=True)
        # Sort fixes by line_no descending so multiple fixes per line work
        fixes_by_line: dict[int, list[tuple[str, str]]] = {}
        for line_no, broken, repl, _ in fixes:
            fixes_by_line.setdefault(line_no, []).append((broken, repl))
        per_file_changed = 0
        for line_no, repls in fixes_by_line.items():
            old_line = lines[line_no - 1]
            new_line = old_line
            for broken, repl in repls:
                candidate, changed = rewrite_link_in_line(
                    new_line, broken, file_str, repl
                )
                if changed:
                    new_line = candidate
                    per_file_changed += 1
                    fixed += 1
                else:
                    skipped += 1
            lines[line_no - 1] = new_line
        if per_file_changed > 0:
            verb = "rewrote" if args.apply else "would rewrite"
            print(f"  {verb} {file_str} ({per_file_changed} fixes)")
            for line_no, repls in sorted(fixes_by_line.items()):
                for broken, repl in repls:
                    print(f"    L{line_no}: {broken} → {repl}")
            if args.apply:
                path.write_text("".join(lines), encoding="utf-8")

    if args.apply:
        print(f"\napplied: {fixed} fixes, {skipped} skipped", file=sys.stderr)
    else:
        print(f"\ndry run: {fixed} fixes possible, {skipped} skipped", file=sys.stderr)
        print("rerun with --apply to write changes.", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
