#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLAN54 arc A — extract the IR store-schema registration from the Rust that
# `loft --native --show-rust` generates for tools/ir_schema/ir.loft.
#
# Pipeline (hybrid, per design 2026-06-01):
#   ir.loft  --(loft --native)-->  generated.rs  --(this script)-->  schema block
#   then a hand-written typed API (src/data_store.rs) is layered on top.
#
# This first cut is a PROBE: it isolates the IR-type registration lines and
# reports which external `tN` type-ids they reference (base types + any stdlib
# types), so we can decide the rebasing strategy before emitting final code.
#
# Usage:  python3 tools/ir_schema/extract.py tools/ir_schema/generated.rs

import re
import sys

# Our IR type names (must match ir.loft).  The schema block is the run of
# `init` lines registering these, starting at `enumerate("TypeT")`.
IR_ENUMS = {"TypeT", "Node"}
IR_STRUCTS = {
    "Position", "Key", "SortKey", "NameRef", "IntegerSpec",
    "Block", "ParForBody", "Attribute", "Variable", "Function",
    "LinkedFieldGroup", "Definition", "Data",
}
# Enum-variant structs are recognised by the Ty/Nd prefix.
VARIANT_RE = re.compile(r'structure\("(Ty[A-Z]\w*|Nd[A-Z]\w*)"')


def main(path: str) -> int:
    lines = open(path).read().splitlines()

    # Locate init() body.
    init_start = next(i for i, l in enumerate(lines) if l.startswith("fn init("))
    init_end = next(i for i in range(init_start + 1, len(lines)) if lines[i] == "}")

    # The IR block begins at the first `enumerate("TypeT")` (the first of OUR
    # types) and runs to db.finish() / end of init.
    block_start = next(
        i for i in range(init_start, init_end)
        if 'enumerate("TypeT")' in lines[i]
    )
    block_end = init_end  # up to (not incl.) the closing brace

    block = lines[block_start:block_end]

    # Which `tN` ids does the block DEFINE (let tN = …) vs REFERENCE?
    defined = set()
    referenced = set()
    def_re = re.compile(r'let (t\d+)\s*=')
    ref_re = re.compile(r'\bt(\d+)\b')
    for l in block:
        m = def_re.search(l)
        if m:
            defined.add(m.group(1))
        for rid in ref_re.findall(l):
            referenced.add("t" + rid)

    external = sorted(referenced - defined, key=lambda s: int(s[1:]))

    print(f"init() body: lines {init_start+1}..{init_end+1}")
    print(f"IR schema block: lines {block_start+1}..{block_end+1} ({len(block)} lines)")
    print(f"types defined in block: {len(defined)}")
    print(f"external tN ids referenced (need prelude or rebasing): {external}")
    print()
    print("first 8 block lines:")
    for l in block[:8]:
        print("  " + l.strip())
    print("...")
    print("last 4 block lines:")
    for l in block[-4:]:
        print("  " + l.strip())
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1] if len(sys.argv) > 1 else "tools/ir_schema/generated.rs"))
