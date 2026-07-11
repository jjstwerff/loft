# Phase 1 byte-identical instrument (loft-codegen Mode B gate)

`p1-field-subpattern-corpus.loft` — one fn per field-sub-pattern path the edit touches.
The edit (struct-enum recursion in `parse_field_sub_pattern`) must leave these EXISTING
paths byte-identical; the new struct-enum nested case is proven standalone (separate file).

## Capture (BOTH backends)

    LB=<repo root>; C=p1-field-subpattern-corpus.loft
    # interp IR+bytecode — WARM UP FIRST (run once, discard), then capture; the first
    # invocation of a fresh process flakes slot numbers (2 vs 65535); runs 2+ are steady.
    LOFT_LOG=static $LB/target/debug/loft --interpret $C 2>/dev/null >/dev/null   # warm-up
    LOFT_LOG=static $LB/target/debug/loft --interpret $C 2>after-ir.txt >/dev/null
    # native Rust (deterministic, no warm-up needed)
    $LB/target/debug/loft --native-emit after-native.rs $C

## Prove (empty diff == unchanged emit, both backends)

    diff before-ir.txt after-ir.txt        # empty
    diff before-native.rs after-native.rs  # empty

## Pre-existing tooling bugs found (file separately; not from this branch — main repros)

1. `loft introspect` / `loft --dump` PANIC on a match with an enum `==` comparison:
   `debug.rs:1077` indexes `database.types[634]` into a len-66 table (the dump State's
   types table is truncated vs the bytecode's full-table type indices). Blocks introspect
   as the instrument → use `LOFT_LOG=static` (full-table run) + `--native-emit` instead.
2. `LOFT_LOG=static` first-invocation flake: the very first run of a fresh process shows
   assigned slots (2), runs 2+ show unassigned (65535). Warm up once before capturing.
