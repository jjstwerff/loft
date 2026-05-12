#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Render doc/claude/presentations/par/par_slides.html to par_slides.pdf
# via headless Chromium.  The HTML is the source of truth; the PDF is
# a derived artefact.
#
# Usage (run from anywhere):
#   ./doc/claude/presentations/par/build_par_slides.sh           # write par_slides.pdf
#   ./doc/claude/presentations/par/build_par_slides.sh --check   # render to a temp file
#                                                                # and diff against the
#                                                                # committed PDF; exits
#                                                                # non-zero on size drift
#   ./doc/claude/presentations/par/build_par_slides.sh -o foo.pdf  # custom output path
#
# Requires: chromium (or google-chrome / chromium-browser) on PATH.
# Tested with Chromium 147.x snap.  The --no-pdf-header-footer flag
# requires Chrome >= 90; older binaries take --print-to-pdf-no-header.
#
# The script lives next to the assets it renders — the PDF is a
# committed artefact reviewed by hand before each meetup, not part of
# the CI gate.  Run on demand when SOURCE.html changes.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PRES_DIR="$SCRIPT_DIR"
SOURCE_HTML="$PRES_DIR/par_slides.html"
DEFAULT_OUTPUT="$PRES_DIR/par_slides.pdf"

# Pick the first chromium-shaped binary that exists.  Order matters:
# google-chrome is the upstream binary name on Debian/Ubuntu .deb;
# chromium is the snap / Arch / Fedora name; chromium-browser is the
# legacy Ubuntu name.
find_chromium() {
    for bin in google-chrome chromium chromium-browser chrome; do
        if command -v "$bin" >/dev/null 2>&1; then
            echo "$bin"
            return 0
        fi
    done
    return 1
}

usage() {
    sed -n '5,23p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

OUTPUT="$DEFAULT_OUTPUT"
CHECK_MODE=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        -o|--output)
            OUTPUT="$2"; shift 2 ;;
        --check)
            CHECK_MODE=1; shift ;;
        -h|--help)
            usage; exit 0 ;;
        *)
            echo "unknown argument: $1" >&2
            usage >&2
            exit 2 ;;
    esac
done

if [[ ! -f "$SOURCE_HTML" ]]; then
    echo "source HTML not found: $SOURCE_HTML" >&2
    exit 1
fi

CHROMIUM="$(find_chromium)" || {
    echo "no chromium binary on PATH (looked for: google-chrome chromium chromium-browser chrome)" >&2
    exit 1
}

# In --check mode, render to a temp file and diff against the committed
# PDF.  Useful as a hook before committing HTML changes.
#
# Both temp files live under PRES_DIR rather than /tmp because the snap
# build of Chromium (Ubuntu 22.04+) sandboxes /tmp into a private mount
# namespace; chromium reports a successful write but the file is
# invisible to the host.  Writing inside the repo avoids the boundary.
TMP_LIST=()
cleanup_tmps() {
    if (( ${#TMP_LIST[@]} > 0 )); then
        rm -f "${TMP_LIST[@]}"
    fi
}
trap cleanup_tmps EXIT

if [[ $CHECK_MODE -eq 1 ]]; then
    TMP_PDF="$PRES_DIR/.par_slides_check.$$.pdf"
    TMP_LIST+=("$TMP_PDF")
    OUTPUT="$TMP_PDF"
fi

echo "rendering $SOURCE_HTML -> $OUTPUT (via $CHROMIUM)"

# Headless Chromium's stdout is verbose across versions; redirect to a
# log file inside PRES_DIR (same snap-namespace caveat as above).
LOG="$PRES_DIR/.par_slides_render.$$.log"
TMP_LIST+=("$LOG")

"$CHROMIUM" \
    --headless=new \
    --no-sandbox \
    --disable-gpu \
    --no-pdf-header-footer \
    --print-to-pdf="$OUTPUT" \
    "file://$SOURCE_HTML" \
    > "$LOG" 2>&1 || {
        echo "chromium render failed; full log:" >&2
        cat "$LOG" >&2
        exit 1
    }

if [[ ! -s "$OUTPUT" ]]; then
    echo "render produced an empty file; chromium log:" >&2
    cat "$LOG" >&2
    exit 1
fi

if [[ $CHECK_MODE -eq 1 ]]; then
    if [[ ! -f "$DEFAULT_OUTPUT" ]]; then
        echo "no committed PDF at $DEFAULT_OUTPUT to compare against" >&2
        exit 1
    fi
    NEW_SIZE=$(stat -c%s "$OUTPUT" 2>/dev/null || stat -f%z "$OUTPUT")
    OLD_SIZE=$(stat -c%s "$DEFAULT_OUTPUT" 2>/dev/null || stat -f%z "$DEFAULT_OUTPUT")
    DELTA=$(( NEW_SIZE - OLD_SIZE ))
    ABS_DELTA=${DELTA#-}
    # 5 % tolerance — Chromium emits PDF metadata timestamps so byte-
    # equality is not achievable across runs even from identical HTML.
    THRESH=$(( OLD_SIZE / 20 ))
    echo "committed: $OLD_SIZE bytes"
    echo "rendered:  $NEW_SIZE bytes (delta $DELTA)"
    if (( ABS_DELTA > THRESH )); then
        echo "drift exceeds 5 % threshold ($THRESH bytes); review the HTML and regenerate the PDF" >&2
        exit 1
    fi
    echo "ok: rendered PDF size matches committed within 5 %"
    exit 0
fi

SIZE=$(stat -c%s "$OUTPUT" 2>/dev/null || stat -f%z "$OUTPUT")
echo "wrote $SIZE bytes to $OUTPUT"
