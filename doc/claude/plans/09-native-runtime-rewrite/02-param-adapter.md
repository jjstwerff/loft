# Phase 02 — Parameter adaptation

**Status:** OPEN

**Kind:** Simplification — **prerequisite for phase 05 (P200 write
side) and phase 08 (P200 read side).**  Without this, the dual-role
`narrow_int_cast` keeps biting; previous direct fixes failed
because they tried to surgery the cast inside its tangled call
sites.  Pulling param adaptation into per-type adapters removes the
shared cast-decision code and lets each Op's emitter do the right
thing locally.

**Depends on:** Phase 00.

## What's tangled today

`src/generation/calls.rs:200-324` is the per-parameter substitution
matrix for `#rust"…@arg…"` templates.  ~125 lines of stacked `if
matches!(…) { … continue; }` for ~10 special cases:

| Adapter case | Trigger | Emission |
|---|---|---|
| EnumNull | enum param + `Value::Null` | `(255u8)` |
| RefNull | ref/vector/sorted/hash/index/struct-enum param + `Value::Null` | `(DbRef { store_nr: u16::MAX, rec: 0, pos: 8 })` |
| CharFromInt | char param + `Value::Int` | `(char::from_u32(N_u32).unwrap_or('\0'))` |
| CharFromVar | char param + char-typed Var | `(ops::to_char(var))` |
| CharFromCall | char param + char-returning Call | `(ops::to_char(call))` |
| TextFromCall | text param + text-returning Call | `(&*(call))` |
| IntFromChar | int param + char value | `expr as u32 as i32` |
| FnRefTupleElem | int param + fn-ref tuple element | `(i64::from((expr).0))` |
| U32FieldOffset | template uses `u32::from(@name)` | replace with `(expr) as u32` |
| NarrowInt | param type is u8/u16/i8/i16 | suffix patch or `as <narrow>` cast |

Every condition re-queries the IR for the same value.

## Detailed steps with validation

### Step 2.1 — Extract `ParamAdapter` trait

**Action**: create `src/generation/ops/params.rs`:
```rust
use std::io::{self, Write};
use crate::data::{Value, Type};
use super::EmitCtx;

pub trait ParamAdapter: Send + Sync {
    /// True if this adapter handles the (param_ty, arg, arg_ty) triple.
    fn applies(&self, param_ty: &Type, arg: &Value, arg_ty: &Type) -> bool;
    /// Emit the substitution string for the @argname placeholder.
    fn emit(
        &self,
        ctx: &mut EmitCtx<'_, dyn Write>,
        param_ty: &Type,
        arg: &Value,
        arg_ty: &Type,
    ) -> io::Result<String>;
}

pub fn adapt_param(
    ctx: &mut EmitCtx<'_, dyn Write>,
    param_ty: &Type,
    arg: &Value,
    arg_ty: &Type,
) -> io::Result<String> {
    for adapter in ADAPTERS.iter() {
        if adapter.applies(param_ty, arg, arg_ty) {
            return adapter.emit(ctx, param_ty, arg, arg_ty);
        }
    }
    DefaultAdapter.emit(ctx, param_ty, arg, arg_ty)
}

// ADAPTERS: declared in dependency order.  Earlier entries take priority.
static ADAPTERS: &[&dyn ParamAdapter] = &[
    &EnumNullAdapter,
    &RefNullAdapter,
    &CharFromIntAdapter,
    &CharFromVarAdapter,
    &CharFromCallAdapter,
    &TextFromCallAdapter,
    &IntFromCharAdapter,
    &FnRefTupleElemAdapter,
    &U32FieldOffsetAdapter,
    &NarrowIntAdapter,
];
```

**Validation**:
```bash
cargo build --release  # compiles, no behavioural test yet
```

### Step 2.2 — Extract one adapter (EnumNull) end-to-end

**Action**: implement `EnumNullAdapter`.  Replace the corresponding
arm in `calls.rs:200-324` with a call to `adapt_param`.  Add an
adapter unit test:

```rust
// tests/codegen_emitter.rs
#[test]
fn enum_null_adapter_emits_255() {
    let result = run_adapter("EnumNull",
        Type::Enum(0, false, vec![]),
        Value::Null,
        Type::Null);
    assert_eq!(result, "(255u8)");
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::enum_null_adapter_emits_255
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test codegen_emitter::baseline_emission_unchanged
# Baseline diff MUST stay byte-identical — the adapter is just a
# refactor of the same string output.
```

### Step 2.3 — Extract remaining adapters one by one

**Action**: repeat step 2.2 for each adapter in the table above.
One commit per adapter.  Each adds:
- The adapter impl in `params.rs`
- A unit test pinning its emission for representative inputs
- The corresponding arm removed from `calls.rs:200-324`

**Validation per adapter**:
- Unit test green
- Baseline emission byte-identical
- Full suite green (`issues` + `threading` + `native`)

### Step 2.4 — Replace `calls.rs:200-324` body with single `adapt_param` call

**Action**: after every adapter is extracted, the loop body in
`output_call_template` collapses to:
```rust
for (a_nr, a) in def_fn.attributes.iter().enumerate() {
    let name = "@".to_string() + &a.name;
    if a_nr < vals.len() {
        let arg_ty = self.value_type(&vals[a_nr]);
        let with = adapt_param(ctx, &a.typedef, &vals[a_nr], &arg_ty)?;
        res = res.replace(&name, &with);
    }
}
```

**Validation**:
```bash
cargo test --release --test codegen_emitter::baseline_emission_unchanged
```

If diff fails, an adapter handles a case slightly differently than
the original arm — go back and fix.

