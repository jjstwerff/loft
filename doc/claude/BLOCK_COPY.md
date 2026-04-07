
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Block Copy Efficiency: Analysis and Recommendations

## What's actually expensive

Block copy has two phases:

| Phase | Function | Cost | What it does |
|---|---|---|---|
| 1. `copy_block` | `structures.rs:777` | O(struct_size) memcpy | Raw byte copy of struct fields |
| 2. `copy_claims` | `allocation.rs:642` | O(total_owned_data) + allocations | Deep copy of ALL owned sub-structures |

Phase 1 is cheap — typically 4-128 bytes of memcpy.

Phase 2 is the real cost.  For each field, `copy_claims` recursively:
- **Text fields:** re-interns the string (allocate store record + memcpy)
- **Vectors:** allocates new vector record, copies header + all elements,
  then recursively copies each element's owned data
- **Nested structs:** recurses into each struct field
- **Arrays/Hash/Index:** O(n) allocations, each with recursive traversal

A `Mat4` with a `vector<float>` costs: 1 store allocation + vector
allocation + 16 floats copied.  A `Scene` with meshes/materials/nodes:
dozens of allocations, hundreds of bytes.

## Where deep copies happen today

Deep copies (OpCopyRecord) fire in exactly three codegen paths, all in
`gen_set_first_at_tos` (codegen.rs:931-983):

```
x = func()      →  gen_set_first_ref_copy      →  OpCopyRecord
x = y            →  gen_set_first_ref_var_copy  →  OpCopyRecord
(a, b) = expr    →  gen_set_first_ref_tuple_copy →  OpCopyRecord
```

Each emits: `OpConvRefFromNull` → `OpDatabase` → `OpCopyRecord`.

Return values themselves are cheap (12-byte DbRef shallow copy in
`copy_result`).  The deep copy only fires at first assignment.

## Optimization candidates

### O-B1. Last-use move — **IMPLEMENT THIS**

**Pattern:**
```loft
temp = compute();
result = temp;     // temp never used again → move DbRef, skip copy
```

**Detection:** At the `x = y` codegen site (`gen_set_first_ref_var_copy`),
check `stack.function.last_use(src) <= stack.function.first_def(v)`.
If true, the source variable is never read after this assignment.
Transfer the DbRef instead of deep copying.

**Implementation:**
```rust
// In gen_set_first_ref_var_copy, before emitting OpCopyRecord:
if stack.function.last_use(src) <= stack.function.first_def(v) {
    // Move: just copy the DbRef (12 bytes), skip deep copy.
    // Mark src as moved so OpFreeRef skips it.
    stack.add_op("OpVarRef", self);
    let src_pos = stack.position - stack.function.stack(src);
    self.code_add(src_pos);
    stack.function.set_skip_free(src);
    return;
}
```

**Impact:** Eliminates ALL deep copies for temporary-to-final patterns.
Common in math-heavy code:
```loft
m = mat4_identity();        // allocate + build
mvp = mat4_mul(proj, m);    // m's last use → move, not copy
```

**Complexity:** S — 5-10 lines in one function.  No new opcodes,
no ABI changes.  Uses existing `last_use`, `first_def`, `skip_free`.

**Risk:** Low.  The liveness analysis is already computed and used
for slot assignment.  `skip_free` already exists for other purposes.
Must verify that `last_use` accounts for implicit frees (OpFreeRef)
— if it does, the check is safe.

---

### O-B2. Last-use move for function returns — **IMPLEMENT THIS**

**Pattern:**
```loft
result = make_point();   // function returns struct, immediately assigned
// make_point's return value is never aliased
```

Currently: function returns DbRef (shallow), then `gen_set_first_ref_copy`
deep copies it.  The return value's store is freed immediately after.

**Detection:** The RHS is `Value::Call(OpCopyRecord, args)` where args[0]
is a function call.  The intermediate DbRef from the function call is
always a temporary — it has no variable, so it's always "last use".

**Implementation:** In `gen_set_first_ref_copy`, when the inner call
is a user function (not OpCopyRecord itself), the return value is a
one-shot temporary.  Skip the OpCopyRecord and directly adopt the
returned DbRef.

However, there's a subtlety: the returned DbRef points into the
callee's store, which may be freed.  Need to verify store lifetime.
If the callee's store outlives the return (it should — stores are
not freed until explicit OpFreeRef), this is safe.

**Impact:** Eliminates deep copies for `x = func()` patterns.

**Complexity:** M — needs careful store lifetime analysis.

---

### O-B3. Destination passing for struct returns — **DEFER**

**Pattern:** Extend the text `RefVar(Text)` destination-passing
mechanism to struct-returning functions.  Caller pre-allocates the
destination store, callee writes directly into it.

**Why defer:** Requires ABI changes (hidden parameter), callee
rewriting to use the destination store for all field writes, and
interaction with existing OpCopyRecord codegen.  The last-use move
(O-B1) handles the most common cases with much less complexity.

