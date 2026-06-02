<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# cluster-I probes — confinement & escape landmarks (Goal E)

These are the **landmarks** of the store-lifetime investigation: each one records
a shape, what the lifetime model did, and what the `LOFT_STORE_GUARD` confinement
analysis must say about it.  *This worked; this was broken; this is what we fixed.*
Run any with `LOFT_STORE_GUARD=1 loft --interpret --tests <file>` (a line per
late-freed block-confined store) and `--tests` for the correctness assert.

`00_*` are the core landmarks; the rest are edge probes.  Verdicts: **confined** =
guard SHOULD fire (a real late-free the fix will close); **escape** / **loop-reuse**
= guard MUST stay silent (freeing at block exit would be wrong / pointless).

## Matrix

| Probe | Shape | Verdict | Lands |
|---|---|---|---|
| `00_rc_trace_one_block` | one block-local vector | — | **rc crux**: `dec_rc=0`, store is rc=1, single owner — no rc holds it past scope |
| `00_watermark_many_blocks` | 5 sibling if-block vectors | confined×5 | the O(block-locals) watermark this closes |
| `00_soundness_danger` | escape-after-block + dead-at-block + loop reuse | mixed | the correctness guard (both backends) the fix must keep |
| `16_nested_block` | vector in block-in-block | confined | innermost-block confinement |
| `17_match_arm` | vector in a match arm | confined | match arms are blocks |
| `18_both_branch_escape` | set in both branches, read after | escape | read-after-block ⇒ not confined |
| `19_per_branch_confined` | distinct vectors in then/else | confined×2 | per-branch confinement |
| `20_forloop_over_confined` | created in block, iterated there | confined | **LCA**: for-loop adds a `#For` sub-block — exact-match missed it |
| `22_copied_in_block` | `w = a` in the block | confined | copy is independent |
| `23_vec_of_vec` | `[[1,2],[3,4]]` | confined | inner vectors store inline |
| `24_early_return` | `return a[0]+a[2]` (values, not `a`) | confined | element values escape, not the store |
| `25_read_in_loop` | read inside a loop body, declared outside | confined | **LCA**: loop-internal read attributes to the enclosing block |
| `26_nested_if_use` | created in block, used in nested if | confined | **LCA**: nested sub-block |
| `27_partial_outer` | reassigned in branch, read after | escape | read-after ⇒ escape |
| `30_escape_return` | `return a` | escape | **direct escape** — `a`'s store is handed to the caller |
| `31_escape_struct_field` | `b = Bag{items:a}`, b returned | confined | struct construction **deep-copies** (so `a` is private) |
| `32_grow_in_block` | `a += [..]` in the block | confined | grown vector still confined |
| `33_loop_local` | vector declared in a loop body | loop-reuse | **loop-path**: per-iteration reuse, not a watermark |
| `34_nested_loop` | vector in a doubly-nested loop | loop-reuse | loop-path through both loops |
| `35_reassign_in_block` | `a = [..]; a = [..]` in a block | confined | cluster-III shape, confined |
| `u1_struct_field_indep` | `Bag{items:a}` + mutate `a` | (fn-level) | **proves** struct-field is a deep copy (`b.items[0]` stays 1) |
| `u2_return_struct` | block-local vec into a returned struct | confined | sound — the struct copied it |
| `u3_block_result` | `x = { …; a }` (block result) | escape | **block-result escape** — `x` gets dep `["a"]`, `a`'s store is shared |

## What the probes fixed in the confinement analysis

1. **Exact-match → least-common-ancestor** (20/25/26).  A reference in a nested
   sub-block (for-loop `#For` block, nested `if`, loop body) belongs to the
   enclosing block; require the LCA of all reference scope-paths, not equal
   innermost scopes.  Exact-match under-fired on the *most common* shapes.
2. **Loop-path exclusion** (33/34).  A confinement whose path passes through any
   loop is per-iteration reuse, not a watermark — silent.
3. **Direct escape** (30).  `return`/`yield`/`break` of the local hands its store
   to the caller — exclude (`guard_escapes`).
4. **Block-result escape** (u3).  A local that is a block's last expression flows
   out of the block (`x = { …; a }` ⇒ `x["a"]`).  Exclude — plus a dep-escape
   check (any variable that depends on the local and outlives the block).
5. **rc crux dissolved** (`00_rc_trace`, `rc_*`).  No vector shape shows a
   `dec_rc`: stores are rc=1, single-owner.  The store is freeable the moment its
   data is last read — no rc surgery, just emit the free at the confined last use.

## Still uncertain (probe next)

- Deeper aliasing chains (`y` depends on `x` depends on `a`) — the dep-escape
  check only follows direct dependents.
- `return (a, n)` / tuple- and match-destructuring bindings.
- `parallel {}` / `par_for` blocks (threading + store ownership).

## Round 2 — aliasing chains, tuples, parallel (the three "still uncertain" edges)

| Probe | Shape | Verdict | Lands |
|---|---|---|---|
| `a1_nested_blockresult` | `x = { y = { a }; y }` | escape (silent) | **borrow chains are sound** — block-result fix handles nesting |
| `a2_blockresult_alias` | `x = { a }; out = x` (outer) | escape (silent) | block-result aliased to outer — silent ✓ |
| `t1_tuple_return` | `(a, n)` returned | — | **BUG**: vector-in-returned-tuple crashes (`bug_tuple_vec`) |
| `t2_tuple_in_block` | `return (a, n)` from a block | (moot) | guard sees the tuple-escape (`escapes_value`); shape crashes |
| `bug_tuple_vec` | minimal `(a,5)` return | **CRASH** | `Write to read-only store … CONST_STORE init` — vector literal → const store, tuple write panics. **Separate bug**, not confinement. `(int,int)` tuples are fine. |
| `par3_forpar` | `for p in a par(r=dbl(p),4)` | confined | **parallel is sound** — fused for-par over a confined input vector; guard fires correctly, test passes |

**Outcome of round 2:** aliasing chains and parallel are sound; the only finding is
a real, separate **tuple+vector-return crash** (`bug_tuple_vec` = its minimal repro,
kept as a landmark).  `escapes_value` now also treats a vector as a direct tuple /
literal element so the confinement stays sound once the tuple bug is fixed.

**Still to probe** (per "fire away till 500"): deeper aliasing where the dependent
is itself a block-result temp; `match`-arm destructuring bindings; `parallel { }`
(the explicit form, vs the fused `for…par`).
