# Phase 03 — Parallel-for emitter family

**Status:** OPEN

**Kind:** Simplification — **prerequisite for phase 06 (P202).**
Without this, adding the missing `n_parallel_queue*` runtime fns
duplicates the 95-line special case in `dispatch.rs:837-930` for
each new helper.  Cleaning up the parallel-for emission first means
phase 06's queue fns each register as ~15 lines on top of shared
emitter helpers.

**Depends on:** Phase 00.

## What's tangled today

`src/generation/dispatch.rs:837-930` is a 95-line special case for
emitting `n_parallel_for` / `n_parallel_for_light` calls.  The
emission must:

1. Pull the worker fn def out of `vals[4]` and read its return type.
2. Pick which native helper to call (`n_parallel_for_native`,
   `n_parallel_for_ref_native`, `n_parallel_for_text_native`)
   based on text / heap-typed / scalar return.
3. Synthesise extra-arg `let _ex0 = …; let _ex1 = …;` bindings.
4. Emit a closure with one of FOUR shapes:
   - text → `|cell, elm| { let mut _w = String::new(); worker(cell, elm, …, &mut _w); _w }`
   - heap ref → `|cell, elm| { worker(cell, elm, …) }`
   - float → `|cell, elm| { worker(cell, elm, …).to_bits() as i64 }`
   - scalar → `|cell, elm| { worker(cell, elm, …) as i64 }`
5. Close N braces matching the let-bindings.

## Detailed steps with validation

### Step 3.1 — Capture parallel-for emission corpus

**Action**: identify representative tests covering each closure
shape (text return, heap-ref return, float return, scalar return,
plus zero/one/multiple extra args).  Capture the generated `.rs`
for each as a golden:

```bash
mkdir -p tests/golden/parallel
# Pick tests that cover each shape — example list, refine after
# inspecting the actual test corpus.
TESTS=(
    "tests/scripts/par_scalar.loft"      # scalar return
    "tests/scripts/par_float.loft"       # float return
    "tests/scripts/par_text.loft"        # text return
    "tests/scripts/par_ref.loft"         # heap-ref return
    "tests/scripts/par_with_extras.loft" # 2 extra args
    "tests/scripts/par_no_extras.loft"   # 0 extra args
)
for t in "${TESTS[@]}"; do
    name=$(basename "$t" .loft)
    cargo run --bin loft --release --quiet -- \
        --native-emit "tests/golden/parallel/$name.rs" "$t"
done
```

If any of the listed tests don't exist, write minimal reproducers
in `tests/scripts/` first.

**Validation**: each golden compiles; `cargo test wrap par` passes.

### Step 3.2 — Extract helper functions

**Action**: create `src/generation/ops/parallel.rs` with the four
helpers as free functions.  No registration yet:

```rust
pub(crate) fn pick_helper(ret: &Type) -> &'static str {
    if matches!(ret, Type::Text(_)) { "n_parallel_for_text_native" }
    else if ret.heap_def_nr().is_some() { "n_parallel_for_ref_native" }
    else { "n_parallel_for_native" }
}

#[derive(Copy, Clone)]
pub(crate) enum ClosureShape { Text, HeapRef, Float, Scalar }

pub(crate) fn closure_shape(ret: &Type) -> ClosureShape {
    if matches!(ret, Type::Text(_)) { ClosureShape::Text }
    else if ret.heap_def_nr().is_some() { ClosureShape::HeapRef }
    else if matches!(ret, Type::Float | Type::Single) { ClosureShape::Float }
    else { ClosureShape::Scalar }
}

pub(crate) fn emit_extra_bindings(...) -> io::Result<()> { ... }
pub(crate) fn emit_closure(...) -> io::Result<()> { ... }
```

