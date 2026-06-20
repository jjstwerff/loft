#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later

"""PKG.REG R9 — `loft-lang/registry` PR validation.

Drop this file at `tools/validate.py` in the `loft-lang/registry`
repo.  Wired in by `.github/workflows/pr-validate.yml` (also in this
template directory).

Four gates per PR:

1. **Schema lint** — every package + version row has the required
   fields, types match, schema_version is unchanged.
2. **Tarball verify** — download every newly-added `versions.<v>.url`,
   hash it, compare to the PR's declared sha256.  Reject on
   mismatch.  Caught: publisher pasted wrong hash, tarball was
   re-uploaded after PR opened, opportunistic supply-chain swap.
3. **Reproducible-build re-check** — for each newly-added version
   whose homepage points at a public GitHub repo, clone the tag,
   run `loft package`, compare the produced sha256 to the PR's
   claim.  Caught: source repo's tag points at different bytes
   than the uploaded release tarball.
4. **Trigger uniqueness** — every Tier-1 `method:receiver` trigger
   must be owned by at most one package across the whole registry.
   A consumer auto-loads a library from a bare `obj.method()` call,
   so two packages claiming `text.matches` would make the auto-load
   ambiguous.  Caught: a new library claiming a method-on-type
   trigger another package already owns.

Exits 0 on all-pass; non-zero with line-prefixed errors on any
failure.  The workflow surfaces those lines as PR comments.
"""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

INDEX_PATH = Path("index.json")
SCHEMA_VERSION = 1


def fail(msg: str) -> None:
    print(f"::error::{msg}")
    sys.exit(1)


def load_index() -> dict:
    if not INDEX_PATH.exists():
        fail(f"{INDEX_PATH} not found")
    with INDEX_PATH.open(encoding="utf-8") as f:
        try:
            return json.load(f)
        except json.JSONDecodeError as e:
            fail(f"{INDEX_PATH}: invalid JSON: {e}")


def load_previous_index() -> dict:
    """Read the version of index.json on `main` so we can find the
    NEW entries (ones the PR adds)."""
    try:
        out = subprocess.check_output(
            ["git", "show", "origin/main:index.json"], text=True
        )
        return json.loads(out)
    except subprocess.CalledProcessError:
        # First publish — main may not have index.json yet.
        return {"schema_version": SCHEMA_VERSION, "packages": {}}


def gate_schema(idx: dict) -> None:
    if idx.get("schema_version") != SCHEMA_VERSION:
        fail(
            f"schema_version must be {SCHEMA_VERSION}, "
            f"got {idx.get('schema_version')!r}"
        )
    if "packages" not in idx or not isinstance(idx["packages"], dict):
        fail("`packages` must be an object")
    for name, pkg in idx["packages"].items():
        if "versions" not in pkg or not isinstance(pkg["versions"], dict):
            fail(f"package `{name}`: missing or non-object `versions`")
        for ver, vobj in pkg["versions"].items():
            for required in ("url", "sha256", "size", "loft", "published"):
                if required not in vobj:
                    fail(f"`{name}` v{ver}: missing required field `{required}`")
            if not isinstance(vobj["sha256"], str) or len(vobj["sha256"]) != 64:
                fail(f"`{name}` v{ver}: sha256 must be 64-char hex")
            if not isinstance(vobj["size"], int) or vobj["size"] <= 0:
                fail(f"`{name}` v{ver}: size must be a positive integer")


def gate_tarball_verify(idx: dict, prev: dict) -> None:
    new_entries = _new_entries(idx, prev)
    for name, ver, vobj in new_entries:
        print(f"[verify] downloading {name} v{ver} from {vobj['url']}")
        try:
            with urllib.request.urlopen(vobj["url"], timeout=60) as resp:
                data = resp.read()
        except Exception as e:  # noqa: BLE001 — surface any failure
            fail(f"`{name}` v{ver}: download failed: {e}")
        actual_sha = hashlib.sha256(data).hexdigest()
        if actual_sha.lower() != vobj["sha256"].lower():
            fail(
                f"`{name}` v{ver}: sha256 MISMATCH\n"
                f"  PR claims: {vobj['sha256']}\n"
                f"  actual:    {actual_sha}"
            )
        if len(data) != vobj["size"]:
            fail(
                f"`{name}` v{ver}: size MISMATCH\n"
                f"  PR claims: {vobj['size']} bytes\n"
                f"  actual:    {len(data)} bytes"
            )
        print(f"[verify] {name} v{ver} sha256 + size match")


