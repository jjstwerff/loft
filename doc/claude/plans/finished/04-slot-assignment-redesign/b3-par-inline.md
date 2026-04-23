<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase B.3 follow-up — par() loop variable as inline alias

## Context

The atomic B.3 bundle (`06a8d14`) regresses `wrap::threading` +
`wrap::script_threading`.  Root cause chain:

1. `build_parallel_for_ir` creates `b_var` via `create_var(result_name,
   &b_type)`.
2. `add_variable` is name-keyed, so two par blocks sharing the user's
   loop variable name (`b`) collapse onto **one** `b_var`.
3. Under HEAD's slot-move codegen, the shared slot was repositioned
   at every first-Set and the body-local value lived at that
   transient position.  Type mismatches between pars were silently
   absorbed.
4. Under the atomic bundle's slot-aware `OpPut*` dispatch, the
   shared `b_var`'s declared type drifts with each par and the
   emitted `OpPut*` under-reads or over-reads the eval stack.  Stack
   drift eventually corrupts the `_par_results_N` slot → SIGSEGV on
   the next `OpGetVector`.

The prior attempt (`b3-par-type-lie.md`) tried to fix the declared
type with a second-pass `set_type` override.  It fails because the
variable is shared across pars — `set_type` oscillates and the last
par wins.  See that doc's post-mortem.

## Design — treat `b` as an alias for the element-accessor expression

Under this design `b` is **not a runtime variable**.  It never gets a
slot or a type in `Function::variables`.  Every reference to `b` in
the par body is rewritten (at parse time) to the IR expression that
reads element `idx` from `_par_results_N`:

```
b  →  OpGetField(OpGetVector(Var(results_var), Int(elem_size), Var(idx_var)), fld)
```

For `Reference` and `Vector` returns the `OpGetField` wrap is
skipped:

```
b  →  OpGetVector(Var(results_var), Int(elem_size), Var(idx_var))
```

The result vector (`_par_results_N`) **owns** the elements.  `b` is a
borrowed read — the view semantics match the access pattern.  At
scope exit `OpFreeRef(_par_results_N)` frees the whole vector and
its inline/owned fields, so no per-iteration cleanup for `b` is
needed.  This is exactly the ownership model a borrowed-view
argument has elsewhere in loft.

Memory cost: if the body mentions `b` N times, the IR contains N
copies of the accessor expression → N `OpGetField(OpGetVector(…))`
chains at runtime.  Per the user: "we do not expect many references
to it in the body" — the overhead is acceptable.  For the common
`sum += b` body, N = 1.

## No syntax change

Users continue to write:

```loft
for a in q.items par(b = worker(a), 1) {
    sum += b;
    if b > threshold { log("big: {b}") }
}
```

The alias is entirely internal.

## Why this sidesteps the name-sharing bug

Without a `b_var`, there is nothing to share across pars.  Each par
has:

- its own unique `_par_results_N` (already the case via
  `create_unique("par_results", …)`);
- its own unique `_par_len_N`, `_par_idx_N` (already unique via
  `create_unique`);
- no `b_var` at all.

Two par blocks in the same function no longer interfere.

## Implementation plan

### Step 1 — prevent `b_var` creation

In `src/parser/collections.rs::build_parallel_for_ir` (around line
1284), skip creating `b_var` as a runtime variable.  Parse the
body with the name `result_name` registered as a **parse-time
alias** that resolves to a placeholder variable used only for
body-parse resolution.

Two mechanical sub-steps:

1. Create a placeholder `b_var` via `create_unique(result_name,
   &b_type)` so `b` has a unique internal name (e.g.
   `_b_<counter>`) that cannot collide across pars.
2. Register `result_name` → `b_var` via `self.vars.set_name(result_name, b_var)`
   (same pattern as match-arm aliases at `control.rs:867`).

After body parsing:

3. Call `self.vars.remove_name(result_name)` to release the alias.

At this point, `b_var` exists in `Function::variables` but is named
uniquely and not registered under `result_name`.  The body's IR
still references `Value::Var(b_var)` wherever `b` was used.

### Step 2 — rewrite body IR to replace `Var(b_var)` with the accessor

After `parse_block` returns, walk the body IR and replace every
`Value::Var(b_var)` with a clone of the accessor expression (the
value that `b_assign = v_set(b_var, get_call)` previously set):

```rust
let accessor = if matches!(ret_type, Type::Reference(_, _)) || fn_d_nr == u32::MAX {
    get_vec.clone()
} else if vec_tp != u32::MAX {
    self.get_field(vec_tp, usize::MAX, get_vec.clone())
} else {
    get_vec.clone()
};

// Walk `block` and replace every `Value::Var(b_var)` with `accessor.clone()`.
replace_var_in_place(&mut block, b_var, &accessor);
```

The walk is a plain recursive IR traversal — loft already has
`collect_vars_mut` / similar helpers.  Add a
`replace_var_in_ir(value: &mut Value, target: u16, replacement: &Value)`
helper that matches `Value::Var(target)` and assigns
`*value = replacement.clone()`.

After the walk, `b_var` is referenced nowhere.  `assign_slots` will
see it has no uses and skip placing it (or it will be removed from
the IR before scope analysis).

### Step 3 — drop the `b_assign` / `loop_var(b_var)` calls

The current code emits:

