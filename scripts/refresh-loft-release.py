#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN112 — build doc/claude/loft-release-snapshot.json: loft ITSELF (the language binary)
# for the catalogue's top-level overview, so LIBRARIES.md shows loft AND the libs in one go,
# gathered the SAME automated way lib info is. loft is not yet a registry entry (RELEASE.md:
# the registry release-entry is still `[build]`), so — until it is — the authoritative source
# is loft's own GitHub RELEASE: each `loft-<version>-<triple>.zip` carries a `.zip.sha256`
# sidecar. When loft lands in the registry proper, flip the source here to the registry index;
# the snapshot shape + the generator's overview render stay the same.
#
# CONTENT-ADDRESSED (the "check cheap, reuse the rest" invariant): keyed by the release TAG.
# If the committed snapshot's tag == the latest published tag, REUSE it unchanged — no fetch.
#
# Usage:  scripts/refresh-loft-release.py
from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "doc" / "claude" / "loft-release-snapshot.json"
GH_REPO = "loft-lang/loft"


def gh(args: list[str]) -> str:
    r = subprocess.run(["gh", *args], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


def latest_release() -> dict:
    """The latest PUBLISHED (non-draft) release: {tagName, publishedAt, assets:[{name,size,url}]}."""
    out = gh(["release", "view", "-R", GH_REPO, "--json", "tagName,publishedAt,isDraft,assets"])
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return {}


def target_of(asset_name: str, version: str) -> str:
    """`loft-<version>-<triple>.zip` -> `<triple>`."""
    return asset_name.removeprefix(f"loft-{version}-").removesuffix(".zip")


def sha_map(tag: str) -> dict[str, str]:
    """Download the `.zip.sha256` sidecars and parse `<sha>  <file>` -> {filename: sha}."""
    out: dict[str, str] = {}
    with tempfile.TemporaryDirectory() as td:
        gh(["release", "download", tag, "-R", GH_REPO, "--pattern", "*.zip.sha256", "--dir", td, "--clobber"])
        for p in Path(td).glob("*.zip.sha256"):
            txt = p.read_text(encoding="utf-8").strip()
            if txt:
                out[p.name.removesuffix(".sha256")] = txt.split()[0]
    return out


def main() -> int:
    rel = latest_release()
    tag = (rel.get("tagName") or "").strip()
    if not tag:
        sys.stderr.write("refresh-loft-release: could not read the latest loft release (gh)\n")
        return 1
    version = tag.lstrip("v")

    # Content-addressed reuse: same tag as the committed snapshot -> nothing to do.
    if OUT.exists():
        try:
            if json.loads(OUT.read_text(encoding="utf-8")).get("tag") == tag:
                print(f"refresh-loft-release: {tag} unchanged — reused")
                return 0
        except json.JSONDecodeError:
            pass

    shas = sha_map(tag)
    targets: dict[str, dict] = {}
    for a in rel.get("assets", []):
        name = a.get("name", "")
        if name.startswith(f"loft-{version}-") and name.endswith(".zip"):
            targets[target_of(name, version)] = {
                "url": a.get("url", ""),
                "sha256": shas.get(name, ""),
                "size": a.get("size", 0),
            }
    if not targets:
        sys.stderr.write(f"refresh-loft-release: {tag} has no loft-*-*.zip assets\n")
        return 1

    snapshot = {"version": version, "tag": tag, "published": rel.get("publishedAt", ""), "targets": targets}
    OUT.write_text(json.dumps(snapshot, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"refresh-loft-release: wrote {OUT} ({tag}, {len(targets)} targets)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
