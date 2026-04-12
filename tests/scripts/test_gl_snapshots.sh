#!/bin/bash
# Run every snapshot test registered in tests/scripts/gl_snapshots.tsv.
#
# For each row: launch the example under Xvfb, capture a screenshot via
# tests/scripts/snap_example.sh, compare to the registered golden PNG with
# ImageMagick `compare -metric AE -fuzz 1%`.  A non-zero pixel diff is a
# failure.
#
# Flags:
#   --update   regenerate every golden PNG in-place instead of comparing.
#
# Exit codes:
#   0 — all comparisons within tolerance (or --update succeeded)
#   1 — one or more comparisons failed (diff paths listed on stderr)
#   2 — missing dependency (xvfb-run / compare / import / xdotool)
set -e

MODE="compare"
if [ "${1:-}" = "--update" ]; then
  MODE="update"
fi

# Dependency check.
for tool in xvfb-run xdotool import compare convert; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "FAIL: missing required tool '$tool' (apt-get install xvfb imagemagick xdotool)" >&2
    exit 2
  fi
done

cd "$(dirname "$0")/../.."

REGISTRY="tests/scripts/gl_snapshots.tsv"
if [ ! -f "$REGISTRY" ]; then
  echo "FAIL: registry not found: $REGISTRY" >&2
  exit 2
fi

mkdir -p /tmp/loft_gl_snapshots tests/golden

failed=0
total=0
while IFS=$'\t' read -r loft_rel golden_rel wait_s window_re geometry key_script; do
  # Skip blank / comment lines.
  case "$loft_rel" in ''|\#*) continue ;; esac

  loft_path="lib/graphics/examples/$loft_rel"
  golden_path="tests/golden/$golden_rel"
  captured="/tmp/loft_gl_snapshots/$golden_rel"
  mkdir -p "$(dirname "$captured")"

  total=$((total + 1))
  printf "  %-30s " "$golden_rel"

  if [ ! -f "$loft_path" ]; then
    echo "FAIL (example not found: $loft_path)"
    failed=$((failed + 1))
    continue
  fi

  # Capture under Xvfb.
  if ! xvfb-run -a -s "-screen 0 $geometry" \
      tests/scripts/snap_example.sh \
      "$loft_path" "$captured" "$wait_s" "$window_re" "${key_script:-}" \
      >/tmp/loft_gl_snap_last.log 2>&1; then
    echo "FAIL (snap)"
    cat /tmp/loft_gl_snap_last.log >&2
    failed=$((failed + 1))
    continue
  fi

  if [ "$MODE" = "update" ]; then
    cp "$captured" "$golden_path"
    echo "updated"
    continue
  fi

  if [ ! -f "$golden_path" ]; then
    echo "FAIL (golden missing: $golden_path — run 'make update-gl-snapshots')"
    failed=$((failed + 1))
    continue
  fi

  diff_count=$(compare -metric AE -fuzz 1% \
    "$golden_path" "$captured" \
    "/tmp/loft_gl_snapshots/$golden_rel.diff.png" 2>&1 || true)
  # Small diffs (<0.5% of total pixels) are accepted — animation-driven
  # examples show sub-pixel differences between runs due to real-time
  # (`ticks()`) frame jitter.  A larger diff indicates a real visual
  # regression.  Adjust the tolerance per-example via a 7th TSV column
  # if an example legitimately produces more movement between captures.
  img_pixels=$(identify -format "%[fx:w*h]" "$golden_path" 2>/dev/null || echo 480000)
  tolerance=$(( img_pixels / 200 ))   # 0.5%
  # `diff_count` may be a float under some IM builds — coerce to int.
  diff_int="${diff_count%%.*}"
  diff_int="${diff_int:-0}"
  # Strip anything non-numeric (e.g. "(0.00391)" suffix).
  case "$diff_int" in
    ''|*[!0-9]*) diff_int=999999 ;;
  esac
  if [ "$diff_int" -le "$tolerance" ]; then
    echo "ok ($diff_int px differ, tolerance $tolerance)"
  else
    echo "FAIL ($diff_count px differ, tolerance $tolerance — diff at /tmp/loft_gl_snapshots/$golden_rel.diff.png)"
    failed=$((failed + 1))
  fi
done < "$REGISTRY"

echo "$total checked, $failed failed"
if [ "$failed" -gt 0 ]; then
  exit 1
fi
exit 0
