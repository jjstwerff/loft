#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Read `Contract:` trailers off fix commits and apply the `contract:` labels.

The `contract:` axis (.github/LABELS.md) records whether closing a bug had to MOVE the
written standard.  It is set at FIX time, because that is the only moment the answer
exists — it is what the fix turned out to need, not something the filer could know.

`.githooks/commit-msg` asks for the trailer while you are typing the message.  This is
the BACKSTOP for the ones that get through anyway: `--report` names every `Fixes #N`
commit on the branch whose trailer is missing, so a miss is recoverable later instead of
becoming a permanently unjudged issue.  That backstop is the point — `Fixes #N` itself
was in CLAUDE.md, ISSUE_TRACKING.md and two skills before a hook existed, and fixes still
shipped without it; prose alone did not make it stick, and neither will this.

Usage:
    scripts/contract_labels.py                     # report what the branch says (dry run)
    scripts/contract_labels.py --apply             # label the issues on GitHub
    scripts/contract_labels.py --base origin/main  # commits since this ref (default)
    scripts/contract_labels.py --report            # ONLY the unjudged fixes, for a nudge
"""
import argparse
import re
import subprocess
import sys

TRAILER = re.compile(r"^[ \t]*Contract:[ \t]*(settled|strained)\b", re.I | re.M)
CLOSES = re.compile(r"\b(?:fixes|closes|resolves)\s+#(\d+)", re.I)


def commits(base):
    """Every commit on HEAD not in `base`, as (sha, subject, body)."""
    sep = "\x1e"
    out = subprocess.run(
        ["git", "log", f"{base}..HEAD", "--no-merges", f"--format=%H%x1f%s%x1f%B{sep}"],
        capture_output=True, text=True, check=False)
    if out.returncode != 0:
        sys.exit(f"git log failed: {out.stderr.strip()}")
    for rec in out.stdout.split(sep):
        rec = rec.strip("\n")
        if not rec:
            continue
        sha, subject, body = rec.split("\x1f", 2)
        yield sha, subject, body


def judgements(base):
    """(issue -> verdict, and the unjudged fixes) read off the branch's commits.

    A later commit wins: a follow-up that revises the call is the newer knowledge, and
    the alternative — first-writer-wins — would pin a verdict the author already
    corrected.
    """
    verdict, unjudged = {}, []
    for sha, subject, body in commits(base):
        issues = sorted({int(n) for n in CLOSES.findall(body)})
        if not issues:
            continue
        m = TRAILER.search(body)
        if m:
            for n in issues:
                verdict[n] = m.group(1).lower()
        else:
            unjudged.append((sha[:8], subject, issues))
    return verdict, unjudged


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--apply", action="store_true", help="write the labels via gh")
    ap.add_argument("--report", action="store_true", help="only list unjudged fixes")
    a = ap.parse_args()

    verdict, unjudged = judgements(a.base)

    if not a.report:
        print(f"=== `Contract:` trailers on {a.base}..HEAD ===")
        if not verdict:
            print("  (none)")
        for n, v in sorted(verdict.items()):
            print(f"  #{n:<6} contract:{v}")

    if unjudged:
        print(f"\n=== UNJUDGED — `Fixes #N` with no `Contract:` trailer ({len(unjudged)}) ===")
        print("  These count as unjudged, never as settled.  Amend the commit, or set the")
        print("  label on the issue by hand.")
        for sha, subject, issues in unjudged:
            print(f"  {sha}  {' '.join('#'+str(i) for i in issues):<24} {subject[:56]}")
    elif not a.report:
        print("\n  every fix on this branch carries a verdict")

    if a.apply:
        for n, v in sorted(verdict.items()):
            other = "strained" if v == "settled" else "settled"
            subprocess.run(["gh", "issue", "edit", str(n),
                            "--add-label", f"contract:{v}",
                            "--remove-label", f"contract:{other}"],
                           capture_output=True, text=True, check=False)
            print(f"  labelled #{n} contract:{v}")
    elif verdict:
        print("\n  (dry run — re-run with --apply to write these labels)")

    return 1 if unjudged and a.report else 0


if __name__ == "__main__":
    sys.exit(main())
