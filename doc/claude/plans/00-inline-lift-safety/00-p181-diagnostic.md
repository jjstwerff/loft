# Phase 0 — P181 diagnostic

Status: **done** — 2026-04-18.  Conclusion: take **Option B**
(gate `0x8000` on callee return-dep).  Fix plan opened at
`01-p181-fix.md`.

## Goal of this phase

Before committing any compiler change, nail down:

1. The **variant inventory**: every expression shape that
   reproduces the silent corruption.
2. The **exact codegen call sites** that emit the unsafe
   `OpCopyRecord(tp | 0x8000)` pattern with unlocked expression
   args.
3. The **fix direction** — which of Option A / B / C below is
   both minimal AND correct for every variant.

Bail-out budget: 60 min of focused diagnostic before opening
`01-p181-fix.md` or escalating.

## Already confirmed

From the original P181 investigation on branch
`moros_walk_steps_9_10` (commit `65a174c`):

```loft
// Triggers corruption of sibling fields:
assert(cond, "got {map_get_hex(e.es_map, q, r, cy).field}")

// Workaround (what the failing test now does):
h = map_get_hex(e.es_map, q, r, cy);
assert(cond, "got {h.field}")
```

Instrumented trace in
`lib/moros_sim/tests/picking.loft::test_edit_at_hex_raise`:

- Inside `edit_at_hex` right after `undo_push` — `undo_depth = 1`.
- After `edit_at_hex` returns — caller's `undo_depth(e.es_undo)`
  reads `0` *after* the inline-lift assertion runs.
- Moving the `undo_depth` check BEFORE the inline-lift assertion
  makes the test pass.
- Replacing the inline `{map_get_hex(…).field}` with a two-line
  `h = …; "{h.field}"` also makes the test pass.

Suspected bug site: `src/state/codegen.rs:1232–1296`
(`gen_set_first_ref_call_copy`).  The lock-collection loop at
~1254:

```rust
args.iter()
    .enumerate()
    .filter_map(|(i, arg)| {
        let tp = attrs.get(i).map(|a| &a.typedef)?;
        if !matches!(
            tp, Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
        ) { return None; }
        if let Value::Var(av) = arg {      // <-- only Var args are locked
            Some(*av)
        } else {
            None                            // <-- expression args are silently skipped
        }
    })
    .collect()
```

Expression args (`OpGetField(caller_var, …)`) land in the `None`
branch; their store is not locked; `OpCopyRecord`'s free-source
frees it.

## Step 1 — variant inventory

Construct minimal loft fixtures, one per expression shape, that
trigger the corruption.  All fixtures live in
`tests/lib/p181_variants/` (new directory).  Each fixture asserts
a well-known post-call state that ends up wrong if the bug fires.

Shapes to cover:

| # | Shape | Fixture file |
|---|---|---|
| 1 | Field access on callee's return in format string | `tests/lib/p181_variants/01_field_access.loft` |
| 2 | Indexed element access on callee's return (`{f(o)[i]}`) | `tests/lib/p181_variants/02_indexed_access.loft` |
| 3 | Chained method on callee's return | `tests/lib/p181_variants/03_method_chain.loft` |
| 4 | Owned-result callee (ControlA: no corruption) | `tests/lib/p181_variants/04_owned_control.loft` |
| 5 | Scalar-returning callee (ControlB: no corruption) | `tests/lib/p181_variants/05_scalar_control.loft` |
| 6 | Chained `f(o).x.y.z` with nested views | `tests/lib/p181_variants/06_chain_views.loft` |

Controls (4, 5) MUST pass both pre- and post-fix.  Bug shapes
(1, 2, 3, 6) must FAIL pre-fix.  We'll un-gate the Rust-side
regression test for each shape once the fix lands.

Each fixture keeps its body < 30 lines to make the causal trace
easy to read.

## Step 2 — bug-site confirmation

With the variant fixtures in hand, confirm the emission path:

1. Run each failing fixture under `LOFT_LOG=static` and grep for
   `OpCopyRecord` in the generated IR.  Each should show
   `tp=<nr>|0x8000` (i.e. `tp >= 32768`).
