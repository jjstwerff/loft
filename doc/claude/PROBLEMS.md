
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Problems in Loft

Known bugs, unimplemented features, and limitations in the loft
language and interpreter.  Each entry records the symptom, workaround, and
recommended fix path.

Completed fixes are removed — history lives in git and `CHANGELOG.md`.

**Before opening a new issue here, check
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md)** — the closed-by-decision
register holds items explicitly evaluated and declined (C3 / C38 /
C54.D / …).  If your symptom maps onto one of those, the fix is to
produce new evidence (reproducer, incident, measurement) on the
existing entry, not re-open it as a bug.

## Contents
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Unimplemented Features](#unimplemented-features)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)
- [Graphics / WebGL](#graphics--webgl)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround |
|---|-------|----------|------------|
| 198 | `tests/scripts/95-alias-copy.loft` leaks Database 3 (allocated by `OpInitRef` at pc≈4788) — `p146_script_95_alias_copy_leak` regression test panics on `roadmap-lsp-eclipse`.  Passes on main; the regression sits in commits between `main` (`05b53b2`) and `roadmap-lsp-eclipse` HEAD.  Most likely culprits: plan-04/05 slot allocator refit, plan-06 par-safety series, or the plan-07 Span IR walker arms missing the alias-copy free path.  Investigated as part of [ARC.md A1](plans/06-typed-par/ARC.md). | High | None at the loft language level — the test catches a runtime invariant. Investigate scopes.rs `scan_set` aliased-return free-emission against the new IR variants (`Value::Span`, `Value::ParFor`) added on this branch. |
| 199 | Native codegen E0499/E0502 when nested calls borrow `&mut Stores` simultaneously.  **Partially fixed (2026-05-01)**: native ABI changed from `&mut Stores` to `&UnsafeCell<Stores>` parameter — generated functions derive a fresh `&mut Stores` from the cell at function entry, so multiple cells can coexist without borrow conflict.  Closes the canonical reproducer (`native_tuple_script` PASSES) and lifts `native_dir` from 7/30 → 23/30 doc scripts.  **Remaining classes (3 sub-issues, see below):** P199.A Op-stub nesting, P199.B template compound expressions, P199.C `n_parallel_queue` native missing. | Medium | Hoist the inner call into a temporary: `let r = add_pair(p); assert(r == 30);` makes the second borrow fall outside `n_assert`'s argument list. |
| 200 | Native codegen for `f += <integer>` against a binary file (`BigEndian` / `LittleEndian` open mode) emits a value with mismatched expected width — `rustc` raises E0308 mismatched types.  Reproducer: `tests/scripts/20-binary.loft` line 67 / 128.  Passes on main, fails on `roadmap-lsp-eclipse` — regression on this branch. | Medium | Add the explicit width cast the parser warning already suggests: `f += val as i32` (or `as i8` / `as u32` etc.). |
| 201 | `tests/html_wasm.rs` Mutex-poison cascade: when one test panics inside `build_lock().lock().unwrap()` (line 174), every subsequent html_wasm test fails with the unhelpful `called Result::unwrap() on an Err value: PoisonError { .. }` instead of the original error message.  The `--html` driver writes to a fixed `/tmp/loft_html.rs` so the lock is genuinely needed; the issue is that a poisoned lock should report the original panic, not be re-unwrapped naively. | Low (test infra) | Use `lock().unwrap_or_else(\|e\| e.into_inner())` to recover from poison; the cascade then surfaces the actual first failure instead of hiding it.  Or have `assert_wasm_rlib_fresh()` fail the test before acquiring the lock so a stale rlib doesn't poison the build serial. |

## Interpreter Robustness

### 198. Alias-copy leak regression — `p146_script_95_alias_copy_leak`

**Symptom:** running `cargo test --release --test leak
p146_script_95_alias_copy_leak` panics:

```
Database 3 not correctly freed (allocated by OpInitRef at pc=4788;
rerun with LOFT_LOG=alloc_free for the full trace)
```

**Where:** `tests/scripts/95-alias-copy.loft`; the original P146 fix
(`scopes.rs::scan_set`, mirroring codegen's `has_ref_params == true`
branch — strip LHS variable's declared deps so OpFreeRef is emitted).

**Branch state (2026-04-29):** passes on `main` (commit `05b53b2`),
fails on `roadmap-lsp-eclipse` (commit `88518bf`).  Regression sits
in the 25 commits the branch added, most likely candidates:

1. plan-04/05 slot allocator refit (`19a06c5`).
2. plan-06 par-safety series (G5 / G5.1 / 5e / 5b').
3. plan-07 Span IR walker arms — `Value::Span` was added to several
   walkers; if `scan_set`'s aliased-return path looks at IR shape
   without `unspan()`-ing, the free emission may be missed.
4. plan-06 `Value::ParFor` walker arms — same concern as above for
   `ParFor`-shaped sub-trees.

**Fix path:** check `src/scopes.rs::scan_set` (and `get_free_vars` /
its helpers) against `Value::Span` and `Value::ParFor` — both
variants need passthrough so the alias-copy detection still fires
through wrapped IR.  Re-run the test under `LOFT_LOG=alloc_free` to
see exactly which alloc has no matching free.

**Test:** `tests/leak.rs::p146_script_95_alias_copy_leak`.

**Tracked in plan-06:** [ARC.md A1](plans/06-typed-par/ARC.md) gates
on this — A1 investigates whether the regression is a plan-06
par-safety series side effect.  If confirmed, the fix becomes a new
arc step before A2.

**A1 status (2026-04-30):** test still fails on `roadmap-lsp-eclipse`
post-A1 commit `b9ad7af` (heavy `parallel_execute_and_collect`
retired).  Current symptom: `Database 3 not correctly freed
(allocated by OpInitRef at pc=4842; …)` — pc shifted from 4788 → 4842
from intermediate commits, but the alloc-without-free shape is
unchanged.  A1 retired only `parallel_execute_and_collect` and the
two `run_parallel_*` helpers it called; that code is unreachable from
non-par scripts, so it is not the cause.  The leak persists into A2
unchanged and remains a candidate for an A2-prerequisite spot fix in
`scopes.rs::scan_set` Span/ParFor passthrough.

### 199. Native codegen E0499 — `n_assert(stores, n_add_pair(stores, …), …)`

**Symptom:** `cargo test --release --test native native_tuple_script`
fails with rustc:

```
error[E0499]: cannot borrow `*stores` as mutable more than once at a time
   --> /tmp/loft_native_50_tuples.rs:439:32
    |
439 |   n_assert(stores, (n_add_pair(stores, var_p)) == (30_i64), { …
    |   -------- ------              ^^^^^^ second mutable borrow
    |   |        |
    |   |        first mutable borrow
```

**Where:** `tests/scripts/50-tuples.loft` line 56:
`assert(add_pair(p) == 30, …)`.  Native codegen lowers
`assert(expr, msg)` to `n_assert(stores, expr_eval, msg_eval, …)` and
inlines both arguments — when the inner expression itself takes
`&mut stores`, rustc rejects.

**Branch state (2026-05-01):** canonical case (`native_tuple_script`)
**PASSES** after the UnsafeCell ABI refactor.  See "Shipped fix" below
for what landed and "Remaining sub-issues" for what still fails.

**Workaround (still applies for the remaining sub-issues):** rewrite
the loft source as `r = add_pair(p); assert(r == 30, …);` so the
second borrow leaves the outer call's argument list.

**Tests that surface this (each runs `assert` with a nested
`&mut stores` call inside the message format-string or the test
expression):**

| Test | Failure shape | Inner call |
|---|---|---|
| `cargo test --release --test native native_tuple_script` | format-string with `add_pair(p)` | `n_add_pair(stores, var_p)` |
| `cargo test --release --test native native_tuple_return_script` | same fingerprint as above | `n_add_pair` |
| `cargo test --release --test native native_binary_script` | format-string with `vector_len(rv)` | `t_6vector_len(stores, var_rv)` |
| `cargo test --release --test native native_dir` | 23 of 30 doc scripts | various user-fn calls inside `assert(...)` format-strings |
| `cargo test --release --test html_wasm moros_editor_html_smoke` | `OpCopyRecord(stores, n_build_chunk(stores, …), …)` | `n_build_chunk` |
| `bench/11_par/bench.loft` native column | format-string with `t_5float_round(stores, …)` | `t_5float_round` |

#### Shipped fix (2026-05-01) — UnsafeCell ABI refactor

Native function ABI changed from `(stores: &mut Stores, …)` to
`(cell: &std::cell::UnsafeCell<Stores>, …)`.  Each generated function
opens its body with:

```rust
let stores: &mut Stores = unsafe { &mut *cell.get() };
```

so all subsequent template substitutions and inner emissions reference
`stores` exactly as before.  When function A calls function B, A
passes `cell` (a copyable shared borrow of the cell) — multiple `&cell`
references coexist freely under Rust's borrow checker, eliminating
E0499 by construction at the function-call boundary.

`UnsafeCell<T>` is `repr(transparent)`; deriving the `&mut T` is
zero-cost.  Each function's `&mut Stores` is scoped to its body and
dropped before the function returns, so on a single-threaded call
stack only one `&mut Stores` is actively dereferenced at any moment —
this is the canonical safe usage of UnsafeCell-derived references.

**Files touched:**
- `src/generation/mod.rs` — function signature emission, init, main entry, native API call paths.
- `src/generation/calls.rs` — `output_call_user_fn` passes `cell` for user-fns; `Op*` and `CODEGEN_RUNTIME_FNS` stubs still pass `stores` (legacy ABI).
- `src/generation/coroutine.rs` — coroutine factory takes `cell`; `next_i64` body derives `cell` from `stores` for inner user-fn calls.
- `src/generation/dispatch.rs` — fn-ref match dispatch passes `cell`; parallel worker closures take `cell`.
- `src/generation/emit.rs` — fn-ref candidate match arms pass `cell`.
- `src/codegen_runtime.rs` — `n_parallel_for_*` helpers updated to take `Fn(&UnsafeCell<Stores>, …)` closures and cast `&mut ws.stores` at the worker boundary via the `repr(transparent)` cast.
- `src/main.rs` — test main bootstrap wraps `Stores::new()` in `UnsafeCell` at startup.

**Verification (after refactor):**

| Test | Result |
|---|---|
| `cargo test --release --test issues` | 538/540 (2 P144/P157 test-pattern checks need updating for new ABI) |
| `cargo test --release --test threading` | 43/43 |
| `cargo test --release --test threading_chars` | 35/35 |
| `cargo test --release --test native native_tuple_script` | PASS |
| `cargo test --release --test native native_dir` | 23/30 (was 7/30) |
| `cargo test --release --test native` (full) | 37/92 |

#### Remaining sub-issues

##### 199.A — Op-stub nested calls still hit E0499

**Pattern:** `OpNewRecord(stores, OpGetRecord(stores, …))` — two
codegen_runtime.rs Op stubs nested at the same scope.  Both take
`&mut Stores` (the legacy ABI we deliberately preserved for stubs); the
borrow checker rejects two simultaneous mutable borrows.

**Affects:** `15_lexer`, `16_parser` in `native_dir`; possibly more
non-doc native tests.

**Fix path:** Convert the ~32 `Op*` and `n_*` helpers in
`src/codegen_runtime.rs` from `(stores: &mut Stores, …)` to
`(cell: &UnsafeCell<Stores>, …)` + entry prelude.  Mechanical edit,
same pattern as the generated-function refactor.  Once converted,
remove the `is_op_stub`/`is_codegen_runtime_fn` special case in
`src/generation/calls.rs::output_call_user_fn` so calls pass `cell`
to these too.

**Effort:** ~2-3 hours.  Well-bounded.

##### 199.B — Template compound-expression conflict (E0502)

**Pattern:** `stores.store_mut(&db).set_int(…, {…stores.st… inner…})`
— `@v1.field = @v2` template substitutes both placeholders with text
referencing `stores`.  After substitution, the outer `store_mut(&db)`
holds `&mut stores` while the inner `stores.store(&db).foo()` wants
`&stores` (immutable) — E0502.

**Affects:** `08_struct`, `17_libraries`, `18_locks` in `native_dir`.

**Fix path:** Either (1) re-attempt the IR-level lift in scope analysis
focused on this specific compound shape, OR (2) rewrite the offending
templates in `default/01_code.loft` to use `let _t = …;
stores.store_mut(&db).set_int(…, _t);` form.  Approach (2) is more
surgical — likely 5-10 specific templates touch this.

**Effort:** ~2-4 hours.

##### 199.C — `n_parallel_queue` native missing

**Pattern:** `n_parallel_queue(cell, var__vector_1, 8, 8, 4, 547, 0)`
— this function is registered in `src/native.rs` (interpreter
opcode) but has no `codegen_runtime.rs` implementation.  Generated
code calls it with the new `cell` ABI; rustc reports "unknown
function".

**Affects:** `19_threading` in `native_dir`; any native script using
`parallel_queue`.

**Fix path:** Either implement `n_parallel_queue` (and its `_text`,
`_ref`, `_narrow`, `_fn` variants) in `codegen_runtime.rs` mirroring
the interpreter version in `src/native.rs:826`, OR document as an
interpreter-only feature.

**Effort:** ~2-4 hours if implementing all variants; ~30 min if
documenting.

**Pre-existing — not caused by P199 fix.**

#### Original investigation (superseded — kept for context)

The original investigation explored a 4-tier fix in
`src/generation/pre_eval.rs` (text-based pre-evaluation hoisting).
That approach was abandoned in favour of the UnsafeCell ABI refactor
above because each tier exposed a deeper interaction (Span wrappers,
counter sharing, Op-template `@v0` letprop substitution).  The
ABI-level fix sidesteps the entire problem class.

The IR-level scope-analysis lift (`scopes.rs::scan_args` extension)
was also explored and reverted — it conflicted with downstream slot
allocation invariants in stdlib hot paths (`t_4text_split`).

#### Acceptance for closing P199

P199 closes when ALL of these hold (sub-issues 199.A, 199.B, 199.C
landed):

```bash
cargo test --release --test native                       # 5/5 named native tests passing
cargo test --release --test native native_dir            # 30/30
cargo test --release --test html_wasm moros_editor_html_smoke  # PASS
cargo test --release --test issues                       # 540 passing — unchanged
cargo test --release --test threading                    # 43 passing — unchanged
cargo test --release --test threading_chars              # 35 passing — unchanged
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo build --no-default-features
```

### 200. Native codegen E0308 — `f += <integer>` width mismatch on binary file

**Symptom:** `cargo test --release --test native native_binary_script`
fails with rustc E0308 mismatched types when emitting a binary-file
write through the `BigEndian` / `LittleEndian` open mode.

**Where:** `tests/scripts/20-binary.loft` lines 67 and 128 —
`f += <integer>` where `f` is a binary file and the integer literal
has no width cast.  The parser already warns: *"`f += <integer>`
without a width cast writes 8 bytes; for binary files
(BigEndian / LittleEndian) add `as i8` / `as i16` / `as i32` /
`as u8` / `as u16` / `as u32`."*  Native codegen still emits a
mismatched-width Rust expression that rustc rejects.

**Branch state (2026-04-29):** passes on `main`, fails on
`roadmap-lsp-eclipse`.  Regression on this branch.

**Fix path:** native codegen for `OpAddIntoFile` (or the equivalent
binary-file-append op) must emit the same width cast the
interpreter applies — the test exercises the un-cast 8-byte default
path which is supposed to compile to `f.write_all(&val.to_be_bytes())`
or similar.  Trace the expected and actual emitted Rust for the
failing line in `/tmp/loft_native_20_binary.rs:522`.

**Workaround:** add the cast the parser warning suggests: `f += val
as i32` (or whatever width matches the binary record format).

**Test:** `tests/native.rs::native_binary_script`.

### 201. html_wasm Mutex-poison cascade obscures original failure

**Symptom:** when any test in `tests/html_wasm.rs` panics inside the
`build_lock().lock().unwrap()` critical section (line 174), every
subsequent test in the same binary fails with:

```
called `Result::unwrap()` on an `Err` value: PoisonError { .. }
```

The original panic message (e.g. "expected 'sum=45' in output.
stdout: � �", or "stale wasm32-unknown-unknown rlib — rebuild with
…") is reported only by the first test to panic; the others are
unhelpful PoisonError noise.

**Where:** `tests/html_wasm.rs:174` —
`let _guard = build_lock().lock().unwrap();`.  The lock is genuinely
needed (the loft `--html` driver writes to a fixed
`/tmp/loft_html.rs` path; parallel test invocations would overwrite
each other's emitted Rust mid-build), but its unwrap doesn't tolerate
poisoning.

**Fix path:** swap `lock().unwrap()` for `lock().unwrap_or_else(|e|
e.into_inner())`.  Rust documents this as the canonical recovery
pattern — the lock data isn't shared mutable state in this case, the
guard is a sequencing token, so taking it from a poisoned lock is
safe.  Alternatively, run `assert_wasm_rlib_fresh()` outside the
lock so a stale rlib panics every test independently with the clear
"rebuild" message instead of cascading PoisonError onto siblings.

**Workaround:** `cargo test --test html_wasm -- --test-threads=1`
serialises tests so the cascade doesn't compound, but the first
panic's message is still the only useful diagnostic.

**Test:** `tests/html_wasm.rs::*` (any test that runs after a
panicked sibling).

## Web Services

*(none)*

## Graphics / WebGL

*(none)*

## Package / Multi-file

*(none)*

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [CAVEATS.md](CAVEATS.md) — Verifiable edge cases with reproducers
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements
