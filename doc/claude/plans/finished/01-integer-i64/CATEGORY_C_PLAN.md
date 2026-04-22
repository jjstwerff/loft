# Category C — native codegen retry plan (Strategy 1)

## Context

Six tests still fail after D.1 closed:

- `native_binary_script`       (tests/native.rs)
- `native_dir`                 (tests/native.rs)
- `native_scripts`             (tests/native.rs)
- `native_tuple_script`        (tests/native.rs)
- `native_tuple_return_script` (tests/native.rs)
- `p171_native_copy_record_high_bit_does_not_panic` (tests/exit_codes.rs)

All fail at the same layer: the `--native` code generator emits
loft-integer-typed variables as Rust `i32`, but the runtime helpers
in `src/ops.rs` are i64 post-Phase-2c.  Typical rustc error:

```
error[E0308]: mismatched types
  --> /tmp/loft_native_03_integer.rs:257:26
   |
257 |   return ops::op_abs_int((var_both))
   |          --------------- ^^^^^^^^^^ expected `i64`, found `i32`
   |
484 | pub fn op_abs_int(val: i64) -> i64 {
```

This is the same root cause documented in
`CATEGORY_C_FINDINGS.md` (branch `int_migrate`, commit `8edb15c`,
attempt reverted 2026-04-20).  The finding doc recommends
**Strategy 1 — widen runtime signatures completely**.  That
recommendation still stands; this plan turns it into a concrete
sequence with a concrete failure gate.

## Why this matters

- Phase 2c's whole goal was `integer = i64` end-to-end.  The
  interpreter path finished that work; the native path did not.
  Until native ships, `--native` (which is the default release
  path, CI-gated) silently falls back to the interpreter for any
  file touching integer arithmetic — half the corpus.
- Six tests is the biggest single remaining cluster on the branch.
- The prior attempt's net-negative result came from a runtime
  panic (`types.rs:358`) that surfaced **only after** rustc
  errors cleared.  That panic is almost certainly a separate bug
  the rustc failures were masking — Category D-shape, not C.  A
  30-minute pre-investigation resolves ownership before the main
  work starts.

## Approach

Strategy 1 from `CATEGORY_C_FINDINGS.md` — widen every runtime
signature that represents a loft integer to i64, keep i32 only
for genuinely internal constants (tp-numbers, field offsets,
flag enums), then switch `Value::Int` emission to `_i64` for
user literals.

The prior attempt applied this in three layers (A declared
types, B use-site widening, C runtime signature widening).  The
learned lesson from `CATEGORY_C_FINDINGS.md § The Value::Int
ambiguity` is that Layer B is the fragile one — threading `as
i64` widening through every Value::Int site is error-prone, and
Layer C was left incomplete.  This retry **completes Layer C
first**, which removes the need for most of Layer B.

## Staged plan

### Step 0 — diagnose the masked runtime panic (30 min, prerequisite)

Before any codegen edits, reproduce the `types.rs:358` panic
that blocked the last attempt.  This is the panic that surfaced
when rustc errors cleared in the prior try:

```
thread '...' panicked at src/database/types.rs:358:46:
index out of bounds: len is 60 but index is 60
```

Path: `Types::vector(content)` asked to create `vector<60>` when
only 60 types exist — i.e. a tp-number that was narrowed from
i64 to i32 (or widened) lost its sentinel, so a `content == 60`
lookup happened where content should have been `u16::MAX` or
similar.

Outcome: classify the panic as (a) Category C artefact → this
plan owns it, or (b) pre-existing Category D bug that the rustc
failures were masking → file as separate D issue, proceed with
C.

Deliverable: 2-3 sentence note in `PHASE_2C_PROGRESS.md §
Category C` + a category label.

### Step 1 — widen every loft-integer runtime signature (45 min)

File: `src/codegen_runtime.rs` (+ call sites in `src/ops.rs`).

Widen to `i64` every parameter / return that is a **loft
integer** (not a tp-number, not a field offset, not a flag enum):

| Function | Current | Target | Reason |
|----------|---------|--------|--------|
| `OpSizeofRef` | `-> i32` | `-> i64` | sizeof result is loft integer |
| `OpAppendCopy(count, tp)` | `count: i32` | `count: i64` | count is loft integer |
| `cr_rand_int(lo, hi) -> i32` | all i32 | all i64 | user-visible integer |
| `OpGetTextSub(from, till)` | `i32, i32` | `i64, i64` | character offsets |
| `OpLengthCharacter` | `-> i32` | `-> i64` | UTF-8 byte count |
| `OpReadFile(bytes, db_tp)` | `i32, i32` | `i64, i32` | bytes is loft integer |
| `n_assert(... line)` | `line: i32` | `line: i64` | line is loft integer |
| `file_handle_write/read` | `-> i32` | unchanged | internal errno-like |
| `file_to_bytes(db_tp)` | `i32` | unchanged | tp-number |

(Full inventory above from `CATEGORY_C_FINDINGS.md § Layer C`;
check against current `codegen_runtime.rs` during edit — signature
drift since 2026-04-20 is possible.)

Gate for this step: `cargo check --release` still compiles.

### Step 2 — switch Value::Int emission to i64 (10 min)

File: `src/generation/emit.rs:36` (or wherever `{v}_i32` is
formatted for `Value::Int`).

Change `_i32` → `_i64` for Value::Int.  Verify the specific sites
that emit tp-numbers / field indices (`dispatch.rs:58`,
`dispatch.rs:592`, and any similar) already use explicit
`"{v}_i32"` format literals — they should not be affected.

Gate: `cargo check --release` still compiles.  If any sites
break, they're the "tp-number via Value::Int" ambiguity — fix by
switching those specific sites to an explicit i32 literal path
(Content::TpNumber?  new Value variant?  local cast?).

### Step 3 — widen declared variable + return types (20 min)

File: `src/generation/mod.rs` (+ `emit.rs` for null sentinel).

From `CATEGORY_C_FINDINGS.md § Layer A`:

| Location | Change |
|----------|--------|
| `src/generation/mod.rs:294` | `Type::Integer => "i64"` for variables |
| `src/generation/mod.rs:457` | `Type::Integer => " -> i64"` for returns |
| `src/generation/mod.rs:1204` | `Type::Integer => "i64"` for vector elements |
| `src/generation/mod.rs:1184` | `needs_ret_cast` tail `as i64` |
| `src/generation/emit.rs:388` | `Type::Integer => "i64::MIN"` null |

Character stays i32.  Narrow subtypes (u8/u16/i8/i16) keep their
precise narrow type in `Context::Result` and widen to i32 in
`Context::Argument` (unchanged).

Gate: `cargo check --release` still compiles; `cargo run --bin
loft -- --native-emit /tmp/foo.rs tests/scripts/03-integer.loft`
+ `rustc --edition=2024 --crate-type=lib /tmp/foo.rs` produces
zero type-mismatch errors on integer sites.

### Step 4 — close any remaining mismatch at call sites (as needed)

With the runtime widened (Step 1) and `Value::Int` emitting i64
(Step 2), most of Layer B's explicit `as i64` widening becomes
unnecessary.  Whatever mismatches rustc still reports on
`native_binary_script` are real — either an internal i32 value
flowing into an i64 slot (add explicit cast) or a runtime
signature that stayed i32 but shouldn't have (back to Step 1).

Gate: `cargo run --bin loft -- --native-emit ...` + rustc passes
cleanly for 03-integer, 20-binary, 50-tuples.

### Step 5 — run the six targets, iterate (30-60 min)

```bash
cargo test --release --test native native_binary_script
cargo test --release --test native native_dir
cargo test --release --test native native_scripts
cargo test --release --test native native_tuple_script
cargo test --release --test native native_tuple_return_script
cargo test --release --test exit_codes p171_native_copy_record_high_bit
```

Each failing test either shows (a) a rustc error — jump back to
the appropriate earlier step, or (b) a runtime panic — this is
the Step 0 territory; if Step 0 classified it as a pre-existing
Category D bug, file it and move on.

### Step 6 — full-suite regression sweep

`./scripts/find_problems.sh --bg` → verify no NEW failures
outside the six targets.  Prior attempt regressed
`n1_native_pipeline_trivial_program` — that's the specific
canary.

## Verification

Success criteria:

1. All six named tests pass (`native_*` ×5 plus
   `p171_native_copy_record_high_bit_does_not_panic`).
2. No new test failures introduced — in particular
   `n1_native_pipeline_trivial_program` still passes.
3. `cargo clippy --release -- -D warnings` is clean.
4. The generated native code for a representative program
   (e.g. `tests/scripts/03-integer.loft`) has `i64` for every
   loft-integer variable / return type and `_i64` for every
   user literal.

## Critical files

- `src/generation/mod.rs` — variable / return type emission.
- `src/generation/emit.rs` — Value::Int literal suffix, null
  sentinel.
- `src/generation/calls.rs` — call-arg substitution (may or may
  not need post-Layer-C edits).
- `src/generation/pre_eval.rs` — parallel-call pre-eval path.
- `src/generation/dispatch.rs` — Set (variable assign) emission.
- `src/codegen_runtime.rs` — runtime signatures called from
  emitted code.
- `src/ops.rs` — `op_*_int` / `op_*_long` helpers (already i64;
  check for stragglers).
- `tests/native.rs`, `tests/exit_codes.rs` — the six gating
  tests.

## Risk + estimate

Effort: **3-5 hours** (matches the `CATEGORY_C_FINDINGS.md`
estimate).  Risk: medium — touches runtime signatures the
bytecode path also calls, so every widened signature needs its
interpreter-side caller verified.  Mitigation: Step 1 finishes
before any codegen edits, and each step has a `cargo check`
gate — the build breaks immediately on mismatch, not silently.

Fallback: if Step 1 balloons beyond the 45-min estimate
(signature count >20 with non-obvious ownership), pause and
revisit Strategy 2 (context-aware Value::Int emission).  Don't
let the session drag past ~5 hours without a working commit.

## Out of scope

- Category G (HTML/WASM `p137_html_*`, `q9_html_*`) — separate
  workstream, different failure mode (`loft_start` trap).
- `moros_glb_cli_end_to_end` — GLB version byte; unrelated to
  integer widening.  Own commit when addressed.

## Where to resume from

Branch `int_migrate`, tip `ae121cb` (D.1 wrap-test fix).
Working tree clean.  Start with Step 0 on a fresh
`cargo test --release --test native native_binary_script`
reproduction.
