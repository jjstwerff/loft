// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# LIFO Store-Free Bug in Native Code Generation

## Summary

`native_dir` and `native_scripts` tests fail with:
```
thread 'main' panicked at src/database/allocation.rs:89:9:
Stores must be freed in LIFO order: freeing store 0 but max is 2
```

This is a pre-existing bug in the native code generator (`src/generation.rs`) that was
hidden because the old `target/release/libloft.rlib` (used by the tests) was compiled
without debug assertions. After fixing `find_loft_rlib()` to pick the freshest rlib
(which is the debug build), the `debug_assert!` in `free_named` now fires.

---

## What is failing and why

### Root cause

When a loft function uses two vector-backed variables (e.g., `positions` and `nexts`),
each backed by a private store allocated via `OpDatabase`:

- `__vdb_1` is allocated **first** → becomes store 0
- `__vdb_2` is allocated **second** → becomes store 1

LIFO requires freeing store 1 (`__vdb_2`) **before** store 0 (`__vdb_1`).

The scopes analysis (`src/scopes.rs`) produces the correct free order in the IR:
`OpFreeRef(__vdb_1)` **before** `OpFreeRef(__vdb_2)`.

Wait — this is **wrong** order. The bytecode dump confirms:

```
# tests/dumps/02-text.loft.txt (line 7-8, 140-141)
__vdb_2(1):ref = null;         ← declared first
__vdb_1(1):ref = null;         ← declared second
...
[62] OpDatabase(__vdb_1, 19);  ← allocated first → store 0
[63] OpDatabase(__vdb_2, 19);  ← allocated second → store 1
...
OpFreeRef(__vdb_1(1));         ← freed first  ← WRONG (store 0, but max=2)
OpFreeRef(__vdb_2(1));         ← freed second
```

### Why the interpreter does not crash

The `debug_assert!` in `free_named` (allocation.rs:89):
```rust
debug_assert!(al == self.max - 1, "Stores must be freed in LIFO order...");
```

...fires only in **debug builds**. The interpreter test binary is a debug build, yet
the interpreter tests **pass**.

**Open question**: Why does the interpreter not panic on this wrong free order?

Possible explanations (not yet verified):
1. The interpreter frees stores in a different actual order despite the IR ordering
2. The `__vdb_N` variables are somehow reset to the null sentinel (u16::MAX) by the
   time `OpFreeRef` executes, causing `free_named` to return early (line 76)
3. The bytecode compiler changes the execution order from the IR order

From `LOFT_STORE_LOG=1` on the interpreter, stores ARE freed in correct LIFO order:
```
[store] free  store=2 (max=3)   ← __vdb_2 freed first  (CORRECT)
[store] free  store=1 (max=2)   ← __vdb_1 freed second
```

So the interpreter correctly frees `__vdb_2` first despite the IR having `__vdb_1`
first. The bytecode compiler or runtime must be reversing the order somehow.

The native code generator follows the IR order literally → frees `__vdb_1` first →
LIFO violation.

---

## Affected files and failing cases

From manual testing (`/tmp/loft_native_*_bin`):

| Binary | Error |
|---|---|
| `02_text` | LIFO: freeing store 0, max=2 |
| `07_vector` | LIFO: freeing store 1, max=14 |
| `08_struct` | LIFO: freeing store 5, max=8 |
| `12_binary` | Different: vector read len=0 assertion |
| `13_file` | LIFO: freeing store 7, max=10 |
| `15_lexer` | LIFO: freeing store 0, max=3 |
| `16_parser` | LIFO: freeing store 0, max=3 |
| `17_libraries` | LIFO: freeing store 5, max=8 |
| `21_random` | LIFO: freeing store 0, max=5 |

`native_scripts` additionally fails with:
`06_functions`, `07_structs`, `08_enums`, `03_text`, `17_map_filter_reduce`,
`09_vectors`, `12_binary`, `16_stress`, `21_lambdas` — all exit 101.