**When to revisit:** If profiling shows that `x = func()` copies
remain a bottleneck after O-B1/O-B2, destination passing is the
next step.

---

### O-B4. Shallow copy for immutable borrows — **DEFER**

**Pattern:** `x = y` where x is never mutated — share the DbRef
instead of deep copying.

**Why defer:** Requires copy-on-write or reference counting to
prevent double-free and aliasing bugs.  The auto-const analysis
(CONST_REF.md O3) would need to run first to identify which
variables are immutable.  High complexity for moderate gain.

---

## Status after O-B1

**O-B1 is implemented** (codegen.rs `gen_set_first_ref_var_copy`).
When `x = y` and y has `uses == 1` (only read here), not an argument,
not captured: emits OpVarRef + skip_free instead of the full deep copy.

### Remaining deep copy sites

| Site | Codegen function | Pattern | Frequency |
|---|---|---|---|
| 1 | `gen_set_first_ref_copy` | `x = func()` | **Very common** — every struct-returning call |
| 2 | `gen_set_first_ref_var_copy` | `x = y` (uses > 1) | Rare — O-B1 handles uses == 1 |
| 3 | `gen_set_first_ref_tuple_copy` | `(a, b) = expr` | Rare — tuple destructuring |

**Site 1 is the dominant remaining cost.** Every `m = mat4_mul(a, b)`
allocates a fresh store, deep copies, and **leaks the callee's store**.

### Store leak on struct returns

When a function returns a struct, the callee's store is kept alive
(scopes.rs `in_ret` check skips OpFreeRef).  After the caller deep
copies from it, nobody frees it.  This is a latent store leak that
grows linearly with struct-returning calls.

## Recommendation

### O-B2: Return store adoption — **IMPLEMENT NEXT**

For `x = func()`, the source is always a temporary on the eval stack
(no variable holds it).  Instead of allocating a NEW store + deep
copy, adopt the returned DbRef directly.  This fixes BOTH the copy
cost AND the store leak.

**Safety concern:** The returned DbRef might point to a parameter's
store (e.g. `fn identity(p: Point) -> Point { p }`).  Adopting it
would cause the caller to free a shared store.

**Safe implementation:** After the function call + OpCopyRecord runs,
free the source store.  The deep copy is still performed, but the
leak is fixed.  Then separately optimize the copy away for provably
fresh returns (callee constructs a new struct, never returns a param).

**Detecting fresh returns:** A function whose return type has empty
dependencies (`dep.is_empty()`) and whose return expression is a
struct constructor or a call to another struct-returning function
(not a Var pointing to a parameter) is safe to adopt.

**Complexity:** M — two-phase: (1) fix the leak (S), (2) skip copy
for fresh returns (M, needs callee analysis).

### O-B3 and O-B4 — **DEFER**

Destination passing (O-B3) is the clean long-term solution but
requires ABI changes.  Shallow copy for immutables (O-B4) needs
copy-on-write.  Both deferred until the simpler O-B2 is in place.

### Expected savings after O-B1 + O-B2

| Pattern | Current cost | After O-B1+O-B2 |
|---|---|---|
| `m = mat4_identity(); mvp = m` | Deep copy | 12B DbRef move (O-B1) |
| `result = temp_struct` where temp dies | Deep copy | 12B DbRef move (O-B1) |
| `x = func()` (func builds new struct) | Deep copy + store leak | 12B DbRef adopt (O-B2) |
| `x = func()` (func returns param) | Deep copy + store leak | Deep copy, no leak (O-B2 phase 1) |
| Loop: `acc = transform(acc)` | Deep copy per iter | 12B move per iter (O-B1) |

For Breakout (60 rect_mvp/frame): ~15 KB deep copies + 60 leaked
stores/frame → 720B moves + 0 leaks.

---

## Implementation status

| Optimisation | Status | Issue |
|---|---|---|
| O-B1: last-use move `x = y` | **Done** | — |
| O-B2: adoption for no-ref-param functions | **Done** (codegen branch for `n_*` functions) | — |
| O-B2: deep copy for ref-param functions | **Partially done** (`gen_set_first_ref_call_copy`) | P116 |
| Store leak fix (callee store after copy) | **Partial** — O-B2 adoption fixes no-ref-param case | P117 |
| Threading regression | **Blocked** — needs investigation | P118 |

### Known issues found during optimisation

- **P116**: `x = func(s)` where func has Reference params aliases
  the store.  Codegen branch added but needs regression testing.
- **P117**: Store leak for struct-returning functions.  Fixed for
  no-ref-param functions by O-B2 adoption.  Remaining: ref-param case.
- **P118**: `22-threading.loft` panics "Incomplete record" after
  P64/P66 checked arithmetic changes.  Not yet diagnosed.
