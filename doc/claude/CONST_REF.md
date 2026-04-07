
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Const Reference Optimization — Skip Deep Copy for Const Parameters

## Problem

When a struct is passed as a `const` parameter to a function, the interpreter
deep-copies the entire struct via `OpCopyRecord` + `store.copy_block()`.  The
`const` keyword guarantees the parameter is never mutated, so the copy is
unnecessary — a reference (12-byte DbRef) would suffice.

**Example:**
```loft
fn rect_mvp(const proj: math::Mat4, bx: float, by: float, bw: float, bh: float) -> Mat4 {
  // proj is 128 bytes (16 × f64).  Copied on every call despite being const.
  model = Mat4 { ... };
  math::mat4_mul(proj, model)
}
```

Called 60 times per frame in Breakout → 60 × 128 bytes = **7.7 KB/frame** of
unnecessary copies.  In the renderer's PBR pass, per-node Mat4 uniforms
add more.

## Current behaviour

### Parse time

The `const` keyword on a user function parameter:
1. Sets `Variable.const_param = true` in the variable table
2. `is_const_param()` check in `assign_text`, `create_vector`, and
   `parse_assign_op` emits a compile error on mutation attempts
3. **No effect on code generation** — `const` is purely a mutation guard

### Code generation

For a `const Reference` parameter (e.g., `const Mat4`):
1. Caller pushes the struct's DbRef onto the stack (12 bytes)
2. Callee's `generate_set` → `gen_set_first_at_tos` → `gen_set_first_ref_copy`
3. `gen_set_first_ref_copy` emits:
   - `OpConvRefFromNull` — allocate a NEW store
   - `OpDatabase` — create a record in the new store
   - `OpCopyRecord` — deep-copy ALL fields from source to new record
4. The local variable now owns an independent copy

### Runtime cost

`OpCopyRecord` calls `store.copy_block()` which copies `struct_size` bytes.
For `Mat4` (16 × 8 bytes): 128 bytes per copy.  For a scene `Node` with
transform + material references: potentially hundreds of bytes.

## Proposed fix

### Principle

When a `const Reference` parameter is first assigned in the callee, skip the
deep copy.  Instead, create a `DbRef` that points to the CALLER's store.  The
callee borrows the data read-only.

### Safety argument

- `const` prevents all mutation at compile time (parser enforces this)
- The caller's scope outlives the callee (stack discipline)
- No aliasing hazard: the callee cannot write through the reference
- If the caller mutates the struct during the callee's execution (via `&`
  parameter to another function), the callee's `const` view sees the mutation.
  This is correct — `const` means "this function won't modify it", not
  "it won't change".  Same as C++ const reference semantics.

### Implementation

**File:** `src/state/codegen.rs`, function `gen_set_first_at_tos` (~line 935)

Current code for `Type::Reference` + `Value::Null` (initial assignment):
```rust
} else if matches!(stack.function.tp(v), Type::Reference(_, _))
    && *value == Value::Null
{
    self.gen_set_first_ref_null(stack, v);
}
```

For first assignment from a caller's reference, `gen_set_first_ref_copy` is
called.  **Add a check:** if the variable is `const_param`, emit
`OpCreateStack` (which creates a DbRef pointing into the caller's stack)
instead of the full copy sequence.

```rust
// In gen_set_first_at_tos, before the existing Reference + OpCopyRecord path:
if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
    && stack.function.is_const_param(v)
    && let Value::Var(src) = value
{
    // Const parameter: borrow caller's reference, no deep copy.
    let src_pos = stack.position - stack.function.stack(*src);
    stack.add_op("OpVarRef", self);
    self.code_add(src_pos);
    // The local variable now holds a DbRef to the caller's data.
    return;
}
```

### What changes

| Scenario | Before | After |
|---|---|---|
| `const Reference` param, first use | OpConvRefFromNull + OpDatabase + OpCopyRecord | OpVarRef (12 bytes, no copy) |
| `const Reference` param, field read | Read from local copy | Read from caller's store |
| Mutable parameter (no `const`) | Full copy | Full copy (unchanged) |
| `&Reference` parameter | OpCreateStack | OpCreateStack (unchanged) |
| `const` scalar (int, float) | Stack copy | Stack copy (unchanged) |
| `const vector<T>` param | Already DbRef | Already DbRef (unchanged) |

### Files to modify

| File | Change |
|---|---|
| `src/state/codegen.rs` | Add const-param check in `gen_set_first_at_tos` before ref copy |
| `src/variables/mod.rs` | Ensure `is_const_param()` is accessible from codegen |

### What NOT to change

- Parser: `const` enforcement is already correct
- Runtime: `OpVarRef` already exists and does the right thing
- Native codegen: separate optimization path, not affected
- `&T` parameters: already use `OpCreateStack` (reference, not copy)

## Performance impact

### Breakout game (per frame at 60 fps)

| Function | Calls/frame | Bytes saved | Total |
|---|---|---|---|
| `rect_mvp(const proj: Mat4)` | ~60 | 128 bytes | 7.7 KB |
| `mat4_mul(const a: Mat4)` | ~60 | 128 bytes | 7.7 KB |
| **Total** | | | **~15 KB/frame** |

### Renderer PBR pass (per frame)

| Function | Calls/frame | Bytes saved |
|---|---|---|
| `render_frame(const scene, const camera)` | 1 | Scene + Camera structs |
| Per-node uniform setup | ~4 | Mat4 per node |

### Interpreter overhead reduction

- Eliminates `store.claim()` + `store.copy_block()` for every const struct param
- Reduces store allocation pressure (fewer records to track/free)
- Reduces GC-equivalent work (fewer stores to scan on scope exit)

## Verification

1. `cargo test` — all 44 tests pass
2. `make test-packages` — 16/16 pass
3. Breakout game runs correctly (const Mat4 params work)
4. Renderer demo (example 24) renders correctly
5. Test: function modifying a struct AFTER passing it as const to another
   function sees the mutation — verify correct aliasing semantics

## Future work

- **O2:** Extend to `const vector<T>` parameters that are indexed but never
  pushed to — skip the vector store allocation.
- **O3:** Auto-detect const-ness for non-annotated parameters that are
  never mutated within the function body (whole-program analysis).
- **O4:** Native codegen: emit `&T` instead of `T` for const params in
  generated Rust code.

## Related

- [PERFORMANCE.md](PERFORMANCE.md) — benchmark data and optimization plan
- [OPTIMISATIONS.md](OPTIMISATIONS.md) — planned interpreter optimizations
- [SLOTS.md](SLOTS.md) — stack slot assignment design