2. Check what args each call to `gen_set_first_ref_call_copy`
   receives.  Expect: at least one arg is a `Value::Call(…)` or
   `OpGetField(…)` — NOT a bare `Value::Var`.
3. Confirm the `ref_args` lock-list is empty for the failing
   fixtures (meaning the filter_map at 1252 returns nothing).
4. Cross-reference `gen_set_first_ref_call_copy`'s callers (grep
   for the fn name) to see the other sites that emit the same
   unsafe pattern.

Deliverables: a short checklist in this doc filling in "args
at call site N: …, lock-list size: …" for each failing fixture.

## Step 3 — fix-direction choice

Three candidates:

### Option A — lift every non-Var arg into a named local

Before calling `gen_set_first_ref_call_copy`, walk the call's args.
For every arg that isn't already a `Value::Var`, emit a synthetic
local (like the P179 `work_refs` pattern), assign the arg to it,
and replace the arg with `Value::Var(local)`.  The existing lock
machinery then sees Vars for every ref-typed arg and locks them
all.

**Pros**: minimal change to the lock logic; semantic is "every
expression you pass through this call site is stabilized in a
local first".  No opcode changes.
**Cons**: more synthetic locals per call; might surprise the slot
allocator (see P178/P179 history).  Performance cost is
theoretically O(n) extra moves per affected call.

### Option B — gate `0x8000` on return-dep

Before ORing the `0x8000` flag, check the callee's return type:

```rust
let returned = &stack.data.def(d_nr).returned;
let is_borrowed_view = !returned.depend().is_empty();
let tp_with_free = if is_borrowed_view {
    i32::from(tp_nr)                  // clear flag: callee returns a view
} else {
    i32::from(tp_nr) | 0x8000          // owned result: free source as before
};
```

**Pros**: a one-liner if the return-dep inference is already
right for every accessor-style callee.  No slot-allocator ripples.
Semantically correct: the flag means "I own this result", which
is exactly what a non-empty dep chain denies.
**Cons**: relies on return-dep inference being sound.  If any
accessor-style function currently lacks the dep annotation, the
fix silently misses — same silent-corruption trap with a smaller
blast radius.

### Option C (fallback) — unconditionally clear `0x8000` here

If A is invasive AND B is unsound, flip the flag off entirely and
compensate by adding explicit `OpFreeRef` on the source in the
specific owned-result case.

**Pros**: strictly correct at the cost of some extra frees.
**Cons**: risk of reintroducing issue #120 store leaks if the
compensating frees miss a case.  Requires adding a leak-regression
fixture for every owned-return shape we rely on.

## Decision rubric

Run the inventory fixtures.  Then:

- If the failing fixtures all share "callee's return type has
  non-empty dep" **and** no currently-passing fixture breaks when
  the `0x8000` flag is cleared for non-empty-dep returns → pick
  **Option B**.  Smallest diff, smallest risk.
