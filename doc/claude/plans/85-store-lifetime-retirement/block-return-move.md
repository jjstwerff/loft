<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 residual cluster — move-on-block-return

**Status: active (reopened 2026-07-09).** An inline block that returns a
locally-allocated struct — `x = { z = P{…}; z }` — **borrows** instead of
**moving**. It violates ownership invariant #2 (*move on return*) and, via slot
reuse, #1 (*single owner*). Surfaced by @PLN47 (`f#read as Struct`). This doc is
the stepped implementation plan; it uses an **oracle/switch** migration so the
change to load-bearing ownership classification lands without a suite regression.

## The defect, precisely

`x = { z = P{…}; z }` produces a block whose tail value is the fresh block-local
`z`. The parser classifies the block value with a dep (`x["z"]`) — a *borrow* —
and codegen `PutRef`-aliases `z`'s store into `x`. `x` never becomes the owner;
`z` is the block's return var so `get_free_vars` skips it; nobody frees the store.

The correct behaviour is the inline-block analogue of what a function return
already does (`Definition::return_adopts_fresh_store` → move via a return
buffer): **the LHS adopts the fresh block-local's store (empty dep); the local is
moved out, not freed, not borrowed.**

Two failure modes, one root cause:

| Mode | Repro | Today | Correct |
|---|---|---|---|
| **Leak** | sibling blocks reusing a local name / same site in a loop | 1 store leaked per occurrence | freed once |
| **Corruption** | `a={z=P{1,2};z}; b={z=P{3,4};z}; print(a,b)` | `3 4 3 4` (b's local reuses a's still-live slot) | `1 2 3 4` |

## Probe matrix — the executable spec

[`probes/block-return-move/`](probes/block-return-move/) — baseline captured
2026-07-09, both backends (interp + native identical):

| Probe | Shape | Baseline | Target |
|---|---|---|---|
| `p1-sibling-reused-name` | two sibling blocks, local `z` reused | ❌ leak 1 | ✅ no leak |
| `p2-same-scope-used-after` | `a`,`b` bound then both read later | ❌ `3 4 3 4` (corruption) | ✅ `1 2 3 4` |
| `p3-loop-single-site` | pure block-temp in a loop | ✅ no leak (boundary) | ✅ unchanged |
| `p3b-file-read-loop` | `f#read as P` write+read each iter (PLN47) | ❌ leak N | ✅ no leak |
| `p4-distinct-names` | distinct local names | ✅ clean (control) | ✅ unchanged |
| `p5-fn-return` | `a = mk(v)` | ✅ clean (the move we mirror) | ✅ unchanged |
| `p8-negative-borrow-outer` | `a = { base }` returns an OUTER var | ✅ `9 8` (must stay a borrow) | ✅ unchanged |

**p8 is the guard**: the fix must move ONLY a fresh block-local; a block that
returns an outer var / param / field / element is a genuine borrow and must keep
its dep. Over-moving p8 would free `base` while the outer scope still owns it —
turning a leak into a use-after-free. Every step re-checks p8.

## The invariant (the ONE fact to compute)

> A block's tail value that is a **fresh local defined inside that block** and
> owns its store ⇒ the block value is **owned** (empty dep); the consuming
> binding **adopts** it and frees it once at its own scope exit. A tail value
> that **references a binding defined outside the block** (param, outer local,
> field, element) ⇒ the block value **borrows** that binding (dep preserved);
> the binding's owner frees it.

"Fresh local defined inside the block" = the tail `Var(w)` where `w` is
first-assigned/allocated within `bl.operators` and `w` is not in any enclosing
scope. This is the block-level twin of `return_adopts_fresh_store`.

## Chokepoint

One place decides the block value's own-vs-borrow — do not spread the fix.
**Reuse the existing oracle, do not invent one:** the canonical fact is
`crate::use_analysis::ownership_of(data, d_nr, value)` (the @PLN90 D-own-1
oracle, already default-on at `scopes.rs:2977` with 0/54 over-free), and the
block-value classifier is:

- **`src/parser/control.rs` `classify_reference_delivery`** (L1201; called from
  `block_result` ~L1013-1026): decides whether a block/tail Reference value is
  delivered as owned or borrowing. This is where "tail is a fresh block-local ⇒
  owned (empty dep, adopt)" must be added — as a fact the `ownership_of` oracle
  reports, NOT a new per-site heuristic.
