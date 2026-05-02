# Phase 03 — Migrate format + append dispatch arms

**Status:** OPEN

**Closes:** ~12 hardcoded match arms in
`src/generation/dispatch.rs::output_call_inner` for the Format*
and Append* Op families.

**Tier:** 2 (structural cleanup)

**Estimated cost:** ~3-4 hours.

## What's being migrated

Current dispatch.rs has hardcoded inline emission for these Ops:

**Format ops (8 arms):**
- `OpFormatInt | OpFormatStackInt`
- `OpFormatFloat | OpFormatStackFloat`
- `OpFormatSingle | OpFormatStackSingle`
- `OpFormatText | OpFormatStackText` (via `format_text` helper)

**Append ops (4 arms):**
- `OpAppendText`
- `OpAppendStackText`
- `OpAppendCharacter | OpAppendStackCharacter`

Plus 2 cleanup-Op aliases:
- `OpClearStackText | OpClearText` (single arm)
- `OpClearVector` (single arm)

Total: ~12 arms across closely-related Op families.

## Why migrate

The wart-budget gate
(`tests/codegen_emitter.rs::dispatch_op_arm_budget_not_exceeded`)
caps the count at 26.  Currently 24 (plan-09 retired
n_parallel_for + OpGetRecord + OpIterate).  Each remaining arm
is a simplification candidate — moving it to a custom emitter
reduces the special-case count and puts emission logic next to
its definition.

The format/append family is the highest-coherence cluster — all
12 arms share emission shape (write into a target text/vector
buffer) and a small number of helper functions (`format_text`,
`append_text`, etc.).  A single `FormatAppendEmitter` family
file in `src/generation/ops/` can absorb them with ~20 lines per
arm.

## Detailed steps

### Step 3.1 — Survey the existing arm bodies

```bash
grep -B1 -A20 '"OpFormat\|"OpAppend\|"OpClearStack\|"OpClearVector\|"OpClearText' \
    src/generation/dispatch.rs > /tmp/p12-step3-arms.txt
wc -l /tmp/p12-step3-arms.txt
```

For each arm, document:
- Exact Op names handled
- Helper functions invoked (`self.format_text`, `self.append_text`, etc.)
- Argument shape (which `vals[N]` consumed)
- Any context flow (does the arm read state outside `vals`?)

Output: a per-arm spec sheet that the emitter family will
implement.

### Step 3.2 — Forwarding-first per the recipe

Per `feedback_forwarding_first_recipe.md`: register a forwarding
emitter for each Op name FIRST, run the byte-identical baseline,
confirm green, THEN swap to real emission.

```rust
// src/generation/ops/format_append.rs (new file)
use super::{EmitCtx, OpEmitter, default::DefaultEmitter};
use crate::data::Value;
use std::io;

pub struct FormatAppendForwarder;

impl OpEmitter for FormatAppendForwarder {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        DefaultEmitter.emit(ctx, args)
    }
}
```

**Pre-flight check** (per the recipe): confirm none of the 12
arms produce output that DefaultEmitter can't replicate.  All
12 arms call helper fns (`format_text`, `append_text`, etc.)
that ARE template-substituted via `default/01_code.loft`'s
`#rust"..."` annotations.  So forwarding to DefaultEmitter
should produce equivalent output.

If forwarding produces byte-identical baseline → safe to swap to
real implementations in subsequent commits.  If not → diagnose
the divergence before continuing (likely a special context-flow
case that the dispatch arm handles inline).

### Step 3.3 — Implement the real emitters

Replace the forwarder with shape-specific emitters.  The
suggested factoring:

```rust
// FormatEmitter — handles all 8 OpFormat* variants
pub struct FormatEmitter;
impl OpEmitter for FormatEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Dispatch on ctx.def_fn.name to pick the format helper.
        // Body absorbs the dispatch.rs arm logic verbatim.
        ...
    }
}

// AppendEmitter — handles all 4 OpAppend* + 2 OpClear* variants
pub struct AppendEmitter;
impl OpEmitter for AppendEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // Same shape: dispatch on name, absorb the arm body.
        ...
    }
}
```

Each emitter owns a small `match ctx.def_fn.name.as_str() { ... }`
that absorbs the original arm's logic.  The arms in
dispatch.rs get DELETED (not just commented out — clean removal).

### Step 3.4 — Per-Op structural test

Add to `tests/codegen_emitter.rs`:

```rust
#[test]
fn p12_03_format_append_emitters_registered() {
    let src = std::fs::read_to_string(project_root().join("src/generation/ops/mod.rs"))
        .expect("read ops/mod.rs");
    for name in [
        "OpFormatInt", "OpFormatStackInt",
        "OpFormatFloat", "OpFormatStackFloat",
        "OpFormatSingle", "OpFormatStackSingle",
        "OpFormatText", "OpFormatStackText",
        "OpAppendText", "OpAppendStackText",
        "OpAppendCharacter", "OpAppendStackCharacter",
        "OpClearStackText", "OpClearText", "OpClearVector",
    ] {
        let pat = format!("\"{name}\"");
        assert!(
            src.contains(&pat),
            "{name} must be registered after plan-12 phase 03"
        );
    }
}

#[test]
fn p12_03_dispatch_format_arms_retired() {
    let src = std::fs::read_to_string(project_root().join("src/generation/dispatch.rs"))
        .expect("read dispatch.rs");
    for name in [
        "\"OpFormatInt\"", "\"OpAppendText\"", "\"OpClearStackText\"",
        // representative subset — full list per step 3.1
    ] {
        assert!(
            !src.contains(&format!("{name} =>")),
            "{name} match arm should be retired in dispatch.rs after plan-12 phase 03"
        );
    }
}
```

### Step 3.5 — Run the acceptance gate

```bash
# Byte-identical (or refresh if intentional emission change)
scripts/p09_fast_gate.sh

# Full suite
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 95/95
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test codegen_emitter 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy --tests --release -- -D warnings
```

Per zero-regression rule: any failure aborts.

### Step 3.6 — Update wart-budget gate count

The `dispatch_op_arm_budget_not_exceeded` test reports the
remaining count.  After phase 03 the count should drop from 24
to ~12 (depends on whether any arm splits into multiple emitter
registrations or vice versa).  Update the gate's expected
budget value.

## Acceptance

```bash
cargo test --release --test codegen_emitter::p12_03_format_append_emitters_registered
cargo test --release --test codegen_emitter::p12_03_dispatch_format_arms_retired
# + full regression sweep
```

All green.  Dispatch.rs arm count: 24 → ~12.

## Risks

| Risk | Mitigation |
|---|---|
| One arm has hidden context flow that DefaultEmitter can't replicate | Forwarding-first catches it; arm stays a special case until investigated. |
| Emitter consolidation (e.g. one `FormatEmitter` for 8 Ops) makes individual arm logic harder to find | Inline-comment each branch with the original arm's location for archeology. |
| Migration changes byte-identical baseline | Acceptable when intentional (the new emitter writes the same code differently); refresh per-step.  Acceptance: behavioural outcome unchanged. |

## Commit shape

3-4 commits:
1. Forwarding emitters registered + structural test for registration.
2. `FormatEmitter` real implementation + dispatch.rs format arms removed.
3. `AppendEmitter` real implementation + dispatch.rs append/clear arms removed.
4. (optional) Polish — emitter unit tests, doc comments.

## Findings

_(populate during step 3.1 — per-arm spec sheet)_

## Implementation notes

_(append per non-obvious decision — particularly any arm that
resists migration)_