Add unit tests:
```rust
#[test]
fn pick_helper_returns_correct_native_fn() {
    assert_eq!(pick_helper(&Type::Integer(8)), "n_parallel_for_native");
    assert_eq!(pick_helper(&Type::Float), "n_parallel_for_native");
    assert_eq!(pick_helper(&Type::Text(0)), "n_parallel_for_text_native");
    // etc.
}

#[test]
fn closure_shape_categorises_returns() {
    assert!(matches!(closure_shape(&Type::Integer(8)), ClosureShape::Scalar));
    assert!(matches!(closure_shape(&Type::Float), ClosureShape::Float));
    assert!(matches!(closure_shape(&Type::Text(0)), ClosureShape::Text));
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::pick_helper_returns_correct_native_fn
cargo test --release --test codegen_emitter::closure_shape_categorises_returns
```

Existing tests still pass (helpers aren't called yet).

### Step 3.3 — Wrap in `ParallelForEmitter` and register

**Action**: implement `ParallelForEmitter` using the helpers,
register it for `n_parallel_for` and `n_parallel_for_light`.
Replace `dispatch.rs:837-930` with `return emit_op(ctx, name, args)`.

**Validation**:
```bash
# Re-emit the parallel goldens; MUST be byte-identical.
for golden in tests/golden/parallel/*.rs; do
    name=$(basename "$golden" .rs)
    cargo run --bin loft --release --quiet -- \
        --native-emit /tmp/p09-step33-$name.rs tests/scripts/$name.loft
    diff "$golden" /tmp/p09-step33-$name.rs
done

cargo test --release --test wrap par
cargo test --release --test threading 2>&1 | tail -3       # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3 # 35/35
cargo test --release --test issues 2>&1 | tail -3
```

If any golden differs, the helper extraction missed a detail —
fix and rerun.

### Step 3.4 — Document parallel-emitter extension surface

**Action**: in `parallel.rs` add a doc comment showing how phase 06
will register `ParallelQueueEmitter` etc. using the same helpers.
Include a worked example.

**Validation**: review.

### Step 3.5 — Add structural test that no parallel-for special case remains in `dispatch.rs`

**Action**:
```rust
#[test]
fn no_parallel_special_case_in_dispatch() {
    let src = std::fs::read_to_string("src/generation/dispatch.rs").unwrap();
    assert!(!src.contains("n_parallel_for_text_native"),
        "dispatch.rs still has parallel-for special case — phase 03 incomplete");
    assert!(!src.contains("n_parallel_for_ref_native"),
        "dispatch.rs still has parallel-for special case — phase 03 incomplete");
}
```

**Validation**: test passes.

## Acceptance for phase 03 overall

```bash
cargo test --release --test codegen_emitter::no_parallel_special_case_in_dispatch
cargo test --release --test wrap par
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

Plus all `tests/golden/parallel/*.rs` compile + match emission.

Net diff target: ~95 lines deleted from `dispatch.rs`, ~120 lines
added across `src/generation/ops/parallel.rs`.

## Gate updates per step

| Step | Gate update |
|---|---|
| 3.1 | Captures parallel goldens at `tests/golden/parallel/*.rs`. |
| 3.2 | Helper functions extracted; byte-identical for all parallel test paths. |
| 3.3 | `ParallelForEmitter` registered.  Gate's `custom_count` increments by 1 (or more if also covering text/ref variants).  `dispatch.rs op match arms` count drops by ~6 (the parallel special case retires). |
| 3.4 | Documentation only. |
| 3.5 | New structural test `no_parallel_special_case_in_dispatch`. |

This phase coordinates with phase 09 (parallel runtime
consolidation) — phase 03 cleans up the EMITTER side, phase 09
cleans up the runtime fn side.  Phase 01 step 1.7 (cleanup) is
unblocked once both land (the last `LegacyStores` entries are the
`n_parallel_for_*` family).

## Problems encountered

_(append per problem — closure capture interaction with extra-arg
bindings has been historically delicate; verify each closure shape
produces the same generated code as today before accepting)_

## Implementation notes

_(append per non-obvious decision)_
