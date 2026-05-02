# Phase 07 — Generic text emitter

**Status:** DONE (2026-05-02)

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

> **Pre-flight (per [forwarding-first recipe](00-scaffold.md#verifying-a-new-op-emitter-the-forwarding-first-recipe))**:
> identify the offending Op via step 7.2 first.  Then check
> `grep -n '"<OpName>" =>' src/generation/dispatch.rs` — if hit,
> the real emitter must absorb that arm's logic; if empty, forwarding
> is safe and step 7.5 follows the recipe.

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

#### Probe attempt 1 (2026-05-02): Outcome B confirmed

Removed `text_return`'s gate at `parser/control.rs:375` so it runs
for `DefType::Generic` (the gate now applies only to
`ref_return`/`Vector`/`Reference`/`Enum` paths).  Generic
specialisations DID then receive `text_return`-shaped emission,
but the dangling pattern persists:

```rust
fn t_6P205_S_p205_label(cell: &..., mut var_p205_x: DbRef) -> Str {
    let mut var___ret_1: String =
        t_6P205_S_p205_to_label(cell, var_p205_x).to_string();
    return Str::new(&var___ret_1)   // ← var___ret_1 dropped at return
}
```

`text_return`'s shape is a buffer-promotion: it expects to convert
`-> text` returns into `-> ()` with a `&mut String` write-buffer
parameter.  But for the bounded-generic specialisation it only
created the `var___ret_1: String` local without changing the
function signature — so the function still returns `Str` and the
local dangles.

Conclusion: the bug is not the skip itself; `text_return`'s
transformation isn't complete enough for the bounded-generic case.
Outcome B applies — proceed to step 7.5 (custom emitter).

The emitter approach sidesteps `text_return` entirely by emitting
owned `String` from the Op directly, eliminating the buffer
indirection that `text_return` was trying to create.

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

## Gate updates per step

Branches by probe outcome:

- **Outcome A (skip-removal clean)**: parser-only fix.  Gate's
  emission shape changes for generic text returns — refresh
  baseline if corpus baselines reference such paths.  No
  `custom_count` change.
- **Outcome B (custom emitter required)**: `OpReturnGenericText`
  emitter registered.  `custom_count` += 1.  Baseline refresh
  required (emission shape changes for affected sites).

Both outcomes add `p205_*` regression tests.

## Commit shape

3-5 commits depending on outcome:
- Outcome A: probe + 1-line fix + regression test = 2-3 commits
- Outcome B: probe + emitter + regression tests = 4-5 commits

## Diagnosis findings

_(populate during pre-work probe — Outcome A or B; if B, list the
specific tests that broke and what they relied on)_

## Problems encountered

### Two emission sites, not one (2026-05-02)

The plan-doc anticipated registering an `OpReturnGenericText` emitter
for "the dangling Op."  Implementation revealed the dangle isn't
emitted by a single Op — it's emitted by `src/generation/emit.rs`'s
return-handling code at TWO sites:

1. **`Value::Return(val)` emission** (`emit.rs:155-200`).  When the
   function returns Type::Text and the return value is a Var (not
   a Call returning Str), the code wraps with `Str::new(<value>)`.
2. **Block-tail wrap_result emission** (`emit.rs:884-905`).  When the
   block's return expression is a text value, same `Str::new(<value>)`
   wrapping.

Both sites fired for the P205 case — the actual dangling depended
on which path the IR took for a given test.  Fix had to go in
both places.

### `text_return` doesn't set `hidden=true` (2026-05-02)

Initial fix detection used `a.hidden && Type::RefVar(Type::Text(_))`.
This filtered out EVERY text-returning function because
`text_return`'s `add_attribute` call (`parser/control.rs:2358`)
doesn't set `hidden=true` (only `ref_return` does, at line 2452).
Fix: drop the `hidden` filter — just check for `Type::RefVar(Type::Text(_))`
attribute presence.  Documented in the emit.rs comment so future
maintainers don't re-add the filter.

## Implementation notes

### Emit-time scratch routing (2026-05-02)

Instead of writing a custom `OpEmitter` (the original phase-doc
plan), the fix patches the return-emission code paths directly.
Two new branches in `Value::Return` (line 188+) and the
block-tail wrap_result code (line 887+):

```rust
let needs_p205_scratch = wrap_text /* or wrap_result */ && {
    let def = self.data.def(self.def_nr);
    matches!(def.returned, Type::Text(_))
        && !def.attributes.iter().any(|a| {
            matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_)))
        })
};
if needs_p205_scratch {
    write!(w, "{{ stores.scratch.push((")?;
    // emit value
    write!(w, ").to_string()); Str::new(stores.scratch.last().unwrap()) }}")?;
}
```

The detection: if the function returns `Type::Text` but has NO
`Type::RefVar(Type::Text(_))` attribute (= `text_return` didn't
set up a proper work buffer for it), then `Str::new(<value>)`
would borrow into a dropping local.  Route through `stores.scratch`
instead — the scratch entry lives as long as `stores`, so the
returned `Str` pointer stays valid for the caller's use.

The `(value).to_string()` coerces &str / String / Str all into
an owned String for the scratch push.

**Why not a custom emitter**: the dangle isn't tied to a single
Op — it's emit.rs's choice to wrap with `Str::new(...)` for
text-returning functions.  Two emit sites needed the fix; an
`OpEmitter` would have to intercept all text-Op calls inside
those functions, which is much more invasive than 4 lines per
emit site.

**Why not fix `text_return` parser-side**: phase 07's probe
(2026-05-02 attempt 1) tried this — removing the `DefType::Generic`
gate at `parser/control.rs:375`.  Result: text_return ran but
didn't promote any work buffer because the bounded-generic
specialisation has no local text variables to promote.  The
function signature still returned `Str` and the body still
borrowed from a local String.  Outcome B confirmed.

### Over-eager firing on inner functions (2026-05-02)

The fix fires for any text-returning function without a
`Type::RefVar(Type::Text(_))` attribute.  This includes some
functions that wouldn't actually dangle today — e.g.
`p205_to_label` returns `self.p205_name` which fetches a `&str`
from the store (long-lived).  For those, the scratch routing
produces correct (but slower-by-one-clone) code.

This is acceptable because:
- The clone happens once per call, not per inner operation.
- Scratch entries are cheap pushes onto a Vec<String>.
- The pattern is consistent across all text-returning fns
  without a work buffer — easier to reason about than a
  conditional behaviour.

If profiling later shows scratch routing is a hot path, narrow
the detection to "function returns Type::Text AND its return
expression's source is a local String binding."  Today's scope
is a correctness fix; performance optimization deferred.

### Why `stores.scratch` (not `Box::leak` or thread-local) (2026-05-02)

Three options considered for the long-lived String storage:

1. **`Box::leak(String::into_boxed_str())`** — `'static &str` lifetime.
   Memory leak per call.  Avoided.
2. **Thread-local `Vec<String>`** — clean lifetime, but adds a new
   global state surface to the runtime.
3. **`stores.scratch`** — already exists for
   `n_parallel_buf_get_text_native`.  Lives as long as `stores`.
   No new surface area.

Picked option 3 — reuses existing infrastructure.  The growth
concern (unbounded scratch) applies to all three options that
don't free; option 3 is at least co-located with other
similar-lifetime data.
