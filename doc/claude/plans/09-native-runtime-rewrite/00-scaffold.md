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

### Step 0.7b — Let-bind-on-repeat for repeated `@<name>` placeholders

> **Status (2026-05-02): the structural fix already shipped** as a
> direct edit to `src/generation/calls.rs::output_call_template`.
> P203 closed.  This step is documentation that explains **why
> the pattern matters**, **why the existing code resisted a
> cleaner design**, and **what phase 00's extraction has to
> preserve**.  Reading this section is the cheapest way for a
> future contributor to avoid re-introducing the bug class when
> refactoring the substitution code.

#### Why the bug class exists

The original substitution mechanism does string replacement:
`res.replace("@v1", "<arg_expr>")`.  If `@v1` appears two or more
times in the template AND `<arg_expr>` has side effects, the
generated code evaluates the side effect once per occurrence.

Five templates in `default/01_code.loft` hit this:

| Line | Op | `@v1` count | `@v2` count |
|---|---|---|---|
| 690 | char→int (`OpConvIntFromChar`) | 2 | — |
| 705 | enum→int (`OpConvIntFromEnum`) — P203's manifestation | 2 | — |
| 707 | int→enum (`OpCastEnumFromInt`) | 2 | — |
| 751 | ref equality (`OpEqRef`) | 4 | 4 |
| 753 | ref inequality (`OpNeRef`) | 4 | 4 |

P203's specific symptom: `delete(path) == FileResult.Ok` becomes
`if (n_delete(path) as u8) == 255 { i64::MIN } else { i64::from((n_delete(path) as u8)) }`.
First call deletes the file; second sees nothing; comparison
fails; assertion panics.

#### Why the existing substitution code was complex

`src/generation/calls.rs:200-324` is the **per-parameter
substitution matrix**: 125 lines of stacked `if matches!(…) { … continue; }`
arms, each computing a `with` value and calling
`res = res.replace(&name, ...)`.  The arms exist because
substituted text needs different wrapping based on `(parameter
type, value type)`:

- enum-typed param + `Value::Null` → `(255u8)`
- ref-typed param + `Value::Null` → null DbRef sentinel
- char param + `Value::Int` → `char::from_u32(N)`
- char param + char-Var/Call → `ops::to_char(...)`
- text param + text-Call → `(&*(...))`
- int param + char value → `as u32 as i32`
- int param + fn-ref tuple → `(i64::from((..).0))`
- u32-from-field-offset → `(...) as u32`
- narrow int (u8/u16/i8/i16) → suffix patch or cast wrap

Every arm re-queries the IR (variable type, return type, typedef
enum) for the same value.  Adding the let-bind-on-repeat to each
arm would have meant 10+ edit sites with subtle interaction
between them — exactly the kind of change that historically
regresses tests.

This complexity is documented further in [phase 02's
characterisation](02-param-adapter.md) — the simplification
phase 02 retires this matrix entirely.

#### What shipped (the direct fix)

A **pre-pass loop**, run BEFORE the substitution arms touch the
template.  It scans for repeated placeholders and rewrites the
template into a let-binding form:

```rust
// In output_call_template, immediately after the Str::new(...) unwrap:
for a in &def_fn.attributes {
    let placeholder = format!("@{}", a.name);
    if res.matches(&placeholder).count() >= 2 {
        let local = format!("_v_{}", a.name);
        res = res.replace(&placeholder, &local);
        res = format!("{{ let {local} = {placeholder}; {res} }}");
    }
}
```

The arms then run unchanged.  Each arm sees:
- A template with at most ONE `@<name>` per repeated attribute
  (in the let-RHS, prepended by the pre-pass).
- The arms substitute that single occurrence with their wrapped
  value as today.