```rust
let b_assign = v_set(b_var, get_call);
let mut lp = vec![stop, b_assign, block, idx_inc];
```

Remove `b_assign` — there is no Set to emit any more:

```rust
let mut lp = vec![stop, block, idx_inc];
```

Also drop `self.vars.loop_var(b_var)` (line 1296) — `b` is not a
loop variable any more.  Its stand-in (`_par_results_N`) already
has the correct liveness.

### Step 4 — handle `skip_free` / `in_use` marks

Current code sets `skip_free` for `ret_type = Reference(_, _)` (line
1307).  Since `b_var` is gone, this call is obsolete — delete.

The `in_use` mark at line 1302 (`Type::Integer | Type::Unknown`)
was a correctness hack for the first-pass placeholder.  Also
delete — no `b_var` to flag.

### Step 5 — single-reference expressions remain cloned

The rewrite substitutes `accessor.clone()` at every `Var(b_var)`
site.  For the expected common case (one or two references in the
body), the bytecode cost is one or two `OpGetVector` + `OpGetField`
chains per iteration — a few bytes of code and a few runtime ops.
Acceptable per user direction.

## Files changed

| File | Change |
|---|---|
| `src/parser/collections.rs` | `build_parallel_for_ir`: swap `create_var` → `create_unique`; parse body under `set_name` alias; post-parse `replace_var_in_ir`; drop `b_assign`, `loop_var`, `skip_free`, `in_use` calls on `b_var`. |
| `src/data.rs` *(or a small helper module)* | Add `fn replace_var_in_ir(val: &mut Value, target: u16, replacement: &Value)`.  Recursive walk covering every `Value` variant's children. |

Total estimated LOC: ~50 (parser edits) + ~40 (IR walker).

## Verification

Steps (all should pass):

1. `cargo check` — clean.
2. Repro fixtures under `/tmp/par_pairs/*.loft` — every pair passes under
   `target/debug/loft --interpret`.
3. `cargo test --test wrap threading` and `script_threading` — green.
4. `cargo test --test issues` — still 500 / 500.
5. `cargo test --test expressions` — still 119 / 119.
6. `./scripts/find_problems.sh --bg --wait` — full suite green.

Regression guards:

- Extend `tests/scripts/22-threading.loft` with cumulative matrix
  coverage (all ordered return-type pairs).  Under the current
  inline-expansion design, all 49 primitive pairs should pass.
- Add a `par_shared_name_distinct_scopes` issue test:

  ```loft
  fn main() {
    q = make_scores();
    sum_int = 0;
    for a in q.items par(b = double_score(a), 1) { sum_int += b; }
    count_pos = 0;
    for a in q.items par(b = score_positive(a), 1) { if b { count_pos += 1; } }
    assert(sum_int == 120 && count_pos == 3, "int + bool par mix");
  }
  ```

## Risks

- **IR-walker correctness** — the recursive `replace_var_in_ir`
  must cover every `Value` variant's children, or references in
  unusual constructs (nested lambdas, match arms, tuple
  destructuring) will leak the un-rewritten `Var(b_var)`.  Plan:
  model the walker on the existing `collect_vars` pattern
  (src/parser/definitions.rs) which is already exhaustive; add a
  debug_assert at the end that no `Var(b_var)` remains in `block`.
- **Nested par blocks referring to outer `b`** — current syntax
  doesn't support this cleanly; inline expansion inherits the same
  limitation.  Verify with a test (nested par should resolve `b`
  to the innermost par's alias) — this is the natural lexical
  scoping already provided by `set_name` / `remove_name`.
- **Body uses `b` in a pattern that expects an lvalue** — e.g.
  `b.field = 5` or `b += 1`.  Today this would be an error (b is
  a temporary loop-local); under inline expansion the error
  message must be understandable.  Verify with a diagnostic test.
  Expected: the error reports on the expanded `OpGetField(…)`
  expression, which is not assignable; a parser pre-check on
  `result_name` usage pattern can emit a friendlier message.
- **Text / reference returns** — the body receives a view into
  `_par_results_N`.  If the body tries to *store* `b` into a
  longer-lived variable (`saved = b`), semantics must be
  copy-by-value.  For text: `saved += b` already copies via
  `OpAppendText`.  For reference: `saved = b` would be a borrow
  assignment.  Under the B.3 bundle's `generate_set` path,
  `Ref = Ref` first-Set goes through `gen_set_first_ref_var_copy`
  (deep copy).  So a body that captures `b` into a named var
  takes a proper copy — safe across the `_par_results_N` free.

## Non-goals

- Supporting `par(b = worker(a), N) { b = something_else; }` —
  writing to `b` is an error today and remains an error.
- Allowing reference to `b` after the par loop exits — `b` is
  lexically scoped to the body; after the loop, the alias is
  removed.
- Optimising repeated `b` references in one iteration (memoising
  the accessor into a single temp).  Deferred until measured
  as a hot path.

## Status

Designed, ready to land.  Recommended as the replacement for the
shelved `b3-par-type-lie.md` fix.

## Related

- B.3 bundle: `06a8d14` (atomic commit on develop).
- Failed type-lie fix post-mortem: `b3-par-type-lie.md`.
- Name-alias mechanism pattern: `src/parser/control.rs:867`
  (match-arm field aliases).
