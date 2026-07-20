#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN112 — build doc/claude/applications-snapshot.json: the major APPLICATIONS built with
# loft, for the "state of the loft distribution" doc. These are NOT part of the distribution
# (not installed as components) — they are reference EXAMPLES so customers can see how to
# build such apps.
#
# SOURCE OF TRUTH = GitHub ISSUES, not a list in this script. Each LISTED showcase (internal
# OR external) is an open issue labeled `showcase` in loft-lang/loft. The `application_showcase`
# issue template files `showcase:pending` submissions (the intake queue); a maintainer promotes
# an accepted one to `showcase`, so raw submissions never auto-render. `origin` is DERIVED from
# the repo owner (jjstwerff / loft-lang = first-party, else community). Each app's metadata is
# read from the issue's form body — adding/removing an app is an issue action, no code change.
#
# Usage:  scripts/refresh-applications.py
from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "doc" / "claude" / "applications-snapshot.json"
ISSUE_REPO = "loft-lang/loft"
LIST_LABEL = "showcase"  # listed (rendered); `showcase:pending` = intake, not rendered
FIRST_PARTY_OWNERS = {"jjstwerff", "loft-lang"}


def gh_issues() -> list[dict]:
    r = subprocess.run(
        ["gh", "issue", "list", "--repo", ISSUE_REPO, "--label", LIST_LABEL, "--state", "open",
         "--limit", "200", "--json", "number,title,body,url"],
        capture_output=True, text=True,
    )
    try:
        return json.loads(r.stdout) or []
    except json.JSONDecodeError:
        return []


def parse_form(body: str):
    """A GitHub issue-FORM body renders each field as `### <label>\\n\\n<value>`. Return a
    getter that looks a field up by heading PREFIX (so `### What it demonstrates (required)`
    matches `What it demonstrates`). `_No response_` (empty optional) reads as ''."""
    sections: dict[str, str] = {}
    cur, buf = None, []
    for line in (body or "").splitlines():
        if line.startswith("### "):
            if cur is not None:
                sections[cur] = "\n".join(buf).strip()
            cur, buf = line[4:].strip(), []
        elif cur is not None:
            buf.append(line)
    if cur is not None:
        sections[cur] = "\n".join(buf).strip()

    def get(prefix: str) -> str:
        for k, v in sections.items():
            if k.lower().startswith(prefix.lower()):
                return "" if v.strip() in ("_No response_", "") else v.strip()
        return ""

    return get


def normalize_repo(field: str) -> tuple[str, str]:
    """`owner/repo` or a full URL -> (url, owner). The owner drives first-party vs community."""
    s = (field or "").strip()
    if not s:
        return "", ""
    if s.startswith("http"):
        m = re.search(r"github\.com/([^/]+)/", s)
        return s, (m.group(1) if m else "")
    return f"https://github.com/{s}", s.split("/", 1)[0]


def main() -> int:
    apps = []
    for iss in gh_issues():
        get = parse_form(iss.get("body") or "")
        demonstrates = get("What it demonstrates")
        if not demonstrates:  # a well-formed showcase issue must say what it demonstrates
            sys.stderr.write(f"refresh-applications: issue #{iss['number']} has no 'What it demonstrates' — skipped\n")
            continue
        url, owner = normalize_repo(get("Public repository"))
        if not url:
            url = iss["url"]
        apps.append({
            "issue": iss["number"],
            "name": get("App name") or iss.get("title", "").removeprefix("[showcase]").strip(),
            "origin": "first-party" if owner in FIRST_PARTY_OWNERS else "community",
            "demonstrates": demonstrates,
            "description": get("One-line summary"),
            "url": url,
            "homepage": get("Live demo"),
        })
    if not apps:
        sys.stderr.write(f"refresh-applications: no `{LIST_LABEL}` issues found in {ISSUE_REPO}\n")
        # Not a hard error — write an empty set so the generator omits the section cleanly.
    apps.sort(key=lambda a: (a["origin"] != "first-party", a["name"].lower()))
    OUT.write_text(
        json.dumps({"applications": apps}, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"refresh-applications: wrote {OUT} ({len(apps)} applications from {ISSUE_REPO} `{LIST_LABEL}` issues)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
