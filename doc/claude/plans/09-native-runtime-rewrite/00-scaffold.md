# Phase 00 — Scaffold

**Status:** OPEN

**Closes:** — (infrastructure only)

## Goal

Add the per-Op emitter dispatch on top of today's `#rust` template
substitution.  After this phase:

- Every Op-emission call site routes through `emit_op(ctx, name,
  args)`.
- The default emitter performs today's `#rust` template
  substitution unchanged → no behaviour change.
- Any Op can opt into a custom emitter by adding a file under
  `src/generation/ops/<op>.rs` and registering it.

## Files added / changed

| File | Action | Purpose |
|---|---|---|
| `src/generation/ops/mod.rs` | new | `OpEmitter` trait, `EmitCtx` helper, registry, default emitter delegating to `#rust` substitution. |
| `src/generation/ops/default.rs` | new | `DefaultTemplateEmitter` — wraps existing template substitution code. |
| `src/generation/calls.rs` | edit | `output_call_template` thins to `emit_op(ctx, def.name, args)`.  Substitution logic moves into `DefaultTemplateEmitter`. |
| `src/generation/dispatch.rs` | edit | Direct Op emissions route through `emit_op` for consistency. |
| `src/generation/emit.rs` | edit | Fn-ref dispatch arms route through `emit_op`. |
| `tests/codegen_emitter.rs` | new | Validation tests (see below). |

## Detailed steps with validation

### Step 0.1 — Capture baseline emission for diff

**Action**: before any code change, capture the generated Rust for
a representative set of doc tests.  This becomes the "byte-identical"
oracle for phase 00.

```bash
mkdir -p /tmp/p09-baseline
for t in tests/docs/03-integer.loft tests/docs/04-boolean.loft \
         tests/docs/07-vector.loft tests/docs/08-struct.loft \
         tests/docs/13-file.loft tests/docs/19-threading.loft \
         tests/docs/25-generics.loft; do
    name=$(basename "$t" .loft)
    cargo run --bin loft --release --quiet -- \
        --native-emit /tmp/p09-baseline/$name.rs "$t"
done
```

**Validation**: each `/tmp/p09-baseline/*.rs` exists, non-empty, and
compiles when fed to `rustc --crate-type bin --edition 2024`.

### Step 0.2 — Introduce `OpEmitter` trait + empty registry

**Action**: create `src/generation/ops/mod.rs` with the trait,
`EmitCtx`, `emit_op` dispatcher, and an empty registry.

```rust
// src/generation/ops/mod.rs
pub mod default;

use std::io::{self, Write};
use crate::data::{Definition, Value};

pub struct EmitCtx<'a, W: Write + ?Sized> {
    pub w: &'a mut W,
    pub def_fn: &'a Definition,
    pub data: &'a crate::data::Data,
}

pub trait OpEmitter: Send + Sync {
    fn emit(&self, ctx: &mut EmitCtx<'_, dyn Write>, args: &[Value]) -> io::Result<()>;
}

pub fn emit_op<W: Write>(
    ctx: &mut EmitCtx<'_, W>,
    name: &str,
    args: &[Value],
) -> io::Result<()> {
    if let Some(emitter) = registry().get(name) {
        emitter.emit(ctx, args)
    } else {
        default::DefaultTemplateEmitter.emit(ctx, args)
    }
}

fn registry() -> &'static std::collections::HashMap<&'static str, Box<dyn OpEmitter>> {
    static R: std::sync::OnceLock<
        std::collections::HashMap<&'static str, Box<dyn OpEmitter>>
    > = std::sync::OnceLock::new();
    R.get_or_init(|| std::collections::HashMap::new())
}
```

**Validation**:
```bash
cargo build --release
# Compiles cleanly; no use of OpEmitter elsewhere yet.
```

### Step 0.3 — Default emitter wraps existing substitution

**Action**: pull the per-template substitution logic out of
`output_call_template` into a free function
`substitute_template(ctx, args)`, call it from
`DefaultTemplateEmitter`.  No logic change.

**Validation**:
```bash
cargo build --release
# Re-emit the baselines and diff.  MUST be byte-identical.
for t in tests/docs/03-integer.loft tests/docs/04-boolean.loft \
         tests/docs/07-vector.loft tests/docs/08-struct.loft \
         tests/docs/13-file.loft tests/docs/19-threading.loft \
         tests/docs/25-generics.loft; do
    name=$(basename "$t" .loft)
    cargo run --bin loft --release --quiet -- \
        --native-emit /tmp/p09-step03/$name.rs "$t"
    diff -q /tmp/p09-baseline/$name.rs /tmp/p09-step03/$name.rs
done
# Expected: no output (files identical).
```

If diff reports differences, the substitution extraction is not
faithful — fix before proceeding.

### Step 0.4 — Hoist `output_call_template` through `emit_op`

**Action**: replace the body of `output_call_template` with a call
to `emit_op(ctx, def_fn.name, args)`.  All template-keyed calls now
flow through dispatch + default emitter.

