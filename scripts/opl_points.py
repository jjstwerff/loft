#!/usr/bin/env python3
"""Turn an osmium OPL stream on stdin into the point corpus `radix_db::pages` reads.

One line per node: `x<TAB>y<TAB>name`, with coordinates as integers of 1e-7 degrees
— the fixed-point form an OSM-derived index stores, and what `routing`'s `Coord`
holds. A node with no `name` tag keeps an empty third field; most POI nodes have
none, and dropping them would invent a corpus where every record carries a string.

The measurement wants a REAL point set because clustering is its whole subject, and
this is the shortest path from an extract to one:

    osmium tags-filter benelux-latest.osm.pbf \\
        n/amenity n/shop n/tourism n/leisure n/historic n/office n/craft \\
        n/natural n/man_made n/highway -o poi.osm.pbf
    osmium cat -f opl poi.osm.pbf | scripts/opl_points.py > ~/.cache/loft-spatial/points.tsv

See `mod pages` in `src/radix_db.rs` (@PLN136).
"""

import re
import sys

# OPL escapes a special character as %<hex>% — including the spaces and commas that
# would otherwise end a field or a tag. Unescaping with a URL decoder leaves the
# trailing `%` behind, which is how a name silently gains punctuation.
ESCAPE = re.compile(r"%([0-9A-Fa-f]{1,6})%")


def unescape(s: str) -> str:
    return ESCAPE.sub(lambda m: chr(int(m.group(1), 16)), s)


def main() -> int:
    out = sys.stdout
    written = 0
    for line in sys.stdin:
        if not line.startswith("n"):
            continue
        x = y = None
        name = ""
        for field in line.rstrip("\n").split(" "):
            if not field:
                continue
            kind = field[0]
            if kind == "x":
                x = field[1:]
            elif kind == "y":
                y = field[1:]
            elif kind == "T":
                for kv in field[1:].split(","):
                    if kv.startswith("name="):
                        name = unescape(kv[5:]).replace("\t", " ")
                        break
        if not x or not y:
            continue
        try:
            xi = int(round(float(x) * 1e7))
            yi = int(round(float(y) * 1e7))
        except ValueError:
            continue
        # (0, 0) is the null island a dropped coordinate lands on, not a place.
        if xi == 0 and yi == 0:
            continue
        out.write(f"{xi}\t{yi}\t{name}\n")
        written += 1
    print(f"{written} points", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
