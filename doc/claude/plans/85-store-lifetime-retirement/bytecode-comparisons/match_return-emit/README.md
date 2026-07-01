<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# match_return emit — the IR-diff-against-the-proven-path capture

These three IR captures are the @PLN85 `match_return` interp-crash diagnosis, done by the
**IR-diff-against-the-proven-path** method (see [CODEGEN_METHOD.md § Diff the proven path](../../../../CODEGEN_METHOD.md)).
Re-generate with `loft introspect` (one capture carries IR + interp bytecode + native Rust).

| file | what | runtime |
|---|---|---|
| `deliver-BEFORE-off.txt` | `deliver` with `LOFT_JOIN_OWN` OFF — the alias `_mv_items_1 = OpGetField(e,4)` | interp LEAK / native clean |
| `deliver-NOW-on.txt` | `deliver` with the emit ON — `OpAppendVector(_mv_items_1, …)` | interp CRASH-under-churn / native clean |
| `field_return-PROVEN.txt` | `fn deliver(b: Box) -> vector { b.rows }` via `copy_borrow_tail_into_retbuf` | clean both, **survives churn** |

## What the diffs proved (vs three wrong theories)

**BEFORE → NOW** (did my change do what I intended?): yes — alias → copy, and the return dep
dropped `"e"` (`["__retbuf","e"]` → `["__retbuf"]`), so the value is owned. The change is
*semantically* right.

**NOW → PROVEN** (what's the residual divergence = the bug?): the **buffer identity**.
- PROVEN delivers into a separate canonical `__retbuf`, and the delivery block is typed
  `["__retbuf"]` (the return-buffer **attr**) — the store analysis tracks it as the owned return.
- NOW reuses the match-field binding `_mv_items_1` *as* the buffer; the arm/inner block is typed
  `["_mv_items_1"]` (a **var** dep). Under allocation pressure the analysis doesn't treat that var's
  store as the live owned return → a later alloc (churn) reuses the slot → UAF.

The diff retired three theories I'd held WITHOUT the capture: the append tp (both correct for their
own retbuf — 65/64), the `__vdb`/`OpReplaceVector` wrapper (the proven path has neither), and
`skip_free`. None was the cause; the buffer-var identity was. **The captured diff is ground truth;
the mental model was wrong three times.**

## The fix the diff specifies

Don't reuse `_mv_items_1` as the buffer. Deliver into a separate canonical `__retbuf` typed
`["__retbuf"]` — i.e. route the borrowed match-arm through the same per-arm materialise that produces
a `["__retbuf"]`-typed delivery (the `materialize_vector_arms_into` / `copy_borrow_tail_into_retbuf`
machinery), rather than promoting the match binding to be the buffer.

## UPDATE — the proven-clean synthesis target (element-loop + inline owned default)

`PROVEN-CLEAN-element-loop-inline-default.loft` (`elC`) is reliably CLEAN on BOTH backends
(4/4 runs, churn pressure):
```
Filled { items } => { o: vector<E> = []; for x in 0..len(items) { o += [items[x] ?? E{hp:0, name:""}]; } o }
```
What each variable established (isolation probes):
- WHOLE-vector append `o += items` in a match arm → pre-existing non-deterministic corruption.
- element-loop with a BORROWED default (`?? items[0]`) or no default → still crashes (the appended
  element stays a borrowed ref → shallow).
- element-loop with an **inline OWNED default** (`?? E{<field defaults>}`) → CLEAN. The owned-typed
  element forces the deep `OpCopyRecord` path, and the inline default (no helper fn) avoids the
  separate 2-function pre-existing parser bug.
So the @PLN85 synthesis must emit THIS — the element-loop with an inline default constructed from the
element struct's field defaults — NOT the whole-vector append. The remaining work is purely building
that `Iter`/`Loop`/`OpNewRecord`/`OpCopyRecord`/default IR in `jo_copy_borrowed_arm_yield` (the parser's
`parse_vector_for`/`build_comprehension_code` consume the lexer, so it must be hand-built or driven
through a synthetic re-parse).