**Validation**:
```bash
# Same baseline diff — still byte-identical.
for t in /tmp/p09-baseline/*.rs; do
    name=$(basename "$t" .rs)
    cargo run --bin loft --release --quiet -- \
        --native-emit /tmp/p09-step04/$name.rs tests/docs/$name.loft
    diff -q "$t" /tmp/p09-step04/$name.rs
done

cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test threading 2>&1 | tail -3         # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3   # 35/35
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

### Step 0.5 — Hoist `output_call_user_fn` through `emit_op`

**Action**: user-fn calls also route through dispatch.  Default
emitter delegates user-fn cases to a `UserFnEmitter` (still wrapping
existing logic) so the route is uniform.

**Validation**: baseline diff stays byte-identical; full suite
green.

### Step 0.6 — Hoist `dispatch.rs` direct Op emissions

**Action**: each direct `OpDatabase(cell, …)` / `OpFreeRef(cell, …)`
/ etc. emission in `dispatch.rs` becomes a call to `emit_op` with a
trivial pass-through emitter registered for that Op name.

**Validation**: baseline diff stays byte-identical; full suite
green.

### Step 0.7 — Hoist `emit.rs` fn-ref dispatch arms

**Action**: the `match var_X.0 { 1 => fn1(cell, …) }` arm bodies in
`emit.rs:401` route through `emit_op` so a future custom emitter
for the dispatched fn can intercept.

**Validation**: baseline diff stays byte-identical.

### Step 0.8 — Validation tests in `tests/codegen_emitter.rs`

**Action**: add a tests file that programmatically validates the
emitter dispatch:

```rust
// tests/codegen_emitter.rs

/// Confirm every recognized Op name routes through emit_op (not the
/// legacy substitution path).  Today the registry is empty, so all
/// Ops fall through to DefaultTemplateEmitter — but the dispatch
/// MUST run.  Verified by instrumenting EmitCtx.
#[test]
fn emit_op_is_reached_for_every_op_in_registry() {
    // Set a thread-local counter in EmitCtx.  Compile a doc test.
    // Assert counter > 0 (proving emit_op ran at least once).
}

/// Byte-identical guard: compile a fixed corpus of doc tests; the
/// generated source MUST match a checked-in golden.  If a phase 00
/// step changes emission shape, the diff appears here, not in
/// distant test failures.
#[test]
fn baseline_emission_unchanged() {
    let corpus = [
        "tests/docs/03-integer.loft",
        "tests/docs/04-boolean.loft",
        "tests/docs/07-vector.loft",
        "tests/docs/08-struct.loft",
        "tests/docs/13-file.loft",
    ];
    for t in &corpus {
        let actual = compile_to_rust(t);
        let golden = format!("tests/golden/{}.rs",
            std::path::Path::new(t).file_stem().unwrap().to_str().unwrap());
        assert_eq!(actual, std::fs::read_to_string(&golden).unwrap(),
            "{} emission diverged from golden — phase 00 broke byte-identical guarantee",
            t);
    }
}

fn compile_to_rust(test: &str) -> String {
    let out = format!("/tmp/p09-test-{}.rs",
        std::path::Path::new(test).file_stem().unwrap().to_str().unwrap());
    std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--quiet", "--",
               "--native-emit", &out, test])
        .status()
        .unwrap();
    std::fs::read_to_string(&out).unwrap()
}
```

The golden corpus (`tests/golden/*.rs`) is committed in step 0.1
from the baseline capture.

**Validation**:
```bash
cargo test --release --test codegen_emitter
# Expected: all green.
```

### Step 0.9 — Documentation update

**Action**: add a paragraph to `doc/claude/NATIVE.md` describing
the emitter dispatch and how to add a custom emitter.

**Validation**: review.

## Acceptance for phase 00 overall

```bash
cargo build --release --tests
cargo test --release --test codegen_emitter            # new — all green
cargo test --release --test issues 2>&1 | tail -3      # 540/540
cargo test --release --test threading 2>&1 | tail -3   # 43/43
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
# All counts unchanged from baseline.
```

## Commit shape

7-9 commits (one per step), each with the byte-identical diff
check passing.  Phase ships as a single PR for review economy.

## Risks

| Risk | Mitigation |
|---|---|
| `substitute_template` extraction perturbs whitespace or order | Step 0.3's diff catches it immediately; revert and retry. |
| `dispatch.rs` direct emissions have idiosyncratic arg shapes that don't fit `OpEmitter` | Trivial pass-through emitters keep their existing emission body; the trait only covers the entry point.  Step 0.6 verifies via diff. |
| Adding goldens commits ~50KB of generated Rust to repo | Acceptable — they're checksum-style guards, regenerated only on intentional changes. |

## Problems encountered

_(future sessions: append per problem)_

## Implementation notes

_(future sessions: append per non-obvious decision)_
