
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
| 198 | `tests/scripts/95-alias-copy.loft` leaked Database 3 + ran with aliased ac_orig/ac_copy in native.  **Closed (2026-05-01)**: scope analysis (`scan_set`) and native deep-copy emission (`output_set`) both pattern-matched `Value::Call(...)` / `Value::Var(...)` without unwrapping the parser's `Value::Span` wrapper, so the deep-copy and `make_independent` (deps-clearing) paths fell through.  Fix: bind unspanned RHS once at top of each function and pattern-match against that.  Also closes `95_alias_copy` native run failure. | High (closed) | n/a — fix landed in commit 30b01ce. |
| 199 | Native codegen E0499/E0502 when nested calls borrow `&mut Stores` simultaneously.  **Closed (2026-05-01)** for the borrow-conflict class.  Native ABI changed from `&mut Stores` to `&UnsafeCell<Stores>` (PR1), Op-stub helpers in `src/codegen_runtime.rs` follow the new ABI (Track 1), and `OpSet*` field-write templates lift `@val` into a let-binding before `store_mut(...)` (Track 2).  Result: `native_dir` 7/30 → 28/30; canonical reproducer `native_tuple_script` PASSES; 0 interpreter regressions.  Remaining `19_threading` failure tracked as P202 (separate feature gap — `n_parallel_queue` family has no native implementation, distinct from the borrow-conflict issue P199 addressed). | Medium (closed) | Hoist the inner call into a temporary: `let r = add_pair(p); assert(r == 30);` makes the second borrow fall outside `n_assert`'s argument list — only relevant for templates / Op-stubs not yet covered by this fix. |
| 202 | Native codegen for `for ... par(...)` for-loops calls `n_parallel_queue` (and `_text` / `_ref` variants) which only existed in the bytecode interpreter.  **Closed (2026-05-02)** by plan-09 phase 06 — `n_parallel_queue_*_native` runtime fns + `n_parallel_buf_get_*` / `_drop_*` per-row accessors added to `src/codegen_runtime.rs`; `ParallelQueueEmitter` + `ParallelBufRenameEmitter` registered in `src/generation/ops/parallel.rs`; reachability extended in `src/generation/mod.rs` to pull worker fns from queue d_nr args.  Native suite: 87/93 → 89/93 + native_dir 29/30 → 30/30 (closes 19_threading + 22_threading + 40_par_ref_return).  Pinned by `p202_parallel_queue_*` regression tests in `tests/codegen_emitter.rs`. | Medium (closed) | n/a — fix lands in `src/codegen_runtime.rs` + `src/generation/ops/parallel.rs` + `src/generation/mod.rs`. |
| 200 | Native codegen for binary-file read sites emitted block-tail `(read_value) as u8` (block result narrowed) but compared against `_i64` literal RHS — rustc raised E0308 mismatched types.  **Closed (2026-05-02)** by plan-09 phase 10 step 10.3: new `IntCompareEmitter` in `src/generation/ops/int_compare.rs` wraps both operands of `OpEqInt` / `OpNeInt` / `OpLtInt` / `OpLeInt` in `(operand as i64)` to normalise to a common width.  Native suite: 90/93 → 91/93 (all 5 P200 sub-failures retired); `native_binary_script` flips from FAILED → ok.  Pinned by `p200_binary_compiles_under_native` + `p200_int_compare_emitter_registered` regression tests in `tests/codegen_emitter.rs`. | Medium (closed) | n/a — fix lands in `src/generation/ops/int_compare.rs`. |
| 201 | `tests/html_wasm.rs` Mutex-poison cascade: when one test panics inside `build_lock().lock().unwrap()` (line 174), every subsequent html_wasm test fails with the unhelpful `called Result::unwrap() on an Err value: PoisonError { .. }` instead of the original error message.  The `--html` driver writes to a fixed `/tmp/loft_html.rs` so the lock is genuinely needed; the issue is that a poisoned lock should report the original panic, not be re-unwrapped naively. | Low (test infra) | Use `lock().unwrap_or_else(\|e\| e.into_inner())` to recover from poison; the cascade then surfaces the actual first failure instead of hiding it.  Or have `assert_wasm_rlib_fresh()` fail the test before acquiring the lock so a stale rlib doesn't poison the build serial. |
| 203 | Assertion `delete(path) == FileResult.Ok` panicked with `delete existing file`.  **Closed (2026-05-02)**: actual root cause was template double-substitution — five templates in `default/01_code.loft` (lines 690, 705, 707, 751, 753) substituted `@v1`/`@v2` in multiple positions, so any side-effecting call appearing on both sides of an enum/null comparison evaluated twice.  Fix: `src/generation/calls.rs::output_call_template` now scans every template for repeated `@<name>` placeholders and hoists each into a single `let _v_<name> = …;` wrapping `{ … }` block before substitution, so each arg expression evaluates exactly once regardless of how many positions reference it.  `repro_p203.loft` exits 0; full suite: 540/540 issues, 43/43 threading, 35/35 threading_chars, native 86/92 → 87/92.  See [P203 reproducer + diagnosis](#203-native-block-scope-file-not-flushed-on-exit). | Medium (closed) | n/a — fix lands in `src/generation/calls.rs`. |
| 204 | Native: tail-expression `return inner_call()` from a struct-returning function emitted `n_inner(cell, args)` as a void STATEMENT and then `return DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };` (null sentinel) — caller's OpCopyRecord then panics on the null sentinel.  **Closed (2026-05-02)** by plan-11: `src/generation/pre_eval.rs::detect_ref_tail_capture` now unspans `Value::Span` wrappers before matching, so the tail-position Call is properly captured via `let __native_tail_ret: DbRef = call(...);` and returned instead of the null sentinel.  Existing tail-capture infrastructure was correct; the walker just didn't see Span-wrapped operators.  Native suite: 91/93 → **93/93** (`85_yield_resume` + `87_store_leaks` closed, `repro_p204.loft` un-marked).  Pinned by `p204_tail_expression_return_passes_under_native` regression test in `tests/codegen_emitter.rs`. | Medium (closed) | n/a — fix lands in `src/generation/pre_eval.rs`. |
| 205 | Native: bounded-generic dispatch `fn f<T: Trait>(x: T) -> text { x.to_label() }` returned a `Str` whose pointer referenced a local `String` that dropped on function return → dangling pointer, comparison failed.  **Closed (2026-05-02)** by plan-09 phase 07: `src/generation/emit.rs` now detects "function returns Type::Text but has no `Type::RefVar(Type::Text(_))` attribute" at both Value::Return and block-tail wrap_result emit sites, and routes the value through `stores.scratch` (the backing String lives as long as `stores` does — same pattern `n_parallel_buf_get_text_native` uses).  Native suite: 89/93 → 90/93 (`86_interfaces` closed).  Pinned by `p205_repro_passes_under_native` + `p205_no_str_new_of_local_in_corpus` regression tests in `tests/codegen_emitter.rs`. | Medium (closed) | n/a — fix lands in `src/generation/emit.rs`. |
| 207 | Native codegen E0308 when comparing a `character` element read from a `Type::Tuple` against a `character` literal.  Reproducer: `t = ('a', 42); if t.0 == 'a' { ... }` — under `--native` rustc rejected the generated `let _v_v1 = (var_t.0); if _v_v1 == char::from(0) { ... } else { i64::from(_v_v1 as u32) }` because `var_t.0` is typed as `i32` (per the tuple element layout) but the null-sentinel comparison uses `char::from(0)` (a `char`).  The non-tuple form (`c = 'a'; if c == 'a'`) compiled fine — the divergence was specific to tuple-element char reads.  **Closed (2026-05-04)** by `src/generation/calls.rs::substitute_template_body` — added a `Value::TupleGet` arm to the existing char-typed-parameter wrap (mirrors the `Value::Var` arm at line 302).  The arg gets `ops::to_char(var_t.0)` wrapping before the template substitutes it for `@v1`, so the `@v1 == char::from(0)` comparison gets a `char`, not the `i32`-typed tuple-element read.  Pinned by `e1_d1_char_int_local` in `tests/tuple_matrix.rs` (un-ignored). | Medium (closed) | n/a — fix lands in `src/generation/calls.rs`. |
| 208 | Native codegen E0282 "type annotations needed" when a bounded-generic method call returns text and the result is used in `+ "!"` text concatenation.  Reproducer: `fn label<T: Printable>(x: T) -> text { x.to_text() + "!" }`.  Surfaced by plan-17 phase 01 (B) follow-up — was masked by the earlier type-inference failure that rejected the snippet at parse time.  Two scratch.push wraps stacked redundantly: the function body block (`Add text_2: text`) had its last operator as a `Value::Return` whose own emit path wraps with `return { stores.scratch.push((expr).to_string()); Str::new(...) }`; the outer block-tail emit then wrapped the same expression in another `{ stores.scratch.push((block).to_string()); ... }`.  The inner `return` made the outer wrap unreachable; rustc rejected the `to_string()` on the never-typed expression with E0282.  **Closed (2026-05-04)** by extending `value_is_return` in `src/generation/emit.rs::output_block` to walk through Block tails — when a Block's last operator is a Return (recursive), the outer `wrap_result` scratch-push wrap is suppressed.  The inner Return handles all return-value semantics on its own.  Pinned by interpreter + native verification of the reproducer. | Medium (closed) | n/a — fix lands in `src/generation/emit.rs`. |
| 209 | Match guard arms with pattern bindings (`x if x < 0 => …`) saw the binding variable as uninitialised because the binding assignment was prepended only to the arm body, not to the guard expression.  Result: `x` read as 0 inside the guard, so `x if x < 0` failed and `x if x == 0` matched everything; interp shifted arms by one, native always returned arm 2.  **Closed (2026-05-04)** by `src/parser/control.rs::parse_scalar_match` — when an arm has both bindings and a guard, the guard is wrapped in a `binding_guard` block whose statements run the bindings before evaluating the guard.  The enum-variant struct-field path at `build_scalar_chain`'s call site already did this correctly; the scalar-match path was the missing case.  Pinned by `tests/issues.rs::p209_scalar_match_guard_sees_pattern_binding`. | High (closed) | n/a — fix lands in `src/parser/control.rs`. |
| 210 | Native coroutine `while … { yield … }` silently returned 0 because `collect_segments` in `src/generation/coroutine.rs` only recognised `Value::Block` containing yields (the for-loop shape) and missed `Value::Loop` (the while-loop shape).  The state machine ended up with no arms, so every `next_i64` call returned `COROUTINE_EXHAUSTED` and the driving for-loop broke immediately.  Interp drives generators via the bytecode VM, not the state-machine lowering, so it was unaffected.  **Closed (2026-05-04)** by extending the matcher to `Value::Block(_) \| Value::Loop(_)`.  Pinned by `tests/issues.rs::p210_native_coroutine_while_yield`. | High (closed) | n/a — fix lands in `src/generation/coroutine.rs`. |
| 211 | Coroutine `yield text` produces wrong output on both backends.  Interp returns rc=0 with **empty stdout** (should print three names); native fails codegen with "cast cannot be followed by a method call".  Yielded text's backing String likely doesn't survive the yield boundary.  Reproducer: `fn names() -> iterator<text> { yield "alice"; yield "bob"; yield "carol"; }`.  Surfaced by plan-16 pre-flight (2026-05-04).  Same active-risk class as P205 (text lifetime through codegen layers).  Reproducer in /tmp/p16_probes/y2_x1_text_for.loft. | High (S1, open) | n/a — yielding text is broken end-to-end. |
| 212 | Nested tuple literals (`((1,2), (3,4))`, triply nested, or any tuple containing a tuple) panicked at `src/state/codegen.rs:1527:38` because the inline match in `gen_set_first_at_tos`'s `Type::Tuple` arm had no case for an inner element of `Type::Tuple(_)` — it fell through to the "unsupported elem" panic.  **Closed (2026-05-04)** by extracting per-leaf `OpPut*` emission into a recursive helper `emit_tuple_put_ops` that descends through nested tuples, computing each leaf's absolute slot offset and emitting in reverse to match the depth-first push order used by tuple-literal evaluation.  Pinned by `tests/issues.rs::p212_nested_tuple_literal` + `p212_triply_nested_tuple_literal`. | High (closed) | n/a — fix lands in `src/state/codegen.rs`. |
| 213 | Capturing closure stored in struct field panicked under interp at `src/store.rs:963` and rejected in native with E0308 because struct fn-ref fields allocate 4 bytes (just the d_nr) but a capturing-closure value is 16 bytes (4B d_nr + 12B closure DbRef).  Codegen wrote 4 bytes via OpSetInt4 leaving 12 bytes of corrupted state on the stack.  **Closed (2026-05-04, partial)** — `src/parser/mod.rs::set_field_check`'s `Type::Function` arm now detects a `Value::FnRef` with a non-MAX closure_var (walked through Block/Span/Set wrappers via `capturing_fn_ref` helper) and emits a clear diagnostic with the workaround.  The proper fix (widen fn-ref struct fields to 16 bytes so captures persist) is a multi-session refactor touching field allocation, OpSet*/OpGet* family, native codegen, and tuple/vector layouts; tracked as a follow-up.  Pinned by `tests/parse_errors.rs::p213_capturing_closure_in_struct_field_rejected` + `p213_noncapturing_closure_in_struct_field_works`. | High (closed via diagnostic; layout fix deferred) | Define the function at file scope or pass the closure via a fn-typed parameter / return value instead of storing in a struct field. |
| 214 | **Non-capturing** vector-of-closures (`vector<fn(integer) -> integer> = [|x| {x+1}, |x| {x*2}]`) panics under interp at `src/state/mod.rs:319` and rejects in native with E0605 cast `DbRef as (u32, DbRef)`.  Plan-15 README claims D4 should pass for non-capturing closures; pre-flight shows the documented-supported shape doesn't work.  Surfaced by plan-15 pre-flight (2026-05-04).  Reproducer in /tmp/p15_probes/c0_d4_noncap_vec.loft. | High (S2, open) | Define each fn at top level and pass the names: `fn f(x:integer)->integer{x+1} fn g(x:integer)->integer{x*2}; v = [f, g];`. |
| 215 | Closure-typed local var unreachable from inside another closure body.  Calling `inner(y)` where `inner = fn(x:integer)->integer{…}` from inside `outer = fn(y:integer)->integer{ inner(y) + 1 }` rejects with "Unknown function inner".  Workaround `(inner)(y)` panics with index-out-of-bounds at `src/database/allocation.rs:162`.  Captured-closure name resolution gap.  Surfaced by plan-15 pre-flight (2026-05-04).  Reproducer in /tmp/p15_probes/c6_d1_nested_local.loft. | High (S2, open) | Inline the inner body, or define both at top level. |
| 216 | Tuple-capture in closure body diverges silently between backends.  `t = (3, 7); f = fn(x:integer)->integer { t.0 + x }; f(10)` panics under interp at `src/store.rs:227` and produces empty output (rc=0) under native instead of `13`.  Surfaced by plan-15 pre-flight (2026-05-04).  Reproducer in /tmp/p15_probes/c4_d1_tuple_local.loft. | High (S1, open) | Pre-extract: `t0 = t.0; f = fn(x:integer)->integer { t0 + x };`. |

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

### 199. Native codegen E0499 — `n_assert(stores, n_add_pair(stores, …), …)` *(CLOSED 2026-05-01)*

**Status:** Closed for the borrow-conflict class.  Three coordinated
fixes shipped on `roadmap-lsp-eclipse`:

- PR1 — Native ABI from `&mut Stores` to `&UnsafeCell<Stores>` for
  generated functions and the `n_parallel_for_*` helpers.
- Track 1 — Op-stubs in `src/codegen_runtime.rs` (and their direct
  emission sites in `dispatch.rs`) routed through the new ABI.
- Track 2 — `OpSet*` field-write templates in `default/01_code.loft`
  lift `@val` into a `let v = @val;` binding before `store_mut(…)`.

**Result:** `native_dir` 7/30 → 28/30; `native_tuple_script` PASSES; 0
interpreter regressions across 540 issues / 43 threading / 35
threading_chars tests.  Remaining `19_threading` failure tracked as
**P202** (separate feature gap, not a borrow-conflict bug).

#### Original symptom (kept for context)

`cargo test --release --test native native_tuple_script` was failing
with rustc:

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

### 200. Native codegen E0308 — narrow block-tail vs i64 literal in comparison (CLOSED 2026-05-02)

**Closed by:** plan-09 phase 10 step 10.3 (commit landing this entry).
The actual root cause was NOT the write side — phase 00a's
`feedback_actual_error_survey` found that all 5 failing sites in
`tests/scripts/20-binary.loft` were on the read side:

```rust
n_assert(cell, {({ //reading file_4: integer(0, 255)
    let mut var__read_3: i64 = i64::MIN as i64;
    OpReadFile(cell, var_f, &mut var__read_3, 1_i64, 11_i32);
    (var__read_3) as u8     // ← block-tail narrow (narrow_int_cast)
}) == (0_i64)               // ← E0308: u8 vs i64
```

The block-tail narrows the read result to u8 (intentional — the
read returns `integer(0, 255)`).  The comparison RHS literal is
emitted as `_i64` (the default Int emission).  Rust requires
both sides of `==` to share a type → E0308.

Fix: new `IntCompareEmitter` in `src/generation/ops/int_compare.rs`
registered for `OpEqInt` / `OpNeInt` / `OpLtInt` / `OpLeInt`.
Wraps each operand in `(operand as i64)`:

```rust
write!(ctx.w, "((")?;
ctx.emit(&args[0])?;
write!(ctx.w, ") as i64) {op} ((")?;
ctx.emit(&args[1])?;
write!(ctx.w, ") as i64)")
```

`as i64` is widening for u8/u16/i8/i16 and a no-op for i64.
Both sides become i64; comparison works.

Note: `OpGtInt` / `OpGeInt` don't exist in `default/01_code.loft`
— the parser desugars `>` and `>=` into `<` / `<=` with swapped
operands, so the 4-Op set is complete.

### Historical view — original P200 framing (write side, since revised)

**Symptom (pre-fix):** `cargo test --release --test native native_binary_script`
failed with rustc E0308 mismatched types when emitting a binary-file
write through the `BigEndian` / `LittleEndian` open mode.

**Where:** `tests/scripts/20-binary.loft` lines 67 and 128 —
`f += <integer>` where `f` is a binary file and the integer literal
has no width cast.  The parser already warns: *"`f += <integer>`
without a width cast writes 8 bytes; for binary files
(BigEndian / LittleEndian) add `as i8` / `as i16` / `as i32` /
`as u8` / `as u16` / `as u32`."*  Native codegen still emits a
mismatched-width Rust expression that rustc rejects.

**Branch state (2026-05-02):** passes on `main`, fails on
`roadmap-lsp-eclipse`.  Plan-09 phase 00a re-surveyed the
emission and confirmed the root cause is read-side comparison
emission (block-tail role of `narrow_int_cast`), not the write-
side template — see "Plan-09 phase 00a finding" block below.

**Root cause:** the parser emits `Insert([Set(_wf, val), OpWriteFile(file, &_wf, db_tp)])` (`src/parser/objects.rs::write_to_file:431-505`) for `f += val`.  When `val` is `Type::Integer(...)` without a width cast, `db_tp` is the full 8-byte integer type.  Native codegen emits `OpWriteFile<T: FileVal>(cell, file, &mut _wf, db_tp)` with `T = i64`.  The READ counterpart emits a `(_ret) as <narrow>` cast at block-end (in `src/generation/emit.rs:845-849`'s `narrow_int_cast` branch) when `bl.result` is a narrow integer subtype — but in this script the loft `f += <int>` writes 8 bytes with NO narrow cast (so `bl.result` should be plain `Type::Integer`), yet `narrow_int_cast` returns `Some("u8")` because the FILE-level type metadata mislabels the read as `integer(0, 255)`.

**Fix path:**
1. Inspect `/tmp/loft_native_20_binary.rs` line 522 (write site) and 532 (read site) after a fresh build to confirm the actual emitted shapes.  Specifically check: what does `bl.name` look like for the `//reading file_4: integer(0, 255)` block — is it `"reading file"` or richer?
2. In `src/generation/emit.rs:844`, add a `let is_reading_file = bl.name.starts_with("reading file");` guard (mirroring the existing `is_iter_next` exception at line 844) so reading-file blocks skip the narrow cast at the block-result emission point.  This was tried in this session and the COMPILE error went away but a different runtime test (`single LE roundtrip` line 82) failed with wrong values — meaning the narrow cast is also load-bearing for the WRITE path's serialisation.  Don't repeat that fix without the next step.
3. Trace the WRITE path: `src/parser/objects.rs::write_to_file:493` calls `self.get_type(val_type)` for the un-cast case, returning the wide `Type::Integer` db_tp.  Native then serialises 8 bytes (correct), but the matching read uses a narrow db_tp, causing width mismatch.  Either:
   - Make `write_to_file` emit a parser-level error / hard-require the cast (matching the existing warning at line 482-491), forcing the user to write `f += val as i32` — closes P200 by user-fix at the loft level.
   - OR teach native codegen to widen the read result back to i64 at consumer boundaries, matching the interpreter's handling.

**Concrete first-30-min path:** read `/tmp/loft_native_20_binary.rs:520-540`, identify block name + `bl.result` type, then choose option (a) for the simpler fix.

**Plan-09 phase 00a finding (2026-05-02):**

The original phase 05 plan was scoped against the symptom
description (write-side template hard-coding `i32`).  Phase 05
step 5.0's `--native-emit` pre-flight survey showed all five
failing sites in `tests/scripts/20-binary.loft` are read-side
comparisons of the shape:

```rust
{({ //reading file_4: integer(0, 255)
    let mut var__read_3: i64 = i64::MIN as i64;
    OpReadFile(cell, var_f, &mut var__read_3, 1_i64, 11_i32);
    (var__read_3) as u8     // ← block-tail narrow
}) == (0_i64)               // ← E0308: u8 vs i64
}
```

The block-tail `as u8` is `narrow_int_cast`'s role #1 (block-tail
coercion).  The phase 05 plan was writing a fix for role #2 (param
narrowing), which is not what's biting.  Two candidate fix sites:

1. **Drop the block-tail narrow** when the consumer is `==` against
   a constant whose value range fits the narrow type — let the
   comparison happen at i64 width.
2. **Widen the constant** at comparison-emission time to match the
   block-tail's narrowed type (`(... as u8) == (0u8)`).  Site:
   wherever `==` is emitted with an integer literal RHS.

Either fix lives in comparison-emission code, not in
`OpWriteIntFile`'s template.  Phase 02 (param adapter) was
demoted from prereq because its split of role #2 doesn't reach
role #1.  Plan-09 phase 05 needs scope rewrite before
implementation — see
`doc/claude/plans/finished/09-native-runtime-rewrite/05-file.md`
§ "Diagnosis findings".

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

### 202. Native: `n_parallel_queue` family not implemented (CLOSED 2026-05-02)

**Closed by:** plan-09 phase 06 (commit landing this entry).  Three
new components shipped together:

1. **Runtime fns** in `src/codegen_runtime.rs`:
   `n_parallel_queue_native` / `_text_native` / `_ref_native` for
   the queue dispatch (closure-based, mirroring
   `n_parallel_for_*_native`); `n_parallel_buf_get_native` / `_text` /
   `_ref` for per-row reads; `n_parallel_buf_drop_native` / `_text` /
   `_ref` for end-of-loop cleanup.  All take `&UnsafeCell<Stores>`
   per phase 01's ABI; ref-flavour adopts a single result store
   (simpler than the interpreter's per-worker dispenser).
2. **Emitter** in `src/generation/ops/parallel.rs`:
   `ParallelQueueEmitter` mirrors phase 03's `ParallelForEmitter`
   but routes calls to the `_native` queue family.
   `ParallelBufRenameEmitter` is a pass-through name rewriter for
   the buf-get / buf-drop reads.  All 9 names registered in
   `src/generation/ops/mod.rs::build_registry`.
3. **Reachability fix** in `src/generation/mod.rs::collect_calls`
   — the worker-fn-via-d_nr-arg detection now extends to the
   queue family, so worker fns get pulled into the reachable set
   when emitted.  Without this, the closure refers to a fn that
   never gets emitted (E0425 "cannot find function").

**Verification:** native suite went from 87/93 → 89/93 (+2 script
fixes for 22_threading + 40_par_ref_return); native_dir went from
29/30 → 30/30 (19_threading fix).  Three pre-existing P200/P204/P205
sub-failures remain — not in plan-09 phase 06 scope.

**Regression tests** in `tests/codegen_emitter.rs`:
- `p202_parallel_queue_runtime_fns_registered` — pins the 9
  runtime-fn names with `Abi::Cell`.
- `p202_parallel_queue_emitter_registered` — pins the 9 emitter
  registry entries.

**Workaround (historical):** see "Workaround" below — no longer
needed.

**Symptom (historical):** native compilation fails on any script using
`for ... par(...)`:

```
error[E0061]: this function takes 6 arguments but 7 arguments were supplied
   --> /tmp/loft_native_19_threading.rs:371:35
    |
371 |     let mut var__par_len_2: i64 = n_parallel_queue(cell, var__vector_1, 8_i64, 8_i64, 4_i64, 547_i64, 0_i64);
```

**Where:** `default/01_code.loft:1086` declares `parallel_queue` as
"IR-only entry the codegen produces" (5 visible args).  Loft's
`for v in input par(b=worker(v), N) { body }` desugar emits
`n_parallel_queue(input, elem_sz, ret_sz, threads, worker_d_nr,
extras..., n_extra)` — 5+ args.  Bytecode interpreter implements it
in `src/native.rs:826` reading args off the operand stack and
dispatching via `crate::parallel::run_parallel_queue`.  Native
codegen has no equivalent and emits a `todo!()` stub; the call-site
arg count doesn't match the stub's signature, breaking rustc.

**Variants affected:** `n_parallel_queue`, `n_parallel_queue_text`,
`n_parallel_queue_ref`, `n_parallel_queue_narrow`,
`n_parallel_queue_fn` (all listed in `src/native.rs:145-169`).

**Companion ops:** `n_parallel_buf_get` (read result by index),
`n_parallel_buf_drop` (release buffer at end of for-loop) — same
gap.

**Fix path — Option A (intercept at codegen, recommended):**

Extend `src/generation/dispatch.rs:805` (the existing handler that
rewrites `n_parallel_for` / `n_parallel_for_light` to
`n_parallel_for_native`) to ALSO rewrite `n_parallel_queue` /
`n_parallel_queue_text` / `n_parallel_queue_ref` to the
corresponding `n_parallel_for_*_native` helpers.  The semantics
match per `default/01_code.loft:1086` ("Same arg layout as
parallel_for").  At rewrite time:

1. Allocate a result vector via `n_parallel_for_*_native`, store its
   `DbRef` into a fresh `_par_result_N` local.
2. Replace `n_parallel_queue(...)` with `_par_result_N.length()`
   (returns the row count the queue would have pushed).
3. Replace `n_parallel_buf_get(idx)` with vector read into the
   `_par_result_N` store.
4. Replace `n_parallel_buf_drop()` with `OpFreeRef(cell,
   _par_result_N, …)`.

Steps 3-4 require tracking the active `_par_result_N` across the
for-body — push/pop on a stack as the for-loop emits.  This is
~80-150 lines in dispatch.rs.

**Fix path — Option B (implement helpers in codegen_runtime.rs):**

Add `n_parallel_queue` etc. to `src/codegen_runtime.rs` mirroring
`src/native.rs:826`.  Each helper does the same rayon-based
dispatch but takes args as Rust values (not stack pops), pushes
results into a thread-local `Vec<u64>` buffer.  Add
`n_parallel_buf_get` reading from that buffer, `n_parallel_buf_drop`
popping it.  ~200-300 lines.  Closer to the interpreter's runtime
shape but requires duplicating buffer-stack machinery.

**Recommended:** Option A.  Less code, no new runtime state, reuses
existing `n_parallel_for_*_native` infrastructure already proven by
P199's parallel test fixes.

**First-30-min path:** read `dispatch.rs:805-898` (existing
`n_parallel_for` rewrite), trace the argument shape; mirror it for
`n_parallel_queue` family.  Write a probe loft script with
`for x in v par(b=fn(x), 4) { sum += b; }`, dump the IR with
`LOFT_LOG=static`, see exactly what the parser emits.

**Test:** `tests/native::native_dir` slot `19_threading`,
`tests/native::native_scripts` slots `22_threading`,
`40_par_ref_return`.

### 203. Native: block-scope file not flushed on exit (CLOSED 2026-05-02)

**Resolution summary**

The "file not flushed" framing was wrong.  Strace + diagnostic
instrumentation showed the file IS created, written, closed
correctly, then **deleted by the assertion's own `delete()` call**
— which was being **evaluated twice** because the
`OpConvIntFromEnum` template at `default/01_code.loft:705` (and
four sibling templates) substituted `@v1` / `@v2` in multiple
positions:

```
#rust"if @v1 == 255 {{ i64::MIN }} else {{ i64::from(@v1) }}"
                ↑                                       ↑
                first call                              second call
```

For `delete(path) == FileResult.Ok` the first call deletes the
file; the second call sees no file and returns `NotFound`; the
assertion compares `NotFound != Ok` and panics.

**Fix** (commit on `roadmap-lsp-eclipse`): `src/generation/calls.rs::output_call_template`
now pre-scans every template for repeated `@<name>` placeholders.
For each placeholder appearing 2+ times it emits a wrapping
`{ let _v_<name> = @<name>; … }` block, replaces every other
occurrence with `_v_<name>`, and lets the existing substitution
loop substitute the single remaining `@<name>` (the let-RHS).
Result: each argument expression evaluates exactly once regardless
of how many positions reference it.

**Affected templates closed by this fix:**
- `01_code.loft:690` (char→int)
- `01_code.loft:705` (enum→int — P203's specific manifestation)
- `01_code.loft:707` (int→enum)
- `01_code.loft:751` (ref equality)
- `01_code.loft:753` (ref inequality)

**Validation:**
- `cargo run --bin loft --release tests/scripts/repro_p203.loft` exits 0.
- `cargo test --release --test issues`: 540/540.
- `cargo test --release --test threading`: 43/43.
- `cargo test --release --test threading_chars`: 35/35.
- `cargo test --release --test native`: 87/93 (was 85/92; +2 due to P203 closure + repro_p203.loft no longer @EXPECT_FAIL).

**Original diagnosis (preserved for reference):**

`cargo test --release --test native native_scripts` (the
`42_file_result` slot) panics at runtime:

```
thread 'main' panicked at /tmp/loft_native_42_file_result.rs:287:14:
tests/scripts/42-file-result.loft:22 delete existing file
```

**Where:** `tests/scripts/42-file-result.loft` lines 19-23:

```loft
{f = file("test_loft_fr.txt");
 f += "test content";
}
assert(delete("test_loft_fr.txt") == FileResult.Ok, "delete existing file");
```

The file write happens inside a `{...}` block.  In interpreter mode,
the block-scope `OpFreeRef(f)` runs on block exit and that close-hook
flushes + closes the underlying OS file handle.  Native codegen emits
`OpFreeRef(cell, var_f, "var_f"); var_f.store_nr = u16::MAX;` at block
exit but the close happens too late (or not at all) — the next-line
`delete()` call sees the file still open and the OS rejects the
delete.

**Minimal reproducer:** create `tests/scripts/repro_p203.loft`:

```loft
fn main() {
  {f = file("repro_p203.txt");
   f += "x";
  }
  assert(delete("repro_p203.txt") == FileResult.Ok, "delete after block close");
}
```

`cargo run --bin loft --release tests/scripts/repro_p203.loft` —
PASSES (interpreter).
`cargo run --bin loft --release -- --native tests/scripts/repro_p203.loft`
— FAILS (native).

**Verified observation:** running `cargo run --bin loft --release --
tests/scripts/repro_p203.loft` panics on the assert AND the file
`repro_p203.txt` is never created on disk.  So the OS-level write
isn't reaching `File::create` (or it is, but the file is removed
before delete runs).  Compare with interpreter mode (passes) — the
write DOES reach the filesystem there.

**Where the divergence lives:**
- `src/codegen_runtime.rs:OpFreeRef` (line 95-122) DOES close
  File-typed handles when `ref_count <= 1` (line 109).  The lookup
  is by `stores.names.get("File")` — verify this match fires for
  the test's `var_f` at runtime.
- `src/codegen_runtime.rs:file_handle_write` (line 692-726) opens
  the file with `File::create(&file_name)` (line 707).  Returns the
  handle index; subsequent `OpWriteFile` writes via it.

**Fix path:**

1. Add `eprintln!` traces inside `file_handle_write` (line 707) and
   the OpWriteFile path (`src/codegen_runtime.rs:1138-1173`) to
   confirm whether `File::create` succeeds, the byte slice is
   non-empty, and `f.write_all(&data)` returns `Ok`.  Run repro_p203
   under native and inspect.
2. If `file_handle_write` IS invoked and File::create succeeds: the
   file is being created but immediately removed.  Suspects:
   - `file.pos + 28` (the file_ref slot) is being read as
     `i32::MIN` on the OpWriteFile path (line 1149), short-circuiting
     to early return (line 1151).  Verify by tracing.
   - The `var___ref_1` placeholder used by `n_file(...)` is
     uninitialised (related to P204 — same `__ref_*` class).  This
     is the most likely root cause given the symptom.  Native's
     `n_file(cell, "name", var___ref_1)` may write the file struct
     into a null-store placeholder, so `OpWriteFile` reads garbage
     for `file.pos + 28`.
3. If P204 turns out to be the upstream cause: P203 closes
   automatically once P204's `__ref_*` init is fixed.  Test with
   the P204 fix in place and re-run repro_p203 first.

**Actual root cause (identified 2026-05-02 via strace + diagnostic
phase 00 of plan 10):** the file IS created and written and closed
correctly.  The bug is in `default/01_code.loft:705`, the
`OpConvIntFromEnum` template:

```
#rust"if @v1 == 255 {{ i64::MIN }} else {{ i64::from(@v1) }}"
```

`@v1` is substituted **twice**.  When the assertion `delete(…) ==
FileResult.Ok` is generated, `n_delete()` is called twice:
1. First call: file exists → unlinks file → returns `Ok` (1)
2. Second call: file already deleted → returns `NotFound` (2)

The if-test sees the second result, returns 2, comparison `2 == 1`
fails, panic fires.

**strace evidence** (P203 cached binary):
```
openat("repro_p203.txt", O_WRONLY|O_CREAT|O_TRUNC, 0666) = 3
write(3, "x", 1) = 1
close(3) = 0                             ← OpFreeRef closed file
unlink("repro_p203.txt") = 0             ← first n_delete() succeeded
write(2, "thread 'main' panicked …")     ← second n_delete() returned NotFound
```

**Fix path:** five templates in `default/01_code.loft`
double-substitute their @v1/@v2 args and need let-bind:
- line 690 (char→int)
- line 705 (enum→int) — P203's specific bug
- line 707 (int→enum)
- line 751 (ref equality)
- line 753 (ref inequality)

Pattern:
```
#rust"{ let _v = @v1; if _v == 255 { i64::MIN } else { i64::from(_v) } }"
```

~5 trivial template edits + regression tests for side-effect-in-
enum-compare.  Closes a class of latent bugs (any side-effecting
call compared to enum/null/ref produces wrong results).

**Tracked separately** — too small for a multi-phase plan.

The plan-10 directory was originally framed as a P203 fix, but
that framing was wrong (its phase 00 diagnostic surfaced this
template double-substitution bug).  Plan 10 was rescoped as a
deferred structural simplification of the dep-tracking cleanup
gate; it does NOT close P203.  The strace trace + diagnostic
evidence is preserved at
[plan 10's phase 00 characterisation](plans/10-scope-exit-emission/00-characterize.md)
under "Historical context — P203 diagnostic".

**Test:** `tests/native::native_scripts` slot `42_file_result`.
Reproducer `tests/scripts/repro_p203.loft` (verified: panics with
`exit=0` after eprintln but file not on disk).  Interpreter / wrap
test `tests/wrap.rs::file_result` PASSES — issue is native-only.

### 204. Tail-expression return of inner helper call discarded (CLOSED 2026-05-02)

**Closed by:** plan-11 (commit landing this entry).  The fix was
a two-character change in `src/generation/pre_eval.rs`'s
`detect_ref_tail_capture` walker — calling `op.unspan()` before
matching against `Value::Line(_)` / `Value::Call(_, _)` /
`Value::Return(_)`.  Before the fix, Span-wrapped operators
(carrying source-position info) bailed the walker on its
`_ => return None` arm even though the underlying value was a
Call.  After unspanning, the walker correctly identifies the
tail Call and the existing emit-time capture path runs:

```rust
// emit at call_idx:
let __native_tail_ret: DbRef = n_p204_inner(cell, var_p204_x, var_p204_y);
// ... cleanup ops ...
// emit at ret_idx (replacing Return(Null)'s null sentinel):
return __native_tail_ret;
```

Step 11.1's actual-error survey confirmed all 3 failing tests
(`85_yield_resume`, `87_store_leaks`, `repro_p204.loft`) shared
the same shape: `return DbRef { store_nr: u16::MAX, rec: 0, pos: 8 }`
in a struct-returning function whose loft body is
`tail_call_to_struct_returning_fn()`.  Step 11.2's parser-side
investigation found the existing `detect_ref_tail_capture`
infrastructure was correct but its walker didn't unspan.

**Symptom (historical):** `cargo test --release --test native native_scripts`
(slots `87_store_leaks` and `85_yield_resume`) panicked:

```
thread 'main' panicked at src/database/allocation.rs:347:34:
index out of bounds: the len is 4 but the index is 65535
```

Interpreter mode (`cargo run --bin loft --release -- --interpret
tests/scripts/87-store-leaks.loft`) PASSES — the bytecode interpreter
routes the helper's result through a `__ref_*` placeholder.  Native
codegen ignores the helper's return value entirely.

**Where:** `tests/scripts/87-store-leaks.loft:26-29`:

```loft
fn sl_wrap(sl_x: const SlSf) -> SlSf {
  sl_y = SlSf { sl_v: [1.0] };
  sl_inner(sl_x, sl_y)  // tail expression — should be the return value
}
```

The native codegen emits (verified 2026-05-01 via
`--native-emit /tmp/p204b.rs tests/scripts/repro_p204.loft`):

```rust
fn n_p204_wrap(cell, mut var_p204_x: DbRef) -> DbRef {
    n_set_store_lock(stores, var_p204_x, true);
    let mut var_p204_y: DbRef = stores.null_named("var_p204_y");
    var_p204_y = OpDatabase(cell, var_p204_y, 65_i32);
    // ... initialize var_p204_y ...
    n_p204_inner(cell, var_p204_x, var_p204_y);  // result DISCARDED
    OpFreeRef(cell, var_p204_y, "var_p204_y"); var_p204_y.store_nr = u16::MAX;
    n_set_store_lock(stores, var_p204_x, false);
    return DbRef { store_nr: u16::MAX, rec: 0, pos: 8 };  // null sentinel
}
```

The function discards `n_p204_inner`'s return and returns a null
DbRef.  The caller then does
`let _src = n_p204_wrap(...); OpCopyRecord(cell, _src, var_q, ...);`
where `_src.store_nr == u16::MAX` → panic in `OpCopyRecord` reading
`stores.allocations[u16::MAX]`.

**Compare with bytecode interpreter:** The IR for `sl_inner` at
function exit is `[..., return sl_r]` where `sl_r` is the locally-
allocated struct.  Interpreter's `gen_return` (in
`src/state/codegen.rs`) routes the local through the caller-provided
`__ref_*` placeholder via `OpCopyRecord` or adoption.  Native codegen
falls through to the catch-all "Reference return" path that emits
`return DbRef { store_nr: u16::MAX, ... }`.

**Minimal reproducer:** `tests/scripts/repro_p204.loft` (vector field
on the struct triggers the relevant codegen path; without a Reference-
typed field, the bug doesn't surface):

```loft
struct P204_S { p204_v: vector<float> }
fn p204_inner(p204_a: const P204_S, p204_b: const P204_S) -> P204_S {
  P204_S { p204_v: [p204_a.p204_v[0] + p204_b.p204_v[0]] }
}
fn p204_wrap(p204_x: const P204_S) -> P204_S {
  p204_y = P204_S { p204_v: [1.0] };
  p204_inner(p204_x, p204_y)  // tail expression discarded by native
}
fn p204_ref_placeholder_uninitialised() {
  p204_p = P204_S { p204_v: [2.0] };
  p204_q = p204_wrap(p204_p);
  assert(p204_q.p204_v[0] == 3.0, "p204: q.v={p204_q.p204_v[0]}");
}
```

Verified 2026-05-01: `--interpret` exits 0, default (native) panics
with `index out of bounds`.

**Fix path:**

1. Identify the codegen site that emits the `return DbRef { store_nr:
   u16::MAX, ... }` null sentinel for Reference-returning functions.
   Likely in `src/generation/emit.rs::output_block` or a tail-expression
   handler — search for `store_nr: u16::MAX, rec: 0, pos: 8` in
   `src/generation/`.
2. Detect the case where the function's tail expression is a
   `Value::Call(inner_d_nr, …)` whose callee returns a Reference of the
   same type as the outer function's return.
3. Two implementation options:
   - **Option A (capture-and-return):** Emit the inner call into a
     `let _ret = n_inner(cell, args, …);` and then `return _ret;`
     instead of discarding.  Requires the caller's `__ref_*` to be
     initialised so the inner call has somewhere to write.
   - **Option B (pass-through-placeholder):** Make the outer function
     take a `__ref_*` hidden param matching the return type, and pass
     it through to the inner call.  Mirrors how 87_store_leaks's
     `n_sl_wrap` is generated (it DOES take `__ref_1`) — investigate
     why my reproducer takes a different shape (likely `not null`
     annotation difference).
4. Investigation entry point: dump IR with `LOFT_LOG=static` for both
   `tests/scripts/87-store-leaks.loft` (which generates `n_sl_wrap`
   with `__ref_1` param) and `tests/scripts/repro_p204.loft` (which
   generates `n_p204_wrap` WITHOUT `__ref_1`) — diff the IR to see
   what triggers the placeholder injection in scope analysis.

**Investigation note (2026-05-01):** A naive fix attempted in
`src/generation/calls.rs::output_call_user_fn` to emit `var___ref_X
= OpDatabase(...)` before each call with `__ref_*` arg PASSED for
the simpler case but BROKE 62_index_range_queries and 76_struct_vector_return
with "Incomplete record" panic — those tests already had the
OpDatabase elsewhere in the IR; double-init clears the existing
record.  A correct fix needs to be aware of init state, not blanket-
emit.

**Test:** `tests/native::native_scripts` slots `87_store_leaks` and
`85_yield_resume`.  Reproducer `tests/scripts/repro_p204.loft`.

### 205. Generic text return dangles via `Str::new(&local_String)` (CLOSED 2026-05-02)

**Closed by:** plan-09 phase 07 (commit landing this entry).
`src/generation/emit.rs` patched at two emit sites
(`Value::Return` text-wrap path at line 188+ and block-tail
`wrap_result` path at line 887+) to detect the dangling case
and route through `stores.scratch`:

```rust
let needs_p205_scratch = wrap_text && {
    let def = self.data.def(self.def_nr);
    matches!(def.returned, Type::Text(_))
        && !def.attributes.iter().any(|a| {
            matches!(a.typedef, Type::RefVar(ref t) if matches!(**t, Type::Text(_)))
        })
};
if needs_p205_scratch {
    write!(w, "{{ stores.scratch.push((")?;
    self.output_code_inner(w, val)?;
    write!(w, ").to_string()); Str::new(stores.scratch.last().unwrap()) }}")?;
}
```

The detection: function returns `Type::Text` but has no
`Type::RefVar(Type::Text(_))` attribute (= `text_return` didn't
add a proper work buffer).  In that case the otherwise-emitted
`Str::new(&local)` would dangle.  Routing through `stores.scratch`
gives the value a `stores`-scoped backing String.

Two regression tests pin the fix: `p205_repro_passes_under_native`
runs `tests/scripts/repro_p205.loft` under native and asserts
exit 0; `p205_no_str_new_of_local_in_corpus` greps the doc-test
baseline for the `Str::new(&var___ret_*)` pattern and fails if
it reappears.

The probe (Outcome B) is documented in
`doc/claude/plans/finished/09-native-runtime-rewrite/07-generics.md`
§ "Implementation notes — Why not fix `text_return` parser-side."

**Symptom (historical):** `cargo test --release --test native native_scripts`
(slot `86_interfaces`) panics:

```
thread 'main' panicked at /tmp/loft_native_86_interfaces.rs:288:14:
tests/scripts/86-interfaces.loft:47 single bound method call
```

The assert message is just "single bound method call" — it fires
because `if_label(if_it) == "widget"` evaluates to false at runtime.

**Where:** `tests/scripts/86-interfaces.loft:25-32, 45-48`:

```loft
struct IfItem { if_name: text, if_score: integer }
fn to_label(self: IfItem) -> text { return self.if_name; }
fn if_label<T: Labelable>(if_x: T) -> text {
  return if_x.to_label();
}
fn test_single_bound() {
  if_it = IfItem { if_name: "widget", if_score: 5 };
  assert(if_label(if_it) == "widget", "single bound method call");
}
```

The native codegen for `if_label` (specialised as `t_6IfItem_if_label`):

```rust
fn t_6IfItem_if_label(cell, var_if_x: DbRef) -> Str {
    let mut var___ret_1: String = t_6IfItem_to_label(cell, var_if_x).to_string();
    return Str::new(&var___ret_1);  // dangling — &var___ret_1 dies at return
}
```

`Str` is a `(*const u8, u32)` raw pointer + length pair.  `Str::new(&s)`
captures `s.as_ptr()`, but `s` is the local `var___ret_1: String` which
drops at function return.  The returned `Str` then points into freed
memory; the caller's `==` compares garbage bytes against `"widget"`.

**Minimal reproducer:** `tests/scripts/repro_p205.loft` (CamelCase
interface name required by parser):

```loft
interface P205Labelable {
  fn p205_to_label(self: Self) -> text
}
struct P205_S { p205_name: text }
fn p205_to_label(self: P205_S) -> text { return self.p205_name; }
fn p205_label<T: P205Labelable>(p205_x: T) -> text {
  return p205_x.p205_to_label();
}
fn p205_generic_text_return_dangles() {
  p205_s = P205_S { p205_name: "widget" };
  assert(p205_label(p205_s) == "widget", "p205: generic text return");
}
```

Verified 2026-05-01: `cargo run --bin loft --release -- --interpret
tests/scripts/repro_p205.loft` exits 0; default (native) panics with
the assert.

**Root cause:** `src/parser/control.rs:369-377` explicitly skips
`text_return` (the `__work_*` hidden-param injection for text-
returning fns) when the function is `DefType::Generic`:

```rust
// I9-var: skip ref_return/text_return for generic templates.
if self.data.def_type(self.context) != DefType::Generic {
    if let Type::Text(ls) = t {
        self.text_return(ls);
    }
    ...
}
```

The comment explains: the template body is shared across all
specialisations, but `__work_*` injection promotes locals to hidden
params — for non-Text specialisations (Integer, Float) the hidden
param is wrong.  So generics keep their template body unmodified,
and specialised copies inherit it WITHOUT the `__work_*` injection.

When a generic with `Type::Text` return is specialised (via
`src/parser/mod.rs::try_generic_instantiation:1190`), the
specialised function inherits the template's body that ends with
`return Str::new(local_string)` — and at native codegen time
(`src/generation/coroutine.rs` / `src/generation/emit.rs`) this
emits the dangling `Str::new(&var___ret_1)`.

**Fix path:**

1. In `src/parser/mod.rs::try_generic_instantiation` (line 1190),
   AFTER computing `concrete` (line 1210) and creating the
   specialised function, check if `concrete` is `Type::Text(_)`.
2. If yes, run `text_return` on the specialised function's
   return-statement scan list.  This requires either:
   - Re-running the relevant portion of `parse_block` /
     `parse_function_body` for the specialised copy with
     `def_type` temporarily set to `Function` (not `Generic`).
   - OR post-processing the specialised IR to inject `__work_1` as
     a hidden parameter and rewrite trailing `return X` → `*var___work_1 = X.to_string(); return Str::new(var___work_1.as_str())`.
3. The post-processing approach is cleaner: walk the specialised
   function's `code: Block` looking for `Value::Return(Value::*)`
   nodes whose result type is Text, and rewrite them to use the
   work-buffer pattern.  Mirrors what `src/parser/control.rs::text_return`
   does for non-generic functions.
4. Update each call site to pass a `&mut String` work-buffer for
   the new hidden param.  This is automatic for native codegen
   because `output_call_user_fn` already iterates all attributes
   (including hidden); it would emit `&mut _w_<N>` for the new
   `__work_*` param.

**First-30-min path:** read `src/parser/control.rs:2337-2540`
(`text_return` + `parse_block` flow); inspect a working specialised
text-returning function (e.g. `t_4text_split` from
`default/02_images.loft:132`) by grepping the generated rust to see
the working pattern.  Then write the post-processing pass for
specialised generics.

**Test:** `tests/native::native_scripts` slot `86_interfaces`.
Reproducer `tests/scripts/repro_p205.loft` (verified: interpreter
PASSES, native panics with assert message).

**Plan-09 phase 07 probe outcome (2026-05-02):**

Phase 07's diagnostic probe (step 7.3) tested whether removing
the `DefType::Generic` skip at `src/parser/control.rs:375` would
close the dangle on its own.  Result: **Outcome B confirmed** —
the skip removal makes `text_return` run for generics, but the
dangling pattern persists:

```rust
fn t_6P205_S_p205_label(cell: &..., mut var_p205_x: DbRef) -> Str {
    let mut var___ret_1: String =
        t_6P205_S_p205_to_label(cell, var_p205_x).to_string();
    return Str::new(&var___ret_1)   // ← var___ret_1 dropped at return
}
```

Diagnosis: `text_return`'s shape is a buffer-promotion that
expects to convert `-> text` returns into `-> ()` with a
`&mut String` write-buffer parameter.  For the bounded-generic
specialisation it only created the `var___ret_1: String` local
without changing the function signature — so the function still
returns `Str` and the local dangles.

**Conclusion:** the bug is NOT the skip itself; `text_return`'s
transformation isn't complete enough for the bounded-generic case.
Phase 07 needs a custom emitter (Outcome B path) that emits owned
`String` from the Op directly, bypassing the buffer indirection
`text_return` was trying to create.  See
`doc/claude/plans/finished/09-native-runtime-rewrite/07-generics.md` step
7.5 for the emitter-implementation plan.

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
