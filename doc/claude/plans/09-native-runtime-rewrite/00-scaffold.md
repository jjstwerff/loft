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

### Step 0.7b — Add let-bind-on-repeat to `DefaultTemplateEmitter`

**Action**: extend `substitute_template` so when a placeholder
`@vN` appears **two or more times** in the template, the
substitution emits a `let _vN = …;` once and substitutes the
binding name in every position, all wrapped in a `{ … }` block:

```rust
fn substitute_template(ctx: &mut EmitCtx<'_, dyn Write>, args: &[Value]) -> io::Result<String> {
    let template = ctx.def_fn.rust.as_str();
    let mut out = template.to_string();
    let mut lets: Vec<String> = Vec::new();

    for (i, arg) in args.iter().enumerate() {
        // Match @<paramname> per the existing matcher; here pseudocode.
        let placeholder = format!("@{}", ctx.def_fn.attributes[i].name);
        let count = out.matches(&placeholder).count();
        if count == 0 {
            continue;
        }
        let arg_expr = emit_arg(ctx, arg)?;          // existing per-arg emission
        if count >= 2 {
            // Hoist into a let so the arg expression evaluates once.
            let local = format!("_{}", placeholder.trim_start_matches('@'));
            lets.push(format!("let {local} = {arg_expr};"));
            out = out.replace(&placeholder, &local);
        } else {
            out = out.replace(&placeholder, &arg_expr);
        }
    }

    if lets.is_empty() {
        Ok(out)
    } else {
        Ok(format!("{{ {} {} }}", lets.join(" "), out))
    }
}
```

**Why**: today's templates with repeated placeholders cause
double-evaluation of side-effecting arguments.  P203's actual
root cause is exactly this: `default/01_code.loft:705`
substitutes `@v1` twice in the `OpConvIntFromEnum` template, and
when `@v1` is `n_delete(...)`, the file gets deleted twice
(second call returns `NotFound`, panic).  This refinement
eliminates the bug class structurally; future templates with
repeats are auto-protected.

**Compatibility**:
- Templates with each placeholder appearing ≤1 times: emission
  unchanged → byte-identical.
- Templates with repeated placeholders (currently 5 in
  `default/01_code.loft`: lines 690, 705, 707, 751, 753):
  emission shape changes to wrap in `{ let … ; … }`.  Functional
  behaviour either improves (side-effecting calls evaluate once)
  or is unchanged (pure-arg cases produce the same value).

**Validation**:

```bash
# Single-sub corpus stays byte-identical (unchanged from step 0.7):
for t in tests/docs/03-integer.loft tests/docs/04-boolean.loft; do
    # …diff vs golden — still must match.
done

# Repeated-sub templates change shape; verify the new shape:
cargo run --bin loft --release --quiet -- \
    --native-emit /tmp/p09-step07b.rs tests/scripts/repro_p203.loft
grep -A 1 "n_delete" /tmp/p09-step07b.rs
# Expected: ONE n_delete call wrapped in `{ let _v1 = n_delete(…); …; }`,
# not two.

# Functional verification — P203 reproducer now passes (side effect):
cargo run --bin loft --release -- tests/scripts/repro_p203.loft
echo "Exit: $?"     # Expected: 0 (P203 closes as a side effect)

# Full suite green:
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

**Note**: P203's closure here is structural — phase 09's
DefaultTemplateEmitter eliminates the bug class.  P203 is also
fixable as a direct edit to `default/01_code.loft`'s 5 templates
(let-bind each by hand).  If the direct edit ships before phase 00,
this step has nothing to demonstrate (P203 already closed); the
let-bind-on-repeat protection still ships as a structural
guarantee for future templates.  See PROBLEMS.md P203 entry.

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

/// Step 0.7b regression: when a template's placeholder appears
/// multiple times, the substitution emits a `let` binding so the
/// arg expression evaluates once.  P203's bug class.
#[test]
fn template_repeated_placeholder_binds_once() {
    // OpConvIntFromEnum's template substitutes @v1 twice.  Generate
    // code that calls it with a side-effecting expression and verify
    // the call appears ONCE in the emitted source.
    let test = "tests/scripts/p09_repeat_sub.loft";
    std::fs::write(test, r#"
fn p09_counter() -> integer { 0 }
fn main() {
    // Compare a side-effecting call to an enum: today this would
    // generate two calls; with let-bind-on-repeat, it generates one.
    r = file("/tmp/p09_dummy.txt"); assert(p09_counter() == 0, "trivial")
}
    "#).unwrap();
    let src = compile_to_rust(test);
    let _ = std::fs::remove_file(test);

    // The let-bind shape: `{ let _v1 = …; if _v1 == … { … } else { …(_v1)… } }`
    // For any template with repeats, expect a `let _v1 =` (or similar) wrapper.
    // Heuristic: any line containing both `let _v` and `; if`/`; match` is
    // the let-bind-on-repeat shape.
    let has_let_bind = src.lines().any(|line| {
        line.contains("let _v") && (line.contains("; if") || line.contains("; match"))
    });
    // OR the wrapping `{ let _v1 = ...; ... }` form spans multiple lines.
    // Refine the heuristic during implementation.
    assert!(has_let_bind || src.contains("{ let _v"),
        "expected let-bind-on-repeat shape in generated source for repeated placeholders");
}

/// Step 0.7b structural: P203 closes (or is already closed) once
/// the let-bind-on-repeat is in place.
#[test]
fn p203_reproducer_passes_under_native() {
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--",
               "tests/scripts/repro_p203.loft"])
        .status().unwrap();
    assert!(status.success(),
        "P203 reproducer failing — let-bind-on-repeat or template fix not in place");
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
# All counts unchanged from baseline OR improved (P203 reproducer
# transitions from FAIL to PASS once step 0.7b lands).

# Bug-class closure: P203 fixed by step 0.7b (or by direct template
# fix — whichever shipped first):
cargo run --bin loft --release -- tests/scripts/repro_p203.loft
echo "Exit: $?"   # Expected: 0
```

### Byte-identical guarantee — scoped

Phase 00's byte-identical guarantee applies to templates whose
placeholders each appear at most once.  Templates with repeated
placeholders (currently 5 in `default/01_code.loft`: lines 690,
705, 707, 751, 753) change emission shape under step 0.7b — that
change is the bug-class fix, not a regression.  The
`baseline_emission_unchanged` test corpus deliberately avoids
files that exercise repeated-substitution templates so the
byte-identical guarantee remains a meaningful gate for the
non-affected paths.

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
