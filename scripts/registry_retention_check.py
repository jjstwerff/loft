#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Guard the registry's retention promise: a version that was once published must
# stay resolvable forever.
#
# PKG_REGISTRY.md promises that yanked versions stay LISTED, so a consumer whose
# `loft.lock` pins one keeps resolving.  Nothing enforced it, and the promise was
# broken exactly once: `web 0.2.2` was yanked correctly on 2026-07-03 and its
# version block was deleted from `index.json` in the very next commit — a
# "sign: commit index.json + regenerate index.json.sig" whose only index change was
# that deletion.  Nobody decided to remove it.  By the time it was noticed the
# release asset and the `web-v0.2.2` tag were gone too, so it could not be restored:
# there was no source to repackage, and republishing a rebuild that did not
# reproduce sha256 59518a5… would substitute different code under a version
# consumers may trust.  Its one consumer, `routing`, survived by vendoring the
# source.  That loss is permanent, which is why this is a guard and not a repair.
#
# Two halves, because a version can stop resolving in two independent ways and
# neither check sees the other's failure:
#
#   history  — no version ever LEAVES index.json.  Walks every commit that touched
#              index.json and reports any package version present in one revision
#              and absent in the next.  This is the half that would have caught the
#              web 0.2.2 deletion the same night.
#   artifact — every version LISTED today actually downloads.  A one-byte ranged
#              GET per URL.  This is the half that catches an erased release asset,
#              where the index still promises something the internet no longer has.
#
# An entry in EXEMPT is a loss that is already permanent: recorded with its reason,
# printed on every run, and never silently skipped.  Adding one is an admission,
# not a fix — the only correct reason is that the artifact is unrecoverable.
#
# Usage:
#   scripts/registry_retention_check.py [--repo <url-or-path>] [--work <dir>]
#                                       [--skip-artifacts] [--json OUT]
#
# Exit status: 0 = retention intact; 1 = a version was dropped or no longer
# resolves; 2 = the check could not run (clone failed, index unparseable).  The
# last is distinct because "the guard is broken" must never read as "the registry
# is fine" — a green that means nothing is how the vacuous-evidence failures in
# this repo have always started.

import argparse
import json
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_REPO = "https://github.com/loft-lang/registry.git"

# Versions whose absence is accepted, with the reason.  Keyed (package, version).
#
# NEVER add an entry to quiet a red run.  A dropped version is normally REPAIRABLE
# — re-add the block to index.json, since the tarball and its sha256 are still in
# the history this check just read.  An exemption is only for a loss that repair
# cannot reach, and it permanently narrows what the registry promises.
EXEMPT = {
    ("web", "0.2.2"): (
        "Dropped by registry d8ff94c (2026-07-03) as collateral of a signing commit. "
        "Unrecoverable: the loft-libs-net release asset 404s and no web-v0.2.2 tag "
        "exists, so there is no source to repackage and no rebuild can reproduce "
        "sha256 59518a56…  Its consumer `routing` vendored the source instead. "
        "See doc/claude/plans/library-compat-contract/README.md step 0."
    ),
}


def die(msg: str) -> "NoReturn":  # noqa: F821 - forward ref for typing only
    print(f"registry-retention: CANNOT RUN — {msg}", file=sys.stderr)
    sys.exit(2)


def git(repo: Path, *args: str) -> str:
    """Run git in `repo`, returning stdout.  A git failure means the check cannot
    run rather than that the registry is bad, so it exits 2 via `die`."""
    proc = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=False
    )
    if proc.returncode != 0:
        die(f"git {' '.join(args)} failed: {proc.stderr.strip()}")
    return proc.stdout


def version_sets(raw: str) -> "dict[str, set[str]] | None":
    """Map package name → set of listed versions for one revision of index.json.

    Returns None when the revision does not parse.  Unparseable revisions are
    SKIPPED rather than treated as an empty index: a parse failure would otherwise
    read as "every version was dropped", burying a real drop under 2,700 false ones.
    """
    try:
        idx = json.loads(raw)
    except (json.JSONDecodeError, UnicodeDecodeError):
        return None
    packages = idx.get("packages") or {}
    if not isinstance(packages, dict):
        return None
    out = {}
    for name, pkg in packages.items():
        versions = (pkg or {}).get("versions") or {}
        out[name] = set(versions.keys()) if isinstance(versions, dict) else set()
    return out


def check_history(repo: Path) -> "tuple[list[dict], int, int]":
    """Walk index.json's history oldest-first and report versions that disappeared.

    Reports at the revision where the version went missing, not where it was added,
    because that revision is the one whose author can explain it.
    """
    log = git(repo, "log", "--reverse", "--format=%h%x09%ad%x09%s", "--date=short",
              "--", "index.json").strip()
    if not log:
        die("no commits touch index.json — wrong repository?")
    revisions = [line.split("\t", 2) for line in log.split("\n")]

    drops, unparseable = [], 0
    prev, prev_sha = None, None
    for sha, date, subject in revisions:
        current = version_sets(git(repo, "show", f"{sha}:index.json"))
        if current is None:
            unparseable += 1
            continue
        if prev is not None:
            for name in sorted(prev):
                for version in sorted(prev[name] - current.get(name, set())):
                    drops.append({
                        "package": name,
                        "version": version,
                        "last_seen": prev_sha,
                        "dropped_in": sha,
                        "date": date,
                        "subject": subject,
                    })
        prev, prev_sha = current, sha
    return drops, len(revisions), unparseable


