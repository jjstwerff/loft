#!/usr/bin/env python3
"""Compose the nextest filterset for a CI test leg.

Two callers needed the same expression and would otherwise each spell it: the
unsharded push/nightly matrix, and the two-way sharded PR path.  A filter
duplicated across workflow steps drifts silently — a test excluded on one leg
and not the other reads as a flake — so it is built here, once.

Usage:  ci_test_filter.py <event_name> [heavy|rest]

The optional shard restricts the leg to the `heavy-serial` test group, or to
its complement.  The group's membership is NOT repeated here: it is read out of
`.config/nextest.toml`, which is where the group is defined and where nextest
itself reads it from.  (nextest has no `test_group()` filterset predicate as of
0.9.138 — checked — so a shard boundary has to be spelled as binaries, and this
is what keeps that spelling honest.)
"""

import sys
import tomllib
from pathlib import Path

NEXTEST_TOML = Path(__file__).resolve().parent.parent / ".config" / "nextest.toml"

# `index_hygiene` and `viewer_markdown` are extracted to their own advisory ubuntu
# jobs (and the nightly): both are platform-independent — a whole-repo doc-link
# check, and the markdown renderer's HTML output — so there is no value running
# them 3x in the required matrix.  `viewer_markdown` is also an HTTP smoke test
# whose interpreted viewer gets starved by this suite's parallel native-build load
# (empty response under contention), so it runs ISOLATED in the `viewer-smoke` job
# where it's reliable.
BASE = ["not binary(index_hygiene)", "not binary(viewer_markdown)"]

# The Chrome + SwiftShader browser-render tests (html_render, and the headless-page
# asyncify resume in html_asyncify) are GPU/headless-browser FLAKY — they gate the
# PR path with noise for a layer (WebGL/shader) that rarely regresses
# independently.  Keep them OFF the per-PR run; they run nightly (like the
# differential oracle).  The DETERMINISTIC, node-based html_wasm instantiate-probe
# (catches the LinkError / import-mismatch class) STAYS on the PR path — it does
# not flake.
#
# The two exhaustive stdlib round-trips are the single biggest cost in the suite
# AND its floor: 379s + 375s on macOS (308s + 308s on ubuntu), ~65% of the
# `ir_schema_roundtrip` binary and ~11% of ALL per-test time.  A parallel suite
# cannot finish faster than its slowest single test.  What they verify is FORMAT
# STABILITY: the whole parsed stdlib survives serialise->deserialise
# byte-identical.  That breaks when the IR schema or the serialiser changes —
# rare, and always deliberate.  The cheap canary `tests_scripts_round_trip` (83s)
# STAYS on the PR path and still fails if round-tripping breaks at all; the
# exhaustive pair runs on push-to-main and nightly, so a schema change is caught
# the same day, by the run that is nobody's inner loop.
PR_ONLY = [
    "not binary(html_render)",
    "not binary(html_asyncify)",
    "not test(stdlib_load_compares_equal_to_fresh)",
    "not test(stdlib_whole_data_round_trip)",
]


def group_filter(group: str) -> str:
    """The filterset nextest itself uses to populate `group`."""
    with NEXTEST_TOML.open("rb") as fh:
        config = tomllib.load(fh)
    for profile in ("ci", "default"):
        for override in config["profile"].get(profile, {}).get("overrides", []):
            if override.get("test-group") == group:
                return override["filter"]
    raise SystemExit(f"no override defines test-group '{group}' in {NEXTEST_TOML}")


def main() -> None:
    if not 2 <= len(sys.argv) <= 3:
        raise SystemExit(__doc__)
    event, shard = sys.argv[1], (sys.argv[2] if len(sys.argv) == 3 else None)

    clauses = list(BASE)
    if event == "pull_request":
        clauses += PR_ONLY
    if shard == "heavy":
        clauses.append(f"({group_filter('heavy-serial')})")
    elif shard == "rest":
        clauses.append(f"not ({group_filter('heavy-serial')})")
    elif shard is not None:
        raise SystemExit(f"unknown shard '{shard}' (expected 'heavy' or 'rest')")

    print(" and ".join(clauses))


if __name__ == "__main__":
    main()