- All other occurrences are now `_v_<name>` — Rust local names
  that the arms ignore (they don't start with `@`).

Result: the side-effecting `<value>` evaluates once (in the let
binding); subsequent positions read `_v_<name>`.

The pre-pass is ~12 lines, decoupled from the substitution arms,
and required no changes to any of the 10+ if-arms.  This was the
key insight: **don't try to add let-bind logic to each arm; do
it once in a pre-pass that rewrites the template**.

#### Why phase 02 (param adapter) wasn't required first

The user originally hypothesised that the substitution code
needed phase 02's simplification before P203 could land cleanly.
That hypothesis was reasonable — the arms genuinely are tangled.
But the pre-pass approach sidesteps the arms entirely by
rewriting the TEMPLATE before substitution runs.  No arm-level
edits required.

So: phase 02 still has independent value (phase 02's adapter
extraction makes the arms readable for future maintainers), but
it isn't a prerequisite for the let-bind-on-repeat fix.  P203
landed first; phase 02 is still on the simplification roadmap.

#### What phase 00's extraction must preserve

When step 0.3 hoists the substitution into
`DefaultTemplateEmitter`, the pre-pass moves with it.  No
behaviour change is allowed — `baseline_emission_unchanged`
covers the byte-identical guarantee for both pure single-sub
templates AND the let-binding shape of repeat-sub templates.

If extraction produces different emission than the pre-extraction
behaviour (for either single-sub OR repeat-sub templates), the
extraction has a bug — fix before proceeding to step 0.4.

#### Validation

```bash
# Regression guard for the structural fix:
cargo run --bin loft --release -- tests/scripts/repro_p203.loft
echo "Exit: $?"     # Expected: 0

# Full suite green (must remain so after extraction):
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

#### Extension points for future codegen contributors

If adding new `#rust"…@v…"` templates that reference `@<name>`
multiple times: **the pre-pass already protects you.**  Side-
effecting argument expressions evaluate exactly once.  No
explicit let-binding needed in the template itself.

If the substitution mechanism is ever rewritten beyond plan 09's
extraction (e.g., into per-Op emitters that don't go through
template substitution at all): preserve the let-bind-on-repeat
contract or document why it doesn't apply.

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

## Findings (post-completion, 2026-05-02)

Phase 00 shipped in 6 commits across steps 0.2-0.9.  Honest
evaluation against the broader plan-09 goals:

### What works (proven)

- Byte-identical emission across all 7 corpus files through 6
  hoist commits — the `scripts/p09_fast_gate.sh` ran in <5s after
  each step with zero drift.
- Full `cargo test --test issues`: 540/540, no regressions.
- Native suite: 87/93, unchanged from baseline.
- P203 closure (let-bind-on-repeat) preserved through all hoists.
- Every Op-emission call site (template, user-fn, dispatch.rs
  registry guard, fn-ref dispatch) demonstrably routes through
  `emit_op` and the registry is consulted before each call.

### Three weak spots before the first real custom emitter (phase 05+)

1. **The trait has never run a real custom emitter.**  The empty
   registry means every `emit_op` call falls through to
   `DefaultEmitter`.  Phase 05's first custom emitter will be the
   real test of:
   - Whether `ctx.output.generate_expr_buf(value)` can be called
     from inside `OpEmitter::emit` without lifetime conflicts.
   - Whether emitters need helpers beyond what `EmitCtx` exposes
     (field width, ref flavour, generic bindings, …).
   - Whether the trait's `io::Result<()>` return type suffices, or
     emitters need to surface emission-type info to the caller.
   - **Recommendation**: write a smoke-test custom emitter (~30
     min) before phase 05 so lifetime/helper gotchas surface
     early, not in the middle of a P-issue fix.

2. **`Value::RawExpr` is a wart.**  Step 0.7 added an IR variant
   that has no parser source and no runtime semantics — pure
   codegen plumbing for fn-ref dispatch arg hoisting.  Five walker
   files (`data.rs`, `parser/collections.rs`,
   `parser/expressions.rs ×2`, `state/codegen.rs`) needed default
   arms.  Cumulative cost grows linearly per such addition.
   - **Rule**: no more codegen-only `Value` variants.  If a future
     phase needs to thread synthesized values through codegen,
     build a string-aware companion entry point rather than
     extending `Value`.
   - Enforced by `tests/codegen_emitter.rs::no_unsanctioned_codegen_value_variants`.
     Sanctioned list is `["RawExpr"]`; new entries fail the gate.

3. **Two-layer dispatch in `dispatch.rs::output_call_inner`.**  The
   26-arm special-case match coexists with the registry guard.
   Two registry lookups happen per call (one in
   `has_custom_emitter`, one in `emit_op` — both consulting the
   same registry).  Trivial cost, but conceptually redundant.
   - **Migration target**: drain the 26-arm match to zero by
     migrating each Op into a registered custom emitter.  Phases
     03/04 chip away at this.
   - Enforced by `tests/codegen_emitter.rs::dispatch_op_arm_budget_not_exceeded`.
     Budget starts at 26 (current count); shrink as phases land
     custom emitters; never raise without justification in
     `NATIVE.md`.

### Wart-budget gates added (`tests/codegen_emitter.rs`)

Two gates run as part of `cargo test --test codegen_emitter` (sub-
second total runtime):

| Gate | Counts | Budget |
|---|---|---|
| `dispatch_op_arm_budget_not_exceeded` | match arms in `output_call_inner` whose pattern starts with `"Op...` | 26 (today) — shrink only |
| `no_unsanctioned_codegen_value_variants` | "codegen-internal" / "codegen-only" markers in `Value` enum docstrings | tolerance ≤ 5 markers per sanctioned variant |

Both fail loudly (with prose explaining how to fix) when the
codebase drifts away from phase 00's structural commitments.

### Scaling concerns for phases 01-09

- **Phase 02 (param adapter)**: 10 incremental adapter extractions,
  each must preserve byte-identical emission.  Risk: gradual drift
  vs the baseline.  The fast gate catches it but each iteration
  may take longer than phase 00's hoists did.
- **Phase 03 (parallel-for emitter)**: first complex custom
  emitter.  This is where weak spot #1 manifests — the trait
  either holds up or needs extension.  Probable extensions:
  helper trait methods on `EmitCtx` for closure-shape selection,
  return-type analysis, extra-arg binding emission.
- **Phase 05 (file emitter, P200 write)**: needs `int_width_for(value)`
  and `int_signed_for(value)` accessors.  These don't exist yet.
  Either add them to `EmitCtx` (forward to `Output`) or expose via
  `ctx.output` directly.  Either way `EmitCtx`'s surface area
  grows.
- **Phase 07 (generic text emitter, P205)**: needs resolved
  generic-binding info.  Whether existing `Output` state surfaces
  enough info is unclear without trying.

### Verdict

The abstraction is **fit for purpose** for plan 09's phases 01-09.
Pragmatic compromises (`&mut Output<'b>` in EmitCtx, `Value::RawExpr`)
ship today and don't paint us into a corner.  The wart-budget gates
prevent silent accretion.  The first real custom emitter (phase 05)
is the next stress test — write a smoke test before then.
