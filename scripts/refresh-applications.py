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

# Curated showcase set. Each entry is either FETCHED (has "fetch": owner/repo — metadata
# pulled live from GitHub) or EXPLICIT (an in-repo demo with no standalone repo — metadata
# given here). Editorial: a good example of HOW to build something with loft, not every
# experiment. `origin`: first-party (ours) / community (accepted `application_showcase`).
APPS: list[dict] = [
    {"fetch": "jjstwerff/moros", "origin": "first-party", "demonstrates": "a full RPG — game systems, world state, rendering"},
    {"fetch": "jjstwerff/dryopea", "origin": "first-party", "demonstrates": "a 3D free-build / tower-defence game on the lavition engine"},
    {"fetch": "jjstwerff/crawler", "origin": "first-party", "demonstrates": "a hex roguelike with a renderer-agnostic kernel — the first cross-game consumer of the hex_* world libraries"},
    {"fetch": "jjstwerff/routing", "origin": "first-party", "demonstrates": "a phone-first WASM app — loft→WASM, interactive canvas, GPX export"},
    {"fetch": "jjstwerff/ssh_home", "origin": "first-party", "demonstrates": "a native terminal app — SSH + PTY, phone-width tmux, no browser"},
    # In-repo demos (no standalone repo) — explicit metadata + a live GitHub Pages demo.
    {
        "name": "Crystal Editor",
        "origin": "first-party",
        "demonstrates": "generative art in the browser — loft→WASM, an interactive editor, real-time GL rendering",
        "description": "The audience 'crystal' generative-art editor — a self-contained browser demo (tools/audience-demo/, built on lib/audience_crystal).",
        "url": "https://github.com/loft-lang/loft/tree/main/tools/audience-demo",
        "homepage": "https://loft-lang.org/loft/crystal-editor.html",
    },
    {
        "name": "Brick Buster",
        "origin": "first-party",
        "demonstrates": "a complete browser game in loft — loft→WASM, a canvas game loop, input, and power-ups",
        "description": "A Breakout-style brick-breaker — a self-contained, double-click-to-play browser game demo (tools/brick-buster/).",
        "url": "https://github.com/loft-lang/loft/tree/main/tools/brick-buster",
        "homepage": "https://loft-lang.org/loft/brick-buster.html",
    },
    {
        "name": "GL demo gallery",
        "origin": "first-party",
        "demonstrates": "the graphics / GL API by example — a browser gallery of short loft programs (2D canvas, 3D, shaders) that run live",
        "description": "The gallery — one page collecting the loft GL/graphics demos, each a short program that runs live in the browser via WASM. A single entry point to the whole demo collection.",
        "url": "https://github.com/loft-lang/loft/blob/main/doc/gallery-examples.js",
        "homepage": "https://loft-lang.org/loft/gallery.html",
    },
]

# Documented but NOT rendered — tracked here until they are public / ready to showcase.
# (Kept in-project per the owner's ask: "a bit of work to go to be made public".)
PENDING: list[tuple[str, str]] = [
    (
        "jjstwerff/zero-trust-shared-files",
        "zero-trust shared-file system — multi-server federation, signed-plugin collaborative "
        "editing, end-to-end crypto (crypto/cbor/wasm). Private until further along; then flip "
        "to a first-party showcase.",
    ),
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
    for e in APPS:
        if "fetch" in e:  # external repo — pull metadata live
            repo = e["fetch"]
            m = gh_view(repo)
            if not m:
                sys.stderr.write(f"refresh-applications: could not read {repo} (gh) — skipped\n")
                continue
            apps.append({
                "repo": repo,
                "name": m.get("name") or repo.split("/")[-1],
                "origin": e["origin"],
                "demonstrates": e["demonstrates"],
                "description": (m.get("description") or "").strip(),
                "url": m.get("url") or f"https://github.com/{repo}",
                "homepage": (m.get("homepageUrl") or "").strip(),
                "pushed": (m.get("pushedAt") or "")[:10],
            })
        else:  # in-repo demo — explicit metadata, no fetch
            apps.append({
                "repo": e.get("url", ""),
                "name": e["name"],
                "origin": e["origin"],
                "demonstrates": e["demonstrates"],
                "description": (e.get("description") or "").strip(),
                "url": e.get("url", ""),
                "homepage": (e.get("homepage") or "").strip(),
                "pushed": "",
            })
    if not apps:
        sys.stderr.write("refresh-applications: no applications resolved\n")
        return 1
    apps.sort(key=lambda a: (a["origin"] != "first-party", a["name"].lower()))  # first-party first, then by name
    OUT.write_text(
        json.dumps({"applications": apps}, indent=2, sort_keys=True, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"refresh-applications: wrote {OUT} ({len(apps)} applications)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
