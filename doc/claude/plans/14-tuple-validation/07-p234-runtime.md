<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 07 — P234 runtime: tuple-with-Reference function returns

**Status: open (lexer half closed 2026-05-07; runtime half is this phase)**

## Goal

Make `fn make() -> (Point, integer) { p = Point{...}; (p, 5) }`
return correctly under both `--interpret` and `--native`.  Today
`r.1` reads as `0` and `r.0.x` as `null` under native (interp
accidentally works via eval-stack byte retention).

The bug is the `Type::Tuple` return type itself: native compiles
`fn make() -> (DbRef, i64)` but the IR has the tail tuple as a
bare statement (`(var_p, 5_i64);`) followed by `return null` —
Rust discards the bare tuple and returns the null sentinel.

## Why a phase here

Plan-14's matrix already covers element × destination, but
**function-return-as-destination** for Reference-bearing tuples
isn't represented.  Phase 04 (`04-references.md`) handles
References as tuple ELEMENTS in storage destinations, not as
return values.  This phase closes the gap.

## Approach: route through synthetic struct, gated by lifetime concern

Loft already allocates a synthetic `__tuple<T1,T2,…>` struct via
`data.tuple_def(...)` (`src/data.rs:2397`) for `vector<(T1,T2)>`
element storage and other "stored tuple" sites (P189b).

The unified principle:

> **If any part of the tuple has a lifetime concern (heap-owning
> element), rewrite the function's return type from
> `Type::Tuple(elems)` to `Type::Reference(synthetic_tuple_d_nr)`.
> Otherwise use Rust's tuple ABI.**

Lifetime-bearing types are exactly the ones that go through
`text_return` / `ref_return` as a direct function return today:
Text, Reference, Vector, Enum-struct, Sorted / Hash / Index /
Spacial keyed collections, RefVar.  Plus tuples that recursively
contain any of those.

The function then returns a DbRef pointing to a heap-allocated
synthetic struct — exactly like `fn make() -> Point` returns
work today.  Caller-side element access (`r.0`, `r.1`) already
handles `Reference(__tuple<…>)` via `get_val` at the synthetic
struct's per-attribute byte offset
(`src/parser/operators.rs:608-658`).

| Return type | Path |
|---|---|
| `(integer, integer)`, `(integer, boolean)`, `(int, character)`, etc. — every element pure value | Existing Rust-tuple ABI |
| `(text, text)`, `(integer, text)`, `(Point, integer)`, `(vector<T>, X)`, `(Variant{x:int}, T)`, `((Point, int), text)`, etc. — any element heap-owning OR a nested tuple containing one | NEW: route through `Reference(__tuple<…>)`.  Fixes P234. |

Two buckets, one predicate, no special cases:

```rust
fn has_lifetime_concern(t: &Type) -> bool {
    matches!(t,
        Type::Text(_)
        | Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Enum(_, true, _)            // struct-enum payload
        | Type::Sorted(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Index(_, _, _)
        | Type::Spacial(_, _, _)
        | Type::RefVar(_)
    ) || matches!(t, Type::Tuple(elems) if elems.iter().any(has_lifetime_concern))
}
```

**T1.8a's text-tuple machinery becomes superseded** by this
unified gate.  After this phase, `(text, text)` returns also
route through the synthetic struct — text fields stored as
4-byte string offsets, lifetime tracked via the existing
text_return deps machinery.  T1.8a's special-case code in
`src/generation/{mod.rs:366-373, emit.rs:206-210, dispatch.rs}`
becomes redundant.  This phase can either retire it (cleaner) or
leave it as dead-but-tested fallback (safer); decide during
Step 5 verification.

**Why a unified gate is right:**

1. **Predictability.**  Two buckets — "owns heap" → store, "pure
   value" → Rust ABI.  No "this works but that doesn't" lookup
   table.
2. **Retires special-case code.**  T1.8a was added because the
   bare-tuple-ABI couldn't handle text ownership cleanly.  The
   unified gate makes text just another field in the synthetic
   struct — uses standard text-in-store semantics.
3. **Recursive correctness.**  Nested tuples containing any
   heap-owning element auto-recurse via the predicate.
   `((Point, int), text)` works without separate machinery.
4. **Future-proof.**  Any new heap-owning type added to loft
   automatically lands in the right bucket as long as it's added
   to `has_lifetime_concern`.

## Steps

### Step 1 — Parser-side return-type rewrite

`src/parser/definitions.rs::parse_function`: after the function's
declared `returned` type is resolved, post-process using
`has_lifetime_concern`:

```text
if returned is Type::Tuple(elems) and elems.iter().any(has_lifetime_concern):
    let synthetic_d_nr = data.tuple_def(lexer, elems);
    returned = Type::Reference(synthetic_d_nr, vec![]);
```

`data.tuple_def` is idempotent (returns existing d_nr if already
registered) — safe across two-pass parsing and across multiple
call sites with the same shape.

After this rewrite, `definitions[ctx].returned` carries
`Reference(__tuple<…>)`.  All existing Reference-handling code
paths fire unchanged.

### Step 2 — Body-tail tuple-literal rewrite

