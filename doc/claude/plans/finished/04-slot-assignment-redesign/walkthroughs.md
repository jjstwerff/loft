<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# V2 Algorithm Walk-throughs

Paper traces of the [SPEC.md § 2 algorithm](SPEC.md#2-algorithm)
applied to every `tests/slot_v2_baseline.rs` fixture.  Each walk-through
carries five columns:

```text
var          live      size  kind     candidate search               chosen
```

where the *candidate search* column records each `candidate` value
the algorithm tries and why it bumps (spatial overlap, kind
mismatch, RefSlot size mismatch, disjoint — reuse OK, etc.).
`chosen` is the final slot `step 7` emits.

**Notation.**
- `[fd..lu]` is the interval `(live_start, live_end)`.
- `Inline` / `RefSlot` is the `SlotKind` from SPEC § 1.
- `size` is the variable's width in bytes.
- `local_start` is the allocator input: bytes reserved for
  arguments + return-address.  `local_start = 4` for every
  walk-through below (the standard function prologue with one
  return-address slot — adjust if the fixture declares arguments).

---

## § 1. End-to-end: P178 — `p178_is_capture_body` (`n_test`)

Locked layout (from `tests/slot_v2_baseline.rs:533`):

```text
block:1
__work_1+24=4 [0..42]
ft+12=28      [3..41]
```

Input to V2:

| var          | live    | size | kind     |
|--------------|---------|------|----------|
| `__work_1`   | `[0,42]` | 24   | `RefSlot` (Text work buffer) |
| `ft`         | `[3,41]` | 12   | `RefSlot` (`P04Tools` ref) |

`local_start = 4` (the `ft_cur == 2` assertion uses a single
text-formatted argument via `__work_1`, so codegen reserves the
usual return-address prefix).

### Sort by `(live_start, var_nr)`

1. `__work_1` (starts at 0)
2. `ft`       (starts at 3)

### Step trace

**Interval `__work_1`**
- candidate = 4.
- placed = [] → no conflicts → chosen = 4.
- `placed += (4, 24, RefSlot, 0, 42)`, hwm = 28.

**Interval `ft`**
- candidate = 4.
- Check against placed `__work_1 (4, 24, RefSlot, 0, 42)`:
  - overlap live? `ft.live_end=41 ≥ 0` and `ft.live_start=3 ≤ 42` → overlap.
  - kind compatible? both RefSlot ✓, sizes 12 vs 24 differ → step 5b
    blocks reuse.
  - spatial overlap? `[4, 16)` ∩ `[4, 28)` → yes → bump.
  - candidate = 4 + 24 = 28.
- Re-scan: no further conflicts → chosen = 28.
- `placed += (28, 12, RefSlot, 3, 41)`, hwm = 40.

### V2 result

```text
__work_1  → 4
ft        → 28
hwm       = 40
```

### Comparison to V1

| var        | V1 slot | V2 slot | match? |
|------------|---------|---------|--------|
| `__work_1` | 4       | 4       | ✓      |
| `ft`       | 28      | 28      | ✓      |

Byte-for-byte match.  `hwm` matches V1 exactly.

### What V2 fixes

The P178 bug's real site is `n_p04_router`, not `n_test`.  In the
router, V1's orphan placer started `tb_id` at slot 0 (the argument
area) before the `local_start` fix landed.  Under V2, `tb_id` is
just another interval in the colouring — its candidate search starts
at `local_start`, and no orphan-specific code path exists.  P178
cannot recur because the "start at slot 0 for orphans" branch has
been retired.

---

## § 2. End-to-end: P185 — `p185_late_local_after_inner_loop`

**The fixture is `#[ignore]`-d under V1** because codegen produces
overlapping slots that trigger `OpFreeText` on a live text buffer
during scope teardown.  Phase 3 un-ignores it after verifying V2
produces a correct layout.

### Reproducing the bug shape

From `tests/issues.rs:10005`:

```loft
fn test() {
    out = file("/tmp/p04_out.txt");
    for f in file("tests/docs").files() {
        path = "{f.path}";
        if !path.ends_with(".loft") or path.ends_with("/.loft") { continue; }
        body = "";
        for i in 0..3 {
            body += "{i}";
        }
        key = path[path.find("/") + 1..path.len() - 5];
        out += `
          {key}
        `;
        break;
    }
}
```

`key` is declared *after* the inner `for i in 0..3` loop completes
but *before* the outer `for f in …` loop's final iteration work
(the backtick `out +=` block).  Under V1, the orphan placer sees
`key` without a Block/Loop enclosing scope and assigns it a slot
that happens to overlap `body`'s still-live Text buffer.  When the
outer loop's scope tears down, `OpFreeText` decrements the refcount
on `key`'s slot — which is actually `body`'s buffer — causing a
use-after-free.

### Input to V2 (conceptual)

