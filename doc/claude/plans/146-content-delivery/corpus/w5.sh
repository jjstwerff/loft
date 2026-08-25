#!/usr/bin/env bash
# @PLN146 W5 — does the METRIC channel say what the oracle says, and does the
# exit status carry the verdict?
#
#   ./w5.sh              measure every scene in checks/ on both backends
#   ./w5.sh --control    widen the default tolerance first; the diff must see it
#
# `draw.py --once` renders once and exits 1 when a line did not parse or a check
# failed, writing its report to `stats.txt`.  This runs the loft package's own
# `examples/draw.loft` against the same scenes and compares BOTH halves: the
# report text, byte for byte against the oracle's own blocks, and the exit code.
#
# Two blocks of `stats.txt` are deliberately not compared — the density map and
# the composition notes.  Both measure the rendered PICTURE, which `w2.sh`
# already diffs pixel for pixel; this gate measures the SCENE.
#
# `checks/deferred.draw` is the one scene where the two are MEANT to disagree:
# the oracle draws `Petals` and passes, and a build that does not draw that mark
# has to say so rather than answer "fine".  It is asserted, not diffed.
#
# A `use`d library resolves to its compiled `native-auto/` cdylib EVEN UNDER
# `--interpret`, so the interpret lane sets `LOFT_NO_NATIVE_LIBS=1` — without it
# both lanes measure the same binary (@PLN146 W4 finding 2).
set -u
cd "$(dirname "$0")"
HERE=$(pwd)
WORKSPACE=$(cd "$HERE/../../../../../.." && pwd)
DRAWING=${W5_DRAWING:-$WORKSPACE/loft-libs-graphics/drawing}
GRAPHICS=${W5_GRAPHICS:-$WORKSPACE/loft-libs-graphics/graphics/src}
LOFT=${LOFT:-$HERE/../../../../../target/release/loft}
# The interpreter renders a 1000x700 scene in minutes, so the one full-size
# scene runs on the compiled lane only.  Every check FORM is in checks/, and
# those run on both.
BIG=${W5_BIG:-scenes/old_woman.draw}

control=0
[ "${1:-}" = "--control" ] && control=1

if [ ! -f "$DRAWING/src/drawing.loft" ]; then
  echo "RED  no drawing package at $DRAWING — set W5_DRAWING to its root"
  exit 1
fi
if [ ! -x "$LOFT" ]; then
  echo "RED  no loft binary at $LOFT — set LOFT"
  exit 1
fi
if ! command -v python3 >/dev/null; then
  echo "SKIP no python3 — the oracle cannot run"
  exit 0
fi

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

src=$DRAWING
if [ "$control" = 1 ]; then
  src=$out/control-src
  cp -r "$DRAWING" "$src"
  rm -rf "$src/native-auto"
  sed -i 's|^pub const DEFAULT_TOL = 0.02;|pub const DEFAULT_TOL = 0.5;|' "$src/src/drawing.loft"
  grep -q "DEFAULT_TOL = 0.5" "$src/src/drawing.loft" || { echo "RED  control did not patch"; exit 1; }
  echo "control: a bare ~ is judged at 0.5 of the paper instead of 0.02"
fi

# The oracle, once per scene: exit code + the report blocks.
for scene in checks/*.draw "$BIG"; do
  name=$(basename "$scene" .draw)
  SKETCH_OUT="$out/oracle-$name" python3 oracle/draw.py --once "$scene" >/dev/null 2>&1
  echo "$?" > "$out/oracle-$name.exit"
done

run_lane() {
  lane=$1
  shift
  for scene in "$@"; do
    name=$(basename "$scene" .draw)
    env_extra=()
    [ "$lane" = "--interpret" ] && env_extra=(env LOFT_NO_NATIVE_LIBS=1)
    "${env_extra[@]}" "$LOFT" "$lane" --lib "$src/src" --lib "$GRAPHICS" \
      "$src/examples/draw.loft" --once "$HERE/$scene" "$out/$name.png" \
      > "$out/loft-$lane-$name.txt" 2> "$out/loft-$lane-$name.err"
    echo "$?" > "$out/loft-$lane-$name.exit"
  done
}

small=(checks/*.draw)
run_lane --native "${small[@]}" "$BIG"
run_lane --interpret "${small[@]}"

python3 - "$out" "$HERE" "$control" "$BIG" <<'PY'
import sys, os, glob

out, here, control, big = sys.argv[1], sys.argv[2], sys.argv[3] == "1", sys.argv[4]
scenes = [os.path.basename(p)[:-5] for p in sorted(glob.glob(os.path.join(here, "checks", "*.draw")))]

def oracle_report(name):
    """`stats.txt` minus the two blocks that measure the PICTURE."""
    lines = open(os.path.join(out, f"oracle-{name}", "stats.txt")).read().split("\n")
    density = lines.index("density (darker = more ink):")
    elements = lines.index("elements (bbox frac):")
    end = next(i for i, l in enumerate(lines) if l.startswith("COMPOSITION"))
    return lines[:density] + lines[elements:end - 1] + [""]

bad = []
for name in scenes + [os.path.basename(big)[:-5]]:
    lanes = ["--native"] + ([] if name == os.path.basename(big)[:-5] else ["--interpret"])
    if name == "deferred":
        # The scene the two are MEANT to disagree on.
        oex = int(open(os.path.join(out, "oracle-deferred.exit")).read())
        if oex != 0:
            bad.append(f"deferred — the ORACLE failed it ({oex}); this scene only means "
                       "something while the oracle draws Petals and passes")
        for lane in lanes:
            lex = int(open(os.path.join(out, f"loft-{lane}-deferred.exit")).read())
            rep = open(os.path.join(out, f"loft-{lane}-deferred.txt")).read()
            if lex == 0:
                bad.append(f"deferred {lane} — a mark this build does not draw exited 0")
            if "DEFERRED" not in rep or "Petals" not in rep:
                bad.append(f"deferred {lane} — the report does not name the undrawn mark")
        continue
    want = oracle_report(name)
    owant = int(open(os.path.join(out, f"oracle-{name}.exit")).read())
    for lane in lanes:
        got = open(os.path.join(out, f"loft-{lane}-{name}.txt")).read().split("\n")
        gotex = int(open(os.path.join(out, f"loft-{lane}-{name}.exit")).read())
        if got != want:
            first = next((i for i in range(max(len(got), len(want)))
                          if (got[i:i + 1] or [None]) != (want[i:i + 1] or [None])), 0)
            bad.append(f"{name} {lane} — report differs at line {first + 1}:\n"
                       f"      oracle: {(want[first:first+1] or ['<none>'])[0]!r}\n"
                       f"      loft:   {(got[first:first+1] or ['<none>'])[0]!r}")
        if (gotex != 0) != (owant != 0):
            bad.append(f"{name} {lane} — exit {gotex} against the oracle's {owant}")

if control:
    if bad:
        print(f"control fired: {len(bad)} reading(s) went red, as they must")
        sys.exit(0)
    print("CONTROL DID NOT FIRE — the gate is not reading the reports")
    sys.exit(1)
if bad:
    for b in bad:
        print(f"RED  {b}")
    sys.exit(1)
n = len(scenes) + 1
print(f"{n} scene(s): the metric report is the oracle's, and the exit status is the verdict")
PY