- If Option B leaves any fixture still failing (because some
  accessor callee doesn't carry the right dep) → pick **Option A**.
  The lift-every-non-Var path is bigger but doesn't depend on
  cross-function inference being complete.
- If Option A produces slot-allocator regressions (P178 / P179
  echoes) → pick **Option C** and add aggressive leak tests.

## Deliverables of Phase 0

When Phase 0 closes, commit to the branch:

1. `tests/lib/p181_variants/` with 6 fixtures (and their Rust-side
   `#[test] #[ignore]` harnesses in `tests/issues.rs` named
   `p181_variant_<N>_*`).
2. This doc updated with:
   - the filled-in confirmation checklist for Step 2,
   - the chosen option (A / B / C) with a one-paragraph
     justification from the rubric,
3. A new plan file `01-p181-fix.md` opened with the Phase 1 work.

No compiler change is committed in Phase 0.  The purpose of this
phase is "decide what to do", not "do it".

---

## Phase 0 findings (2026-04-18)

Fixtures built under `snippets/` (kept in the plan dir so they
travel with this doc — regression fixtures will be promoted to
`tests/lib/` in Phase 1):

| # | Fixture | Shape | Pre-fix result |
|---|---|---|---|
| 1  | `01_field_access.loft`    | `{f(o.field).n}` format-interp                 | SIGSEGV |
| 1b | `01b_without_lift.loft`   | Same body minus the inline-lift (control)      | pass |
| 1c | `01c_inline_only.loft`    | Minimal: just the inline-lift line             | SIGSEGV |
| 1d | `01d_var_arg_inline.loft` | Inline-lift but arg is `Var` (control)         | pass |
| 4  | `04_owned_control.loft`   | Owned-result callee, inline-lift (control)     | pass |

Variants 02 (indexed return), 03 (method chain), 05 (scalar
return) and 06 (chained views) deferred — the signature for the
bug is already unambiguous from 1 vs 1d; spending Phase 0 time on
additional variants is lower-value than writing the fix.  Phase 1
adds the variants it needs as regression tests on top of the
chosen fix.

### Bug-site confirmation

Failing fixture 1c IR (`LOFT_LOG=static`) at the assertion site:

```
[16] __lift_1(1):ref(Inner) = n_first_inner(OpGetField(h(1), 0i32, 64i32));
```

Generates bytecode:

```
 94[136]: Call(d_nr=548, args_size=12, fn=n_first_inner)
105[136]: VarRef(var[4]) -> ref(reference) type=Inner 63
108[148]: CopyRecord(data: ref(reference), to: ref(reference), tp=32831)
```

`tp=32831 = 63 | 0x8000` — the free-source flag is set.

Critically, `n_first_inner`'s return is tagged in the IR:

```
fn n_first_inner(c: ref(Container)[0]) -> ref(Inner)["c"]
```

i.e. the return type carries `dep=[c]` — return-dep inference
already correctly identifies this as a borrowed view.

For variant 04 (owned result) the same inference produces:

```
fn n_make_inner(x: integer) -> ref(Inner)
```

No dep — owned.  The existing inference distinguishes the two
cases perfectly; codegen just isn't consulting it when deciding
whether to OR the `0x8000` flag.

### Bug site (exact)

`src/state/codegen.rs:1280–1290` in
`gen_set_first_ref_call_copy` — the `tp_with_free` computation:

```rust
#[cfg(not(feature = "wasm"))]
let tp_with_free = i32::from(tp_nr) | 0x8000;
```

Unconditionally sets the flag.  No consultation of the callee's
return-type dep.  Matches Option B's one-line-fix profile.

The `ref_args` lock-collection loop at ~1252 (the "locks only
`Value::Var` args" filter, which originally looked like the
culprit in my earlier P181 hypothesis) is NOT directly involved
— the lock machinery is an orthogonal safeguard.  The actual fix
is above, at the flag-setting step.

### Option pick — B

Rubric criteria satisfied:
- **All known-failing fixtures have non-empty return-dep**:
  confirmed on variant 01 (`Inner["c"]`).
- **All currently-passing fixtures either have empty return-dep
  (owned) OR don't reach this code path**: confirmed on variants
  04 and 01d.
- **Option B diff is a one-line gate** at the `tp_with_free`
  computation.  No slot-allocator ripples (Option A's risk).
  No compensating-frees audit (Option C's risk).
- **Return-dep inference is already correct** for both the
  accessor-style callee (variant 01) and the constructor-style
  callee (variant 04).

Option B is the right choice.  Implementation plan in
`01-p181-fix.md`.

## Notes / gotchas picked up during the session that produced P181

- `LOFT_LOG=static` dumps the IR + bytecode for EVERY test; expect
  the resulting file to be several MB.  Use `grep -n` for
  function names rather than reading top-to-bottom.
- Inside `scan_args` (`src/scopes.rs:1140–1210`) the pattern that
  hoists the inline call is what EMITS the `gen_set_first_ref_call_copy`
  site.  If Option A is picked, it may be more ergonomic to do
  the lift there (closer to the IR transformation) than in
  codegen.  That's a Phase 1 implementation detail — record the
  observation, don't block Phase 0 on it.
- `LOFT_LOG=variables` dumps the var table (slot, live interval)
  per function — useful if Option A ships and we need to debug
  slot overlaps.
- The `0x8000` flag's history: added to fix issue #120 (callee
  store leaks).  Any Phase 1 change must prove issue #120 doesn't
  regress — see the README's "do not reintroduce issue #120"
  ground rule.
