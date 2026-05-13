<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02d — Case B for scalar captures: design analysis

This doc captures the pre-implementation analysis for phase 02d
(scalar — Integer / Text / Float / Single / Boolean / Character /
plain Enum — captures with mutating closures).  The
[02-case-b-design.md](02-case-b-design.md) design deferred 02d
because primitives have no DbRef-to-share shape that 02c's auto-
Reference encoding can leverage.  This doc walks through what 02d
actually requires and proposes the implementation strategy.

## The fundamental difference from 02c

Phase 02c works because Reference captures already store a 12-byte
DbRef pointing at a heap-allocated record.  The closure record's
auto-Reference attribute holds the SAME pointer; mutations through
that pointer reach the same heap record the outer binding points
at.  No outer rewrite needed — the outer binding's `s.x = 7`
already routes through the heap record.

Scalars have no equivalent indirection.  An `Integer` local stores
8 bytes inline on the stack frame.  An `n = 0` followed by `n =
n + 1` reads/writes those 8 bytes directly via stack-slot opcodes
(`OpVarInt`/`OpStoreInt`).  There's nothing to "share by pointer"
because there's no pointer.

For Case B (mutation visible to outer scope), scalars need to be
**boxed** — promoted to a 1-field record.  The outer binding then
becomes a Reference to that record; all reads/writes of the outer
binding route through the box.  When captured, the closure record
holds a copy of the Reference (per 02c's auto-Reference path);
mutations through the closure's view reach the same box.

## The boxing transformation

For each scalar local that's mutated-captured by a closure inside
its scope, the parser would rewrite:

```loft
fn main() {
    n = 0;             // outer-side init
    f = fn() { n = n + 1; };
    f();
    f();
    println(n);        // expected: 2
}
```

into the equivalent of:

```loft
struct __cell_int { value: integer }
fn main() {
    n_box = __cell_int { value: 0 };       // BOXED
    f = fn() { n_box.value = n_box.value + 1; };  // body via box
    f();
    f();
    println(n_box.value);                  // outer reads via box
}
```

The closure body's `n` already gets rewritten to a closure-record
field read in pass 2 (per existing capture machinery).  With the
box transform, the closure-record field type becomes
`Reference(__cell_int, [n_box])` — auto-Reference, share-by-DbRef
(phase 02c's encoding).  Mutations through the closure's view
reach the same box `n_box`; outer reads of `n` (now `n_box.value`)
see the updated value.

## What's hard about this

### (1) Detection happens AFTER the binding is parsed

```loft
fn main() {
    n = 0;             // ← at THIS line, we don't know n will be mutated-captured
    // ...
    f = fn() { n = n + 1 };  // ← discovered HERE
}
```

When the parser processes `n = 0`, the closure literal hasn't been
seen yet.  The decision "should `n` be a box?" can't be made
locally.

**Solution**: a pre-analysis pass that scans the entire function
body for closure literals + their mutated captures, then uses the
result to drive the per-binding boxing decision in the second
pass.  Phase 02a already saved bodies in pass 1; phase 01's walker
populates `mutated_captures` per lambda.  A new aggregation step
collects the union of mutated-scalar names per parent function.

### (2) Every read/write of `n` in the parent body must be rewritten

Every site that emits `OpVarInt(n)` (read) or `OpStoreInt(n, v)`
(write) must instead emit `OpGetInt(n_box, 0)` / `OpSetInt(n_box,
0, v)`.  This affects:

- Direct uses: `n + 1`, `print(n)`, `n = expr`, `n += expr`
- Indirect uses: `n` in format strings, `n` in vector literals,
  `n` as fn-call args, `n` in match patterns, `n` in loop conditions
- Block-result expressions where `n` is the tail value

The set of emit sites that touch a local var by name is
substantial.  The cleanest implementation is to change `n`'s
**type** in the variables table from `Integer` to `Reference(
__cell_int, [n])` and let the existing variable-resolution
machinery emit Reference field reads/writes automatically.

### (3) Initialization at the binding point

`n = 0` doesn't allocate a record; it just stores 0 in a stack
slot.  When n is boxed, the binding needs to allocate a `__cell_int`
record AND set `value` to 0.  The codegen for `Set(n_local, Int(0))`
becomes `OpDatabase(n_local, __cell_int_kt); OpSetInt(n_local, 0, 0)`.

### (4) Drop / freeing

The boxed cell needs to free at scope exit.  The standard
get_free_vars + Reference dep-list mechanism handles this once the
local's type is `Reference(__cell_int, _)`.

## Implementation strategy: 4 sub-phases

### 02d-i — Mutated-captured-scalar accumulation pass (foundation)

In pass 1, after each lambda's `collect_mutated_captures` runs,
push every mutated SCALAR-typed capture's name onto a per-function
`scalars_to_box: HashSet<String>` field on the parent function's
Definition.  No production change yet; the field is populated and
ignored.

Acceptance: Rust-level tests asserting the field has the right
contents for various lambda shapes; 633 issues + 22 closure_matrix
+ 10 mut_closure_matrix + 5 walker tests stay green.

### 02d-ii — Synthesize `__cell_<T>` struct(s)

When `scalars_to_box` is non-empty, ensure a `__cell_<T>` struct
exists for each scalar type used.  One per type: `__cell_int`,
`__cell_text`, `__cell_float`, etc.  Each has a single attribute
`value: T`.  Idempotent registration via `data.find_or_add_def`.

Acceptance: the structs appear in the parsed Data after a snippet
that uses scalar capture mutation.  No behavior change yet.

### 02d-iii — Box at outer-binding time

When parsing a `Set(name, expr)` in the parent function body:
- If `name` is in `scalars_to_box`:
  - Change the variable's type to `Reference(__cell_<T>, [v_name])`.
  - Emit `OpDatabase(v_name, __cell_<T>_kt)` to allocate the cell.
  - Emit `OpSet<T>(v_name, 0, expr_value)` to initialise the
    cell's `value` field.
- Else: emit the existing primitive-store sequence.

This is the major surgery.  The existing primitive-store path in
the parser needs a fork-by-name-membership check.

Acceptance: a snippet `n = 0; f = fn() { n = n + 1 }; f(); f();
println(n);` prints `2` on both backends.  All read/write sites of
`n` (in the outer body, in format strings, in arithmetic, in
function args) work correctly after the type change.

### 02d-iv — Verify closure-side picks up the auto-Reference path

The closure body's capture-name reads/writes already get rewritten
to closure-record field reads (per existing capture machinery).
With the type now being `Reference(__cell_<T>, _)` and 02c's
auto-Reference encoding active for non-empty deps, the closure
body should naturally route through the box.  Verify with cells:

- `b_d1_int_capture_local_mutates`
- `b_d1_text_capture_local_mutates`
- `b_d1_multi_scalar_capture` (2+ different scalars boxed
  simultaneously)
- `b_d2_int_capture_arg_mutates`
- `b_d3_int_capture_field_mutates`
- A leak guard `p22_phase02d_scalar_capture_no_leak`

## Open questions

| Question | Tentative answer |
|---|---|
| Does boxing a `text` capture interact with the existing text-format work-buffer machinery? | Probably yes — text captures already have lifetime-tracking deps that may collide with the boxing dep.  Verify by running a `b_d1_text_capture_local_mutates` cell early. |
| What about `const` parameter captures? | Out of scope — `const` semantically means "no mutation," so the capture is by definition Case A (read-only). |
| Recursive parent functions calling themselves with a closure that boxes `n`? | The box's lifetime is tied to the parent function's scope; recursion creates fresh stack frames with fresh boxes.  No special handling needed. |
| Multi-mixed captures (one scalar boxed + one Reference auto-Reference) in the same lambda | Both mechanisms compose; the closure record has one auto-Reference attribute per mutated capture (boxed scalars and Reference shares look identical to the closure body). |

## Scope estimate

This is genuinely a 4-phase arc, not a single commit.  Compared to
02a-c (3 commits, mostly small additions) phase 02d adds:

- New field on Definition + accumulator pass (smallish, like 02a).
- Synthesise structs (smallish, like ensure_tuple_defs_for_capture).
- **Outer-binding rewrite** — the major work.  Touches the core
  variable-store / variable-load codegen path.  Verification
  needs systematic regression coverage of every primitive-store
  emit site.
- Cells + leak guard (smallish, like 02b).

If 02a-c took ~6 turns combined, 02d should be budgeted at 4-6
turns.

## Recommendation

**This turn**: ship the design doc only (this file).  Phase 02d-i
(the foundation accumulator) lands as the next focused turn; the
outer-binding rewrite is the largest sub-phase and deserves its
own pre-implementation analysis pass (likely a 02d-iii-design.md
analogous to 02-case-b-design.md).

**Alternative path** worth considering: ship phase 03 (Case C —
factory pattern) BEFORE 02d.  Case C for scalars is structurally
simpler than Case B for scalars because the closure escapes the
scope and outer-side reads aren't a concern — the closure can own
its boxed cell exclusively.  The boxing mechanism developed for
Case C scalars then becomes the foundation for Case B scalars
(02d).  This sequencing matches the underlying lattice: Case C is
a structurally weaker requirement than Case B (no outer-side
visibility needed).

## Cross-references

- [02-case-b-design.md](02-case-b-design.md) — design that
  deferred 02d.
- [03-case-c.md](03-case-c.md) — Case C plan (factory pattern);
  scalars there could share infrastructure with 02d.
- `src/parser/objects.rs::resolve_name` — capture-context arm
  (phase 01 + 02c).
- `src/parser/vectors.rs::synthesize_closure_record` — where
  auto-Reference attribute types get set (phase 02c).
- `src/typedef.rs::fill_database` — auto-Reference layout decision
  (phase 02b).
- `src/generation/mod.rs::emit_field` — native auto-Reference
  layout (phase 02c P258 fix).
