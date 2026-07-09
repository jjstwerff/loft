<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 residual cluster — move-on-block-return

**Status: DONE / CLOSED (2026-07-09).** An inline block that returns a
locally-allocated struct — `x = { z = P{…}; z }` — **borrowed** instead of
**moving**, violating ownership invariants #2 (*move on return*) and #1 (*single
owner*): it leaked and, via slot reuse with a still-live consumer, corrupted
(`a` read `b`'s value). Fixed via the oracle/switch migration below — **all 6
steps done**: the fix is now UNCONDITIONAL (switch deleted, Step 6), full suite
**2721/2721** on both backends, regression `tests/scripts/85-block-return-move.loft`.
Surfaced by @PLN47 (`f#read as Struct`).

**Separate residual split out — `p9`** (NOT block-return-move): `f += struct`
then `f#read as struct` leaks the read buffer on **interp only** (native clean),
struct-specific. Root-caused to an interp slot-reuse bug — the read result var
`q` reuses the write `_wf` temp's stack slot; at the free site `q`'s slot holds
`_wf`'s stale (freed) DbRef `#5` rather than the read record `#2` the `PutRef`
wrote, so the free no-ops on the dead store and `#2` leaks (needs runtime
step-debugging; a naive write-path copy-skip CORRUPTS the write). A write-path +
slot-allocator (SLOTS.md) cluster, tracked as its own item — see § below.

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

- **`src/parser/control.rs` `block_result`** (L723): the UNIFORM block-value
  chokepoint — `parse_block` calls it (L608) for every block, return AND local
  assignment. `classify_reference_delivery` (L1201) is only its
  `context == "return from block"` sub-path (delivery into `__retbuf`); the
  LOCAL-assignment dep (`a["z"]`) comes from the tail-expression typing that
  `block_result` returns for `context == "block"`. Step 2 pins the exact line
  that attaches the local dep; the adopt decision ("tail is a fresh block-local
  ⇒ owned, empty dep") belongs here — as a fact the `ownership_of` oracle
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

### Step 1 — the switch (no behaviour change yet) ✅ DONE
`use_analysis::move_block_return()` reads env `LOFT_BLOCK_MOVE` (mirrors
`env_tier`). It is `pub` (lib API surface ⇒ no dead-code lint) and has ZERO
callers in the compile pipeline ⇒ provably inert. Verified: warm-vs-warm
`loft introspect` on `p1` is byte-identical with the flag OFF vs ON (the naive
fresh-vs-cached first diff was `.loft/cache` state, not the flag). Behaviour
with the flag ON is unchanged (p1 still leaks — the adopt path is Step 3).

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

### Step 4 — the oracle gate ✅ DONE
Full suite run with `LOFT_BLOCK_MOVE=1`: **2721 passed, 0 failed, 217 skipped**
(the flaky `engine_host_kernel` pair passed too). The switch-ON path is green
across the whole suite — the fix does not regress anything. **Cache caveat**
(cost hours here): the per-script `.loft/cache` keys on SOURCE hash, not the
binary, so MANUAL `./loft` runs can replay stale bytecode across a rebuild/flag
change — always use `LOFT_NO_CACHE=1` (or fresh `.loft`) for manual A/B. The
test suite runs under Cargo, where the cache is OFF automatically, so the oracle
is authoritative. Verified authoritatively (cache-off, both backends): default
fixes p1 (leak→none) + p2 (`3 4 3 4`→`1 2 3 4`); `=0` restores both.

### Step 5 — flip the default ✅ DONE
`move_block_return` now defaults **ON** (`LOFT_BLOCK_MOVE=0` = legacy fallback),
mirroring the shipped `ownership_of` default-on. Regression:
`tests/scripts/85-block-return-move.loft` (minimal p2 shape — corruption
manifests deterministically only in a stable minimal layout; asserts catch it,
the suite leak-gate catches p1). It passes with the fix and fails under `=0`
(`assertion failed: a.x moved…`), both backends. @PLN47 struct-read "known
limitation" → retired (read-only reads never had the bug; the write+read leak is
`p9`, a separate residual).

### Step 6 — delete the switch ✅ DONE
Full suite green on BOTH switch states (2721/2721) was sufficient burn-in
evidence, so the gate was retired immediately: `block_result`'s branch is now
unconditional and `use_analysis::move_block_return` is deleted — single owner,
one code path. Suite re-run green (2721/2721) with the switch gone. Regression
`tests/scripts/85-block-return-move.loft` still passes both backends.

## `p9` — separate write+read struct residual (OPEN, its own cluster)

`f += s; f#read as S` in one program leaks the read buffer on **interp only**
(native clean), struct-specific, pre-existing. **Not** block-return-move: the
read block-return in isolation is clean. Root cause (bytecode + free trace,
cache-off): the read result `q` reuses the write `_wf` temp's stack slot
`[80,92)`; that slot is NOT re-`InitRef`-ed at read-scope entry, so it enters
holding `_wf`'s freed DbRef `#5`. The read emits `PutRef(q, #2)` then
`FreeRef(q)`, but at runtime the free reclaims `#5` (already-freed → no-op) while
the read record `#2` leaks — i.e. the `PutRef` value does not survive to the
free on a reused-but-uninitialised slot. Needs runtime step-debugging (loft
debugger); a naive write-path copy-skip CORRUPTS the write (`OpCreateStack` on a
struct var mis-serialises), so it is deferred as its own write-path +
slot-allocator ([SLOTS.md](../../SLOTS.md)) investigation. Probe:
[`probes/block-return-move/p9-writeread-slot-leak.loft`](probes/block-return-move/p9-writeread-slot-leak.loft).

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
