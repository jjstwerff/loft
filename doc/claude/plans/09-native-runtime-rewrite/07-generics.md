# Phase 07 — Generic text emitter

**Status:** OPEN

**Closes:** **P205** (bounded-generic text return emits
`Str::new(&local_String)` — dangling pointer).

**Reproducer:** `tests/scripts/repro_p205.loft`.

**Depends on:** Phase 00 (scaffold).  Phase 02 (param adapter) is
recommended but not strictly required if the probe path lands.

## Diagnosis

### Symptom
A bounded-generic function `fn name<T: SomeTrait>() -> text` whose
body returns a `String` produces native code emitting
`Str::new(&local_String)` — a borrow into a stack-local that dies
when the function returns.

### Root cause beyond the symptom
Two interacting forces:

1. `src/parser/control.rs:369-377` skips a `text_return` promotion
   for `DefType::Generic`.  The skip exists for a reason — likely
   to avoid a different cascade — but means the generic path falls
   through to the default text-return template that assumes the
   text source outlives the return.
2. The template can't case-split on "is the generic binding's
   resolved type owned-String or borrowed-`Str`."  Even if the
   skip is removed, the template would still emit the wrong shape
   for some bindings.

## Prior attempts

None recorded.  The skip looks load-bearing but the specific
reason hasn't been documented.

## Why this works now

Two complementary fixes — either alone might cascade:

- **Skip-removal probe** (parser-side, ~1 line): delete the
  `DefType::Generic` skip in a worktree.  Outcomes:
  - **Suite stays green** → ship the 1-line fix; phase 07's
    emitter is unnecessary.
  - **Suite cascades** → diagnose what the skip protects, then
    write the emitter.
- **Emitter** (codegen-side, ~30 lines): replaces the
  borrow-of-local emission with owned-`String` emission.  Works
  whether or not the skip stays.

A 30-minute probe disambiguates between "1-line parser fix" and
"emitter required."

## Detailed steps with validation

### Step 7.1 — Confirm reproducer fails today

**Action**:
```bash
cargo run --bin loft --release -- tests/scripts/repro_p205.loft
echo "Exit: $?"
# Expected: nonzero (P205 active).
cargo run --bin loft --release -- --interpret tests/scripts/repro_p205.loft
echo "Exit: $?"
# Expected: zero (interpreter is correct; only native broken).
```

**Validation**: confirms the bug surface — native fails,
interpreter works.

### Step 7.2 — Inspect the offending generated code

**Action**:
```bash
cargo run --bin loft --release --quiet -- \
    --native-emit /tmp/p205.rs tests/scripts/repro_p205.loft
grep -n "Str::new(&" /tmp/p205.rs
```

**Validation**: locate the offending `Str::new(&local)` lines.
Note the function name + Op surrounding the borrow — needed for
emitter registration in step 7.5.

### Step 7.2b — Corpus-wide survey for sibling dangles

**Action**: P205's reproducer surfaces ONE Op with the
borrow-of-local pattern.  But the same defect may exist for
sibling Ops (other generic-text-return paths, interface dispatch,
fn-ref-of-text-return).  Survey the doc-test corpus to find them
all before declaring P205 closed.

```bash
# Compile every doc test under --native-emit, grep for the
# dangling shape across all generated code:
mkdir -p /tmp/p205-survey
for t in tests/docs/*.loft tests/scripts/*.loft; do
    name=$(basename "$t" .loft)
    cargo run --bin loft --release --quiet -- \
        --native-emit "/tmp/p205-survey/$name.rs" "$t" 2>/dev/null
done

# All occurrences of `Str::new(&_local_*)` — the dangling pattern:
grep -rn "Str::new(&_local_" /tmp/p205-survey/ | sort -u

# Capture context (function name + surrounding Op) for each hit.
# Each unique (function, Op) pair is a candidate for the
# emitter / probe in steps 7.3-7.5.
```

**Validation**: produces a list of all (test, function, Op)
tuples where the dangling shape appears.  This list is the FULL
scope of P205, not just the reproducer's scope.

**Decision**:
- If the list contains ONLY the reproducer's Op → phase 07 scope
  is correct as-is; proceed to step 7.3.
- If the list contains ADDITIONAL Ops → phase 07 scope expands.
  Each additional (function, Op) pair gets the same probe-or-
  emitter treatment.  Document in "Diagnosis findings" before
  proceeding.
