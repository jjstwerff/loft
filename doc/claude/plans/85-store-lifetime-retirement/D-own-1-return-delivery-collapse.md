<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 / D-own-1 — collapse the `block_result` return-delivery thicket

The ownership code-simplification exploration ([OWNERSHIP_MODEL.md § ACTIVE](../../OWNERSHIP_MODEL.md#active--the-simplification-exploration-next-days-exploratory--revertable)),
first slice. Exploratory + REVERTABLE: land a collapse, validate it identical
across both backends with the @PLN89 oracle, keep it if it shrinks the thicket
without regressing — revert if it doesn't pay off.

## The instrument (re-assertion-site count — design-protocol step 2)

`Parser::block_result` (`src/parser/control.rs`, ~lines 573–1032) is **459 lines,
45 special-case helper calls, 15 distinct tail-shape decision helpers**:

```
14 ref_return            3 text_return                  1 tail_whole_arg_vector
 5 nrvo_collapse_tail_set 3 materialize_vector_arms_into 1 tail_terminal_fresh_local_vec (#448)
 4 return_buffer          2 tail_if_has_null_arm          1 tail_is_struct_field_read
                          2 collect_hidden_ref_args       1 returned_uses_buffer (#448)
                                                          1 body_has_buffer_return (#448)
                                                          1 materialize_view_return
                                                          1 callee_forwards_foreign_store
```

Each helper answers ONE question — *which store does this return deliver, and who
frees `__retbuf` vs the returned store* — by re-inspecting the parse-tree SHAPE of
the tail (is it a branch? a bare `return Var`? an arg borrow? a struct-field
read? a call that forwards a foreign store?). The same question, re-derived ~15
ways. #448 added three more helpers to fix one leak — the tell that the structure,
not the logic, is the burden ([[evolve-data-structures-when-burdened]]).

## The deviation (formal/ownership.md D-own-1)

> Ownership is re-derived per-site by codegen, not carried as one `deps` fact.
> Each fix added a codegen condition rather than completing a fact.

The typed `Deps` substrate (D-own-3) is DONE — the fact is now typed and
readable. So the cure is finally available: have these sites READ the deps fact
instead of re-deriving the tail shape.

## The ONE invariant (design-protocol step 4)

> **The return-delivery decision — which store a `return` yields, and whether the
> caller frees `__retbuf` or the returned store — derives from ONE deps fact
> computed once per return binding, not re-derived per tail-shape at the delivery
> site.**

When this holds, the 15 shape-classifiers collapse to one read: a return either
(a) already writes `__retbuf` (deliver as-is), (b) owns a fresh store (move it
into `__retbuf` / rename), or (c) borrows a visible source (copy into `__retbuf`)
— and *which* is a property the deps fact already encodes (owned vs the borrow
source), not a shape to re-classify.

## First collapse target

The **vector return-buffer sub-thicket**: `return_buffer` + the 3
`materialize_vector_arms_into` sites + the #448 trio (`returned_uses_buffer`,
`body_has_buffer_return`, `tail_terminal_fresh_local_vec`) + `tail_terminal_is_branch`.

### Refined finding (from reading the whole `t == Vector(elm, ls)` arm, ~785–960)

The decision is **already deps-branched at the top** — the code keys off `ls` (the
tail Type's deps): `ls.is_empty()` → OWNED-FRESH; `ls.iter().any(is_argument)` ∧
(struct-field ∨ whole-arg) → BORROWS-ARG; else → multi-arm. So the gap is NOT "the
fact isn't read" — it IS read to branch. The gap is that the **three delivery
MECHANISMS are unfused**, each re-handling shape inside its branch:

| mechanism | what it does | when |
|---|---|---|
| **rename** (`ref_return` + `nrvo_collapse_tail_set`) | promote the tail's local to BE `__retbuf` | owned-fresh, buffer FREE |
| **copy** (`OpClearVector`+`OpAppendVector`+free) | clear `__retbuf`, element-append, free the source | borrows an arg, OR owned-fresh but buffer TAKEN (#448) |
| **forward-copy** (`native_forwarder`) | mint a local, run the foreign-store call into it, copy in | a `#native` callee that delivers its own store |

`#448` is exactly "owned-fresh but the buffer is TAKEN by an early Call return →
must **copy**, not rename" — a fourth path bolted on because rename was assumed
whenever owned-fresh. **The collapse is the SELECTOR, not the reads:** one
`return_delivery(ls, buffer_taken, terminal_kind) -> {Rename, Copy, ForwardCopy, AsIs}`
computed once from the deps fact + buffer-availability, then ONE dispatch to the
three mechanism emitters — so #448's "buffer taken ⇒ copy" becomes a cell in the
selector, not a special case, and the per-branch shape re-handling collapses.

## Probe before building (design-protocol — falsify the hypothesis cheaply)

Before touching the code, confirm on the oracle corpus + the matrix that the deps
fact is ALREADY sufficient to distinguish the cases the shape-classifiers split on
(owned-fresh vs already-`__retbuf` vs arg-borrow). If a case is distinguishable
only by shape and NOT by any deps fact, that gap is a D-own-2 (incompleteness) to
close first — the collapse cannot be wider than the fact is complete.

## Safety net + landing rule

Every step: full suite both backends + the @PLN89 oracle `--ignored` sweep
(leak/value/halt identical) + the `tests/leak.rs` + wrap leak gate. Collapse one
sub-thicket at a time; if a step regresses, bisect by site and revert that site
(the #448 first cut regressed `104-split-text` — the matrix caught it). The win is
measured in deleted helpers + shrunk line count, with zero behaviour change.

## Probe result (deps-sufficiency CONFIRMED — from the #448 bytecode)

The `LOFT_LOG=fn:n_pick` dump of the #448 repro already answers the probe:

- the leaking tail is `_vec_1(4):vector<integer>["__vdb_1"]` — **deps `["__vdb_1"]`**:
  it OWNS a fresh local store, distinct from `__retbuf`;
- the early Call arm is `__retbuf(0):vector<integer>` — **delivers `__retbuf`**;
- `returned` carries the `__retbuf` attr.

So the deps fact *already* distinguishes the three cases the shape-helpers
re-derive: **owned-fresh** (deps = a `__vdb` local ≠ buffer), **delivers-buffer**
(deps = `[__retbuf]`), **arg-borrow** (deps = an arg var). Two of the three #448
helpers (`returned_uses_buffer`, `tail_terminal_fresh_local_vec`) are ALREADY
reading deps — they just re-walk the shape to find the terminal first. The
collapse is therefore sound and not blocked by a D-own-2 gap: **one descent to the
return terminal + one deps read** replaces the per-shape classification. (Empirical
re-confirm on the full oracle corpus is the first step of the build, but the
representative case is settled.)

## Status

- [x] Instrument: the thicket counted (459 lines / 45 calls / 15 helpers).
- [x] Invariant named; first target + probe scoped.
- [x] Probe the deps-sufficiency — CONFIRMED on the #448 case (owned-fresh /
      delivers-buffer / arg-borrow are all deps-distinguishable).
- [x] Read the whole `t == Vector` arm — REFINED the target: the decision is
      already deps-branched; the gap is the THREE unfused mechanisms
      (rename / copy / forward-copy), with #448 a fourth bolt-on. The collapse is
      the SELECTOR, not the reads.
- [ ] **NEXT (the build):** introduce `enum Delivery { Rename, Copy, ForwardCopy,
      AsIs }` + `return_delivery(ls, buffer_taken, terminal_kind) -> Delivery`
      computed once from the deps fact; route the existing branches through ONE
      dispatch to three mechanism emitters; #448 becomes the `buffer_taken ⇒ Copy`
      cell (delete the bolt-on trio). Oracle-guarded, one mechanism at a time;
      measure deleted helpers + line shrink. (Delicate function — best done with a
      fresh budget; the design is now precise enough to execute directly.)
- [ ] Repeat for `parse_return` mid-body (the #448 residual lands here too).