---

## The `var_order` mechanism

In `src/scopes.rs`, `get_free_vars()` iterates variables in **reverse `var_order`**:
```rust
for &v_nr in self.var_order.iter().rev() { ... }
```

Variables enter `var_order` in `scan_set()` when first assigned. For `positions = []`:
1. `depend = [__vdb_1]` → `__vdb_1` pushed to `var_order` first
2. `positions` pushed to `var_order` second

For `nexts = []`:
3. `depend = [__vdb_2]` → `__vdb_2` pushed third
4. `nexts` pushed fourth

So `var_order = [..., __vdb_1, positions, __vdb_2, nexts]`.

`iter().rev()` gives `[nexts, __vdb_2, positions, __vdb_1]`.

`get_free_vars` skips `nexts` (dep≠∅) and `positions` (dep≠∅), adds `__vdb_2` then
`__vdb_1`. Result: `[OpFreeRef(__vdb_2), OpFreeRef(__vdb_1)]` — **correct** LIFO.

**BUT** the bytecode dump shows the IR has `__vdb_1` freed FIRST. This contradicts the
`var_order` analysis. The discrepancy is unresolved — the scan order producing the wrong
IR ordering must come from somewhere else.

**Hypothesis**: The pre-initialisation `Set(__vdb_2, Null)` (which appears at line 7 of
the dump, before `Set(__vdb_1, Null)` at line 8) means `__vdb_2` was pushed to
`var_order` **first**, giving `var_order = [..., __vdb_2, positions_stuff, __vdb_1, ...]`.
Then `iter().rev()` gives `[..., __vdb_1, ..., __vdb_2, ...]` → `get_free_vars`
pushes `__vdb_1` first (wrong).

The source of the reversed declaration order (why `__vdb_2` is pre-inited before
`__vdb_1` in the IR) is unknown. It may relate to `find_first_ref_vars` traversal
or `scan_if` pre-init ordering.

---

## What changed to expose this

On branch `n2-n3-n4-n5-n6-n7-native-fixes`, `find_loft_rlib()` in `tests/native.rs`
was fixed to pick the **most recently modified** rlib across both release and debug
profiles. Previously it unconditionally preferred `target/release/libloft.rlib` (built
Mar 22, before `os_directory_native` was added). That stale release rlib:

- Was missing `os_directory_native`/`os_home_native`/`os_executable_native` (compilation failures)
- Was compiled in **release mode** → `debug_assert!` compiled out → no LIFO panics

The current debug rlib (Mar 23) has the symbols and has `debug_assert!` enabled.

---

## Next investigation steps

1. **Why does the interpreter not crash?** Determine how the runtime achieves correct
   LIFO ordering despite the IR having the wrong free order. Candidates:
   - Check if `OpFreeRef` bytecode reads the variable's runtime value (which may have
     been zeroed/nulled by some other path before the free runs)
   - Check if the bytecode compiler reorders free ops
   - Check if there's an early-exit path that frees one store before the other

2. **Fix the native code generator** to emit `OpFreeRef` calls in the correct order.
   The fix must produce the same store-free sequence as the interpreter.

3. **Alternative short-term fix**: Add the failing files to `NATIVE_SKIP` and
   `SCRIPTS_NATIVE_SKIP` until the root cause is understood and fixed.

---

## Key files

| File | Relevance |
|---|---|
| `src/scopes.rs` | `get_free_vars`, `variables`, `scan_set`, `var_order` |
| `src/generation.rs` | Native code output; `output_block` iterates IR in order |
| `src/database/allocation.rs` | `free_named` with the LIFO `debug_assert!` |
| `src/state/io.rs` | `free_ref` (interpreter's `OpFreeRef` handler) |
| `tests/dumps/02-text.loft.txt` | Bytecode dump showing wrong IR free order |
| `tests/native.rs` | `NATIVE_SKIP`, `SCRIPTS_NATIVE_SKIP` lists |
