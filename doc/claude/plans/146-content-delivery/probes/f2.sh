#!/usr/bin/env bash
# @PLN146 F2 — the same source from a local pack and from a URL, and a byte-range
# log that says only the requested keys crossed the wire.
#
#   ./probes/f2.sh              (LOFT=<binary> to pick one; default: `loft`)
#
# Red three ways: the two runs disagree about the bytes, the URL run fetches the
# whole file, or the server is asked for something the two keys do not need.
set -u
cd "$(dirname "$0")/.."
LOFT="${LOFT:-loft}"
PORT="${F2_PORT:-8099}"
work=$(mktemp -d)
srv=""
cleanup() { [ -n "$srv" ] && kill "$srv" 2>/dev/null; rm -rf "$work"; }
trap cleanup EXIT

pack="$work/f2_pack.store"
log="$work/range.log"

F2_PACK_OUT="$pack" LOFT_TIMEOUT=120 "$LOFT" --interpret probes/f2_pack.loft \
  2>/dev/null | grep -q "^packed " || { echo "RED — the packer failed"; exit 1; }
size=$(stat -c%s "$pack")

local_out=$(F2_SOURCE="$pack" LOFT_TIMEOUT=120 "$LOFT" --interpret probes/f2_range.loft 2>/dev/null \
            | grep -E "^hit\.ogg|^resident|^absent-key")

python3 probes/f2_server.py "$work" "$PORT" "$log" &
srv=$!
for _ in $(seq 40); do
  curl -fsS -o /dev/null "http://127.0.0.1:$PORT/f2_pack.store" 2>/dev/null && break
  sleep 0.1
done
: > "$log"                                  # drop the readiness probe's own line

url_out=$(F2_SOURCE="http://127.0.0.1:$PORT/f2_pack.store" LOFT_TIMEOUT=120 \
          "$LOFT" --interpret probes/f2_range.loft 2>/dev/null \
          | grep -E "^hit\.ogg|^resident|^absent-key")

fail=0
echo "local: $local_out" | tr '\n' ' '; echo
echo "url  : $url_out" | tr '\n' ' '; echo
if [ "$local_out" != "$url_out" ]; then
  echo "RED — a URL answers different bytes than the same pack on disk"
  fail=1
elif [ -z "$local_out" ]; then
  echo "RED — the local run produced nothing to compare"
  fail=1
fi

full=$(grep -c "^FULL " "$log" || true)
ranges=$(grep -c "^RANGE " "$log" || true)
fetched=$(awk '/^RANGE /{n += $4} END {print n + 0}' "$log")
echo "server: $ranges range request(s), $full whole-file, $fetched of $size bytes fetched"
if [ "$full" != 0 ]; then
  echo "RED — the reader asked for the whole file; the point is that it does not"
  fail=1
fi
if [ "$ranges" = 0 ]; then
  echo "RED — no range request reached the server, so nothing was proven"
  fail=1
fi
# Two keys of ~2 KB and ~4 KB out of a ~20 KB pack: a paged read must stay well
# under the file.  Half is a generous bound and still fails a whole-file read.
if [ "$fetched" -ge $((size / 2)) ]; then
  echo "RED — $fetched of $size bytes is not a paged read"
  fail=1
fi
[ "$fail" = 0 ] && echo "F2: same answer from disk and from a URL, and only the pages the keys touch"
exit $fail
