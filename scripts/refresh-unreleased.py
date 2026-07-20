#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN112 phase 2 — build doc/claude/unreleased-snapshot.json: for every registry
# library, its `origin/main` public API (the `unreleased` tier), extracted the SAME way
# `loft api` does (`pkg_api_items`).  The committed snapshot makes the catalogue's
# unreleased tier deterministic + CI-checkable (the generator renders from it; no network
# at --check time).
#
# CONTENT-ADDRESSED CACHE (the "check cheap, reuse the rest" invariant): each lib is keyed
# by its `origin/main` sub-path commit sha.  A cheap one-line sha check per lib; if the sha
# is unchanged since the committed snapshot, the entry is REUSED unchanged — no fetch, no
# extract.  A matching sha is a PROOF the source is identical (not a guess), because git
# shas are content-addressed.  There is no local clone to go stale — every read is `gh`
# against the authoritative ref.
#
# Usage:  scripts/refresh-unreleased.py            # refresh all libs
#         scripts/refresh-unreleased.py <name...>  # only these libs (still writes all)
from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
INDEX = REPO / "doc" / "claude" / "registry-index-snapshot.json"
OUT = REPO / "doc" / "claude" / "unreleased-snapshot.json"
LOFT = REPO / "target" / "release" / "loft"


def gh(args: list[str]) -> str:
    r = subprocess.run(["gh", "api", *args], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


def repo_subpath(homepage: str) -> tuple[str, str]:
    """github.com/<owner>/<repo>/tree/main/<subpath> -> (owner/repo, subpath)."""
    rest = homepage.removeprefix("https://github.com/")
    owner_repo = rest.split("/tree/", 1)[0]
    subpath = rest.split("/tree/main/", 1)[1] if "/tree/main/" in rest else ""
    return owner_repo, subpath


def origin_main_sha(owner_repo: str, subpath: str) -> str:
    """The sha of the last commit touching this lib's sub-path on the default branch."""
    return gh([f"repos/{owner_repo}/commits?path={subpath}&per_page=1", "--jq", ".[0].sha"]).strip()


def extract_api(owner_repo: str, subpath: str) -> list[dict]:
    """Fetch the lib's src/*.loft from origin/main and run `loft api --json` on it."""
    srcdir = f"{subpath}/src" if subpath else "src"
    names = [n for n in gh([f"repos/{owner_repo}/contents/{srcdir}", "--jq", ".[].name"]).splitlines() if n.endswith(".loft")]
    if not names:
        return []
    with tempfile.TemporaryDirectory() as td:
        os.makedirs(f"{td}/src", exist_ok=True)
        for n in names:
            raw = gh([f"repos/{owner_repo}/contents/{srcdir}/{n}", "-H", "Accept: application/vnd.github.raw"])
            Path(f"{td}/src/{n}").write_text(raw, encoding="utf-8")
        Path(f"{td}/loft.toml").write_text('name = "probe"\nversion = "0.0.0"\n', encoding="utf-8")
        r = subprocess.run([str(LOFT), "api", td, "--json"], capture_output=True, text=True)
        try:
            return json.loads(r.stdout)
        except json.JSONDecodeError:
            return []


def main() -> int:
    only = set(sys.argv[1:])
    index = json.loads(INDEX.read_text(encoding="utf-8"))
    prior = json.loads(OUT.read_text(encoding="utf-8")) if OUT.exists() else {}
    result: dict[str, dict] = {}
    for name, pkg in sorted(index.get("packages", {}).items()):
        owner_repo, subpath = repo_subpath((pkg.get("homepage") or "").strip())
        if not owner_repo:
            continue
        if only and name not in only:
            if name in prior:  # keep others untouched when refreshing a subset
                result[name] = prior[name]
            continue
        sha = origin_main_sha(owner_repo, subpath)
        if not sha:
            continue
        if prior.get(name, {}).get("sha") == sha:
            result[name] = prior[name]  # not stale — reuse (no fetch, no extract)
            sys.stderr.write(f"  {name}: reuse ({sha[:7]})\n")
            continue
        api = extract_api(owner_repo, subpath)
        result[name] = {"sha": sha, "api": api}
        sys.stderr.write(f"  {name}: fetched ({sha[:7]}, {len(api)} sigs)\n")
    OUT.write_text(json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"refresh-unreleased: wrote {OUT} ({len(result)} libs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
