<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 58 — Nested-vector layout (vector-of-vectors stride & sentinel reconciliation)

## Status — DONE / CLOSED 2026-06-03

`vector<vector<…>>` is a load-bearing data structure (map tiles, matrices,
adjacency lists, comprehension results).  This stability investigation closed
the whole corruption/crash class across **depth × element-type × context**, not
just the one reported crash (#262).  All clusters fixed at their sources, each
locked by a cross-backend regression test.  The full nested-vector matrix is
green on `--interpret` and `--native` except the out-of-scope `#263`
(call-returned fn-ref into *any* collection — a general closures bug).

| Cluster | Was | Fix | Commit | Regression |
|---|---|---|---|---|
| **II — single sentinel** | `vector<…<single>>` SIGSEGV (filed **#262**) | generalize the `@P380` handle-zero to every construction path | `57f7bdb9` | `tests/scripts/183` |
| **III-a — narrow-int literal** | `i32`/`i16`/`u8` nested literals silently corrupt | propagate the declared element type into the inner literal (`parse_item`) | `a00b0d1b` | `184` |
| **boolean — handle stride** | `vector<vector<boolean>>` corrupt→crash (handles overlap at stride 1) | parse-time `known` (outer type for sub-4 inner) + read-stride clamp to ≥4 | `473b1b46` | `185` |
| **IV — comprehension** | nested `[for …]` CONST_STORE panic + off-by-one | deep-copy (`OpCopyRecord`) + element-type `known` (`vector_of`) | `2d598fca` | `186` |
| (adjacent) **`vector<character>` reads** | `v[0]` / `for c in v` "field access on character" | add the missing `Type::Character` arm to `get_val` | `5ef3fb1e` | `187` |

Earlier scaffolding: `cd812721` (plan open + the temporary `--vec4` lever),
`17c70712` (`db_type` element resolution + probe hardening), `b4ec29d9` /
`ecc522de` (cluster-I journal + residual), `0156ecfc` (lever retired, −109 lines).

## Root, in one line

Every cluster was a shape of *"the construct stride / type-id and the read
stride / type-id for a vector-handle element disagree"* — the
`vector<T>` ↔ `vector<vector<T>>` conflation (the #250 family).  ≥4-byte inner
scalars survived by self-consistency; sub-4 (`boolean`) overlapped; narrow ints
lost their width in literals; the comprehension over-wrapped one level.  The
detailed mechanism + the full 34-cell ±backend matrix + the Stage-B fix journal
(three reverted attempts that bisected to the real fix) live in
[RESULTS.md](RESULTS.md); the stride-divergence and single-sentinel mechanisms in
[`cluster-I-stride-divergence.md`](cluster-I-stride-divergence.md) /
[`cluster-II-single-sentinel.md`](cluster-II-single-sentinel.md).

## Out of scope (general bugs surfaced, not nesting)

- **`#263`** — storing a call-returned fn-ref into *any* collection crashes
  (flat too, even non-capturing).  Closures bug; filed; the lone remaining red
  matrix cell.
- **`#248`** — cross-package struct-ctor return → CONST_STORE write panic.  The
  agent proved this is a *distinct* root from cluster IV (store-nr aliasing in
  the struct-return ABI); filed.

## Accepted residual

For a ≥4-byte inner scalar the outer vector still strides handles by the inner
scalar size (8) / preallocs the wide stride (16).  **Benign** — no corruption or
leak (the slot is ≥ the 4-byte handle).  Accepted as-is.  Future direction: a
**stride guard** (the sanitizer half of the dogfood+sanitizer model, GOALS.md)
asserting construct-stride == read-stride for vector handles would surface this
class mechanically — this investigation's hand-built matrix was that guard, by
hand.  See cluster-I doc § Resolution.

## Probe → test mapping

`probes/` (60 files) stays as the characterization landmark suite (full matrix,
pinpoint, strong-value probes).  Permanent guarantees graduated to CI:
`tests/scripts/182` (#250, pre-existing) + `183`–`187` (the five fixes above),
each exercising interp + native + WASM via `wrap.rs` / `native.rs`.

## See also

- [RESULTS.md](RESULTS.md) — full matrix, Stage-B fix journal, per-cluster closure notes.
- `src/parser/{vectors.rs,fields.rs,mod.rs}`, `src/database/structures.rs` — fix sites.
- `tests/scripts/182-deep-nested-vector-copy.loft` — the #250 regression landmark.
- GitHub #262 (fixed) · #263 / #248 (open, out of scope).
