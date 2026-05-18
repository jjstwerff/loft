# Phase 04 — Migrate free + record dispatch arms

**Status:** OPEN

**Closes:** ~10 hardcoded match arms in
`src/generation/dispatch.rs::output_call_inner` for the Free*
and Record/Ref-comparison Op families.

**Tier:** 2 (structural cleanup)

**Effort:** S.

## What's being migrated

Free + cleanup ops:
- `OpFreeText | OpCreateStack` (single arm, returns `Ok(())`)
- `OpFreeRef`
- `OpFreeRefIfDistinct`

Record / DbRef ops:
- `OpCopyRecord`
- `OpEqRef`
- `OpNeRef`
- `OpNullRefSentinel`

Convert ops:
- `OpConvTextFromNull`
- `OpConvRefFromNull`

Coroutine ops:
- `OpCoroutineNext`
- `OpCoroutineExhausted`

GetTextSub:
- `OpGetTextSub`

Total: ~12 arms across these clusters.  Some have natural
groupings (DbRef equality, Coroutine pair, ConvFromNull pair);
others are one-offs.

## Why migrate

Same rationale as @PLAN12 phase 03: drain dispatch.rs's special-
case match toward zero.  These arms differ from the format/append
cluster — they tend to have UNIQUE bodies (no shared helper),
so the consolidation factor is lower.  Each arm becomes its own
small emitter (~15-30 lines).

## Suggested factoring

Group by behaviour shape:

| Emitter file | Ops handled | Why grouped |
|---|---|---|
| `free_ops.rs` | OpFreeText, OpCreateStack, OpFreeRef, OpFreeRefIfDistinct | All cleanup-shape |
| `dbref_compare.rs` | OpEqRef, OpNeRef, OpNullRefSentinel | DbRef equality / null-check |
| `dbref_copy.rs` | OpCopyRecord | Single Op; could share file with others if shape converges |
| `coroutine_state.rs` | OpCoroutineNext, OpCoroutineExhausted | Coroutine state-machine pair |
| `conv_from_null.rs` | OpConvTextFromNull, OpConvRefFromNull | Null-conversion pair |
| `op_get_text_sub.rs` | OpGetTextSub | One-off; substring extraction |

This is a SUGGESTED factoring; the actual split happens at
implementation time based on which arms share enough body to
co-locate.

## Detailed steps

### Step 4.1 — Survey existing arm bodies

Same as phase 03 step 3.1 but for these arms:

```bash
for op in OpFree OpCopyRecord OpEqRef OpNeRef OpNullRefSentinel \
          OpConvTextFromNull OpConvRefFromNull OpCoroutineNext \
          OpCoroutineExhausted OpGetTextSub OpClearVector; do
    grep -B1 -A20 "\"${op}" src/generation/dispatch.rs
    echo "---"
done > /tmp/p12-step4-arms.txt
```

For each arm: helper functions invoked, argument shape, special
context flow.

### Step 4.2 — Forwarding-first per the recipe

For each arm, register a forwarding emitter delegating to
`DefaultEmitter`.  Run byte-identical baseline.  If green per
arm, swap to real implementation in subsequent commits.  If not,
the arm has special context flow that DefaultEmitter can't
replicate — write the real emitter directly.

**Pre-flight check** (per the recipe): some of these arms
DON'T have `#rust"..."` annotations in `default/01_code.loft`
(they're inline-only).  For those, forwarding will FAIL the
byte-identical baseline immediately — that's expected.  Skip
forwarding for those Ops; write the real emitter directly.

### Step 4.3 — Implement the real emitters per cluster

```rust
// src/generation/ops/free_ops.rs (example)
pub struct FreeOpsEmitter;
impl OpEmitter for FreeOpsEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        match ctx.def_fn.name.as_str() {
            "OpFreeText" | "OpCreateStack" => Ok(()),  // void emission
            "OpFreeRef" => { /* absorb arm body */ }
            "OpFreeRefIfDistinct" => { /* absorb arm body */ }
            _ => super::default::DefaultEmitter.emit(ctx, args),
        }
    }
}
```

Each cluster's emitter file gets its own `*Emitter` struct.
Registration in `build_registry` adds one `r.insert(name, ...)`
per Op.

### Step 4.4 — Per-cluster structural tests

For each cluster, add a structural test in
`tests/codegen_emitter.rs`:

```rust
#[test]
fn p12_04_free_ops_emitter_registered() {
    // Pin the registration of the cluster's Op names + check
    // dispatch.rs no longer has the corresponding match arms.
    ...
}

#[test]
fn p12_04_dbref_compare_emitter_registered() { ... }
// etc.
```

### Step 4.5 — Run acceptance gate

```bash
scripts/p09_fast_gate.sh
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 95/95
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test codegen_emitter 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy --tests --release -- -D warnings
```

Zero-regression rule applies.

### Step 4.6 — Update wart-budget gate count

Phase 03 took count from 24 → ~12.  Phase 04 takes it from
~12 → ~2 (only really-special-case arms remain).  The
remaining 2-ish arms might be:
- One that has truly unique context flow that resists
  migration (document in § Findings).
- Or a defensive default-dispatch arm.

Update the `dispatch_op_arm_budget_not_exceeded` test's
expected budget after phase 04 lands.

## Acceptance

```bash
cargo test --release --test codegen_emitter -- p12_04
# + full regression sweep
```

Dispatch.rs arm count: ~12 → ~2 (or whatever ends up irreducible).

## Risks

| Risk | Mitigation |
|---|---|
| OpCopyRecord has hidden type-erasure logic that needs the dispatch's context | The forwarding-first recipe catches this; if forwarding fails baseline, write the real emitter explicitly absorbing the context. |
| OpCoroutineNext / OpCoroutineExhausted touch coroutine state-machine internals | Co-locate with coroutine emission code; share the state-machine helpers. |
| Some arms can't migrate cleanly because they were intentional special cases | Document in § Findings.  Don't force migration if a special case is genuinely simpler than an emitter. |

## Commit shape

4-6 commits, one per cluster:
1. `free_ops.rs` + tests + dispatch.rs free arms removed.
2. `dbref_compare.rs` + tests + dispatch.rs OpEqRef/OpNeRef/etc. removed.
3. `coroutine_state.rs` + tests + arms removed.
4. `conv_from_null.rs` + tests + arms removed.
5. (optional) Polish + remaining one-offs.

## Findings

_(populate during step 4.1 — per-cluster spec; note any arm that
resists migration)_

## Implementation notes

_(append per non-obvious decision)_
