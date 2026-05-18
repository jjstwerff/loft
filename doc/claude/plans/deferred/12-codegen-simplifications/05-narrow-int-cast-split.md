# Phase 05 — Split `narrow_int_cast` dual role

**Status:** OPEN

**Closes:** the dual-role complexity hub in
`src/generation/mod.rs::narrow_int_cast`.  Originally this was
@PLAN09 phase 02's scope; phase 00a demoted phase 02 because
the actual @P200 fix didn't need it, but the underlying
complexity remains and is worth splitting for future
maintainability.

**Tier:** 2 (structural cleanup)

**Effort:** S-M.

## What's tangled today

`narrow_int_cast(tp: &Type) -> Option<&'static str>` at
`src/generation/mod.rs:266`:

```rust
fn narrow_int_cast(tp: &Type) -> Option<&'static str> {
    match tp {
        Type::Integer(s) if s.range() - 1 <= 255 && i64::from(s.min) >= 0 => Some("u8"),
        Type::Integer(s) if s.range() - 1 <= 65536 && i64::from(s.min) >= 0 => Some("u16"),
        Type::Integer(s) if s.range() - 1 <= 255 => Some("i8"),
        Type::Integer(s) if s.range() - 1 <= 65536 => Some("i16"),
        _ => None,
    }
}
```

It serves TWO roles:

**Role 1 — Block-tail coercion** (5 sites in `emit.rs`):
- `emit.rs:162`: Value::Return text-wrap path
- `emit.rs:178`: Value::Return narrow path
- `emit.rs:901`: block-tail wrap_result narrow
- `emit.rs:965`: trailing-void block path
- `emit.rs:977`: empty-block path

These sites narrow the BLOCK'S RESULT to a specific Rust type.
Used for function-return narrowing AND for inline expressions
whose Type::Integer subtype is narrow.

**Role 2 — Parameter narrowing** (in `calls.rs::output_call_template`):
- Used during `#rust"..."` template substitution to decide
  whether to suffix-patch an argument with `as u8` / `as u16`
  etc.  ~125 lines of stacked `if matches!(...)` (the original
  @PLAN09 phase 02 target).

## Why split

The dual role creates a "fix one, break the other" trap.  @P200's
original write-side fix attempt collided with this exact issue.
Plan-09 phase 00a found the actual @P200 bug was elsewhere (it
became phase 10 step 10.3's `IntCompareEmitter`), but the
dual-role hub remained in place.

Splitting the function lets each role evolve independently:

```rust
// Block-tail role: still returns Option<&'static str>; signature
// stays the same to minimise call-site churn.
fn block_tail_narrow_for_int(tp: &Type) -> Option<&'static str> { ... }

// Parameter narrowing role: same signature, different body
// (might handle e.g. signed-vs-unsigned differently for params,
// or use a different range threshold).
fn param_narrow_for_int(tp: &Type) -> Option<&'static str> { ... }
```

Today both roles share the SAME body.  Splitting + duplicating
keeps current behaviour identical; future changes to one role
don't bleed into the other.

## Detailed steps

### Step 5.1 — Survey all call sites

```bash
grep -rn "narrow_int_cast" src/ | head -20
```

For each call site, document:
- Which role: block-tail or param?
- File:line of the caller
- What the result is used for (write `as u8` cast / decide
  whether to apply suffix patch / etc.)

### Step 5.2 — Add the two new functions side-by-side

In `src/generation/mod.rs`, add `block_tail_narrow_for_int` and
`param_narrow_for_int` with bodies IDENTICAL to today's
`narrow_int_cast`.  Document each fn's role in its doc comment.

Keep `narrow_int_cast` for now as a deprecated wrapper:

```rust
#[deprecated(note = "use block_tail_narrow_for_int or param_narrow_for_int based on call-site role")]
fn narrow_int_cast(tp: &Type) -> Option<&'static str> {
    block_tail_narrow_for_int(tp)
}
```

### Step 5.3 — Update each call site to use the role-specific fn

For each call site identified in step 5.1, replace
`narrow_int_cast(tp)` with the role-appropriate function.

Block-tail callers in `emit.rs` use `block_tail_narrow_for_int`.
Parameter callers in `calls.rs` use `param_narrow_for_int`.

### Step 5.4 — Remove the deprecated wrapper

Once all call sites use the role-specific fns, delete the
deprecated `narrow_int_cast` wrapper.

### Step 5.5 — Add structural test

```rust
#[test]
fn p12_05_narrow_int_cast_is_split() {
    let src = std::fs::read_to_string(project_root().join("src/generation/mod.rs"))
        .expect("read mod.rs");
    assert!(
        !src.contains("fn narrow_int_cast"),
        "narrow_int_cast should be split into block_tail_narrow_for_int + param_narrow_for_int"
    );
    assert!(
        src.contains("fn block_tail_narrow_for_int"),
        "block_tail_narrow_for_int must exist after plan-12 phase 05"
    );
    assert!(
        src.contains("fn param_narrow_for_int"),
        "param_narrow_for_int must exist after plan-12 phase 05"
    );
}
```

### Step 5.6 — Run acceptance gate

```bash
scripts/p09_fast_gate.sh   # byte-identical (the bodies are
                           # identical so emission unchanged)
cargo test --release --test issues 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 \
    | grep "native result"  # 95/95
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test threading_chars 2>&1 | tail -3
cargo test --release --test codegen_emitter 2>&1 | tail -3
cargo fmt --all -- --check
cargo clippy --tests --release -- -D warnings
```

Zero-regression rule applies.  Expected: zero behavioural
change since the bodies are identical.

## Acceptance

```bash
cargo test --release --test codegen_emitter::p12_05_narrow_int_cast_is_split
# + full regression sweep
```

Each call site uses the role-specific fn; `narrow_int_cast` no
longer exists.

## Risks

| Risk | Mitigation |
|---|---|
| A call site's role is genuinely ambiguous | Document in § Findings; default to whichever role's body matches the call's intent.  If still unclear, leave both bodies identical (no behavioural change) until a future fix forces divergence. |
| Splitting reveals a third role (e.g. struct-field narrowing) | Stop and document; phase 05 may need sub-phasing. |

## Future evolution

After @PLAN12 phase 05 lands, future P-issue fixes can change
ONE role without affecting the other.  E.g. if a new P-issue
needs the param-narrowing role to handle `i64` differently than
the block-tail role does, only `param_narrow_for_int` changes.

This was the original @PLAN09 phase 02 motivation.  Plan-09
delivered without it; @PLAN12 cleans up the residual complexity.

## Findings

_(populate during step 5.1 — per call site role)_

## Implementation notes

_(append per non-obvious decision — particularly any call site
where the role is ambiguous)_
