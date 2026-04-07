
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Struct Passing, Copies, and Optimization Opportunities

## Loft parameter semantics

Loft passes ALL struct parameters by reference (shared DbRef).  There
is no implicit copy on function calls.  Mutation is the default:

```loft
fn modify(s: Point) { s.x = 99.0; }
fn main() {
  p = Point { x: 1.0, y: 2.0 };
  modify(p);
  // p.x is now 99.0 — caller's struct was mutated
}
```

The three parameter modes:

| Syntax | Semantics | Store locked? |
|---|---|---|
| `param: T` | Mutable reference — callee can mutate caller's data | No |
| `param: &T` | Mutable reference — same as above, explicit | No |
| `param: const T` | Read-only reference — store locked, writes panic | Yes |

---

## Where copies actually happen

Copies are NOT on parameter passing.  They happen on **first local
variable assignment** and **return values**.

### Copy landscape

| Situation | What happens | Cost |
|---|---|---|
| Parameter passing | DbRef shared (12 bytes) | **Zero** |
| Vector element `v[i]` | DbRef pointer arithmetic | **Zero** |
| For-loop iteration | DbRef per element | **Zero** |
| Local reassignment `x = y` | DbRef overwrite | **Zero** |
| **First assignment `x = func()`** | OpCopyRecord deep copy | **Expensive** |
| **First assignment `x = y`** (same struct type) | OpCopyRecord deep copy | **Expensive** |
| **Return values** | copy_block (byte copy) | **Moderate** |
| Const lock check | Bool assert per write | Negligible |

### When OpCopyRecord fires

Only three cases in `gen_set_first_at_tos` (codegen.rs):

1. **`x = func_returning_struct()`** — function return assigned to new
   local variable.  Emits OpConvRefFromNull + OpDatabase + OpCopyRecord.
   Deep copies all fields including nested vectors, text, sub-structs.

2. **`x = y`** where both are same struct type and x is uninitialized —
   same deep copy to give x its own independent store.

3. **Tuple destructuring** `(a, b) = expr` where an element is a
   Reference — deep copy for the extracted element.

### What OpCopyRecord costs

Runtime at `state/io.rs:932`:
```
copy_block(&data, &to, size)     — raw byte copy of struct fields
copy_claims(&data, &to, tp)      — deep copy of nested structures
```

For `Mat4` (16 × f64 + vector wrapper): ~128 bytes + vector record.
For `Scene` with meshes/materials/nodes: hundreds of bytes + all vectors.

### Return value copy (latent issue)

`state/mod.rs:1032` copies return values with `copy_block` only — no
`copy_claims`.  This is a shallow byte copy.  If a returned struct
contains owned nested references (vectors, text), the returned DbRef
shares them with the callee's about-to-be-freed store.  **Potential
use-after-free for complex return types.**

---

## Optimization 1: Move semantics for return values

### Problem

`br_mvp = rect_mvp(proj, x, y, w, h)` — called 60×/frame in Breakout.
Each call: callee constructs Mat4, returns it, caller OpCopyRecord deep
copies it into `br_mvp`'s store.  The callee's original is immediately
freed.  The copy is wasted — the data could transfer ownership.

### Fix: return slot pre-allocation (destination passing)

The caller pre-allocates the destination store and passes a DbRef to the
callee.  The callee writes directly into it.  No copy on return.

```
Before:                              After:
  callee: build Mat4 in local store    callee: build Mat4 in caller's store
  return: copy_block to caller         return: nothing (already there)
  caller: OpCopyRecord to br_mvp      caller: nothing (already in br_mvp)
```

This pattern already exists for text-returning functions
(`try_text_dest_pass` in codegen.rs).  Extending it to struct returns
is the natural next step.

### Implementation

**File:** `src/state/codegen.rs`

1. When generating a function call whose return type is `Reference`:
   - If the result is assigned to a local variable (`x = func(...)`),
     pass `x`'s store DbRef as a hidden first parameter
   - The callee writes into that store instead of its own local
   - Return is a no-op (data already in the right place)

2. Requires the callee to be aware of the destination.  Two options:
   - **Implicit:** codegen detects struct construction and redirects writes
   - **Explicit:** new `__dest` hidden parameter (like text_return)

### Impact

| Function | Calls/frame | Bytes saved per call |
|---|---|---|
| `rect_mvp()` | 60 | ~128 bytes + vector overhead |
| `mat4_mul()` | 60 | ~128 bytes |
| `mat4_perspective()` | 1 | ~128 bytes |
| `mat4_look_at()` | 1 | ~128 bytes |

**~15 KB/frame** eliminated in Breakout.  Proportionally more in the
renderer (PBR pass constructs Mat4 per node).

