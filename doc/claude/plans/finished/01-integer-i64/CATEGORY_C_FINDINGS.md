# Category C — native codegen i32 → i64 widening

Status: **attempted 2026-04-20, reverted**.  See
`PHASE_2C_PROGRESS.md § Category C` for where this sits in the
overall Phase 2c picture.

## What failed before the attempt

`native_binary_script`, `native_dir`, `native_scripts`,
`native_tuple_return_script`, `native_tuple_script`,
`moros_glb_cli_end_to_end`, `p171_native_copy_record_high_bit`
all fail with rustc errors on the emitted native code.  Typical
errors:

```
fn t_6vector_len(stores: &mut Stores, mut var_both: DbRef) -> i32 {
    return i64::from(vector::length_vector(...))
//         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected i32, found i64

let mut var_c__next: i32 = 0_i32;
...
var_c__next = ops::op_add_int((var_c__next), ...);
//            ^^^^^^^^^^^^^^^^ expected i64, found i32
```

Root: native codegen still declares loft-`integer` variables as
Rust `i32`, but the runtime `ops::op_*_int` family is now i64
(post-2c).  Return types, vector elements, null sentinels —
all still emit i32 markers.

## What was tried

### Layer A — widen declared types

Applied and validated:

| File | Line | Change |
|------|------|--------|
| `src/generation/mod.rs` | 294 | `Type::Integer => "i64"` (variables) |
| `src/generation/mod.rs` | 457 | `Type::Integer => " -> i64"` (return) |
| `src/generation/mod.rs` | 1204 | `Type::Integer => "i64"` (vector elements) |
| `src/generation/mod.rs` | 1184 | `needs_ret_cast` tail `as i64` |
| `src/generation/emit.rs` | 388 | `Type::Integer => "i64::MIN"` null |

Character stays i32 (4 bytes on stack).  Narrow subtypes
(u8/u16/i8/i16) keep their precise narrow type in Context::Result
and widen to i32 in Context::Argument.

### Layer B — insert `as i64` widening at use sites

Applied:

| File | Location | Trigger |
|------|----------|---------|
| `src/generation/calls.rs` | 117 | narrow-return widen after call |
| `src/generation/calls.rs` | 282, 284 | template body narrow widen |
| `src/generation/dispatch.rs` | 279 | narrow-var assign widen |
| `src/generation/pre_eval.rs` | 450 | bind_code narrow widen |
| `src/generation/pre_eval.rs` | 633 | post-call narrow widen |
| `src/generation/dispatch.rs` Set | after RHS emit | Value::Int RHS → Integer var |
| `src/generation/calls.rs::output_call_template` | arg substitution | Value::Int → i64 param |
| `src/generation/calls.rs::output_call_user_fn` | after Value::Int emit | Value::Int → i64 param |
| `src/generation/pre_eval.rs` | ~618 | Value::Int → i64 param in pre-eval call emit |

All `_i64` widening sites check
`rust_type(Context::Result) == "i64"` to avoid widening narrow
u8/u16/i8/i16 params that also flow through `Type::Integer`.

### Layer C — widen runtime signatures

Applied partial set:

| Function | Before | After | Reason |
|----------|--------|-------|--------|
| `OpGetTextSub(from, till)` | `i32, i32` | `i64, i64` | character offsets are loft integer |
| `OpLengthCharacter -> ?` | `-> i32` | `-> i64` | UTF-8 byte count is loft integer |
| `OpReadFile(bytes, db_tp)` | `i32, i32` | `i64, i32` | bytes is loft integer; db_tp stays tp-number |
| `n_assert(... line)` | `line: i32` | `line: i64` | line number is loft integer |

## The Value::Int ambiguity

The **blocking** architectural issue.  `Value::Int(n)` is
emitted in two semantically distinct contexts:

1. **User loft integer literal** — post-2c `x = 42` where
   `x: integer` is Rust `i64`.  Needs `42_i64`.
2. **Compile-time tp-number / offset / line constant** — e.g.
   `OpDatabase(db, 27)` where the runtime signature is
   `db_tp: i32`.  Needs `27_i32`.

`src/generation/emit.rs:36` emits all `Value::Int` as `{v}_i32`.

- Changing to `{v}_i64` fixes (1) but breaks (2) — runtime
  signatures for tp-numbers are i32.
- Keeping as `_i32` requires every (1) site to emit `as i64` at
  the use site (what Layer B does).