def artifact_resolves(url: str, timeout: int) -> "str | None":
    """None when the artifact downloads, else a one-line reason.

    Asks for the first byte rather than issuing a HEAD: GitHub serves release assets
    via a redirect to a signed object URL, and a HEAD against that path is not a
    reliable stand-in for what `loft install` actually does.
    """
    request = urllib.request.Request(url, headers={"Range": "bytes=0-0"})
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            code = response.getcode()
            return None if code in (200, 206) else f"HTTP {code}"
    except urllib.error.HTTPError as e:
        return f"HTTP {e.code}"
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        return f"unreachable ({e})"


def check_artifacts(repo: Path, timeout: int) -> "tuple[list[dict], int]":
    """Every version listed in the CURRENT index must still download."""
    index_path = repo / "index.json"
    try:
        idx = json.loads(index_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as e:
        die(f"current index.json is unreadable: {e}")

    listed = [
        (name, version, (entry or {}).get("url", ""))
        for name, pkg in sorted((idx.get("packages") or {}).items())
        for version, entry in sorted(((pkg or {}).get("versions") or {}).items())
    ]
    broken = []
    for name, version, url in listed:
        reason = "no url in the index entry" if not url else artifact_resolves(url, timeout)
        if reason:
            broken.append({"package": name, "version": version, "url": url, "reason": reason})
    return broken, len(listed)


def main() -> int:
    parser = argparse.ArgumentParser(description="Guard the registry's retention promise.")
    parser.add_argument("--repo", default=DEFAULT_REPO,
                        help="registry repo URL, or a path to an existing clone")
    parser.add_argument("--work", help="directory to clone into (default: a temp dir)")
    parser.add_argument("--skip-artifacts", action="store_true",
                        help="run the history half only (no network per version)")
    parser.add_argument("--timeout", type=int, default=30, help="per-artifact seconds")
    parser.add_argument("--json", help="write the full report here")
    args = parser.parse_args()

    with tempfile.TemporaryDirectory() as tmp:
        source = Path(args.repo)
        if source.is_dir():
            repo = source
        else:
            repo = Path(args.work) if args.work else Path(tmp) / "registry"
            if not (repo / ".git").is_dir():
                # Full history: the history half reads index.json at every revision,
                # so a shallow clone would silently shrink the window it can see.
                proc = subprocess.run(["git", "clone", "--quiet", args.repo, str(repo)],
                                      capture_output=True, text=True, check=False)
                if proc.returncode != 0:
                    die(f"clone of {args.repo} failed: {proc.stderr.strip()}")

        drops, revisions, unparseable = check_history(repo)
        broken, listed = ([], 0) if args.skip_artifacts else check_artifacts(repo, args.timeout)

        exempted = [d for d in drops if (d["package"], d["version"]) in EXEMPT]
        drops = [d for d in drops if (d["package"], d["version"]) not in EXEMPT]

        scope = "history only" if args.skip_artifacts else f"{listed} versions listed"
        print(f"registry-retention: {revisions} index.json revisions, {scope}")
        if unparseable:
            print(f"registry-retention: {unparseable} revision(s) did not parse and were skipped")

        # Printed on every run, green or red: an accepted loss that stops being
        # mentioned is one nobody remembers is still owed.
        for e in exempted:
            print(f"registry-retention: EXEMPT {e['package']} {e['version']} "
                  f"(dropped in {e['dropped_in']}, {e['date']})")
            print(f"    {EXEMPT[(e['package'], e['version'])]}")

        for d in drops:
            print(f"registry-retention: DROPPED {d['package']} {d['version']} — listed at "
                  f"{d['last_seen']}, gone in {d['dropped_in']} ({d['date']}: {d['subject']})",
                  file=sys.stderr)
        for b in broken:
            print(f"registry-retention: UNRESOLVABLE {b['package']} {b['version']} — "
                  f"{b['reason']} — {b['url']}", file=sys.stderr)

        if args.json:
            Path(args.json).write_text(json.dumps({
                "revisions": revisions,
                "listed": listed,
                "unparseable_revisions": unparseable,
                "dropped": drops,
                "unresolvable": broken,
                "exempt": [{**e, "reason": EXEMPT[(e["package"], e["version"])]} for e in exempted],
            }, indent=2) + "\n", encoding="utf-8")

        if drops or broken:
            print(f"\nregistry-retention: FAIL — {len(drops)} dropped, "
                  f"{len(broken)} unresolvable.", file=sys.stderr)
            if drops:
                # The tarball bytes and sha256 are still in the history this check
                # just read, so a drop caught promptly costs one revert.
                print("  A dropped version is repairable: restore its block in "
                      "index.json from the revision named above.  Exempt it only if "
                      "the artifact itself is gone.", file=sys.stderr)
            return 1

        # The pass message names only what ran: with --skip-artifacts nothing was
        # downloaded, and a green claiming otherwise is the vacuous evidence this
        # guard exists to prevent.
        checked = "is still listed" if args.skip_artifacts else "is still listed and resolves"
        print(f"registry-retention: OK — every published version {checked}")
        return 0


if __name__ == "__main__":
    sys.exit(main())