`src/parser/control.rs::parse_code` (or wherever the function
body's tail expression is finalised, before scope analysis)
detects:

```text
returned is Type::Reference(d_nr) where def(d_nr).name starts with "__tuple<"
   AND tail expression is Value::Tuple(elements)
```

and rewrites the tail to allocate the synthetic struct and write
each element via `OpSet*` at the right offset:

```loft
{
    __tuple_ret = NewSynthetic(__tuple<Point,integer>);
    __tuple_ret._0 = elements[0];   // OpSetRef at offset 0
    __tuple_ret._1 = elements[1];   // OpSetInt at offset 16
    return __tuple_ret;
}
```

This mirrors how loft compiles struct literals today — same IR
primitives.

### Step 3 — Caller-side element access (already works)

The existing P189b path in `src/parser/operators.rs:608-658`
handles `Reference(__tuple<…>)` element access via `get_val` with
the synthetic struct's per-attribute byte offset.  When the
caller writes `r = make(); r.1`, the access is parsed as
`OpGetInt(r, 16)` automatically.

No new code in the access path.

### Step 4 — Ownership transfer (already works)

The existing `Type::Reference` arm in
`src/parser/control.rs::block_result` (line ~431) calls
`ref_return(ls)` for Reference returns.  After Step 1, our
function's return type IS `Reference(__tuple<…>)` — the existing
arm fires.  `ref_return` promotes local Reference vars (the
inner `p` in our repro) to hidden function parameters, so the
caller pre-allocates Point storage and passes its DbRef.

No new ref_return code.

### Step 5 — Interp padding fix (revisit)

The uncommitted interp codegen fix on `quality-pass`
(`Value::Tuple` padding, `emit_tuple_put_ops` FreeStack,
`emit_tuple_var_pop_put` FreeStack) addresses tuple expressions
on the eval stack.  After Steps 1-2, tuple-with-Reference
returns no longer produce eval-stack tuple expressions — the
rewrite uses `OpSet*` to a heap struct directly.

So the padding fix is **redundant for the P234 path**.

**Verified 2026-05-08**: padding fix is genuinely redundant.
Full suite passes without it (issues 611/0, threading_chars 44/0,
tuple_matrix 17/0, native 5/0).  Reverted.

### Step 6 — Retire superseded T1.8a code paths

After Steps 1-2 ship, T1.8a's tuple-of-text RETURN handling
becomes dead — function returns of `(text, …)` get rewritten to
`Reference(__tuple<…>)` before reaching emit, so the
`Value::Return` text-tuple detection in
`src/generation/emit.rs:205-211 + 235` never fires.

Retire the dead-but-still-present block:

- **`src/generation/emit.rs`** Value::Return arm: remove the
  `prev_tuple_text` save + `tuple_text_to_string = true` setting
  + restore for Tuple-with-Text returns.  Replace with a
  one-line comment pointing at Plan-14 phase 07 as the
  superseding mechanism.

What stays alive (still load-bearing):

- `output_set`'s `tuple_text_to_string` handling
  (`src/generation/dispatch.rs:295-359`) — fires for LOCAL tuple
  vars of `Type::Tuple([Text, …])` (e.g. `t = ("a", "b")` as a
  local).  Local tuple vars are NOT rewritten by this phase
  (which only touches function returns).
- `output_set`'s `tuple_text_elem_clone` (`dispatch.rs:336-353`)
  — same reason: local tuple-with-text element reads.
- `rust_type` Result→Variable recursion (`mod.rs:366-374`) — now
  defensive only (Tuple Result-context never carries Text after
  rewrite), but harmless — leave with a doc-only update noting
  it's superseded for the text case.

A future cleanup could extend Plan-14 phase 07 routing to LOCAL
tuple-with-lifetime-concern variables too, retiring
`output_set`'s entire tuple handling.  Out of scope for this
phase — the local-tuple ABI works correctly today; only the
function-return ABI was broken.

### Step 6 — Regression coverage

- `tests/issues.rs::p234_runtime_*` regression tests using `code!`
  (one already added on `quality-pass`; add cross-mode tests via
  `cross_mode!` so binary path is covered too)
- Verify `tests/threading_chars::par_tuple_return_struct_text`
  (currently ignored under ARC.md A7.3) becomes pass-able; if so
  un-ignore.

## Files

| File | Change |
|---|---|
| `src/parser/definitions.rs` | Step 1: post-process `returned` for Tuple-with-Reference (~10 LoC) |
| `src/parser/control.rs` | Step 2: tail-tuple-literal → struct-build rewrite (~20 LoC) |
| `tests/issues.rs` | `p234_runtime_*` regression tests (one already added; add cross-mode) |
| `src/state/codegen.rs` | Step 5: keep or revert padding fix based on verification |
| `doc/claude/PROBLEMS.md` | Mark P234 fully closed (lexer + runtime) |
| `doc/claude/plans/06-typed-par/ARC.md` | A7.3 status: lexer + runtime closed; verify `par_tuple_return_struct_text` un-ignorable |

## Existing infrastructure to reuse

- **`data.tuple_def(lexer, elems)`** at `src/data.rs:2397` — creates
  or returns the synthetic struct def for a tuple shape.  Already
  used by P189b for vector<tuple> element storage.
- **`OpDatabase`** at fill.rs — allocates a Store record for a
  struct type.  Same primitive struct literals use.
- **`OpSetRef` / `OpSetInt`** at fill.rs — writes a field at byte
  offset within a Store record.
- **`get_val`** at `src/parser/mod.rs:1856` — reads an element from
  a `Reference(__tuple<…>)` at byte offset.
- **`ref_return`** at `src/parser/control.rs:2523` — promotes local
  Reference vars to hidden function arguments (ownership transfer).
- **`block_result`** at `src/parser/control.rs:384` — already calls
  ref_return for `Type::Reference` returns; will fire for our
  rewritten return type with no changes.

## What changes for T1.8a's `(text, text)` case

The unified gate INCLUDES Text in the lifetime-bearing predicate,
so `(text, text)` returns now also route through the synthetic
struct.  T1.8a's existing path stops being exercised for the
return case.

This is the intended unification — text becomes a regular 4-byte
field offset in the synthetic struct (uses standard text-in-store
encoding loft uses for every other text field), and lifetime
tracking goes through the existing `text_return` deps machinery.
T1.8a's special-case code in
`src/generation/{mod.rs:366-373, emit.rs:206-210, dispatch.rs}`
becomes dead.

**Verification gate**: `cargo test --release --test tuple_matrix
-- --ignored e2_d2_return_text_text` MUST pass after the rewrite.
If the test reads `("alpha", "beta")` correctly via the new path,
T1.8a's special-case code can be retired in a follow-up cleanup
(or kept as defensive fallback if the rewrite-gate detection has
edge cases).  Plan-14's broader matrix (`tuple_matrix` ignored
tests) provides the regression net.

