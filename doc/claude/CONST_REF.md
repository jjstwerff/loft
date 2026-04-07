
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

## Phase 2: Auto-const inference — no source changes needed

### The opportunity

Most struct parameters are never mutated but aren't marked `const`.
Every such parameter is deep-copied unnecessarily.  If the compiler
can infer that a parameter is never written, it can apply the same
optimization automatically.

**Examples that benefit immediately (no code changes):**
- `mat4_mul(a: Mat4, b: Mat4)` — both read-only → no copies
- `mesh_to_floats(m: Mesh)` — mesh is read-only → no copy
- `render_frame(scene: Scene, camera: Camera)` — read-only in render loop
- Any getter/accessor on structs

### The timing problem

The mutation analysis (`find_written_vars()` in `parser/mod.rs:2481`)
already identifies which variables are written.  But it runs AFTER
the second pass completes — by then OpCopyRecord has already been
emitted.

### Solution: use the two-pass design

```
First pass:   parse function body → type inference (no codegen)
              → run find_written_vars()
              → mark unwritten Reference params as auto_const
Second pass:  parse function body → codegen
              → gen_set_first_at_tos checks auto_const
              → skip OpCopyRecord for auto_const params
```

The first pass already walks the entire function body.  We just add
the mutation analysis at its end and store the result on the Variable.

### Implementation

#### Step 1: Add `auto_const` flag to Variable

**File:** `src/variables/mod.rs`

```rust
pub struct Variable {
    // ... existing fields ...
    pub auto_const: bool,  // inferred: param never written in function body
}
```

Default `false`.  Set to `true` at end of first pass for Reference
parameters that `find_written_vars()` did not include in its result set.

#### Step 2: Run mutation analysis at end of first pass

**File:** `src/parser/mod.rs`, in `parse_file()` or at the end of
each function's first-pass parsing.

```rust
if self.first_pass {
    let written = self.find_written_vars(&function_body);
    for param_nr in function_params {
        if matches!(self.vars.tp(param_nr), Type::Reference(_, _))
            && !written.contains(&param_nr)
            && !self.vars.is_argument_mut(param_nr)  // not &T
        {
            self.vars.set_auto_const(param_nr, true);
        }
    }
}
```

#### Step 3: Use auto_const in codegen

**File:** `src/state/codegen.rs`, in `gen_set_first_at_tos`

```rust
if let Type::Reference(d_nr, _) = stack.function.tp(v).clone()
    && (stack.function.is_const_param(v) || stack.function.is_auto_const(v))
    && let Value::Var(src) = value
{
    // Const or auto-const: borrow caller's reference, no deep copy.
    let src_pos = stack.position - stack.function.stack(*src);
    stack.add_op("OpVarRef", self);
    self.code_add(src_pos);
    return;
}
```

### What constitutes a write

`find_written_vars()` already correctly detects:

| Operation | Detected as write? |
|---|---|
| `param = value` | Yes |
| `param.field = value` | Yes (recursively finds `param`) |
| `param += [elem]` | Yes (vector append operators) |
| Passing as `&T` to another function | Yes (mutable reference) |
| Passing as `T` or `const T` | No (value/const — no mutation) |
| Reading `param.field` | No |
| Passing to `println("{param}")` | No |

### Scope

This optimization applies to ALL functions automatically — no `const`
keyword needed.  The explicit `const` keyword remains useful for:
- Documentation: makes intent clear to the reader
- Error prevention: catches accidental mutations at compile time
- Functions where mutation analysis is conservative (e.g., recursion)

### Safety

Same as Phase 1 — the caller's scope outlives the callee, and the
parameter is provably never written.  The analysis is conservative:
if `find_written_vars()` can't prove a variable is unwritten (e.g.,
passed to a function whose body hasn't been analyzed yet), it stays
in the written set and the copy is kept.

### Risks

- **False positives from `find_written_vars()`:** The analysis may
  over-report writes (conservative).  This is SAFE — it just means
  some optimizable cases are missed, never that a written variable
  is incorrectly skipped.
- **Cross-function analysis:** Currently per-function only.  A
  parameter passed to another function as `T` (not `&T`) is not
  considered written — this is correct because the callee gets a
  copy.
- **Recursion:** A function calling itself with a parameter doesn't
  constitute a write to the parameter.  This is correct.

### Performance estimate

| Codebase area | Functions affected | Copies eliminated |
|---|---|---|
| math.loft (`mat4_*`) | ~15 functions | 2-4 Mat4 copies each |
| mesh.loft (`mesh_to_*`) | ~5 functions | Mesh struct copies |
| scene.loft (accessors) | ~10 functions | Scene/Node/Material |
| render.loft (render loop) | 3 functions | Scene + Camera per frame |
| User game code | All struct-taking functions | Varies |

**Conservative estimate:** 50-80% of all struct parameter copies
eliminated without any source code changes.

## Future work

- **O4:** Native codegen: emit `&T` instead of `T` for auto-const
  params in generated Rust code.
- **O5:** Extend auto-const to vectors: skip store allocation for
  vectors that are indexed but never pushed to.

## Related

- [PERFORMANCE.md](PERFORMANCE.md) — benchmark data and optimization plan
- [OPTIMISATIONS.md](OPTIMISATIONS.md) — planned interpreter optimizations
- [SLOTS.md](SLOTS.md) — stack slot assignment design