- Supporting: **`src/scopes.rs` `Value::Block`** hoist (~L2343-2406, the
  first-occurrence-only `!var_scope.contains_key` gate) and `get_free_vars`
  (skips the return var) — the adopt path must make the LHS own + free once, and
  the moved local must not be double-freed.
- The fn-return twin already computed: `Definition::return_adopts_fresh_store()`
  / `returns_borrowed_view()` (see [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md)).

## Implementation steps — oracle/switch migration

The classification change is load-bearing (every block-valued assignment,
every loop, every `match`/`if` arm). Land it behind a switch, validate with an
oracle that the OLD behaviour is preserved on everything except the buggy
shapes, then flip and delete. This reuses the SAME oracle pattern @PLN85 already
built for D-own-1: `use_analysis::ownership_of` computes the canonical fact and a
gated comparison against the legacy path catches divergence (0/54 over-free at
close). The block-return fact is one more input to that same oracle — the switch
gates the new fact, the oracle is the existing `ownership_of` cross-check plus
the interp-vs-native cross-mode parity.

### Step 0 — probe matrix + baseline ✅ DONE
Probes above committed; baseline recorded (both backends). Prove the harness can
fail: p1/p2/p3b are red today.

### Step 1 — the switch (no behaviour change yet)
Add a single gate `move_block_return` read ONCE from env `LOFT_BLOCK_MOVE` into a
parser/`Data` flag (like the existing diagnostic toggles). Thread it to the two
chokepoint sites. Default **OFF** ⇒ byte-identical IR to today. Gate:
`loft introspect` on a probe must emit the SAME IR with the switch off (prove the
scaffold is inert).

### Step 2 — the escape-analysis fact
Implement `block_tail_adopts_fresh_local(bl, function) -> bool`: the tail is
`Var(w)`, `w` is defined within `bl.operators` (has a `Set`/`OpDatabase` before
the tail), `w` is owned (Reference/Vector/Enum, empty pre-existing dep), and `w`
is NOT registered in an enclosing scope. Unit-test it against all 7 probes
(p1/p2/p3b true; p4/p5 owned-elsewhere; **p8 false** — `base` is an outer local).

### Step 3 — the adopt path (behind the switch)
When `move_block_return` AND the fact holds: `block_result` emits the block value
with **empty dep** (owned); scopes makes the LHS the owner (adopt, freed at LHS
scope) and marks the moved local skip-free / suppresses its own free. When OFF,
the current borrow path is untouched.

### Step 4 — the oracle gate
Extend the `cross_mode` harness with a `oracle_block_move!` cell that runs each
probe THREE ways and asserts:
1. **switch-ON interp == switch-ON native** (cross-backend parity — the primary
   oracle), and
2. **switch-ON fixes the probe** (p1/p2/p3b: no leak, correct value), and
3. **switch-ON == switch-OFF on the CONTROLS** (p4/p5/p8 unchanged — no
   over-move; p8 must stay a borrow, no UAF).
Then run the WHOLE suite twice (switch off, switch on) and diff: the only
permitted differences are the buggy shapes turning correct. Any other diff is a
regression to root-cause before proceeding. This is the oracle — the old
behaviour is the reference for everything that was already correct.

### Step 5 — flip the default
Make `move_block_return` default **ON**; the switch now toggles OFF for
fallback/bisection. Full suite green on both backends, leak-clean. Graduate
p1/p2/p3b/p8 to `tests/scripts/` (or `tests/binary_io_matrix.rs` for p3b) as
regressions. Update the @PLN47 struct-read "known limitation" note → fixed.

### Step 6 — burn-in + delete
After a clean cycle with default-ON, delete the OLD borrow path and the switch
(single-owner, one code path). Move any reference content to OWNERSHIP_MODEL.md;
close this cluster on the @PLN85 issue.

## Acceptance

- p1/p2/p3b green (no leak, correct values) on BOTH backends; p3/p4/p5/p8
  unchanged.
- Full suite green + leak-clean with the switch defaulted ON.
- Old borrow path + switch deleted (Step 6).
- @PLN47 struct-read leak note retired.

## Cross-references

- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the deps beacon + invariants #1/#2.
- [@PLN47](../47-binary-io-validation/README.md) — surfaced this via `f#read as Struct`.
- `src/parser/control.rs` `block_result` · `src/scopes.rs` `Value::Block` /
  `scan_set` / `get_free_vars` · `Definition::return_adopts_fresh_store`.