### Step 2.5 — Document adapter ordering invariants

**Action**: add a doc comment above `ADAPTERS` explaining why the
order matters.  Specific invariants to capture:

- `CharFromCallAdapter` must precede `IntFromCharAdapter` (a char-
  returning call assigned to an int param: char-from-call wins
  because the call itself is the char source).
- `EnumNullAdapter` must precede `RefNullAdapter` (struct-enum
  shape `Type::Enum(_, true, _)` matches both; enum-null is
  "more specific").

Add an adapter-ordering test:
```rust
#[test]
fn adapter_order_invariants() {
    // CharFromCallAdapter wins over IntFromCharAdapter
    // when both apply.
    let order_correct = ADAPTER_NAMES.iter().position(|n| *n == "CharFromCall")
        < ADAPTER_NAMES.iter().position(|n| *n == "IntFromChar");
    assert!(order_correct, "CharFromCall must precede IntFromChar");
}
```

**Validation**: tests pass.

### Step 2.6 — Confirm dual-role `narrow_int_cast` is now bypassable

**Action**: add a test that documents the structural property
phase 05 will rely on:
```rust
#[test]
fn param_adaptation_does_not_route_through_narrow_int_cast() {
    // Compile a program that exercises NarrowIntAdapter (a u16
    // field write).  Inspect the generated code: it must contain
    // the narrow cast inline and NOT a call/include of the shared
    // narrow_int_cast helper.
    let src = compile_to_rust("tests/scripts/repro_narrow_int.loft");
    assert!(!src.contains("narrow_int_cast("),
        "param-adapter path still routes through narrow_int_cast — \
         phase 05 (P200) cannot proceed");
}
```

**Validation**: this test passes — proving the prerequisite for P200's fix is in place.

### Step 2.7 — Extract shared `narrow_for_int` helper

**Action**: `narrow_int_cast` in `src/generation/emit.rs` is called
from five block-tail-expression sites (`emit.rs:157, 173, 846, 880,
892`).  After step 2.6, the param-adaptation role is gone, but the
function still does double duty if any future change needs to
share width logic.

Extract the core decision — given a width and signedness, produce
the cast string — into a free helper in
`src/generation/ops/params.rs`:

```rust
/// Single source of truth for "given a width (8/16/32/64) and
/// signedness, emit the matching narrow cast for an i64-shaped
/// expression."  Used by NarrowIntAdapter (param adaptation) AND
/// by emit.rs's block-tail-expression coercion.  Retires the
/// dual-role state of narrow_int_cast.
pub fn narrow_for_int(width: u8, signed: bool, src: &str) -> String {
    let prim = if signed { format!("i{width}") } else { format!("u{width}") };
    format!("({src}) as {prim}")
}
```

Route both `NarrowIntAdapter::emit()` and the five
`emit.rs::narrow_int_cast` call sites through `narrow_for_int`.
The block-tail sites previously called `narrow_int_cast(returned)`
which combined width-detection + cast emission; split into:
1. `int_width_for(value)` (already added in phase 02 for the adapter)
2. `narrow_for_int(width, signed, src)` (the helper above)

After this step, `narrow_int_cast` either becomes a thin wrapper
that calls both helpers, or is deleted in favour of direct calls
at each site.

**Validation**:
```rust
#[test]
fn narrow_for_int_is_single_source_of_truth() {
    // Adapter and block-tail both produce the same cast for the
    // same (width, signed, expr).
    assert_eq!(narrow_for_int(16, false, "x"), "(x) as u16");
    assert_eq!(narrow_for_int(32, true, "y"), "(y) as i32");

    // No file under src/generation/ implements its own narrow
    // logic outside of params.rs::narrow_for_int.
    let pat = "_i32";  // legacy ad-hoc cast suffix
    let calls = std::fs::read_to_string("src/generation/calls.rs").unwrap();
    let emit = std::fs::read_to_string("src/generation/emit.rs").unwrap();
    let dispatch = std::fs::read_to_string("src/generation/dispatch.rs").unwrap();
    // Each file should reference narrow_for_int (via use) OR
    // contain no ad-hoc narrow casts; not both diverging.
    // Mechanical check: grep narrow casts; assert ≤ N occurrences
    // (where N is the count after consolidation).
    // Refine N during the actual extraction.
}
```

```bash
cargo test --release --test codegen_emitter::narrow_for_int_is_single_source_of_truth
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test codegen_emitter::baseline_emission_unchanged
# Baseline diff stays byte-identical — the helper is just a refactor.
```

This step retires the dual-role state by **sharing the helper**,
not by extending the emitter dispatch.  Block emission stays
direct; the cast logic is the one shared bit.

## Acceptance for phase 02 overall

```bash
cargo test --release --test codegen_emitter
cargo test --release --test issues 2>&1 | tail -3            # 540/540
cargo test --release --test threading 2>&1 | tail -3
cargo test --release --test native -- --test-threads=1 2>&1 | grep "native result"
# Specifically:
cargo test --release --test codegen_emitter::param_adaptation_does_not_route_through_narrow_int_cast
```

Net diff target: ~125 lines deleted from `calls.rs`, ~150 lines
added across `src/generation/ops/params.rs`.

## Commit shape

12-14 commits (one trait + ten adapters + ordering test +
acceptance test + narrow_for_int extraction); ships as one PR.

## Problems encountered

_(append per problem)_

## Implementation notes

_(append per non-obvious decision — likely: order-dependence of
adapter checks documented above; whether `EmitCtx` needs to expose
`value_type(value) -> Type` as a helper)_