- Layer B covered the main Set + call paths but missed the
  pre_eval parallel call path until a second pass.

## Result after all three layers

- rustc errors on `native_binary_script`: **146 → 0**.
- Compile clean.
- Runtime panic surfaces at `src/database/types.rs:358:46`:
  `index out of bounds: len is 60 but index is 60` (called from
  `Types::vector(content)` — the types table is asked to create
  `vector<60>` when only 60 types exist).
- Full suite: 23 → 24 failures (1 new regression:
  `n1_native_pipeline_trivial_program`).

## Why the partial fix is net-negative

- **0 tests fixed** — all 5 native_* tests still fail
  (rustc-broken → runtime-panicking).
- **1 test regressed** — n1_native_pipeline_trivial_program.
- Runtime panic is not part of Category C scope.

Therefore reverted; the branch is back at commit `8edb15c`.

## Proposed fix strategy

Three plausible paths to actually ship Category C.  Pick one:

### Strategy 1 — Widen runtime signatures completely (recommended)

Update `src/codegen_runtime.rs` so every function that takes a
loft-integer parameter takes `i64`.  Keep i32 only for
tp-numbers, flag enums, and field offsets that are compile-time
constants.

Functions to widen (scope estimate, to verify):

- `OpSizeofRef -> i32` → `-> i64` (sizeof result is loft integer).
- `OpNewRecord(parent_tp, fld)` — both stay i32 (tp-number,
  field-index).  No change.
- `OpFinishRecord(parent_tp, fld)` — same.
- `OpCopyRecord(tp)` — stays i32.
- `OpSortVector(db_tp)` — stays i32.
- `OpStep(on, arg)` — stays i32 (both are internal flags).
- `OpHashRemove(tp)` — stays i32.
- `OpAppendCopy(count, tp)` — `count` is loft integer → i64;
  `tp` stays i32.
- `file_handle_write -> i32`, `file_handle_read -> i32` — stay
  i32 (internal errno-like).
- `file_to_bytes(db_tp)` — stays i32 (tp-number).

Then set `Value::Int` emission to `_i64` in
`src/generation/emit.rs:36` (user loft literal context).  The
few remaining tp-number emission sites already use explicit
`{v}_i32` format literals (e.g. `dispatch.rs:58`, `dispatch.rs:
592`).

Effort: **3-5 hours**.  Risk: medium — touches live runtime
signatures that the bytecode path also calls.  Each widened
signature needs its interpreter-side call site verified.

### Strategy 2 — Context-aware Value::Int emission

Keep `Value::Int` as-is but make the emitter pass the expected
target type down through the emission stack.  When emitting a
Value::Int for a known-i64 slot (variable init, call arg to i64
param), emit `_i64`.  Otherwise emit `_i32`.

Requires threading a `Type` context through
`output_code_inner` / `generate_expr_buf` — ~40 sites.

Effort: **4-6 hours**.  Risk: high — broad API change across
the generation module.  Easier to introduce bugs than Strategy 1.

### Strategy 3 — Post-process the generated file

After emit, scan the generated Rust for `_i32` literals passed
to `i64`-declared positions (by parsing the types of functions
it calls) and rewrite those to `_i64`.

Effort: hard to estimate, very fragile.  **Not recommended.**

## Recommendation

**Strategy 1**.  The runtime-signature widening is what
post-2c semantics actually want (loft integer = i64 both at
rest AND in all contracts).  The residual i32 constants are
genuinely internal (tp numbers, field offsets, flag enums) and
naturally stay narrow.  Effort matches the 3-5 hour estimate in
PHASE_2C_PROGRESS.md for a Category C retry.

## Prerequisite before retry

The `types.rs:358` runtime panic must be diagnosed first (or at
least reproduced outside of the native path).  If it's a
pre-existing D-family bug masked by the rustc failure, Category
C can legitimately claim "compile-clean" as success and let
Category D fix the runtime panic separately.  If it's a new
codegen artefact introduced by the widening, it IS a Category C
issue.  A 30-minute investigation (read types.rs:358, trace the
caller in the generated code, compare to interpreter-path
behaviour) would resolve which category owns it.

## Where the work is right now

Committed: `8edb15c` (cdylib fixture widen).
Working tree at commit `8edb15c`: clean.  All Layer A/B/C
partial edits were reverted after the net-negative suite
result.  The detailed edit list above is preserved here so a
retry session can reproduce the attempt and continue from
Strategy 1.
