
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Const Inference — Lock Stores for Provably Unwritten Parameters

## Loft parameter semantics

Loft passes ALL struct parameters by reference (shared DbRef).  There
is no implicit copy.  This is by design — mutation is the default:

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

**There is no value-copy mode for structs.** All three modes share the
caller's store.  `const` adds a runtime lock that prevents writes.

## The optimization opportunity

Since there's no copying to eliminate, the optimization is different from
what was originally planned.  Instead:

**Auto-lock stores for parameters that are provably never written.**

When a function takes a struct parameter and never mutates it (no field
writes, no passing as `&T`, no vector appends), the store can be locked
automatically — giving the same protection as `const` without requiring
the keyword.

### Benefits

1. **Safety:** catches accidental mutations that the programmer didn't
   intend — the store lock makes them a runtime error instead of silent
   corruption of the caller's data.
2. **Documentation:** the compiler can emit warnings like "parameter 'p'
   is never mutated — consider adding `const`" to encourage explicit
   annotation.
3. **Future copy-on-write:** if loft ever adds value semantics for structs,
   auto-const parameters can skip the copy since they're provably unwritten.

### Non-benefits (corrected from earlier analysis)

- ~~Skip OpCopyRecord~~ — there IS no OpCopyRecord for plain parameters.
  Structs are already passed by reference.
- ~~Save 15 KB/frame~~ — no copies happen, so nothing to save.

## Auto-const inference

### The opportunity

Most struct parameters are never mutated but aren't marked `const`.
Without the lock, accidental mutations silently corrupt the caller's
data.  If the compiler infers that a parameter is never written, it
can auto-lock the store — catching bugs and enabling future
copy-on-write.

**Examples that benefit immediately:**
- `mat4_mul(a: Mat4, b: Mat4)` — both read-only → auto-locked
- `mesh_to_floats(m: Mesh)` — read-only → auto-locked
- `render_frame(scene: Scene, camera: Camera)` — read-only in render loop
- Any getter/accessor on structs

### Passing a param as plain `T` to another function

Since ALL struct parameters are mutable references, passing a param
to another function as plain `T` allows that function to mutate it.
`find_written_vars()` currently does NOT flag this as a write — it
treats plain `T` as value passing.

**This is incorrect for auto-const inference.** If `reader(s)` passes
`s` to `mutator(s)` which writes `s.x = 99`, then `s` IS effectively
written.  The analysis must be conservative:

- Passing as `T` to a function whose const-ness is UNKNOWN → treat
  as potentially written → do NOT auto-lock
- Passing as `T` to a function whose param is known `const` or
  inferred auto-const → safe, not a write
- Passing as `&T` → always a write (explicit intent)

This requires **cross-function analysis** or a conservative default
(only auto-lock when the param is never passed to ANY function).

### Implementation

#### Step 1: Add `auto_const` flag to Variable

**File:** `src/variables/mod.rs`

```rust
pub struct Variable {
    // ... existing fields ...
    pub auto_const: bool,  // inferred: param never written in function body
}
```

#### Step 2: Mutation analysis at end of first pass

**File:** `src/parser/mod.rs`

After first-pass parsing of each function, identify unwritten params:

```rust
if self.first_pass {
    let written = self.find_written_vars(&function_body);
    for param_nr in function_params {
        if matches!(self.vars.tp(param_nr), Type::Reference(_, _))
            && !written.contains(&param_nr)
            && !self.param_escapes_to_function(param_nr, &function_body)
        {
            self.vars.set_auto_const(param_nr, true);
        }
    }
}
```

`param_escapes_to_function` checks if the param is passed as a
non-const argument to any function call.  Conservative: if we can't
prove the callee won't mutate, assume it will.

#### Step 3: Lock stores for auto-const params at runtime

**File:** `src/state/codegen.rs` or runtime

When entering a function, emit store-lock for auto-const params:

```rust
if stack.function.is_auto_const(v) {
    // Lock the store to catch accidental mutations at runtime
    store.lock();
}
```

This provides the same protection as explicit `const` — writes
panic with "Write to locked store".

### What constitutes a write (conservative)

| Operation | Blocks auto-const? |
|---|---|
| `param.field = value` | Yes |
| `param += [elem]` | Yes |
| Passing as `&T` to a function | Yes |
| Passing as `T` to a non-const function | **Yes** (conservative) |
| Passing as `T` to a `const T` function | No |
| Reading `param.field` | No |
| Passing to `println("{param}")` | No |

### Compiler warning

When auto-const inference succeeds, emit a developer warning:

```
Warning: parameter 's' is never mutated — consider adding 'const'
```

This nudges programmers toward explicit annotation, which is better
documentation and enables the compiler to enforce at parse time.

## Test cases for safety verification

### Test 1: plain param mutation is visible to caller (current behavior)

```loft
struct S { x: integer not null }
fn modify(s: S) { s.x = 99; }
fn main() {
  p = S { x: 1 };
  modify(p);
  assert(p.x == 99, "mutation visible to caller");
}
```

This is CORRECT — loft passes by reference.

### Test 2: const param locks the store

```loft
struct S { x: integer not null }
fn read(s: const S) -> integer { s.x }
fn main() {
  p = S { x: 1 };
  assert(read(p) == 1, "const read works");
}
```

### Test 3: const param prevents writes (runtime panic)

```loft
struct S { x: integer not null }
fn mutate_ref(m: &S) { m.x = 99; }
fn bad(s: const S) { mutate_ref(s); }
fn main() {
  p = S { x: 1 };
  bad(p);  // should panic: "Write to locked store"
}
```

### Test 4: auto-const param passed to non-const function (must NOT lock)

```loft
struct S { x: integer not null }
fn helper(s: S) { s.x = 42; }  // mutates!
fn caller(s: S) {
  // s is never directly written in caller...
  helper(s);  // ...but helper mutates it
  // auto-const must NOT lock s because it escapes to a non-const function
}
```

### Test 5: auto-const param only read — safe to lock

```loft
struct S { x: integer not null }
fn getter(s: S) -> integer { s.x }
fn caller(s: S) -> integer {
  // s is never written, never escapes to a mutable function
  // auto-const CAN lock s safely
  getter(s) + 1  // getter is also auto-const
}
```

## Future work

- **Cross-function const propagation:** once auto-const is inferred
  for function A, callers of A can treat its params as const when
  deciding their own auto-const status.  Iterative fixpoint analysis.
- **Copy-on-write semantics:** if loft ever adds value semantics for
  structs, auto-const params can skip the copy entirely.
- **Native codegen:** emit `&T` for const/auto-const params in
  generated Rust code.

## Related

- [PERFORMANCE.md](PERFORMANCE.md) — benchmark data and optimization plan
- [OPTIMISATIONS.md](OPTIMISATIONS.md) — planned interpreter optimizations
- [SLOTS.md](SLOTS.md) — stack slot assignment design
