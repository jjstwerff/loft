<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Lightweight expand-walker — the minimal total tree/graph walk (@PLN86)

A trusted, total, deterministic primitive that lets **sandboxed** code do what a
recursive routine does over a tree or graph — *without recursion* and *without any
admission change*. This is the minimal version: the smallest feature set worth
building so we can experiment against real cases. Everything optional is
[deliberately dropped](#deliberately-dropped) below; add it back only when a case
demands it.

It supersedes the planned 3.2b "structural recursion analysis": rather than teach the
admission checker to prove an arbitrary recursion terminates, we hand the modder one
budgeted primitive that *owns* termination, so their per-node code stays in the
already-admitted non-recursive subset.

## Signature

```loft
fn walk<N>(root: N, expand: fn(N) -> [N], max_steps: integer) -> iterator<N>
```

Used as an ordinary bounded `for` (which the dialect already admits):

```loft
for n in walk(root, |x| { x.children }, 10000) {
    // ... ordinary, non-recursive per-node work ...
}
```

- `root` — the start node.
- `expand` — given a node, returns the next nodes to descend into. **The modder writes
  this**; it is a normal non-recursive function, so it needs no new admission rule. It
  is also where the *edges* are chosen (see [why expand](#why-expand-not-field-generic)).
- `max_steps` — the hard cap on how many nodes the walk yields. **This is the entire
  termination guarantee** (see [the invariant](#the-one-invariant)).

## Semantics — the whole contract

- **DFS pre-order, deterministic.** A node is yielded *before* the nodes `expand`
  returns for it; `expand`'s list is consumed **left-to-right**. Same input → same
  sequence, every run, on either backend. (Determinism matters: game replays, lockstep
  netcode, reproducible debugging.)
- **Null entries are skipped.** `expand` may return a list containing `null` (a direct
  ref like `[x.left, x.right]` is `null` at a leaf). The walk skips nulls rather than
  yielding a null node.
- **The budget ends the walk.** The iterator stops after it has yielded `max_steps`
  nodes, or when the frontier empties — whichever comes first. Hitting the budget is a
  **clean stop**, never an error or abort (consistent with the no-runtime-abort model);
  the `for` loop simply ends.
- **No dedup.** A node reachable by two paths is yielded **twice**. This is faithful to
  recursion (which revisits shared subtrees) and is what path-enumeration /
  state-search / instanced-scene-graph walks need. Termination does *not* depend on
  dedup — the budget covers it.
- **Sequential.** No parallelism, so order is stable.

## The one invariant

> The iterator yields **at most `max_steps`** nodes, so the walk always terminates —
> regardless of whether `expand` reads a pre-existing structure or **generates** nodes
> on the fly (procedural trees, game/minimax search, lazy expansion).

This is why the budget, not the structure, is the guarantee: a generative `expand`
makes "the structure is finite" false, and a cyclic structure makes it false, but
`max_steps` bounds the walk in both cases unconditionally.

**Admission rule** (when `walk` is called from sandboxed code): `max_steps` must be a
literal or a host-bounded value — the same constraint already applied to loop bounds
and the decreasing-variant `while`. A budget the script can inflate is not a budget.
With that, the `walk` call contributes `O(max_steps)` to the complexity degree (3.4)
and the data envelope (§8) sizes memory from it. The modder's `expand` is a
non-recursive, total function, so it is admitted by the existing rules; the entire
recursion-shape lives inside the trusted primitive.

## Algorithm (reference)

An explicit LIFO stack — no recursion in the implementation either:

```
push root
yielded = 0
while stack not empty and yielded < max_steps:
    n = pop
    if n is null: continue
    yield n; yielded += 1
    for c in reverse(expand(n)):   # reverse so the leftmost child pops first
        push c
```

Deterministic (pre-order, left-to-right), bounded by `max_steps` yields.

## Why `expand`, not field-generic

A walker that followed *all* reference-typed fields can't tell a structural child-edge
(`children: [Node]`, descend) from a back-pointer or cross-link (`parent: Node`, do
not), and the differing shapes (vector vs direct ref) compound it. `expand` puts that
choice at the call site: the modder normalises whatever fields they mean into one list
of next-nodes —

```loft
walk(root, |x| { x.children },            B)   // vector field
walk(root, |x| { [x.left, x.right] },     B)   // two direct refs (nulls skipped)
walk(root, |x| { x.children + [x.overlay] }, B) // mixed shapes, combined
```

— and a back-pointer simply never appears in what `expand` returns, so the walk stays
in the intended subtree.

## Cases to experiment with

- **child-vector tree:** `walk(root, |x| { x.children }, B)`
- **binary tree, direct refs:** `walk(root, |x| { [x.left, x.right] }, B)`
- **ignore a back-pointer:** `walk(node, |x| { x.children }, B)` — `parent` not returned
- **mixed-shape self-links:** `walk(root, |x| { x.children + [x.overlay] }, B)`
- **generative / "infinite" tree:** `expand` builds children on demand; the budget
  bounds it. Depth cap lives in `expand`: `|x| { if x.depth < 5 { gen(x) } else { [] } }`
- **DAG / cyclic graph:** revisits happen (no dedup), budget still bounds it.

## Cost

- **Time:** `O(max_steps)` `expand` calls → contributes `O(max_steps)` to the sandbox
  complexity degree.
- **Memory:** peak frontier ≈ `O(max_steps × max fan-out of expand)`. For the data
  envelope, the host bounds `max_steps` and the per-node fan-out.

## Deliberately dropped

Kept out of the minimal version on purpose — add back only when a real case needs it:

- **dedup / visited-set** — revisits are allowed; the budget, not a seen-set, guarantees
  termination. (Re-add as an opt-in `dedup` for once-only/memoised graph walks.)
- **context threading** — `expand(node, ctx) -> [(node, ctx)]` for path-dependent walks
  ("take a different path from here"). The context-free form is the special case.
- **`#child` annotation sugar** — letting `walk(root)` work with no `expand` for the
  common single-structural-field type. Always pass `expand` explicitly for now.
- **profile-default budget** — a `walk_budget` in the `[sandbox]` policy so the call
  site omits `max_steps`. Always pass it explicitly for now.
- **post-order `fold` combinator** — `fold(root, expand, combine)` for bottom-up
  catamorphisms (evaluate an expression tree, subtree sizes). This minimal walker is
  pre-order visit/iterate only.
- **truncation signal, BFS/`order:` option, parallel walk.**

## Build note

Simplest first cut is a **trusted stdlib primitive** (sandboxed code calls it; it is
not itself sandboxed). Two ways to prototype:

- **loft-bodied** (fastest to iterate): a function in a small lib that runs the
  worklist loop above — trusted code may write the worklist `while` that admission
  rejects for sandboxed code. Generic over `N` via the existing generic-fn machinery.
- **native `#rust`**: a `#rust`-bodied iterator if the loft-bodied version is too slow.

Start loft-bodied, experiment across the cases above, then decide whether to go native
or to add any of the dropped features.
