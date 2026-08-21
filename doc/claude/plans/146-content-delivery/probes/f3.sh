#!/usr/bin/env bash
# @PLN146 F3 — zero fetches inside a frame.
#
#   ./probes/f3.sh              (LOFT=<binary> to pick one; default: `loft`)
#
# The instrument is the server's own range log, read DIFFERENTIALLY: the same
# program is run with no frames and with sixty, and the wire must not know the
# difference.  No marker has to correlate two streams, and a fetch inside a frame
# cannot hide in a count that was going to be non-zero anyway.
#
# Red when the sixty-frame run touches the wire more than the boundary alone did,
# or when the control — one frame reaching for a key nobody prefetched — does NOT.
set -u
cd "$(dirname "$0")/.."
here=$(pwd)
LOFT="${LOFT:-loft}"
PORT="${F3_PORT:-8098}"
work=$(mktemp -d)
srv=""
cleanup() { [ -n "$srv" ] && kill "$srv" 2>/dev/null; rm -rf "$work"; }
trap cleanup EXIT

pack="$work/f3_pack.store"
log="$work/range.log"
meta="$here/probes/f1_pack.meta.store"

F2_PACK_OUT="$pack" LOFT_TIMEOUT=300 "$LOFT" --interpret probes/f2_pack.loft \
  2>/dev/null | grep -q "^packed " || { echo "RED — the packer failed"; exit 1; }
[ -f "$meta" ] || F1_KEEP_PACK=1 LOFT_TIMEOUT=120 "$LOFT" --interpret probes/f1_pack.loft >/dev/null 2>&1
[ -f "$meta" ] || { echo "RED — no scene metadata to plan from"; exit 1; }

python3 probes/f2_server.py "$work" "$PORT" "$log" &
srv=$!
url="http://127.0.0.1:$PORT/f3_pack.store"
for _ in $(seq 40); do curl -fsS -o /dev/null "$url" 2>/dev/null && break; sleep 0.1; done

run() {                      # $1 = frames, $2 = miss?  -> prints "<requests> <stdout tail>"
  : > "$log"
  out=$(F3_FRAMES="$1" F3_MISS="$2" F3_META="$meta" F3_SOURCE="$url" \
        LOFT_TIMEOUT=300 "$LOFT" --interpret probes/f3_prefetch.loft 2>/dev/null \
        | grep -E "^prefetch:|frame\(s\)")
  echo "$(grep -c . "$log") | $(echo "$out" | tr '\n' ' ')"
}

base=$(run 0 "")
play=$(run 60 "")
ctrl=$(run 60 1)
nb=${base%% |*}; np=${play%% |*}; nc=${ctrl%% |*}
echo "boundary only : $base"
echo "sixty frames  : $play"
echo "control (miss): $ctrl"

fail=0
if [ "$nb" = 0 ]; then
  echo "RED — the boundary itself fetched nothing, so the instrument saw no wire"
  fail=1
fi
if [ "$np" != "$nb" ]; then
  echo "RED — sixty frames cost $((np - nb)) extra request(s); a frame fetched"
  fail=1
fi
if [ "$nc" -le "$np" ]; then
  echo "RED — the control fetched nothing extra, so this gate cannot see a frame fetch"
  fail=1
fi
[ "$fail" = 0 ] && echo "F3: 60 frames, $((np - nb)) fetches — and the control proves that is a reading"
exit $fail
