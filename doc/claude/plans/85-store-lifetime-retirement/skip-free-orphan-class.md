<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 — the `skip_free`-orphan text-temp class (the final 6 leakers)

After the promotion fixes (16→6 leakers, n3/p54_b6/p227/generic-text), the residual
6 are ONE class: a text temp marked `skip_free` (so its backing `String` outlives
its block, for a consumer to copy) that is then **never freed on the interpreter**
→ orphan. Native drops it via RAII, so it is INTERPRETER-ONLY. Members:

- **`issue_437`** — the `??` ncc temp `__ncc_N` (`operators.rs`, `set_skip_free`).
- **`p329`×3, `p330`×2** — the tuple element hoist `__ret_text_N`
  (`scopes.rs:3454`, `set_skip_free`).

## The load-bearing split: case (a) consumed-in-place vs case (b) outlives

`skip_free` currently means "NEVER free". The correct requirement is finer — free
at the temp's LAST USE, which differs by how the value flows:

- **Case (a) — consumed in place (copied), temp then dead.** The returned/outer value
  is a SEPARATE store the temp was copied INTO. Free the temp right after the
  consuming statement (per-iteration inside loops).
  - `issue_437`: `o += [v[i] ?? ""]` → `OpSetText(_elm, 0, __ncc_N)` COPIES `__ncc_N`
    into the vector element; the return is `o` (the vector), NOT `__ncc_N`. `__ncc_N`
    is created INSIDE the `for` loop, so it needs a per-iteration block-scope free
    after the `+=` — a function-exit free (the `__blk_N` trick) would orphan every
    prior iteration.
- **Case (b) — the temp IS (part of) the returned value.** The caller copies it
  AFTER the function returns, so any in-function free UAFs. This is the return-ABI
  problem — needs the caller to own a buffer (promotion), not a free.
  - `p329`/`p330`: the returned tuple's element is `Var(__ret_text_N)` on interp — a
    VIEW into the temp (native CLONES it: `(var___ret_text_1.to_string(), …)`), so on
    interp the temp must outlive the function. The clean path is the `__tuple<…>`
    synthetic-struct ABI (`needs_tuple_rewrite`), which the generic deliberately
    avoids (`tuple_return_rewrite` `from_tv` gate) with a doc-comment warning that
    routing generics through `__tuple` broke p329/p330/p240/plan17 before.

## Why it is NOT a session-tail change

- **`issue_437` (case a, tractable but wide):** the fix is a per-last-use free of
  `__ncc_N`, gated on it being consumed-in-place (NOT the block/return value). That
  distinction is context-dependent (a `?? ""` that IS the return tail is case b and
  must stay alive), and it sits in the `??` operator — 94 test files use `??`, and
  native keys on the `__ncc_*` `skip_free` PATTERN (`block_contains_ncc_skip_free`,
  `needs_ncc_materialise`, `generation/emit.rs:384`, `calls.rs:302/316`). A blanket
  free-at-exit regresses the case-(b) `??` returns; the correct fix needs a real
  last-use / escape analysis for the temp.
- **`p329`/`p330` (case b):** the return-ABI landmine — either free the returned
  view without UAF (impossible with the Str ABI) or re-open `__tuple`-for-generics
  past the documented regression. Both delicate.

## The right shape for the arc (next session)

A per-temp **last-use + escape** analysis for `skip_free` text temps: if the temp
does NOT escape (not the block-value, not a return element, only copied into a
distinct store) → emit `OpFreeText` after its last use (case a); else leave it
(case b) and solve case b via promotion/buffer. Do `issue_437` (case a) first as its
own probe — a minimal `o += [v[i] ?? ""]` loop, ASan leak oracle, both backends,
full suite (the 94-file `??` blast radius is the gate). Tuples (case b) fold into the
forward-ref/`__tuple` promotion work, not this free.

## Status: MAPPED. No code change — the fix needs escape analysis, not a patch.
