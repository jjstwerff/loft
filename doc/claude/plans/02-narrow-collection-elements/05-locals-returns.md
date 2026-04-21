<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 5 — Local variables, parameters, return types

**Status:** blocked by Phase 4.

**Goal:** `vector<i32>` (and narrow variants) works correctly
outside of struct fields — as local variables, function parameters,
and return types.

---

## Why this is (mostly) already done

Phase 0 put `forced_size` on `Type::Integer` itself.  That field
travels with the Type through `Box<Type>` in Vector, so it reaches:

- **Local variable declarations**: `x: vector<i32> = []` — the
  parser resolves `vector<i32>` via `parse_type_full` → `sub_type`,
  which produces `Type::Vector(Box<Type::Integer(..., Some(4))>,
  ...)`.  The local's type is stored in the variable table with
  forced_size intact.
- **Function parameters**: `fn f(v: vector<i32>)` — same resolution
  path; the parameter's Type carries the forced_size.
- **Return types**: `fn make() -> vector<i32>` — same.

So Phase 5 is **mostly a test phase** — verify that Phases 1-4
already covered these cases.  If a test fails, it surfaces a
specific gap to fix.

---

## Test matrix

```rust
// tests/issues.rs

#[test]
fn p184_vector_i32_local_var() {
    // Local variable.  Forced_size must survive local var table.
    code!(r#"
        fn test() {
            x: vector<i32> = [];
            x += [1 as i32, 2 as i32, 3 as i32];
            assert(x[0] == 1, "x[0] = {x[0]}");
            assert(x[1] == 2, "x[1] = {x[1]}");
            f = file("/tmp/p184_local.bin");
            f#format = LittleEndian;
            f += x;
            assert(f.size == 12, "expected 12 bytes, got {f.size}");
        }
    "#).result(Value::Null);
}

#[test]
fn p184_vector_i32_param() {
    // Function parameter.  The callee receives a narrow vector.
    code!(r#"
        fn sum(v: vector<i32>) -> integer {
            result = 0;
            for e in v { result += e }
            result
        }
        fn test() {
            x: vector<i32> = [];
            x += [10 as i32, 20 as i32, 30 as i32];
            assert(sum(x) == 60, "sum = {sum(x)}");
        }
    "#).result(Value::Null);
}

#[test]
fn p184_vector_i32_return() {
    // Function return.  Caller's receiver must see a narrow vector.
    code!(r#"
        fn make() -> vector<i32> {
            result: vector<i32> = [];
            result += [1 as i32, 2 as i32];
            result
        }
        fn test() {
            x = make();
            assert(x[0] == 1);
            assert(x[1] == 2);
            f = file("/tmp/p184_return.bin");
            f#format = LittleEndian;
            f += x;
            assert(f.size == 8, "expected 8 bytes, got {f.size}");
        }
    "#).result(Value::Null);
}

#[test]
fn p184_vector_i32_ref_param() {
    // `&vector<i32>` parameter for in-place modification.
    code!(r#"
        fn push(v: &vector<i32>, val: integer) {
            v += [val as i32];
        }
        fn test() {
            x: vector<i32> = [];
            push(x, 42);
            push(x, 43);
            assert(x[0] == 42);
            assert(x[1] == 43);
        }
    "#).result(Value::Null);
}
```

---

## Specific gaps to watch for

### Inferred vector types from literals

```loft
x = [1 as i32, 2 as i32, 3 as i32];
```

The vector literal's content Type is inferred from the first
element.  `1 as i32` should produce `Type::Integer(..., Some(4))`.
The inferred local type should be `vector<i32>` (narrow).  Audit
`src/parser/expressions.rs::parse_cast` — does it populate
forced_size when casting to a narrow alias?

### Generic parameters

`fn sum<T>(v: vector<T>) -> T` — if the monomorphised T is `i32`,
does the instantiated function's v parameter carry forced_size?
Generic instantiation happens in
`src/parser/interface_resolve.rs` (or similar).  Audit that the
Type substitution preserves forced_size.

### `as i32` on a `vector<integer>` element

```loft
y: vector<integer> = [1, 2, 3];  // wide
x: vector<i32> = [];              // narrow
for e in y {
  x += [e as i32];
}
```

Works as long as the cast produces a narrow value that the append
coerces correctly.  Test it.

### Bounded-generic callee with narrow arg

Using `fn write<T>(v: vector<T>, f: File) where T: Numeric` — does
the monomorphised instance for T=i32 honour the narrow stride?
This is likely where the most interesting bugs hide.

---

## If a test fails

Read the disassembly via `LOFT_LOG=static` to see what opcodes
parse_type emits.  Compare narrow (`vector<i32>`) vs wide
(`vector<integer>`) to spot where the sizes diverge.

The fix typically lives in one of:
- `src/parser/expressions.rs` — literal / cast type inference.
- `src/parser/collections.rs` — vector literal building.
- `src/parser/control.rs` — for-loop iteration.
- `src/variables/mod.rs` — local var type recording.

---

## Acceptance

- [ ] All `p184_*` tests across Phases 3-5 green.
- [ ] Interpreter and native modes produce identical results for
      every test.
- [ ] `lib/moros_sim/tests/*` and `lib/moros_render/tests/*` all
      green (these exercise complex local + parameter vector
      usage).
