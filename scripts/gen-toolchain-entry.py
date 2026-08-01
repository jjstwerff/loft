#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Derive the registry entry for the loft toolchain from a release's own artifacts.

@PLN78 1b.  The toolchain becomes installable and self-updatable by being a package in
the signed registry index like any other -- that is the whole mechanism, and it is why
there is no separate binary signature: `index.json` is signed, the entry this script
emits carries a sha256 per artifact, and every zip below hangs off those hashes.

The reason this is a script and not a paragraph in RELEASE.md: a hand-written entry is
correct exactly once.  The next release changes four hashes, four URLs, a size and a
date, and a hand-copied entry rots silently -- an index that is *signed* but names last
release's hashes fails verification at the user, far from the mistake.  So the entry is
derived from the artifacts, in the same workflow run that produced them.

Hashes are computed from the zips themselves, never read from the `.zip.sha256`
sidecars.  A sidecar is a claim about a file; recording a claim we did not check would
make the signature attest to something nobody verified.

Usage:
    gen-toolchain-entry.py --version 2026.7.2 --dir artifacts [--out entry.json]
"""

import argparse
import hashlib
import json
import pathlib
import re
import sys
import zipfile

REPO = "loft-lang/loft"
SELF_UPDATE_RS = pathlib.Path(__file__).resolve().parent.parent / "src" / "self_update.rs"


def published_triples() -> list[str]:
    """The target triples a release publishes, read from their one home.

    `self_update::PUBLISHED_TRIPLES` is what the running binary matches its host against,
    so a triple that disagrees with it is a triple no loft will ever ask for.  Reading it
    here rather than restating it keeps the entry and the resolver from drifting apart --
    the failure that shape produces is "no build for your platform" on a platform we did
    in fact build for.
    """
    src = SELF_UPDATE_RS.read_text()
    m = re.search(r"PUBLISHED_TRIPLES: &\[&str\] = &\[(.*?)\];", src, re.S)
    if not m:
        sys.exit(f"{SELF_UPDATE_RS}: cannot find PUBLISHED_TRIPLES")
    triples = re.findall(r'"([^"]+)"', m.group(1))
    if not triples:
        sys.exit(f"{SELF_UPDATE_RS}: PUBLISHED_TRIPLES is empty")
    return triples


def manifest_digest(zip_path: pathlib.Path) -> str:
    """sha256 of the `SHA256SUMS` carried inside a release bundle.

    Read out of the zip rather than recomputed from the staged tree: the value has to
    describe what shipped, and the zip is the only thing that definitely did.
    """
    with zipfile.ZipFile(zip_path) as z:
        names = [n for n in z.namelist() if n.rsplit("/", 1)[-1] == "SHA256SUMS"]
        if len(names) != 1:
            sys.exit(
                f"{zip_path}: expected exactly one SHA256SUMS in the bundle, found "
                f"{len(names)} -- cannot say which one an installation should match"
            )
        return hashlib.sha256(z.read(names[0])).hexdigest()


def sha256_of(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", required=True, help="release version, e.g. 2026.7.2")
    ap.add_argument("--dir", required=True, help="directory holding the release zips")
    ap.add_argument("--out", help="write here instead of stdout")
    ap.add_argument("--published", help="RFC3339 timestamp (default: the tag's date)")
    ap.add_argument(
        "--splice-into",
        metavar="INDEX_JSON",
        help="merge the entry into a registry index in place, instead of writing the "
        "entry on its own",
    )
    args = ap.parse_args()

    ver, root = args.version, pathlib.Path(args.dir)
    base = f"https://github.com/{REPO}/releases/download/v{ver}"

    def artifact(name: str) -> pathlib.Path:
        p = root / name
        if not p.is_file():
            # Fail rather than emit a partial entry.  A missing artifact means a build
            # leg did not produce one, and an entry that quietly omits that platform
            # tells its users "not built for you" for a release that simply failed.
            sys.exit(f"missing release artifact: {p}")
        return p

    src_zip = artifact(f"loft-{ver}-src.zip")

    binaries = {}
    for triple in published_triples():
        zp = artifact(f"loft-{ver}-{triple}.zip")
        # No `loft_ffi_fp`: that field gates a prebuilt *cdylib* against the host's
        # loft-ffi ABI.  A toolchain bundle is an executable plus its stdlib and links
        # against nothing of the host's, so a fingerprint here would be a value with no
        # meaning that `loft install` would nonetheless compare.
        binaries[triple] = {
            "url": f"{base}/loft-{ver}-{triple}.zip",
            "sha256": sha256_of(zp),
            # The digest of the bundle's one manifest.  `sha256` above is verifiable
            # exactly once, at download; what the user then runs is an unpacked
            # directory whose manifest ships inside it.  Naming the manifest here is
            # what lets `loft verify-self` trace an INSTALLED tree back to the
            # signature -- one hash over one file, covering every file it lists.
            "manifest_sha256": manifest_digest(zp),
        }

    entry = {
        # The package name is the key this object is stored under, as for every other
        # package in the index -- not a `name` field inside it.
        "description": "The loft toolchain: compiler, interpreter and standard library.",
        "homepage": f"https://github.com/{REPO}",
        "categories": ["toolchain"],
        "yanked": [],
        "versions": {
            ver: {
                # The version-level artifact is the SOURCE archive, so a toolchain entry
                # means what a package entry means -- source, plus prebuilt binaries per
                # target.  `binaries` is what an install or a self-update actually fetches.
                "url": f"{base}/loft-{ver}-src.zip",
                "sha256": sha256_of(src_zip),
                "size": src_zip.stat().st_size,
                # Deliberately permissive.  This constrains which *loft* may install the
                # entry, and the loft that most needs a newer toolchain is an old one.
                "loft": ">=0",
                "published": args.published or f"{ver[:4]}-01-01T00:00:00Z",
                "binaries": binaries,
            }
        },
    }

    if args.splice_into:
        splice(pathlib.Path(args.splice_into), entry, ver)
        return

    out = json.dumps({"loft": entry}, indent=2, sort_keys=False) + "\n"
    if args.out:
        pathlib.Path(args.out).write_text(out)
    else:
        sys.stdout.write(out)


def splice(index_path: pathlib.Path, entry: dict, ver: str) -> None:
    """Merge the entry into a registry index in place.

    A script rather than a hand-edit for one reason that is easy to get wrong by hand:
    the versions map ADDS.  A toolchain entry pasted over the previous one drops every
    release before it, and the users it strands are exactly the ones on an old version
    -- `find_best_version` would still resolve, so nothing fails loudly; those releases
    just quietly cease to exist.  Refusing to overwrite an existing version is the other
    half: a re-submitted version with different hashes is either a rebuild or tampering,
    and neither should land silently.
    """
    index = json.loads(index_path.read_text())
    packages = index.setdefault("packages", {})
    existing = packages.get("loft")
    if existing:
        prior = existing.get("versions", {})
        if ver in prior:
            sys.exit(
                f"{index_path}: `loft` {ver} is already in the index.  Re-publishing a "
                f"version is not a splice -- if this is a rebuild, yank and use a new "
                f"version number."
            )
        entry["versions"] = {**prior, **entry["versions"]}
        # `yanked` is the maintainer's, not ours to reset.
        entry["yanked"] = existing.get("yanked", [])
    packages["loft"] = entry
    index["updated"] = entry["versions"][ver]["published"]
    # `ensure_ascii=False`, because this file gets SIGNED after a human reads the
    # diff.  Python's default escapes every non-ASCII character, and the registry
    # index is full of them -- em dashes in package descriptions and API docs.  The
    # default rewrote all 33 other packages into `\uXXXX` and turned a one-package
    # addition into a 533-line diff: semantically identical, and unreviewable, which
    # for the one artifact whose trust root IS the maintainer's look at what changed
    # is the property that matters.  Round-trips byte-identically now, so the diff
    # is the entry and nothing else.
    index_path.write_text(json.dumps(index, indent=2, ensure_ascii=False) + "\n")
    kept = len(entry["versions"]) - 1
    print(f"spliced loft {ver} into {index_path} ({kept} earlier version(s) kept)")


if __name__ == "__main__":
    main()