| var        | live (approx)      | size | kind     |
|------------|--------------------|------|----------|
| `out`      | `[0, 95]`          | 12   | RefSlot (File handle) |
| `f`        | `[10, 80]` (per-iter) | 12 | RefSlot |
| `path`     | `[15, 60]`         | 24   | RefSlot (Text) |
| `body`     | `[30, 80]`         | 24   | RefSlot (Text) |
| `key`      | `[60, 75]`         | 24   | RefSlot (Text) |
| `i`        | `[35, 45]`         | 8    | Inline |
| `i#index`  | `[33, 47]`         | 8    | Inline |

Exact intervals will be produced by `compute_intervals`; the above
are illustrative.  The key property: `key.live_start = 60` is
**after** `body.live_end = 80`?  No — `body` is still live at
seq 80 because the outer loop uses it in the `out +=` backtick block
(where `{key}` is interpolated).  So `body` and `key` overlap
*spatially and temporally*.

### Trace (sketch)

Walking in `live_start` order, the algorithm places `body` first
(at some slot `S`, e.g. 52).  When it reaches `key`:

- `key` live `[60, 75]`, `body` live `[30, 80]`.
- `60 ≤ 80` and `75 ≥ 30` → live overlap.
- Both RefSlot, both size 24 → kind and size match.
- Spatial overlap test: candidate `S` intersects `body`'s slot
  `[S, S+24)` → **overlap → bump**.
- candidate bumps to `S + 24 = 76` (or the next free slot).
- `key` lands at `76`, distinct from `body`.

**V2 places `key` on a fresh slot above `body`.**  No overlap, no
UAF, no `#[ignore]`.  The fix is *structural*: V2 does not have an
orphan placer whose search starts at `local_start` without seeing
live RefSlot conflicts.  The colouring step's live-overlap check
catches this by construction.

### V2 result (qualitative)

`hwm` for this function grows by one additional Text slot (24 bytes)
compared to the buggy V1 layout.  This is acceptable:

- V1's slot-reusing layout was unsafe (UAF).
- V2's layout is safe and costs 24 B extra stack per function
  with this shape.  The § 5 slack of 32 covers it.

Once V2 lands, the fixture's `.slots(…)` spec is recorded (harvested
from a live run, same workflow as the other 24 passing fixtures)
and `#[ignore]` is removed.

---

## § 3. Sanity check: `zone1_reuse_two_ints_same_block`

Walking the simplest fixture end-to-end confirms the algorithm on a
hot-path case.

Locked layout:

```text
block:1
a+8=4 [1..2]
_+8=4 [3..7]
b+8=12 [5..6]
```

Input:

| var   | live     | size | kind   |
|-------|----------|------|--------|
| `a`   | `[1,2]`  | 8    | Inline |
| `_`   | `[3,7]`  | 8    | Inline |
| `b`   | `[5,6]`  | 8    | Inline |

`local_start = 4`.

### Sort

1. `a` (start 1)
2. `_` (start 3)
3. `b` (start 5)

### Step trace

**Interval `a`**
- candidate = 4.  placed = [] → no conflicts.
- chosen = 4.  `placed += (4, 8, Inline, 1, 2)`.  hwm = 12.

**Interval `_`**
- candidate = 4.
- Check `a (4, 8, Inline, 1, 2)`: `_.live_start=3 > a.live_end=2`
  → disjoint → reuse OK, skip.
- chosen = 4.  `placed += (4, 8, Inline, 3, 7)`.  hwm = 12.

**Interval `b`**
- candidate = 4.
- Check `a`: `b.live_start=5 > a.live_end=2` → disjoint → skip.
- Check `_ (4, 8, Inline, 3, 7)`: `b.live_start=5 ≤ 7` and
  `b.live_end=6 ≥ 3` → overlap.  Kind Inline ✓, size match ✓.
  Spatial overlap on `[4, 12)` ∩ `[4, 12)` → bump.
- candidate = 4 + 8 = 12.
- Re-scan: no further conflicts.  chosen = 12.
  `placed += (12, 8, Inline, 5, 6)`.  hwm = 20.

### V2 result

```text
a  → 4
_  → 4  (reuses a's slot)
b  → 12
hwm = 20
```

Matches V1 byte-for-byte.  ✓

---

## § 4. All-fixture structural rationale

