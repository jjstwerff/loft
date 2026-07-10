<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 — the `skip_free`-orphan text-temp class (case a FIXED; 5 leakers left)

After the promotion fixes (16→6 leakers, n3/p54_b6/p227/generic-text), the residual
6 were ONE class: a text temp marked `skip_free` (so its backing `String` outlives
its block, for a consumer to copy) that is then **never freed on the interpreter**
→ orphan. Native drops it via RAII, so it is INTERPRETER-ONLY. Members:

- **`issue_437`** — the `??` ncc temp `__ncc_N` (`operators.rs`, `set_skip_free`).
  **FIXED** (case a, 6→5) — see "The fix (case a)" below.
- **`p329`×3, `p330`×2** — the tuple element hoist `__ret_text_N`
  (`scopes.rs:3454`, `set_skip_free`). Still open (case b, the return-ABI class).

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

## The fix (case a) — free after the consuming statement

`src/scopes.rs::convert` now, after popping the block tail, walks each **non-tail**
statement and — via `collect_consumed_ncc_text` — finds every `skip_free` text
`__ncc_N` temp whose `ncc`-named value-block is nested inside it (the temps that
statement CONSUMES IN PLACE). It emits `OpFreeText(temp)` right after the statement.

Why this is the exact chokepoint, and why it's safe:
- **The free CANNOT live at the ncc block** — its result ALIASES `__ncc` (the true
  arm yields the temp), so a free there dangles the value the consumer still reads
  (verified: un-suppressing `skip_free` for text put the free at the ncc block exit
  and corrupted the value on interp — assertions failed — while native, which
  borrows and treats `OpFreeText` as a no-op, stayed clean).
- **Every text consumer COPIES** (SetText / assignment / append copy the String), so
  once the consuming *statement* completes the temp's String is dead. The free lands
  after `OpSetText` and before `OpFinishRecord`, per loop iteration → no orphan.
- **The tail expression is left untouched** — that IS case b (the value the caller
  copies after return); an in-function free would UAF. The walker only processes
  non-tail statements, so a tail `?? ""` return stays `skip_free`.
- **Interp-only, no native change.** Native treats `OpFreeText` as a no-op
  (`generation/ops/text_ops.rs:64`, `pre_eval.rs:169` — Rust drops via RAII) and keys
  its `__ncc_*` detection on the NAME, not the flag, so the added op is invisible to
  native: no double-free, and the `block_contains_ncc_skip_free` machinery is intact.
- The walker STOPS at non-`ncc` `Block`/`Loop` scopes (they run their own `convert`
  and free their own temps) → no double-free of a temp consumed in a nested scope.
- Only `Type::Text` ncc temps are freed; heap-DbRef ncc temps (`?? []`, `?? Enum{}`)
  keep their existing skip_free treatment (native materialises them).

Gate: full suite green (the 94-`??`-file blast radius), correctness + leak=0 on both
backends across a consumer matrix (vector element, assignment, struct field, concat,
nested `??`, if-arm, loop, non-text control). Guard probe:
`probes/residual-19/issue437_case_a_ncc_FIXED.loft` (leak=0).

## Why case b is NOT a session-tail change

- **`p329`/`p330` (case b):** the return-ABI landmine — either free the returned
  view without UAF (impossible with the Str ABI) or re-open `__tuple`-for-generics
  past the documented regression. Both delicate. A `?? ""` that IS the return tail
  is the same shape (the ncc temp escapes as the returned value) and is left
  `skip_free` by design.

## The right shape for the arc (next session)

A per-temp **last-use + escape** analysis for `skip_free` text temps: if the temp
does NOT escape (not the block-value, not a return element, only copied into a
distinct store) → emit `OpFreeText` after its last use (case a); else leave it
(case b) and solve case b via promotion/buffer. Do `issue_437` (case a) first as its
own probe — a minimal `o += [v[i] ?? ""]` loop, ASan leak oracle, both backends,
full suite (the 94-file `??` blast radius is the gate). Tuples (case b) fold into the
forward-ref/`__tuple` promotion work, not this free.

## Status: case a FIXED (issue_437, 6→5). Case b (p329/p330 tuple) still open.
