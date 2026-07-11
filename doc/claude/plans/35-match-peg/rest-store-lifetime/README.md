<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN35 — `..rest` store-lifetime investigation (probes + reporting oracle)

@PLN85-style attack on the store-lifetime leaks `..rest` surfaced. **Probes first,
matrix on both backends, a reporting OBSERVER oracle that explains WHY** — no fix
until the class is mapped.

## The shape under investigation

`..rest` materialises the tail sub-slice into a FRESH vector (its own `__vdb` store)
INSIDE a match arm. Depending on how the arm's result and the head captures flow, the
fresh store's free is placed correctly (clean) or mis-placed (leak). Confirmed
mechanisms so far (before this systematic sweep):

- **e1 class** — an escaping variant-head field capture (`[K { w }, ..rest] => <w as
  promoted &text>`) leaves the rest store freed BEFORE its own allocation (the `&text`
  return isn't hoisted; native-dangle-safe but leaks the sibling store). `store_confinement`
  can't confine the rest store: its `__vdb` is dep-backed by BOTH the user `rest` local
  AND the per-element `_elm` temp → rejected as **ambiguous**.
- Naively excluding `_`-temps from the ambiguity gate FIXES e1 but REGRESSES p1
  (over-confines a still-used `rest` → empty read). So the fact needed is narrower.

## Ground truth

The interpreter store-leak check (`LOFT_STORES=warn` / the test harness) is the leak
oracle. Value-correctness is the assertion. Both backends.

## Files

- `probes/*.loft` — one shape per file, each with a hand-computed expected value and a
  leak expectation (`@LEAK ok` / `@LEAK leak`) in a header comment.
- `run_matrix.sh` — runs every probe on `--interpret` (leak-checked) + `--native`,
  records value×leak, prints the matrix.
- The reporting oracle (`LOFT_REST_ORACLE=1`) — per-function store-lifetime facts:
  each `__vdb` store, its backer, scope, the `store_confinement` verdict + the exact
  gate that rejects it, and where its free lands.
