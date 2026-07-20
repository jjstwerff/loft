#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN112 — build doc/claude/applications-snapshot.json: the major APPLICATIONS built with
# loft, for the "state of the loft distribution" doc. These are NOT part of the distribution
# (not installed/fetched as components) — they are reference EXAMPLES so customers can see
# how to build such apps. The SET is editorial (curated below); each app's metadata
# (description, demo link, last activity) is fetched LIVE the same automated way, so it
# self-maintains.
#
# `origin` distinguishes `first-party` (ours) from `community` (contributor showcases). The
# community intake is the `application_showcase` GitHub issue (label `showcase`); an ACCEPTED
# submission is added to the curated list below with origin="community".
#
# Usage:  scripts/refresh-applications.py
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "doc" / "claude" / "applications-snapshot.json"

# Curated showcase set: (owner/repo, origin, what it demonstrates). Editorial — a good
# example of HOW to build something with loft, not every experiment. Add community apps
# here with origin="community".
APPS: list[tuple[str, str, str]] = [
    ("jjstwerff/moros", "first-party", "a full RPG — game systems, world state, rendering"),
    ("jjstwerff/dryopea", "first-party", "a 3D free-build / tower-defence game on the lavition engine"),
    ("jjstwerff/crawler", "first-party", "a hex roguelike with a renderer-agnostic kernel — the first cross-game consumer of the hex_* world libraries"),
    ("jjstwerff/routing", "first-party", "a phone-first WASM app — loft→WASM, interactive canvas, GPX export"),
    ("jjstwerff/ssh_home", "first-party", "a native terminal app — SSH + PTY, phone-width tmux, no browser"),
]


def gh_view(repo: str) -> dict:
    r = subprocess.run(
        ["gh", "repo", "view", repo, "--json", "name,description,url,homepageUrl,pushedAt"],
        capture_output=True, text=True,
    )
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return {}


def main() -> int:
    apps = []
    for repo, origin, demonstrates in APPS:
        m = gh_view(repo)
        if not m:
            sys.stderr.write(f"refresh-applications: could not read {repo} (gh) — skipped\n")
            continue
        apps.append({
            "repo": repo,
            "name": m.get("name") or repo.split("/")[-1],
            "origin": origin,
            "demonstrates": demonstrates,
            "description": (m.get("description") or "").strip(),
            "url": m.get("url") or f"https://github.com/{repo}",
            "homepage": (m.get("homepageUrl") or "").strip(),
            "pushed": (m.get("pushedAt") or "")[:10],
        })
    if not apps:
        sys.stderr.write("refresh-applications: no applications resolved\n")
        return 1
    apps.sort(key=lambda a: (a["origin"] != "first-party", a["repo"]))  # first-party first, then by repo
    OUT.write_text(
        json.dumps({"applications": apps}, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"refresh-applications: wrote {OUT} ({len(apps)} applications)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