---

## Optimization 2: Last-use move (elide copy when source dies)

### Problem

```loft
a = Point { x: 1.0, y: 2.0 };
b = a;       // OpCopyRecord — deep copy
// a is never used again
```

The copy is unnecessary — `a`'s store could be transferred to `b`.

### Fix: last-use analysis

If `x = y` and `y` is never read again after this point (last use),
transfer `y`'s DbRef to `x` and null out `y`.  No copy needed.

The variable liveness analysis in `src/variables/` already tracks
`first_def` and `last_use`.  If `last_use(y) == current_statement`,
it's safe to move.

### Implementation

**File:** `src/state/codegen.rs`, in `gen_set_first_at_tos`

Before emitting OpCopyRecord for `x = y`:
```rust
if let Value::Var(src) = value
    && stack.function.last_use(*src) == current_def_position
{
    // Move: transfer src's DbRef to x, no copy
    let src_pos = stack.position - stack.function.stack(*src);
    stack.add_op("OpVarRef", self);
    self.code_add(src_pos);
    // Mark src as moved — OpFreeRef will skip it
    return;
}
```

### Impact

Eliminates copies for temporary struct results that are immediately
assigned and never reused.  Common in builder patterns:

```loft
m = mat4_translate(1.0, 2.0, 3.0);      // result → m (move, no copy)
mvp = mat4_mul(proj, mat4_mul(view, m)); // inner result → temp (move)
```

---

## Optimization 3: Auto-const inference (safety, not performance)

### Purpose

Not a performance optimization (parameters aren't copied).  Instead:
auto-lock stores for provably unwritten parameters to catch accidental
mutation bugs at runtime.

### When to auto-lock

A struct parameter can be auto-locked when:
- Never directly written (`param.field = x`)
- Never appended to (`param.vec += [x]`)
- Never passed as `&T` to another function
- **Never passed as plain `T` to a non-const function** (conservative —
  callee might mutate through the shared reference)

### Implementation

1. Add `auto_const: bool` to Variable
2. Run `find_written_vars()` at end of first pass
3. Add escape analysis: check if param is passed to any function call
   where the receiving parameter is not `const`
4. Lock store at function entry for auto-const params

### Compiler warning

When inference succeeds:
```
Warning: parameter 's' is never mutated — consider adding 'const'
```

---

## Test cases

### Test 1: mutation through plain parameter (current behavior, correct)

```loft
struct S { x: integer not null }
fn modify(s: S) { s.x = 99; }
fn main() {
  p = S { x: 1 };
  modify(p);
  assert(p.x == 99, "mutation visible to caller");
}
```

### Test 2: const parameter locks store

```loft
struct S { x: integer not null }
fn read(s: const S) -> integer { s.x }
fn main() {
  p = S { x: 1 };
  assert(read(p) == 1, "const read works");
}
```

### Test 3: const prevents mutation via &T (runtime panic)

```loft
struct S { x: integer not null }
fn mutate_ref(m: &S) { m.x = 99; }
fn bad(s: const S) { mutate_ref(s); }
fn main() { bad(S { x: 1 }); }
// Panics: "Write to locked store"
```

### Test 4: escape to non-const blocks auto-lock

```loft
struct S { x: integer not null }
fn helper(s: S) { s.x = 42; }
fn caller(s: S) {
  helper(s);  // s escapes to mutable function — cannot auto-lock
}
```

### Test 5: return value copy (current behavior)

```loft
struct Point { x: float not null, y: float not null }
fn make() -> Point { Point { x: 1.0, y: 2.0 } }
fn main() {
  p = make();       // OpCopyRecord fires here
  q = p;            // OpCopyRecord fires here
  q.x = 99.0;
  assert(p.x == 1.0, "p isolated from q after copy");
}
```

### Test 6: move optimization target

```loft
fn make() -> Point { Point { x: 1.0, y: 2.0 } }
fn main() {
  p = make();       // Could be a move (no copy) if dest-passing works
  println("{p.x}");
}
```

---

## Priority order

| # | Optimization | Impact | Effort | Risk |
|---|---|---|---|---|
| 1 | Return slot / destination passing | ~15 KB/frame in games | M | Low — text_return already does this |
| 2 | Last-use move for `x = y` | Eliminates temp copies | S | Low — liveness data available |
| 3 | Auto-const inference | Safety, not perf | M | Medium — needs escape analysis |

---

## Related

- [PERFORMANCE.md](PERFORMANCE.md) — benchmark data and optimization plan
- [OPTIMISATIONS.md](OPTIMISATIONS.md) — planned interpreter optimizations
- [SLOTS.md](SLOTS.md) — stack slot assignment design