def gate_reproducible_build(idx: dict, prev: dict) -> None:
    """Clone the homepage repo at the version tag, run `loft package`,
    compare sha256 to the PR's claim.

    Caught here: PR claims hash X, but the source tree at the tag
    produces hash Y when re-packaged.  Either publisher tampered or
    upstream history rewrote the tag.

    Skipped when the package has no `homepage` (private deps,
    third-party-hosted tarballs).  Schema lint always runs; this gate
    is the additional reproducibility check.
    """
    new_entries = _new_entries(idx, prev)
    for name, ver, vobj in new_entries:
        pkg_meta = idx["packages"][name]
        homepage = pkg_meta.get("homepage", "")
        if not homepage or "github.com" not in homepage:
            print(f"[repro] {name} v{ver} — no GitHub homepage, skipping")
            continue
        with tempfile.TemporaryDirectory() as tmp:
            tag = f"v{ver}"
            print(f"[repro] cloning {homepage} @ {tag}")
            try:
                subprocess.check_call(
                    ["git", "clone", "--depth", "1", "--branch", tag, homepage, tmp],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.PIPE,
                )
            except subprocess.CalledProcessError as e:
                fail(
                    f"`{name}` v{ver}: git clone of {homepage}@{tag} failed: "
                    f"{e.stderr.decode(errors='replace') if e.stderr else e}"
                )
            try:
                subprocess.check_call(
                    ["loft", "package"], cwd=tmp, stdout=subprocess.DEVNULL,
                )
            except subprocess.CalledProcessError as e:
                fail(f"`{name}` v{ver}: `loft package` failed: {e}")
            artefact = Path(tmp) / f"{name}-{ver}.tar.gz"
            if not artefact.exists():
                fail(f"`{name}` v{ver}: `loft package` produced no artefact")
            actual = hashlib.sha256(artefact.read_bytes()).hexdigest()
            if actual.lower() != vobj["sha256"].lower():
                fail(
                    f"`{name}` v{ver}: REPRODUCIBLE-BUILD MISMATCH\n"
                    f"  PR claims sha256: {vobj['sha256']}\n"
                    f"  rebuilt from src: {actual}\n"
                    f"  homepage:         {homepage}@{tag}\n"
                    f"  The source repo at {tag} produces a different "
                    f"tarball than the one uploaded to releases.  Either:\n"
                    f"    (a) the GitHub release tarball is stale — re-upload, OR\n"
                    f"    (b) the git tag was force-pushed — investigate.\n"
                )
            print(f"[repro] {name} v{ver} reproduces from source")

            # S7-CI: re-derive the function-level `api` from the cloned source and
            # reject a pasted field that disagrees, so `loft search`'s function
            # discovery can never point at a function the source does not have.
            # The `api` field is AUTO-DERIVED by `loft publish` — it must equal
            # the source, never hand-edited.  (Only checkable where the source is
            # cloned; a package without a GitHub homepage has its `api` trusted-
            # as-pasted, exactly like its sha256.)
            submitted_api = vobj.get("api")
            if submitted_api is not None:
                try:
                    derived = json.loads(
                        subprocess.check_output(
                            ["loft", "api", "--json", tmp], stderr=subprocess.PIPE
                        )
                    )
                except (subprocess.CalledProcessError, json.JSONDecodeError) as e:
                    fail(f"`{name}` v{ver}: `loft api --json` re-derive failed: {e}")
                if derived != submitted_api:
                    fail(
                        f"`{name}` v{ver}: `api` MISMATCH — the pasted `api` field "
                        f"does not match the source at {tag}.\n"
                        f"  The `api` field is auto-derived; re-run `loft publish` "
                        f"and paste the regenerated entry rather than hand-editing it."
                    )
                print(f"[verify] {name} v{ver} api ({len(derived)} fns) matches source")


def gate_trigger_uniqueness(idx: dict) -> None:
    """Every `method:receiver` Tier-1 trigger must be owned by at most one
    package across the whole registry.

    A consumer auto-loads a library from a bare `obj.method()` call, so the
    trigger that maps `matches` -> the providing package must be globally
    unique; two packages claiming `text.matches` would make the auto-load
    ambiguous.  A package re-declaring its own trigger across versions is fine
    — only a *cross-package* collision is rejected.

    Runs over the full index (not just new rows): `main` is assumed clean, so
    any collision present here was introduced by this PR.
    """
    owner: dict[str, str] = {}
    for name, pkg in idx.get("packages", {}).items():
        for vobj in pkg.get("versions", {}).values():
            for trig in vobj.get("triggers", []) or []:
                if not trig:
                    continue
                prior = owner.get(trig)
                if prior is not None and prior != name:
                    fail(
                        f"trigger `{trig}` is claimed by both `{prior}` and "
                        f"`{name}`; a method-on-type trigger must be unique "
                        f"across the registry — rename the method in `{name}` "
                        f"or drop its `[triggers]` opt-in"
                    )
                owner.setdefault(trig, name)
    print("[triggers] all method-on-type triggers are uniquely owned")


def _new_entries(idx: dict, prev: dict) -> list[tuple[str, str, dict]]:
    """Return list of (name, version, version_object) for rows that
    are present in `idx` but not in `prev`."""
    out: list[tuple[str, str, dict]] = []
    for name, pkg in idx.get("packages", {}).items():
        prev_versions: dict = (
            prev.get("packages", {}).get(name, {}).get("versions", {})
        )
        for ver, vobj in pkg.get("versions", {}).items():
            if ver not in prev_versions:
                out.append((name, ver, vobj))
    return out


def main() -> None:
    idx = load_index()
    prev = load_previous_index()
    print("[gate 1] schema lint")
    gate_schema(idx)
    print("[gate 2] tarball sha256 + size verify")
    gate_tarball_verify(idx, prev)
    skip_repro = os.environ.get("LOFT_VALIDATE_SKIP_REPRO") == "1"
    if skip_repro:
        print("[gate 3] reproducible-build re-check — SKIPPED (env)")
    else:
        print("[gate 3] reproducible-build re-check")
        gate_reproducible_build(idx, prev)
    print("[gate 4] trigger uniqueness")
    gate_trigger_uniqueness(idx)
    print("All gates passed.")


if __name__ == "__main__":
    main()
