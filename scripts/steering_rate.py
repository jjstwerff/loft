#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""How much did the owner have to push to get the work done right?

The `wa:` labels measure how bad the bugs were for whoever hit them.  This measures
the other side: how much steering the FIXING took.  Both feed the same question — is
loft settling — and neither can answer it alone.

The signal is the owner INTERRUPTING a running turn.  Not every interruption is
steering, though: the owner also interrupts to add a fact they forgot, which says
nothing about the work.  The two are told apart by TIMING, not by wording:

* a short gap after the owner's OWN previous message means they never waited for the
  turn to process — their own flow, an addition or an amendment;
* a long gap means they had been watching the work and stopped it.

That holds because a correction of the agent is uncorrelated with the owner's typing
rhythm, while an amendment to themselves follows it within seconds.  Reading the
messages at each extreme confirms it: under 20s gives "it is merged" and "publish the
3 libs", over 5 minutes gives "The fix is not committed" and "Are you introducing
runtime errors? Remove that immediately".

CAVEAT worth knowing before quoting a number: the gap distribution is UNIMODAL with a
long tail, so the two populations overlap and no threshold is principled.  `--threshold`
therefore shifts the LEVEL.  It does not shift the SHAPE, which is why a trend over
weeks is worth more than any single figure.  Print the histogram (`--histogram`) before
believing a level.

The transcripts live outside the repository, in Claude Code's project directory, and
nothing in the repo preserves them — if they are pruned, the baseline is gone.

Usage:
    scripts/steering_rate.py                     # weekly table, default 60s threshold
    scripts/steering_rate.py --histogram         # gap distribution (density per second)
    scripts/steering_rate.py --threshold 120     # sensitivity: re-run at another cut
    scripts/steering_rate.py --dir <path>        # a transcript directory to read
"""

import argparse
import collections
import datetime
import glob
import json
import os
import sys

INTERRUPT = "[Request interrupted by user]"


def transcript_dir(explicit):
    """The Claude Code transcript directory for this checkout, or `explicit`."""
    if explicit:
        return explicit
    # Claude Code slugifies the project path: /home/u/workspace/loft -> -home-u-workspace-loft
    root = os.path.realpath(os.path.join(os.path.dirname(__file__), ".."))
    return os.path.expanduser("~/.claude/projects/" + root.replace("/", "-"))


def human_messages(directory):
    """Every message the OWNER typed, de-duplicated, oldest first within each session.

    Yields `(session_file, timestamp, text)`.  Tool results, harness-injected
    reminders and command echoes are dropped — they are not the owner speaking.
    A message can appear in more than one session file (a resumed session replays
    its history), so `uuid` de-duplicates; that is ~4% of rows.
    """
    seen = set()
    for path in sorted(glob.glob(os.path.join(directory, "*.jsonl"))):
        rows = []
        with open(path, errors="ignore") as handle:
            for line in handle:
                try:
                    entry = json.loads(line)
                except ValueError:
                    continue
                if entry.get("type") != "user":
                    continue
                content = entry.get("message", {}).get("content")
                stamp = entry.get("timestamp", "")
                uid = entry.get("uuid")
                if isinstance(content, str):
                    text = content
                elif isinstance(content, list):
                    text = " ".join(
                        b.get("text", "")
                        for b in content
                        if isinstance(b, dict) and b.get("type") == "text"
                    )
                else:
                    continue
                text = text.strip()
                if not text or not stamp or uid in seen:
                    continue
                head = text[:60]
                if (
                    text.startswith("<")
                    or text.startswith("Caveat:")
                    or "local-command" in head
                    or "system-reminder" in head
                ):
                    continue
                seen.add(uid)
                rows.append((path, _parse(stamp), text))
        yield from rows


def _parse(stamp):
    try:
        return datetime.datetime.fromisoformat(stamp.replace("Z", "+00:00"))
    except ValueError:
        return None


def collect(directory):
    """`(messages_per_week, interrupts_per_week, [(gap_seconds, week)])`."""
    msgs = collections.Counter()
    interrupts = collections.Counter()
    gaps = []
    previous = {}  # session file -> the owner's previous message time
    for path, when, text in human_messages(directory):
        if when is None:
            continue
        week = when.date().isocalendar()[:2]
        if text == INTERRUPT:
            interrupts[week] += 1
            if path in previous:
                gaps.append(((when - previous[path]).total_seconds(), week))
        else:
            msgs[week] += 1
            previous[path] = when
    return msgs, interrupts, gaps


def histogram(gaps):
    """Gap distribution as DENSITY per second — the bins are unequal, so raw
    counts mislead.  A clean split would show two humps; loft's shows one."""
    edges = [0, 5, 10, 15, 20, 30, 45, 60, 90, 120, 180, 300, 600, 1800]
    print(f"{'bucket':>10} {'n':>5} {'per-sec':>9}")
    for lo, hi in zip(edges, edges[1:]):
        n = sum(1 for g, _ in gaps if lo <= g < hi)
        print(f"{f'{lo}-{hi}s':>10} {n:5d} {n / (hi - lo):9.2f}")
    tail = sum(1 for g, _ in gaps if g >= edges[-1])
    print(f"{'>' + str(edges[-1]) + 's':>10} {tail:5d}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dir", help="transcript directory (default: this checkout's)")
    ap.add_argument("--threshold", type=float, default=60.0,
                    help="gap in seconds above which an interrupt counts as steering")
    ap.add_argument("--histogram", action="store_true", help="print the gap distribution")
    args = ap.parse_args()

    directory = transcript_dir(args.dir)
    if not os.path.isdir(directory):
        print(f"no transcripts at {directory}", file=sys.stderr)
        return 1
    msgs, interrupts, gaps = collect(directory)
    if not msgs:
        print(f"no owner messages found in {directory}", file=sys.stderr)
        return 1

    if args.histogram:
        histogram(gaps)
        print()

    print(f"threshold: interrupts >= {args.threshold:.0f}s after the owner's own message")
    print(f"{'week':10} {'msgs':>6} {'int':>5} {'int/100':>8} {'steer':>6} {'steer/100':>10}")
    for week in sorted(msgs):
        m = msgs[week]
        i = interrupts[week]
        s = sum(1 for g, w in gaps if w == week and g >= args.threshold)
        print(f"{f'{week[0]}-W{week[1]}':10} {m:6d} {i:5d} {100 * i / m:8.1f} "
              f"{s:6d} {100 * s / m:10.1f}")
    print()
    print("Per BUG is the meaningful denominator, but only in weeks where fixing bugs")
    print("WAS the work — dividing by bugs in a feature week says nothing.  Bugs closed")
    print("per week: gh issue list --state closed --label bug --json closedAt")
    return 0


if __name__ == "__main__":
    sys.exit(main())