V1 is not V2's reference; invariants I1–I6 from
[SPEC.md § 5a](SPEC.md#5a-invariant-based-verification) are.  For
each fixture below the relevant column is **what structural
property the fixture exercises** — i.e., which invariant would
fail under a buggy allocator.  This is what Phase 2's
`.invariants_pass()` replacement checks.

| # | Fixture | Structural property V2 must preserve | Invariant(s) |
|---|---------|----------------------------------------|--------------|
| 1 | `zone1_reuse_two_ints_same_block` | Disjoint-lifetime Inline integers in the same scope reuse one slot (no stack growth) | I1, O1 |
| 2 | `loop_scope_small_vars_sequential` | Loop-carried vars (`s`, `i#index`) do not share slots with loop-body vars across iterations | I1, I6 |
| 3 | `text_block_return_vs_child_text` | Parent Text buffer and child-block Text buffer cannot share a slot while both live (both RefSlot, size match → only disjoint lifetimes reuse) | I1, I5 |
| 4 | `insert_preamble_lift_ordering` | `__lift_N` temps placed above `local_start`; no overlap with the Insert's target | I1, I2, I4 |
| 5 | `sibling_scopes_share_frame_area` | Two sibling If/block arms' locals may share slots because `scopes_can_conflict` is false | I1 (with scope-tree relaxation) |
| 6 | `sequential_lifted_calls` | Four sequential `__work_N` Text buffers get distinct slots (intervals overlap pairwise via the preamble wrapper) | I1, I5 |
| 7 | `p122p_comprehension_then_literal` | Comprehension iteration temps don't leak out of the comprehension scope | I1, I6 |
| 8 | `p122q_sorted_range_comprehension` | Same as #7 with sorted-range variant | I1, I6 |
| 9 | `p122r_par_loop_with_inner_for` (ignored — par codegen) | Nested loop scopes' locals don't cross-contaminate | I1, I6 |
| 10 | `parent_refs_plus_child_loop_index` | Child loop's index var does not overlap any still-live parent ref | I1 |
| 11 | `call_with_block_arg` | Block-as-argument locals are placed before the Call's target slot (no post-hoc rewrite needed) | I1, I4 |
| 12 | `parent_var_set_inside_child_scope` | A parent-scope local written from inside a child block still gets a slot above `local_start` and below/at the child's frame area | I1, I2, I4 |
| 13 | `p178_is_capture_body` | `is`-capture binder never lands in the argument region | I2 |
| 14 | `p185_late_local_after_inner_loop` *(currently #[ignore])* | Late local declared after an inner text-accumulator loop does not alias the still-live accumulator's slot | I1, I5 (V1 violates I1; V2 passes) |
| 15 | `fn_with_only_arguments` | A sole local placed above `local_start` in a function whose args dominate the frame prefix | I2 |
| 16 | `nested_if_block_branches` | Three If-arm scopes with disjoint lifetimes may share one slot | I1 (with scope-tree) |
| 17 | `large_vector_then_small_int` | Mixing a 12-byte `RefSlot` vector and an 8-byte `Inline` int does not produce a kind-mixed shared slot | I5 |
| 18 | `two_sibling_blocks_shared_outer` | A shared outer var's slot is preserved while two sibling blocks freely reuse their local scratch | I1 (with scope-tree), I4 |
| 19 | `for_loop_two_loop_locals` | Two loop-body locals placed above the loop's carried state; no cross-iteration alias | I1, I6 |
| 20 | `nested_for_in_for` | Inner loop's slots do not alias outer loop's carried `i` / `row` | I1, I6 |
| 21 | `match_with_arm_bindings` | Match-arm binders (in sibling arm scopes) share a slot; scrutinee kept disjoint | I1 (with scope-tree), I5 |
| 22 | `struct_block_return_non_text` | Block-built struct placed on a slot that does not overlap the surrounding locals' intervals | I1, I5 |
| 23 | `nested_call_chain` | Deeply-nested call-chain temps cleaned up before the outer Call's result is placed | I4, I6 |
| 24 | `vector_accumulator_loop` | `acc`'s extended-lifetime interval (via loop-carry) blocks any internal reuse | I1, I6 |
| 25 | `early_return_from_nested_scope` | Early `return` inside a helper does not leak into the caller's slot layout (separate function; invariants hold per function) | I1, I4 per fn |
| 26 | `method_mutation_extends_lifetime` | A vector mutated multiple times has its `last_use` correctly reflected; no later local steals its slot | I1 |

### Reading the table

Each row names the **structural property** — not a specific slot
number — that V2 must preserve.  If V2's output fails that
property, the failure surfaces as an invariant panic from
`validate_slots` with a full variable-table dump (I1–I6 each
have distinct diagnostic text, so the exact failure mode is
obvious without stepping through the algorithm).

A future allocator rework (V3, native-codegen variants, …) is
held to the same bar: these fixtures still apply.  The fixtures
are invariant-based so they survive any valid layout shift.

### Why V2 is safer than V1 on this corpus

V1 has an orphan placer that satisfies I1 by construction on the
cases it encounters, but the fall-through path from
`process_scope` into `place_orphaned_vars` misses variables
hidden under Insert-rooted IR (P178) and under text-accumulator
loop tails (P185).  V2 has no orphan path — every live local
enters the main colouring loop — so I4 is satisfied structurally,
and I1 is satisfied by the colouring's overlap check regardless
of IR shape.

`p185_late_local_after_inner_loop` is the concrete proof: V1
produces a layout that fails I1 (the test would panic on
`validate_slots` if debug_assertions were on during its
compilation); V2 places `key` on a non-overlapping slot.
