---
name: Store leak debugging status
description: Current state of store leak fixes and the remaining WASM breakout use-after-free bug
type: project
---

## Store leak fixes — all committed on `first-game` branch

Six store leak types fixed, all passing locally (6 leak tests + 9 wrap tests + 96 scripts):

1. **Struct field init from call** (`src/parser/objects.rs`) — work_ref captures callee return store via inline_ref
2. **Deep-copied returns** (`src/scopes.rs`) — clear dep on first assignment only (not reassignment) when codegen will deep-copy
3. **#fields iteration** (`src/parser/collections.rs`) — free loop var between iterations, inline_ref for loop var, proper skip_free
4. **Callee return store** (`src/state/codegen.rs:1127`) — high-bit flag on OpCopyRecord in gen_set_first_ref_call_copy frees source store after deep copy
5. **Var-to-var deep copies** (`src/scopes.rs`) — clear dep for Value::Var assignments (iterator objects)
6. **Generate_set FreeRef** (`src/state/codegen.rs:~866`) — commented out TODO; was freeing old store on owned Reference reassignment, but causes use-after-free in WASM breakout

## Remaining bug: WASM breakout use-after-free on store 14

**Symptom**: `get_vector: use-after-free on store 14 (rec=1 pos=8)` at `bc=17019` (inside `mat4_mul` reading `proj.m[0]`). Crashes after ~60 frames in browser/Node.js WASM.

**Key trace data**:
- Store 14 freed at `bc=26756` (`n_main+4123`) — an end-of-scope FreeRef
- Store 14 was created at `bc=16154`, last operated at `bc=26682`
- The freed store is then read by `mat4_mul` reading `br_proj.m` vector
- 54 stores leaked per frame (one per `rect_mvp` call) when generate_set FreeRef is disabled

**What we've ruled out**:
- NOT caused by our generate_set FreeRef changes (also crashes with it disabled)
- NOT reproduced in native Rust interpreter even with 1000 frames + yield/resume
- NOT reproduced with local `Mat4` definition + yield_test library
- NOT reproduced even with real `math.loft` + `graphics.loft` loaded + yield/resume
- The bug is WASM-specific — something in the WASM compilation or runtime causes it

**What we know**:
- The `bc=26756` FreeRef is a pre-existing end-of-scope free in `n_main`
- It frees a store that `br_proj.m` vector data lives in
- `br_proj = ortho()` uses O-B2 adoption (no ref params) — store is adopted from callee
- The adopted store somehow gets freed by a different variable's end-of-scope cleanup
- This suggests store aliasing: two variables share the same store number, one gets freed

## Test infrastructure

- `tests/leak.rs` — workbench with `run_leak_check_str`, `run_leak_check`, and `breakout_yield_resume` test
- `tests/lib/yield_test/` — mock library with `#native "mock_yield_frame"` + 46 dummy natives
- `tests/scripts/85-yield-resume.loft` — breakout-pattern script using real `math::Mat4` + `use graphics;`
- `test_breakout.mjs` — Node.js reproduction of WASM crash (run with `node test_breakout.mjs`)
- WASM diagnostics in `src/database/allocation.rs` (store 14 alloc/free logging) and `src/fill.rs` (get_vector pre-crash logging) and `src/state/io.rs` (free_ref bc+function logging)
- Store growth warnings in `src/wasm.rs` resume_frame

## Session 2 findings (2026-04-08)

### Per-frame leak fix is already in place

The generate_set FreeRef at codegen.rs:866-882 is ACTIVE with a dep-empty guard. The dep-clearing in scopes.rs (line 403-427) correctly distinguishes deep copies from O-B2 adoption. All 9 leak tests pass including 1000-frame breakout simulations. The 54/frame leak from the status doc appears to be fixed.

### Bug reproduces on native Ubuntu — NOT WASM-specific

`cargo run --bin loft -- --interpret lib/graphics/examples/25-breakout.loft` crashes immediately:
```
get_vector: use-after-free on store 14 (rec=1 pos=8)
```

The simplified leak tests pass because they don't use `use graphics;` — the real breakout has more variables and different store allocation order.

### Root cause identified: br_mvp is TWO separate variables

The bytecode dump reveals **two distinct `br_mvp` variables at different stack slots**:

- `br_mvp[397]` — scoped to the `if bricks[...] == 1` block inside the brick-drawing loop (line 217)
- `br_mvp[381]` — scoped to the outer frame loop, used for paddle/ball/lives drawing (lines 227, 233, 240)

Because `br_mvp` is first assigned inside an `if` block (line 217), loft scopes it to that block. The later assignments at lines 227, 233, 240 create a NEW variable with the same name in the outer scope.

### Three FreeRef locations that free store 14

Traced with `LOFT_STORE_DEBUG=1` (temporary diagnostic in io.rs):

| Bytecode offset | What | Purpose |
|---|---|---|
| n_main+3832 | `VarRef(br_mvp[397]) + FreeRef` | Reassignment FreeRef before `br_mvp = rect_mvp(...)` inside brick loop |
| n_main+3963 | `VarRef(br_mvp[397]) + FreeRef` | End-of-`if`-block scope cleanup — frees br_mvp[397]'s store |
| n_main+4122 | `VarRef(br_mvp[381]) + FreeRef` | Reassignment FreeRef before `br_mvp = rect_mvp(...)` in ball section — **THE FATAL ONE** |

### Bytecode flow at the crash

```
; Draw paddle — first assignment of br_mvp[381]
3990[381]: ConvRefFromNull()                     ; allocate null store for br_mvp[381]
3991[393]: Database(var[381], db_tp=53)           ; allocate real Mat4 store
3996[393]: VarRef(br_proj[265])                   ; push br_proj for rect_mvp call
...
4032[449]: Call(n_rect_mvp)                       ; call rect_mvp
4043[405]: VarRef(br_mvp[381])                    ; push dest for CopyRecord
4046[417]: CopyRecord(tp=32821)                   ; deep copy with free_source (0x8000 flag)

; Draw ball — reassignment of br_mvp[381]
4119[393]: VarRef(br_mvp[381])                    ; push OLD br_mvp[381]
4122[405]: FreeRef                                ; FREE OLD STORE ← this frees store 14
4123[393]: VarRef(br_proj[265])                   ; push br_proj ← USE-AFTER-FREE if store 14 was br_proj's store!
```

### Why store 14 ends up being br_proj's store

Store 14 is initially allocated as one of the 14 ConvRefFromNull stores at the top of main. Through repeated alloc/free cycling in the brick loop (br_mvp[397] alloc+free on each iteration), store 14 gets recycled. At some point, the "Draw paddle" section's `CopyRecord` with `free_source` flag (tp=32821 = 0x8000 | 53) frees the source store from rect_mvp's return. This source store happens to be the same store number as br_proj's store, causing br_proj's data to be freed.

**The actual aliasing**: when rect_mvp is called for the paddle, the callee creates temporary stores. One of these temporaries may reuse store 14 (which was freed from the brick loop's br_mvp[397]). The `free_source` in CopyRecord then frees this temporary — which is store 14. But br_proj also lives in store 14.

### Probable fix

The breakout code needs `br_mvp` declared BEFORE the brick loop so it's a single variable in the outer scope:

```loft
br_mvp = math::mat4_identity();  // declare in outer scope
// then use br_mvp everywhere
```

But this is also a compiler issue: the FreeRef at n_main+4122 (reassignment of br_mvp[381]) should NOT free a store that belongs to another live variable. The store aliasing between br_proj and the CopyRecord source needs investigation.

### Wrap test fix

Added `85-yield-resume.loft` to `ignored_scripts()` in wrap.rs since it requires lib paths (graphics, math, yield_test) that the wrap runner doesn't provide. Already tested via `breakout_yield_resume` in leak.rs.

### Diagnostic changes (temporary)

- `src/state/io.rs`: added `LOFT_STORE_DEBUG` env var for non-WASM store 14 free tracing
- `tests/leak.rs`: added lib_dirs to `dump_breakout_bytecode`, enabled bytecode dump for n_main
- `tests/wrap.rs`: added `85-yield-resume.loft` to ignored_scripts()

## Next steps

1. **Investigate the CopyRecord free_source aliasing** — the `CopyRecord(tp=32821)` at bc=4046 frees the source store after deep copy. If this source store is store 14 (same as br_proj), that's the root cause. Need to verify whether rect_mvp's return store can alias with br_proj's store.

2. **Fix the breakout code** — declare `br_mvp` before the brick loop to make it a single outer-scope variable. This avoids the two-variable problem.

3. **Investigate whether the compiler should prevent this** — when two variables at different scopes have the same name, the end-of-scope FreeRef for the inner one should not affect the outer one. But the store aliasing through CopyRecord's free_source is a separate issue.

4. **Clean up diagnostic code** — remove LOFT_STORE_DEBUG after the bug is fixed.
