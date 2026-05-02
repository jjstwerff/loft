# Phase 02 — Retire `forwarding_smoke.rs`

**Status:** OPEN

**Closes:** dead-weight in the production registry.

**Tier:** 1 (cleanup + cheap)

**Estimated cost:** 10-15 minutes.

## Why retire

Phase 00 of plan-09 introduced
`src/generation/ops/forwarding_smoke.rs` containing
`ForwardingEmitter` — a no-op emitter that just delegates to
`DefaultEmitter::emit`.  9 Op names were registered to it as a
runtime smoke test proving the dispatch path fires.

That smoke test served its purpose.  Plan-09 + plan-11 shipped
**5 production custom emitters**:
- `ParallelForEmitter` (phase 03)
- `OpGetRecordEmitter` + `OpIterateEmitter` (phase 04)
- `ParallelQueueEmitter` + `ParallelBufRenameEmitter` (phase 06)
- `IntCompareEmitter` (phase 10 step 10.3)

Plus the implicit P205 fix at the emit.rs level (no custom
emitter, but the dispatch is exercised heavily).

The 9 forwarding entries are now:
- Pure overhead (registry lookup that always returns
  `DefaultEmitter` behaviour).
- Conceptually misleading ("custom emitter registered" — but it
  does nothing custom).
- Unnecessary surface area for future contributors to navigate.

## Detailed steps

### Step 2.1 — Identify the affected sites

```bash
cat src/generation/ops/forwarding_smoke.rs   # the emitter + name list
grep -n "FORWARDING_OP_NAMES\|ForwardingEmitter\|forwarding_smoke" \
    src/generation/ops/mod.rs tests/codegen_emitter.rs
```

The for-loop in `build_registry`:
```rust
for op_name in forwarding_smoke::FORWARDING_OP_NAMES {
    r.insert(op_name, Box::new(forwarding_smoke::ForwardingEmitter));
}
```

### Step 2.2 — Verify no real consumer relies on the forwarding entries

Check that no code path NEEDS one of the 9 forwarded Op names
to be in the registry (vs. falling through to `DefaultEmitter`):

```bash
# Check if any test or code path explicitly looks up these names:
for name in $(grep -oE '"Op[A-Z][a-zA-Z]+"' src/generation/ops/forwarding_smoke.rs); do
    grep -rn "$name" src/ tests/ default/ | grep -v "forwarding_smoke" | head -5
done
```

If any production code expects these names to be IN the registry
(rather than falling through), keep those specific entries and
retire only the rest.  Most likely: zero such cases.

### Step 2.3 — Remove the registration loop

Edit `src/generation/ops/mod.rs::build_registry` to delete the
`for op_name in forwarding_smoke::FORWARDING_OP_NAMES` block.

### Step 2.4 — Remove the file

```bash
git rm src/generation/ops/forwarding_smoke.rs
```

Update `src/generation/ops/mod.rs` to remove the
`pub mod forwarding_smoke;` declaration.

### Step 2.5 — Refresh fast-gate baseline

The byte-identical baseline at `/tmp/p09-baseline/` may include
emission shapes that depended on the forwarding emitter being
registered.  In practice it shouldn't — forwarding is
behaviour-identical to `DefaultEmitter` — but refresh defensively:

```bash
scripts/p09_fast_gate.sh --capture
```

### Step 2.6 — Run regression sweep

```bash
cargo build --release --tests
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 95/95
cargo test --release --test codegen_emitter 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy --tests --release -- -D warnings
```

Per zero-regression rule: any regression aborts.  The expected
outcome: zero behavioural change.  Suite stays at 95/95 native.

### Step 2.7 — Update fast-gate's "custom emitters registered" count

`scripts/p09_fast_gate.sh` reports
`custom emitters registered: N (X individual + Y via lists)`.
After plan-12 phase 02:
- "via lists" drops by 9 (the forwarding loop)
- "individual" stays unchanged

Update any plan-09 docs that reference the absolute count if it
matters; mostly it's informational.

### Step 2.8 — Update README

Plan-12 README's progress markers and any doc cross-reference
to forwarding-smoke should clean up.  Plan-09's phase 00 doc
references the smoke test — leave that as historical record;
just note in the phase 02 § Findings that the smoke test served
its purpose and was retired post-plan-09.

## Acceptance

```bash
# File doesn't exist
[ ! -f src/generation/ops/forwarding_smoke.rs ]

# Module declaration removed
! grep -q "pub mod forwarding_smoke" src/generation/ops/mod.rs

# Registration loop removed
! grep -q "FORWARDING_OP_NAMES" src/generation/ops/mod.rs

# Suite still green
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 95/95
```

All clean.

## Risks

| Risk | Mitigation |
|---|---|
| One of the 9 forwarded Ops actually NEEDS the registry entry to fire emission differently | Step 2.2's grep audit catches this.  If found, keep that specific entry. |
| Tests that count registry size regress | Update the count assertion in the structural test (it's a number, not a behavioural assertion). |
| The "9 via lists" entries served as a discoverability hint for new contributors | The plan-09 phase 00 doc still describes the forwarding-first recipe; that's the actual onboarding path, not the production registry entries. |

## Findings

_(populate during steps 2.2 + 2.6 — note any Op that turned out
to need its entry vs. could be safely removed)_