## Why this won't break ARC.md A7 par-tuple paths

A7's par-tuple-return canaries are blocked at the par-side parser
gate (`Parallel worker return type 'tuple(...)' is not supported`,
`src/parser/collections.rs:1543`), not at the body-tail wrap step.
This phase doesn't touch the par gate.  After landing, A7.3's
canary `par_tuple_return_struct_text` becomes potentially
unblock-able since the underlying tuple-with-struct return now
works — verify and un-ignore in Step 6 if so.

## Verification

```bash
# Native (binary default) — was failing
./target/release/loft /tmp/p234_v6.loft           # → "OK r.1 == 5"

# Interp — already passing
./target/release/loft --interpret /tmp/p234_v6.loft   # → "OK r.1 == 5"

# Unit tests
cargo test --release --test issues p234_runtime    # all pass
cargo test --release --test tuple_matrix -- --ignored e2_d2_return_text_text   # T1.8a's case still passes
cargo test --release                                # no regressions

# CI gate
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo build --release --no-default-features

# Once fixed, attempt to un-ignore the par-side canary
cargo test --release --test threading_chars par_tuple_return_struct_text
```

## Risks

1. **Step 2's rewrite point** — identifying the right place to
   inject the tail-tuple → struct-build transform.  parse_code
   processes the body tail; need to inject after type checking
   but before scope analysis.  If injected too early, type
   inference breaks; too late and scope-analysis already
   mis-handled.  Mitigation: incremental — land Step 1 alone
   first and observe what fires; the Step-1-only state may give
   useful signal about where Step 2 needs to land.
2. **Inner Reference ownership** — when `(p, 5)` becomes
   `__tuple_ret._0 = p; return __tuple_ret`, the inner Set on
   `_0` does a shallow Ref copy.  `OpFreeRef(p)` then frees the
   storage that `_0` points to.  ref_return should handle this
   by promoting `p` to a hidden param of `make`, but verify the
   chain through the synthetic-struct's deps tracking works.
   This is the same risk plain `fn() -> Point { p = Point{}; p }`
   has and solves today — should transfer cleanly.

## Out of scope

- ARC.md A7.1 tuple wide-return runtime (separate plan-06 step)
- Native-codegen tuple-of-text optimisation (T1.8a already shipped)
- Splitting `ls` deps by tuple element (the conservative
  over-keep ref_return does today is fine)
- Recursive tuples (`((Point, T), U)`) — verify they work after
  Step 1's recursive `data.tuple_def` registration; file follow-up
  if a depth-limit issue surfaces

## Follow-up after this phase closes

- Update `doc/claude/PROBLEMS.md` P234 row: lexer + runtime closed,
  remove the workaround note
- Update `doc/claude/plans/06-typed-par/ARC.md` A7.3 status to
  reflect runtime closure
- Update Plan-14 `00-matrix.md` if a new "function return"
  destination column gets added; ref this phase
- See [Phase 08 — LOCAL tuple-with-lifetime-concern variables](08-p234-runtime-locals.md)
  for the natural follow-up: extending the same routing pattern
  to LOCAL var declarations + destructure temps + match subjects.
  Independently executable; can be prioritised, deferred, or
  skipped at will (it's a refactor for uniformity, not a bug fix).