- If the list is empty → P205 doesn't surface in the doc corpus at
  all.  Either the reproducer is misleading or the corpus is too
  narrow.  Investigate before continuing.

### Step 7.3 — Skip-removal probe

**Action**: in an isolated worktree:
```bash
git worktree add /tmp/p205-probe -B p205-probe HEAD
cd /tmp/p205-probe

# Edit src/parser/control.rs:369-377 to remove the
# `if matches!(def_type, DefType::Generic) { return; }` (or
# whatever shape the skip has — read the actual code first).

cargo build --release
cargo test --release --test issues 2>&1 | tail -20
cargo test --release --test wrap generics 2>&1 | tail -20
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | tail -10

# Specifically check the reproducer:
cargo run --bin loft --release -- tests/scripts/repro_p205.loft
echo "After skip removal: $?"
```

**Validation**:
- **Outcome A**: full suite green AND repro_p205 exit 0 → skip
  isn't load-bearing.  Branch to step 7.4 (1-line fix).  Skip step
  7.5 entirely.
- **Outcome B**: any regression OR repro_p205 still fails → skip
  protects something.  Document which tests broke and why.
  Continue to step 7.5 (emitter).

Document outcome in "Diagnosis findings" below.

### Step 7.4 — (Outcome A only) Land the 1-line parser fix

**Action**: in main checkout, apply the same edit.  Add a regression
test:
```rust
#[test]
fn p205_generic_text_return_no_dangle() {
    let status = std::process::Command::new("cargo")
        .args(["run", "--bin", "loft", "--release", "--",
               "tests/scripts/repro_p205.loft"])
        .status().unwrap();
    assert!(status.success(), "P205 reproducer fails — fix regressed");
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::p205_generic_text_return_no_dangle
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test wrap generics
```

Update PROBLEMS.md and skip step 7.5.

### Step 7.5 — (Outcome B only) Implement the emitter

**Action**: identify the exact Op name from step 7.2.  Create
`src/generation/ops/op_return_generic_text.rs` (or whatever the
real Op is named):

```rust
pub struct Emitter;

impl OpEmitter for Emitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, dyn Write>, args: &[Value]) -> io::Result<()> {
        let [value] = args else { panic!("OpReturnGenericText arity") };
        // Owned String — caller decides ownership.  No borrow-of-local.
        write!(ctx.w, "{}", ctx.emit(value)?)?;
        Ok(())
    }
}
```

Register it.

If outcome B revealed specific tests that the skip protects,
extend the emitter to handle those cases without falling into the
cascade pattern.  Document in "Implementation notes."

**Validation**:
```bash
cargo test --release --test codegen_emitter::p205_generic_text_return_no_dangle
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test wrap generics
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
# Confirm the previously-broken tests from step 7.3 outcome B
# now also pass.
```

### Step 7.6 — Add structural test pinning the fix

**Action**:
```rust
#[test]
fn p205_no_str_new_of_local_in_generic_text_return() {
    let src = compile_to_rust("tests/scripts/repro_p205.loft");
    // The dangling shape is `Str::new(&_local_*)`.  Forbid it.
    assert!(!src.contains("Str::new(&_local_"),
        "P205: generated code still emits Str::new of a local — \
         dangling pointer regression");
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::p205_no_str_new_of_local_in_generic_text_return
```

### Step 7.7 — Update PROBLEMS.md

**Action**: mark P205 CLOSED with "fix path: phase 07 of plan 09
(outcome A: skip removal / outcome B: emitter)".  Reference the
regression tests added.

**Validation**: review.

## Acceptance for phase 07 overall

```bash
cargo test --release --test codegen_emitter::p205_generic_text_return_no_dangle
cargo test --release --test codegen_emitter::p205_no_str_new_of_local_in_generic_text_return
cargo test --release --test wrap generics
cargo test --release --test issues 2>&1 | tail -3
cargo run --bin loft --release -- tests/scripts/repro_p205.loft   # exit 0
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
```

## Commit shape

3-5 commits depending on outcome:
- Outcome A: probe + 1-line fix + regression test = 2-3 commits
- Outcome B: probe + emitter + regression tests = 4-5 commits

## Diagnosis findings

_(populate during pre-work probe — Outcome A or B; if B, list the
specific tests that broke and what they relied on)_

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision)_
